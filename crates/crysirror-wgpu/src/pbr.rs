// SPDX-License-Identifier: MPL-2.0
//! Physically based renderer for `PbrScene` (ADR 0010).
//!
//! Frame graph: 4 sun shadow cascades → 1× depth/normal/motion prepass →
//! SSAO + depth-aware blur → 4×MSAA HDR opaque pass (+ sky) → resolve → TAA
//! (jittered projection, reprojection, YCoCg clamp) → bloom (dual filter) →
//! tonemap into the caller's sRGB target. Image-based lighting uses a GGX-prefiltered equirectangular map,
//! SH9 irradiance and a split-sum BRDF LUT. All GPU objects are rebuilt from
//! logical scene data; none are saved or exposed to VMs.
use crate::Gpu;
use anyhow::{Context, Result, ensure};
use crysirror_asset::pbr::{self as cpu, AlphaMode, PbrMaterial, PbrVertex};
use crysirror_core::state::ClientId;
use crysirror_runtime::assets::Assets;
use crysirror_scene::view::{PbrDrawable, PbrScene, Tonemap};
use glam::{Mat4, Vec3};
use std::{collections::HashMap, sync::Arc};
use wgpu::util::DeviceExt;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MSAA: u32 = 4;
const SHADOW_SIZE: u32 = 2048;
const CASCADES: usize = 4;
const PREFILTER_W: u32 = 1024;
const PREFILTER_MIPS: u32 = 7;
const BLOOM_LEVELS: usize = 6;
const OBJECT_STRIDE: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    view_proj_nojitter: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    cascades: [[[f32; 4]; 4]; CASCADES],
    splits: [f32; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    env: [f32; 4],
    screen: [f32; 4],
    misc: [f32; 4],
    sh: [[f32; 4]; 9],
    /// x near, y far, z uploaded local lights, w 1/ln(far/near).
    clusters: [f32; 4],
    /// rgb color, a density
    fog_color: [f32; 4],
    /// x height falloff, y base height, z start distance, w max opacity (0 = off)
    fog_params: [f32; 4],
    /// x sun scattering, y sun exponent
    fog_sun: [f32; 4],
    /// x SSR intensity (0 = off), y roughness where SSR has faded out
    ssr: [f32; 4],
    /// Probe slots: [3k] position + intensity, [3k+1] half extent + blend, [3k+2].x parallax.
    probes: [[f32; 4]; 24],
    vsm_view: [[f32; 4]; 4],
    /// x level-0 texel, y levels, z/w depth range along the light.
    vsm: [f32; 4],
    /// x enabled, y pixel footprint per meter of distance.
    vsm2: [f32; 4],
    /// Irradiance volume (ADR 0028): centre + intensity, half extent + enabled, counts.
    gi_center: [f32; 4],
    gi_extent: [f32; 4],
    gi_counts: [f32; 4],
    rt_shadow: [f32; 4],
}
/// Byte offset of the irradiance probes in the shadow-data buffer
/// (after 64 tile matrices and the virtual shadow page table).
const GI_OFFSET: u64 =
    64 * crate::lights::MAX_SHADOW_TILES as u64 + 8 * crate::vsm::TABLE_ENTRIES as u64;
const GI_PROBE_BYTES: u64 = 64;
/// Per-probe 8×8 octahedral distance moments after all probe states
/// (`GI_VIS` in pbr.wgsl): 32 vec4.
const GI_VIS_OFFSET: u64 = GI_OFFSET
    + GI_PROBE_BYTES * crysirror_scene::components::IrradianceVolumeSpec::MAX_PROBES as u64;
const GI_VIS_BYTES: u64 = 512;
/// Probes seeing more back faces than this fraction are inside geometry.
const GI_BACKFACE_LIMIT: f32 = 0.25;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GiJob {
    probe: [u32; 4],
    faces: [[f32; 4]; 12],
    params: [f32; 4],
}
/// Irradiance volume refresh statistics (ADR 0028).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GiStats {
    pub probes: u32,
    /// Probe refreshes this frame and since the renderer was created.
    pub refreshed: u32,
    pub total_refreshed: u64,
    /// Complete passes over all probes since the last scene change.
    pub sweeps: u32,
    /// True once every probe has had three refreshes with nothing changed.
    pub converged: bool,
}
/// Irradiance volume update state (ADR 0028).
struct GiGpu {
    bgl: wgpu::BindGroupLayout,
    pipe: wgpu::ComputePipeline,
    key: Option<String>,
    /// Refresh order of the current sweep (nearest to the camera first).
    order: Vec<u32>,
    cursor: usize,
    /// Hash of lights, static object transforms, sun and environment.
    scene_hash: u64,
    stats: GiStats,
}
impl GiGpu {
    fn new(d: &wgpu::Device) -> Self {
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi project"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gi project"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/gi_project.wgsl").into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi project"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: None,
            label: Some("gi project"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_project"),
            compilation_options: Default::default(),
        });
        Self {
            bgl,
            pipe,
            key: None,
            order: Vec::new(),
            cursor: 0,
            scene_hash: 0,
            stats: GiStats::default(),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkUniform {
    inv_view_proj: [[f32; 4]; 4],
    light_view: [[f32; 4]; 4],
    camera: [f32; 4],
    params: [f32; 4],
    centers: [[i32; 4]; 8],
}
/// GPU side of the sun's virtual shadow maps (ADR 0027).
struct VsmGpu {
    cache: crate::vsm::PageCache,
    mark_bgl: wgpu::BindGroupLayout,
    mark_pipe: wgpu::ComputePipeline,
    clear_pipe: wgpu::RenderPipeline,
    requests: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// Marks copied this frame (window centres at marking time), mapped after submit.
    copied: Option<Vec<[i32; 2]>>,
    pending: Option<(
        std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
        Vec<[i32; 2]>,
    )>,
    requested: Vec<crate::vsm::PageKey>,
    /// Expand-only depth range along the light (moving casters must not
    /// change the cache key).
    depth: Option<[f32; 2]>,
    /// World bounding sphere of each opaque caster last frame (static/dynamic split).
    bounds: HashMap<u64, (Vec3, f32)>,
}
impl VsmGpu {
    fn new(d: &wgpu::Device) -> Self {
        let bytes = 4 * crate::vsm::TABLE_ENTRIES as u64;
        let mark_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vsm mark"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vsm mark"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vsm_mark.wgsl").into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm mark"),
            bind_group_layouts: &[Some(&mark_bgl)],
            immediate_size: 0,
        });
        let mark_pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: None,
            label: Some("vsm mark"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_mark"),
            compilation_options: Default::default(),
        });
        let clear = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow page clear"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                    return vec4<f32>(p * 2.0 - 1.0, 1.0, 1.0);
                }"
                .into(),
            ),
        });
        let empty = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let clear_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("shadow page clear"),
            layout: Some(&empty),
            vertex: wgpu::VertexState {
                module: &clear,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
        });
        Self {
            cache: crate::vsm::PageCache::new(),
            mark_bgl,
            mark_pipe,
            clear_pipe,
            requests: d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vsm page requests"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            staging: d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vsm page readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            copied: None,
            pending: None,
            requested: Vec::new(),
            depth: None,
            bounds: HashMap::new(),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsrUniform {
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    reproject: [[f32; 4]; 4],
    params: [f32; 4],
    screen: [f32; 4],
    misc: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoUniform {
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    params: [f32; 4],
    screen: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    alpha: [f32; 4],
    uv_transform: [f32; 4],
    surface: [f32; 4],
    effect_params: [[f32; 4]; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowUniform {
    view_proj: [[f32; 4]; 4],
    camera_time: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    prev_model: [[f32; 4]; 4],
    skin: [u32; 4],
}

struct GpuPrim {
    /// Joints (u16x4) + weights (f32x4) stream for skinned primitives.
    skin: Option<wgpu::Buffer>,
    vertices: wgpu::Buffer,
    vertex_count: usize,
    indices: wgpu::Buffer,
    count: u32,
    material: usize,
    bounds: (Vec3, Vec3),
    /// Cluster DAG of a static primitive (ADR 0026).
    lod: Option<LodPrim>,
}
struct LodPrim {
    /// All clusters' triangles, cluster after cluster.
    indices: wgpu::Buffer,
    clusters: Vec<LodCluster>,
    /// Index count of the leaf level, an upper bound for any cut.
    leaf_indices: u32,
    /// GPU cluster table + source indices for `lod_select.wgsl`.
    gpu_bind: wgpu::BindGroup,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuCluster {
    start: u32,
    count: u32,
    lod_error: f32,
    parent_error: f32,
    lod_bounds: [f32; 4],
    parent_bounds: [f32; 4],
    bounds: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LodParams {
    eye: [f32; 4],
    planes: [[f32; 4]; 6],
    misc: [f32; 4],
    slots: [u32; 4],
    firsts: [u32; 4],
}
const LOD_PARAMS_STRIDE: u64 = 256;
/// GPU cluster selection state (ADR 0026 step 2).
struct LodGpu {
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    pipe: wgpu::ComputePipeline,
    out: wgpu::Buffer,
    args: wgpu::Buffer,
}
impl LodGpu {
    fn new(d: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lod select"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<LodParams>() as u64
                        ),
                    },
                ),
                entry(1, storage(false)),
                entry(2, storage(false)),
            ],
        });
        let bgl1 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lod clusters"),
            entries: &[entry(0, storage(true)), entry(1, storage(true))],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lod select"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lod_select.wgsl").into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lod select"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
            immediate_size: 0,
        });
        let pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: None,
            label: Some("lod select"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_select"),
            compilation_options: Default::default(),
        });
        let (out, args) = Self::buffers(d, 4, 2);
        Self {
            bgl0,
            bgl1,
            pipe,
            out,
            args,
        }
    }
    fn buffers(d: &wgpu::Device, indices: u64, draws: u64) -> (wgpu::Buffer, wgpu::Buffer) {
        (
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster LOD output indices"),
                size: (indices * 4).max(16),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDEX,
                mapped_at_creation: false,
            }),
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster LOD indirect args"),
                size: (draws * 20).max(20),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        )
    }
}
struct LodCluster {
    start: u32,
    count: u32,
    bounds: crysirror_asset::cluster::Sphere,
    lod_error: f32,
    lod_bounds: crysirror_asset::cluster::Sphere,
    parent_error: f32,
    parent_bounds: crysirror_asset::cluster::Sphere,
}
/// Cluster-LOD statistics of the last frame (camera cut).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodStats {
    pub lod_primitives: usize,
    pub source_triangles: usize,
    pub drawn_triangles: usize,
    pub clusters_drawn: usize,
}
struct GpuMaterial {
    shader: Option<String>,
    alpha: AlphaMode,
    bind: wgpu::BindGroup,
    double_sided: bool,
    /// Coverage inputs for ray-traced alpha testing (ADR 0055).
    rt: RtCoverage,
}
#[derive(Clone, Copy, Default)]
struct RtCoverage {
    /// A preset effect varies with position/time, so its coverage cannot be baked.
    effect: bool,
    cutoff: f32,
    uv_transform: [f32; 4],
    emissive: [f32; 3],
}
struct CachedMaterial {
    material: GpuMaterial,
    _textures: Vec<wgpu::Texture>,
}
struct GpuModel {
    /// CPU node/skin/animation data (images dropped) for posing scenes.
    cpu: Arc<cpu::PbrModel>,
    meshes: Vec<Vec<GpuPrim>>,
    /// Whether static primitives were given cluster DAGs at load.
    lod_built: bool,
    materials: Vec<GpuMaterial>,
    _textures: Vec<wgpu::Texture>,
}
struct GpuEnv {
    key: String,
    source: wgpu::TextureView,
    prefiltered: wgpu::TextureView,
    sh: [[f32; 4]; 9],
    _textures: [wgpu::Texture; 2],
}
struct Targets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    msaa_depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    composite: wgpu::TextureView,
    pre_depth: wgpu::TextureView,
    normal: wgpu::TextureView,
    rt_ao: Option<wgpu::TextureView>,
    velocity: wgpu::TextureView,
    ao_raw: wgpu::TextureView,
    ao: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    history_valid: bool,
    bloom: Vec<(wgpu::TextureView, [u32; 2])>,
    /// SSR radiance (a = confidence) and the previous frame's final HDR color.
    ssr: wgpu::TextureView,
    pixel_gi: wgpu::TextureView,
    lighting_array: wgpu::TextureView,
    prev_hdr: wgpu::TextureView,
    prev_valid: bool,
}
fn blend_state(alpha: AlphaMode) -> Option<wgpu::BlendState> {
    use wgpu::{BlendComponent as B, BlendFactor as F, BlendOperation as O};
    match alpha {
        AlphaMode::Opaque | AlphaMode::Mask => None,
        AlphaMode::Blend => Some(wgpu::BlendState::ALPHA_BLENDING),
        AlphaMode::Premultiplied => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        AlphaMode::Additive => Some(wgpu::BlendState {
            color: B {
                src_factor: F::SrcAlpha,
                dst_factor: F::One,
                operation: O::Add,
            },
            alpha: B {
                src_factor: F::Zero,
                dst_factor: F::One,
                operation: O::Add,
            },
        }),
    }
}
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

pub struct PbrRenderer {
    profiler: crysirror_tasks::profile::Profiler,
    profile_frame: u64,
    hardware_rt: Option<crate::rt_pbr::PbrRt>,
    hardware_encoded: bool,
    exposure: AutoExposureState,
    light_bufs: crate::lights::LightBuffers,
    lod_stats: LodStats,
    lod_gpu: LodGpu,
    vsm: VsmGpu,
    gi: GiGpu,
    local_shadows: LocalShadows,
    pipelines: SurfacePipelines,
    custom_pipelines: HashMap<String, SurfacePipelines>,
    gpu: Arc<Gpu>,
    frame_bgl: wgpu::BindGroupLayout,
    material_bgl: wgpu::BindGroupLayout,
    object_bgl: wgpu::BindGroupLayout,
    shadow_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
    step_bgl: wgpu::BindGroupLayout,
    copy_hdr: wgpu::RenderPipeline,
    overrides: HashMap<String, CachedMaterial>,
    palette_buf: wgpu::Buffer,
    palette_capacity: u64,
    prev_palettes: HashMap<u64, Vec<[[f32; 4]; 4]>>,
    /// Per-draw morphed vertex buffers keyed by draw key: (weights hash, buffer, vertex count).
    morph_buffers: HashMap<u64, (u64, wgpu::Buffer, usize)>,
    ssao_bgl: wgpu::BindGroupLayout,
    blur_bgl: wgpu::BindGroupLayout,
    taa_bgl: wgpu::BindGroupLayout,
    ssao_pipe: wgpu::RenderPipeline,
    ssr_bgl: wgpu::BindGroupLayout,
    ssr_pipe: wgpu::RenderPipeline,
    blur_pipe: wgpu::RenderPipeline,
    taa_pipe: wgpu::RenderPipeline,
    cascade_bufs: Vec<wgpu::Buffer>,
    cascade_layers: Vec<wgpu::TextureView>,
    prev_models: HashMap<u64, [[f32; 4]; 4]>,
    prev_view_proj: Option<Mat4>,
    particles: crate::particles::Particles,
    frame_index: u32,
    sky: wgpu::RenderPipeline,
    prefilter: wgpu::RenderPipeline,
    probes: crate::probes::ProbeResources,
    bloom_down: wgpu::RenderPipeline,
    bloom_up: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    linear: wgpu::Sampler,
    env_sampler: wgpu::Sampler,
    material_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    frame_buf: wgpu::Buffer,
    post_buf: wgpu::Buffer,
    object_buf: wgpu::Buffer,
    object_capacity: u64,
    object_bind: wgpu::BindGroup,
    shadow_map: wgpu::TextureView,
    brdf_lut: wgpu::TextureView,
    defaults: [wgpu::TextureView; 4],
    _owned: Vec<wgpu::Texture>,
    models: HashMap<String, GpuModel>,
    env: Option<GpuEnv>,
    targets: Option<Targets>,
    scene: Option<PbrScene>,
}

fn tex_entry(binding: u32, sample: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: sample,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn float_tex(binding: u32) -> wgpu::BindGroupLayoutEntry {
    tex_entry(binding, wgpu::TextureSampleType::Float { filterable: true })
}
fn uniform_entry(
    binding: u32,
    vis: wgpu::ShaderStages,
    dynamic: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}
/// Read-only fragment storage buffer (clustered lights).
fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32, ty: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(ty),
        count: None,
    }
}
fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PbrVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}
fn skin_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![4 => Uint16x4, 5 => Float32x4];
    wgpu::VertexBufferLayout {
        array_stride: 24,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}
fn shader(d: &wgpu::Device, label: &str, src: String) -> wgpu::ShaderModule {
    d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

type Levels = Vec<(u32, u32, Vec<[f32; 3]>)>;
/// Float box-filtered mip chain of an equirect image (filtered importance sampling).
fn equirect_levels(img: &cpu::HdrImage) -> Levels {
    let mut levels: Vec<(u32, u32, Vec<[f32; 3]>)> = vec![(img.width, img.height, img.rgb.clone())];
    while levels.last().unwrap().0 > 1 {
        let (w, h, src) = levels.last().unwrap();
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut dst = vec![[0f32; 3]; (nw * nh) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = src[(((y * 2 + dy).min(h - 1)) * w + (x * 2 + dx).min(w - 1)) as usize];
                    for k in 0..3 {
                        acc[k] += p[k].min(65000.) * 0.25;
                    }
                }
                dst[(y * nw + x) as usize] = acc;
            }
        }
        levels.push((nw, nh, dst));
    }
    levels
}
/// Upload an equirect mip chain as an HDR texture (f16, clamped finite).
fn upload_levels(gpu: &Gpu, label: &str, levels: &Levels) -> wgpu::Texture {
    let d = &gpu.device;
    let (width, height) = (levels[0].0, levels[0].1);
    let src_tex = d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width,
            height: height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, (w, h, px)) in levels.iter().enumerate() {
        // f16 tops out at 65504: clamp (e.g. the sun disc) so no texel is Inf.
        let data: Vec<half::f16> = px
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0])
            .map(|v| {
                half::f16::from_f32(if v.is_finite() {
                    v.clamp(0., 60000.)
                } else {
                    0.
                })
            })
            .collect();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src_tex,
                mip_level: i as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
    src_tex
}
/// Mix an instance key with a sub-index into a stable draw identity.
/// Native ray queries read renderer streams directly: BLAS input plus storage
/// for GPU skinning and UV gathers (ADR 0055). Empty without the RT feature.
fn rt_buffer_usage(d: &wgpu::Device) -> wgpu::BufferUsages {
    if d.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT
    } else {
        wgpu::BufferUsages::empty()
    }
}
fn draw_key(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17) ^ b.wrapping_add(1)
}
fn view_mips(t: &wgpu::Texture, base: u32, count: u32) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: base,
        mip_level_count: Some(count),
        ..Default::default()
    })
}
fn bytes_of_vertices(v: &[PbrVertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 48);
    for x in v {
        for f in x
            .position
            .iter()
            .chain(&x.normal)
            .chain(&x.uv)
            .chain(&x.tangent)
        {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    out
}

struct SurfacePipelines {
    opaque: wgpu::RenderPipeline,
    opaque_double: wgpu::RenderPipeline,
    opaque_skin: wgpu::RenderPipeline,
    opaque_skin_double: wgpu::RenderPipeline,
    prepass: wgpu::RenderPipeline,
    prepass_double: wgpu::RenderPipeline,
    prepass_skin: wgpu::RenderPipeline,
    prepass_skin_double: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_skin: wgpu::RenderPipeline,
    transparent: Vec<wgpu::RenderPipeline>,
}
impl SurfacePipelines {
    fn build(
        gpu: &Gpu,
        frame_bgl: &wgpu::BindGroupLayout,
        material_bgl: &wgpu::BindGroupLayout,
        object_bgl: &wgpu::BindGroupLayout,
        shadow_bgl: &wgpu::BindGroupLayout,
        custom: Option<&str>,
    ) -> Result<Self> {
        let [color, pre, shadow] = crate::effect::modules(custom)?;
        let d = &gpu.device;
        let error_scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        let pbr_sh = shader(d, "surface color", color);
        let pre_pass_sh = shader(d, "surface prepass", pre);
        let shadow_sh = shader(d, "surface shadow", shadow);
        let main_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr"),
            bind_group_layouts: &[Some(frame_bgl), Some(material_bgl), Some(&object_bgl)],
            immediate_size: 0,
        });
        let depth_state = |write: bool, cmp: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(cmp),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let msaa = wgpu::MultisampleState {
            count: MSAA,
            ..Default::default()
        };
        let opaque_pipeline = |cull: Option<wgpu::Face>, skinned: bool, alpha: AlphaMode| {
            let buffers = [vertex_layout(), skin_layout()];
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some("pbr opaque"),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &pbr_sh,
                    entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
                    buffers: &buffers[..if skinned { 2 } else { 1 }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &pbr_sh,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR,
                        blend: blend_state(alpha),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(
                    !alpha.transparent(),
                    wgpu::CompareFunction::LessEqual,
                )),
                multisample: if alpha.transparent() {
                    Default::default()
                } else {
                    msaa
                },
                multiview_mask: None,
            })
        };
        let opaque = opaque_pipeline(Some(wgpu::Face::Back), false, AlphaMode::Opaque);
        let opaque_double = opaque_pipeline(None, false, AlphaMode::Opaque);
        let opaque_skin = opaque_pipeline(Some(wgpu::Face::Back), true, AlphaMode::Opaque);
        let opaque_skin_double = opaque_pipeline(None, true, AlphaMode::Opaque);
        let mut transparent = Vec::new();
        for alpha in [
            AlphaMode::Blend,
            AlphaMode::Additive,
            AlphaMode::Premultiplied,
        ] {
            for skin in [false, true] {
                for double in [false, true] {
                    transparent.push(opaque_pipeline(
                        if double { None } else { Some(wgpu::Face::Back) },
                        skin,
                        alpha,
                    ));
                }
            }
        }
        let prepass_pipeline = |cull: Option<wgpu::Face>, skinned: bool| {
            let buffers = [vertex_layout(), skin_layout()];
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some("prepass"),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &pre_pass_sh,
                    entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
                    buffers: &buffers[..if skinned { 2 } else { 1 }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &pre_pass_sh,
                    entry_point: Some("fs_main"),
                    targets: &[
                        Some(HDR.into()),
                        Some(wgpu::TextureFormat::Rg16Float.into()),
                    ],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
                multisample: Default::default(),
                multiview_mask: None,
            })
        };
        let prepass = prepass_pipeline(Some(wgpu::Face::Back), false);
        let prepass_double = prepass_pipeline(None, false);
        let prepass_skin = prepass_pipeline(Some(wgpu::Face::Back), true);
        let prepass_skin_double = prepass_pipeline(None, true);
        let shadow_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(shadow_bgl), Some(object_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });
        let shadow_pipeline = |skinned: bool| {
            let buffers = [vertex_layout(), skin_layout()];
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some("shadow"),
                layout: Some(&shadow_layout),
                vertex: wgpu::VertexState {
                    module: &shadow_sh,
                    entry_point: Some(if skinned { "vs_skinned" } else { "vs_main" }),
                    buffers: &buffers[..if skinned { 2 } else { 1 }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shadow_sh,
                    entry_point: Some("fs_main"),
                    targets: &[],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                    ..depth_state(true, wgpu::CompareFunction::Less)
                }),
                multisample: Default::default(),
                multiview_mask: None,
            })
        };
        let shadow = shadow_pipeline(false);
        let shadow_skin = shadow_pipeline(true);
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            anyhow::bail!("material pipeline: {error}");
        }
        Ok(Self {
            opaque,
            opaque_double,
            opaque_skin,
            opaque_skin_double,
            prepass,
            prepass_double,
            prepass_skin,
            prepass_skin_double,
            shadow,
            shadow_skin,
            transparent,
        })
    }
}

/// Eye adaptation: ping-pong 1×1 adapted log2 luminance.
struct AutoExposureState {
    bgl: wgpu::BindGroupLayout,
    pipe: wgpu::RenderPipeline,
    state: Vec<(wgpu::Texture, wgpu::TextureView)>,
    buf: wgpu::Buffer,
    current: usize,
    valid: bool,
    last_time: Option<f32>,
}

/// Local-light shadow atlas, per-tile light uniforms and the tile matrices
/// the PBR shader samples (ADR 0018).
struct LocalShadows {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    tile_bufs: Vec<wgpu::Buffer>,
    tile_binds: Vec<wgpu::BindGroup>,
    matrices: wgpu::Buffer,
    tiles: Vec<crate::lights::ShadowTile>,
    requested: usize,
    granted: usize,
}
impl LocalShadows {
    fn new(d: &wgpu::Device, shadow_bgl: &wgpu::BindGroupLayout) -> Self {
        use crate::lights::{MAX_SHADOW_TILES, SHADOW_ATLAS};
        let texture = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("local shadow atlas"),
            size: wgpu::Extent3d {
                width: crate::lights::SHADOW_ATLAS_WIDTH,
                height: SHADOW_ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let tile_bufs: Vec<_> = (0..MAX_SHADOW_TILES)
            .map(|_| {
                d.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("local shadow tile"),
                    size: std::mem::size_of::<ShadowUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let tile_binds = tile_bufs
            .iter()
            .map(|b| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("local shadow tile"),
                    layout: shadow_bgl,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: b.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let matrices = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("local shadow matrices"),
            // Local tile matrices, then the virtual shadow page table (ADR 0027).
            size: GI_VIS_OFFSET
                + GI_VIS_BYTES
                    * crysirror_scene::components::IrradianceVolumeSpec::MAX_PROBES as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            _texture: texture,
            view,
            tile_bufs,
            tile_binds,
            matrices,
            tiles: Vec::new(),
            requested: 0,
            granted: 0,
        }
    }
}

impl PbrRenderer {
    pub fn new(gpu: Arc<Gpu>, output: wgpu::TextureFormat) -> Result<Self> {
        let d = &gpu.device;
        let frag = wgpu::ShaderStages::FRAGMENT;
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr frame"),
            entries: &[
                uniform_entry(0, both, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler_entry(2, wgpu::SamplerBindingType::Comparison),
                float_tex(3),
                sampler_entry(4, wgpu::SamplerBindingType::Filtering),
                float_tex(5),
                float_tex(6),
                float_tex(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(9),
                storage_entry(10),
                storage_entry(11),
                storage_entry(12),
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14, visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
                    }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: frag,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let mut mat_entries = vec![uniform_entry(0, frag, false)];
        for b in 1..=5 {
            mat_entries.push(float_tex(b));
        }
        mat_entries.push(sampler_entry(6, wgpu::SamplerBindingType::Filtering));
        mat_entries.push(float_tex(7));
        mat_entries.push(float_tex(8));
        let material_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr material"),
            entries: &mat_entries,
        });
        let object_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr object"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shadow_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow light"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, false)],
        });
        let unfiltered = |binding: u32| {
            tex_entry(
                binding,
                wgpu::TextureSampleType::Float { filterable: false },
            )
        };
        let post_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap"),
            entries: &[
                uniform_entry(0, frag, false),
                float_tex(1),
                sampler_entry(2, wgpu::SamplerBindingType::Filtering),
                float_tex(3),
                unfiltered(4),
            ],
        });
        let exposure_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("auto exposure"),
            entries: &[uniform_entry(0, frag, false), float_tex(1), unfiltered(2)],
        });
        // prefilter + bloom share: uniform, source texture, sampler.
        let step_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fullscreen step"),
            entries: &[
                uniform_entry(0, frag, false),
                float_tex(1),
                sampler_entry(2, wgpu::SamplerBindingType::Filtering),
            ],
        });

        let common = crate::effect::common(crate::effect::PRESETS);
        let fs = include_str!("shaders/fullscreen.wgsl");

        let ssao_sh = shader(
            d,
            "ssao",
            format!("{fs}\n{}", include_str!("shaders/ssao.wgsl")),
        );
        let taa_sh = shader(
            d,
            "taa",
            format!("{fs}\n{}", include_str!("shaders/taa.wgsl")),
        );
        let ssr_sh = shader(
            d,
            "ssr",
            format!("{fs}\n{}", include_str!("shaders/ssr.wgsl")),
        );
        let sky_sh = shader(
            d,
            "sky",
            format!("{common}\n{}", include_str!("shaders/sky.wgsl")),
        );

        let brdf_sh = shader(
            d,
            "brdf",
            format!("{fs}\n{}", include_str!("shaders/brdf.wgsl")),
        );
        let pre_sh = shader(
            d,
            "prefilter",
            format!("{fs}\n{}", include_str!("shaders/prefilter.wgsl")),
        );
        let tone_sh = shader(
            d,
            "tonemap",
            format!("{fs}\n{}", include_str!("shaders/tonemap.wgsl")),
        );
        let bloom_sh = shader(
            d,
            "bloom",
            format!("{fs}\n{}", include_str!("shaders/bloom.wgsl")),
        );

        let depth_state = |write: bool, cmp: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(cmp),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let msaa = wgpu::MultisampleState {
            count: MSAA,
            ..Default::default()
        };
        let sky_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&frame_bgl)],
            immediate_size: 0,
        });
        let sky = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: &sky_sh,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_sh,
                entry_point: Some("fs_main"),
                targets: &[Some(HDR.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::LessEqual)),
            multisample: msaa,
            multiview_mask: None,
        });

        let fullscreen = |label: &str,
                          module: &wgpu::ShaderModule,
                          entry: &str,
                          bgl: &wgpu::BindGroupLayout,
                          format: wgpu::TextureFormat,
                          blend: Option<wgpu::BlendState>| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
            })
        };
        let prefilter = fullscreen("prefilter", &pre_sh, "fs_prefilter", &step_bgl, HDR, None);
        let bloom_down = fullscreen(
            "bloom down",
            &bloom_sh,
            "fs_bloom_down",
            &step_bgl,
            HDR,
            None,
        );
        let copy_shader = shader(
            d,
            "copy HDR",
            format!("{fs}\n{}", include_str!("shaders/copy_hdr.wgsl")),
        );
        let copy_hdr = fullscreen("copy HDR", &copy_shader, "fs_copy", &step_bgl, HDR, None);
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let bloom_up = fullscreen(
            "bloom up",
            &bloom_sh,
            "fs_bloom_up",
            &step_bgl,
            HDR,
            Some(additive),
        );
        let tonemap = fullscreen("tonemap", &tone_sh, "fs_tonemap", &post_bgl, output, None);
        let exposure_sh = shader(
            d,
            "auto exposure",
            format!(
                "{}\n{}",
                include_str!("shaders/fullscreen.wgsl"),
                include_str!("shaders/exposure.wgsl")
            ),
        );
        let exposure_pipe = fullscreen(
            "auto exposure",
            &exposure_sh,
            "fs_exposure",
            &exposure_bgl,
            wgpu::TextureFormat::R32Float,
            None,
        );
        let exposure_state: Vec<(wgpu::Texture, wgpu::TextureView)> = (0..2)
            .map(|_| {
                let t = d.create_texture(&wgpu::TextureDescriptor {
                    label: Some("adapted luminance"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let v = t.create_view(&Default::default());
                (t, v)
            })
            .collect();
        let exposure_buf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto exposure"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth_tex = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: frag,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let unfiltered =
            |b: u32| tex_entry(b, wgpu::TextureSampleType::Float { filterable: false });
        let ssao_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao"),
            entries: &[uniform_entry(0, frag, false), depth_tex(1), unfiltered(2)],
        });
        let blur_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao blur"),
            entries: &[
                uniform_entry(0, frag, false),
                depth_tex(1),
                unfiltered(2),
                unfiltered(3),
            ],
        });
        let taa_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa"),
            entries: &[
                uniform_entry(0, frag, false),
                unfiltered(1),
                sampler_entry(2, wgpu::SamplerBindingType::Filtering),
                float_tex(3),
                unfiltered(4),
            ],
        });
        let ao_format = wgpu::TextureFormat::Rgba8Unorm;
        let ssao_pipe = fullscreen("ssao", &ssao_sh, "fs_ssao", &ssao_bgl, ao_format, None);
        let blur_pipe = fullscreen("ssao blur", &ssao_sh, "fs_blur", &blur_bgl, ao_format, None);
        let taa_pipe = fullscreen("taa", &taa_sh, "fs_taa", &taa_bgl, HDR, None);
        let ssr_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssr"),
            entries: &[
                uniform_entry(0, frag, false),
                depth_tex(1),
                unfiltered(2),
                float_tex(3),
                sampler_entry(4, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let ssr_pipe = fullscreen("ssr", &ssr_sh, "fs_ssr", &ssr_bgl, HDR, None);

        let sampler = |filter: wgpu::FilterMode,
                       u: wgpu::AddressMode,
                       v: wgpu::AddressMode,
                       aniso: u16,
                       cmp: Option<wgpu::CompareFunction>| {
            d.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: u,
                address_mode_v: v,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: match filter {
                    wgpu::FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
                    wgpu::FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
                },
                anisotropy_clamp: aniso,
                compare: cmp,
                ..Default::default()
            })
        };
        use wgpu::{AddressMode as A, FilterMode as F};
        let linear = sampler(F::Linear, A::ClampToEdge, A::ClampToEdge, 1, None);
        let env_sampler = sampler(F::Linear, A::Repeat, A::ClampToEdge, 1, None);
        let material_sampler = sampler(F::Linear, A::Repeat, A::Repeat, 16, None);
        let shadow_sampler = sampler(
            F::Linear,
            A::ClampToEdge,
            A::ClampToEdge,
            1,
            Some(wgpu::CompareFunction::LessEqual),
        );

        let uniform = |label: &str, size: u64| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let frame_buf = uniform("pbr frame", std::mem::size_of::<FrameUniform>() as u64);
        let cascade_bufs: Vec<wgpu::Buffer> = (0..CASCADES)
            .map(|_| uniform("cascade light", std::mem::size_of::<ShadowUniform>() as u64))
            .collect();
        let post_buf = uniform("post", 32);
        let object_capacity = 256;
        let object_buf = uniform("pbr objects", OBJECT_STRIDE * object_capacity);
        let particles = crate::particles::Particles::new(d, &frame_bgl, &common, MSAA);
        let palette_capacity = 1024u64;
        let palette_buf = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("joint palettes"),
            size: 64 * palette_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let object_bind = Self::object_bind(d, &object_bgl, &object_buf, &palette_buf);

        let mut owned = Vec::new();
        let shadow_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_map = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let cascade_layers = (0..CASCADES as u32)
            .map(|l| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: l,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        owned.push(shadow_tex);

        // Split-sum BRDF LUT, computed once on the GPU.
        let lut_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("brdf lut"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let brdf_lut = lut_tex.create_view(&Default::default());
        {
            let empty = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[],
            });
            let p = fullscreen("brdf lut", &brdf_sh, "fs_brdf", &empty, HDR, None);
            let empty_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &empty,
                entries: &[],
            });
            let mut enc = d.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    multiview_mask: None,
                    label: Some("brdf lut"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        depth_slice: None,
                        view: &brdf_lut,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&p);
                pass.set_bind_group(0, &empty_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            gpu.queue.submit(Some(enc.finish()));
        }
        owned.push(lut_tex);

        let mut defaults = Vec::new();
        for (rgba, srgb) in [
            ([255u8, 255, 255, 255], true),
            ([255, 255, 255, 255], false),
            ([128, 128, 255, 255], false),
            ([0, 0, 0, 255], true),
        ] {
            let (t, v) = Self::upload_rgba(&gpu, 1, 1, &[rgba.to_vec()], srgb);
            owned.push(t);
            defaults.push(v);
        }
        let defaults: [wgpu::TextureView; 4] = defaults
            .try_into()
            .map_err(|_| anyhow::anyhow!("defaults"))?;

        let pipelines = SurfacePipelines::build(
            &gpu,
            &frame_bgl,
            &material_bgl,
            &object_bgl,
            &shadow_bgl,
            None,
        )?;
        Ok(Self {
            profiler: Default::default(),
            profile_frame: 0,
            hardware_rt: None,
            hardware_encoded: false,
            exposure: AutoExposureState {
                bgl: exposure_bgl,
                pipe: exposure_pipe,
                state: exposure_state,
                buf: exposure_buf,
                current: 0,
                valid: false,
                last_time: None,
            },
            light_bufs: crate::lights::LightBuffers::new(d),
            lod_stats: LodStats::default(),
            lod_gpu: LodGpu::new(d),
            vsm: VsmGpu::new(d),
            gi: GiGpu::new(d),
            probes: crate::probes::ProbeResources::new(
                d,
                &gpu.queue,
                HDR,
                DEPTH,
                MSAA,
                PREFILTER_MIPS,
                &shader(
                    d,
                    "probe",
                    format!("{fs}\n{}", include_str!("shaders/probe.wgsl")),
                ),
            ),
            local_shadows: LocalShadows::new(d, &shadow_bgl),
            pipelines,
            custom_pipelines: HashMap::new(),
            gpu,
            frame_bgl,
            material_bgl,
            object_bgl,
            shadow_bgl,
            post_bgl,
            step_bgl,
            copy_hdr,
            overrides: HashMap::new(),
            palette_buf,
            palette_capacity,
            prev_palettes: HashMap::new(),
            morph_buffers: HashMap::new(),
            ssao_bgl,
            blur_bgl,
            taa_bgl,
            ssao_pipe,
            ssr_bgl,
            ssr_pipe,
            blur_pipe,
            taa_pipe,
            cascade_bufs,
            cascade_layers,
            prev_models: HashMap::new(),
            prev_view_proj: None,
            particles,
            frame_index: 0,
            sky,
            prefilter,
            bloom_down,
            bloom_up,
            tonemap,
            linear,
            env_sampler,
            material_sampler,
            shadow_sampler,
            frame_buf,
            post_buf,
            object_buf,
            object_capacity,
            object_bind,
            shadow_map,
            brdf_lut,
            defaults,
            _owned: owned,
            models: HashMap::new(),
            env: None,
            targets: None,
            scene: None,
        })
    }
    fn object_bind(
        d: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        buf: &wgpu::Buffer,
        palettes: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr objects"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: buf,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palettes.as_entire_binding(),
                },
            ],
        })
    }
    fn upload_rgba(
        gpu: &Gpu,
        w: u32,
        h: u32,
        levels: &[Vec<u8>],
        srgb: bool,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let t = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pbr texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let (mut lw, mut lh) = (w, h);
        for (i, data) in levels.iter().enumerate() {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &t,
                    mip_level: i as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(lw * 4),
                    rows_per_image: Some(lh),
                },
                wgpu::Extent3d {
                    width: lw,
                    height: lh,
                    depth_or_array_layers: 1,
                },
            );
            lw = (lw / 2).max(1);
            lh = (lh / 2).max(1);
        }
        let v = t.create_view(&Default::default());
        (t, v)
    }

    pub(crate) fn set_hardware_rt(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.hardware_rt.is_some() {
            return Ok(());
        }
        self.hardware_rt = if enabled {
            Some(crate::rt_pbr::PbrRt::new(&self.gpu)?)
        } else {
            None
        };
        self.hardware_encoded = false;
        self.targets = None;
        Ok(())
    }
    pub(crate) fn hardware_rt_stats(&self) -> Option<serde_json::Value> {
        self.hardware_rt.as_ref().map(|rt| rt.stats_json())
    }
    pub(crate) fn set_profiler(&mut self, p: crysirror_tasks::profile::Profiler, frame: u64) {
        self.profiler = p;
        self.profile_frame = frame;
    }
    pub fn set_scene(&mut self, scene: Option<PbrScene>) {
        if scene.is_none() {
            self.models.clear();
            self.overrides.clear();
            if let Some(rt) = &mut self.hardware_rt {
                rt.clear();
            }
        }
        self.scene = scene;
    }
    /// Frames a single still capture should accumulate for TAA to converge.
    pub fn warmup_frames(&self) -> u32 {
        let Some(s) = &self.scene else { return 1 };
        let env = &s.environment;
        let mut frames = 1;
        if env.temporal_aa {
            frames = frames.max(16);
        }
        // SSR reflects the previous frame.
        if env.reflections.is_some() {
            frames = frames.max(2);
        }
        // Every probe gets three refreshes (bounces accumulate per refresh);
        // ray-traced updates blend with hysteresis and need about twelve.
        if let Some(v) = s.volumes.iter().min_by_key(|v| v.key) {
            let v = &v.spec;
            let refreshes = if v.ray_traced && self.hardware_rt.is_some() { 12 } else { 3 };
            frames = frames.max(
                (refreshes
                    * v.counts
                        .iter()
                        .product::<u32>()
                        .div_ceil(v.probes_per_frame))
                .clamp(3, 64),
            );
        }
        // Pages are marked in one frame and rendered in the next.
        if env.virtual_shadows.is_some() {
            frames = frames.max(3);
        }
        // Ray-traced soft shadows converge over their visibility history.
        if self.hardware_rt.is_some()
            && (env.sun.as_ref().is_some_and(|s| s.angular_diameter_degrees > 0.)
                || s.lights.iter().any(|l| l.shadows && l.source_radius > 0.))
        {
            frames = frames.max(crate::rt_pbr::MAX_HISTORY / 2);
        }
        frames
    }
    /// Virtual shadow map page statistics of the last encoded frame.
    pub fn vsm_stats(&self) -> crate::vsm::VsmStats {
        self.vsm.cache.stats
    }
    /// Map this frame's page marks for reading; call after the frame's submit.
    pub fn after_submit(&mut self) {
        if self.hardware_encoded {
            if let Some(rt) = &mut self.hardware_rt {
                rt.mark_submitted();
            }
            self.hardware_encoded = false;
        }
        if let Some(centers) = self.vsm.copied.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.vsm
                .staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
            self.vsm.pending = Some((rx, centers));
        }
    }
    /// Debug readback of the GPU cut: (main, shadow) index counts per clustered
    /// draw of the last frame. Blocks on the GPU; tests and tools only.
    pub fn debug_gpu_lod_counts(&self, draws: usize) -> Result<Vec<(u32, u32)>> {
        let d = &self.gpu.device;
        let size = (draws as u64 * 40).max(40);
        let staging = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lod args readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = d.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.lod_gpu.args, 0, &staging, 0, size);
        self.gpu.queue.submit(Some(enc.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        d.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv().context("readback")??;
        let words: Vec<u32> = bytemuck::cast_slice(&staging.slice(..).get_mapped_range()).to_vec();
        Ok((0..draws)
            .map(|i| (words[i * 10], words[i * 10 + 5]))
            .collect())
    }
    /// Cluster-LOD statistics of the last encoded frame.
    pub fn lod_stats(&self) -> LodStats {
        self.lod_stats
    }
    /// Irradiance volume refresh statistics of the last encoded frame.
    pub fn gi_stats(&self) -> GiStats {
        self.gi.stats
    }
    /// Debug readback of irradiance probe states: (usable, back-face
    /// fraction, refreshed, 0) per probe of the current volume.
    pub fn debug_gi_probe_states(&self) -> Result<Vec<[f32; 4]>> {
        let d = &self.gpu.device;
        let probes = self.gi.stats.probes as u64;
        let size = (probes * GI_PROBE_BYTES).max(16);
        let staging = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = d.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.local_shadows.matrices, GI_OFFSET, &staging, 0, size);
        self.gpu.queue.submit(Some(enc.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        d.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv().context("readback")??;
        let v: Vec<[f32; 4]> = bytemuck::cast_slice(&staging.slice(..).get_mapped_range()).to_vec();
        Ok((0..probes as usize).map(|i| v[i * 4 + 3]).collect())
    }
    /// Reflection probe statistics of the last encoded frame.
    pub fn probe_stats(&self) -> crate::probes::ProbeStats {
        self.probes.stats
    }
    /// Clustered light statistics of the last encoded frame.
    pub fn light_stats(&self) -> crate::lights::ClusterStats {
        self.light_bufs.stats
    }
    /// Local shadows (requested lights, granted lights, atlas tiles) last frame.
    pub fn local_shadow_stats(&self) -> (usize, usize, usize) {
        (
            self.local_shadows.requested,
            self.local_shadows.granted,
            self.local_shadows.tiles.len(),
        )
    }
    pub fn live_particles(&self) -> usize {
        self.particles.live()
    }
    pub fn active(&self) -> bool {
        self.scene.is_some()
    }

    fn material_bind(
        &self,
        m: &MaterialUniform,
        views: [&wgpu::TextureView; 7],
    ) -> wgpu::BindGroup {
        let d = &self.gpu.device;
        let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pbr material"),
            contents: bytemuck::bytes_of(m),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: buf.as_entire_binding(),
        }];
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: match i {
                    5 => 7,
                    6 => 8,
                    _ => i as u32 + 1,
                },
                resource: wgpu::BindingResource::TextureView(v),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: wgpu::BindingResource::Sampler(&self.material_sampler),
        });
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr material"),
            layout: &self.material_bgl,
            entries: &entries,
        })
    }
    fn rt_coverage<T>(m: &crysirror_scene::material::Material<T>) -> RtCoverage {
        RtCoverage {
            effect: m.effect.is_some(),
            cutoff: m.alpha_cutoff,
            uv_transform: [m.uv_scale[0], m.uv_scale[1], m.uv_offset[0], m.uv_offset[1]],
            emissive: m.emissive_factor.map(|e| e * m.emissive_strength),
        }
    }
    fn material_uniform<T>(m: &crysirror_scene::material::Material<T>) -> MaterialUniform {
        MaterialUniform {
            base_color: m.base_color_factor,
            emissive: [
                m.emissive_factor[0],
                m.emissive_factor[1],
                m.emissive_factor[2],
                m.emissive_strength,
            ],
            params: [
                m.metallic_factor,
                m.roughness_factor,
                m.normal_scale,
                m.occlusion_strength,
            ],
            alpha: [
                m.alpha_mode.code(),
                m.alpha_cutoff,
                if m.unlit { 1. } else { 0. },
                if m.opacity_from_alpha { 1. } else { 0. },
            ],
            uv_transform: [m.uv_scale[0], m.uv_scale[1], m.uv_offset[0], m.uv_offset[1]],
            surface: [
                m.depth_fade,
                m.effect.as_ref().map_or(0., |e| e.uniforms().0),
                // The default flat-normal texture is RGB encoded.
                if m.normal_texture.is_some() {
                    m.normal_encoding.code()
                } else {
                    0.
                },
                m.specular_factor,
            ],
            effect_params: m.effect.as_ref().map_or([[0.; 4]; 4], |e| e.uniforms().1),
        }
    }
    fn prim(&self, p: &cpu::PbrPrimitive) -> GpuPrim {
        self.prim_lod(p, false)
    }
    fn prim_lod(&self, p: &cpu::PbrPrimitive, lod: bool) -> GpuPrim {
        let d = &self.gpu.device;
        let rt_usage = rt_buffer_usage(d);
        let lod = (lod
            && p.skin_attrs.is_none()
            && p.indices.len() / 3 > 2 * crysirror_asset::cluster::MAX_TRIS)
            .then(|| {
                let positions: Vec<Vec3> = p
                    .vertices
                    .iter()
                    .map(|v| Vec3::from_array(v.position))
                    .collect();
                let dag = crysirror_asset::cluster::build(&positions, &p.indices);
                let mut all = Vec::new();
                let clusters = dag
                    .clusters
                    .iter()
                    .map(|c| {
                        let start = all.len() as u32;
                        all.extend_from_slice(&c.indices);
                        LodCluster {
                            start,
                            count: c.indices.len() as u32,
                            bounds: c.bounds,
                            lod_error: c.lod_error,
                            lod_bounds: c.lod_bounds,
                            parent_error: c.parent_error,
                            parent_bounds: c.parent_bounds,
                        }
                    })
                    .collect();
                let clusters: Vec<LodCluster> = clusters;
                let sphere = |s: crysirror_asset::cluster::Sphere| {
                    [s.center.x, s.center.y, s.center.z, s.radius]
                };
                let table: Vec<GpuCluster> = clusters
                    .iter()
                    .map(|c| GpuCluster {
                        start: c.start,
                        count: c.count,
                        lod_error: c.lod_error,
                        parent_error: if c.parent_error.is_finite() {
                            c.parent_error
                        } else {
                            f32::MAX
                        },
                        lod_bounds: sphere(c.lod_bounds),
                        parent_bounds: sphere(c.parent_bounds),
                        bounds: sphere(c.bounds),
                    })
                    .collect();
                let indices = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cluster LOD indices"),
                    contents: bytemuck::cast_slice(&all),
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
                });
                let table = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cluster LOD table"),
                    contents: bytemuck::cast_slice(&table),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let gpu_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cluster LOD"),
                    layout: &self.lod_gpu.bgl1,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: table.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: indices.as_entire_binding(),
                        },
                    ],
                });
                LodPrim {
                    leaf_indices: clusters
                        .iter()
                        .filter(|c| c.lod_error == 0.)
                        .map(|c| c.count)
                        .sum(),
                    indices,
                    clusters,
                    gpu_bind,
                }
            });
        GpuPrim {
            lod,
            skin: p.skin_attrs.as_ref().map(|a| {
                let mut bytes = Vec::with_capacity(a.len() * 24);
                for (j, w) in a {
                    for x in j {
                        bytes.extend_from_slice(&x.to_le_bytes());
                    }
                    for x in w {
                        bytes.extend_from_slice(&x.to_le_bytes());
                    }
                }
                d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pbr skin"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::VERTEX | rt_usage,
                })
            }),
            vertices: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pbr vertices"),
                contents: &bytes_of_vertices(&p.vertices),
                usage: wgpu::BufferUsages::VERTEX | rt_usage,
            }),
            indices: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pbr indices"),
                contents: bytemuck::cast_slice(&p.indices),
                usage: wgpu::BufferUsages::INDEX | rt_usage,
            }),
            count: p.indices.len() as u32,
            vertex_count: p.vertices.len(),
            material: p.material,
            bounds: (Vec3::from_array(p.bounds.0), Vec3::from_array(p.bounds.1)),
        }
    }

    fn override_key(client: u32, m: &crysirror_scene::material::MaterialSpec) -> Result<String> {
        Ok(format!("{client}:{}", serde_json::to_string(m)?))
    }
    fn load_effect(
        &mut self,
        assets: &mut Assets,
        client: u32,
        effect: Option<&crysirror_scene::effect::ShaderEffect>,
    ) -> Result<Option<String>> {
        let Some(asset) = effect.and_then(|e| e.custom_asset()) else {
            return Ok(None);
        };
        let key = assets.key(ClientId(client), asset)?;
        if !self.custom_pipelines.contains_key(&key) {
            ensure!(
                self.custom_pipelines.len() < 32,
                "renderer custom shader limit (32 modules)"
            );
            let bytes = assets.read_for(ClientId(client), asset)?;
            let source = std::str::from_utf8(&bytes).context("custom shader UTF-8")?;
            let pipelines = SurfacePipelines::build(
                &self.gpu,
                &self.frame_bgl,
                &self.material_bgl,
                &self.object_bgl,
                &self.shadow_bgl,
                Some(source),
            )
            .with_context(|| format!("custom shader client {client} asset {asset}"))?;
            self.custom_pipelines.insert(key.clone(), pipelines);
        }
        Ok(Some(key))
    }
    fn load_override(
        &mut self,
        assets: &mut Assets,
        client: u32,
        m: &crysirror_scene::material::MaterialSpec,
    ) -> Result<()> {
        m.validate_paths()?;
        let key = Self::override_key(client, m)?;
        if self.overrides.contains_key(&key) {
            return Ok(());
        }
        let shader = self.load_effect(assets, client, m.effect.as_ref())?;
        let mut textures = Vec::new();
        let mut views = Vec::new();
        for (i, path) in m.textures().into_iter().enumerate() {
            if let Some(path) = path {
                let surface =
                    crysirror_image::decode_standard(&assets.read_for(ClientId(client), path)?)?;
                let srgb = matches!(i, 0 | 4 | 6);
                let levels = cpu::mip_chain(surface.width, surface.height, surface.rgba, srgb);
                let (t, v) =
                    Self::upload_rgba(&self.gpu, surface.width, surface.height, &levels, srgb);
                textures.push(t);
                views.push(Some(v));
            } else {
                views.push(None)
            }
        }
        let fallback = [0, 1, 2, 1, 0, 1, 0];
        let v: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, v)| v.as_ref().unwrap_or(&self.defaults[fallback[i]]))
            .collect();
        let bind = self.material_bind(
            &Self::material_uniform(m),
            v.try_into()
                .map_err(|_| anyhow::anyhow!("material slots"))?,
        );
        self.overrides.insert(
            key,
            CachedMaterial {
                material: GpuMaterial {
                    shader,
                    bind,
                    double_sided: m.double_sided,
                    alpha: m.alpha_mode,
                    rt: Self::rt_coverage(m),
                },
                _textures: textures,
            },
        );
        Ok(())
    }
    fn load_model(&mut self, assets: &mut Assets, client: u32, asset: &str) -> Result<bool> {
        let _span = self.profiler.span(self.profile_frame, "pbr.model_upload");
        let key = format!("{client}@{}:{asset}", assets.revision(ClientId(client)));
        let want_lod = self
            .scene
            .as_ref()
            .is_some_and(|s| s.environment.geometry_lod.is_some());
        if self
            .models
            .get(&key)
            .is_some_and(|m| m.lod_built || !want_lod)
        {
            return Ok(true);
        }
        let Some(model) = assets.model_for(ClientId(client), asset)? else {
            return Ok(false);
        };
        let shader_keys = model
            .materials
            .iter()
            .map(|m| self.load_effect(assets, client, m.effect.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        let mut textures = Vec::new();
        let mut views = Vec::new();
        for img in &model.images {
            let (t, v) = Self::upload_rgba(&self.gpu, img.width, img.height, &img.levels, img.srgb);
            textures.push(t);
            views.push(v);
        }
        let materials = model
            .materials
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let pick = |i: Option<usize>, fallback: usize| {
                    i.and_then(|i| views.get(i))
                        .unwrap_or(&self.defaults[fallback])
                };
                GpuMaterial {
                    rt: Self::rt_coverage(m),
                    shader: shader_keys[index].clone(),
                    bind: self.material_bind(
                        &Self::material_uniform(m),
                        [
                            pick(m.base_color_texture, 0),
                            pick(m.metallic_roughness_texture, 1),
                            pick(m.normal_texture, 2),
                            pick(m.occlusion_texture, 1),
                            pick(m.emissive_texture, 0),
                            pick(m.opacity_texture, 1),
                            pick(m.specular_texture, 0),
                        ],
                    ),
                    double_sided: m.double_sided,
                    alpha: m.alpha_mode,
                }
            })
            .collect();
        let lod_built = self
            .scene
            .as_ref()
            .is_some_and(|s| s.environment.geometry_lod.is_some());
        let meshes = model
            .meshes
            .iter()
            .map(|m| m.iter().map(|p| self.prim_lod(p, lod_built)).collect())
            .collect();
        self.models.insert(
            key,
            GpuModel {
                lod_built,
                cpu: model,
                meshes,
                materials,
                _textures: textures,
            },
        );
        Ok(true)
    }

    fn terrain_key(client: u32, spec: &crysirror_scene::terrain::TerrainSpec) -> String {
        format!(
            "terrain:{client}:{}",
            serde_json::to_string(spec).unwrap_or_default()
        )
    }
    fn load_terrain(
        &mut self,
        assets: &mut Assets,
        client: u32,
        spec: &crysirror_scene::terrain::TerrainSpec,
    ) -> Result<()> {
        let key = Self::terrain_key(client, spec);
        if self.models.contains_key(&key) {
            return Ok(());
        }
        let bytes = match &spec.heightmap {
            Some(p) => Some(assets.read_for(ClientId(client), p)?.to_vec()),
            None => None,
        };
        let grid = crysirror_asset::terrain::heights(spec, bytes.as_deref())?;
        let (prim, image, material) = crysirror_asset::terrain::mesh(spec, &grid);
        let (tex, view) =
            Self::upload_rgba(&self.gpu, image.width, image.height, &image.levels, true);
        let mat = GpuMaterial {
            rt: Self::rt_coverage(&material),
            shader: None,
            bind: self.material_bind(
                &Self::material_uniform(&material),
                [
                    &view,
                    &self.defaults[1],
                    &self.defaults[2],
                    &self.defaults[1],
                    &self.defaults[3],
                    &self.defaults[1],
                    &self.defaults[0],
                ],
            ),
            double_sided: false,
            alpha: AlphaMode::Opaque,
        };
        let gp = self.prim(&prim);
        self.models.insert(
            key,
            GpuModel {
                // Terrain keeps its own grid LOD path; never rebuilt for clusters.
                lod_built: true,
                cpu: Arc::new(cpu::PbrModel {
                    meshes: vec![vec![prim]],
                    materials: vec![material],
                    ..Default::default()
                }),
                meshes: vec![vec![gp]],
                materials: vec![mat],
                _textures: vec![tex],
            },
        );
        Ok(())
    }
    fn load_env(
        &mut self,
        assets: &mut Assets,
        client: u32,
        hdr: Option<&str>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<()> {
        let key = format!("{client}:{}", hdr.unwrap_or("<procedural>"));
        if self.env.as_ref().is_some_and(|e| e.key == key) {
            return Ok(());
        }
        let img = match hdr {
            Some(path) => cpu::load_hdr(&assets.read_for(ClientId(client), path)?)
                .with_context(|| format!("environment {path}"))?,
            None => procedural_sky(512, 256),
        };
        ensure!(
            img.width <= self.gpu.device.limits().max_texture_dimension_2d,
            "environment exceeds device texture size"
        );
        // Source with a float box-filtered mip chain (used for filtered importance sampling).
        let levels = equirect_levels(&img);
        let sh = project_sh9(
            &levels
                .iter()
                .find(|l| l.0 <= 128)
                .unwrap_or(levels.last().unwrap()),
        );
        let d = &self.gpu.device;
        let src_tex = upload_levels(&self.gpu, "environment source", &levels);
        let source = src_tex.create_view(&Default::default());
        let pre_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("environment prefiltered"),
            size: wgpu::Extent3d {
                width: PREFILTER_W,
                height: PREFILTER_W / 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: PREFILTER_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        for mip in 0..PREFILTER_MIPS {
            let roughness = mip as f32 / (PREFILTER_MIPS - 1) as f32;
            let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("prefilter params"),
                contents: bytemuck::cast_slice(&[
                    roughness,
                    img.width as f32,
                    levels.len() as f32,
                    0.,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("prefilter"),
                layout: &self.step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.env_sampler),
                    },
                ],
            });
            let target = view_mips(&pre_tex, mip, 1);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("prefilter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.prefilter);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let prefiltered = pre_tex.create_view(&Default::default());
        self.env = Some(GpuEnv {
            key,
            source,
            prefiltered,
            sh,
            _textures: [src_tex, pre_tex],
        });
        Ok(())
    }

    fn ensure_targets(&mut self, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let d = &self.gpu.device;
        let mk = |label: &str, size: [u32; 2], format, samples, usage| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
        };
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let rs = ra | wgpu::TextureUsages::TEXTURE_BINDING;
        let mut bloom = Vec::new();
        let mut s = size;
        for _ in 0..BLOOM_LEVELS {
            s = [(s[0] / 2).max(1), (s[1] / 2).max(1)];
            bloom.push((mk("bloom", s, HDR, 1, rs), s));
        }
        let ao_format = wgpu::TextureFormat::Rgba8Unorm;
        // Reflections and diffuse GI share one sampled texture binding (the
        // forward material stage already uses its 16-texture budget).
        let lighting = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen ray lighting array"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 2 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: HDR, usage: if self.hardware_rt.is_some() { rs | wgpu::TextureUsages::STORAGE_BINDING } else { rs }, view_formats: &[],
        });
        let lighting_array = lighting.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        let lighting_layer = |layer| lighting.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: layer,
            array_layer_count: Some(1), ..Default::default()
        });
        self.targets = Some(Targets {
            size,
            msaa_color: mk("hdr msaa", size, HDR, MSAA, ra),
            msaa_depth: mk("depth msaa", size, DEPTH, MSAA, ra),
            resolved: mk("hdr resolved", size, HDR, 1, rs),
            composite: mk("HDR transparency composite", size, HDR, 1, rs),
            pre_depth: mk("prepass depth", size, DEPTH, 1, rs),
            normal: mk("view normals", size, HDR, 1, rs),
            rt_ao: self.hardware_rt.as_ref().map(|_| {
                mk(
                    "hardware RT AO/visibility",
                    size,
                    HDR,
                    1,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                )
            }),
            velocity: mk(
                "motion vectors",
                size,
                wgpu::TextureFormat::Rg16Float,
                1,
                rs,
            ),
            ao_raw: mk("ssao raw", size, ao_format, 1, rs),
            ao: mk("ssao", size, ao_format, 1, rs),
            history: [
                mk("taa history a", size, HDR, 1, rs),
                mk("taa history b", size, HDR, 1, rs),
            ],
            history_valid: false,
            bloom,
            ssr: lighting_layer(0),
            pixel_gi: lighting_layer(1),
            lighting_array,
            prev_hdr: mk("previous HDR frame", size, HDR, 1, rs),
            prev_valid: false,
        });
    }

    /// Record the whole PBR frame and tonemap it into `target` at viewport `vp`.
    pub fn encode(
        &mut self,
        assets: &mut Assets,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        canvas: [u32; 2],
        vp: [f32; 4],
    ) -> Result<()> {
        let _span = self.profiler.span(self.profile_frame, "pbr.encode");
        self.hardware_encoded = false;
        let use_rt = self.hardware_rt.is_some();
        assets.poll_tasks();
        let mut scene = self.scene.clone().context("no PBR scene")?;
        let env = scene.environment.clone();
        self.load_env(
            assets,
            scene.environment_client,
            env.hdr.as_deref(),
            encoder,
        )?;
        let active_models: std::collections::HashSet<String> = scene
            .instances
            .iter()
            .filter_map(|i| match &i.drawable {
                PbrDrawable::Model { client, asset, .. }
                | PbrDrawable::Scene { client, asset, .. } => Some(format!(
                    "{client}@{}:{asset}",
                    assets.revision(ClientId(*client))
                )),
                PbrDrawable::Terrain { client, spec } => Some(Self::terrain_key(*client, spec)),
                _ => None,
            })
            .collect();
        self.models.retain(|key, _| active_models.contains(key));
        let mut ready_models = std::collections::HashSet::new();
        for inst in &scene.instances {
            match &inst.drawable {
                PbrDrawable::Model { client, asset, .. }
                | PbrDrawable::Scene { client, asset, .. } => {
                    if self.load_model(assets, *client, asset)? {
                        ready_models.insert((*client, asset.clone()));
                    }
                }
                PbrDrawable::Terrain { client, spec } => {
                    self.load_terrain(assets, *client, spec)?
                }
                PbrDrawable::Triangles { .. } => {}
            }
        }
        let models_complete = scene
            .instances
            .iter()
            .all(|instance| match &instance.drawable {
                PbrDrawable::Model { client, asset, .. }
                | PbrDrawable::Scene { client, asset, .. } => {
                    ready_models.contains(&(*client, asset.clone()))
                }
                _ => true,
            });
        // Do not bake an empty reflection capture while model imports are pending.
        if !models_complete {
            scene.probes.retain(|probe| probe.source.is_some());
        }
        let resource_stamp = {
            use std::hash::{Hash, Hasher};
            let mut keys: Vec<_> = ready_models.iter().collect();
            keys.sort();
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            for (client, path) in keys {
                client.hash(&mut hash);
                path.hash(&mut hash);
                assets.revision(ClientId(*client)).hash(&mut hash);
            }
            hash.finish()
        };
        scene.instances.retain(|instance| match &instance.drawable {
            PbrDrawable::Model { client, asset, .. } | PbrDrawable::Scene { client, asset, .. } => {
                ready_models.contains(&(*client, asset.clone()))
            }
            _ => true,
        });
        let mut pose_keys = Vec::new();
        let mut pose_requests = Vec::new();
        for instance in &scene.instances {
            if let PbrDrawable::Scene {
                client,
                asset,
                animation,
                ..
            } = &instance.drawable
            {
                let key = format!("{client}@{}:{asset}", assets.revision(ClientId(*client)));
                let model = self.models[&key].cpu.clone();
                let layers = animation
                    .iter()
                    .map(|l| {
                        crysirror_asset::anim::clip_index(&model, &l.clip)
                            .map(|clip| crysirror_asset::anim::Layer {
                                clip,
                                time: l.time,
                                weight: l.weight,
                            })
                            .with_context(|| format!("{asset}: no animation clip '{}'", l.clip))
                    })
                    .collect::<Result<Vec<_>>>()?;
                pose_keys.push(instance.key);
                pose_requests.push((model, layers));
            }
        }
        let poses = crysirror_asset::anim::poses_parallel(&assets.cpu_tasks(), &pose_requests)?;
        let mut poses: HashMap<_, _> = pose_keys.into_iter().zip(poses).collect();
        let used: std::collections::HashSet<String> = scene
            .instances
            .iter()
            .filter_map(|i| i.material.as_ref())
            .map(|(c, m)| Self::override_key(*c, m))
            .collect::<Result<_>>()?;
        ensure!(
            used.len() <= 256,
            "more than 256 distinct material overrides in one frame"
        );
        self.overrides.retain(|k, _| used.contains(k));
        for inst in &scene.instances {
            if let Some((client, material)) = &inst.material {
                self.load_override(assets, *client, material)?;
            }
        }
        self.ensure_targets(canvas);

        // Flat drawables get transient buffers/materials for this frame.
        let mut flat: Vec<(GpuPrim, GpuMaterial)> = Vec::new();
        let mut flat_cpu: Vec<Arc<cpu::PbrPrimitive>> = Vec::new();
        struct DrawRef {
            /// Stable identity across frames (instance and primitive).
            key: u64,
            override_key: Option<String>,
            object: u32,
            kind: usize, // 0 model, 1 flat
            model: Option<String>,
            mesh: u32,
            prim: usize,
            flat: usize,
            skinned: bool,
            /// Cluster-LOD index ranges for this frame (empty = whole primitive).
            ranges: Vec<std::ops::Range<u32>>,
            /// GPU-selected cut: indirect args slot pair (main = 2n, shadow = 2n+1).
            gpu: Option<u64>,
            /// Draws from this instance's morphed vertex buffer (`morph_buffers[key]`).
            morph: bool,
        }
        let mut palettes: Vec<[[f32; 4]; 4]> = Vec::new();
        let mut next_prev_palettes: HashMap<u64, Vec<[[f32; 4]; 4]>> = HashMap::new();
        // Morph targets: (draw key, weights hash, blended vertices to upload).
        let mut morph_uploads: Vec<(u64, u64, Vec<cpu::PbrVertex>)> = Vec::new();
        let mut morph_live: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut next_prev_models: HashMap<u64, [[f32; 4]; 4]> = HashMap::new();
        let mut draws = Vec::new();
        let mut objects: Vec<ObjectUniform> = Vec::new();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for inst in &scene.instances {
            let override_key = inst
                .material
                .as_ref()
                .map(|(c, m)| Self::override_key(*c, m))
                .transpose()?;
            let m = Mat4::from_cols_array_2d(&inst.matrix);
            let obj = objects.len() as u32;
            let prev_model = *self.prev_models.get(&inst.key).unwrap_or(&inst.matrix);
            next_prev_models.insert(inst.key, inst.matrix);
            objects.push(ObjectUniform {
                model: inst.matrix,
                normal: m.inverse().transpose().to_cols_array_2d(),
                prev_model,
                skin: [0; 4],
            });
            let grow = |b: (Vec3, Vec3), lo: &mut Vec3, hi: &mut Vec3| {
                for i in 0..8 {
                    let c = Vec3::new(
                        if i & 1 == 0 { b.0.x } else { b.1.x },
                        if i & 2 == 0 { b.0.y } else { b.1.y },
                        if i & 4 == 0 { b.0.z } else { b.1.z },
                    );
                    let w = m.transform_point3(c);
                    *lo = lo.min(w);
                    *hi = hi.max(w);
                }
            };
            match &inst.drawable {
                PbrDrawable::Model {
                    client,
                    asset,
                    mesh,
                } => {
                    let key = format!("{client}@{}:{asset}", assets.revision(ClientId(*client)));
                    let model = &self.models[&key];
                    let prims = model
                        .meshes
                        .get(*mesh as usize)
                        .with_context(|| format!("{asset}: mesh {mesh} missing"))?;
                    for (pi, p) in prims.iter().enumerate() {
                        grow(p.bounds, &mut lo, &mut hi);
                        draws.push(DrawRef {
                            key: draw_key(inst.key, pi as u64),
                            override_key: override_key.clone(),
                            object: obj,
                            kind: 0,
                            model: Some(key.clone()),
                            mesh: *mesh,
                            prim: pi,
                            flat: 0,
                            skinned: false,
                            ranges: Vec::new(),
                            gpu: None,
                            morph: false,
                        });
                    }
                }
                PbrDrawable::Terrain { client, spec } => {
                    let key = Self::terrain_key(*client, spec);
                    let p = &self.models[&key].meshes[0][0];
                    grow(p.bounds, &mut lo, &mut hi);
                    draws.push(DrawRef {
                        key: inst.key,
                        override_key: override_key.clone(),
                        object: obj,
                        kind: 0,
                        model: Some(key),
                        mesh: 0,
                        prim: 0,
                        flat: 0,
                        skinned: false,
                        ranges: Vec::new(),
                        gpu: None,
                        morph: false,
                    });
                }
                PbrDrawable::Scene {
                    client,
                    asset,
                    animation: _,
                    morphs,
                } => {
                    let key = format!("{client}@{}:{asset}", assets.revision(ClientId(*client)));
                    let model = self.models[&key].cpu.clone();
                    let global = poses
                        .remove(&inst.key)
                        .context("prepared animation pose missing")?;
                    for (ni, node) in model.nodes.iter().enumerate() {
                        let Some(mesh) = node.mesh else { continue };
                        let world = m * global[ni];
                        let sub = inst.key.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (ni as u64 + 1);
                        let wm = world.to_cols_array_2d();
                        let prev = *self.prev_models.get(&sub).unwrap_or(&wm);
                        next_prev_models.insert(sub, wm);
                        let mut skin = [0u32; 4];
                        if let Some(si) = node.skin {
                            let pal: Vec<[[f32; 4]; 4]> =
                                crysirror_asset::anim::palette(&model, si, ni, &global)
                                    .iter()
                                    .map(|x| x.to_cols_array_2d())
                                    .collect();
                            let prev_pal = self
                                .prev_palettes
                                .get(&sub)
                                .cloned()
                                .unwrap_or_else(|| pal.clone());
                            skin = [
                                palettes.len() as u32,
                                (palettes.len() + pal.len()) as u32,
                                1,
                                0,
                            ];
                            palettes.extend_from_slice(&pal);
                            palettes.extend_from_slice(&prev_pal);
                            next_prev_palettes.insert(sub, pal);
                        }
                        let obj = objects.len() as u32;
                        objects.push(ObjectUniform {
                            model: wm,
                            normal: world.inverse().transpose().to_cols_array_2d(),
                            prev_model: prev,
                            skin,
                        });
                        for (pi, p) in self.models[&key].meshes[mesh].iter().enumerate() {
                            let dk = draw_key(sub, pi as u64);
                            let mut morph = false;
                            if let Some(cp) = model.meshes.get(mesh).and_then(|m| m.get(pi)) {
                                if !morphs.is_empty() && !cp.morphs.is_empty() {
                                    let weight = |name: &str| {
                                        morphs.iter().find(|(n, _)| n == name).map_or(0., |(_, w)| *w)
                                    };
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    for t in &cp.morphs {
                                        std::hash::Hash::hash(&weight(&t.name).to_bits(), &mut h);
                                    }
                                    let hash = std::hash::Hasher::finish(&h);
                                    if self.morph_buffers.get(&dk).is_some_and(|b| b.0 == hash) {
                                        morph = true;
                                    } else if let Some(v) = cp.morphed(weight) {
                                        morph_uploads.push((dk, hash, v));
                                        morph = true;
                                    }
                                    if morph {
                                        morph_live.insert(dk);
                                    }
                                }
                            }
                            let b = p.bounds;
                            for i in 0..8 {
                                let c = Vec3::new(
                                    if i & 1 == 0 { b.0.x } else { b.1.x },
                                    if i & 2 == 0 { b.0.y } else { b.1.y },
                                    if i & 4 == 0 { b.0.z } else { b.1.z },
                                );
                                let w = world.transform_point3(c);
                                lo = lo.min(w);
                                hi = hi.max(w);
                            }
                            draws.push(DrawRef {
                                key: dk,
                                override_key: override_key.clone(),
                                object: obj,
                                kind: 0,
                                model: Some(key.clone()),
                                mesh: mesh as u32,
                                prim: pi,
                                flat: 0,
                                skinned: skin[2] == 1 && p.skin.is_some(),
                                ranges: Vec::new(),
                                gpu: None,
                                morph,
                            });
                        }
                    }
                }
                PbrDrawable::Triangles {
                    vertices,
                    indices,
                    color,
                } => {
                    if indices.is_empty() {
                        continue;
                    }
                    let p = cpu::flat_primitive(vertices, indices);
                    let lin = |c: u8| crate::geometry::linear(c);
                    let mat = PbrMaterial {
                        base_color_factor: [lin(color[0]), lin(color[1]), lin(color[2]), 1.],
                        roughness_factor: 0.7,
                        ..Default::default()
                    };
                    let g = self.prim(&p);
                    if use_rt {
                        flat_cpu.push(Arc::new(p));
                    }
                    grow(g.bounds, &mut lo, &mut hi);
                    let bind = self.material_bind(
                        &Self::material_uniform(&mat),
                        [
                            &self.defaults[0],
                            &self.defaults[1],
                            &self.defaults[2],
                            &self.defaults[1],
                            &self.defaults[3],
                            &self.defaults[1],
                            &self.defaults[0],
                        ],
                    );
                    flat.push((
                        g,
                        GpuMaterial {
                            shader: None,
                            bind,
                            double_sided: false,
                            alpha: AlphaMode::Opaque,
                            rt: RtCoverage::default(),
                        },
                    ));
                    draws.push(DrawRef {
                        key: inst.key,
                        override_key: override_key.clone(),
                        object: obj,
                        kind: 1,
                        model: None,
                        mesh: 0,
                        prim: 0,
                        flat: flat.len() - 1,
                        skinned: false,
                        ranges: Vec::new(),
                        gpu: None,
                        morph: false,
                    });
                }
            }
        }
        // Native RT draws reference the renderer's own GPU streams (ADR 0055).
        let rt_draws = if use_rt {
            let _span = self
                .profiler
                .span(self.profile_frame, "rt.geometry.prepare");
            fn hash<T: std::hash::Hash>(value: T) -> u64 {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                value.hash(&mut h);
                std::hash::Hasher::finish(&h)
            }
            let mut out = Vec::with_capacity(draws.len());
            for draw in &draws {
                let (prim, material, material_id, source) = if draw.kind == 0 {
                    let key = draw.model.as_ref().context("RT model key")?;
                    let model = &self.models[key];
                    let prim = model
                        .meshes
                        .get(draw.mesh as usize)
                        .and_then(|m| m.get(draw.prim))
                        .context("RT model primitive missing")?;
                    (
                        prim,
                        &model.materials[prim.material],
                        format!("{key}#{}", prim.material),
                        hash((key, draw.mesh, draw.prim)),
                    )
                } else {
                    // Flat primitives are rebuilt every frame; their identity is their content.
                    let cpu_prim = &flat_cpu[draw.flat];
                    let bits: Vec<u32> = cpu_prim
                        .vertices
                        .iter()
                        .flat_map(|v| v.position.map(f32::to_bits))
                        .chain(cpu_prim.indices.iter().copied())
                        .collect();
                    let (prim, material) = &flat[draw.flat];
                    (prim, material, String::from("flat"), hash(("flat", &bits)))
                };
                let (material, material_id) = match &draw.override_key {
                    Some(k) => (&self.overrides[k].material, format!("override:{k}")),
                    None => (material, material_id),
                };
                // Blended surfaces cast no raster shadows either.
                if material.alpha.transparent() {
                    continue;
                }
                ensure!(
                    material.shader.is_none(),
                    "hardware RT cannot evaluate custom fragment-discard shaders"
                );
                let mask = material.alpha == AlphaMode::Mask;
                ensure!(
                    !(mask && material.rt.effect),
                    "hardware RT alpha testing cannot bake position/time-dependent effect coverage"
                );
                // Effect presets bake at the origin and time 0: their hit colour
                // in reflections/GI is approximate; opaque coverage is exact.
                let rt_material = Some(crate::rt_pbr::MaterialSource {
                    material: material_id,
                    bind: material.bind.clone(),
                    mask,
                    cutoff: material.rt.cutoff,
                    uv_transform: material.rt.uv_transform,
                    emissive: material.rt.emissive,
                });
                let object = &objects[draw.object as usize];
                let morph = if draw.morph {
                    self.morph_buffers.get(&draw.key)
                } else {
                    None
                };
                let dynamic = draw.skinned || morph.is_some();
                let revision = if dynamic {
                    let pose: &[[[f32; 4]; 4]] = if draw.skinned {
                        &palettes[object.skin[0] as usize..object.skin[1] as usize]
                    } else {
                        &[]
                    };
                    let bits: Vec<u32> = pose.iter().flatten().flatten().map(|v| v.to_bits()).collect();
                    hash((&bits, morph.map(|m| m.0)))
                } else {
                    0
                };
                out.push(crate::rt_pbr::PbrRtDraw {
                    geometry: if dynamic { hash(("dynamic", draw.key)) } else { source },
                    uv_key: source,
                    vertices: morph.map_or_else(|| prim.vertices.clone(), |m| m.1.clone()),
                    vertex_count: u32::try_from(prim.vertex_count)?,
                    indices: prim.indices.clone(),
                    index_count: prim.count,
                    transform: object.model,
                    dynamic,
                    skin: draw
                        .skinned
                        .then(|| {
                            prim.skin.clone().map(|stream| crate::rt_pbr::SkinSource {
                                stream,
                                palette_offset: object.skin[0],
                            })
                        })
                        .flatten(),
                    revision,
                    material: rt_material,
                });
            }
            Some(out)
        } else {
            None
        };
        // Object uniforms with dynamic offsets.
        if objects.len() as u64 > self.object_capacity {
            self.object_capacity = (objects.len() as u64).next_power_of_two();
            self.object_buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pbr objects"),
                size: OBJECT_STRIDE * self.object_capacity,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.object_bind = Self::object_bind(
                &self.gpu.device,
                &self.object_bgl,
                &self.object_buf,
                &self.palette_buf,
            );
        }
        if palettes.len() as u64 > self.palette_capacity {
            self.palette_capacity = (palettes.len() as u64).next_power_of_two();
            self.palette_buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("joint palettes"),
                size: 64 * self.palette_capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.object_bind = Self::object_bind(
                &self.gpu.device,
                &self.object_bgl,
                &self.object_buf,
                &self.palette_buf,
            );
        }
        if !palettes.is_empty() {
            self.gpu
                .queue
                .write_buffer(&self.palette_buf, 0, bytemuck::cast_slice(&palettes));
        }
        let mut obytes = vec![0u8; (objects.len() as u64 * OBJECT_STRIDE) as usize];
        for (i, o) in objects.iter().enumerate() {
            let at = i * OBJECT_STRIDE as usize;
            obytes[at..at + std::mem::size_of::<ObjectUniform>()]
                .copy_from_slice(bytemuck::bytes_of(o));
        }
        if !obytes.is_empty() {
            self.gpu.queue.write_buffer(&self.object_buf, 0, &obytes);
        }

        // Camera, TAA jitter and cascaded sun.
        let cam = &scene.camera;
        let aspect = canvas[0] as f32 / canvas[1] as f32;
        let view = Mat4::look_at_rh(
            Vec3::from_array(cam.eye),
            Vec3::from_array(cam.target),
            Vec3::from_array(cam.up),
        );
        let proj = Mat4::perspective_rh(cam.fov_y_degrees.to_radians(), aspect, cam.near, cam.far);
        let view_proj_nojitter = proj * view;
        self.frame_index = self.frame_index.wrapping_add(1);
        let mut jproj = proj;
        if env.temporal_aa {
            let k = self.frame_index % 16 + 1;
            let jx = (halton(k, 2) - 0.5) * 2.0 / canvas[0] as f32;
            let jy = (halton(k, 3) - 0.5) * 2.0 / canvas[1] as f32;
            jproj.z_axis.x += jx;
            jproj.z_axis.y += jy;
        }
        let view_proj = jproj * view;
        // Cluster LOD cut per draw (ADR 0026). The projected error bound is
        // pixel_error · 2·tan(fov/2) / height; the ratio error/distance is
        // evaluated in object space (invariant under uniform scale).
        let mut lod_stats = LodStats::default();
        let mut lod_jobs: Vec<(usize, LodParams)> = Vec::new();
        if let Some(g) = env.geometry_lod.as_ref().filter(|g| g.gpu) {
            // GPU path: per-draw parameters; selection runs in a compute pass.
            let threshold = g.pixel_error * 2. * (cam.fov_y_degrees.to_radians() * 0.5).tan()
                / canvas[1] as f32;
            let eye = Vec3::from_array(cam.eye);
            let vp = view_proj_nojitter;
            let rows = [vp.row(0), vp.row(1), vp.row(2), vp.row(3)];
            let world_planes: Vec<glam::Vec4> = [
                rows[3] + rows[0],
                rows[3] - rows[0],
                rows[3] + rows[1],
                rows[3] - rows[1],
                rows[2],
                rows[3] - rows[2],
            ]
            .iter()
            .map(|p| *p / p.truncate().length())
            .collect();
            let mut first = 0u32;
            for (di, r) in draws.iter_mut().enumerate() {
                let Some(model) = r.model.as_ref().filter(|_| r.kind == 0) else {
                    continue;
                };
                let Some(lod) = &self.models[model].meshes[r.mesh as usize][r.prim].lod else {
                    continue;
                };
                let m = Mat4::from_cols_array_2d(&objects[r.object as usize].model);
                let local_eye = m.inverse().transform_point3(eye);
                let scale = m
                    .x_axis
                    .truncate()
                    .length()
                    .max(m.y_axis.truncate().length())
                    .max(m.z_axis.truncate().length());
                let slot = lod_jobs.len() as u32;
                let mut planes = [[0f32; 4]; 6];
                for (k, p) in world_planes.iter().enumerate() {
                    planes[k] = (m.transpose() * *p).to_array();
                }
                lod_jobs.push((
                    di,
                    LodParams {
                        eye: [local_eye.x, local_eye.y, local_eye.z, threshold],
                        planes,
                        misc: [scale, 1., 0., 0.],
                        slots: [2 * slot, 2 * slot + 1, lod.clusters.len() as u32, 0],
                        firsts: [first, first + lod.leaf_indices, 0, 0],
                    },
                ));
                first += 2 * lod.leaf_indices;
                r.gpu = Some(slot as u64);
                lod_stats.lod_primitives += 1;
                lod_stats.source_triangles += lod.leaf_indices as usize / 3;
            }
            let draws_n = lod_jobs.len() as u64 * 2;
            if self.lod_gpu.out.size() < first as u64 * 4 || self.lod_gpu.args.size() < draws_n * 20
            {
                let (o, a) = LodGpu::buffers(
                    &self.gpu.device,
                    (first as u64).next_power_of_two(),
                    draws_n.next_power_of_two(),
                );
                self.lod_gpu.out = o;
                self.lod_gpu.args = a;
            }
            let mut init = Vec::with_capacity(lod_jobs.len() * 10);
            for (_, p) in &lod_jobs {
                init.extend_from_slice(&[0u32, 1, p.firsts[0], 0, 0, 0, 1, p.firsts[1], 0, 0]);
            }
            if !init.is_empty() {
                self.gpu
                    .queue
                    .write_buffer(&self.lod_gpu.args, 0, bytemuck::cast_slice(&init));
            }
        }
        if let Some(g) = env.geometry_lod.as_ref().filter(|g| !g.gpu) {
            let threshold = g.pixel_error * 2. * (cam.fov_y_degrees.to_radians() * 0.5).tan()
                / canvas[1] as f32;
            let eye = Vec3::from_array(cam.eye);
            for r in &mut draws {
                let Some(model) = r.model.as_ref().filter(|_| r.kind == 0) else {
                    continue;
                };
                let Some(lod) = &self.models[model].meshes[r.mesh as usize][r.prim].lod else {
                    continue;
                };
                let local_eye = Mat4::from_cols_array_2d(&objects[r.object as usize].model)
                    .inverse()
                    .transform_point3(eye);
                lod_stats.lod_primitives += 1;
                for c in &lod.clusters {
                    if c.lod_error == 0. {
                        lod_stats.source_triangles += c.count as usize / 3;
                    }
                    let own =
                        crysirror_asset::cluster::projected(c.lod_error, c.lod_bounds, local_eye);
                    let parent = crysirror_asset::cluster::projected(
                        c.parent_error,
                        c.parent_bounds,
                        local_eye,
                    );
                    if own <= threshold && parent > threshold {
                        lod_stats.clusters_drawn += 1;
                        lod_stats.drawn_triangles += c.count as usize / 3;
                        match r.ranges.last_mut() {
                            Some(last) if last.end == c.start => last.end = c.start + c.count,
                            _ => r.ranges.push(c.start..c.start + c.count),
                        }
                    }
                }
            }
        }
        self.lod_stats = lod_stats;
        let prev_view_proj = self.prev_view_proj.unwrap_or(view_proj_nojitter);
        if lo.x > hi.x {
            (lo, hi) = (Vec3::splat(-1.), Vec3::splat(1.));
        }
        let scene_radius = ((hi - lo).length() * 0.5).max(0.01);
        let scene_center = (lo + hi) * 0.5;
        let sun = env.sun.clone().unwrap_or(crysirror_scene::view::Sun {
            intensity: 0.,
            shadows: false,
            ..Default::default()
        });
        let l = Vec3::from_array(sun.direction).normalize();
        let up = if l.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        // Practical split scheme (lambda 0.75) over [near, shadow distance].
        let far = env.shadow_distance.min(cam.far);
        let near = cam.near;
        let mut splits = [0f32; CASCADES];
        for (i, s) in splits.iter_mut().enumerate() {
            let p = (i + 1) as f32 / CASCADES as f32;
            let log = near * (far / near).powf(p);
            let lin = near + (far - near) * p;
            *s = 0.75 * log + 0.25 * lin;
        }
        let inv_vp = view_proj_nojitter.inverse();
        let mut cascades = [[[0f32; 4]; 4]; CASCADES];
        let mut prev_split = near;
        let ndc_z = |d: f32| {
            let c = proj * glam::Vec4::new(0., 0., -d, 1.);
            c.z / c.w
        };
        for (i, split) in splits.iter().enumerate() {
            let (z0, z1) = (ndc_z(prev_split), ndc_z(*split));
            let mut corners = Vec::with_capacity(8);
            for z in [z0, z1] {
                for (x, y) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                    let w = inv_vp * glam::Vec4::new(x, y, z, 1.);
                    corners.push(w.truncate() / w.w);
                }
            }
            let center = corners.iter().fold(Vec3::ZERO, |a, c| a + *c) / 8.;
            let radius = corners
                .iter()
                .map(|c| (*c - center).length())
                .fold(0f32, f32::max)
                .max(0.01);
            // Stable cascades: snap the center to shadow texels.
            let light_view = Mat4::look_at_rh(Vec3::ZERO, l, up);
            let texel = 2. * radius / SHADOW_SIZE as f32;
            let mut lc = light_view.transform_point3(center);
            lc.x = (lc.x / texel).floor() * texel;
            lc.y = (lc.y / texel).floor() * texel;
            let center = light_view.inverse().transform_point3(lc);
            // Extend the near plane to the whole scene so off-screen casters count.
            let back = (center - scene_center).length() + scene_radius + radius;
            let lv = Mat4::look_at_rh(center - l * back, center, up);
            let lp = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.01, back + radius);
            cascades[i] = (lp * lv).to_cols_array_2d();
            self.gpu.queue.write_buffer(
                &self.cascade_bufs[i],
                0,
                bytemuck::bytes_of(&ShadowUniform {
                    view_proj: cascades[i],
                    camera_time: [
                        scene.camera.eye[0],
                        scene.camera.eye[1],
                        scene.camera.eye[2],
                        scene.time,
                    ],
                }),
            );
            prev_split = *split;
        }
        self.particles.prepare(
            &self.gpu.device,
            &scene.emitters,
            scene.time,
            Vec3::from_array(cam.eye),
        );
        let e = self.env.as_ref().unwrap();
        let mut fu = FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: inv_vp.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            view_proj_nojitter: view_proj_nojitter.to_cols_array_2d(),
            prev_view_proj: prev_view_proj.to_cols_array_2d(),
            cascades,
            splits,
            camera_pos: [cam.eye[0], cam.eye[1], cam.eye[2], 1.],
            sun_dir: [l.x, l.y, l.z, sun.intensity],
            sun_color: [
                sun.color[0],
                sun.color[1],
                sun.color[2],
                if sun.shadows { 1. } else { 0. },
            ],
            env: [
                env.intensity,
                env.rotation_degrees.to_radians(),
                (PREFILTER_MIPS - 1) as f32,
                1. / SHADOW_SIZE as f32,
            ],
            screen: [
                canvas[0] as f32,
                canvas[1] as f32,
                1. / canvas[0] as f32,
                1. / canvas[1] as f32,
            ],
            misc: [
                env.ambient_occlusion,
                self.frame_index as f32,
                scene.time,
                0.,
            ],
            sh: e.sh,
            clusters: [cam.near, cam.far, 0., 1. / (cam.far / cam.near).ln()],
            fog_color: env
                .fog
                .as_ref()
                .map_or([0.; 4], |f| [f.color[0], f.color[1], f.color[2], f.density]),
            fog_params: env.fog.as_ref().map_or([0.; 4], |f| {
                [
                    f.height_falloff,
                    f.base_height,
                    f.start_distance,
                    f.max_opacity,
                ]
            }),
            fog_sun: env
                .fog
                .as_ref()
                .map_or([0.; 4], |f| [f.sun_scattering, f.sun_exponent, 0., 0.]),
            ssr: env
                .reflections
                .as_ref()
                .map_or([0.; 4], |r| [r.intensity, r.max_roughness, if use_rt && r.ray_traced {1.} else {0.}, 0.]),
            probes: [[0.; 4]; 24],
            vsm_view: [[0.; 4]; 4],
            vsm: [0.; 4],
            vsm2: [0.; 4],
            gi_center: [0.; 4],
            gi_extent: [0.; 4],
            gi_counts: [0.; 4],
            rt_shadow: [if use_rt { 1. } else { 0. }, 0., 0., 0.],
        };
        // Irradiance volume (ADR 0028): the first volume; a few probes refresh
        // per frame, nearest to the camera first. After three complete sweeps
        // with no light, static object, material override, sun or environment
        // change, the rate drops to `idle_probes_per_frame`.
        let mut gi_jobs: Vec<(u32, Vec3)> = Vec::new();
        let mut gi_max_distance = 0f32;
        // Ray-traced probe updates replace raster face captures (ADR 0059).
        let mut rt_gi_jobs: Vec<u32> = Vec::new();
        self.gi.stats.refreshed = 0;
        let e_key_for_gi = e.key.clone();
        if let Some(v) = scene.volumes.iter().min_by_key(|v| v.key) {
            let key = format!("{}:{:?}:{:?}", v.key, v.center, v.spec);
            let counts = v.spec.counts;
            let total = counts.iter().product::<u32>();
            let (c, e) = (Vec3::from_array(v.center), Vec3::from_array(v.spec.extent));
            let cells = (glam::UVec3::from_array(counts) - 1).as_vec3();
            let probe_pos = |i: u32| {
                let idx = glam::UVec3::new(
                    i % counts[0],
                    (i / counts[0]) % counts[1],
                    i / (counts[0] * counts[1]),
                );
                c - e + idx.as_vec3() / cells * 2. * e
            };
            if self.gi.key.as_ref() != Some(&key) {
                self.gpu.queue.write_buffer(
                    &self.local_shadows.matrices,
                    GI_OFFSET,
                    &vec![0u8; (GI_PROBE_BYTES * total as u64) as usize],
                );
                self.gi.key = Some(key);
                self.gi.order.clear();
                self.gi.cursor = 0;
                self.gi.stats.sweeps = 0;
                self.gi.scene_hash = 0;
            }
            let hash = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{:?}", scene.lights).hash(&mut h);
                for d in draws.iter().filter(|d| !d.skinned) {
                    d.key.hash(&mut h);
                    d.model.hash(&mut h);
                    d.override_key.hash(&mut h);
                    bytemuck::bytes_of(&objects[d.object as usize].model).hash(&mut h);
                }
                bytemuck::bytes_of(&[fu.sun_dir, fu.sun_color]).hash(&mut h);
                (&e_key_for_gi, env.intensity.to_bits(), env.show_sky).hash(&mut h);
                h.finish()
            };
            if hash != self.gi.scene_hash {
                self.gi.scene_hash = hash;
                self.gi.stats.sweeps = 0;
                // The sweep in progress started before the change.
                self.gi.order.truncate(self.gi.cursor);
            }
            // Ray-traced updates blend with hysteresis: more sweeps to converge.
            let sweeps_to_converge = if use_rt && v.spec.ray_traced { 12 } else { 3 };
            let converged = self.gi.stats.sweeps >= sweeps_to_converge;
            let budget = if converged {
                v.spec.idle_probes_per_frame
            } else {
                v.spec.probes_per_frame
            }
            .min(total);
            let eye = Vec3::from_array(cam.eye);
            for _ in 0..budget {
                if self.gi.cursor >= self.gi.order.len() {
                    let complete = self.gi.order.len() == total as usize;
                    if complete {
                        self.gi.stats.sweeps += 1;
                    }
                    let mut order: Vec<u32> = (0..total).collect();
                    order.sort_by(|a, b| {
                        (probe_pos(*a) - eye)
                            .length_squared()
                            .total_cmp(&(probe_pos(*b) - eye).length_squared())
                    });
                    self.gi.order = order;
                    self.gi.cursor = 0;
                }
                let i = self.gi.order[self.gi.cursor];
                self.gi.cursor += 1;
                gi_jobs.push((i, probe_pos(i)));
            }
            if self.gi.cursor == self.gi.order.len() && self.gi.order.len() == total as usize {
                self.gi.stats.sweeps += 1;
                self.gi.order.clear();
                self.gi.cursor = 0;
            }
            if use_rt && v.spec.ray_traced {
                rt_gi_jobs = gi_jobs.drain(..).map(|(i, _)| i).collect();
            }
            self.gi.stats.probes = total;
            self.gi.stats.refreshed = (gi_jobs.len() + rt_gi_jobs.len()) as u32;
            self.gi.stats.total_refreshed += (gi_jobs.len() + rt_gi_jobs.len()) as u64;
            self.gi.stats.converged = self.gi.stats.sweeps >= sweeps_to_converge;
            gi_max_distance = 2. * (2. * e / cells).length();
            fu.gi_center = [c.x, c.y, c.z, v.spec.intensity];
            fu.gi_extent = [e.x, e.y, e.z, 1.];
            fu.gi_counts = [
                counts[0] as f32,
                counts[1] as f32,
                counts[2] as f32,
                if v.spec.visibility { 1. } else { 0. },
            ];
        } else {
            self.gi.key = None;
            self.gi.stats = GiStats {
                total_refreshed: self.gi.stats.total_refreshed,
                ..Default::default()
            };
        }
        // Virtual shadow maps (ADR 0027): marks of the previous frame decide
        // which pages are resident; only new or invalidated pages render.
        let vsm_cfg = env.virtual_shadows.clone().filter(|_| sun.shadows);
        let mut vsm_render: Vec<(crate::vsm::PageKey, u32)> = Vec::new();
        let mut vsm_setup: Option<(Mat4, [f32; 2], f32, Vec<[i32; 2]>)> = None;
        let mut vsm_marked: Option<Vec<[i32; 2]>> = None;
        if let Some(cfg) = &vsm_cfg {
            if let Some((rx, centers)) = self.vsm.pending.take() {
                self.gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                rx.recv().context("VSM readback callback missing")??;
                let slice = self.vsm.staging.slice(..);
                let marks: Vec<u32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
                self.vsm.staging.unmap();
                self.vsm.requested = crate::vsm::decode_requests(&marks, &centers);
            }
            let light_view = Mat4::look_at_rh(Vec3::ZERO, l, up);
            let along = l.dot(scene_center);
            let need = [
                along - scene_radius * 1.05 - 1.,
                along + scene_radius * 1.05 + 1.,
            ];
            let depth = match self.vsm.depth {
                Some(d) if d[0] <= need[0] && d[1] >= need[1] => d,
                // Grow with margin so slowly moving casters rarely re-key the cache.
                Some(d) => {
                    let pad = (need[1] - need[0]) * 0.25;
                    [d[0].min(need[0] - pad), d[1].max(need[1] + pad)]
                }
                None => need,
            };
            self.vsm.depth = Some(depth);
            let footprint = 2. * (cam.fov_y_degrees.to_radians() * 0.5).tan() / canvas[1] as f32;
            let centers = crate::vsm::centers(
                &light_view,
                Vec3::from_array(cam.eye),
                cfg.texel_size,
                cfg.levels,
            );
            // Static/dynamic split: casters that appeared, vanished, moved or
            // are skinned (pose changes) dirty the pages under their old and
            // new light-space footprints; everything else stays cached.
            let mut now: HashMap<u64, (Vec3, f32)> = HashMap::new();
            let mut dirty: Vec<(f32, f32, f32)> = Vec::new();
            let mut mark = |c: Vec3, r: f32| {
                let lc = light_view.transform_point3(c);
                dirty.push((lc.x, lc.y, r));
            };
            for d in &draws {
                let m = Mat4::from_cols_array_2d(&objects[d.object as usize].model);
                let b = if d.kind == 0 {
                    self.models[d.model.as_ref().unwrap()].meshes[d.mesh as usize][d.prim].bounds
                } else {
                    flat[d.flat].0.bounds
                };
                let scale = m
                    .x_axis
                    .truncate()
                    .length()
                    .max(m.y_axis.truncate().length())
                    .max(m.z_axis.truncate().length());
                let sphere = (
                    m.transform_point3((b.0 + b.1) * 0.5),
                    (b.1 - b.0).length() * 0.5 * scale,
                );
                match self.vsm.bounds.get(&d.key) {
                    Some(old) if *old == sphere && !d.skinned => {}
                    Some(old) => {
                        mark(old.0, old.1);
                        mark(sphere.0, sphere.1);
                    }
                    None => mark(sphere.0, sphere.1),
                }
                now.insert(d.key, sphere);
            }
            for (k, old) in &self.vsm.bounds {
                if !now.contains_key(k) {
                    mark(old.0, old.1);
                }
            }
            self.vsm.bounds = now;
            let key = crate::vsm::CacheKey {
                light_view: light_view.to_cols_array(),
                depth,
                texel: cfg.texel_size,
                levels: cfg.levels,
            };
            if self.vsm.cache.key.as_ref() == Some(&key) {
                self.vsm.cache.invalidate(cfg.texel_size, &dirty);
            }
            let requested: Vec<_> = self
                .vsm
                .requested
                .iter()
                .copied()
                .filter(|k| k.0 < cfg.levels)
                .collect();
            vsm_render = self.vsm.cache.update(key, &requested);
            let table = self.vsm.cache.table(&centers);
            self.gpu.queue.write_buffer(
                &self.local_shadows.matrices,
                64 * crate::lights::MAX_SHADOW_TILES as u64,
                bytemuck::cast_slice(&table),
            );
            fu.vsm_view = light_view.to_cols_array_2d();
            fu.vsm = [cfg.texel_size, cfg.levels as f32, depth[0], depth[1]];
            fu.vsm2 = [1., footprint, 0., 0.];
            vsm_setup = Some((light_view, depth, footprint, centers));
        } else {
            self.vsm.pending = None;
            self.vsm.requested.clear();
            self.vsm.bounds.clear();
            self.vsm.depth = None;
        }
        // Reflection probe slots: nearest probes first, slots kept per key.
        let env_key = format!("{}:assets:{resource_stamp}", e.key);
        let mut wanted: Vec<&crysirror_scene::view::PbrProbe> = scene.probes.iter().collect();
        let eye = Vec3::from_array(cam.eye);
        wanted.sort_by(|a, b| {
            (Vec3::from_array(a.position) - eye)
                .length()
                .total_cmp(&(Vec3::from_array(b.position) - eye).length())
        });
        wanted.truncate(crate::probes::MAX_PROBES);
        let mut assign: Vec<Option<&crysirror_scene::view::PbrProbe>> =
            vec![None; crate::probes::MAX_PROBES];
        for p in &wanted {
            if let Some(s) = self
                .probes
                .slots
                .iter()
                .position(|c| c.as_ref().is_some_and(|c| c.key == p.key))
            {
                assign[s] = Some(*p);
            }
        }
        for p in &wanted {
            if !assign.iter().flatten().any(|q| q.key == p.key) {
                let s = assign
                    .iter()
                    .position(Option::is_none)
                    .expect("fewer probes than slots");
                assign[s] = Some(*p);
            }
        }
        let mut captures = Vec::new();
        for (s, a) in assign.iter().enumerate() {
            let Some(p) = a else {
                self.probes.slots[s] = None;
                continue;
            };
            let environment = match &p.source {
                Some((c, a)) => format!("source:{c}:{a}"),
                None => env_key.clone(),
            };
            let want = crate::probes::Captured {
                key: p.key,
                revision: p.revision,
                position: p.position,
                environment,
            };
            if self.probes.slots[s].as_ref() != Some(&want) {
                // Authored probes upload their equirect source instead of capturing.
                let authored = match &p.source {
                    Some((c, a)) => {
                        let img = cpu::load_hdr(&assets.read_for(ClientId(*c), a)?)
                            .with_context(|| format!("reflection probe source {a}"))?;
                        ensure!(
                            img.width <= self.gpu.device.limits().max_texture_dimension_2d,
                            "probe source exceeds device texture size"
                        );
                        let levels = equirect_levels(&img);
                        Some((
                            upload_levels(&self.gpu, "probe source", &levels),
                            img.width,
                            levels.len() as u32,
                        ))
                    }
                    None => None,
                };
                captures.push((s, want, authored));
            }
        }
        let mut fu = fu;
        for (s, a) in assign.iter().enumerate() {
            if let Some(p) = a {
                fu.probes[3 * s] = [p.position[0], p.position[1], p.position[2], p.intensity];
                fu.probes[3 * s + 1] = [p.extent[0], p.extent[1], p.extent[2], p.blend_distance];
                fu.probes[3 * s + 2] = [if p.parallax { 1. } else { 0. }, 0., 0., 0.];
            }
        }
        self.probes.stats = crate::probes::ProbeStats {
            submitted: scene.probes.len(),
            active: wanted.len(),
            captured_this_frame: captures.len(),
        };
        let alloc = crate::lights::allocate_shadows(&scene.lights, Vec3::from_array(cam.eye));
        {
            let ls = &mut self.local_shadows;
            let mats: Vec<[[f32; 4]; 4]> = alloc
                .tiles
                .iter()
                .map(|t| t.view_proj.to_cols_array_2d())
                .collect();
            if !mats.is_empty() {
                self.gpu
                    .queue
                    .write_buffer(&ls.matrices, 0, bytemuck::cast_slice(&mats));
            }
            for (t, buf) in alloc.tiles.iter().zip(&ls.tile_bufs) {
                self.gpu.queue.write_buffer(
                    buf,
                    0,
                    bytemuck::bytes_of(&ShadowUniform {
                        view_proj: t.view_proj.to_cols_array_2d(),
                        camera_time: [cam.eye[0], cam.eye[1], cam.eye[2], scene.time],
                    }),
                );
            }
            ls.tiles = alloc.tiles.clone();
            ls.requested = alloc.requested;
            ls.granted = alloc.granted;
        }
        let mut clusters = crate::lights::build(
            &scene.lights,
            &crate::lights::ClusterView {
                view,
                fov_y_radians: cam.fov_y_degrees.to_radians(),
                aspect,
                near: cam.near,
                far: cam.far,
            },
            |i| alloc.slots[i],
        );
        // Hardware RT traces the nearest two shadowed point/spot lights into
        // visibility channels B/A (ADR 0057); others keep atlas shadows. The
        // atlas tiles stay allocated for probe and transparent views.
        let rt_local: Vec<usize> = if use_rt {
            let eye = Vec3::from_array(cam.eye);
            let mut order: Vec<usize> = (0..scene.lights.len())
                .filter(|i| {
                    let l = &scene.lights[*i];
                    l.shadows
                        && matches!(
                            l.shape,
                            crysirror_scene::components::LightShape::Point
                                | crysirror_scene::components::LightShape::Spot { .. }
                        )
                })
                .collect();
            let dist = |i: usize| {
                ((Vec3::from_array(scene.lights[i].position) - eye).length() - scene.lights[i].range).max(0.)
            };
            order.sort_by(|a, b| dist(*a).total_cmp(&dist(*b)));
            order.truncate(2);
            order
        } else {
            Vec::new()
        };
        for (gi, li) in clusters.order.iter().enumerate() {
            if let Some(channel) = rt_local.iter().position(|x| x == li) {
                clusters.lights[gi].cone[3] = channel as f32 + 1.;
            }
        }
        let mut fu = fu;
        fu.rt_shadow[1] = if rt_local.is_empty() { 0. } else { 1. };
        if let Some(g) = &env.ray_traced_gi {
            ensure!(use_rt, "per-pixel ray-traced GI requires the hardware ray-tracing rendering path");
            fu.ssr[3] = g.intensity;
        }
        fu.clusters[2] = clusters.lights.len() as f32;
        self.light_bufs
            .upload(&self.gpu.device, &self.gpu.queue, &clusters);
        self.gpu
            .queue
            .write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&fu));
        let ssao_u = SsaoUniform {
            proj: jproj.to_cols_array_2d(),
            inv_proj: jproj.inverse().to_cols_array_2d(),
            params: [
                0.5,
                0.025,
                env.ambient_occlusion.max(0.),
                (self.frame_index % 64) as f32,
            ],
            screen: fu.screen,
        };
        let tonemap = match env.tonemap {
            Tonemap::PbrNeutral => 0.,
            Tonemap::Aces => 1.,
            Tonemap::None => 2.,
        };
        self.gpu.queue.write_buffer(
            &self.post_buf,
            0,
            bytemuck::cast_slice(&[
                2f32.powf(env.exposure),
                tonemap,
                env.bloom,
                0.,
                if env.auto_exposure.is_some() { 1. } else { 0. },
                env.auto_exposure.as_ref().map_or(0., |a| a.compensation),
                0.,
                0.,
            ]),
        );

        let d = &self.gpu.device;
        let t = self.targets.as_ref().unwrap();
        let frame_bind_with =
            |opaque_depth: &wgpu::TextureView,
             frame_buf: &wgpu::Buffer,
             light_bufs: &crate::lights::LightBuffers| {
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pbr frame"),
                    layout: &self.frame_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: frame_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.shadow_map),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&e.prefiltered),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.env_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&self.brdf_lut),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(&e.source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(
                                t.rt_ao.as_ref().unwrap_or(&t.ao),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::TextureView(opaque_depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: light_bufs.lights.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: light_bufs.grid.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 11,
                            resource: light_bufs.indices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 12,
                            resource: self.local_shadows.matrices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 13,
                            resource: wgpu::BindingResource::TextureView(&self.local_shadows.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 14,
                            resource: wgpu::BindingResource::TextureView(&t.lighting_array),
                        },
                        wgpu::BindGroupEntry {
                            binding: 15,
                            resource: wgpu::BindingResource::TextureView(&self.probes.maps_view),
                        },
                    ],
                })
            };
        let frame_bind_for = |opaque_depth: &wgpu::TextureView| {
            frame_bind_with(opaque_depth, &self.frame_buf, &self.light_bufs)
        };
        let frame_bind = frame_bind_for(&t.pre_depth);
        // Morphed vertex buffers: upload changed blends, drop unused ones.
        for (dk, hash, vertices) in morph_uploads {
            let bytes = bytes_of_vertices(&vertices);
            match self.morph_buffers.get_mut(&dk) {
                Some(b) if b.2 == vertices.len() => {
                    self.gpu.queue.write_buffer(&b.1, 0, &bytes);
                    b.0 = hash;
                }
                _ => {
                    let buffer = self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("pbr morphed vertices"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::VERTEX
                            | wgpu::BufferUsages::COPY_DST
                            | rt_buffer_usage(&self.gpu.device),
                    });
                    self.morph_buffers.insert(dk, (hash, buffer, vertices.len()));
                }
            }
        }
        self.morph_buffers.retain(|k, _| morph_live.contains(k));
        // Do not bind the writable prepass attachment as a sampled texture.
        let pre_frame_bind = frame_bind_for(&self.cascade_layers[0]);
        let morph_buffers = &self.morph_buffers;
        let vertex_buffer = |r: &DrawRef, p: &'_ GpuPrim| -> wgpu::Buffer {
            if r.morph {
                if let Some(b) = morph_buffers.get(&r.key) {
                    return b.1.clone();
                }
            }
            p.vertices.clone()
        };
        let get = |r: &DrawRef| -> (
            &GpuPrim,
            &wgpu::BindGroup,
            bool,
            AlphaMode,
            &SurfacePipelines,
        ) {
            let (prim, mat) = if r.kind == 0 {
                let model = &self.models[r.model.as_ref().unwrap()];
                let p = &model.meshes[r.mesh as usize][r.prim];
                (p, &model.materials[p.material])
            } else {
                let (p, m) = &flat[r.flat];
                (p, m)
            };
            let mat = if let Some(k) = &r.override_key {
                &self.overrides[k].material
            } else {
                mat
            };
            (
                prim,
                &mat.bind,
                mat.double_sided,
                mat.alpha,
                mat.shader
                    .as_ref()
                    .map_or(&self.pipelines, |k| &self.custom_pipelines[k]),
            )
        };
        let mut transparent_draws: Vec<_> =
            draws.iter().filter(|r| get(r).3.transparent()).collect();
        let depth = |r: &DrawRef| {
            let (p, _, _, _, _) = get(r);
            let world = Mat4::from_cols_array_2d(&objects[r.object as usize].model)
                .transform_point3((p.bounds.0 + p.bounds.1) * 0.5);
            -view.transform_point3(world).z
        };
        transparent_draws.sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        let simple_pass = |enc: &mut wgpu::CommandEncoder,
                           label: &str,
                           dst: &wgpu::TextureView,
                           pipe: &wgpu::RenderPipeline,
                           bind: &wgpu::BindGroup| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: dst,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };

        // 0. GPU cluster-LOD selection: one dispatch per clustered draw.
        let lod_out = &self.lod_gpu.out;
        let lod_args = &self.lod_gpu.args;
        if !lod_jobs.is_empty() {
            let mut bytes = vec![0u8; lod_jobs.len() * LOD_PARAMS_STRIDE as usize];
            for (i, (_, p)) in lod_jobs.iter().enumerate() {
                let at = i * LOD_PARAMS_STRIDE as usize;
                bytes[at..at + std::mem::size_of::<LodParams>()]
                    .copy_from_slice(bytemuck::bytes_of(p));
            }
            let pbuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lod select params"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind0 = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lod select"),
                layout: &self.lod_gpu.bgl0,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &pbuf,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<LodParams>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: lod_out.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: lod_args.as_entire_binding(),
                    },
                ],
            });
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("lod select"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.lod_gpu.pipe);
            for (i, (di, p)) in lod_jobs.iter().enumerate() {
                let r = &draws[*di];
                let lod = self.models[r.model.as_ref().unwrap()].meshes[r.mesh as usize][r.prim]
                    .lod
                    .as_ref()
                    .unwrap();
                cpass.set_bind_group(0, &bind0, &[(i as u64 * LOD_PARAMS_STRIDE) as u32]);
                cpass.set_bind_group(1, &lod.gpu_bind, &[]);
                let n = p.slots[2];
                let x = n.clamp(1, 65535);
                cpass.dispatch_workgroups(x, n.div_ceil(x), 1);
            }
        }
        // Draw a primitive: GPU-selected cut (indirect), CPU ranges, or whole.
        macro_rules! draw_prim {
            ($pass:expr, $r:expr, $p:expr, $shadow:expr) => {{
                match (&$p.lod, $r.gpu) {
                    (Some(_), Some(slot)) => {
                        $pass.set_index_buffer(lod_out.slice(..), wgpu::IndexFormat::Uint32);
                        $pass.draw_indexed_indirect(lod_args, (2 * slot + $shadow) * 20);
                    }
                    (Some(l), None) if !$r.ranges.is_empty() => {
                        $pass.set_index_buffer(l.indices.slice(..), wgpu::IndexFormat::Uint32);
                        for range in &$r.ranges {
                            $pass.draw_indexed(range.clone(), 0, 0..1);
                        }
                    }
                    _ => {
                        $pass.set_index_buffer($p.indices.slice(..), wgpu::IndexFormat::Uint32);
                        $pass.draw_indexed(0..$p.count, 0, 0..1);
                    }
                }
            }};
        }

        // 1. Shadow cascades.
        for c in 0..CASCADES {
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cascade light"),
                layout: &self.shadow_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cascade_bufs[c].as_entire_binding(),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("sun cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.cascade_layers[c],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if sun.shadows && vsm_cfg.is_none() {
                pass.set_bind_group(0, &bind, &[]);
                for r in &draws {
                    let (p, mat, _, alpha, pipelines) = get(r);
                    if alpha.transparent() {
                        continue;
                    }
                    pass.set_bind_group(2, mat, &[]);
                    pass.set_pipeline(if r.skinned {
                        &pipelines.shadow_skin
                    } else {
                        &pipelines.shadow
                    });
                    pass.set_bind_group(
                        1,
                        &self.object_bind,
                        &[(r.object as u64 * OBJECT_STRIDE) as u32],
                    );
                    if r.skinned {
                        pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                    }
                    pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                    draw_prim!(pass, r, p, 1);
                }
            }
        }
        // 1b. Local-light shadow atlas: casters within each light's range.
        let page_binds: HashMap<(crate::vsm::PageKey, u32), wgpu::BindGroup> =
            match (&vsm_cfg, &vsm_setup) {
                (Some(cfg), Some((light_view, depth, _, _))) => vsm_render
                    .iter()
                    .map(|(key, phys)| {
                        let proj = crate::vsm::page_projection(cfg.texel_size, *key, *depth);
                        let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("vsm page"),
                            contents: bytemuck::bytes_of(&ShadowUniform {
                                view_proj: (proj * *light_view).to_cols_array_2d(),
                                camera_time: [cam.eye[0], cam.eye[1], cam.eye[2], scene.time],
                            }),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("vsm page"),
                            layout: &self.shadow_bgl,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buf.as_entire_binding(),
                            }],
                        });
                        ((*key, *phys), bind)
                    })
                    .collect(),
                _ => HashMap::new(),
            };
        {
            let ls = &self.local_shadows;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("local shadow atlas"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &ls.view,
                    // Cached virtual shadow pages survive; tiles are cleared one by one.
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let spheres: Vec<(Vec3, f32)> = draws
                .iter()
                .map(|r| {
                    let (p, ..) = get(r);
                    let m = Mat4::from_cols_array_2d(&objects[r.object as usize].model);
                    let scale = m
                        .x_axis
                        .truncate()
                        .length()
                        .max(m.y_axis.truncate().length())
                        .max(m.z_axis.truncate().length());
                    (
                        m.transform_point3((p.bounds.0 + p.bounds.1) * 0.5),
                        (p.bounds.1 - p.bounds.0).length() * 0.5 * scale,
                    )
                })
                .collect();
            macro_rules! draw_casters {
                ($pass:expr, $keep:expr) => {{
                    let pass = &mut $pass;
                    let keep = $keep;
                    for (r, (c, rad)) in draws.iter().zip(&spheres) {
                        if !keep(*c, *rad) {
                            continue;
                        }
                        let (p, mat, _, alpha, pipelines) = get(r);
                        if alpha.transparent() {
                            continue;
                        }
                        pass.set_bind_group(2, mat, &[]);
                        pass.set_pipeline(if r.skinned {
                            &pipelines.shadow_skin
                        } else {
                            &pipelines.shadow
                        });
                        pass.set_bind_group(
                            1,
                            &self.object_bind,
                            &[(r.object as u64 * OBJECT_STRIDE) as u32],
                        );
                        if r.skinned {
                            pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                        }
                        pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                        draw_prim!(pass, r, p, 1);
                    }
                }};
            }
            // Virtual shadow pages: clear and draw casters overlapping the page.
            if let (Some(cfg), Some((light_view, _, _, _))) = (&vsm_cfg, &vsm_setup) {
                let page_px = crate::vsm::PAGE;
                for (key, phys) in &vsm_render {
                    let (x, y) = crate::vsm::phys_rect(*phys);
                    pass.set_viewport(x as f32, y as f32, page_px as f32, page_px as f32, 0., 1.);
                    pass.set_scissor_rect(x, y, page_px, page_px);
                    pass.set_pipeline(&self.vsm.clear_pipe);
                    pass.draw(0..3, 0..1);
                    pass.set_bind_group(0, &page_binds[&(*key, *phys)], &[]);
                    let pw = crate::vsm::page_world(cfg.texel_size, key.0);
                    let mid = glam::Vec2::new((key.1 as f32 + 0.5) * pw, (key.2 as f32 + 0.5) * pw);
                    draw_casters!(pass, |c: Vec3, rad: f32| {
                        let lc = light_view.transform_point3(c);
                        (lc.x - mid.x).abs() <= pw * 0.5 + rad
                            && (lc.y - mid.y).abs() <= pw * 0.5 + rad
                    });
                }
            }
            for (k, tile) in ls.tiles.iter().enumerate() {
                let (x, y, size) = crate::lights::tile_rect(k);
                pass.set_viewport(x as f32, y as f32, size as f32, size as f32, 0., 1.);
                pass.set_scissor_rect(x, y, size, size);
                pass.set_pipeline(&self.vsm.clear_pipe);
                pass.draw(0..3, 0..1);
                pass.set_bind_group(0, &ls.tile_binds[k], &[]);
                for (r, (c, rad)) in draws.iter().zip(&spheres) {
                    if (*c - tile.position).length() > tile.range + rad {
                        continue;
                    }
                    let (p, mat, _, alpha, pipelines) = get(r);
                    if alpha.transparent() {
                        continue;
                    }
                    pass.set_bind_group(2, mat, &[]);
                    pass.set_pipeline(if r.skinned {
                        &pipelines.shadow_skin
                    } else {
                        &pipelines.shadow
                    });
                    pass.set_bind_group(
                        1,
                        &self.object_bind,
                        &[(r.object as u64 * OBJECT_STRIDE) as u32],
                    );
                    if r.skinned {
                        pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                    }
                    pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                    draw_prim!(pass, r, p, 1);
                }
            }
        }
        // 1c. Reflection captures (ADR 0024): six 90° faces of the opaque scene
        // and sky (no AO/SSR/probes; sun cascades are the main view's), then
        // cube → equirect, a box-filtered mip chain and the GGX prefilter.
        // Render the six capture faces around a point into `probes.faces`
        // (reflection captures, ADR 0024; irradiance probes, ADR 0028).
        // Irradiance captures ($gi) use the small GI targets, draw back faces
        // (to detect probes inside geometry) and write hit distances to alpha.
        let cap_spheres: Vec<(Vec3, f32)> = if captures.is_empty() && gi_jobs.is_empty() {
            Vec::new()
        } else {
            draws
                .iter()
                .map(|r| {
                    let (p, ..) = get(r);
                    let m = Mat4::from_cols_array_2d(&objects[r.object as usize].model);
                    let scale = m
                        .x_axis
                        .truncate()
                        .length()
                        .max(m.y_axis.truncate().length())
                        .max(m.z_axis.truncate().length());
                    (
                        m.transform_point3((p.bounds.0 + p.bounds.1) * 0.5),
                        (p.bounds.1 - p.bounds.0).length() * 0.5 * scale,
                    )
                })
                .collect()
        };
        macro_rules! capture_faces {
            ($pos:expr, $gi:expr) => {{
                let pr = &self.probes;
                let pos: Vec3 = $pos;
                let gi: bool = $gi;
                let face_px = if gi {
                    crate::probes::GI_FACE
                } else {
                    crate::probes::FACE
                } as f32;
                for (k, (f, u)) in crate::probes::FACES.iter().enumerate() {
                    let fview = Mat4::look_at_rh(pos, pos + *f, *u);
                    let fproj =
                        Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1., cam.near, cam.far);
                    let vp = (fproj * fview).to_cols_array_2d();
                    let mut ffu = fu;
                    ffu.view_proj = vp;
                    ffu.view_proj_nojitter = vp;
                    ffu.prev_view_proj = vp;
                    ffu.inv_view_proj = (fproj * fview).inverse().to_cols_array_2d();
                    ffu.view = fview.to_cols_array_2d();
                    ffu.camera_pos = [pos.x, pos.y, pos.z, 1.];
                    ffu.screen = [face_px, face_px, 1. / face_px, 1. / face_px];
                    ffu.misc[0] = 0.;
                    ffu.misc[3] = if gi { 1. } else { 0. };
                    ffu.ssr = [0.; 4];
                    ffu.probes = [[0.; 4]; 24];
                    ffu.rt_shadow = [0.; 4];
                    let cl = crate::lights::build(
                        &scene.lights,
                        &crate::lights::ClusterView {
                            view: fview,
                            fov_y_radians: std::f32::consts::FRAC_PI_2,
                            aspect: 1.,
                            near: cam.near,
                            far: cam.far,
                        },
                        |i| alloc.slots[i],
                    );
                    ffu.clusters[2] = cl.lights.len() as f32;
                    let mut lb = crate::lights::LightBuffers::new(d);
                    lb.upload(d, &self.gpu.queue, &cl);
                    let fbuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("probe face frame"),
                        contents: bytemuck::bytes_of(&ffu),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let bind = frame_bind_with(&self.cascade_layers[0], &fbuf, &lb);
                    let face_view = crate::probes::ProbeResources::layer_view(
                        if gi { &pr.gi_faces } else { &pr.faces },
                        k as u32,
                        0,
                    );
                    // Store, not Discard: wgpu tracks every discarded attachment
                    // per encoder and rescans that list on each new discard, which
                    // made hundreds of capture passes per frame quadratic.
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        multiview_mask: None,
                        label: Some("probe capture face"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            depth_slice: None,
                            view: if gi {
                                &pr.gi_msaa_color
                            } else {
                                &pr.msaa_color
                            },
                            resolve_target: Some(&face_view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: if gi {
                                &pr.gi_msaa_depth
                            } else {
                                &pr.msaa_depth
                            },
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_bind_group(0, &bind, &[]);
                    let right = f.cross(*u);
                    let planes = [*f + right, *f - right, *f + *u, *f - *u]
                        .map(|n| n * std::f32::consts::FRAC_1_SQRT_2);
                    for (r, (c, rad)) in draws.iter().zip(&cap_spheres) {
                        if planes.iter().any(|n| n.dot(*c - pos) < -rad) {
                            continue;
                        }
                        let (p, mat, double, alpha, pipelines) = get(r);
                        if alpha.transparent() {
                            continue;
                        }
                        pass.set_pipeline(match (r.skinned, double || gi) {
                            (false, false) => &pipelines.opaque,
                            (false, true) => &pipelines.opaque_double,
                            (true, false) => &pipelines.opaque_skin,
                            (true, true) => &pipelines.opaque_skin_double,
                        });
                        if r.skinned {
                            pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                        }
                        pass.set_bind_group(1, mat, &[]);
                        pass.set_bind_group(
                            2,
                            &self.object_bind,
                            &[(r.object as u64 * OBJECT_STRIDE) as u32],
                        );
                        pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                        draw_prim!(pass, r, p, 1);
                    }
                    if env.show_sky {
                        pass.set_pipeline(&self.sky);
                        pass.set_bind_group(0, &bind, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
            }};
        }
        for (slot, want, authored) in &captures {
            let pr = &self.probes;
            let (source, source_width, source_mips) = match authored {
                Some((tex, w, m)) => (tex.create_view(&Default::default()), *w, *m),
                None => {
                    capture_faces!(Vec3::from_array(want.position), false);
                    let faces_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("probe faces"),
                        layout: &pr.bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: pr.faces_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&pr.faces_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.linear),
                            },
                        ],
                    });
                    simple_pass(
                        encoder,
                        "probe equirect",
                        &crate::probes::ProbeResources::layer_view(&pr.source, 0, 0),
                        &pr.pipeline,
                        &faces_bind,
                    );
                    for m in 1..pr.source_mips {
                        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("probe source mip"),
                            layout: &self.step_bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: self.post_buf.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(
                                        &crate::probes::ProbeResources::layer_view(
                                            &pr.source,
                                            0,
                                            m - 1,
                                        ),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&self.linear),
                                },
                            ],
                        });
                        simple_pass(
                            encoder,
                            "probe source mip",
                            &crate::probes::ProbeResources::layer_view(&pr.source, 0, m),
                            &self.copy_hdr,
                            &bind,
                        );
                    }
                    (
                        pr.source.create_view(&Default::default()),
                        crate::probes::PROBE_W,
                        pr.source_mips,
                    )
                }
            };
            for mip in 0..PREFILTER_MIPS {
                let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("probe prefilter"),
                    contents: bytemuck::cast_slice(&[
                        mip as f32 / (PREFILTER_MIPS - 1) as f32,
                        source_width as f32,
                        source_mips as f32,
                        0.,
                    ]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("probe prefilter"),
                    layout: &self.step_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ubuf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.env_sampler),
                        },
                    ],
                });
                simple_pass(
                    encoder,
                    "probe prefilter",
                    &crate::probes::ProbeResources::layer_view(&pr.maps, *slot as u32, mip),
                    &self.prefilter,
                    &bind,
                );
            }
        }
        // 1d. Irradiance probe refreshes: capture faces, project to SH L1.
        for (index, pos) in &gi_jobs {
            capture_faces!(*pos, true);
            let mut faces = [[0f32; 4]; 12];
            for (k, (f, u)) in crate::probes::FACES.iter().enumerate() {
                faces[2 * k] = [f.x, f.y, f.z, 0.];
                faces[2 * k + 1] = [u.x, u.y, u.z, 0.];
            }
            let job = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gi job"),
                contents: bytemuck::bytes_of(&GiJob {
                    probe: [
                        *index,
                        (GI_OFFSET / 16) as u32,
                        (GI_VIS_OFFSET / 16) as u32,
                        0,
                    ],
                    faces,
                    params: [gi_max_distance, GI_BACKFACE_LIMIT, 0., 0.],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gi project"),
                layout: &self.gi.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.probes.gi_faces_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.local_shadows.matrices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: job.as_entire_binding(),
                    },
                ],
            });
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi project"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.gi.pipe);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(1, 1, 1);
        }
        // 2. Depth / view normal / motion prepass.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("prepass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        depth_slice: None,
                        view: &t.normal,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.,
                                g: 0.,
                                b: 1.,
                                a: 0.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        depth_slice: None,
                        view: &t.velocity,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.pre_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &pre_frame_bind, &[]);
            for r in &draws {
                let (p, mat, double, alpha, pipelines) = get(r);
                if alpha.transparent() {
                    continue;
                }
                pass.set_pipeline(match (r.skinned, double) {
                    (false, false) => &pipelines.prepass,
                    (false, true) => &pipelines.prepass_double,
                    (true, false) => &pipelines.prepass_skin,
                    (true, true) => &pipelines.prepass_skin_double,
                });
                if r.skinned {
                    pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                }
                pass.set_bind_group(1, mat, &[]);
                pass.set_bind_group(
                    2,
                    &self.object_bind,
                    &[(r.object as u64 * OBJECT_STRIDE) as u32],
                );
                pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                draw_prim!(pass, r, p, 0);
            }
        }
        // 2b. Mark the virtual shadow pages this frame's pixels sample.
        if let (Some(cfg), Some((light_view, _, footprint, centers))) = (&vsm_cfg, &vsm_setup) {
            if self.vsm.pending.is_none() {
                let mut c = [[0i32; 4]; 8];
                for (l, v) in centers.iter().enumerate() {
                    c[l / 2][(l % 2) * 2] = v[0];
                    c[l / 2][(l % 2) * 2 + 1] = v[1];
                }
                let u = MarkUniform {
                    inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                    light_view: light_view.to_cols_array_2d(),
                    camera: [cam.eye[0], cam.eye[1], cam.eye[2], *footprint],
                    params: [
                        cfg.texel_size,
                        cfg.levels as f32,
                        canvas[0] as f32,
                        canvas[1] as f32,
                    ],
                    centers: c,
                };
                let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vsm mark"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vsm mark"),
                    layout: &self.vsm.mark_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ubuf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&t.pre_depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.vsm.requests.as_entire_binding(),
                        },
                    ],
                });
                encoder.clear_buffer(&self.vsm.requests, 0, None);
                {
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("vsm mark"),
                        timestamp_writes: None,
                    });
                    cpass.set_pipeline(&self.vsm.mark_pipe);
                    cpass.set_bind_group(0, &bind, &[]);
                    cpass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
                }
                encoder.copy_buffer_to_buffer(
                    &self.vsm.requests,
                    0,
                    &self.vsm.staging,
                    0,
                    self.vsm.requests.size(),
                );
                vsm_marked = Some(centers.clone());
            }
        }
        // 3. SSAO + blur.
        {
            let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ssao"),
                contents: bytemuck::bytes_of(&ssao_u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let raw = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao"),
                layout: &self.ssao_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&t.pre_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&t.normal),
                    },
                ],
            });
            simple_pass(encoder, "ssao", &t.ao_raw, &self.ssao_pipe, &raw);
            let blur = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao blur"),
                layout: &self.blur_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&t.pre_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&t.normal),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&t.ao_raw),
                    },
                ],
            });
            simple_pass(encoder, "ssao blur", &t.ao, &self.blur_pipe, &blur);
        }
        if let Some(rt) = &mut self.hardware_rt {
            let towards = (-l).normalize_or_zero();
            ensure!(towards.length_squared() > 0.5, "hardware shadow sun direction must be nonzero");
            let shadow_view = crate::rt_pbr::ShadowView {
                inverse_view_projection: view_proj.inverse(),
                view_projection: view_proj,
                view,
                towards_sun: towards,
                angular_diameter_degrees: sun.angular_diameter_degrees,
                local: [0, 1].map(|k| {
                    rt_local.get(k).map(|i| {
                        let l = &scene.lights[*i];
                        crate::rt_pbr::LocalShadow {
                            position: Vec3::from_array(l.position),
                            radius: l.source_radius,
                            range: l.range,
                        }
                    })
                }),
            };
            rt.encode_shadows(
                &self.gpu,
                rt_draws.as_deref().unwrap_or(&[]),
                &self.material_bgl,
                &self.palette_buf,
                &shadow_view,
                canvas,
                &t.pre_depth,
                &t.normal,
                &t.ao,
                t.rt_ao.as_ref().context("RT target absent")?,
                encoder,
            )?;
            self.hardware_encoded = true;
            if !rt_gi_jobs.is_empty() {
                let e = self.env.as_ref().context("environment for RT probes")?;
                let gi_view = crate::rt_pbr::GiView {
                    sun_dir: fu.sun_dir,
                    sun_color: fu.sun_color,
                    env_intensity: fu.env[0],
                    env_rotation: fu.env[1],
                    gi_center: fu.gi_center,
                    gi_extent: fu.gi_extent,
                    gi_counts: fu.gi_counts,
                    gi_offset: (GI_OFFSET / 16) as u32,
                    visibility_offset: (GI_VIS_OFFSET / 16) as u32,
                    max_distance: gi_max_distance,
                    backface_limit: GI_BACKFACE_LIMIT,
                    light_count: clusters.lights.len() as u32,
                };
                rt.encode_gi(
                    &self.gpu,
                    &gi_view,
                    &rt_gi_jobs,
                    &e.source,
                    &self.env_sampler,
                    &self.local_shadows.matrices,
                    &self.light_bufs.lights,
                    encoder,
                )?;
            }
        }
        // 3b. Reflections: ray traced against the scene on the hardware RT
        // path when requested (ADR 0058), otherwise screen space against the
        // prepass, colored by the previous frame.
        let mut traced_reflections = false;
        if let (Some(r), Some(rt)) = (&env.reflections, &mut self.hardware_rt) {
            if r.ray_traced {
                let reflection_view = crate::rt_pbr::ReflectionView {
                    inverse_view_projection: view_proj.inverse(),
                    view_projection: view_proj,
                    view,
                    previous_view_projection: t.prev_valid.then_some(prev_view_proj),
                    camera: Vec3::from_array(cam.eye),
                    sun_dir: fu.sun_dir,
                    sun_color: fu.sun_color,
                    env_intensity: fu.env[0],
                    env_rotation: fu.env[1],
                    sh: fu.sh,
                    max_distance: r.max_distance,
                    samples: r.ray_samples, diffuse: false, light_count: clusters.lights.len() as u32,
                };
                traced_reflections = rt.encode_reflections(
                    &self.gpu,
                    &reflection_view,
                    canvas,
                    &t.pre_depth,
                    &t.normal,
                    &t.prev_hdr,
                    &self.linear,
                    &t.ssr,
                    &self.light_bufs.lights,
                    &self.env.as_ref().context("environment for RT reflection")?.source,
                    encoder,
                )?;
            }
        }
        if let (Some(g), Some(rt)) = (&env.ray_traced_gi, &mut self.hardware_rt) {
            let view = crate::rt_pbr::ReflectionView {
                inverse_view_projection: view_proj.inverse(), view_projection: view_proj, view,
                previous_view_projection: None, camera: Vec3::from_array(cam.eye),
                sun_dir: fu.sun_dir, sun_color: fu.sun_color,
                env_intensity: fu.env[0], env_rotation: fu.env[1], sh: fu.sh,
                max_distance: g.max_distance, samples: g.samples, diffuse: true,
                light_count: clusters.lights.len() as u32,
            };
            let traced = rt.encode_reflections(&self.gpu, &view, canvas, &t.pre_depth, &t.normal,
                &t.prev_hdr, &self.linear, &t.pixel_gi, &self.light_bufs.lights,
                &self.env.as_ref().context("environment for pixel GI")?.source, encoder)?;
            ensure!(traced, "per-pixel RT GI requires nonempty traceable scene geometry");
        }
        if let Some(r) = env.reflections.as_ref().filter(|_| !traced_reflections) {
            let u = SsrUniform {
                proj: jproj.to_cols_array_2d(),
                inv_proj: jproj.inverse().to_cols_array_2d(),
                reproject: (prev_view_proj * view.inverse()).to_cols_array_2d(),
                params: [
                    r.max_distance,
                    r.thickness,
                    r.steps as f32,
                    if t.prev_valid { 1. } else { 0. },
                ],
                screen: fu.screen,
                misc: [(self.frame_index % 64) as f32, cam.near, 0., 0.],
            };
            let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ssr"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssr"),
                layout: &self.ssr_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&t.pre_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&t.normal),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&t.prev_hdr),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            simple_pass(encoder, "ssr", &t.ssr, &self.ssr_pipe, &bind);
        }
        // 4. Opaque + sky, 4x MSAA, resolved into HDR.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("pbr opaque"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &t.msaa_color,
                    resolve_target: Some(&t.resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.msaa_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &frame_bind, &[]);
            for r in &draws {
                let (p, mat, double, alpha, pipelines) = get(r);
                if alpha.transparent() {
                    continue;
                }
                pass.set_pipeline(match (r.skinned, double) {
                    (false, false) => &pipelines.opaque,
                    (false, true) => &pipelines.opaque_double,
                    (true, false) => &pipelines.opaque_skin,
                    (true, true) => &pipelines.opaque_skin_double,
                });
                if r.skinned {
                    pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                }
                pass.set_bind_group(1, mat, &[]);
                pass.set_bind_group(
                    2,
                    &self.object_bind,
                    &[(r.object as u64 * OBJECT_STRIDE) as u32],
                );
                pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                draw_prim!(pass, r, p, 0);
            }
            if env.show_sky {
                pass.set_pipeline(&self.sky);
                pass.set_bind_group(0, &frame_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            self.particles.draw(&mut pass, &frame_bind);
        }
        // 5. TAA resolve into the history ping-pong.
        let write = (self.frame_index % 2) as usize;
        let hdr_out: &wgpu::TextureView = if env.temporal_aa {
            let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("taa"),
                contents: bytemuck::cast_slice(&[
                    1. / canvas[0] as f32,
                    1. / canvas[1] as f32,
                    if t.history_valid { 1. } else { 0. },
                    0.1,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("taa"),
                layout: &self.taa_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&t.resolved),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&t.history[1 - write]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&t.velocity),
                    },
                ],
            });
            simple_pass(encoder, "taa", &t.history[write], &self.taa_pipe, &bind);
            &t.history[write]
        } else {
            &t.resolved
        };
        let hdr_out = if transparent_draws.is_empty() {
            hdr_out
        } else {
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("copy opaque history"),
                layout: &self.step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.post_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(hdr_out),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            simple_pass(
                encoder,
                "opaque HDR copy",
                &t.composite,
                &self.copy_hdr,
                &bind,
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    multiview_mask: None,
                    label: Some("sorted HDR transparency"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        depth_slice: None,
                        view: &t.composite,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.pre_depth,
                        depth_ops: None,
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_bind_group(0, &frame_bind, &[]);
                for r in transparent_draws {
                    let (p, mat, double, alpha, pipelines) = get(r);
                    let mode = match alpha {
                        AlphaMode::Blend => 0,
                        AlphaMode::Additive => 1,
                        AlphaMode::Premultiplied => 2,
                        _ => unreachable!(),
                    };
                    pass.set_pipeline(
                        &pipelines.transparent
                            [mode * 4 + usize::from(r.skinned) * 2 + usize::from(double)],
                    );
                    if r.skinned {
                        pass.set_vertex_buffer(1, p.skin.as_ref().unwrap().slice(..));
                    }
                    pass.set_bind_group(1, mat, &[]);
                    pass.set_bind_group(
                        2,
                        &self.object_bind,
                        &[(r.object as u64 * OBJECT_STRIDE) as u32],
                    );
                    pass.set_vertex_buffer(0, vertex_buffer(r, p).slice(..));
                    draw_prim!(pass, r, p, 0);
                }
            }
            &t.composite
        };
        // 5b. Keep this frame's HDR color for next frame's reflections.
        let reflections = env.reflections.is_some();
        if reflections {
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("previous HDR copy"),
                layout: &self.step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.post_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(hdr_out),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            simple_pass(
                encoder,
                "previous HDR copy",
                &t.prev_hdr,
                &self.copy_hdr,
                &bind,
            );
        }
        // 6. Bloom chain.
        let step = |src: &wgpu::TextureView, texel: [f32; 2], first: bool| {
            let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bloom step"),
                contents: bytemuck::cast_slice(&[
                    texel[0],
                    texel[1],
                    if first { 1. } else { 0. },
                    0.,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom step"),
                layout: &self.step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            })
        };
        let run = |enc: &mut wgpu::CommandEncoder,
                   dst: &wgpu::TextureView,
                   pipe: &wgpu::RenderPipeline,
                   bind: &wgpu::BindGroup,
                   load: wgpu::LoadOp<wgpu::Color>| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: dst,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        let mut src_size = canvas;
        let mut src = hdr_out;
        for (i, (dst, size)) in t.bloom.iter().enumerate() {
            let b = step(
                src,
                [1. / src_size[0] as f32, 1. / src_size[1] as f32],
                i == 0,
            );
            run(
                encoder,
                dst,
                &self.bloom_down,
                &b,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            );
            src = dst;
            src_size = *size;
        }
        for i in (1..t.bloom.len()).rev() {
            let (small, s) = &t.bloom[i];
            let b = step(small, [1. / s[0] as f32, 1. / s[1] as f32], false);
            run(
                encoder,
                &t.bloom[i - 1].0,
                &self.bloom_up,
                &b,
                wgpu::LoadOp::Load,
            );
        }
        // 6b. Auto exposure: meter the HDR frame into the next state texture.
        if let Some(ae) = &env.auto_exposure {
            let x = &mut self.exposure;
            let dt = x.last_time.map_or(0., |t| (scene.time - t).clamp(0., 0.25));
            x.last_time = Some(scene.time);
            self.gpu.queue.write_buffer(
                &x.buf,
                0,
                bytemuck::cast_slice(&[
                    ae.min_log2,
                    ae.max_log2,
                    ae.speed_up,
                    ae.speed_down,
                    dt,
                    if x.valid { 1. } else { 0. },
                    0.,
                    0.,
                ]),
            );
            let (prev, next) = (x.current, 1 - x.current);
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("auto exposure"),
                layout: &x.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: x.buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(hdr_out),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&x.state[prev].1),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("auto exposure"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &x.state[next].1,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&x.pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            x.current = next;
            // While virtual shadow pages are still missing (first frames) the
            // sun is unshadowed; do not let that seed the exposure history.
            x.valid = vsm_cfg.is_none() || self.vsm.cache.stats.resident > 0;
        } else {
            self.exposure.valid = false;
            self.exposure.last_time = None;
        }
        // 7. Tonemap into the caller's target.
        let post_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap"),
            layout: &self.post_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.post_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(hdr_out),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&t.bloom[0].0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &self.exposure.state[self.exposure.current].1,
                    ),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("tonemap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: target,
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
            pass.set_viewport(vp[0], vp[1], vp[2], vp[3], 0., 1.);
            pass.set_pipeline(&self.tonemap);
            pass.set_bind_group(0, &post_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // History for the next frame.
        self.targets.as_mut().unwrap().history_valid = env.temporal_aa;
        self.targets.as_mut().unwrap().prev_valid = reflections;
        for (slot, want, _) in captures {
            self.probes.slots[slot] = Some(want);
        }
        self.vsm.copied = vsm_marked;
        self.prev_view_proj = Some(view_proj_nojitter);
        self.prev_models = next_prev_models;
        self.prev_palettes = next_prev_palettes;
        Ok(())
    }
}

/// A simple physically plausible sky gradient + ground for scenes without an HDR.
fn procedural_sky(w: u32, h: u32) -> cpu::HdrImage {
    let mut rgb = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let theta = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
        let up = theta.cos();
        for _ in 0..w {
            let c = if up >= 0. {
                let t = up.powf(0.5);
                Vec3::new(0.9, 1.0, 1.1).lerp(Vec3::new(0.25, 0.45, 0.95), t) * 1.2
            } else {
                Vec3::new(0.25, 0.22, 0.2) * (1. + up * 0.5)
            };
            rgb.push(c.to_array());
        }
    }
    cpu::HdrImage {
        width: w,
        height: h,
        rgb,
    }
}

/// Project an equirectangular image onto SH9 and fold in the clamped-cosine
/// convolution and basis constants, so the shader evaluates irradiance with
/// plain polynomials (Ramamoorthi & Hanrahan 2001).
fn project_sh9(level: &(u32, u32, Vec<[f32; 3]>)) -> [[f32; 4]; 9] {
    let (w, h, px) = level;
    let mut c = [[0f64; 3]; 9];
    let pi = std::f64::consts::PI;
    for y in 0..*h {
        let theta = (y as f64 + 0.5) / *h as f64 * pi;
        let d_omega = (2. * pi / *w as f64) * (pi / *h as f64) * theta.sin();
        for x in 0..*w {
            let phi = ((x as f64 + 0.5) / *w as f64 - 0.5) * 2. * pi;
            let (dx, dy, dz) = (
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let basis = [
                1.,
                dy,
                dz,
                dx,
                dx * dy,
                dy * dz,
                3. * dz * dz - 1.,
                dx * dz,
                dx * dx - dy * dy,
            ];
            let p = px[(y * w + x) as usize];
            for (i, b) in basis.iter().enumerate() {
                for k in 0..3 {
                    c[i][k] += p[k] as f64 * b * d_omega;
                }
            }
        }
    }
    // K_lm^2 * A_l: projection uses the polynomial basis, so multiply by the
    // squared normalization once here.
    let k = [
        0.282095f64,
        0.488603,
        0.488603,
        0.488603,
        1.092548,
        1.092548,
        0.315392,
        1.092548,
        0.546274,
    ];
    let a = [
        pi,
        2. * pi / 3.,
        2. * pi / 3.,
        2. * pi / 3.,
        pi / 4.,
        pi / 4.,
        pi / 4.,
        pi / 4.,
        pi / 4.,
    ];
    let mut out = [[0f32; 4]; 9];
    for i in 0..9 {
        for ch in 0..3 {
            out[i][ch] = (c[i][ch] * k[i] * k[i] * a[i]) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sh9_of_constant_environment_gives_pi_times_radiance() {
        // Irradiance of a uniform radiance L is pi * L for every normal.
        let px = vec![[2.0f32, 1.0, 0.5]; 64 * 32];
        let sh = project_sh9(&(64, 32, px));
        assert!(
            (sh[0][0] - 2.0 * std::f32::consts::PI).abs() < 0.02,
            "{:?}",
            sh[0]
        );
        for band in &sh[1..] {
            assert!(band[0].abs() < 0.02);
        }
    }
}
