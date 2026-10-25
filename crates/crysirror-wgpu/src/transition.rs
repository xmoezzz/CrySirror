// SPDX-License-Identifier: MPL-2.0
//! Screen transitions (ADR 0022). Every frame is composed into one of two
//! retained frame textures and then presented. While a transition is active
//! the last presented frame stays frozen as the old scene and the vendored
//! siglus_rs `wipe.wgsl` (unmodified, included from the vendored tree)
//! composes old → new. As in siglus_rs, wipe inputs are sampled as
//! gamma-encoded bytes (non-sRGB views), so blends match Siglus arithmetic.
use crysirror_scene::layer::TransitionFrame;
use wgpu::util::DeviceExt;

struct FrameTex {
    _texture: wgpu::Texture,
    /// View in the presentation format: render passes write through it.
    render: wgpu::TextureView,
    /// Raw (non-sRGB) view: the wipe samples gamma-encoded bytes.
    raw: wgpu::TextureView,
}
struct Frames {
    size: [u32; 2],
    tex: [FrameTex; 2],
    out: FrameTex,
    last: usize,
    has_last: bool,
}
pub(crate) struct Transitions {
    format: wgpu::TextureFormat,
    raw_format: wgpu::TextureFormat,
    present_layout: wgpu::BindGroupLayout,
    copy_pipeline: wgpu::RenderPipeline,
    decode_pipeline: wgpu::RenderPipeline,
    wipe_layout: wgpu::BindGroupLayout,
    wipe_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    frames: Option<Frames>,
    black: wgpu::TextureView,
    white: wgpu::TextureView,
    mask: Option<(String, wgpu::Texture, wgpu::TextureView)>,
}
/// Uniform layout of siglus_rs `WipeUniform`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WipeUniform {
    kind_progress: [f32; 4],
    options: [f32; 16],
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
fn pixel(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("transition constant"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
        )
        .create_view(&Default::default())
}

impl Transitions {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let raw_format = format.remove_srgb_suffix();
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present layout"),
            entries: &[texture_entry(0)],
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/present.wgsl").into()),
        });
        let present_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[&present_layout],
            push_constant_ranges: &[],
        });
        let present = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("present"),
                layout: Some(&present_pl),
                vertex: wgpu::VertexState {
                    module: &present_shader,
                    entry_point: "vs_main",
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &present_shader,
                    entry_point: entry,
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
            })
        };
        let copy_pipeline = present("fs_copy");
        let decode_pipeline = present(if format.is_srgb() { "fs_decode" } else { "fs_copy" });
        let wipe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wipe layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                sampler_entry(3),
                texture_entry(4),
                sampler_entry(5),
                texture_entry(6),
                sampler_entry(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let wipe_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("siglus wipe"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../siglus-upstream/siglus_scene_vm/src/render/shaders/wipe.wgsl").into(),
            ),
        });
        let wipe_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wipe"),
            bind_group_layouts: &[&wipe_layout],
            push_constant_ranges: &[],
        });
        let wipe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("siglus wipe"),
            layout: Some(&wipe_pl),
            vertex: wgpu::VertexState {
                module: &wipe_shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &wipe_shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(raw_format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wipe sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            format,
            raw_format,
            present_layout,
            copy_pipeline,
            decode_pipeline,
            wipe_layout,
            wipe_pipeline,
            sampler,
            frames: None,
            black: pixel(device, queue, [0, 0, 0, 255]),
            white: pixel(device, queue, [255; 4]),
            mask: None,
        }
    }
    fn frame_tex(&self, device: &wgpu::Device, size: [u32; 2]) -> FrameTex {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composed frame"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.raw_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[self.format],
        });
        let render = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.format), ..Default::default() });
        let raw = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.raw_format), ..Default::default() });
        FrameTex { _texture: texture, render, raw }
    }
    /// The view this frame composes into.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, size: [u32; 2]) -> &wgpu::TextureView {
        if self.frames.as_ref().map_or(true, |f| f.size != size) {
            self.frames = Some(Frames {
                size,
                tex: [self.frame_tex(device, size), self.frame_tex(device, size)],
                out: self.frame_tex(device, size),
                last: 1,
                has_last: false,
            });
        }
        let f = self.frames.as_ref().unwrap();
        &f.tex[1 - f.last].render
    }
    /// Keyed mask image (gamma bytes, sampled raw like siglus_rs).
    pub(crate) fn mask_key(&self) -> Option<&str> {
        self.mask.as_ref().map(|m| m.0.as_str())
    }
    pub(crate) fn set_mask(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: String, width: u32, height: u32, rgba: &[u8]) {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("transition mask"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        let view = texture.create_view(&Default::default());
        self.mask = Some((key, texture, view));
    }
    /// Present the composed frame, through the wipe when `transition` is active.
    pub(crate) fn finish(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        transition: Option<&TransitionFrame>,
        canvas: [u32; 2],
    ) {
        let f = self.frames.as_mut().expect("begin() before finish()");
        let cur = 1 - f.last;
        let present_src;
        let pipeline;
        match transition {
            Some(t) => {
                let mut options = [0f32; 16];
                for (d, s) in options.iter_mut().zip(t.options) {
                    *d = s as f32;
                }
                let uniform = WipeUniform {
                    kind_progress: [t.code as f32, t.progress.clamp(0., 1.), canvas[0] as f32, canvas[1] as f32],
                    options,
                };
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("wipe uniform"),
                    contents: bytemuck::bytes_of(&uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let old = if f.has_last { &f.tex[f.last].raw } else { &self.black };
                let mask = match (&t.mask, &self.mask) {
                    (Some(_), Some(m)) => &m.2,
                    _ => &self.white,
                };
                let s = wgpu::BindingResource::Sampler(&self.sampler);
                let v = wgpu::BindingResource::TextureView;
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("wipe"),
                    layout: &self.wipe_layout,
                    entries: &[
                        // `under`: the whole frame is the wiped range, so nothing lies beneath.
                        wgpu::BindGroupEntry { binding: 0, resource: v(&self.black) },
                        wgpu::BindGroupEntry { binding: 1, resource: s.clone() },
                        wgpu::BindGroupEntry { binding: 2, resource: v(&f.tex[cur].raw) },
                        wgpu::BindGroupEntry { binding: 3, resource: s.clone() },
                        wgpu::BindGroupEntry { binding: 4, resource: v(old) },
                        wgpu::BindGroupEntry { binding: 5, resource: s.clone() },
                        wgpu::BindGroupEntry { binding: 6, resource: v(mask) },
                        wgpu::BindGroupEntry { binding: 7, resource: s },
                        wgpu::BindGroupEntry { binding: 8, resource: buffer.as_entire_binding() },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("siglus wipe"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &f.out.raw,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.wipe_pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
                drop(pass);
                present_src = &f.out.raw;
                pipeline = &self.decode_pipeline;
            }
            None => {
                present_src = &f.tex[cur].render;
                pipeline = &self.copy_pipeline;
            }
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.present_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(present_src) }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        // Outside a transition the frame just shown becomes the retained old scene.
        if transition.is_none() {
            f.last = cur;
            f.has_last = true;
        }
    }
}
