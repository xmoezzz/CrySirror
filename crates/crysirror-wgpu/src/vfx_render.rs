// SPDX-License-Identifier: MPL-2.0
//! VFX rendering only. Simulation and native timing never live in this module.
//! CPU frames and GPU-resident state share the same storage layout and shaders.
use crate::{vfx_gpu::GpuParticle, Gpu};
use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use crysirror_core::state::ClientId;
use crysirror_runtime::assets::Assets;
use crysirror_vfx::{
    BlendMode, EmitterFrame, FacingMode, FrameSnapshot, MeshShape, Particle, RenderSpec,
    SimulationSpace,
};
use glam::{Mat4, Vec3};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use wgpu::util::DeviceExt;

pub const MAX_RENDER_PARTICLES: usize = 200_000;
pub const MAX_RENDER_VERTICES: usize = 1_200_000;
const TEXTURE_BUDGET: u64 = 128 * 1024 * 1024;
const MESH_BUDGET: u64 = 64 * 1024 * 1024;
type Key = (u64, String);

/// Matrices must describe the same jittered view as the supplied depth prepass.
#[derive(Clone, Debug)]
pub struct VfxView {
    pub view_projection: Mat4,
    pub view: Mat4,
    pub camera: Vec3,
    pub viewport: [u32; 2],
    pub time: f32,
    pub fog_color: [f32; 3],
    /// density, height falloff, base height, start distance.
    pub fog: [f32; 4],
}
impl Default for VfxView {
    fn default() -> Self {
        Self {
            view_projection: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
            camera: Vec3::ZERO,
            viewport: [1, 1],
            time: 0.,
            fog_color: [0.; 3],
            fog: [0.; 4],
        }
    }
}

/// Buffer authority comes from the matching, owner-produced emitter frame.
/// The first sixteen bytes of indirect are ordinary DrawIndirectArgs.
pub struct GpuEmitterDraw<'a> {
    pub emitter: &'a EmitterFrame,
    pub particles: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub indirect: &'a wgpu::Buffer,
    pub capacity: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RenderStats {
    pub cpu_particles_submitted: usize,
    pub cpu_particles_drawn: usize,
    pub cpu_particles_culled: usize,
    pub gpu_emitters: usize,
    /// Capacity is not an observation of the resident live count.
    pub gpu_capacity: usize,
    pub gpu_live_particles: Option<usize>,
    pub sprite_batches: usize,
    pub mesh_batches: usize,
    pub ribbon_batches: usize,
    pub ribbon_segments: usize,
    pub gpu_sorted_emitters: usize,
    pub gpu_culled_emitters: usize,
    pub draw_calls: usize,
    pub resources_pending: usize,
    pub texture_bytes: u64,
    pub mesh_bytes: u64,
    pub particle_buffer_bytes: u64,
    /// Per-emitter sorting followed by emitter distance ordering, not OIT.
    pub unified_scene_transparency: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera: [f32; 4],
    mode: [u32; 4],
    animation: [f32; 4],
    axis: [f32; 4],
    screen: [f32; 4],
    fog_color: [f32; 4],
    fog_params: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
    uv: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RibbonVertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SortUniform {
    model: [[f32; 4]; 4],
    camera: [f32; 4],
    pass: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullUniform {
    view_projection: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    options: [f32; 4],
}

struct Image {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes: u64,
    size: [u32; 2],
    // Keep source identity alive while its GPU resource is cached.
    _source: Option<Arc<crysirror_image::ImageSurface>>,
}
struct Geometry {
    buffer: wgpu::Buffer,
    vertices: u32,
    radius: f32,
    bytes: u64,
    _source: Option<Arc<crysirror_asset::pbr::PbrModel>>,
}
struct CpuBuffers {
    particles: wgpu::Buffer,
    indices: wgpu::Buffer,
    capacity: usize,
}
struct SortBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
}
struct VisibleBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
    index_offset: u64,
}
enum Shape {
    Sprite,
    Mesh(String),
    Ribbon(wgpu::Buffer, u32),
}
struct Batch {
    view: wgpu::BindGroup,
    material: wgpu::BindGroup,
    shape: Shape,
    blend: BlendMode,
    count: u32,
    indirect: Option<wgpu::Buffer>,
    distance: f32,
}

/// A submitted frame owns bind-group references; caches retain only used data.
pub struct VfxRenderer {
    particle_layout: wgpu::BindGroupLayout,
    ribbon_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::RenderPipeline>,
    sort_layout: wgpu::BindGroupLayout,
    sort_initialize: wgpu::ComputePipeline,
    sort_step: wgpu::ComputePipeline,
    cull_layout: wgpu::BindGroupLayout,
    cull_initialize: wgpu::ComputePipeline,
    cull_compact: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    images: BTreeMap<String, Image>,
    meshes: BTreeMap<String, Geometry>,
    cpu: BTreeMap<Key, CpuBuffers>,
    sorted: BTreeMap<Key, SortBuffer>,
    visible: BTreeMap<Key, VisibleBuffer>,
    wanted_images: BTreeSet<String>,
    wanted_meshes: BTreeSet<String>,
    wanted_emitters: BTreeSet<Key>,
    wanted_gpu_emitters: BTreeSet<Key>,
    batches: Vec<Batch>,
    view: VfxView,
    stats: RenderStats,
    epoch: Option<u64>,
    store: Option<u64>,
}

fn storage(
    binding: u32,
    visibility: wgpu::ShaderStages,
    read_only: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn uniform(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn blend_index(blend: BlendMode) -> usize {
    match blend {
        BlendMode::Alpha => 0,
        BlendMode::Additive => 1,
        BlendMode::Premultiplied => 2,
        BlendMode::Multiply => 3,
    }
}
fn is_sorted(blend: BlendMode) -> bool {
    matches!(blend, BlendMode::Alpha | BlendMode::Premultiplied)
}
fn blend_state(blend: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    match blend {
        BlendMode::Alpha => wgpu::BlendState::ALPHA_BLENDING,
        BlendMode::Premultiplied => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        BlendMode::Additive => wgpu::BlendState {
            color: C {
                src_factor: F::One,
                dst_factor: F::One,
                operation: O::Add,
            },
            alpha: C {
                src_factor: F::Zero,
                dst_factor: F::One,
                operation: O::Add,
            },
        },
        BlendMode::Multiply => wgpu::BlendState {
            color: C {
                src_factor: F::Dst,
                dst_factor: F::Zero,
                operation: O::Add,
            },
            alpha: C {
                src_factor: F::Zero,
                dst_factor: F::One,
                operation: O::Add,
            },
        },
    }
}

impl VfxRenderer {
    pub fn new(d: &wgpu::Device, color: wgpu::TextureFormat, samples: u32) -> Result<Self> {
        ensure!(
            [1, 4].contains(&samples),
            "VFX renderer supports single-sample or 4x MSAA targets"
        );
        ensure!(
            d.limits().max_storage_buffers_per_shader_stage >= 4,
            "VFX alpha sorting requires four storage bindings"
        );
        ensure!(
            d.limits().max_compute_workgroup_size_x >= 256
                && d.limits().max_compute_invocations_per_workgroup >= 256,
            "VFX view culling/sorting requires a 256-invocation compute workgroup"
        );
        let particle_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX particle view"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage(1, wgpu::ShaderStages::VERTEX, true),
                storage(2, wgpu::ShaderStages::VERTEX, true),
            ],
        });
        let ribbon_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX ribbon view"),
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let material_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX material"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VFX shared render"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vfx_render.wgsl").into()),
        });
        let mut pipelines = Vec::new();
        const MESH_ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2];
        const RIBBON_ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4];
        for kind in 0..3 {
            let entry = ["vs_sprite", "vs_mesh", "vs_ribbon"][kind];
            let buffers = match kind {
                1 => vec![wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_ATTRS,
                }],
                2 => vec![wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RibbonVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &RIBBON_ATTRS,
                }],
                _ => vec![],
            };
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("VFX render layout"),
                bind_group_layouts: &[
                    Some(if kind == 2 {
                        &ribbon_layout
                    } else {
                        &particle_layout
                    }),
                    Some(&material_layout),
                ],
                immediate_size: 0,
            });
            for blend in [
                BlendMode::Alpha,
                BlendMode::Additive,
                BlendMode::Premultiplied,
                BlendMode::Multiply,
            ] {
                let fragment = ["fs_alpha", "fs_additive", "fs_premultiplied", "fs_multiply"]
                    [blend_index(blend)];
                pipelines.push(d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("VFX draw"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(entry),
                        buffers: &buffers,
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: color,
                            blend: Some(blend_state(blend)),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        cull_mode: if kind == 1 {
                            Some(wgpu::Face::Back)
                        } else {
                            None
                        },
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        ..Default::default()
                    },
                    multiview_mask: None,
                    cache: None,
                }));
            }
        }
        let sort_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX alpha sort"),
            entries: &[
                uniform(0, wgpu::ShaderStages::COMPUTE),
                storage(1, wgpu::ShaderStages::COMPUTE, true),
                storage(2, wgpu::ShaderStages::COMPUTE, false),
                storage(3, wgpu::ShaderStages::COMPUTE, true),
                storage(4, wgpu::ShaderStages::COMPUTE, true),
            ],
        });
        let sort_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VFX sort"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vfx_sort.wgsl").into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VFX sort layout"),
            bind_group_layouts: &[Some(&sort_layout)],
            immediate_size: 0,
        });
        let make = |entry| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("VFX sort"),
                layout: Some(&layout),
                module: &sort_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let sort_initialize = make("initialize");
        let sort_step = make("sort");
        let cull_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VFX per-view culling"),
            entries: &[
                uniform(0, wgpu::ShaderStages::COMPUTE),
                storage(1, wgpu::ShaderStages::COMPUTE, true),
                storage(2, wgpu::ShaderStages::COMPUTE, true),
                storage(3, wgpu::ShaderStages::COMPUTE, true),
                storage(4, wgpu::ShaderStages::COMPUTE, false),
            ],
        });
        let cull_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VFX culling"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vfx_cull.wgsl").into()),
        });
        let cull_pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VFX cull layout"),
            bind_group_layouts: &[Some(&cull_layout)],
            immediate_size: 0,
        });
        let cull_pipeline = |entry| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("VFX visibility"),
                layout: Some(&cull_pipeline_layout),
                module: &cull_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let cull_initialize = cull_pipeline("initialize");
        let cull_compact = cull_pipeline("compact");
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("VFX texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            particle_layout,
            ribbon_layout,
            material_layout,
            pipelines,
            sort_layout,
            sort_initialize,
            sort_step,
            cull_layout,
            cull_initialize,
            cull_compact,
            sampler,
            images: BTreeMap::new(),
            meshes: BTreeMap::new(),
            cpu: BTreeMap::new(),
            sorted: BTreeMap::new(),
            visible: BTreeMap::new(),
            wanted_images: BTreeSet::new(),
            wanted_meshes: BTreeSet::new(),
            wanted_emitters: BTreeSet::new(),
            wanted_gpu_emitters: BTreeSet::new(),
            batches: vec![],
            view: Default::default(),
            stats: Default::default(),
            epoch: None,
            store: None,
        })
    }
    pub fn clear(&mut self) {
        self.batches.clear();
        self.images.clear();
        self.meshes.clear();
        self.cpu.clear();
        self.sorted.clear();
        self.visible.clear();
        self.store = None;
        self.stats = Default::default();
    }
    pub fn begin_frame(&mut self, view: &VfxView, epoch: u64) -> Result<()> {
        ensure!(
            view.viewport.iter().all(|v| *v > 0)
                && view.view_projection.is_finite()
                && view.view.is_finite()
                && view.camera.is_finite()
                && view.time.is_finite(),
            "invalid VFX render view"
        );
        ensure!(
            view.view_projection.determinant().abs() > 1e-12,
            "singular VFX view projection"
        );
        if self.epoch != Some(epoch) {
            self.clear();
            self.epoch = Some(epoch);
        }
        self.view = view.clone();
        self.batches.clear();
        self.wanted_images.clear();
        self.wanted_meshes.clear();
        self.wanted_emitters.clear();
        self.wanted_gpu_emitters.clear();
        self.stats = RenderStats {
            gpu_live_particles: Some(0),
            ..Default::default()
        };
        Ok(())
    }
    fn check_store(&mut self, assets: &Assets) {
        if self.store.is_some_and(|id| id != assets.store_id()) {
            self.clear();
        }
        self.store = Some(assets.store_id());
    }
    pub fn stats(&self) -> &RenderStats {
        &self.stats
    }
    pub fn live(&self) -> usize {
        self.stats.cpu_particles_drawn
    }
    pub fn finish_frame(&mut self) {
        self.images.retain(|k, _| self.wanted_images.contains(k));
        self.meshes.retain(|k, _| self.wanted_meshes.contains(k));
        self.cpu.retain(|k, _| self.wanted_emitters.contains(k));
        self.sorted
            .retain(|k, _| self.wanted_gpu_emitters.contains(k));
        self.visible
            .retain(|k, _| self.wanted_gpu_emitters.contains(k));
        self.stats.texture_bytes = self.images.values().map(|r| r.bytes).sum();
        self.stats.mesh_bytes = self.meshes.values().map(|r| r.bytes).sum();
        self.stats.particle_buffer_bytes = self
            .cpu
            .values()
            .map(|r| r.particles.size() + r.indices.size())
            .sum::<u64>()
            + self.sorted.values().map(|r| r.buffer.size()).sum::<u64>()
            + self.visible.values().map(|r| r.buffer.size()).sum::<u64>();
        self.batches
            .sort_by(|a, b| match (is_sorted(a.blend), is_sorted(b.blend)) {
                (true, true) => b.distance.total_cmp(&a.distance),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            });
    }
    fn image(
        &mut self,
        gpu: &Gpu,
        assets: &mut Assets,
        client: u32,
        path: Option<&str>,
    ) -> Result<Option<String>> {
        let surface = path
            .and_then(|path| assets.surface(ClientId(client), path))
            .cloned();
        let (key, image) = if let Some(surface) = &surface {
            (
                format!("surface:{client}:{}:{}", path.unwrap(), surface.revision),
                None,
            )
        } else if let Some(path) = path {
            let Some(image) = assets.image_for(ClientId(client), path)? else {
                self.stats.resources_pending += 1;
                return Ok(None);
            };
            (
                format!("{client}:{path}:{:p}", Arc::as_ptr(&image)),
                Some(image),
            )
        } else {
            ("white".into(), None)
        };
        self.wanted_images.insert(key.clone());
        if !self.images.contains_key(&key) {
            let (width, height, rgba) = if let Some(surface) = &surface {
                (surface.width, surface.height, surface.rgba.as_ref())
            } else {
                image.as_ref().map_or((1, 1, &[255u8; 4][..]), |i| {
                    (i.width, i.height, i.rgba.as_slice())
                })
            };
            ensure!(
                width > 0
                    && height > 0
                    && width <= gpu.device.limits().max_texture_dimension_2d
                    && height <= gpu.device.limits().max_texture_dimension_2d,
                "VFX texture dimensions exceed device limits"
            );
            let bytes = u64::from(width) * u64::from(height) * 4;
            if self.images.values().map(|r| r.bytes).sum::<u64>() + bytes > TEXTURE_BUDGET {
                self.images.retain(|k, _| self.wanted_images.contains(k));
            }
            ensure!(
                self.images.values().map(|r| r.bytes).sum::<u64>() + bytes <= TEXTURE_BUDGET,
                "VFX texture budget exceeded"
            );
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("VFX shared image resource"),
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
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
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
            self.images.insert(
                key.clone(),
                Image {
                    _texture: texture,
                    view,
                    bytes,
                    size: [width, height],
                    _source: image,
                },
            );
        }
        Ok(Some(key))
    }
    fn geometry(
        &mut self,
        gpu: &Gpu,
        assets: &mut Assets,
        client: u32,
        shape: &MeshShape,
    ) -> Result<Option<String>> {
        let (key, source) = match shape {
            MeshShape::Cube => ("cube".into(), None),
            MeshShape::Quad => ("quad".into(), None),
            MeshShape::Asset { path } => {
                let Some(model) = assets.model_for(ClientId(client), path)? else {
                    self.stats.resources_pending += 1;
                    return Ok(None);
                };
                (
                    format!("{client}:{path}:{:p}", Arc::as_ptr(&model)),
                    Some(model),
                )
            }
        };
        self.wanted_meshes.insert(key.clone());
        if !self.meshes.contains_key(&key) {
            let vertices = if let Some(model) = &source {
                asset_vertices(model)?
            } else {
                builtin_vertices(matches!(shape, MeshShape::Cube))
            };
            ensure!(
                !vertices.is_empty() && vertices.len() <= MAX_RENDER_VERTICES,
                "VFX mesh vertex budget exceeded"
            );
            let bytes = std::mem::size_of_val(vertices.as_slice()) as u64;
            if self.meshes.values().map(|r| r.bytes).sum::<u64>() + bytes > MESH_BUDGET {
                self.meshes.retain(|k, _| self.wanted_meshes.contains(k));
            }
            ensure!(
                self.meshes.values().map(|r| r.bytes).sum::<u64>() + bytes <= MESH_BUDGET,
                "VFX mesh memory budget exceeded"
            );
            let radius = vertices
                .iter()
                .map(|v| Vec3::from(v.position).length())
                .fold(0., f32::max);
            let buffer = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("VFX particle mesh"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            self.meshes.insert(
                key.clone(),
                Geometry {
                    buffer,
                    vertices: vertices.len() as u32,
                    radius,
                    bytes,
                    _source: source,
                },
            );
        }
        Ok(Some(key))
    }
    fn settings<'a>(&self, e: &'a EmitterFrame) -> Result<(Uniform, Option<&'a str>, BlendMode)> {
        let model = if e.space == SimulationSpace::Local {
            Mat4::from_cols_array_2d(&e.transform)
        } else {
            Mat4::IDENTITY
        };
        ensure!(model.is_finite(), "nonfinite VFX transform");
        let mut u = Uniform {
            view_projection: self.view.view_projection.to_cols_array_2d(),
            inverse_view_projection: self.view.view_projection.inverse().to_cols_array_2d(),
            view: self.view.view.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            camera: self.view.camera.extend(1.).to_array(),
            mode: [0, 0, 1, 1],
            animation: [1., 0., 0., 0.],
            axis: [0., 1., 0., 0.],
            screen: [
                self.view.viewport[0] as f32,
                self.view.viewport[1] as f32,
                self.view.time,
                0.,
            ],
            fog_color: [
                self.view.fog_color[0],
                self.view.fog_color[1],
                self.view.fog_color[2],
                1.,
            ],
            fog_params: self.view.fog,
        };
        let (texture, blend) = match &e.render {
            RenderSpec::Sprite {
                texture,
                columns,
                rows,
                frames,
                fps,
                soft_distance,
                blend,
                facing,
            } => {
                ensure!(
                    *columns > 0
                        && *rows > 0
                        && *frames > 0
                        && u64::from(*frames) <= u64::from(*columns) * u64::from(*rows)
                        && fps.is_finite()
                        && *fps >= 0.
                        && soft_distance.is_finite()
                        && *soft_distance >= 0.,
                    "invalid VFX sprite settings"
                );
                ensure!(
                    texture.is_some() || (*columns == 1 && *rows == 1 && *frames == 1),
                    "VFX flipbooks require an image resource"
                );
                u.mode[2] = *columns;
                u.mode[3] = *rows;
                u.animation = [
                    *frames as f32,
                    *fps,
                    *soft_distance,
                    if texture.is_none() { 1. } else { 0. },
                ];
                match facing {
                    FacingMode::Camera => {}
                    FacingMode::Velocity => u.mode[1] = 1,
                    FacingMode::Axis { axis } => {
                        ensure!(
                            Vec3::from(*axis).is_finite()
                                && Vec3::from(*axis).length_squared() > 1e-10,
                            "invalid VFX facing axis"
                        );
                        u.mode[1] = 2;
                        u.axis = [axis[0], axis[1], axis[2], 0.];
                    }
                }
                (texture.as_deref(), *blend)
            }
            RenderSpec::Mesh { texture, blend, .. } => {
                u.mode[0] = 1;
                (texture.as_deref(), *blend)
            }
            RenderSpec::Ribbon {
                texture,
                blend,
                width,
                uv_scale,
                max_segments,
            } => {
                u.mode[0] = 2;
                ensure!(
                    width.is_finite()
                        && *width > 0.
                        && uv_scale.is_finite()
                        && *uv_scale > 0.
                        && *max_segments > 0,
                    "invalid VFX ribbon settings"
                );
                (texture.as_deref(), *blend)
            }
        };
        Ok((u, texture, blend))
    }
    fn material(&self, d: &wgpu::Device, key: &str, depth: &wgpu::TextureView) -> wgpu::BindGroup {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX material bind"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.images[key].view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        })
    }
    fn view_bind(
        &self,
        d: &wgpu::Device,
        u: &Uniform,
        buffers: Option<(&wgpu::Buffer, &wgpu::Buffer)>,
    ) -> wgpu::BindGroup {
        self.view_bind_offset(d, u, buffers.map(|(a, b)| (a, b, 0)))
    }
    fn view_bind_offset(
        &self,
        d: &wgpu::Device,
        u: &Uniform,
        buffers: Option<(&wgpu::Buffer, &wgpu::Buffer, u64)>,
    ) -> wgpu::BindGroup {
        let uniform = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("VFX view parameters"),
            contents: bytemuck::bytes_of(u),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        if let Some((particles, indices, offset)) = buffers {
            entries.push(wgpu::BindGroupEntry {
                binding: 1,
                resource: particles.as_entire_binding(),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: indices,
                    offset,
                    size: None,
                }),
            });
        }
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX view bind"),
            layout: if buffers.is_some() {
                &self.particle_layout
            } else {
                &self.ribbon_layout
            },
            entries: &entries,
        })
    }
    pub fn prepare_cpu(
        &mut self,
        gpu: &Gpu,
        frame: &FrameSnapshot,
        assets: &mut Assets,
        depth: &wgpu::TextureView,
    ) -> Result<()> {
        self.check_store(assets);
        for e in &frame.emitters {
            if e.particles.is_empty() {
                continue;
            }
            self.stats.cpu_particles_submitted = self
                .stats
                .cpu_particles_submitted
                .checked_add(e.particles.len())
                .context("VFX render particle count overflow")?;
            ensure!(
                self.stats.cpu_particles_submitted + self.stats.gpu_capacity
                    <= MAX_RENDER_PARTICLES,
                "VFX global render budget exceeded before allocation"
            );
            let key = (e.key, e.emitter_id.clone());
            self.wanted_emitters.insert(key.clone());
            let (u, path, blend) = self.settings(e)?;
            let path = path.map(str::to_owned);
            let Some(image) = self.image(gpu, assets, e.client, path.as_deref())? else {
                continue;
            };
            ensure!(
                u.mode[0] != 0
                    || u.animation[3] > 0.5
                    || (self.images[&image].size[0] >= u.mode[2]
                        && self.images[&image].size[1] >= u.mode[3]),
                "VFX flipbook cells must contain at least one source texel"
            );
            let material = self.material(&gpu.device, &image, depth);
            let model = Mat4::from_cols_array_2d(&u.model);
            if let RenderSpec::Ribbon {
                width,
                uv_scale,
                max_segments,
                ..
            } = &e.render
            {
                let vertices = ribbon_vertices(
                    &e.particles,
                    model,
                    self.view.camera,
                    *width,
                    *uv_scale,
                    *max_segments,
                )?;
                if vertices.is_empty() {
                    continue;
                }
                let count = vertices.len() as u32;
                self.stats.ribbon_segments += vertices.len() / 6;
                self.stats.ribbon_batches += 1;
                ensure!(
                    self.stats.ribbon_segments * 6 <= MAX_RENDER_VERTICES,
                    "VFX ribbon frame vertex budget exceeded"
                );
                let buffer = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("VFX ribbon geometry"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let center = vertices
                    .iter()
                    .fold(Vec3::ZERO, |a, v| a + Vec3::from(v.position))
                    / vertices.len() as f32;
                self.batches.push(Batch {
                    view: self.view_bind(&gpu.device, &u, None),
                    material,
                    shape: Shape::Ribbon(buffer, count),
                    blend,
                    count: 1,
                    indirect: None,
                    distance: center.distance_squared(self.view.camera),
                });
                self.stats.cpu_particles_drawn += e.particles.len();
                self.stats.draw_calls += 1;
                continue;
            }
            let shape = if let RenderSpec::Mesh { mesh, .. } = &e.render {
                let Some(key) = self.geometry(gpu, assets, e.client, mesh)? else {
                    continue;
                };
                self.stats.mesh_batches += 1;
                Shape::Mesh(key)
            } else {
                self.stats.sprite_batches += 1;
                Shape::Sprite
            };
            let radius = match &shape {
                Shape::Mesh(key) => self.meshes[key].radius,
                _ => 1.,
            };
            let mut visible = e
                .particles
                .iter()
                .filter(|p| particle_visible(p, model, &self.view, radius))
                .collect::<Vec<_>>();
            self.stats.cpu_particles_culled += e.particles.len() - visible.len();
            if is_sorted(blend) {
                visible.sort_by(|a, b| {
                    let da = model
                        .transform_point3(Vec3::from(a.position))
                        .distance_squared(self.view.camera);
                    let db = model
                        .transform_point3(Vec3::from(b.position))
                        .distance_squared(self.view.camera);
                    db.total_cmp(&da).then(a.id.cmp(&b.id))
                });
            }
            if visible.is_empty() {
                continue;
            }
            let particles = visible.iter().map(|p| gpu_particle(p)).collect::<Vec<_>>();
            let count = particles.len();
            if !self.cpu.get(&key).is_some_and(|b| b.capacity >= count) {
                let capacity = count.next_power_of_two();
                let bytes = (capacity * std::mem::size_of::<GpuParticle>()) as u64;
                ensure!(
                    bytes <= gpu.device.limits().max_storage_buffer_binding_size as u64,
                    "VFX particle upload exceeds storage limit"
                );
                self.cpu.insert(
                    key.clone(),
                    CpuBuffers {
                        particles: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("VFX CPU particle storage"),
                            size: bytes,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        indices: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("VFX CPU alive indices"),
                            size: capacity as u64 * 4,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        capacity,
                    },
                );
            }
            let buffers = &self.cpu[&key];
            gpu.queue
                .write_buffer(&buffers.particles, 0, bytemuck::cast_slice(&particles));
            let indices = (0..count as u32).collect::<Vec<_>>();
            gpu.queue
                .write_buffer(&buffers.indices, 0, bytemuck::cast_slice(&indices));
            let view = self.view_bind(
                &gpu.device,
                &u,
                Some((&buffers.particles, &buffers.indices)),
            );
            let center = visible.iter().fold(Vec3::ZERO, |a, p| {
                a + model.transform_point3(Vec3::from(p.position))
            }) / count as f32;
            self.batches.push(Batch {
                view,
                material,
                shape,
                blend,
                count: count as u32,
                indirect: None,
                distance: center.distance_squared(self.view.camera),
            });
            self.stats.cpu_particles_drawn += count;
            self.stats.draw_calls += 1;
        }
        Ok(())
    }
    pub fn prepare_gpu(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        emitters: &[GpuEmitterDraw<'_>],
        assets: &mut Assets,
        depth: &wgpu::TextureView,
    ) -> Result<()> {
        self.check_store(assets);
        for input in emitters {
            let e = input.emitter;
            ensure!(!matches!(e.render,RenderSpec::Ribbon{..}),"GPU ribbons require resident trail history and are not supported; select CPU for this emitter");
            ensure!(
                input.capacity > 0
                    && input.particles.size() >= u64::from(input.capacity) * 96
                    && input.indices.size() >= u64::from(input.capacity) * 4
                    && input.indirect.size() >= 16,
                "VFX GPU draw buffers do not match declared capacity"
            );
            self.stats.gpu_capacity += input.capacity as usize;
            ensure!(
                self.stats.cpu_particles_submitted + self.stats.gpu_capacity
                    <= MAX_RENDER_PARTICLES,
                "VFX global render budget exceeded"
            );
            self.stats.gpu_emitters += 1;
            self.stats.gpu_live_particles = None;
            let key = (e.key, e.emitter_id.clone());
            self.wanted_gpu_emitters.insert(key.clone());
            let (u, path, blend) = self.settings(e)?;
            let path = path.map(str::to_owned);
            let Some(image) = self.image(gpu, assets, e.client, path.as_deref())? else {
                continue;
            };
            ensure!(
                u.mode[0] != 0
                    || u.animation[3] > 0.5
                    || (self.images[&image].size[0] >= u.mode[2]
                        && self.images[&image].size[1] >= u.mode[3]),
                "VFX flipbook cells must contain at least one source texel"
            );
            let shape = if let RenderSpec::Mesh { mesh, .. } = &e.render {
                let Some(key) = self.geometry(gpu, assets, e.client, mesh)? else {
                    continue;
                };
                self.stats.mesh_batches += 1;
                Shape::Mesh(key)
            } else {
                self.stats.sprite_batches += 1;
                Shape::Sprite
            };
            let (radius, vertex_count) = match &shape {
                Shape::Mesh(key) => (self.meshes[key].radius, self.meshes[key].vertices),
                _ => (1., 6),
            };
            self.cull_indices(gpu, encoder, &key, input, &u, radius, vertex_count)?;
            self.stats.gpu_culled_emitters += 1;
            let (indices, index_offset) = if is_sorted(blend) {
                self.sort_indices(gpu, encoder, &key, input, &u)?;
                self.stats.gpu_sorted_emitters += 1;
                (&self.sorted[&key].buffer, 0)
            } else {
                (&self.visible[&key].buffer, self.visible[&key].index_offset)
            };
            let view = self.view_bind_offset(
                &gpu.device,
                &u,
                Some((input.particles, indices, index_offset)),
            );
            let material = self.material(&gpu.device, &image, depth);
            self.batches.push(Batch {
                view,
                material,
                shape,
                blend,
                count: 0,
                indirect: Some(self.visible[&key].buffer.clone()),
                distance: Mat4::from_cols_array_2d(&e.transform)
                    .w_axis
                    .truncate()
                    .distance_squared(self.view.camera),
            });
            self.stats.draw_calls += 1;
        }
        Ok(())
    }
    fn cull_indices(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        key: &Key,
        input: &GpuEmitterDraw<'_>,
        view: &Uniform,
        radius: f32,
        vertex_count: u32,
    ) -> Result<()> {
        if !self
            .visible
            .get(key)
            .is_some_and(|b| b.capacity >= input.capacity)
        {
            let alignment =
                u64::from(gpu.device.limits().min_storage_buffer_offset_alignment).max(4);
            let index_offset = 16u64.div_ceil(alignment) * alignment;
            self.visible.insert(
                key.clone(),
                VisibleBuffer {
                    buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("VFX visible indices and draw arguments"),
                        size: index_offset + u64::from(input.capacity) * 4,
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::INDIRECT
                            | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    capacity: input.capacity,
                    index_offset,
                },
            );
        }
        let output = &self.visible[key];
        let params = CullUniform {
            view_projection: view.view_projection,
            model: view.model,
            options: [
                radius,
                (output.index_offset / 4) as f32,
                input.capacity as f32,
                vertex_count as f32,
            ],
        };
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("VFX cull view parameters"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VFX cull bind"),
            layout: &self.cull_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: input.particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: input.indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: input.indirect.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: output.buffer.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("VFX cull initialize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.cull_initialize);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("VFX visible particle compact"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.cull_compact);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(input.capacity.div_ceil(256), 1, 1);
        }
        Ok(())
    }
    fn sort_indices(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        key: &Key,
        input: &GpuEmitterDraw<'_>,
        view: &Uniform,
    ) -> Result<()> {
        let capacity = input.capacity.next_power_of_two();
        ensure!(capacity <= 262_144, "VFX GPU alpha sort budget exceeded");
        if !self.sorted.get(key).is_some_and(|b| b.capacity >= capacity) {
            self.sorted.insert(
                key.clone(),
                SortBuffer {
                    buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("VFX sorted indices"),
                        size: u64::from(capacity) * 4,
                        usage: wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    }),
                    capacity,
                },
            );
        }
        let output = &self.sorted[key].buffer;
        let encode = |encoder: &mut wgpu::CommandEncoder, j, k, initialize| {
            let u = SortUniform {
                model: view.model,
                camera: view.camera,
                pass: [j, k, capacity, 0],
            };
            let buffer = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("VFX sort stage"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("VFX sort bind"),
                layout: &self.sort_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: input.particles.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.visible[key].buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.visible[key].buffer,
                            offset: self.visible[key].index_offset,
                            size: None,
                        }),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("VFX alpha index sort"),
                timestamp_writes: None,
            });
            pass.set_pipeline(if initialize {
                &self.sort_initialize
            } else {
                &self.sort_step
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(capacity.div_ceil(256), 1, 1);
        };
        encode(encoder, 0, 0, true);
        let mut k = 2;
        while k <= capacity {
            let mut j = k / 2;
            while j > 0 {
                encode(encoder, j, k, false);
                j /= 2;
            }
            k *= 2;
        }
        Ok(())
    }
    /// Number of vertices the simulation must put in DrawIndirectArgs[0].
    pub fn gpu_vertex_count(
        &mut self,
        gpu: &Gpu,
        assets: &mut Assets,
        e: &EmitterFrame,
    ) -> Result<Option<u32>> {
        match &e.render {
            RenderSpec::Sprite { .. } => Ok(Some(6)),
            RenderSpec::Mesh { mesh, .. } => Ok(self
                .geometry(gpu, assets, e.client, mesh)?
                .map(|key| self.meshes[&key].vertices)),
            RenderSpec::Ribbon { .. } => anyhow::bail!("GPU ribbons require trail history"),
        }
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        for batch in &self.batches {
            let kind = match &batch.shape {
                Shape::Sprite => 0,
                Shape::Mesh(_) => 1,
                Shape::Ribbon(_, _) => 2,
            };
            pass.set_pipeline(&self.pipelines[kind * 4 + blend_index(batch.blend)]);
            pass.set_bind_group(0, &batch.view, &[]);
            pass.set_bind_group(1, &batch.material, &[]);
            let vertices = match &batch.shape {
                Shape::Sprite => 6,
                Shape::Mesh(key) => {
                    let mesh = &self.meshes[key];
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    mesh.vertices
                }
                Shape::Ribbon(buffer, count) => {
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    *count
                }
            };
            if let Some(indirect) = &batch.indirect {
                pass.draw_indirect(indirect, 0);
            } else {
                pass.draw(0..vertices, 0..batch.count);
            }
        }
    }
}

fn gpu_particle(p: &Particle) -> GpuParticle {
    GpuParticle {
        position_age: [p.position[0], p.position[1], p.position[2], p.age],
        velocity_lifetime: [p.velocity[0], p.velocity[1], p.velocity[2], p.lifetime],
        previous_position_rotation: [
            p.previous_position[0],
            p.previous_position[1],
            p.previous_position[2],
            p.rotation,
        ],
        color: p.color,
        size: [p.size[0], p.size[1], 0., 0.],
        metadata: [p.id as u32, (p.id >> 32) as u32, p.ribbon_id, 1],
    }
}
fn particle_visible(p: &Particle, model: Mat4, view: &VfxView, radius: f32) -> bool {
    let position = model.transform_point3(Vec3::from(p.position));
    let scale = model
        .x_axis
        .truncate()
        .length()
        .max(model.y_axis.truncate().length())
        .max(model.z_axis.truncate().length());
    let radius = radius * p.size[0].max(p.size[1]).max(0.) * scale * 1.5;
    let m = view.view_projection.transpose();
    let rows = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
    [
        rows[3] + rows[0],
        rows[3] - rows[0],
        rows[3] + rows[1],
        rows[3] - rows[1],
        rows[2],
        rows[3] - rows[2],
    ]
    .iter()
    .all(|plane| plane.truncate().dot(position) + plane.w >= -radius * plane.truncate().length())
}
fn asset_vertices(model: &crysirror_asset::pbr::PbrModel) -> Result<Vec<MeshVertex>> {
    let matrices = model.rest_world();
    let mut out = Vec::new();
    let mut reachable = BTreeSet::new();
    let mut pending = model.roots.clone();
    while let Some(i) = pending.pop() {
        if reachable.insert(i) {
            pending.extend_from_slice(
                &model
                    .nodes
                    .get(i)
                    .context("VFX mesh root/node index")?
                    .children,
            );
        }
    }
    for i in reachable {
        let node = &model.nodes[i];
        let Some(mesh) = node.mesh else { continue };
        ensure!(
            node.skin.is_none(),
            "VFX mesh particles currently require static unskinned geometry"
        );
        for primitive in model.meshes.get(mesh).context("VFX mesh index")? {
            ensure!(
                primitive.skin_attrs.is_none() && primitive.morphs.is_empty(),
                "VFX mesh particles do not evaluate skin/morph animation"
            );
            ensure!(
                primitive.indices.len() % 3 == 0,
                "VFX mesh requires a triangle list"
            );
            ensure!(
                out.len() + primitive.indices.len() <= MAX_RENDER_VERTICES,
                "VFX imported mesh vertex budget exceeded"
            );
            for index in &primitive.indices {
                let v = primitive
                    .vertices
                    .get(*index as usize)
                    .context("VFX mesh vertex index")?;
                out.push(MeshVertex {
                    position: matrices[i]
                        .transform_point3(Vec3::from(v.position))
                        .to_array(),
                    uv: v.uv,
                });
            }
        }
    }
    Ok(out)
}
fn builtin_vertices(cube: bool) -> Vec<MeshVertex> {
    let faces = if cube {
        vec![
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
            [
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
            ],
            [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
            ],
            [
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
            [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
            ],
        ]
    } else {
        vec![[
            [-0.5, -0.5, 0.],
            [0.5, -0.5, 0.],
            [0.5, 0.5, 0.],
            [-0.5, 0.5, 0.],
        ]]
    };
    let uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    faces
        .iter()
        .flat_map(|f| {
            [0, 1, 2, 0, 2, 3].map(|i| MeshVertex {
                position: f[i],
                uv: uv[i],
            })
        })
        .collect()
}
fn ribbon_vertices(
    particles: &[Particle],
    model: Mat4,
    eye: Vec3,
    width: f32,
    uv_scale: f32,
    max_segments: u32,
) -> Result<Vec<RibbonVertex>> {
    ensure!(
        max_segments as usize <= MAX_RENDER_VERTICES / 6,
        "VFX ribbon segment budget exceeded"
    );
    let mut strips: BTreeMap<u32, Vec<&Particle>> = BTreeMap::new();
    for p in particles {
        strips.entry(p.ribbon_id).or_default().push(p);
    }
    let mut out = Vec::new();
    let mut segments = 0;
    for points in strips.values_mut() {
        points.sort_by_key(|p| p.id);
        let mut distance = 0.;
        for pair in points.windows(2) {
            if segments >= max_segments {
                break;
            }
            let a = model.transform_point3(Vec3::from(pair[0].position));
            let b = model.transform_point3(Vec3::from(pair[1].position));
            let delta = b - a;
            if delta.length_squared() < 1e-10 {
                continue;
            }
            let side = delta.cross(eye - (a + b) * 0.5).normalize_or(Vec3::X) * (width * 0.5);
            let end = distance + delta.length() * uv_scale;
            let quad = [
                RibbonVertex {
                    position: (a - side).to_array(),
                    uv: [distance, 0.],
                    color: pair[0].color,
                },
                RibbonVertex {
                    position: (a + side).to_array(),
                    uv: [distance, 1.],
                    color: pair[0].color,
                },
                RibbonVertex {
                    position: (b + side).to_array(),
                    uv: [end, 1.],
                    color: pair[1].color,
                },
                RibbonVertex {
                    position: (b - side).to_array(),
                    uv: [end, 0.],
                    color: pair[1].color,
                },
            ];
            out.extend([0, 1, 2, 0, 2, 3].map(|i| quad[i]));
            distance = end;
            segments += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(id: u64, position: [f32; 3], strip: u32) -> Particle {
        Particle {
            id,
            position,
            previous_position: position,
            velocity: [0.; 3],
            age: 0.,
            lifetime: 1.,
            color: [1.; 4],
            size: [0.1; 2],
            rotation: 0.,
            ribbon_id: strip,
        }
    }
    #[test]
    fn mesh_geometry_has_real_depth_and_ribbons_preserve_strip_boundaries() {
        let cube = builtin_vertices(true);
        let quad = builtin_vertices(false);
        assert_eq!(cube.len(), 36);
        assert_eq!(quad.len(), 6);
        assert!(cube.iter().any(|v| v.position[2] < 0.) && cube.iter().any(|v| v.position[2] > 0.));
        assert!(quad.iter().all(|v| v.position[2] == 0.));
        let particles = [
            p(2, [1., 0., 0.], 0),
            p(1, [0., 0., 0.], 0),
            p(3, [20., 20., 0.], 1),
        ];
        let ribbon = ribbon_vertices(
            &particles,
            Mat4::IDENTITY,
            Vec3::new(0., 0., 5.),
            0.2,
            2.,
            10,
        )
        .unwrap();
        assert_eq!(ribbon.len(), 6, "a different strip must not be joined");
        let separation = Vec3::from(ribbon[0].position).distance(Vec3::from(ribbon[1].position));
        assert!(
            (separation - 0.2).abs() < 1e-5,
            "width is full width, independent of size"
        );
        assert!(ribbon.iter().all(|v| v.position[0] <= 1.));
        assert!(ribbon.iter().any(|v| v.uv[0] == 2.));
    }
    #[test]
    fn culling_uses_current_view_and_local_transform_and_rejects_huge_geometry() {
        let view = VfxView::default();
        let inside = p(1, [0., 0., 0.5], 0);
        let outside = p(2, [10., 0., 0.5], 0);
        assert!(particle_visible(&inside, Mat4::IDENTITY, &view, 1.));
        assert!(!particle_visible(&outside, Mat4::IDENTITY, &view, 1.));
        assert!(particle_visible(
            &outside,
            Mat4::from_translation(Vec3::new(-10., 0., 0.)),
            &view,
            1.
        ));
        assert!(ribbon_vertices(&[], Mat4::IDENTITY, Vec3::Z, 1., 1., u32::MAX).is_err());
        assert_eq!(std::mem::size_of::<GpuParticle>(), 96);
    }
    #[test]
    #[ignore = "requires real GPU; explicit visibility/indirect readback"]
    fn gpu_visibility_compaction_updates_mesh_draw_args_for_current_view() {
        use std::{sync::mpsc, time::Duration};
        let gpu = pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let mut assets = Assets::new(temp.path()).unwrap();
        assets.register(ClientId(1), "").unwrap();
        let camera = Vec3::new(0., 0., 3.);
        let view = Mat4::look_at_rh(camera, Vec3::ZERO, Vec3::Y);
        let view = VfxView {
            camera,
            view,
            view_projection: Mat4::orthographic_rh(-1., 1., -1., 1., 0.1, 10.) * view,
            viewport: [128, 128],
            ..Default::default()
        };
        let particle = |position: [f32; 3], id| GpuParticle {
            position_age: [position[0], position[1], position[2], 0.],
            velocity_lifetime: [0., 0., 0., 1.],
            previous_position_rotation: [0.; 4],
            color: [1.; 4],
            size: [0.2, 0.2, 0., 0.],
            metadata: [id, 0, 0, 1],
        };
        let particles = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("visibility test states"),
                contents: bytemuck::cast_slice(&[
                    particle([0., 0., 0.], 1),
                    particle([100., 0., 0.], 2),
                    particle([0., 0., 20.], 3),
                ]),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let indices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("visibility test alive"),
                contents: bytemuck::cast_slice(&[0u32, 1, 2]),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // Source vertex count intentionally stale (sprite=6). Current mesh is 36.
        let indirect = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("visibility test source args"),
                contents: bytemuck::cast_slice(&[6u32, 3, 0, 0]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            });
        let emitter = EmitterFrame {
            system_key: 1,
            key: 1,
            client: 1,
            emitter_id: "cull".into(),
            transform: Mat4::IDENTITY.to_cols_array_2d(),
            space: SimulationSpace::World,
            render: RenderSpec::Mesh {
                mesh: MeshShape::Cube,
                texture: None,
                blend: BlendMode::Additive,
            },
            particles: Arc::from([]),
        };
        let input = GpuEmitterDraw {
            emitter: &emitter,
            particles: &particles,
            indices: &indices,
            indirect: &indirect,
            capacity: 3,
        };
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("visibility unused opaque depth"),
            size: wgpu::Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut renderer =
            VfxRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
        renderer.begin_frame(&view, 1).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer
            .prepare_gpu(
                &gpu,
                &mut encoder,
                &[input],
                &mut assets,
                &depth.create_view(&Default::default()),
            )
            .unwrap();
        renderer.finish_frame();
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline visibility args"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            &renderer.visible[&(1, "cull".into())].buffer,
            0,
            &output,
            0,
            16,
        );
        let submission = gpu.queue.submit(Some(encoder.finish()));
        let (tx, rx) = mpsc::channel();
        output.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(20)),
            })
            .unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let bytes = output.slice(..).get_mapped_range();
        let args = bytes
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(args, [36, 1, 0, 0]);
    }
}
