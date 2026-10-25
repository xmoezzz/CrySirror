// SPDX-License-Identifier: MPL-2.0
//! Real wgpu 0.20 backend, independent of winit and native VM instruction sets.
#![forbid(unsafe_code)]
pub mod geometry;
mod glyph_run;
pub mod lights;
pub mod particles;
pub mod pbr;
pub mod probes;
mod puppet;
mod raytrace;
pub mod text;
mod transition;
pub mod vsm;
use anyhow::{Context, Result, ensure};
pub use crysirror_core::graphics::{GraphicsCapabilities, RenderingPath};
use crysirror_core::state::{ClientId, Color, Rect, SceneState, TextRun, Visual};
use crysirror_runtime::{assets::Assets, model::Scene3d};
use crysirror_scene::{
    Primitive,
    layer::{Blend, Paint, rectangle},
};
use geometry::{Vertex2d, Vertex3d, Viewport};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{Arc, mpsc},
};
use wgpu::util::DeviceExt;

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}
impl Gpu {
    pub fn capabilities(&self) -> GraphicsCapabilities {
        GraphicsCapabilities {
            raster: true,
            compute_raytrace: self
                .adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
                && self.device.limits().max_storage_buffers_per_shader_stage >= 1
                && self.device.limits().max_storage_textures_per_shader_stage >= 1,
            hardware_raytrace: false,
        }
    }
    pub async fn request(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface,
                force_fallback_adapter: false,
            })
            .await
            .context("no suitable GPU adapter (headless Linux needs Vulkan/Mesa)")?;
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("CrySirror"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                },
                None,
            )
            .await?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }
}
struct Texture {
    bind: wgpu::BindGroup,
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
    bytes: u64,
}
struct Depth {
    size: [u32; 2],
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}
pub struct RenderTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub size: [u32; 2],
}
struct Draw {
    key: String,
    range: Range<u32>,
    clip: Option<[f32; 4]>,
    blend: Blend,
    /// (mask texture key, tone-curve texture key) when a layer filter applies.
    filter: Option<(String, String)>,
}
/// GPU record of `LayerFilter` (see quad.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FilterGpu {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
    e: [f32; 4],
}
fn filter_gpu(f: &crysirror_scene::layer::LayerFilter) -> FilterGpu {
    use crysirror_scene::layer::Distortion;
    let flags = u32::from(f.tone_curve.is_some())
        | u32::from(f.mask.is_some()) << 1
        | u32::from(f.mask.as_ref().is_some_and(|m| m.gray_alpha)) << 2
        | u32::from(f.alpha_test) << 3;
    let (kind, p) = match f.distortion {
        None => (0., [0.; 5]),
        Some(Distortion::Mosaic { cut, aspect }) => (1., [cut, aspect, 0., 0., 0.]),
        Some(Distortion::Raster {
            vertical,
            fractions,
            waves,
            power,
            progress,
        }) => (
            if vertical { 3. } else { 2. },
            [fractions, waves, power, progress, 0.],
        ),
        Some(Distortion::RadialBlur {
            center,
            power,
            coeff,
        }) => (4., [center[0], center[1], power, coeff, 0.]),
        Some(Distortion::ThresholdFade {
            fade,
            threshold,
            invert,
        }) => (if invert { 6. } else { 5. }, [fade, threshold, 0., 0., 0.]),
    };
    let curve = f.tone_curve.as_ref();
    FilterGpu {
        a: [f.mono, f.invert, f.bright, f.dark],
        b: [f.color_rate, f.color[0], f.color[1], f.color[2]],
        c: [f.add[0], f.add[1], f.add[2], flags as f32],
        d: [
            curve.map_or(0., |t| t.row),
            curve.map_or(0., |t| t.saturation),
            kind,
            p[0],
        ],
        e: [p[1], p[2], p[3], p[4]],
    }
}
pub struct Renderer {
    pub gpu: Arc<Gpu>,
    pub format: wgpu::TextureFormat,
    quad_pipelines: Vec<wgpu::RenderPipeline>,
    filtered_pipelines: Vec<wgpu::RenderPipeline>,
    filter_layout: wgpu::BindGroupLayout,
    filter_buffer: Option<wgpu::Buffer>,
    /// (mask key, curve key) → bind group over this frame's filter buffer.
    filter_binds: std::collections::HashMap<(String, String), wgpu::BindGroup>,
    engine_frame: Vec<Primitive>,
    ray_composite_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: BTreeMap<String, Texture>,
    glyph_cells: BTreeMap<String, glyph_run::GlyphBitmap>,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    scene3d: Option<Scene3d>,
    mesh_buffer: Option<wgpu::Buffer>,
    mesh_count: u32,
    depth: Option<Depth>,
    path: RenderingPath,
    raytracer: Option<raytrace::RayTracer>,
    /// Physically based path (ADR 0010); created on first use.
    pbr: Option<pbr::PbrRenderer>,
    transitions: transition::Transitions,
    transition: Option<crysirror_scene::layer::TransitionFrame>,
    /// Puppet layer targets and pipelines (ADR 0029).
    puppets: puppet::PuppetRenderer,
}
impl Renderer {
    pub fn new(gpu: Arc<Gpu>, format: wgpu::TextureFormat) -> Self {
        let d = &gpu.device;
        let texture_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let quad_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad layout"),
            bind_group_layouts: &[&texture_layout],
            push_constant_ranges: &[],
        });
        let camera_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let mesh_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh layout"),
            bind_group_layouts: &[&camera_layout],
            push_constant_ranges: &[],
        });
        let quad_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/quad.wgsl").into()),
        });
        let mesh_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh.wgsl").into()),
        });
        const QUAD_ATTR: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x2,2=>Float32x4,3=>Float32];
        let filter_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer filter layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let filtered_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filtered quad layout"),
            bind_group_layouts: &[&texture_layout, &filter_layout],
            push_constant_ranges: &[],
        });
        const MESH_ATTR: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4];
        let blends = [
            Blend::Normal,
            Blend::Add,
            Blend::Subtract,
            Blend::Multiply,
            Blend::Screen,
        ];
        let quad_pipeline = |blend: Blend, filtered: bool| {
            let color = match blend {
                Blend::Normal => wgpu::BlendState::ALPHA_BLENDING.color,
                Blend::Add => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                Blend::Subtract => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::ReverseSubtract,
                },
                Blend::Multiply => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                Blend::Screen => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("layer compositing"),
                layout: Some(if filtered {
                    &filtered_layout
                } else {
                    &quad_layout
                }),
                vertex: wgpu::VertexState {
                    module: &quad_shader,
                    entry_point: "vs_main",
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex2d>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &QUAD_ATTR,
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &quad_shader,
                    entry_point: match (filtered, matches!(blend, Blend::Multiply | Blend::Screen))
                    {
                        (false, true) => "fs_premultiply",
                        (false, false) => "fs_main",
                        (true, true) => "fs_filtered_premultiply",
                        (true, false) => "fs_filtered",
                    },
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color,
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        let quad_pipelines: Vec<_> = blends.iter().map(|b| quad_pipeline(*b, false)).collect();
        let filtered_pipelines: Vec<_> = blends.iter().map(|b| quad_pipeline(*b, true)).collect();
        let mesh_pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D opaque"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3d>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_ATTR,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let ray_composite_pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("replace raytraced color"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex2d>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &QUAD_ATTR,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let camera_buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear image"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let puppets =
            puppet::PuppetRenderer::new(d, &texture_layout, wgpu::TextureFormat::Rgba8UnormSrgb);
        Self {
            puppets,
            format,
            quad_pipelines,
            filtered_pipelines,
            filter_layout,
            filter_buffer: None,
            filter_binds: std::collections::HashMap::new(),
            engine_frame: Vec::new(),
            ray_composite_pipeline,
            mesh_pipeline,
            texture_layout,
            sampler,
            textures: BTreeMap::new(),
            glyph_cells: BTreeMap::new(),
            camera_buffer,
            camera_bind,
            scene3d: None,
            mesh_buffer: None,
            mesh_count: 0,
            depth: None,
            path: RenderingPath::Raster,
            raytracer: None,
            pbr: None,
            transitions: transition::Transitions::new(&gpu.device, &gpu.queue, format),
            transition: None,
            gpu,
        }
    }
    /// The PBR renderer once a PBR scene was set (statistics accessors).
    pub fn pbr(&self) -> Option<&pbr::PbrRenderer> {
        self.pbr.as_ref()
    }
    /// This frame's screen transition (ADR 0022); `None` presents directly.
    pub fn set_transition(
        &mut self,
        transition: Option<crysirror_scene::layer::TransitionFrame>,
    ) -> Result<()> {
        if let Some(t) = &transition {
            ensure!(
                crysirror_scene::layer::transition_code_supported(t.code) && t.progress.is_finite(),
                "unsupported transition"
            );
            if t.code == crysirror_scene::layer::ORDERED_DITHER_TRANSITION_CODE {
                ensure!(
                    (1..=8192).contains(&t.options[0])
                        && (1..=8192).contains(&t.options[1])
                        && (0..=1).contains(&t.options[2])
                        && (1..=65_536).contains(&t.options[3])
                        && t.mask.is_none()
                        && !t.generated_mask,
                    "invalid ordered dither transition parameters"
                );
            }
        }
        self.transition = transition;
        Ok(())
    }
    fn sync_transition_mask(&mut self, assets: &mut Assets) -> Result<()> {
        let Some((owner, asset)) = self
            .transition
            .as_ref()
            .and_then(|t| Some((t.owner, t.mask.clone()?)))
        else {
            return Ok(());
        };
        let key = Self::image_key(assets, owner, &asset)?;
        if self.transitions.mask_key() == Some(key.as_str()) {
            return Ok(());
        }
        let (w, h, rgba) = match assets.surface(ClientId(owner), &asset) {
            Some(s) => (s.width, s.height, s.rgba.to_vec()),
            None => {
                let bytes = assets.read_for(ClientId(owner), &asset)?;
                let image = assets.decode_image(ClientId(owner), &asset, bytes.as_ref())?;
                (image.width, image.height, image.rgba)
            }
        };
        let limit = self.gpu.device.limits().max_texture_dimension_2d;
        ensure!(
            w > 0 && h > 0 && w <= limit && h <= limit,
            "transition mask dimensions"
        );
        self.transitions
            .set_mask(&self.gpu.device, &self.gpu.queue, key, w, h, &rgba);
        Ok(())
    }
    /// Select the PBR path with an extracted scene, or the legacy path with `None`.
    pub fn set_pbr_scene(&mut self, scene: Option<crysirror_scene::view::PbrScene>) -> Result<()> {
        if scene.is_some() && self.pbr.is_none() {
            self.pbr = Some(pbr::PbrRenderer::new(self.gpu.clone(), self.format)?);
        }
        if let Some(p) = &mut self.pbr {
            p.set_scene(scene);
        }
        Ok(())
    }
    fn pbr_active(&self) -> bool {
        self.pbr.as_ref().is_some_and(|p| p.active())
    }
    pub fn set_rendering_path(&mut self, path: RenderingPath) -> Result<()> {
        self.gpu.capabilities().require(path).map_err(|e| {
            anyhow::anyhow!("{e}; hardware ray tracing is not wired in this backend")
        })?;
        if path == RenderingPath::ComputeRaytrace && self.raytracer.is_none() {
            self.raytracer = Some(raytrace::RayTracer::new(&self.gpu));
        }
        self.path = path;
        Ok(())
    }
    pub fn set_scene3d(&mut self, scene: Option<&Scene3d>) -> Result<()> {
        let vertices = match scene {
            Some(s) => geometry::triangles(s)?,
            None => Vec::new(),
        };
        self.mesh_count = u32::try_from(vertices.len())?;
        self.mesh_buffer = if vertices.is_empty() {
            None
        } else {
            Some(
                self.gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("static 3D scene"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };
        self.scene3d = scene.cloned();
        Ok(())
    }
    pub fn set_engine_frame(&mut self, frame: Vec<Primitive>) -> Result<()> {
        ensure!(frame.len() <= 32768, "extracted frame limit");
        for p in &frame {
            p.paint.validate()?;
            ensure!(
                p.size.iter().all(|v| *v > 0 && *v <= 16384)
                    && p.extent
                        .iter()
                        .all(|v| v.is_finite() && *v > 0. && *v <= 16384.)
                    && p.quad.iter().flatten().all(|v| v.is_finite())
                    && p.quad.iter().all(|v| v[3] > 0.)
                    && p.uv.iter().all(|v| v.is_finite()),
                "invalid extracted primitive"
            );
            if let Some(c) = p.clip {
                ensure!(
                    crysirror_scene::layer::rect_valid(c),
                    "invalid extracted clip"
                );
            }
        }
        self.engine_frame = frame;
        Ok(())
    }
    fn upload(&mut self, key: String, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        let limit = self.gpu.device.limits().max_texture_dimension_2d;
        ensure!(
            width > 0 && height > 0 && width <= limit && height <= limit,
            "texture exceeds device dimensions"
        );
        let bytes = u64::from(width) * u64::from(height) * 4;
        ensure!(bytes == rgba.len() as u64, "invalid RGBA length");
        ensure!(
            self.textures.values().map(|t| t.bytes).sum::<u64>() + bytes <= 256 * 1024 * 1024,
            "GPU texture cache exceeds 256 MiB"
        );
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&key),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.gpu.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&key),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        self.textures.insert(
            key,
            Texture {
                bind,
                view,
                _texture: texture,
                bytes,
            },
        );
        Ok(())
    }
    fn puppet_size(p: &Primitive) -> [u32; 2] {
        [
            p.size[0].clamp(1, puppet::MAX_TARGET),
            p.size[1].clamp(1, puppet::MAX_TARGET),
        ]
    }
    /// An empty sRGB texture the GPU renders into (puppet layers).
    fn render_texture(&mut self, key: String, size: [u32; 2]) -> Result<()> {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        ensure!(
            self.textures.values().map(|t| t.bytes).sum::<u64>() + bytes <= 256 * 1024 * 1024,
            "GPU texture cache exceeds 256 MiB"
        );
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&key),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&key),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        self.textures.insert(
            key,
            Texture {
                bind,
                view,
                _texture: texture,
                bytes,
            },
        );
        Ok(())
    }
    /// Texture-cache key of an image asset (dynamic surfaces carry their revision).
    fn image_key(assets: &Assets, owner: u32, asset: &str) -> Result<String> {
        Ok(match assets.surface(ClientId(owner), asset) {
            Some(s) => format!(
                "surface:{}@{}",
                assets.key(ClientId(owner), asset)?,
                s.revision
            ),
            None => format!("image:{}", assets.key(ClientId(owner), asset)?),
        })
    }
    fn ensure_image(
        &mut self,
        assets: &mut Assets,
        owner: u32,
        asset: &str,
        key: &str,
    ) -> Result<()> {
        if self.textures.contains_key(key) {
            return Ok(());
        }
        if key.starts_with("surface:") {
            let s = assets
                .surface(ClientId(owner), asset)
                .context("surface vanished")?
                .clone();
            return self.upload(key.to_owned(), s.width, s.height, &s.rgba);
        }
        let bytes = assets.read_for(ClientId(owner), asset)?;
        let image = assets.decode_image(ClientId(owner), asset, bytes.as_ref())?;
        self.upload(key.to_owned(), image.width, image.height, &image.rgba)
    }
    fn prepare(
        &mut self,
        scene: &SceneState,
        assets: &mut Assets,
        canvas: [u32; 2],
    ) -> Result<(Vec<Vertex2d>, Vec<Draw>)> {
        let mut frame = self.engine_frame.clone();
        for (id, node) in scene.draw_order() {
            let (r, paint) = match &node.visual {
                Visual::Sprite(s) => (
                    s.bounds,
                    Paint::Image {
                        asset: s.asset.relative().into(),
                        tint: s.tint.0,
                    },
                ),
                Visual::Text(t) => (
                    t.bounds,
                    Paint::Text {
                        text: t.text.clone(),
                        font: t.font.clone(),
                        size: t.size_px,
                        color: t.color.0,
                        layout: t.layout_profile.clone(),
                    },
                ),
            };
            frame.push(rectangle(
                id.client.0,
                id.local,
                0,
                [r.x as f32, r.y as f32, r.width as f32, r.height as f32],
                paint,
                None,
                [node.z, 0],
            ));
        }
        frame.sort_by_key(|p| (p.order, p.owner, p.id, p.sub));
        // Retained runs remain logical engine state; only extraction expands
        // them to independent reusable glyph resources and transformed quads.
        let frame = self.expand_glyph_runs(frame, assets)?;
        let text = |p: &Primitive| -> Option<TextRun> {
            if let Paint::Text {
                text,
                font,
                size,
                color,
                layout,
            } = &p.paint
            {
                Some(TextRun {
                    text: text.clone(),
                    font: font.clone(),
                    size_px: *size,
                    bounds: Rect {
                        x: 0,
                        y: 0,
                        width: p.size[0],
                        height: p.size[1],
                    },
                    color: Color(*color),
                    layout_profile: layout.clone(),
                })
            } else {
                None
            }
        };
        let keys = frame
            .iter()
            .map(|p| match &p.paint {
                Paint::Solid { .. } => Ok("solid:white".to_string()),
                Paint::Image { asset, .. } => match assets.surface(ClientId(p.owner), asset) {
                    // Dynamic surfaces re-upload when their revision changes.
                    Some(s) => Ok(format!(
                        "surface:{}@{}",
                        assets.key(ClientId(p.owner), asset)?,
                        s.revision
                    )),
                    None => Ok(format!("image:{}", assets.key(ClientId(p.owner), asset)?)),
                },
                Paint::Text { .. } => assets.text.key(&text(p).expect("text paint")),
                Paint::GlyphRun { .. } => glyph_run::key(assets, &p.paint),
                Paint::Puppet { .. } => {
                    let [w, h] = Self::puppet_size(p);
                    Ok(format!("puppet:{}:{}:{w}x{h}", p.owner, p.id))
                }
            })
            .collect::<Result<Vec<_>>>()?;
        // Layer filters: mask / tone-curve textures (white and identity by default).
        let filter_keys = frame
            .iter()
            .map(|p| {
                let Some(f) = &p.filter else { return Ok(None) };
                let mask = match &f.mask {
                    Some(m) => Self::image_key(assets, p.owner, &m.asset)?,
                    None => "solid:white".to_string(),
                };
                let curve = match &f.tone_curve {
                    Some(t) => Self::image_key(assets, p.owner, &t.asset)?,
                    None => "curve:identity".to_string(),
                };
                Ok(Some((mask, curve)))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut used: BTreeSet<String> = keys.iter().cloned().collect();
        let mut puppet_textures: Vec<Vec<String>> = Vec::new();
        for p in &frame {
            let names = match (&p.paint, &p.puppet) {
                (Paint::Puppet { .. }, Some(f)) => f
                    .textures
                    .iter()
                    .map(|t| Self::image_key(assets, p.owner, t))
                    .collect::<Result<Vec<_>>>()?,
                _ => Vec::new(),
            };
            used.extend(names.iter().cloned());
            puppet_textures.push(names);
        }
        self.puppets.retain(
            &keys
                .iter()
                .filter(|k| k.starts_with("puppet:"))
                .cloned()
                .collect(),
        );
        for (m, c) in filter_keys.iter().flatten() {
            used.insert(m.clone());
            used.insert(c.clone());
        }
        self.textures.retain(|key, _| used.contains(key.as_str()));
        self.glyph_cells.retain(|key, _| used.contains(key));
        if filter_keys.iter().any(Option::is_some) {
            if !self.textures.contains_key("solid:white") {
                self.upload("solid:white".into(), 1, 1, &[255; 4])?;
            }
            if !self.textures.contains_key("curve:identity") {
                let ramp: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v, 255]).collect();
                self.upload("curve:identity".into(), 256, 1, &ramp)?;
            }
            for (p, fk) in frame.iter().zip(&filter_keys) {
                let (Some(f), Some((mk, ck))) = (&p.filter, fk) else {
                    continue;
                };
                if let Some(m) = &f.mask {
                    self.ensure_image(assets, p.owner, &m.asset, mk)?;
                }
                if let Some(t) = &f.tone_curve {
                    self.ensure_image(assets, p.owner, &t.asset, ck)?;
                }
            }
        }
        let mut records = vec![FilterGpu {
            a: [0.; 4],
            b: [0.; 4],
            c: [0.; 4],
            d: [0.; 4],
            e: [0.; 4],
        }];
        let mut vertices = Vec::with_capacity(frame.len() * 6);
        let mut draws = Vec::new();
        for (((p, key), fkeys), tex_keys) in
            frame.iter().zip(keys).zip(filter_keys).zip(puppet_textures)
        {
            let color = match &p.paint {
                Paint::Solid { color } => {
                    if !self.textures.contains_key(&key) {
                        self.upload(key.clone(), 1, 1, &[255; 4])?;
                    }
                    *color
                }
                Paint::Image { asset, tint } => {
                    if !self.textures.contains_key(&key) && key.starts_with("surface:") {
                        let s = assets
                            .surface(ClientId(p.owner), asset)
                            .context("surface vanished")?
                            .clone();
                        self.upload(key.clone(), s.width, s.height, &s.rgba)?;
                    }
                    if !self.textures.contains_key(&key) {
                        let bytes = assets.read_for(ClientId(p.owner), asset)?;
                        let image =
                            assets.decode_image(ClientId(p.owner), asset, bytes.as_ref())?;
                        self.upload(key.clone(), image.width, image.height, &image.rgba)?;
                    }
                    *tint
                }
                Paint::Puppet { tint, .. } => {
                    let size = Self::puppet_size(p);
                    if !self.textures.contains_key(&key) {
                        self.render_texture(key.clone(), size)?;
                    }
                    if let Some(f) = &p.puppet {
                        for (t, k) in f.textures.iter().zip(&tex_keys) {
                            self.ensure_image(assets, p.owner, t, k)?;
                        }
                    }
                    let textures = &self.textures;
                    self.puppets.render(
                        &self.gpu.device,
                        &self.gpu.queue,
                        &self.texture_layout,
                        &self.sampler,
                        &key,
                        size,
                        p.puppet.as_deref(),
                        &|i| {
                            tex_keys
                                .get(i as usize)
                                .and_then(|k| textures.get(k))
                                .map(|t| &t.bind)
                        },
                        &textures[&key].view,
                    )?;
                    *tint
                }
                Paint::Text { color, .. } => {
                    if !self.textures.contains_key(&key) {
                        let run = text(p).expect("text paint");
                        let pixels = assets.text.rasterize(&run)?;
                        self.upload(key.clone(), p.size[0], p.size[1], &pixels)?;
                    }
                    *color
                }
                Paint::GlyphRun { color, .. } => {
                    if !self.textures.contains_key(&key) {
                        let cell = self
                            .glyph_cells
                            .get(&key)
                            .context("glyph resource vanished")?;
                        let (width, height, rgba) = (cell.width, cell.height, cell.rgba.clone());
                        self.upload(key.clone(), width, height, &rgba)?;
                    }
                    *color
                }
            };
            let start = vertices.len() as u32;
            let mut quad = geometry::layer_quad(p, canvas, color);
            if let Some(f) = &p.filter {
                let index = records.len() as f32;
                records.push(filter_gpu(f));
                for v in &mut quad {
                    v.filter = index;
                }
            }
            vertices.extend_from_slice(&quad);
            draws.push(Draw {
                key,
                range: start..start + 6,
                clip: p.clip,
                blend: p.blend,
                filter: fkeys,
            });
        }
        // Upload this frame's filter records and (re)build mask/curve bind groups.
        self.filter_binds.clear();
        if records.len() > 1 {
            let bytes: &[u8] = bytemuck::cast_slice(&records);
            if self
                .filter_buffer
                .as_ref()
                .map_or(true, |b| b.size() < bytes.len() as u64)
            {
                self.filter_buffer = Some(self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("layer filters"),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            let buffer = self.filter_buffer.as_ref().unwrap();
            self.gpu.queue.write_buffer(buffer, 0, bytes);
            for d in &draws {
                let Some((mk, ck)) = &d.filter else { continue };
                if self.filter_binds.contains_key(&(mk.clone(), ck.clone())) {
                    continue;
                }
                let bind = self
                    .gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("layer filter"),
                        layout: &self.filter_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(
                                    &self.textures[mk].view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(
                                    &self.textures[ck].view,
                                ),
                            },
                        ],
                    });
                self.filter_binds.insert((mk.clone(), ck.clone()), bind);
            }
        }
        Ok((vertices, draws))
    }
    fn depth(&mut self, size: [u32; 2]) {
        if self.depth.as_ref().is_some_and(|d| d.size == size) {
            return;
        }
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.depth = Some(Depth {
            size,
            _texture: texture,
            view,
        });
    }
    pub fn render(
        &mut self,
        scene: &SceneState,
        assets: &mut Assets,
        target: RenderTarget<'_>,
        canvas: [u32; 2],
        background: [u8; 4],
    ) -> Result<()> {
        ensure!(
            target.size.iter().all(|n| *n > 0) && canvas.iter().all(|n| *n > 0),
            "zero-size render target"
        );
        ensure!(
            target
                .size
                .iter()
                .all(|n| *n <= self.gpu.device.limits().max_texture_dimension_2d),
            "render target too large"
        );
        let (vertices, draws) = self.prepare(scene, assets, canvas)?;
        self.sync_transition_mask(assets)?;
        self.depth(target.size);
        if let Some(s) = &self.scene3d {
            self.gpu.queue.write_buffer(
                &self.camera_buffer,
                0,
                bytemuck::bytes_of(&geometry::camera(s, canvas)),
            );
        }
        let quad_buffer = if vertices.is_empty() {
            None
        } else {
            Some(
                self.gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("2D frame"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };
        let vp = Viewport::fit(canvas, target.size);
        let color = geometry::tint(background);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let pbr_active = self.pbr_active();
        // Compose into a retained frame; `transitions.finish` presents it.
        let frame_view = self.transitions.begin(&self.gpu.device, target.size);
        ensure!(
            !(pbr_active && self.path != RenderingPath::Raster),
            "the PBR scene renders on the raster path only; compute ray tracing of PBR materials is not implemented"
        );
        let ray_texture = if self.path == RenderingPath::ComputeRaytrace {
            Some(
                self.raytracer
                    .as_ref()
                    .context("ray tracer missing")?
                    .encode(
                        &self.gpu,
                        self.scene3d
                            .as_ref()
                            .context("compute raytrace requires a 3D scene")?,
                        canvas,
                        background,
                        &mut encoder,
                    )?,
            )
        } else {
            None
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D/background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: color[0] as f64,
                            g: color[1] as f64,
                            b: color[2] as f64,
                            a: color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().context("depth target")?.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(buffer) = self
                .mesh_buffer
                .as_ref()
                .filter(|_| self.path == RenderingPath::Raster && !pbr_active)
            {
                pass.set_viewport(vp.x, vp.y, vp.width, vp.height, 0., 1.);
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_bind_group(0, &self.camera_bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..self.mesh_count, 0..1);
            }
        }
        if pbr_active {
            self.pbr.as_mut().unwrap().encode(
                assets,
                &mut encoder,
                frame_view,
                canvas,
                [vp.x, vp.y, vp.width, vp.height],
            )?;
        }
        if let Some(texture) = ray_texture {
            let view = texture.create_view(&Default::default());
            let bind = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("raytrace composite"),
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
            let vertices = geometry::quad(
                crysirror_core::state::Rect {
                    x: 0,
                    y: 0,
                    width: canvas[0],
                    height: canvas[1],
                },
                canvas,
                [255; 4],
            );
            let buffer = self
                .gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("raytrace fullscreen quad"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raytrace composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport(vp.x, vp.y, vp.width, vp.height, 0., 1.);
            pass.set_pipeline(&self.ray_composite_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
        if let Some(buffer) = &quad_buffer {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("2D overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport(vp.x, vp.y, vp.width, vp.height, 0., 1.);

            pass.set_vertex_buffer(0, buffer.slice(..));
            for draw in &draws {
                let c = draw
                    .clip
                    .unwrap_or([0., 0., canvas[0] as f32, canvas[1] as f32]);
                let x = (vp.x + c[0] * vp.width / canvas[0] as f32)
                    .floor()
                    .clamp(0., target.size[0] as f32) as u32;
                let y = (vp.y + c[1] * vp.height / canvas[1] as f32)
                    .floor()
                    .clamp(0., target.size[1] as f32) as u32;
                let right = (vp.x + (c[0] + c[2]) * vp.width / canvas[0] as f32)
                    .ceil()
                    .clamp(0., target.size[0] as f32) as u32;
                let bottom = (vp.y + (c[1] + c[3]) * vp.height / canvas[1] as f32)
                    .ceil()
                    .clamp(0., target.size[1] as f32) as u32;
                if right <= x || bottom <= y {
                    continue;
                }
                pass.set_scissor_rect(x, y, right - x, bottom - y);
                let blend_index = match draw.blend {
                    Blend::Normal => 0,
                    Blend::Add => 1,
                    Blend::Subtract => 2,
                    Blend::Multiply => 3,
                    Blend::Screen => 4,
                };
                match &draw.filter {
                    Some(k) => {
                        pass.set_pipeline(&self.filtered_pipelines[blend_index]);
                        pass.set_bind_group(
                            1,
                            self.filter_binds.get(k).context("filter bind group")?,
                            &[],
                        );
                    }
                    None => pass.set_pipeline(&self.quad_pipelines[blend_index]),
                }
                pass.set_bind_group(
                    0,
                    &self
                        .textures
                        .get(&draw.key)
                        .context("prepared texture disappeared")?
                        .bind,
                    &[],
                );
                pass.draw(draw.range.clone(), 0..1);
            }
        }
        self.transitions.finish(
            &self.gpu.device,
            &mut encoder,
            target.view,
            self.transition.as_ref(),
            canvas,
        );
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some(p) = &mut self.pbr {
            p.after_submit();
        }
        Ok(())
    }
    /// Renders through the same GPU pipelines, then maps a padded COPY_SRC readback.
    /// No fake/software-only screenshot is substituted when GPU initialization fails.
    pub fn capture(
        &mut self,
        scene: &SceneState,
        assets: &mut Assets,
        canvas: [u32; 2],
        background: [u8; 4],
    ) -> Result<image::RgbaImage> {
        ensure!(
            self.format == wgpu::TextureFormat::Rgba8UnormSrgb,
            "capture renderer must use RGBA8 sRGB"
        );
        let (width, height) = (canvas[0], canvas[1]);
        ensure!(
            width > 0 && height > 0 && width <= 8192 && height <= 8192,
            "capture size"
        );
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        // TAA needs history: a still capture accumulates jittered frames.
        let passes = self
            .pbr
            .as_ref()
            .filter(|p| p.active())
            .map_or(1, |p| p.warmup_frames());
        for _ in 0..passes {
            self.render(
                scene,
                assets,
                RenderTarget {
                    view: &view,
                    size: canvas,
                },
                canvas,
                background,
            )?;
        }
        let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.gpu.device.poll(wgpu::Maintain::Wait);
        rx.recv().context("GPU map callback missing")??;
        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in data.chunks_exact(stride as usize).take(height as usize) {
            pixels.extend_from_slice(&row[..(width * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        image::RgbaImage::from_raw(width, height, pixels).context("invalid capture dimensions")
    }
}

pub mod effect;
