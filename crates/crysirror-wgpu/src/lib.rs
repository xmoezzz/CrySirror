// SPDX-License-Identifier: MPL-2.0
//! wgpu backend, independent of winit and native VM instruction sets.
#![deny(unsafe_code)]
mod composite;
mod texture_cache;
mod clip_sampling;
mod frame_history;
pub mod geometry;
mod glyph_run;
mod hardware_rt;
pub mod lights;
pub mod particles;
pub mod pbr;
pub mod probes;
mod profile;
mod projected_mesh;
mod stored_blend;
mod coverage;
mod pipeline;
pub use pipeline::PreparationPending;
mod puppet;
mod ray_geometry;
mod raytrace;
mod ray_lite;
pub mod rt_capabilities;
mod rt_pbr;
mod sampling;
pub mod text;
mod transition;
mod render_targets;
mod rgba_stencil;
pub mod vfx_gpu;
pub mod vfx_render;
pub mod vsm;
use anyhow::{ensure, Context, Result};
pub use crysirror_core::graphics::{GraphicsCapabilities, RenderingPath};
use crysirror_core::service::MeshFrameHistoryProvider;
use crysirror_core::state::{ClientId, Color, Rect, SceneState, TextRun, Visual};
use crysirror_runtime::{assets::Assets, model::Scene3d};
use crysirror_scene::{
    layer::{rectangle, Blend, Paint},
    Primitive,
};
use geometry::{Vertex2d, Vertex3d, Viewport};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{mpsc, Arc},
};
use wgpu::util::DeviceExt;

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}
// The only unsafe operation in this renderer is the upstream experimental-API
// consent token. It never bypasses shader/AS validation or introduces pointers.
#[allow(unsafe_code)]
fn experimental_ray_query_token() -> wgpu::ExperimentalFeatures {
    // SAFETY: explicit hardware-RT selection opts into wgpu's experimental
    // ray-query contract. Only advertised features and validated bounds are
    // requested; all AS construction/query calls use wgpu's checked APIs.
    unsafe { wgpu::ExperimentalFeatures::enabled() }
}
impl Gpu {
    pub fn capabilities(&self) -> GraphicsCapabilities {
        GraphicsCapabilities {
            raster: true,
            // Compute profile, or the lite fragment profile where compute
            // storage is absent (ray_lite.rs).
            compute_raytrace: (self
                .adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
                && self.device.limits().max_storage_buffers_per_shader_stage >= 1
                && self.device.limits().max_storage_textures_per_shader_stage >= 1
                && !ray_lite::preferred(self))
                || ray_lite::supported(self),
            hardware_raytrace: rt_capabilities::report(&self.adapter, Some(&self.device)).enabled,
        }
    }
    pub fn hardware_rt_report(&self) -> rt_capabilities::HardwareRtReport {
        rt_capabilities::report(&self.adapter, Some(&self.device))
    }
    pub async fn request(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self> {
        Self::request_for(instance, surface, RenderingPath::Raster).await
    }
    /// Negotiate the explicitly requested path across compatible adapters. RT
    /// features are never enabled as a side effect of a raster/compute request.
    pub async fn request_for(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        path: RenderingPath,
    ) -> Result<Self> {
        let adapter = if path == RenderingPath::HardwareRaytrace {
            let all = instance.enumerate_adapters(wgpu::Backends::all()).await;
            let mut diagnostics = Vec::new();
            let mut candidates = Vec::new();
            for adapter in all {
                let report = rt_capabilities::report(&adapter, None);
                let compatible = surface.is_none_or(|s| adapter.is_surface_supported(s));
                diagnostics.push(format!(
                    "{} [{}]: {}",
                    report.adapter_name,
                    report.backend,
                    if !compatible {
                        "surface incompatible"
                    } else {
                        report.reason.as_deref().unwrap_or("available")
                    }
                ));
                if report.available && compatible {
                    candidates.push(adapter);
                }
            }
            candidates.sort_by_key(|a| match a.get_info().device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                _ => 3,
            });
            candidates.into_iter().next().with_context(|| {
                format!(
                    "hardware ray tracing unavailable; {}",
                    diagnostics.join("; ")
                )
            })?
        } else {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    apply_limit_buckets: false,
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: surface,
                    force_fallback_adapter: false,
                })
                .await
                .context("no suitable GPU adapter")?
        };
        let rt = path == RenderingPath::HardwareRaytrace;
        let limits = if rt {
            rt_capabilities::negotiate_limits(&adapter.limits())?
        } else {
            wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("CrySirror"),
                required_features: profile::features(&adapter)
                    | if rt {
                        wgpu::Features::EXPERIMENTAL_RAY_QUERY
                    } else {
                        wgpu::Features::empty()
                    },
                required_limits: limits,
                experimental_features: if rt {
                    experimental_ray_query_token()
                } else {
                    wgpu::ExperimentalFeatures::disabled()
                },
                ..Default::default()
            })
            .await?;
        let gpu = Self {
            adapter,
            device,
            queue,
        };
        gpu.capabilities()
            .require(path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(gpu)
    }
}

/// The single readback texture of `Paint::Backdrop`.
const BACKDROP_KEY: &str = "backdrop:frame";
#[derive(Clone)]
struct Texture {
    bind: wgpu::BindGroup,
    view: wgpu::TextureView,
    texture: wgpu::Texture,
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
    rgba_contribution: bool,
    output: Option<(String, [u32; 2], u64)>,
    feedback_source: Option<String>,
    owner:u32,
    coverage:crysirror_scene::layer::coverage::Operation,
    key: String,
    range: Range<u32>,
    clip: Option<[f32; 4]>,
    blend: Blend,
    /// Paint::Backdrop's history mode: capture after drawing instead of before.
    capture_after: bool,
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
    f: [f32; 4],
    g: [f32; 4],
    h: [f32; 4],
    i: [f32; 4],
}
fn filter_gpu(f: &crysirror_scene::layer::LayerFilter,_viewport:[f32;4]) -> FilterGpu {
    use crysirror_scene::layer::Distortion;
    let coverage = f.mask.as_ref().and_then(|m|m.coverage);
    let flags = u32::from(f.tone_curve.is_some())
        | u32::from(f.mask.is_some()) << 1
        | u32::from(f.mask.as_ref().is_some_and(|m| m.gray_alpha)) << 2
        | u32::from(f.alpha_test) << 3
        | u32::from(f.ignore_tint_alpha) << 4
        | u32::from(f.encoded_sampling) << 5
        | u32::from(coverage.is_some()) << 14
        | u32::from(coverage.is_some_and(|c|c.canvas.is_some())) << 15
        | u32::from(f.stencil.is_some()) << 16
        | u32::from(f.stencil.as_ref().is_some_and(|s|s.nearest)) << 17
        | u32::from(f.stencil.as_ref().is_some_and(|s|s.outside_zero)) << 18
        | u32::from(f.stencil.as_ref().is_some_and(|s|s.rgba)) << 19;
    let (kind, p) = match f.distortion {
        None => (0., [0.; 5]),
        Some(Distortion::NineTap { offset, center_weight, neighbor_weight }) =>
            (9., [offset[0], offset[1], center_weight, neighbor_weight, 0.]),
        Some(Distortion::Quantized { steps, phase }) =>
            (10., [steps[0], steps[1], phase[0], phase[1], 0.]),
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
        Some(Distortion::Wave { vertical, amplitude, cycles, phase }) => (
            if vertical { 8. } else { 7. },
            [amplitude, cycles, phase, 0., 0.],
        ),
        Some(Distortion::ThresholdFade {
            fade,
            threshold,
            invert,
        }) => (if invert { 6. } else { 5. }, [fade, threshold, 0., 0., 0.]),
    };
    let curve = f.tone_curve.as_ref();
    let mut record = FilterGpu {
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
        f: {let weights=f.luma_weights.unwrap_or([0.2989,0.5886,0.1145]);[weights[0],weights[1],weights[2],f.rgb_alpha_power]},
        g: f.mask.as_ref().map_or([0.;4],|mask|{
            use crysirror_scene::layer::MaskChannel;
            [match mask.channel{MaskChannel::Rgba=>0.,MaskChannel::Red=>1.,MaskChannel::Green=>2.,MaskChannel::Blue=>3.,MaskChannel::Alpha=>4.},mask.offset,if mask.preserve_source_alpha {1.}else{0.},if mask.source_space {1.}else{0.}]
        }),
        h: [f.mask.as_ref().map_or(1.,|mask|mask.scale),f.multiply[0],f.multiply[1],f.multiply[2]],
        i: [0.;4],
    };
    if let Some(c) = coverage {
        record.g[1] = c.progress;
        record.g[2] = c.feather;
        record.g[3] = c.canvas.map_or(0.,|s|s[0] as f32);
        record.h[0] = c.canvas.map_or(0.,|s|s[1] as f32);
    }
    if let Some(canvas)=f.stencil.as_ref().map(|s|s.canvas).or_else(||coverage.and_then(|c|c.canvas)) {
        record.g[3]=canvas[0] as f32;record.h[0]=canvas[1] as f32;
    }
    if let Some(s)=&f.stencil {record.d[0]=s.origin[0];record.d[1]=s.origin[1];}
    record
}
fn filter_sampling(record:&mut FilterGpu,sampling:crysirror_scene::layer::ImageSampling) {
    use crysirror_scene::layer::{ImageSampleProfile,TextureAddressMode};
    let mut flags=record.c[3] as u32;
    if !sampling.linear { flags|=64; }
    if sampling.profile==ImageSampleProfile::StoredByte128 {
        flags|=32|128;
        flags|=match sampling.address_u {TextureAddressMode::ClampToEdge=>0,TextureAddressMode::Repeat=>256,TextureAddressMode::MirrorRepeat=>512};
        flags|=match sampling.address_v {TextureAddressMode::ClampToEdge=>0,TextureAddressMode::Repeat=>1024,TextureAddressMode::MirrorRepeat=>2048};
    }
    record.c[3]=flags as f32;
}
/// Immutable pipeline handles and compatible layouts for one device and output
/// format. Isolated subtree renderers share these objects with their parent.
/// Render targets, uniforms, texture caches and temporal state stay per renderer.
struct DrawPipelines {
    composite: composite::CompositePipeline,
    puppet: Arc<puppet::PuppetPipelines>,
    transition: Arc<transition::TransitionPipelines>,
    camera_layout: wgpu::BindGroupLayout,
    quad_pipelines: Vec<wgpu::RenderPipeline>,
    filtered_pipelines: Vec<wgpu::RenderPipeline>,
    rgba_add_pipeline: wgpu::RenderPipeline,
    rgba_subtract_pipeline: wgpu::RenderPipeline,
    stored_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    ray_composite_pipeline: wgpu::RenderPipeline,
    coverage_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    coverage_white: wgpu::BindGroup,
    _coverage_white_texture: wgpu::Texture,
}
pub struct Renderer {
    draw_pipelines: Arc<DrawPipelines>,
    /// Consecutive-frame capture (movies): temporal history carries over from
    /// the previous capture, so only the first capture accumulates warm-up passes.
    movie_capture: bool,
    movie_warmed: bool,
    coverage_planes:BTreeMap<u32,coverage::Plane>,
    blend_origin: [f32;2],
    presentation_scale: crysirror_core::service::PresentationScale,
    histories: Arc<frame_history::Provider>,
    profiler: crysirror_tasks::profile::Profiler,
    profile_frame: u64,
    gpu_timings: profile::GpuTimings,
    pub gpu: Arc<Gpu>,
    pub format: wgpu::TextureFormat,
    compositor: Option<composite::Compositor>,
    /// Content hashes of plain project images pinned by their first snapshot.
    snapshot_plain_images: std::collections::HashMap<(u32, String), blake3::Hash>,
    snapshot_rasterizations: u64,
    filter_buffer: Option<wgpu::Buffer>,
    mask_viewport:[f32;4],
    /// (mask key, curve key) → bind group over this frame's filter buffer.
    filter_binds: std::collections::HashMap<(String, String), wgpu::BindGroup>,
    engine_frame: Vec<Primitive>,
    sampler: wgpu::Sampler,
    textures: BTreeMap<String, Texture>,
    recent_snapshots: texture_cache::RecentSnapshots,
    /// Explicit capture-only generations are bounded even in a headless host
    /// that prepares images without submitting a normal display frame.
    snapshot_readbacks: std::collections::VecDeque<String>,
    /// Logical scene draws are outside engine_frame; preserve their actual
    /// keys while an immutable capture temporarily installs a probe frame.
    scene_texture_keys: BTreeSet<String>,
    capture_texture_keys: BTreeSet<String>,
    snapshot_readers: BTreeMap<String, Vec<std::sync::Weak<dyn crysirror_core::service::AssetReader>>>,
    target_write_revisions: BTreeMap<String, u64>,
    pending_target_outputs: BTreeSet<String>,
    target_readback_serial: u64,
    flushing_target_writes: bool,
    glyph_cells: BTreeMap<String, glyph_run::GlyphBitmap>,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    scene3d: Option<Scene3d>,
    mesh_buffer: Option<wgpu::Buffer>,
    mesh_count: u32,
    depth: Option<Depth>,
    path: RenderingPath,
    raytracer: Option<raytrace::RayTracer>,
    ray_lite: Option<ray_lite::RayLite>,
    hardware_rt: Option<hardware_rt::HardwareRt>,
    /// Physically based path (ADR 0010); created on first use.
    pbr: Option<pbr::PbrRenderer>,
    transitions: transition::Transitions,
    transition: Option<crysirror_scene::layer::TransitionFrame>,
    /// Puppet layer targets and pipelines (ADR 0029).
    puppets: puppet::PuppetRenderer,
}
impl Renderer {
    /// Captures are consecutive movie frames: keep temporal history between
    /// captures instead of re-accumulating warm-up passes for each one.
    pub fn set_movie_capture(&mut self, enabled: bool) {
        self.movie_capture = enabled;
        self.movie_warmed = false;
    }
}

impl DrawPipelines {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
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
            bind_group_layouts: &[Some(&texture_layout)],
            immediate_size: 0,
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
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let quad_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(crysirror_shaders::wgsl::QUAD_V2.into()),
        });
        let mesh_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(crysirror_shaders::wgsl::MESH.into()),
        });
        const QUAD_ATTR: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x2,2=>Float32x4,3=>Float32];
        const LAYER_ATTR: [wgpu::VertexAttribute; 6] =
            wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x2,2=>Float32x4,3=>Float32,4=>Float32x4,5=>Float32x4];
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
        let coverage_layout=d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("ordered coverage layout"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None}
        ]});
        let coverage_white_texture=d.create_texture(&wgpu::TextureDescriptor{label:Some("neutral coverage"),size:wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&coverage_white_texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},&[255;4],wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1});
        let white_view=coverage_white_texture.create_view(&Default::default());
        let coverage_white=d.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("neutral coverage binding"),layout:&coverage_layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&white_view)}]});
        let filtered_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filtered quad layout"),
            bind_group_layouts: &[Some(&texture_layout), Some(&filter_layout),Some(&coverage_layout)],
            immediate_size: 0,
        });
        const MESH_ATTR: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4];
        let blends = [
            Blend::Normal,
            Blend::Add,
            Blend::Subtract,
            Blend::Multiply,
            Blend::Screen,
            Blend::Atop,
            Blend::AtopMultiply,
            Blend::AtopScreen,
            Blend::AtopCopy,
            Blend::SourceOver,
            Blend::Modulate,
            Blend::InverseDestination,
            Blend::ModulateAdd,
            Blend::DestinationIn,
        ];
        // Fixed-function factor pairs apply to colour and alpha alike.
        let fixed = |src_factor, dst_factor| wgpu::BlendComponent { src_factor, dst_factor, operation: wgpu::BlendOperation::Add };
        let quad_pipeline = |blend: Blend, filtered: bool, rgba_contribution: bool| {
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
                // Premultiplied source-atop: S·Da + D·(1 - Sa).
                Blend::Atop => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::DstAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                // Premultiplied PS multiply: S·D + D·(1 - Sa) = D·(1 - a + a·s).
                Blend::AtopMultiply => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                // Premultiplied PS screen: S·Da + D·(1 - S).
                Blend::AtopScreen => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::DstAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                // Straight colour replaced under the kept alpha: S·Da.
                Blend::AtopCopy => wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::DstAlpha,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                Blend::SourceOver => fixed(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
                Blend::Modulate => fixed(wgpu::BlendFactor::Zero, wgpu::BlendFactor::Src),
                Blend::InverseDestination => fixed(wgpu::BlendFactor::OneMinusDst, wgpu::BlendFactor::Zero),
                Blend::ModulateAdd => fixed(wgpu::BlendFactor::Dst, wgpu::BlendFactor::One),
                Blend::DestinationIn => fixed(wgpu::BlendFactor::Zero, wgpu::BlendFactor::SrcAlpha),
                Blend::StoredByte { .. } => wgpu::BlendComponent::REPLACE,
            };
            // Atop keeps destination alpha; destination-in masks the complete
            // premultiplied destination without introducing source colour.
            let alpha = match blend {
                Blend::Atop | Blend::AtopMultiply | Blend::AtopScreen | Blend::AtopCopy => {
                    wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    }
                }
                Blend::SourceOver | Blend::Modulate | Blend::InverseDestination | Blend::ModulateAdd | Blend::DestinationIn | Blend::StoredByte { .. } => color,
                _ => wgpu::BlendComponent::OVER,
            };
            let (color,alpha)=if rgba_contribution {
                (wgpu::BlendComponent {src_factor:wgpu::BlendFactor::One,dst_factor:wgpu::BlendFactor::One,
                    operation:if blend==Blend::Subtract {wgpu::BlendOperation::ReverseSubtract}else{wgpu::BlendOperation::Add}},
                    fixed(wgpu::BlendFactor::One,wgpu::BlendFactor::One))
            } else {(color,alpha)};
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some("layer compositing"),
                layout: Some(if filtered {
                    &filtered_layout
                } else {
                    &quad_layout
                }),
                vertex: wgpu::VertexState {
                    module: &quad_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<clip_sampling::LayerVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &LAYER_ATTR,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &quad_shader,
                    entry_point: Some(if matches!(blend,Blend::StoredByte{..}) {
                        "fs_stored"
                    } else {
                        match (
                            filtered,
                            matches!(
                                blend,
                                Blend::Multiply
                                    | Blend::Screen
                                    | Blend::Atop
                                    | Blend::AtopMultiply
                                    | Blend::AtopScreen
                            ),
                        ) {
                            (false, true) => "fs_premultiply",
                            (false, false) => "fs_main",
                            (true, true) => "fs_filtered_premultiply",
                            (true, false) => "fs_filtered",
                        }
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState { color, alpha }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
            })
        };
        let quad_pipelines: Vec<_> = blends.iter().map(|b| quad_pipeline(*b, false, false)).collect();
        let filtered_pipelines: Vec<_> = blends.iter().map(|b| quad_pipeline(*b, true, false)).collect();
        let rgba_add_pipeline=quad_pipeline(Blend::Add,true,true);
        let rgba_subtract_pipeline=quad_pipeline(Blend::Subtract,true,true);
        let stored_pipeline = quad_pipeline(Blend::StoredByte {
            mode: crysirror_image::rgb_lut::ConstantByteBlend::Normal,
            force_source_opaque:false,force_output_opaque:false,
        },true,false);
        let mesh_pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("3D opaque"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3d>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_ATTR,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_main"),
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });
        let ray_composite_pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("replace raytraced color"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: Some("vs_plain"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex2d>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &QUAD_ATTR,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: Some("fs_main"),
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
            multiview_mask: None,
        });
        let puppet = Arc::new(puppet::PuppetPipelines::new(
            d,
            &texture_layout,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            puppet::premultiplied_format(&gpu.adapter),
        ));
        Self {
            composite: composite::CompositePipeline::new(d, format),
            puppet,
            transition: Arc::new(transition::TransitionPipelines::new(d, &gpu.queue, format)),
            camera_layout, texture_layout, filter_layout, coverage_layout,
            quad_pipelines, filtered_pipelines, rgba_add_pipeline, rgba_subtract_pipeline,
            mesh_pipeline, ray_composite_pipeline, stored_pipeline, coverage_white,
            _coverage_white_texture: coverage_white_texture,
        }
    }
}

impl Renderer {
    pub fn new(gpu: Arc<Gpu>, format: wgpu::TextureFormat) -> Self {
        let draw_pipelines = Arc::new(DrawPipelines::new(&gpu, format));
        Self::with_draw_pipelines(gpu, format, draw_pipelines)
    }

    fn with_draw_pipelines(gpu: Arc<Gpu>, format: wgpu::TextureFormat, draw_pipelines: Arc<DrawPipelines>) -> Self {
        let d = &gpu.device;
        let camera_buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &draw_pipelines.camera_layout,
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
        let puppets = puppet::PuppetRenderer::new(draw_pipelines.puppet.clone());
        let transitions = transition::Transitions::new(draw_pipelines.transition.clone());
        Self {
            movie_capture: false,
            movie_warmed: false,
            presentation_scale:Default::default(),
            blend_origin: [0.;2],
            histories: frame_history::Provider::new(gpu.clone()),
            gpu_timings: profile::GpuTimings::new(&gpu),
            profiler: Default::default(),
            profile_frame: 0,
            puppets,
            format,
            compositor: None,
            snapshot_plain_images: std::collections::HashMap::new(),
            snapshot_rasterizations: 0,
            draw_pipelines,
            mask_viewport:[1.,1.,0.,0.],
            coverage_planes: BTreeMap::new(),
            filter_buffer: None,
            filter_binds: std::collections::HashMap::new(),
            engine_frame: Vec::new(),
            sampler,
            textures: BTreeMap::new(),
            recent_snapshots: Default::default(),
            snapshot_readbacks: Default::default(),
            scene_texture_keys: Default::default(),
            capture_texture_keys: Default::default(),
            snapshot_readers: Default::default(),
            target_write_revisions: Default::default(),
            pending_target_outputs: Default::default(),
            target_readback_serial: 0,
            flushing_target_writes: false,
            glyph_cells: BTreeMap::new(),
            camera_buffer,
            camera_bind,
            scene3d: None,
            mesh_buffer: None,
            mesh_count: 0,
            depth: None,
            path: RenderingPath::Raster,
            raytracer: None,
            ray_lite: None,
            hardware_rt: None,
            pbr: None,
            transitions,
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
        let _span = self
            .profiler
            .span(self.profile_frame, "renderer.scene_pipelines");
        if scene.is_some() && self.pbr.is_none() {
            self.pbr = Some(pbr::PbrRenderer::new(self.gpu.clone(), self.format)?);
        }
        if let Some(p) = &mut self.pbr {
            p.set_profiler(self.profiler.clone(), self.profile_frame);
            p.set_hardware_rt(self.path == RenderingPath::HardwareRaytrace)?;
            p.set_scene(scene);
        }
        Ok(())
    }
    fn pbr_active(&self) -> bool {
        self.pbr.as_ref().is_some_and(|p| p.active())
    }
    pub fn set_profiler(&mut self, profiler: crysirror_tasks::profile::Profiler, frame: u64) {
        self.profiler = profiler;
        self.profile_frame = frame;
        if let Some(p) = &mut self.pbr {
            p.set_profiler(self.profiler.clone(), frame);
        }
    }
    pub fn profile_report(&mut self) -> Result<serde_json::Value> {
        self.gpu_timings.poll(&self.gpu)?;
        Ok(
            serde_json::json!({"timings":self.profiler.snapshot(),"cpu_trace":self.profiler.chrome_trace(),"gpu_timestamps_enabled":self.gpu_timings.enabled(),"gpu_samples_pending":self.gpu_timings.pending(),"gpu_samples_dropped":self.gpu_timings.dropped,"gpu_samples_failed":self.gpu_timings.failures,"snapshot_rasterizations":self.snapshot_rasterizations,"frame_history":self.histories.stats()}),
        )
    }
    /// Explicit capture/shutdown completion; interactive frames use profile_report.
    pub fn profile_report_offline(&mut self) -> Result<serde_json::Value> {
        self.gpu_timings.finish_offline(&self.gpu)?;
        self.profile_report()
    }
    pub fn hardware_rt_stats(&self) -> Option<serde_json::Value> {
        if self.pbr_active() {
            return self.pbr.as_ref()?.hardware_rt_stats();
        }
        let stats = self.hardware_rt.as_ref()?.stats();
        Some(
            serde_json::json!({"builds_encoded":stats.builds_encoded,"primary_dispatches":stats.primary_dispatches,"shadow_dispatches":stats.shadow_dispatches,"triangles":stats.triangles,"blas_builds_encoded":stats.blas_builds_encoded,"tlas_builds_encoded":stats.tlas_builds_encoded,"resident_blas":stats.resident_blas,"instances":stats.instances,"cpu_geometry_hits":stats.cpu_geometry_hits,"cpu_geometry_misses":stats.cpu_geometry_misses}),
        )
    }
    pub fn set_rendering_path(&mut self, path: RenderingPath) -> Result<()> {
        self.gpu.capabilities().require(path).map_err(|e| {
            anyhow::anyhow!(
                "{e}; {}",
                self.gpu.hardware_rt_report().reason.unwrap_or_default()
            )
        })?;
        if let Some(p) = &mut self.pbr {
            p.set_hardware_rt(path == RenderingPath::HardwareRaytrace)?;
        }
        if path == RenderingPath::ComputeRaytrace && self.raytracer.is_none() && self.ray_lite.is_none() {
            if ray_lite::preferred(&self.gpu) {
                self.ray_lite = Some(ray_lite::RayLite::new(&self.gpu));
            } else {
                self.raytracer = Some(raytrace::RayTracer::new(&self.gpu));
            }
        }
        self.path = path;
        Ok(())
    }
    /// Trace `ComputeRaytrace` scenes through the lite fragment profile even
    /// where compute is available: previews the console profile on desktop
    /// (it is chosen automatically on GPUs without compute storage).
    pub fn use_lite_rays(&mut self) -> Result<()> {
        ensure!(ray_lite::supported(&self.gpu), "GPU cannot run the lite ray profile");
        self.raytracer = None;
        self.ray_lite = Some(ray_lite::RayLite::new(&self.gpu));
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
    /// Install before VM boot; capabilities contain logical layer IDs only.
    pub fn mesh_frame_history_provider(&self) -> Arc<dyn MeshFrameHistoryProvider> {
        #[cfg(not(target_arch = "wasm32"))]
        { self.histories.clone() }
        #[cfg(target_arch = "wasm32")]
        { Arc::new(frame_history::BrowserHistory) }
    }
    /// Advance mesh feedback and isolated subtree input snapshots once per
    /// logical frame, also offscreen. No final framebuffer readback is needed.
    /// Queue submissions establish dependencies without a CPU readback fence.
    pub fn advance_frame_histories(
        &mut self,
        assets: &mut Assets,
        frame: Vec<Primitive>,
        canvas: [u32; 2],
    ) -> Result<()> {
        let composites = frame.iter().any(|p| p.composite.is_some());
        if !self.histories.active() && !composites {
            return Ok(());
        }
        let frame = if composites {
            frame
        } else {
            frame
                .into_iter()
                .filter(|p| {
                    p.puppet.is_some() && self.histories.registered(ClientId(p.owner), p.id)
                })
                .collect()
        };
        let previous = std::mem::take(&mut self.engine_frame);
        let result = self.set_engine_frame(frame).and_then(|()| {
            self.prepare_composites(assets, canvas)?;
            self.prepare_snapshots(assets)?;
            self.prepare(&SceneState::default(), assets, canvas)
                .map(|_| ())
        });
        self.engine_frame = previous;
        result
    }
    pub fn set_engine_frame(&mut self, frame: Vec<Primitive>) -> Result<()> {
        ensure!(frame.len() <= 32768, "extracted frame limit");
        for p in &frame {
            p.paint.validate()?;
            for allocation in p.target.iter().chain(p.output.iter()) {
                allocation.spec.validate()?;
                ensure!(allocation.owner == p.owner && allocation.generation != 0, "render target owner/generation");
            }
            ensure!(p.output.is_none() || !stored_blend::uses_stored(p), "stored-byte routed blending requires target-scoped destination capture");
            ensure!(p.output.is_none() || (p.coverage == crysirror_scene::layer::coverage::Operation::None && p.composite.is_none() && matches!(p.paint, Paint::Solid {..} | Paint::Image {..} | Paint::Text {..} | Paint::GlyphRun {..})), "unsupported routed composition payload");
            if let Some(filter) = p.filter.as_ref().filter(|filter| {
                cfg!(target_os = "vita")
                    && (filter.stencil.is_some()
                        || filter.mask.as_ref().is_some_and(|mask| mask.coverage.is_some()))
            }) {
                ensure!(filter.distortion.is_none() && !stored_blend::uses_stored(p),
                    "Vita continuous masks and canvas stencils require undistorted GPU blending");
            }
            if p.filter.as_ref().and_then(|f|f.stencil.as_ref()).is_some_and(|s|s.rgba) {
                ensure!(!matches!(p.paint,Paint::ProjectedMesh{..}) && matches!(p.blend,Blend::Normal|Blend::Add|Blend::Subtract|Blend::Multiply|Blend::Screen), "component matte needs a supported 2D blend");
            }
            if p.sampling != crysirror_scene::layer::ImageSampling::default() {
                ensure!(
                    // Shared extraction evaluates retained LegacyMesh into
                    // ProjectedMesh; that raster path already consumes the
                    // same sampler (projected_mesh.rs). Rejecting it here
                    // discarded a valid scene contract before preparation.
                    matches!(p.paint, Paint::Image { .. } | Paint::ProjectedMesh { .. })
                        && p.sampling.valid_uv(p.uv),
                    "invalid extracted image sampling: owner={} layer={} projected_mesh={} uv={:?} sampling={:?}",
                    p.owner, p.id, matches!(p.paint, Paint::ProjectedMesh { .. }), p.uv, p.sampling
                );
            }
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
    /// The texture `Paint::Backdrop` samples, in the composed frame's own format.
    fn ensure_backdrop(&mut self, size: [u32; 2]) -> Result<()> {
        let Some((_, raw, format)) = self.transitions.composing() else { return Ok(()) };
        if self.textures.get(BACKDROP_KEY).is_some_and(|t| { let s = t.texture.size(); [s.width, s.height] == size && s.width > 1 }) {
            return Ok(());
        }
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(BACKDROP_KEY),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: raw,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[format],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(format), ..Default::default() });
        let bind = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(BACKDROP_KEY),
            layout: &self.draw_pipelines.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        self.textures.insert(BACKDROP_KEY.to_owned(), Texture { bind, view, texture, bytes });
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
            self.trim_recent_snapshots_for(bytes) + bytes <= 256 * 1024 * 1024,
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
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&key),
                layout: &self.draw_pipelines.texture_layout,
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
                texture,
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
            self.trim_recent_snapshots_for(bytes) + bytes <= 256 * 1024 * 1024,
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&key),
                layout: &self.draw_pipelines.texture_layout,
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
                texture,
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
            None => format!(
                "image:{}@{}",
                assets.key(ClientId(owner), asset)?,
                assets.revision(ClientId(owner))
            ),
        })
    }
    fn ensure_image(
        &mut self,
        assets: &mut Assets,
        owner: u32,
        asset: &str,
        key: &str,
    ) -> Result<bool> {
        if self.textures.contains_key(key) {
            return Ok(true);
        }
        if key.starts_with("surface:") {
            let s = assets
                .surface(ClientId(owner), asset)
                .context("surface vanished")?
                .clone();
            self.upload(key.to_owned(), s.width, s.height, &s.rgba)?;
            return Ok(true);
        }
        let Some(image) = assets.image_for(ClientId(owner), asset)? else {
            return Ok(false);
        };
        self.upload(key.to_owned(), image.width, image.height, &image.rgba)?;
        Ok(true)
    }
    fn prepare(
        &mut self,
        scene: &SceneState,
        assets: &mut Assets,
        canvas: [u32; 2],
    ) -> Result<(Vec<clip_sampling::LayerVertex>, Vec<Draw>)> {
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
        // Dynamic puppet frames use the same mesh renderer as rig-driven frames.
        // Their source layer keeps its retained transform/opacity/order and does
        // not need a fictional rig asset merely to publish a worker result.
        for primitive in &mut frame {
            if primitive.puppet.is_some() && !matches!(primitive.paint, Paint::Puppet { .. } | Paint::Mesh { .. }) {
                primitive.paint = Paint::Puppet {
                    asset: String::new(),
                    tint: [255; 4],
                };
            }
        }
        frame.sort_by(crysirror_scene::layer::compare_primitives);
        // Retained runs remain logical engine state; only extraction expands
        // them to independent reusable glyph resources and transformed quads.
        let frame = self.expand_glyph_runs(frame, assets)?;
        // Freeze all old pictures before any new layer is drawn, including
        // mutual/cross-layer references. Duplicate extraction of one immutable
        // declaration does not advance history a second time.
        let mut history_render = std::collections::BTreeMap::new();
        let mut history_dependencies = BTreeMap::new();
        for p in &frame {
            if self.histories.registered(ClientId(p.owner), p.id) {
                if let Some(mesh) = &p.puppet {
                    let size = Self::puppet_size(p);
                    let key = format!("puppet:{}:{}:{}x{}", p.owner, p.id, size[0], size[1]);
                    if !self.textures.contains_key(&key) {
                        self.render_texture(key.clone(), size)?;
                    }
                    let dependencies = mesh
                        .textures
                        .iter()
                        .map(|t| {
                            if crysirror_core::service::frame_history_layer(t).is_some() {
                                Ok(t.clone())
                            } else {
                                Self::image_key(assets, p.owner, t)
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let needed = self.histories.begin(
                        p.owner,
                        p.id,
                        size,
                        mesh,
                        &self.textures[&key].texture,
                        &dependencies,
                    )?;
                    history_dependencies.insert((p.owner, p.id), dependencies);
                    history_render.insert((p.owner, p.id), needed);
                }
            }
        }
        let mut sources = BTreeSet::new();
        for p in &frame {
            if history_render.get(&(p.owner, p.id)) == Some(&false) {
                continue;
            }
            if let Some(mesh) = &p.puppet {
                for texture in &mesh.textures {
                    if let Some(layer) = crysirror_core::service::frame_history_layer(texture) {
                        sources.insert((p.owner, layer));
                    }
                }
            }
        }
        for (owner, layer) in sources {
            self.histories.freeze(owner, layer)?;
        }
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
            .map(|p| {
                if let Some(target) = &p.target {
                    let key = Self::target_key(assets, target);
                    return Ok(if p.output.as_ref() == Some(target) { format!("{key}:read") } else { key });
                }
                match &p.paint {
                Paint::Solid { .. } => Ok("solid:white".to_string()),
                Paint::ProjectedMesh { .. } => Ok("solid:white".to_string()),
                Paint::LegacyMesh { .. } => anyhow::bail!("legacy mesh must be evaluated by shared layer extraction"),
                Paint::CoverageWrite{..}|Paint::CoverageReset{..}|Paint::GlyphView{..}|Paint::Group { .. } | Paint::OrderedState { .. } => anyhow::bail!("composition command must be consumed by shared layer extraction"),
                Paint::Backdrop { .. } => Ok(BACKDROP_KEY.to_string()),
                Paint::Snapshot { size, layers } => {
                    Self::snapshot_key(assets, p.owner, *size, layers)
                }
                Paint::Image { asset, .. } => {
                    Self::sampled_image_key(assets, p.owner, asset, p.sampling)
                }
                Paint::Text { .. } => assets.text.for_client(crysirror_core::state::ClientId(p.owner)).key(&text(p).expect("text paint")),
                Paint::GlyphRun { .. } => glyph_run::key(assets, p.owner, &p.paint),
                Paint::Puppet { .. } | Paint::Mesh { .. } => {
                    let [w, h] = Self::puppet_size(p);
                    Ok(format!("puppet:{}:{}:{w}x{h}", p.owner, p.id))
                }
            }})
            .collect::<Result<Vec<_>>>()?;
        // Layer filters: mask / tone-curve textures (white and identity by default).
        let coverage_owners: BTreeSet<_>=frame.iter().filter(|p|p.coverage!=crysirror_scene::layer::coverage::Operation::None).map(|p|p.owner).collect();
        let filter_keys = frame
            .iter()
            .map(|p| {
                let Some(f) = &p.filter else {
                    return Ok((p.target.as_ref().is_some_and(|t|t.spec.opaque)||coverage_owners.contains(&p.owner)||matches!(p.paint,Paint::ProjectedMesh{..}) || stored_blend::uses_stored(p)
                        ||p.sampling.profile==crysirror_scene::layer::ImageSampleProfile::StoredByte128)
                        .then(||("solid:white".to_string(),"curve:identity".to_string())));
                };
                let mask = match &f.mask {
                    Some(m) => Self::image_key(assets, p.owner, &m.asset)?,
                    None => "solid:white".to_string(),
                };
                let curve = match &f.tone_curve {
                    Some(t) => Self::image_key(assets, p.owner, &t.asset)?,
                    None => match &f.stencil {Some(s)=>Self::image_key(assets,p.owner,&s.asset)?,None=>"curve:identity".to_string()},
                };
                Ok(Some((mask, curve)))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut used: BTreeSet<String> = keys.iter().cloned().collect();
        if frame.iter().any(stored_blend::uses_stored) { used.insert(BACKDROP_KEY.into()); }
        // Sampling aliases share the admitted image allocation. Keep its base
        // key resident so alias creation never requires a second image upload.
        for p in &frame {
            for target in p.target.iter().chain(p.output.iter()) {
                used.insert(Self::target_key(assets, target));
            }
            if let Paint::Image { asset, .. } = &p.paint {
                if p.target.is_none() {used.insert(Self::image_key(assets, p.owner, asset)?);}
            }
            if let Paint::ProjectedMesh { frame, .. } = &p.paint {
                for part in &frame.parts {
                    if let Some(asset) = &part.texture {
                        used.insert(Self::image_key(assets,p.owner,asset)?);
                        used.insert(Self::sampled_image_key_filter(assets,p.owner,asset,p.sampling,part.linear)?);
                    }
                }
            }
        }
        let mut puppet_textures: Vec<Vec<String>> = Vec::new();
        for p in &frame {
            let names = match (&p.paint, &p.puppet) {
                (Paint::Puppet { .. } | Paint::Mesh { .. }, Some(f)) => f
                    .textures
                    .iter()
                    .map(|t| match crysirror_core::service::frame_history_layer(t) {
                        Some(id) => {
                            ensure!(
                                self.histories.registered(ClientId(p.owner), id),
                                "unreserved GPU frame history"
                            );
                            Ok(format!("history:{}:{id}", p.owner))
                        }
                        // A mesh may sample what the last Paint::Backdrop captured.
                        None if t == BACKDROP_KEY => Ok(BACKDROP_KEY.to_string()),
                        None => Self::image_key(assets, p.owner, t),
                    })
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
        self.preserve_capture_textures(&mut used);
        self.retain_frame_textures(&used);
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
                if let Some(s)=&f.stencil {self.ensure_image(assets,p.owner,&s.asset,ck)?;}
            }
        }
        let mut records = vec![FilterGpu {
            a: [0.; 4],
            b: [0.; 4],
            c: [0.; 4],
            d: [0.; 4],
            e: [0.; 4],
            f: [0.2989,0.5886,0.1145,0.],
            g: [0.; 4],
            h: [1.; 4],
            i: [0.;4],
        }];
        let mut vertices = Vec::with_capacity(frame.len() * 6);
        let mut draws = Vec::new();
        let mut pending_targets = BTreeSet::new();
        for p in &frame {
            for allocation in p.target.iter().chain(p.output.iter()) {
                let key = Self::target_key(assets,allocation);
                if !self.textures.contains_key(&key) { pending_targets.insert(key); }
            }
        }
        let mut coverage_active=BTreeMap::new();
        for (((p, key), fkeys), tex_keys) in
            frame.iter().zip(keys).zip(filter_keys).zip(puppet_textures)
        {
            use crysirror_scene::layer::coverage::Operation as C;
            let draw_canvas = p.output.as_ref().map_or(canvas, |t| t.spec.size);
            let output = p.output.as_ref().map(|t| (Self::target_key(assets, t), t.spec.size, t.revision));
            if output.as_ref().is_some_and(|(key,_,revision)| self.target_write_revisions.get(key)==Some(revision)) { continue; }
            if output.as_ref().is_some_and(|(key,_,_)| !self.textures.contains_key(key)) { continue; }
            if p.target.as_ref().is_some_and(|t| !self.textures.contains_key(&Self::target_key(assets,t))) { continue; }
            if p.target.is_some() && p.opacity == 0 { continue; }
            let active=coverage_active.entry(p.owner).or_insert(false);
            let consume=match p.coverage {C::None=>*active && p.output.is_none(),C::Write=>{*active=true;false},C::Reset{value}=>{*active=value!=255;false}};
            if let C::Reset{..}=p.coverage {
                draws.push(Draw{rgba_contribution:false,output:None,feedback_source:None,owner:p.owner,coverage:p.coverage,key:"solid:white".into(),range:0..0,clip:None,blend:Blend::Normal,capture_after:false,filter:None});
                continue;
            }
            if matches!(p.paint,Paint::ProjectedMesh{..}) {
                self.prepare_projected_mesh(p,assets,canvas,&mut records,&mut vertices,&mut draws,fkeys,consume)?;
                continue;
            }
            // The capture exists from the first frame a Backdrop layer has drawn.
            if tex_keys.iter().any(|key| key == BACKDROP_KEY && !self.textures.contains_key(key)) {
                continue;
            }
            if fkeys.as_ref().is_some_and(|(a, b)| {
                !self.textures.contains_key(a) || !self.textures.contains_key(b)
            }) {
                if let Some((key,_,_)) = &output { pending_targets.insert(key.clone()); }
                continue;
            }
            let color = if p.target.is_some() {match &p.paint {Paint::Image {tint,..}=>*tint,_=>[255;4]}} else { match &p.paint {
                Paint::ProjectedMesh { .. } => unreachable!("projected mesh handled before quads"),
                Paint::LegacyMesh { .. } => unreachable!("legacy mesh evaluated during shared layer extraction"),
                Paint::CoverageWrite{..}|Paint::CoverageReset{..}|Paint::GlyphView{..}|Paint::Group { .. } | Paint::OrderedState { .. } => unreachable!("composition commands consumed during shared layer extraction"),
                Paint::Solid { color } => {
                    if !self.textures.contains_key(&key) {
                        self.upload(key.clone(), 1, 1, &[255; 4])?;
                    }
                    *color
                }
                // A placeholder until the frame's size is known at the copy.
                Paint::Backdrop { .. } => {
                    if !self.textures.contains_key(&key) {
                        self.upload(key.clone(), 1, 1, &[0; 4])?;
                    }
                    [255; 4]
                }
                Paint::Snapshot { .. } => {
                    // A pending immutable dependency has not published a texture.
                    if !self.textures.contains_key(&key) {
                        continue;
                    }
                    [255; 4]
                }
                Paint::Image { asset, tint } => {
                    if !self.ensure_sampled_image(assets, p.owner, asset, &key, p.sampling)? {
                        if let Some((key,_,_)) = &output { pending_targets.insert(key.clone()); }
                        continue;
                    }
                    *tint
                }
                Paint::Puppet { tint, .. } | Paint::Mesh { tint, .. } => {
                    let size = Self::puppet_size(p);
                    if !self.textures.contains_key(&key) {
                        self.render_texture(key.clone(), size)?;
                    }
                    let mut ready = true;
                    if let Some(f) = &p.puppet {
                        for (t, k) in f.textures.iter().zip(&tex_keys) {
                            if let Some(id) = crysirror_core::service::frame_history_layer(t) {
                                let texture = self.histories.history(p.owner, id)?;
                                let view = texture.create_view(&Default::default());
                                let bind =
                                    self.gpu
                                        .device
                                        .create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some("frame history binding"),
                                            layout: &self.draw_pipelines.texture_layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &view,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 1,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.sampler,
                                                    ),
                                                },
                                            ],
                                        });
                                self.textures.insert(
                                    k.clone(),
                                    Texture {
                                        texture,
                                        view,
                                        bind,
                                        bytes: 0,
                                    },
                                );
                            } else {
                                ready &= self.ensure_image(assets, p.owner, t, k)?;
                            }
                        }
                    }
                    if !ready {
                        continue;
                    }
                    let textures = &self.textures;
                    if history_render.get(&(p.owner, p.id)) != Some(&false) {
                        self.puppets.render(
                            &self.gpu.device,
                            &self.gpu.queue,
                            &self.draw_pipelines.texture_layout,
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
                        if let Some(mesh) = &p.puppet {
                            self.histories.complete(
                                p.owner,
                                p.id,
                                mesh.clone(),
                                textures[&key].texture.clone(),
                                history_dependencies
                                    .remove(&(p.owner, p.id))
                                    .unwrap_or_default(),
                            )?;
                        }
                    }
                    *tint
                }
                Paint::Text { color, .. } => {
                    if !self.textures.contains_key(&key) {
                        let run = text(p).expect("text paint");
                        let pixels = assets.text.for_client(crysirror_core::state::ClientId(p.owner)).rasterize(&run)?;
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
            }};
            if p.filter.as_ref().and_then(|f|f.stencil.as_ref()).is_some_and(|s|s.rgba) {
                self.prepare_rgba_stencil(p,draw_canvas,color,key,fkeys,output,consume,&mut records,&mut vertices,&mut draws,assets)?;
                continue;
            }
            let start = vertices.len() as u32;
            let mut quad = clip_sampling::quad(p, draw_canvas, color);
            // Transparent solid containers (common VM group roots) cannot
            // change any pixel under these blends: the unfiltered shader
            // emits white·colour, and each blend is the identity for it.
            // Rasterising them anyway costs a full-target fill per layer.
            if p.target.is_none() && matches!(p.paint,Paint::Solid{..}) && !consume && p.filter.is_none()
                && p.coverage==C::None
                && p.sampling.profile!=crysirror_scene::layer::ImageSampleProfile::StoredByte128 {
                let alpha_zero=quad.iter().all(|v|v.color[3]==0.);
                let all_zero=quad.iter().all(|v|v.color==[0.;4]);
                if match p.blend {
                    Blend::Normal|Blend::Add|Blend::Subtract=>alpha_zero,
                    Blend::SourceOver|Blend::Screen=>all_zero,
                    _=>false,
                } {
                    continue;
                }
            }
            if p.target.as_ref().is_some_and(|t|t.spec.opaque) || consume || p.filter.is_some() || matches!(p.blend,Blend::StoredByte{..})
                ||p.sampling.profile==crysirror_scene::layer::ImageSampleProfile::StoredByte128 {
                let index = records.len() as f32;
                let mut record=filter_gpu(&p.filter.clone().unwrap_or_default(),if p.output.is_some(){[1.,1.,0.,0.]}else{self.mask_viewport});
                filter_sampling(&mut record,p.sampling);
                stored_blend::configure(&mut record,p.blend,if p.output.is_some(){[0.;2]}else{self.blend_origin},p.opacity);
                if p.target.as_ref().is_some_and(|t|t.spec.opaque) { record.i[3]=(record.i[3] as u32 | 1) as f32; }
                coverage::configure(&mut record,consume);
                records.push(record);
                for v in &mut quad {
                    v.filter = index;
                }
            }
            vertices.extend_from_slice(&quad);
            draws.push(Draw {
                rgba_contribution:false,
                output,
                feedback_source: p.target.as_ref().filter(|t|p.output.as_ref()==Some(*t)).map(|t|Self::target_key(assets,t)),
                owner:p.owner,coverage:p.coverage,
                key,
                range: start..start + 6,
                clip: p.clip,
                blend: p.blend,
                capture_after: matches!(p.paint, Paint::Backdrop { history: true }),
                filter: fkeys,
            });
        }
        // Each destination is admitted as one dependency set. No partial
        // alpha writes are replayed when a later image/curve finishes loading.
        loop {
            let before = pending_targets.len();
            for p in &frame {
                if let (Some(source),Some(destination)) = (&p.target,&p.output) {
                    if pending_targets.contains(&Self::target_key(assets,source)) {
                        pending_targets.insert(Self::target_key(assets,destination));
                    }
                }
            }
            if pending_targets.len()==before { break; }
        }
        draws.retain(|draw|draw.output.as_ref().is_none_or(|(key,_,_)|!pending_targets.contains(key)));
        self.pending_target_outputs = pending_targets;
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
                        layout: &self.draw_pipelines.filter_layout,
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
                            wgpu::BindGroupEntry {
                                binding:3,
                                resource:wgpu::BindingResource::TextureView(&self.textures
                                    .get(BACKDROP_KEY).unwrap_or(&self.textures["solid:white"]).view),
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
    pub fn set_presentation_scale(&mut self, mode:crysirror_core::service::PresentationScale) {
        self.presentation_scale=mode;
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
        self.scene_texture_keys = Self::scene_cache_keys(assets, scene)?;
        self.prepare_render_targets(assets)?;
        self.prepare_snapshots(assets)?;
        self.prepare_composites(assets, canvas)?;
        //Destination bindings are built during prepare. Admit the actual
        //composition texture first, including on the renderer's first frame,
        //so StoredByte never retains the neutral fallback in its bind group.
        if !self.flushing_target_writes { self.transitions.begin(&self.gpu.device,target.size); }
        let blend_viewport=Viewport::with_mode(canvas,target.size,self.presentation_scale);
        self.mask_viewport=[target.size[0] as f32/blend_viewport.width,target.size[1] as f32/blend_viewport.height,
            blend_viewport.x/blend_viewport.width,blend_viewport.y/blend_viewport.height];
        self.blend_origin=[blend_viewport.x.max(0.).floor(),blend_viewport.y.max(0.).floor()];
        if self.engine_frame.iter().any(stored_blend::uses_stored) {
            let vp=Viewport::with_mode(canvas,target.size,self.presentation_scale);
            self.ensure_backdrop([vp.width.ceil() as u32,vp.height.ceil() as u32])?;
        }
        let owners=self.engine_frame.iter().filter(|p|p.coverage!=crysirror_scene::layer::coverage::Operation::None).map(|p|p.owner).collect();
        self.ensure_coverage(&owners,target.size)?;
        let (vertices, draws) = self.prepare(scene, assets, canvas)?;
        self.sync_transition_mask(assets)?;
        if !self.flushing_target_writes { self.depth(target.size); }
        if let Some(s) = self.scene3d.as_ref().filter(|_|!self.flushing_target_writes) {
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
        self.gpu_timings.poll(&self.gpu)?;
        let _profile_span = self
            .profiler
            .span(self.profile_frame, "renderer.prepare_encode_submit");
        let vp = Viewport::with_mode(canvas, target.size, self.presentation_scale);
        let color = geometry::tint(background);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        // Each render owns a fresh encoder and submits internally on success.
        // Any tokens left by a prior error belong to an abandoned encoder.
        if !self.flushing_target_writes {
            if let Some(p) = &mut self.pbr { p.cancel_unsubmitted_vfx()?; }
        }
        let gpu_sample =
            self.gpu_timings
                .begin(&mut encoder, self.profiler.clone(), self.profile_frame);
        let pbr_active = !self.flushing_target_writes && self.pbr_active();
        // Compose into a retained frame; `transitions.finish` presents it.
        if !self.flushing_target_writes { self.transitions.begin(&self.gpu.device, target.size); }
        let backdrop_source = if draws.iter().any(|draw| draw.key == BACKDROP_KEY || matches!(draw.blend,Blend::StoredByte{..})) {
            self.ensure_backdrop([vp.width.ceil() as u32, vp.height.ceil() as u32])?;
            self.transitions.composing()
        } else {
            None
        };
        let frame_view = if self.flushing_target_writes {target.view} else {self.transitions.begin(&self.gpu.device, target.size)};
        ensure!(
            !(pbr_active && self.path == RenderingPath::ComputeRaytrace),
            "the PBR scene renders on the raster path only; compute ray tracing of PBR materials is not implemented"
        );
        let ray_texture = if !self.flushing_target_writes && self.path == RenderingPath::HardwareRaytrace && !pbr_active {
            let scene = self
                .scene3d
                .as_ref()
                .context("hardware RT requires a 3D scene")?;
            let triangles = ray_geometry::simple(scene)?;
            let inverse =
                glam::Mat4::from_cols_array_2d(&geometry::camera(scene, canvas)).inverse();
            if self.hardware_rt.is_none() {
                self.hardware_rt = Some(hardware_rt::HardwareRt::new(&self.gpu)?);
            }
            Some(
                self.hardware_rt
                    .as_mut()
                    .context("hardware RT missing")?
                    .encode_primary(
                        &self.gpu,
                        &triangles,
                        inverse.to_cols_array_2d(),
                        canvas,
                        color,
                        &mut encoder,
                    )?,
            )
        } else if let (false, RenderingPath::ComputeRaytrace, Some(lite)) = (self.flushing_target_writes, self.path, &self.ray_lite) {
            Some(lite.encode(
                &self.gpu,
                self.scene3d.as_ref().context("compute raytrace requires a 3D scene")?,
                canvas,
                background,
                &mut encoder,
            )?)
        } else if !self.flushing_target_writes && self.path == RenderingPath::ComputeRaytrace {
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
                multiview_mask: None,
                label: Some("3D/background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
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
                depth_stencil_attachment: if self.flushing_target_writes { None } else { Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().context("depth target")?.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }) },
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(buffer) = self
                .mesh_buffer
                .as_ref()
                .filter(|_| !self.flushing_target_writes && self.path == RenderingPath::Raster && !pbr_active)
            {
                pass.set_viewport(vp.x, vp.y, vp.width, vp.height, 0., 1.);
                pass.set_pipeline(&self.draw_pipelines.mesh_pipeline);
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
                    layout: &self.draw_pipelines.texture_layout,
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
                multiview_mask: None,
                label: Some("raytrace composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
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
            pass.set_pipeline(&self.draw_pipelines.ray_composite_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
        if let Some(buffer) = &quad_buffer {
            use crysirror_scene::layer::coverage::Operation as C;
            let mut active=BTreeSet::new();
            // Consecutive draws into the same attachment share one render
            // pass. A pass per draw reloads/stores the whole target each time,
            // which on tile-based GPUs costs full-frame bandwidth per sprite.
            // Passes end only at ordering fences: coverage clears, backdrop
            // copies and attachment changes. Target key: None = frame view,
            // Some(owner) = that owner's coverage plane.
            let mut open:Option<(Option<String>,wgpu::RenderPass<'static>)>=None;
            for draw in &draws {
                let writing=draw.coverage==C::Write;
                if let C::Reset{value}=draw.coverage {
                    open=None;
                    if value==255 {active.remove(&draw.owner);} else {
                        let view=&self.coverage_planes[&draw.owner].view;
                        let shade=geometry::tint([value,value,value,255]);
                        let _clear=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("uniform ordered coverage"),multiview_mask:None,
                            color_attachments:&[Some(wgpu::RenderPassColorAttachment{view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color{r:shade[0]as f64,g:shade[1]as f64,b:shade[2]as f64,a:1.}),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None});
                        active.insert(draw.owner);
                    }
                    continue;
                }
                if writing&&active.insert(draw.owner) {
                    open=None;
                    //A newly admitted coverage writer starts on zero. Later
                    //writers blend over the same plane until an ordered reset.
                    let _clear=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("initial ordered coverage"),multiview_mask:None,
                        color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&self.coverage_planes[&draw.owner].view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::BLACK),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None});
                }
                if draw.range.is_empty(){continue;}
                if let Some(source) = &draw.feedback_source {
                    open = None;
                    let source = &self.textures[source].texture;
                    let destination = &self.textures[&draw.key].texture;
                    encoder.copy_texture_to_texture(source.as_image_copy(), destination.as_image_copy(), source.size());
                }
                let draw_canvas = draw.output.as_ref().map_or(canvas, |(_,size,_)| *size);
                let draw_size = draw.output.as_ref().map_or(target.size, |(_,size,_)| *size);
                let draw_vp = if draw.output.is_some() { Viewport::with_mode(draw_canvas, draw_size, crysirror_core::service::PresentationScale::default()) } else { vp };
                let capture=|encoder:&mut wgpu::CommandEncoder|->Result<()> {
                    let source=if writing {Some(self.coverage_planes[&draw.owner].texture.clone())}
                        else if let Some((key,_,_)) = &draw.output {Some(self.textures[key].texture.clone())}
                        else {backdrop_source.as_ref().map(|(source,_,_)|source.clone())};
                    if let Some(source)=source {
                        let destination=&self.textures.get(BACKDROP_KEY).context("backdrop texture")?.texture;
                        let extent=destination.size();let origin=[draw_vp.x.max(0.)as u32,draw_vp.y.max(0.)as u32];
                        let copy=wgpu::Extent3d{width:extent.width.min(source.width().saturating_sub(origin[0])),height:extent.height.min(source.height().saturating_sub(origin[1])),depth_or_array_layers:1};
                        encoder.copy_texture_to_texture(wgpu::TexelCopyTextureInfo{texture:&source,mip_level:0,origin:wgpu::Origin3d{x:origin[0],y:origin[1],z:0},aspect:wgpu::TextureAspect::All},wgpu::TexelCopyTextureInfo{texture:destination,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},copy);
                    }Ok(())
                };
                if draw.key==BACKDROP_KEY&&!draw.capture_after||matches!(draw.blend,Blend::StoredByte{..}){open=None;capture(&mut encoder)?;}
                let target_key=if writing {Some(format!("coverage:{}",draw.owner))}else{draw.output.as_ref().map(|(key,_,_)|key.clone())};
                if open.as_ref().is_none_or(|(key,_)|*key!=target_key) {
                    open=None;
                    let view=if writing{&self.coverage_planes[&draw.owner].view}else if let Some((key,_,_))=&draw.output{&self.textures[key].view}else{frame_view};
                    let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("ordered 2D draws"),multiview_mask:None,
                        color_attachments:&[Some(wgpu::RenderPassColorAttachment{view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None}).forget_lifetime();
                    pass.set_viewport(draw_vp.x,draw_vp.y,draw_vp.width,draw_vp.height,0.,1.);
                    pass.set_vertex_buffer(0,buffer.slice(..));
                    open=Some((target_key,pass));
                }
                let pass=&mut open.as_mut().context("ordered 2D pass")?.1;
                let c = draw
                    .clip
                    .unwrap_or([0., 0., draw_canvas[0] as f32, draw_canvas[1] as f32]);
                let x = (draw_vp.x + c[0] * draw_vp.width / draw_canvas[0] as f32)
                    .floor()
                    .clamp(0., draw_size[0] as f32) as u32;
                let y = (draw_vp.y + c[1] * draw_vp.height / draw_canvas[1] as f32)
                    .floor()
                    .clamp(0., draw_size[1] as f32) as u32;
                let right = (draw_vp.x + (c[0] + c[2]) * draw_vp.width / draw_canvas[0] as f32)
                    .ceil()
                    .clamp(0., draw_size[0] as f32) as u32;
                let bottom = (draw_vp.y + (c[1] + c[3]) * draw_vp.height / draw_canvas[1] as f32)
                    .ceil()
                    .clamp(0., draw_size[1] as f32) as u32;
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
                    Blend::Atop => 5,
                    Blend::AtopMultiply => 6,
                    Blend::AtopScreen => 7,
                    Blend::AtopCopy => 8,
                    Blend::SourceOver => 9,
                    Blend::Modulate => 10,
                    Blend::InverseDestination => 11,
                    Blend::ModulateAdd => 12,
                    Blend::DestinationIn => 13,
                    Blend::StoredByte { .. } => 0,
                };
                if draw.rgba_contribution {
                    pass.set_pipeline(if draw.blend==Blend::Subtract {&self.draw_pipelines.rgba_subtract_pipeline}else{&self.draw_pipelines.rgba_add_pipeline});
                    pass.set_bind_group(1,self.filter_binds.get(draw.filter.as_ref().context("component matte filter record")?).context("component matte filter binding")?,&[]);
                } else if matches!(draw.blend,Blend::StoredByte{..}) {
                    pass.set_pipeline(&self.draw_pipelines.stored_pipeline);
                    pass.set_bind_group(1,self.filter_binds.get(draw.filter.as_ref()
                        .context("stored blend filter record missing")?)
                        .context("stored blend destination binding missing")?,&[]);
                } else { match &draw.filter {
                    Some(k) => {
                        pass.set_pipeline(&self.draw_pipelines.filtered_pipelines[blend_index]);
                        pass.set_bind_group(
                            1,
                            self.filter_binds.get(k).context("filter bind group")?,
                            &[],
                        );
                    }
                    None => pass.set_pipeline(&self.draw_pipelines.quad_pipelines[blend_index]),
                } }
                pass.set_bind_group(
                    0,
                    &self
                        .textures
                        .get(&draw.key)
                        .context("prepared texture disappeared")?
                        .bind,
                    &[],
                );
                if draw.filter.is_some() || matches!(draw.blend,Blend::StoredByte{..}) {
                    let coverage=if !writing&&active.contains(&draw.owner){&self.coverage_planes[&draw.owner].bind}else{&self.draw_pipelines.coverage_white};
                    pass.set_bind_group(2,coverage,&[]);
                }
                pass.draw(draw.range.clone(), 0..1);

                if draw.key==BACKDROP_KEY&&draw.capture_after{open=None;capture(&mut encoder)?;}
            }
            drop(open);
        }
        if !self.flushing_target_writes { self.transitions.finish(
            &self.gpu.device,
            &mut encoder,
            target.view,
            self.transition.as_ref(),
            canvas,
        ); }
        self.gpu_timings.end(&mut encoder, gpu_sample);
        self.gpu.queue.submit(Some(encoder.finish()));
        for draw in &draws {
            if let Some((key,_,revision)) = &draw.output { self.target_write_revisions.insert(key.clone(),*revision); }
        }
        self.gpu_timings.submitted(&self.gpu, gpu_sample);
        if !self.flushing_target_writes && self.path == RenderingPath::HardwareRaytrace && !pbr_active {
            if let Some(rt) = &mut self.hardware_rt {
                rt.mark_submitted();
            }
        }
        if pbr_active {
            if let Some(p) = &mut self.pbr {
                p.after_submit()?;
            }
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
            matches!(
                self.format,
                wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
            ),
            "capture renderer must use RGBA8 or BGRA8 sRGB"
        );
        // Developer diagnostic: CRYSIRROR_CAPTURE_TARGET=WxH renders into a
        // target of another size (as a HiDPI window surface does).
        let (width, height) = std::env::var("CRYSIRROR_CAPTURE_TARGET")
            .ok()
            .and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or((canvas[0], canvas[1]));
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
        let passes = if self.movie_capture && self.movie_warmed {
            1
        } else {
            self.pbr
                .as_ref()
                .filter(|p| p.active())
                .map_or(1, |p| p.warmup_frames())
        };
        self.movie_warmed = self.pbr.as_ref().is_some_and(|p| p.active());
        for _ in 0..passes {
            self.render(
                scene,
                assets,
                RenderTarget {
                    view: &view,
                    size: [width, height],
                },
                canvas,
                background,
            )?;
        }
        self.read_texture(&texture, width, height)
    }
    /// Read back an RGBA8/BGRA8 sRGB texture with COPY_SRC usage (captures;
    /// a window surface configured with COPY_SRC for diagnostics).
    pub fn read_texture(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<image::RgbaImage> {
        frame_history::read_texture(&self.gpu, texture, width, height)
    }
}

pub mod effect;
