// SPDX-License-Identifier: MPL-2.0
//! Puppet layers (ADR 0029): a [`PuppetFrame`] is drawn into a premultiplied
//! HDR target with stencil masks, then resolved to straight alpha into the
//! layer's texture, which the 2D compositor treats like an image.
use bytemuck::{Pod, Zeroable};
use crysirror_puppet::{PartBlend, PuppetFrame};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

const PREMUL: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const STENCIL: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
pub const MAX_TARGET: u32 = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Stencil {
    None,
    Inside,
    Outside,
}

struct Target {
    size: [u32; 2],
    premul: wgpu::TextureView,
    premul_bind: wgpu::BindGroup,
    stencil: wgpu::TextureView,
}

pub struct PuppetRenderer {
    parts: HashMap<(PartBlend, Stencil), wgpu::RenderPipeline>,
    mask: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    targets: HashMap<String, Target>,
}

fn blend_state(b: PartBlend) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    let keep_alpha = C { src_factor: F::Zero, dst_factor: F::One, operation: O::Add };
    let over = C { src_factor: F::One, dst_factor: F::OneMinusSrcAlpha, operation: O::Add };
    match b {
        PartBlend::Normal => wgpu::BlendState { color: over, alpha: over },
        PartBlend::Add => wgpu::BlendState { color: C { src_factor: F::One, dst_factor: F::One, operation: O::Add }, alpha: keep_alpha },
        PartBlend::Subtract => wgpu::BlendState {
            color: C { src_factor: F::One, dst_factor: F::One, operation: O::ReverseSubtract },
            alpha: keep_alpha,
        },
        // Premultiplied multiply: src·dst + dst·(1 − a).
        PartBlend::Multiply => wgpu::BlendState { color: C { src_factor: F::Dst, dst_factor: F::OneMinusSrcAlpha, operation: O::Add }, alpha: keep_alpha },
        PartBlend::Screen => wgpu::BlendState { color: C { src_factor: F::OneMinusDst, dst_factor: F::One, operation: O::Add }, alpha: keep_alpha },
    }
}

impl PuppetRenderer {
    pub fn new(d: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout, output: wgpu::TextureFormat) -> Self {
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("puppet"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/puppet.wgsl").into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("puppet"),
            bind_group_layouts: &[texture_layout],
            push_constant_ranges: &[],
        });
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
        };
        let face = |compare, pass_op| wgpu::StencilFaceState { compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op };
        let depth = |compare, pass_op| wgpu::DepthStencilState {
            format: STENCIL,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState { front: face(compare, pass_op), back: face(compare, pass_op), read_mask: 0xff, write_mask: 0xff },
            bias: Default::default(),
        };
        let pipeline = |label: &str, fs: &str, target: wgpu::ColorTargetState, ds: wgpu::DepthStencilState| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: "vs_part", buffers: &[vertex.clone()], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: fs, targets: &[Some(target)], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(ds),
                multisample: Default::default(),
                multiview: None,
            })
        };
        let mut parts = HashMap::new();
        for b in [PartBlend::Normal, PartBlend::Add, PartBlend::Subtract, PartBlend::Multiply, PartBlend::Screen] {
            for (s, compare) in [
                (Stencil::None, wgpu::CompareFunction::Always),
                (Stencil::Inside, wgpu::CompareFunction::Equal),
                (Stencil::Outside, wgpu::CompareFunction::NotEqual),
            ] {
                let target = wgpu::ColorTargetState { format: PREMUL, blend: Some(blend_state(b)), write_mask: wgpu::ColorWrites::ALL };
                parts.insert((b, s), pipeline("puppet part", "fs_part", target, depth(compare, wgpu::StencilOperation::Keep)));
            }
        }
        let mask = pipeline(
            "puppet mask",
            "fs_mask",
            wgpu::ColorTargetState { format: PREMUL, blend: None, write_mask: wgpu::ColorWrites::empty() },
            depth(wgpu::CompareFunction::Always, wgpu::StencilOperation::Replace),
        );
        let resolve = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("puppet resolve"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs_full", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: "fs_unpremultiply", targets: &[Some(output.into())], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });
        Self { parts, mask, resolve, targets: HashMap::new() }
    }

    /// Drops targets of puppet layers that no longer exist.
    pub fn retain(&mut self, keys: &std::collections::BTreeSet<String>) {
        self.targets.retain(|k, _| keys.contains(k));
    }

    /// Renders `frame` fitted (aspect preserved, centred) into `output`.
    /// `bind(i)` is the bind group of the frame's texture `i`.
    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        &mut self,
        d: &wgpu::Device,
        q: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        key: &str,
        size: [u32; 2],
        frame: Option<&PuppetFrame>,
        bind: &dyn Fn(u32) -> Option<&'a wgpu::BindGroup>,
        output: &wgpu::TextureView,
    ) -> anyhow::Result<()> {
        let target = match self.targets.get(key) {
            Some(t) if t.size == size => t,
            _ => {
                let tex = |label, format, usage| {
                    d.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                };
                let premul = tex("puppet premultiplied", PREMUL, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
                    .create_view(&Default::default());
                let stencil = tex("puppet stencil", STENCIL, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
                let premul_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("puppet premultiplied"),
                    layout: texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&premul) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                    ],
                });
                self.targets.insert(key.to_owned(), Target { size, premul, premul_bind, stencil });
                &self.targets[key]
            }
        };
        // Fit the puppet canvas into the target.
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut ranges = Vec::new();
        if let Some(f) = frame {
            let s = (size[0] as f32 / f.size[0]).min(size[1] as f32 / f.size[1]);
            let ox = (size[0] as f32 - f.size[0] * s) * 0.5;
            let oy = (size[1] as f32 - f.size[1] * s) * 0.5;
            for p in &f.parts {
                let base = vertices.len() as u32;
                let start = indices.len() as u32;
                for i in 0..p.positions.len() {
                    let [x, y] = p.positions[i];
                    vertices.push(Vertex {
                        position: [(x * s + ox) / size[0] as f32 * 2. - 1., 1. - (y * s + oy) / size[1] as f32 * 2.],
                        uv: p.uvs[i],
                        color: p.colors[i],
                    });
                }
                indices.extend(p.indices.iter().map(|i| base + i));
                ranges.push(start..indices.len() as u32);
            }
        }
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("puppet") });
        let vb = (!vertices.is_empty()).then(|| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("puppet vertices"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX })
        });
        let ib = (!indices.is_empty()).then(|| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("puppet indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX })
        });
        // Each masked part gets its own stencil reference; a new pass with a
        // cleared stencil starts when the 8-bit references run out.
        let mut next_part = 0usize;
        let parts = frame.map_or(&[][..], |f| &f.parts[..]);
        let mut first = true;
        loop {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("puppet parts"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.premul,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if first { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.stencil,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Store }),
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            first = false;
            if let (Some(vb), Some(ib)) = (&vb, &ib) {
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            }
            let mut reference = 0u32;
            let mut restart = false;
            while next_part < parts.len() {
                let p = &parts[next_part];
                if !p.visible {
                    next_part += 1;
                    continue;
                }
                let Some(tex) = bind(p.texture) else {
                    anyhow::bail!("puppet texture {} not loaded", p.texture);
                };
                let mode = if p.masks.is_empty() {
                    Stencil::None
                } else {
                    if reference == 255 {
                        restart = true;
                        break;
                    }
                    reference += 1;
                    pass.set_stencil_reference(reference);
                    pass.set_pipeline(&self.mask);
                    for m in &p.masks {
                        let mp = &parts[*m];
                        let Some(mt) = bind(mp.texture) else { anyhow::bail!("puppet mask texture {} not loaded", mp.texture) };
                        pass.set_bind_group(0, mt, &[]);
                        pass.draw_indexed(ranges[*m].clone(), 0, 0..1);
                    }
                    if p.invert_mask { Stencil::Outside } else { Stencil::Inside }
                };
                pass.set_pipeline(&self.parts[&(p.blend, mode)]);
                pass.set_bind_group(0, tex, &[]);
                pass.draw_indexed(ranges[next_part].clone(), 0, 0..1);
                next_part += 1;
            }
            if !restart {
                break;
            }
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("puppet resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &target.premul_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        q.submit(Some(enc.finish()));
        Ok(())
    }
}
