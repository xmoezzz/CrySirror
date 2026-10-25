// SPDX-License-Identifier: MPL-2.0
//! Box reflection captures (ADR 0024): GPU resources and slot bookkeeping.
//! The capture render itself lives in `pbr.rs` because it reuses the frame's
//! draw list and surface pipelines.
use glam::Vec3;

pub const MAX_PROBES: usize = 8;
/// Equirect probe map width (height is half); mips match the environment.
pub const PROBE_W: u32 = 256;
pub const FACE: u32 = 128;
/// Irradiance probe capture faces (ADR 0028): diffuse light and 8×8
/// octahedral visibility need far less than reflections.
pub const GI_FACE: u32 = 32;
/// (forward, up) of the six capture faces; shared with `probe.wgsl` via a uniform.
pub const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// What a slot holds; a capture is redone when this changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Captured {
    pub key: u64,
    pub revision: u32,
    pub position: [f32; 3],
    pub environment: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeStats {
    pub submitted: usize,
    pub active: usize,
    pub captured_this_frame: usize,
}

pub struct ProbeResources {
    pub maps: wgpu::Texture,
    /// D2Array view of all slots and mips (bound in the frame group).
    pub maps_view: wgpu::TextureView,
    pub faces: wgpu::Texture,
    pub faces_view: wgpu::TextureView,
    pub msaa_color: wgpu::TextureView,
    pub msaa_depth: wgpu::TextureView,
    pub gi_faces: wgpu::Texture,
    pub gi_faces_view: wgpu::TextureView,
    pub gi_msaa_color: wgpu::TextureView,
    pub gi_msaa_depth: wgpu::TextureView,
    pub source: wgpu::Texture,
    pub source_mips: u32,
    pub bgl: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub faces_buf: wgpu::Buffer,
    pub slots: Vec<Option<Captured>>,
    pub stats: ProbeStats,
}

impl ProbeResources {
    pub fn new(
        d: &wgpu::Device,
        q: &wgpu::Queue,
        hdr: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        msaa: u32,
        mips: u32,
        module: &wgpu::ShaderModule,
    ) -> Self {
        let tex = |label: &str, w: u32, h: u32, layers: u32, mips: u32, samples: u32, format, usage| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers },
                mip_level_count: mips,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rs = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let maps = tex("reflection probes", PROBE_W, PROBE_W / 2, MAX_PROBES as u32, mips, 1, hdr, rs);
        let maps_view = maps.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let faces = tex("probe capture faces", FACE, FACE, 6, 1, 1, hdr, rs);
        let faces_view = faces.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa_color = tex("probe msaa color", FACE, FACE, 1, 1, msaa, hdr, ra).create_view(&Default::default());
        let msaa_depth = tex("probe msaa depth", FACE, FACE, 1, 1, msaa, depth, ra).create_view(&Default::default());
        let gi_faces = tex("irradiance capture faces", GI_FACE, GI_FACE, 6, 1, 1, hdr, rs);
        let gi_faces_view = gi_faces.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let gi_msaa_color = tex("irradiance msaa color", GI_FACE, GI_FACE, 1, 1, msaa, hdr, ra).create_view(&Default::default());
        let gi_msaa_depth = tex("irradiance msaa depth", GI_FACE, GI_FACE, 1, 1, msaa, depth, ra).create_view(&Default::default());
        let source_mips = PROBE_W.ilog2() + 1;
        let source = tex("probe equirect source", PROBE_W, PROBE_W / 2, 1, source_mips, 1, hdr, rs);
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe faces"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
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
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("probe faces"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("probe cube to equirect"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: "fs_cube_to_equirect",
                targets: &[Some(hdr.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });
        let mut basis = [[0f32; 4]; 12];
        for (k, (f, u)) in FACES.iter().enumerate() {
            basis[2 * k] = [f.x, f.y, f.z, 0.];
            basis[2 * k + 1] = [u.x, u.y, u.z, 0.];
        }
        let faces_buf = wgpu::util::DeviceExt::create_buffer_init(d, &wgpu::util::BufferInitDescriptor {
            label: Some("probe face bases"),
            contents: bytemuck::cast_slice(&basis),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let _ = q;
        Self {
            maps,
            maps_view,
            faces,
            faces_view,
            msaa_color,
            msaa_depth,
            gi_faces,
            gi_faces_view,
            gi_msaa_color,
            gi_msaa_depth,
            source,
            source_mips,
            bgl,
            pipeline,
            faces_buf,
            slots: vec![None; MAX_PROBES],
            stats: ProbeStats::default(),
        }
    }
    pub fn layer_view(t: &wgpu::Texture, layer: u32, mip: u32) -> wgpu::TextureView {
        t.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: mip,
            mip_level_count: Some(1),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        })
    }
}
