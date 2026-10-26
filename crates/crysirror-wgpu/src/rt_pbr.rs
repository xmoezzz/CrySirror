// SPDX-License-Identifier: MPL-2.0
//! Instanced native ray-query scene for the PBR renderer (ADR 0051, 0055).
//!
//! BLAS inputs are the renderer's own GPU vertex/index streams, so static
//! meshes are never copied to the CPU. Skinned and morphed instances own a
//! persistent dynamic BLAS: a compute pass applies the current joint palette
//! to the (morphed) vertex stream and the BLAS is rebuilt in place only when
//! its pose or morph weights changed. wgpu 29 implements `PreferUpdate` as a
//! full rebuild, so this is a fast-build rebuild, not a refit.
//!
//! Alpha-tested (MASK) casters keep per-triangle UVs gathered on the GPU and
//! a shared alpha atlas baked from each material with the raster coverage
//! function; shadow rays re-trace past transparent hits.
use crate::Gpu;
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeMap, HashMap, HashSet};
use wgpu::util::DeviceExt;

/// Tiles of the shared alpha atlas (ATLAS / TILE squared slots).
const ATLAS: u32 = 2048;
const TILE: u32 = 128;
const SLOTS: usize = ((ATLAS / TILE) * (ATLAS / TILE)) as usize;
/// Linear base colour (stored sRGB-encoded) and coverage alpha per material.
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Dynamic (skinned/morphed) BLAS kept resident at once.
pub const MAX_DYNAMIC: usize = 256;
/// Corners with gathered attributes (UV and normal, 8 bytes each).
const MAX_CORNERS: u64 = 3 * crate::rt_capabilities::MAX_RT_PRIMITIVES as u64;

/// What a material contributes to ray-traced coverage and hit shading.
#[derive(Clone)]
pub(crate) struct MaterialSource {
    /// Stable identity of the material (model key + index or override key).
    pub material: String,
    pub bind: wgpu::BindGroup,
    /// Alpha-tested: rays re-trace past texels below `cutoff`.
    pub mask: bool,
    pub cutoff: f32,
    pub uv_transform: [f32; 4],
    /// Linear emitted radiance (factor × strength; emissive textures are not baked).
    pub emissive: [f32; 3],
}

/// Joint palette input of a skinned instance.
#[derive(Clone)]
pub(crate) struct SkinSource {
    /// Joints u16x4 + weights f32x4 per vertex (24 bytes).
    pub stream: wgpu::Buffer,
    /// First matrix of this instance in the frame's joint palette buffer.
    pub palette_offset: u32,
}

#[derive(Clone)]
pub(crate) struct PbrRtDraw {
    /// Geometry identity: the source primitive for static draws, the draw
    /// (instance + primitive) for dynamic ones.
    pub geometry: u64,
    /// Identity of the source primitive's corner attributes.
    pub uv_key: u64,
    /// 48-byte PbrVertex stream (the morphed stream when morphs apply).
    pub vertices: wgpu::Buffer,
    pub vertex_count: u32,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    pub transform: [[f32; 4]; 4],
    /// Skinned or morphed: positions are produced per frame on the GPU.
    pub dynamic: bool,
    pub skin: Option<SkinSource>,
    /// Changes whenever the dynamic positions change (pose and morphs).
    pub revision: u64,
    /// Baked material; `None` shades hits flat grey and never alpha-tests.
    pub material: Option<MaterialSource>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GeometryParams {
    count: u32,
    palette_offset: u32,
    skinned: u32,
    out_base: u32,
}
/// Camera and sun for one visibility pass.
pub(crate) struct ShadowView {
    pub inverse_view_projection: glam::Mat4,
    pub view_projection: glam::Mat4,
    pub view: glam::Mat4,
    /// Unit vector towards the sun.
    pub towards_sun: glam::Vec3,
    /// Angular diameter of the sun disc; 0 traces one exact hard-shadow ray.
    pub angular_diameter_degrees: f32,
    /// Shadowed point/spot lights traced into visibility channels B and A.
    pub local: [Option<LocalShadow>; 2],
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalShadow {
    pub position: glam::Vec3,
    /// Emitter sphere radius; 0 traces one exact hard ray.
    pub radius: f32,
    pub range: f32,
}
/// Stochastic soft-shadow rays per pixel and frame (denoised over time).
pub const SOFT_SAMPLES: u32 = 2;
/// Upper bound of accumulated frames in the visibility history.
pub const MAX_HISTORY: u32 = 32;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TraceParams {
    inverse_view_projection: [f32; 16],
    inverse_view: [f32; 16],
    towards_sun: [f32; 4],
    size: [u32; 4],
    soft: [f32; 4],
    local: [[f32; 4]; 2],
    local_range: [[f32; 4]; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DenoiseParams {
    inverse_view_projection: [f32; 16],
    view: [f32; 16],
    prev_view_projection: [f32; 16],
    prev_view: [f32; 16],
    size: [u32; 4],
    soft: [f32; 4],
}
struct Denoise {
    size: [u32; 2],
    noisy: (wgpu::Texture, wgpu::TextureView),
    history: [(wgpu::Texture, wgpu::TextureView); 2],
    history_depth: [(wgpu::Texture, wgpu::TextureView); 2],
    current: usize,
    valid: bool,
    prev: Option<(glam::Mat4, glam::Mat4)>,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Slot {
    rect: [f32; 4],
    transform: [f32; 4],
    /// x alpha cutoff, y 1 when alpha-tested.
    cutoff: [f32; 4],
    emissive: [f32; 4],
}

struct Geometry {
    blas: wgpu::Blas,
    /// Dynamic BLAS input (positions, 12-byte stride).
    positions: Option<wgpu::Buffer>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_count: u32,
    index_count: u32,
    built_revision: Option<u64>,
    pending_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PbrRtStats {
    pub static_blas: usize,
    pub dynamic_blas: usize,
    pub static_builds: u64,
    pub dynamic_rebuilds: u64,
    pub dynamic_reused: u64,
    pub tlas_builds: u64,
    pub instances: usize,
    pub alpha_instances: usize,
    pub alpha_slots: usize,
    pub alpha_bakes: u64,
    pub denoised_frames: u64,
    pub reflection_dispatches: u64,
    pub pixel_gi_dispatches: u64,
    pub gi_probe_updates: u64,
}

pub(crate) struct PbrRt {
    geometry: HashMap<u64, Geometry>,
    tlas: Option<wgpu::Tlas>,
    tlas_signature: Vec<(u64, [u32; 12])>,
    tlas_submitted: bool,
    pending: bool,
    // UV gather for alpha-tested geometry: uv key → first corner.
    uv_ranges: BTreeMap<u64, u32>,
    uv_buffer: wgpu::Buffer,
    uv_submitted: bool,
    /// Atlas slots baked in work that was not submitted yet.
    unsubmitted_slots: usize,
    // Alpha atlas.
    _atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    slots: HashMap<String, usize>,
    slot_data: Vec<Slot>,
    bake: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    // Pipelines.
    skin_layout: wgpu::BindGroupLayout,
    uv_layout: wgpu::BindGroupLayout,
    skin_pipeline: wgpu::ComputePipeline,
    uv_pipeline: wgpu::ComputePipeline,
    shadow_layout: wgpu::BindGroupLayout,
    shadow: wgpu::ComputePipeline,
    clear_layout: wgpu::BindGroupLayout,
    clear: wgpu::ComputePipeline,
    reflect_layout: wgpu::BindGroupLayout,
    reflect: wgpu::ComputePipeline,
    gi_layout: wgpu::BindGroupLayout,
    gi: wgpu::ComputePipeline,
    gi_history: Option<wgpu::Buffer>,
    /// Instance table and material slots of this frame's scene.
    tables: Option<(wgpu::Buffer, wgpu::Buffer)>,
    denoise_layout: wgpu::BindGroupLayout,
    temporal: wgpu::ComputePipeline,
    spatial: wgpu::ComputePipeline,
    denoise: Option<Denoise>,
    frame: u32,
    dummy: wgpu::Buffer,
    pub stats: PbrRtStats,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn texture(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn compute(d: &wgpu::Device, layout: &wgpu::BindGroupLayout, label: &str, source: &str, entry: &str) -> wgpu::ComputePipeline {
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// Shadow pass source: the shared ray-query prelude plus the pass.
pub(crate) fn shadow_source() -> String {
    format!("{}\n{}", include_str!("shaders/rt_common.wgsl"), include_str!("shaders/rt_pbr_shadow.wgsl"))
}
/// Reflection pass source.
pub(crate) fn reflect_source() -> String {
    format!("{}\n{}\n{}", include_str!("shaders/rt_common.wgsl"), include_str!("shaders/rt_lighting.wgsl"), include_str!("shaders/rt_reflect.wgsl"))
}

/// Camera, sun and environment for ray-traced reflections.
pub(crate) struct ReflectionView {
    pub inverse_view_projection: glam::Mat4,
    pub view_projection: glam::Mat4,
    pub view: glam::Mat4,
    /// Previous frame's view-projection (for reusing its colour); `None` when invalid.
    pub previous_view_projection: Option<glam::Mat4>,
    pub camera: glam::Vec3,
    /// Direction light travels and illuminance.
    pub sun_dir: [f32; 4],
    /// Linear colour and 1 when the sun casts shadows.
    pub sun_color: [f32; 4],
    pub env_intensity: f32,
    pub env_rotation: f32,
    pub sh: [[f32; 4]; 9],
    pub max_distance: f32,
    pub samples: u32,
    pub diffuse: bool,
    pub light_count: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectParams {
    inverse_view_projection: [f32; 16],
    inverse_view: [f32; 16],
    view_projection: [f32; 16],
    reproject: [f32; 16],
    camera: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    env: [f32; 4],
    sh: [[f32; 4]; 9],
    size: [u32; 4],
    reflection: [f32; 4],
}

/// Probe update source.
pub(crate) fn gi_source() -> String {
    format!("{}\n{}\n{}", include_str!("shaders/rt_common.wgsl"), include_str!("shaders/rt_lighting.wgsl"), include_str!("shaders/rt_gi.wgsl"))
}
/// Probe updates per dispatch (`jobs` uniform capacity).
pub const MAX_GI_JOBS: usize = 64;
/// Weight of the previous probe value in each ray-traced update.
pub const GI_HYSTERESIS: f32 = 0.85;

/// Lighting and volume layout for ray-traced irradiance probe updates.
pub(crate) struct GiView {
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub env_intensity: f32,
    pub env_rotation: f32,
    pub gi_center: [f32; 4],
    pub gi_extent: [f32; 4],
    pub gi_counts: [f32; 4],
    /// First vec4 of the GI and visibility sections in `data`.
    pub gi_offset: u32,
    pub visibility_offset: u32,
    pub max_distance: f32,
    pub backface_limit: f32,
    pub light_count: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GiParams {
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    env: [f32; 4],
    gi_center: [f32; 4],
    gi_extent: [f32; 4],
    gi_counts: [f32; 4],
    offsets: [u32; 4],
    limits: [f32; 4],
    jobs: [[u32; 4]; 16],
}

fn geometry_size(vertex_count: u32, index_count: u32) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(index_count),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    }
}

/// Row-major 3x4 TLAS transform from a column-major matrix.
fn tlas_transform(cols: [[f32; 4]; 4]) -> [f32; 12] {
    [
        cols[0][0], cols[1][0], cols[2][0], cols[3][0], cols[0][1], cols[1][1], cols[2][1],
        cols[3][1], cols[0][2], cols[1][2], cols[2][2], cols[3][2],
    ]
}

impl PbrRt {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let d = &gpu.device;
        ensure!(
            d.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
            "hardware RT requires an explicitly enabled native ray-query device"
        );
        for (label, source) in [
            ("RT geometry", include_str!("shaders/rt_geometry.wgsl")),
            ("RT PBR shadow", shadow_source().as_str()),
            ("RT reflections", reflect_source().as_str()),
            ("RT irradiance probes", gi_source().as_str()),
        ] {
            crate::hardware_rt::validate_wgsl(source).with_context(|| label.to_owned())?;
        }
        let skin_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT dynamic positions"),
            entries: &[uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, false)],
        });
        let uv_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT alpha UV gather"),
            entries: &[uniform(0), storage(1, true), storage(5, true), storage(6, false)],
        });
        let source = include_str!("shaders/rt_geometry.wgsl");
        let skin_pipeline = compute(d, &skin_layout, "RT dynamic positions", source, "skin_positions");
        let uv_pipeline = compute(d, &uv_layout, "RT corner attributes", source, "gather_attributes");
        let shadow_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT PBR shadow"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                texture(2, wgpu::TextureSampleType::Depth),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                texture(5, wgpu::TextureSampleType::Float { filterable: false }),
                storage(6, true),
                storage(7, true),
                storage(8, true),
                texture(9, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shadow = compute(d, &shadow_layout, "RT PBR sun visibility", &shadow_source(), "main");
        let filtered = wgpu::TextureSampleType::Float { filterable: true };
        let reflect_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT reflections"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                texture(2, wgpu::TextureSampleType::Depth),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                texture(4, filtered),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage(6, true),
                storage(7, true),
                storage(8, true),
                storage(12, true),
                texture(13, filtered),
                texture(9, filtered),
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let reflect = compute(d, &reflect_layout, "RT reflections", &reflect_source(), "main");
        let gi_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT irradiance probes"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                texture(2, filtered),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage(4, true),
                storage(5, false),
                storage(6, true),
                storage(7, true),
                storage(8, true),
                storage(12, true),
                texture(9, filtered),
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let gi = compute(d, &gi_layout, "RT irradiance probes", &gi_source(), "update");
        let clear_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT PBR empty visibility"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                texture(5, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let clear = compute(d, &clear_layout, "RT PBR empty visibility", &shadow_source(), "clear");
        crate::hardware_rt::validate_wgsl(include_str!("shaders/rt_denoise.wgsl")).context("RT denoise")?;
        let storage_out = |binding, format| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let denoise_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT visibility denoise"),
            entries: &[
                uniform(0),
                texture(1, wgpu::TextureSampleType::Depth),
                texture(2, unfiltered),
                texture(3, unfiltered),
                storage_out(4, wgpu::TextureFormat::Rgba16Float),
                texture(5, unfiltered),
                storage_out(7, wgpu::TextureFormat::Rgba16Float),
                texture(8, unfiltered),
                storage_out(9, wgpu::TextureFormat::R32Float),
            ],
        });
        let denoise_source = include_str!("shaders/rt_denoise.wgsl");
        let temporal = compute(d, &denoise_layout, "RT visibility temporal", denoise_source, "temporal");
        let spatial = compute(d, &denoise_layout, "RT visibility spatial", denoise_source, "spatial");
        let atlas = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("RT alpha atlas"),
            size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("RT alpha atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT dummy storage"),
            size: 256,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uv_buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RT alpha UVs"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Ok(Self {
            geometry: HashMap::new(),
            tlas: None,
            tlas_signature: Vec::new(),
            tlas_submitted: false,
            pending: false,
            uv_ranges: BTreeMap::new(),
            uv_buffer,
            uv_submitted: false,
            unsubmitted_slots: 0,
            _atlas: atlas,
            atlas_view,
            sampler,
            slots: HashMap::new(),
            slot_data: Vec::new(),
            bake: None,
            skin_layout,
            uv_layout,
            skin_pipeline,
            uv_pipeline,
            shadow_layout,
            shadow,
            clear_layout,
            clear,
            reflect_layout,
            reflect,
            gi_history: None,
            gi_layout,
            gi,
            tables: None,
            denoise_layout,
            temporal,
            spatial,
            denoise: None,
            frame: 0,
            dummy,
            stats: PbrRtStats::default(),
        })
    }

    pub fn clear(&mut self) {
        self.gi_history = None;
        self.geometry.clear();
        self.tlas = None;
        self.tlas_signature.clear();
        self.tlas_submitted = false;
        self.uv_ranges.clear();
        self.slots.clear();
        self.slot_data.clear();
    }

    /// Commit builds encoded since the last call; call only after the
    /// submission that contains them succeeded.
    pub fn mark_submitted(&mut self) {
        if !self.pending {
            return;
        }
        for g in self.geometry.values_mut() {
            if let Some(r) = g.pending_revision.take() {
                g.built_revision = Some(r);
            }
        }
        self.tlas_submitted = self.tlas.is_some();
        self.uv_submitted = true;
        self.unsubmitted_slots = 0;
        self.pending = false;
    }

    /// The previous frame's encoder was never submitted: forget what it built.
    fn abandon_unsubmitted(&mut self) {
        for g in self.geometry.values_mut() {
            g.pending_revision = None;
        }
        self.tlas_submitted = false;
        self.uv_submitted = false;
        let keep = self.slot_data.len() - self.unsubmitted_slots;
        self.slot_data.truncate(keep);
        self.slots.retain(|_, index| *index < keep);
        self.unsubmitted_slots = 0;
        self.pending = false;
    }

    fn bake_pipeline(&mut self, gpu: &Gpu, material_layout: &wgpu::BindGroupLayout) -> Result<()> {
        if self.bake.is_some() {
            return Ok(());
        }
        let source = format!(
            "{}\n{}\n{}",
            crate::effect::shadow_material_prelude(),
            include_str!("shaders/surface_sample.wgsl"),
            include_str!("shaders/rt_alpha_bake.wgsl")
        );
        crate::hardware_rt::validate_wgsl(&source).context("RT alpha bake shader")?;
        let d = &gpu.device;
        let bake_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT alpha bake tile"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT alpha bake"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT alpha bake"),
            bind_group_layouts: &[Some(material_layout), Some(&bake_layout)],
            immediate_size: 0,
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RT alpha bake"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_bake"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_bake"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ATLAS_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        self.bake = Some((pipeline, bake_layout));
        Ok(())
    }

    /// Slot of a material in the alpha atlas, baking it on first use.
    fn slot(&mut self, gpu: &Gpu, alpha: &MaterialSource, encoder: &mut wgpu::CommandEncoder) -> Result<usize> {
        if let Some(s) = self.slots.get(&alpha.material) {
            return Ok(*s);
        }
        ensure!(self.slots.len() < SLOTS, "RT alpha atlas full ({SLOTS} materials)");
        let index = self.slots.len();
        let per_row = (ATLAS / TILE) as usize;
        let (x, y) = ((index % per_row) as u32 * TILE, (index / per_row) as u32 * TILE);
        let (pipeline, layout) = self.bake.as_ref().context("RT alpha bake pipeline")?;
        let rect = [x as f32, y as f32, TILE as f32, TILE as f32];
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT alpha bake tile"),
            contents: bytemuck::cast_slice(&rect),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT alpha bake tile"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RT alpha bake"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.atlas_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(x as f32, y as f32, TILE as f32, TILE as f32, 0., 1.);
            pass.set_scissor_rect(x, y, TILE, TILE);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &alpha.bind, &[]);
            pass.set_bind_group(1, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // Sample tile centres only: half a texel inset on every side.
        let inset = 0.5 / ATLAS as f32;
        let scale = TILE as f32 / ATLAS as f32;
        self.slot_data.push(Slot {
            rect: [x as f32 / ATLAS as f32 + inset, y as f32 / ATLAS as f32 + inset, scale - 2. * inset, scale - 2. * inset],
            transform: alpha.uv_transform,
            cutoff: [alpha.cutoff, if alpha.mask { 1. } else { 0. }, 0., 0.],
            emissive: [alpha.emissive[0], alpha.emissive[1], alpha.emissive[2], 0.],
        });
        self.slots.insert(alpha.material.clone(), index);
        self.unsubmitted_slots += 1;
        self.stats.alpha_bakes += 1;
        Ok(index)
    }

    /// `buffers` are (binding, buffer) pairs after the uniform at binding 0.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        layout: &wgpu::BindGroupLayout,
        params: GeometryParams,
        buffers: &[(u32, &wgpu::Buffer)],
        groups: u32,
    ) {
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT geometry params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        for (binding, b) in buffers {
            entries.push(wgpu::BindGroupEntry { binding: *binding, resource: b.as_entire_binding() });
        }
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT geometry gather"),
            layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT geometry"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(groups, 1, 1);
    }

    /// Bring BLAS/TLAS, UVs and the alpha atlas up to date for `draws`.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        draws: &[PbrRtDraw],
        material_layout: &wgpu::BindGroupLayout,
        palette: &wgpu::Buffer,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Option<(wgpu::Buffer, wgpu::Buffer)>> {
        if self.pending {
            self.abandon_unsubmitted();
        }
        let limits = gpu.device.limits();
        ensure!(draws.len() <= limits.max_tlas_instance_count as usize, "hardware RT instance budget exceeded");
        if draws.iter().any(|d| d.material.is_some()) {
            self.bake_pipeline(gpu, material_layout)?;
        }
        // Geometry: create missing BLAS; produce dynamic positions.
        let used: HashSet<u64> = draws.iter().map(|d| d.geometry).collect();
        self.geometry.retain(|k, _| used.contains(k));
        let dynamic = draws.iter().filter(|d| d.dynamic).map(|d| d.geometry).collect::<HashSet<_>>().len();
        ensure!(dynamic <= MAX_DYNAMIC, "hardware RT dynamic BLAS budget ({MAX_DYNAMIC}) exceeded");
        let mut seen = HashSet::new();
        let mut to_build: Vec<u64> = Vec::new();
        for d in draws {
            if !seen.insert(d.geometry) {
                continue;
            }
            ensure!(d.index_count % 3 == 0 && d.index_count > 0 && d.vertex_count > 0, "RT primitive has no triangles");
            ensure!(d.index_count / 3 <= limits.max_blas_primitive_count, "RT primitive exceeds the BLAS primitive limit");
            let rebuild_needed = match self.geometry.get(&d.geometry) {
                Some(g) => g.vertex_count != d.vertex_count || g.index_count != d.index_count,
                None => true,
            };
            if rebuild_needed {
                let blas = gpu.device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some(if d.dynamic { "RT dynamic BLAS" } else { "RT static BLAS" }),
                        flags: if d.dynamic {
                            wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD | wgpu::AccelerationStructureFlags::ALLOW_UPDATE
                        } else {
                            wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
                        },
                        update_mode: if d.dynamic {
                            wgpu::AccelerationStructureUpdateMode::PreferUpdate
                        } else {
                            wgpu::AccelerationStructureUpdateMode::Build
                        },
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![geometry_size(d.vertex_count, d.index_count)] },
                );
                let positions = d.dynamic.then(|| {
                    gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RT dynamic positions"),
                        size: u64::from(d.vertex_count) * 12,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                        mapped_at_creation: false,
                    })
                });
                self.geometry.insert(
                    d.geometry,
                    Geometry {
                        blas,
                        positions,
                        vertices: d.vertices.clone(),
                        indices: d.indices.clone(),
                        vertex_count: d.vertex_count,
                        index_count: d.index_count,
                        built_revision: None,
                        pending_revision: None,
                    },
                );
            }
            let g = self.geometry.get_mut(&d.geometry).unwrap();
            g.vertices = d.vertices.clone();
            g.indices = d.indices.clone();
            let revision = if d.dynamic { d.revision } else { 0 };
            if g.built_revision == Some(revision) {
                if d.dynamic {
                    self.stats.dynamic_reused += 1;
                }
                continue;
            }
            g.pending_revision = Some(revision);
            let positions = g.positions.clone();
            to_build.push(d.geometry);
            if d.dynamic {
                let positions = positions.as_ref().context("RT dynamic positions")?;
                let (stream, palette, offset, skinned) = match &d.skin {
                    Some(s) => (&s.stream, palette, s.palette_offset, 1),
                    None => (&self.dummy, &self.dummy, 0, 0),
                };
                self.dispatch(
                    gpu,
                    encoder,
                    &self.skin_pipeline,
                    &self.skin_layout,
                    GeometryParams { count: d.vertex_count, palette_offset: offset, skinned, out_base: 0 },
                    &[(1, &d.vertices), (2, stream), (3, palette), (4, positions)],
                    d.vertex_count.div_ceil(64),
                );
                self.stats.dynamic_rebuilds += 1;
            } else {
                self.stats.static_builds += 1;
            }
        }
        // Corner attributes: regather all when the set of geometry changes.
        let mut alpha_keys: BTreeMap<u64, &PbrRtDraw> = BTreeMap::new();
        for d in draws {
            alpha_keys.entry(d.uv_key).or_insert(d);
        }
        if alpha_keys.keys().ne(self.uv_ranges.keys()) || (!alpha_keys.is_empty() && !self.uv_submitted) {
            let corners: u64 = alpha_keys.values().map(|d| u64::from(d.index_count)).sum();
            ensure!(corners <= MAX_CORNERS, "RT corner attribute budget exceeded");
            self.uv_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT corner attributes"),
                size: (corners * 8).max(64),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            self.uv_ranges.clear();
            let mut base = 0u32;
            for (key, d) in &alpha_keys {
                self.dispatch(
                    gpu,
                    encoder,
                    &self.uv_pipeline,
                    &self.uv_layout,
                    GeometryParams { count: d.index_count, palette_offset: 0, skinned: 0, out_base: base },
                    &[(1, &d.vertices), (5, &d.indices), (6, &self.uv_buffer)],
                    d.index_count.div_ceil(64),
                );
                self.uv_ranges.insert(*key, base);
                base += d.index_count;
            }
            self.uv_submitted = false;
        }
        // Instance table and alpha slots.
        let mut table = Vec::with_capacity(draws.len());
        let mut alpha_instances = 0;
        for d in draws {
            let slot = match &d.material {
                Some(m) => {
                    alpha_instances += usize::from(m.mask);
                    self.slot(gpu, m, encoder)? as u32 + 1
                }
                None => 0,
            };
            table.push([self.uv_ranges[&d.uv_key], slot, 0, 0]);
        }
        // TLAS: rebuilt when instances moved, geometry changed or after a failed submission.
        let signature: Vec<(u64, [u32; 12])> = draws.iter().map(|d| (d.geometry, tlas_transform(d.transform).map(f32::to_bits))).collect();
        let rebuild_tlas = !to_build.is_empty() || signature != self.tlas_signature || !self.tlas_submitted;
        if signature.is_empty() {
            self.tlas = None;
            self.tlas_signature.clear();
        } else if rebuild_tlas {
            let mut tlas = gpu.device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("RT PBR instances"),
                max_instances: signature.len() as u32,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            });
            for (i, d) in draws.iter().enumerate() {
                tlas[i] = Some(wgpu::TlasInstance::new(&self.geometry[&d.geometry].blas, tlas_transform(d.transform), i as u32, 0xff));
            }
            let sizes: Vec<_> = to_build
                .iter()
                .map(|k| {
                    let g = &self.geometry[k];
                    geometry_size(g.vertex_count, g.index_count)
                })
                .collect();
            let entries: Vec<_> = to_build
                .iter()
                .zip(&sizes)
                .map(|(k, size)| {
                    let g = &self.geometry[k];
                    let (vertex_buffer, vertex_stride) = match &g.positions {
                        Some(p) => (p, 12),
                        None => (&g.vertices, 48),
                    };
                    wgpu::BlasBuildEntry {
                        blas: &g.blas,
                        geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer,
                            first_vertex: 0,
                            vertex_stride,
                            index_buffer: Some(&g.indices),
                            first_index: Some(0),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        }]),
                    }
                })
                .collect();
            encoder.build_acceleration_structures(entries.iter(), [&tlas]);
            self.tlas = Some(tlas);
            self.tlas_signature = signature;
            self.tlas_submitted = false;
            self.stats.tlas_builds += 1;
        }
        self.pending = true;
        self.stats.instances = draws.len();
        self.stats.alpha_instances = alpha_instances;
        self.stats.alpha_slots = self.slots.len();
        self.stats.static_blas = self.geometry.values().filter(|g| g.positions.is_none()).count();
        self.stats.dynamic_blas = self.geometry.values().filter(|g| g.positions.is_some()).count();
        if self.tlas.is_none() {
            return Ok(None);
        }
        let instances = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT instance table"),
            contents: bytemuck::cast_slice(&table),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let slots = if self.slot_data.is_empty() {
            vec![Slot::zeroed()]
        } else {
            self.slot_data.clone()
        };
        let slots = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT alpha slots"),
            contents: bytemuck::cast_slice(&slots),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Some((instances, slots)))
    }

    /// Encode PBR sun visibility into `output` (AO in R, visibility in G).
    #[allow(clippy::too_many_arguments)]
    pub fn encode_shadows(
        &mut self,
        gpu: &Gpu,
        draws: &[PbrRtDraw],
        material_layout: &wgpu::BindGroupLayout,
        palette: &wgpu::Buffer,
        view: &ShadowView,
        canvas: [u32; 2],
        depth: &wgpu::TextureView,
        normals: &wgpu::TextureView,
        ao: &wgpu::TextureView,
        output: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool> {
        let prepared = self.prepare(gpu, draws, material_layout, palette, encoder)?;
        self.tables = prepared.clone();
        let Some((instances, slots)) = prepared else {
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT PBR empty visibility"),
                layout: &self.clear_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(output) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(ao) },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT PBR empty visibility"), timestamp_writes: None });
            pass.set_pipeline(&self.clear);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
            return Ok(false);
        };
        self.frame = self.frame.wrapping_add(1);
        let radius = (view.angular_diameter_degrees * 0.5).to_radians();
        let soft_mask = [
            radius > 0.,
            view.local[0].is_some_and(|l| l.radius > 0.),
            view.local[1].is_some_and(|l| l.radius > 0.),
        ];
        let soft = soft_mask.iter().any(|s| *s);
        let local = view.local.map(|l| l.map_or([0.; 4], |l| l.position.extend(l.radius).to_array()));
        let local_range = view.local.map(|l| l.map_or([0.; 4], |l| [l.range, 1., 0., 0.]));
        let params = TraceParams {
            inverse_view_projection: view.inverse_view_projection.to_cols_array(),
            inverse_view: view.view.inverse().to_cols_array(),
            towards_sun: view.towards_sun.extend(0.).to_array(),
            size: [canvas[0], canvas[1], 0, 0],
            soft: [if radius > 0. { radius.cos() } else { 1. }, SOFT_SAMPLES as f32, (self.frame % 4096) as f32, 0.],
            local,
            local_range,
        };
        if !soft {
            self.denoise = None;
        } else if self.denoise.as_ref().is_none_or(|d| d.size != canvas) {
            let target = |label, format| {
                let t = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: canvas[0], height: canvas[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let v = t.create_view(&Default::default());
                (t, v)
            };
            self.denoise = Some(Denoise {
                size: canvas,
                noisy: target("RT noisy visibility", wgpu::TextureFormat::Rgba16Float),
                history: [
                    target("RT visibility history A", wgpu::TextureFormat::Rgba16Float),
                    target("RT visibility history B", wgpu::TextureFormat::Rgba16Float),
                ],
                history_depth: [
                    target("RT history depth A", wgpu::TextureFormat::R32Float),
                    target("RT history depth B", wgpu::TextureFormat::R32Float),
                ],
                current: 0,
                valid: false,
                prev: None,
            });
        }
        let trace_output = match &self.denoise {
            Some(d) => &d.noisy.1,
            None => output,
        };
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT PBR shadow params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT PBR shadow"),
            layout: &self.shadow_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.tlas.as_ref().unwrap().as_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(normals) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(trace_output) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(ao) },
                wgpu::BindGroupEntry { binding: 6, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.uv_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: slots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT PBR sun visibility"), timestamp_writes: None });
            pass.set_pipeline(&self.shadow);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
        }
        if let Some(dn) = &mut self.denoise {
            let (prev_view_projection, prev_view) = dn.prev.unwrap_or((view.view_projection, view.view));
            let params = DenoiseParams {
                inverse_view_projection: view.inverse_view_projection.to_cols_array(),
                view: view.view.to_cols_array(),
                prev_view_projection: prev_view_projection.to_cols_array(),
                prev_view: prev_view.to_cols_array(),
                size: [canvas[0], canvas[1], u32::from(dn.valid && dn.prev.is_some()), MAX_HISTORY],
                soft: [f32::from(u8::from(soft_mask[0])), f32::from(u8::from(soft_mask[1])), f32::from(u8::from(soft_mask[2])), 0.],
            };
            let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT denoise params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let (prev, cur) = (dn.current ^ 1, dn.current);
            let bind = |sampled: usize, written: usize| {
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("RT visibility denoise"),
                    layout: &self.denoise_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&dn.noisy.1) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&dn.history[sampled].1) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&dn.history[written].1) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(normals) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(output) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&dn.history_depth[sampled].1) },
                        wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&dn.history_depth[written].1) },
                    ],
                })
            };
            let temporal = bind(prev, cur);
            // The spatial pass samples the fresh history and writes the
            // (unsampled) older one only nominally; it stores to `output`.
            let spatial = bind(cur, prev);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT visibility denoise"), timestamp_writes: None });
            for (pipeline, bind) in [(&self.temporal, &temporal), (&self.spatial, &spatial)] {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
            }
            drop(pass);
            dn.prev = Some((view.view_projection, view.view));
            dn.current ^= 1;
            dn.valid = true;
            self.stats.denoised_frames += 1;
        }
        Ok(true)
    }

    /// Encode mirror reflections into `output` (rgb radiance, a = hit) from
    /// the scene prepared by this frame's `encode_shadows`. Returns false
    /// when there is no geometry (the caller clears the target).
    #[allow(clippy::too_many_arguments)]
    pub fn encode_reflections(
        &mut self,
        gpu: &Gpu,
        view: &ReflectionView,
        canvas: [u32; 2],
        depth: &wgpu::TextureView,
        normals: &wgpu::TextureView,
        previous: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        output: &wgpu::TextureView,
        lights: &wgpu::Buffer,
        environment: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool> {
        let (Some(tlas), Some((instances, slots))) = (&self.tlas, &self.tables) else {
            return Ok(false);
        };
        let params = ReflectParams {
            inverse_view_projection: view.inverse_view_projection.to_cols_array(),
            inverse_view: view.view.inverse().to_cols_array(),
            view_projection: view.view_projection.to_cols_array(),
            reproject: view.previous_view_projection.unwrap_or(view.view_projection).to_cols_array(),
            camera: view.camera.extend(1.).to_array(),
            sun_dir: view.sun_dir,
            sun_color: view.sun_color,
            env: [view.env_intensity, view.env_rotation, 0., 0.],
            sh: view.sh,
            size: [canvas[0], canvas[1], u32::from(view.previous_view_projection.is_some()), 0],
            reflection: [view.max_distance, view.samples as f32, u32::from(view.diffuse) as f32, view.light_count as f32],
        };
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RT reflection params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RT reflections"),
            layout: &self.reflect_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: tlas.as_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(normals) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(previous) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 6, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.uv_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: slots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(output) },
                wgpu::BindGroupEntry { binding: 12, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(environment) },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT reflections"), timestamp_writes: None });
        pass.set_pipeline(&self.reflect);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
        if view.diffuse { self.stats.pixel_gi_dispatches += 1; } else { self.stats.reflection_dispatches += 1; }
        Ok(true)
    }

    /// Refresh irradiance probes `jobs` in the shared GI `data` buffer with
    /// ray queries against the scene prepared by this frame's `encode_shadows`.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_gi(
        &mut self,
        gpu: &Gpu,
        view: &GiView,
        jobs: &[u32],
        environment: &wgpu::TextureView,
        environment_sampler: &wgpu::Sampler,
        data: &wgpu::Buffer,
        lights: &wgpu::Buffer,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool> {
        let (Some(tlas), Some((instances, slots))) = (&self.tlas, &self.tables) else {
            return Ok(false);
        };
        // Every workgroup and every chunk reads one immutable previous
        // volume. Updating neighbouring probes cannot race their bounce reads.
        if self.gi_history.as_ref().is_none_or(|h| h.size() != data.size()) {
            self.gi_history = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT GI previous sweep"), size: data.size(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let previous = self.gi_history.as_ref().expect("GI history allocated");
        encoder.copy_buffer_to_buffer(data, 0, previous, 0, data.size());
        for chunk in jobs.chunks(MAX_GI_JOBS) {
            let mut packed = [[0u32; 4]; 16];
            for (i, probe) in chunk.iter().enumerate() {
                packed[i / 4][i % 4] = *probe;
            }
            let params = GiParams {
                sun_dir: view.sun_dir,
                sun_color: view.sun_color,
                env: [view.env_intensity, view.env_rotation, GI_HYSTERESIS, (self.frame % 4096) as f32],
                gi_center: view.gi_center,
                gi_extent: view.gi_extent,
                gi_counts: view.gi_counts,
                offsets: [view.gi_offset, view.visibility_offset, chunk.len() as u32, 0],
                limits: [view.max_distance, view.backface_limit, view.light_count as f32, 0.],
                jobs: packed,
            };
            let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT irradiance probe jobs"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT irradiance probes"),
                layout: &self.gi_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: tlas.as_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(environment) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(environment_sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: previous.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: data.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 12, resource: lights.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: instances.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: self.uv_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: slots.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("RT irradiance probes"), timestamp_writes: None });
            pass.set_pipeline(&self.gi);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(chunk.len() as u32, 1, 1);
        }
        self.stats.gi_probe_updates += jobs.len() as u64;
        Ok(true)
    }

    pub fn stats_json(&self) -> serde_json::Value {
        let s = self.stats;
        serde_json::json!({
            "path": "pbr-instanced-gpu-geometry",
            "instances": s.instances,
            "static_blas": s.static_blas,
            "dynamic_blas": s.dynamic_blas,
            "static_builds_encoded": s.static_builds,
            "dynamic_rebuilds_encoded": s.dynamic_rebuilds,
            "dynamic_reused": s.dynamic_reused,
            "tlas_builds_encoded": s.tlas_builds,
            "alpha_instances": s.alpha_instances,
            "material_slots": s.alpha_slots,
            "material_bakes": s.alpha_bakes,
            "denoised_frames": s.denoised_frames,
            "reflection_dispatches": s.reflection_dispatches,
            "pixel_gi_dispatches": s.pixel_gi_dispatches,
            "gi_probe_updates": s.gi_probe_updates,
            "refit": "wgpu 29 implements PreferUpdate as a full rebuild",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rt_geometry_shadow_and_alpha_bake_shaders_validate() {
        for source in [
            include_str!("shaders/rt_geometry.wgsl").to_owned(),
            shadow_source(),
            reflect_source(),
            gi_source(),
            format!(
                "{}\n{}\n{}",
                crate::effect::shadow_material_prelude(),
                include_str!("shaders/surface_sample.wgsl"),
                include_str!("shaders/rt_alpha_bake.wgsl")
            ),
        ] {
            crate::hardware_rt::validate_wgsl(&source).unwrap();
        }
        // Every shadow query proceeds exactly once on force-opaque rays.
        let common = include_str!("shaders/rt_common.wgsl");
        assert_eq!(common.matches("rayQueryProceed(").count(), 2, "visible() and closest()");
        assert!(!common.contains("while"));
        for pass in [
            include_str!("shaders/rt_pbr_shadow.wgsl"),
            include_str!("shaders/rt_reflect.wgsl"),
            include_str!("shaders/rt_gi.wgsl"),
        ] {
            assert!(!pass.contains("rayQueryProceed("), "passes query only through rt_common");
        }
    }
}
