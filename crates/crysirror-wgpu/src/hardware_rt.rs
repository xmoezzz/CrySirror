// SPDX-License-Identifier: MPL-2.0
//! Native acceleration structures and hardware ray queries. No software traversal.
//! CPU callers supply an owned/immutable triangle snapshot. GPU resources belong
//! to this renderer service and are never serialized into project/VM data.
use crate::Gpu;
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use std::hash::{Hash, Hasher};
use wgpu::util::DeviceExt;

const MAX_TRIANGLES: usize = 1_048_576;
const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_OUTPUT_PIXELS: u64 = 16_777_216;
const IDENTITY: [f32; 12] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];

/// Opaque world-space triangle. Alpha-tested/translucent geometry must be
/// rejected by the frontend until an explicit candidate/alpha policy is added.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RtTriangle {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub c: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Metadata {
    normal: [f32; 4],
    color: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PrimaryParams {
    inverse_view_projection: [[f32; 4]; 4],
    background: [f32; 4],
    size_count: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowParams {
    inverse_view_projection: [[f32; 4]; 4],
    inverse_view: [[f32; 4]; 4],
    towards_sun: [f32; 4],
    size: [u32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HardwareRtStats {
    /// Encoded build commands, not GPU completion measurements.
    pub builds_encoded: u64,
    pub primary_dispatches: u64,
    pub shadow_dispatches: u64,
    pub triangles: usize,
}
struct AccelerationScene {
    hash: u64,
    positions: Vec<[f32; 3]>,
    _vertices: wgpu::Buffer,
    blas: wgpu::Blas,
    tlas: wgpu::Tlas,
    submitted: bool,
}
/// A bounded opaque-triangle hardware RT service shared by simple primary rays
/// and PBR directional visibility. Geometry changes rebuild native AS; material
/// metadata changes do not. All work is encoded into the caller's GPU graph.
pub(crate) struct HardwareRt {
    primary: wgpu::ComputePipeline,
    primary_layout: wgpu::BindGroupLayout,
    shadow: wgpu::ComputePipeline,
    shadow_layout: wgpu::BindGroupLayout,
    clear_shadow: wgpu::ComputePipeline,
    clear_shadow_layout: wgpu::BindGroupLayout,
    scene: Option<AccelerationScene>,
    stats: HardwareRtStats,
}
impl HardwareRt {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        ensure!(
            gpu.device
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
            "hardware RT requires an explicitly enabled native ray-query device"
        );
        let limits = gpu.device.limits();
        ensure!(
            limits.max_blas_primitive_count > 0
                && limits.max_blas_geometry_count > 0
                && limits.max_tlas_instance_count > 0
                && limits.max_acceleration_structures_per_shader_stage > 0,
            "hardware RT acceleration-structure limits were not negotiated"
        );
        validate_shader(include_str!("shaders/hardware_rt.wgsl"))?;
        validate_shader(include_str!("shaders/hardware_shadow.wgsl"))?;
        let d = &gpu.device;
        let primary_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hardware RT primary resources"),
            entries: &[
                uniform(0),
                acceleration(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                output(3),
            ],
        });
        let shadow_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hardware RT shadow resources"),
            entries: &[
                uniform(0),
                acceleration(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                output(4),
                sampled(5),
            ],
        });
        let primary = pipeline(
            d,
            &primary_layout,
            "hardware primary and shadow rays",
            include_str!("shaders/hardware_rt.wgsl"),
        );
        let shadow = pipeline(
            d,
            &shadow_layout,
            "hardware PBR sun visibility",
            include_str!("shaders/hardware_shadow.wgsl"),
        );
        let clear_shadow_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("empty RT visibility/AO resources"),
            entries: &[output(4), sampled(5)],
        });
        let clear_shadow = pipeline_entry(
            d,
            &clear_shadow_layout,
            "empty hardware visibility",
            include_str!("shaders/hardware_shadow.wgsl"),
            "clear",
        );
        Ok(Self {
            clear_shadow,
            clear_shadow_layout,
            primary,
            primary_layout,
            shadow,
            shadow_layout,
            scene: None,
            stats: HardwareRtStats::default(),
        })
    }
    pub fn stats(&self) -> HardwareRtStats {
        self.stats
    }
    /// Call after successful queue submission of this service's encoded work.
    /// Queue ordering makes subsequent queries safe without a CPU/GPU wait.
    /// A dropped/error encoder leaves `submitted=false` and the next encode
    /// rebuilds instead of incorrectly assuming that the GPU saw the build.
    pub fn mark_submitted(&mut self) {
        if let Some(scene) = &mut self.scene {
            scene.submitted = true;
        }
    }
    fn prepare(
        &mut self,
        gpu: &Gpu,
        triangles: &[RtTriangle],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<()> {
        let positions = checked_positions(triangles, &gpu.device.limits())?;
        self.stats.triangles = triangles.len();
        if positions.is_empty() {
            self.scene = None;
            return Ok(());
        }
        let hash = geometry_hash(&positions);
        let unchanged = self
            .scene
            .as_ref()
            .is_some_and(|s| s.hash == hash && s.positions == positions);
        if !unchanged {
            let vertices = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("hardware RT world-space vertices"),
                    contents: bytemuck::cast_slice(&positions),
                    usage: wgpu::BufferUsages::BLAS_INPUT,
                });
            let size = geometry_size(positions.len())?;
            let blas = gpu.device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("hardware RT cached world BLAS"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size],
                },
            );
            let mut tlas = gpu.device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("hardware RT world TLAS"),
                max_instances: 1,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            });
            tlas[0] = Some(wgpu::TlasInstance::new(&blas, IDENTITY, 0, 0xff));
            self.scene = Some(AccelerationScene {
                hash,
                positions,
                _vertices: vertices,
                blas,
                tlas,
                submitted: false,
            });
        }
        let scene = self.scene.as_ref().context("hardware scene absent")?;
        if !scene.submitted {
            let size = geometry_size(scene.positions.len())?;
            let entry = wgpu::BlasBuildEntry {
                blas: &scene.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size: &size,
                        vertex_buffer: &scene._vertices,
                        first_vertex: 0,
                        vertex_stride: 12,
                        index_buffer: None,
                        first_index: None,
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            };
            encoder.build_acceleration_structures([&entry], [&scene.tlas]);
            self.stats.builds_encoded += 1;
        }
        Ok(())
    }
    pub fn encode_primary(
        &mut self,
        gpu: &Gpu,
        triangles: &[RtTriangle],
        inverse_view_projection: [[f32; 4]; 4],
        canvas: [u32; 2],
        background: [f32; 4],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture> {
        check_camera(inverse_view_projection)?;
        ensure!(
            background
                .iter()
                .all(|v| v.is_finite() && *v >= 0. && *v <= 65504.),
            "hardware RT background must be finite nonnegative half-float color"
        );
        let texture = target(gpu, canvas, "hardware RT primary color")?;
        self.prepare(gpu, triangles, encoder)?;
        if triangles.is_empty() {
            clear(encoder, &texture, background);
            return Ok(texture);
        }
        let metadata: Vec<Metadata> = triangles
            .iter()
            .map(|t| {
                let n = glam::Vec3::from_array(t.normal);
                let n = if n.length_squared() > 1e-20 {
                    n.normalize()
                } else {
                    let face = (glam::Vec3::from_array(t.b) - glam::Vec3::from_array(t.a))
                        .cross(glam::Vec3::from_array(t.c) - glam::Vec3::from_array(t.a));
                    face.try_normalize().unwrap_or(glam::Vec3::Y)
                };
                Metadata {
                    normal: [n.x, n.y, n.z, 0.],
                    color: t.color,
                }
            })
            .collect();
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hardware RT camera"),
                contents: bytemuck::bytes_of(&PrimaryParams {
                    inverse_view_projection,
                    background,
                    size_count: [canvas[0], canvas[1], triangles.len() as u32, 0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // Per-dispatch immutable metadata avoids queue.write_buffer races when
        // callers encode multiple views/material revisions before submitting.
        let metadata = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hardware RT surface metadata"),
                contents: bytemuck::cast_slice(&metadata),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let view = texture.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hardware RT primary bind"),
            layout: &self.primary_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene.as_ref().unwrap().tlas.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: metadata.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        dispatch(encoder, &self.primary, &bind, canvas);
        self.stats.primary_dispatches += 1;
        Ok(texture)
    }
    /// Direction is TOWARDS the sun. Prepass normals are signed view-space xyz.
    /// Output packs existing AO in R and hardware directional visibility in G,
    /// with B=0/A=1. Sharing the AO slot preserves downlevel texture limits.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn encode_sun_shadows(
        &mut self,
        gpu: &Gpu,
        triangles: &[RtTriangle],
        inverse_view_projection: [[f32; 4]; 4],
        inverse_view: [[f32; 4]; 4],
        sun_direction: [f32; 3],
        canvas: [u32; 2],
        depth: &wgpu::TextureView,
        normals: &wgpu::TextureView,
        ao: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture> {
        let texture = target(gpu, canvas, "hardware RT combined AO/sun visibility")?;
        let view = texture.create_view(&Default::default());
        self.encode_sun_shadows_into(
            gpu,
            triangles,
            inverse_view_projection,
            inverse_view,
            sun_direction,
            canvas,
            depth,
            normals,
            ao,
            &view,
            encoder,
        )?;
        Ok(texture)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn encode_sun_shadows_into(
        &mut self,
        gpu: &Gpu,
        triangles: &[RtTriangle],
        inverse_view_projection: [[f32; 4]; 4],
        inverse_view: [[f32; 4]; 4],
        sun_direction: [f32; 3],
        canvas: [u32; 2],
        depth: &wgpu::TextureView,
        normals: &wgpu::TextureView,
        ao: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<()> {
        check_canvas(gpu, canvas)?;
        let output_texture = output_view.texture();
        ensure!(
            output_texture.format() == wgpu::TextureFormat::Rgba16Float
                && output_texture.width() >= canvas[0]
                && output_texture.height() >= canvas[1]
                && output_texture
                    .usage()
                    .contains(wgpu::TextureUsages::STORAGE_BINDING),
            "hardware shadow output requires an RGBA16F storage view covering the canvas"
        );
        check_camera(inverse_view_projection)?;
        check_matrix(inverse_view)?;
        let light = glam::Vec3::from_array(sun_direction);
        ensure!(
            light.is_finite()
                && light.length_squared().is_finite()
                && light.length_squared() > 1e-12,
            "hardware shadow sun direction must be finite and nonzero"
        );
        let light = light.normalize();
        self.prepare(gpu, triangles, encoder)?;
        if triangles.is_empty() {
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("empty hardware shadow bind"),
                layout: &self.clear_shadow_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(output_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(ao),
                    },
                ],
            });
            dispatch(encoder, &self.clear_shadow, &bind, canvas);
            return Ok(());
        }
        let params = ShadowParams {
            inverse_view_projection,
            inverse_view,
            towards_sun: [light.x, light.y, light.z, 0.],
            size: [canvas[0], canvas[1], 0, 0],
        };
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hardware shadow camera/light"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hardware RT shadow bind"),
            layout: &self.shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene.as_ref().unwrap().tlas.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(normals),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(ao),
                },
            ],
        });
        dispatch(encoder, &self.shadow, &bind, canvas);
        self.stats.shadow_dispatches += 1;
        Ok(())
    }
}
fn check_matrix(matrix: [[f32; 4]; 4]) -> Result<()> {
    ensure!(
        matrix.iter().flatten().all(|v| v.is_finite()),
        "hardware RT transform contains nonfinite values"
    );
    Ok(())
}
fn check_camera(inverse: [[f32; 4]; 4]) -> Result<()> {
    check_matrix(inverse)?;
    let matrix = glam::Mat4::from_cols_array_2d(&inverse);
    ensure!(
        matrix.determinant().is_finite() && matrix.determinant().abs() > f32::MIN_POSITIVE,
        "hardware RT inverse projection must be nonsingular"
    );
    for x in [-1., 1.] {
        for y in [-1., 1.] {
            let near = matrix * glam::Vec4::new(x, y, 0., 1.);
            let far = matrix * glam::Vec4::new(x, y, 1., 1.);
            ensure!(
                near.is_finite()
                    && far.is_finite()
                    && near.w.abs() > 1e-12
                    && far.w.abs() > 1e-12
                    && near.w.signum() == far.w.signum(),
                "hardware RT camera crosses an infinite or invalid homogeneous plane"
            );
            let ray = far.truncate() / far.w - near.truncate() / near.w;
            ensure!(
                ray.is_finite() && ray.length_squared().is_finite() && ray.length_squared() > 1e-8,
                "hardware RT camera produces an invalid ray range"
            );
        }
    }
    Ok(())
}
fn geometry_size(vertices: usize) -> Result<wgpu::BlasTriangleGeometrySizeDescriptor> {
    Ok(wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: u32::try_from(vertices)?,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    })
}
fn checked_positions(triangles: &[RtTriangle], limits: &wgpu::Limits) -> Result<Vec<[f32; 3]>> {
    ensure!(
        triangles.len() <= MAX_TRIANGLES
            && triangles.len() as u64 <= u64::from(limits.max_blas_primitive_count),
        "hardware RT triangle limit exceeded"
    );
    let vertex_bytes = (triangles.len() as u64)
        .checked_mul(36)
        .context("hardware RT vertex size overflow")?;
    let metadata_bytes = (triangles.len() as u64)
        .checked_mul(32)
        .context("hardware RT metadata size overflow")?;
    ensure!(
        vertex_bytes + metadata_bytes <= MAX_INPUT_BYTES
            && vertex_bytes <= limits.max_buffer_size
            && metadata_bytes <= u64::from(limits.max_storage_buffer_binding_size),
        "hardware RT input buffer budget exceeded"
    );
    let mut positions = Vec::with_capacity(triangles.len() * 3);
    for t in triangles {
        ensure!(
            t.a.iter()
                .chain(&t.b)
                .chain(&t.c)
                .all(|v| v.is_finite() && v.abs() <= 1e8),
            "hardware RT positions must be finite and within world bounds"
        );
        ensure!(
            t.normal.iter().all(|v| v.is_finite() && v.abs() <= 1e8),
            "hardware RT normal is invalid"
        );
        ensure!(
            t.color
                .iter()
                .all(|v| v.is_finite() && *v >= 0. && *v <= 65504.)
                && (t.color[3] - 1.).abs() < 1e-6,
            "hardware RT currently accepts opaque finite colors only"
        );
        positions.extend([t.a, t.b, t.c]);
    }
    Ok(positions)
}
fn geometry_hash(positions: &[[f32; 3]]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytemuck::cast_slice::<[f32; 3], u8>(positions).hash(&mut h);
    h.finish()
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
fn acceleration(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure {
            vertex_return: false,
        },
        count: None,
    }
}
fn output(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}
fn sampled(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    source: &str,
) -> wgpu::ComputePipeline {
    pipeline_entry(device, layout, label, source, "main")
}
fn pipeline_entry(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    source: &str,
    entry: &str,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}
fn check_canvas(gpu: &Gpu, canvas: [u32; 2]) -> Result<()> {
    let pixels = u64::from(canvas[0]) * u64::from(canvas[1]);
    ensure!(
        canvas
            .iter()
            .all(|v| *v > 0 && *v <= gpu.device.limits().max_texture_dimension_2d)
            && pixels <= MAX_OUTPUT_PIXELS,
        "hardware RT output dimension/pixel budget exceeded"
    );
    Ok(())
}
fn target(gpu: &Gpu, canvas: [u32; 2], label: &str) -> Result<wgpu::Texture> {
    check_canvas(gpu, canvas)?;
    Ok(gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: canvas[0],
            height: canvas[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }))
}
fn clear(encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, color: [f32; 4]) {
    let view = texture.create_view(&Default::default());
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("empty hardware scene"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: f64::from(color[0]),
                    g: f64::from(color[1]),
                    b: f64::from(color[2]),
                    a: f64::from(color[3]),
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}
fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    bind: &wgpu::BindGroup,
    canvas: [u32; 2],
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("native hardware ray queries"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
}
fn validate_shader(source: &str) -> Result<()> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| anyhow::anyhow!("{}", e.emit_to_string(source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| anyhow::anyhow!("hardware ray query shader validation: {e:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle() -> RtTriangle {
        RtTriangle {
            a: [0., 0., 0.],
            b: [1., 0., 0.],
            c: [0., 1., 0.],
            normal: [0., 0., 1.],
            color: [1.; 4],
        }
    }
    fn limits() -> wgpu::Limits {
        wgpu::Limits::default().using_minimum_supported_acceleration_structure_values()
    }
    #[test]
    fn genuine_ray_query_shaders_parse_and_validate() {
        for shader in [
            include_str!("shaders/hardware_rt.wgsl"),
            include_str!("shaders/hardware_shadow.wgsl"),
        ] {
            validate_shader(shader).unwrap();
            assert!(shader.contains("rayQueryInitialize"));
            assert!(!shader.contains("distance_to_triangle"));
        }
    }
    #[test]
    fn positions_cache_identity_ignores_material_metadata_but_detects_moves() {
        let a = triangle();
        let mut b = a.clone();
        b.color = [0., 1., 0., 1.];
        let pa = checked_positions(&[a], &limits()).unwrap();
        let pb = checked_positions(&[b.clone()], &limits()).unwrap();
        assert_eq!(pa, pb);
        assert_eq!(geometry_hash(&pa), geometry_hash(&pb));
        b.a[0] = 0.25;
        assert_ne!(pa, checked_positions(&[b], &limits()).unwrap());
    }
    #[test]
    fn singular_and_infinite_camera_rays_fail_before_encoding() {
        assert!(check_camera([[0.; 4]; 4]).is_err());
        let projection = glam::Mat4::perspective_rh(50f32.to_radians(), 1., 0.05, 100.);
        check_camera(projection.inverse().to_cols_array_2d()).unwrap();
        let infinite = glam::Mat4::perspective_infinite_rh(50f32.to_radians(), 1., 0.05);
        assert!(check_camera(infinite.inverse().to_cols_array_2d()).is_err());
    }
    #[test]
    fn invalid_geometry_and_limits_fail_before_gpu_work() {
        let mut bad = triangle();
        bad.a[0] = f32::NAN;
        assert!(checked_positions(&[bad], &limits()).is_err());
        let mut alpha = triangle();
        alpha.color[3] = 0.5;
        assert!(checked_positions(&[alpha], &limits()).is_err());
        let mut limits = limits();
        limits.max_blas_primitive_count = 0;
        assert!(checked_positions(&[triangle()], &limits).is_err());
        assert!(checked_positions(&[], &limits).unwrap().is_empty());
    }
    fn read_output(
        gpu: &Gpu,
        texture: &wgpu::Texture,
        mut encoder: wgpu::CommandEncoder,
        rt: &mut HardwareRt,
    ) -> Vec<[f32; 4]> {
        let width = texture.width();
        let height = texture.height();
        let stride = (width * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hardware RT test readback"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
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
        let submitted = gpu.queue.submit([encoder.finish()]);
        rt.mark_submitted();
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(std::time::Duration::from_secs(20)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let mapped = slice.get_mapped_range();
        let mut out = Vec::with_capacity((width * height) as usize);
        for row in mapped.chunks_exact(stride as usize) {
            for pixel in row[..width as usize * 8].chunks_exact(8) {
                out.push(std::array::from_fn(|i| {
                    half::f16::from_bits(u16::from_le_bytes([pixel[2 * i], pixel[2 * i + 1]]))
                        .to_f32()
                }));
            }
        }
        drop(mapped);
        buffer.unmap();
        out
    }
    #[test]
    #[ignore = "requires actual hardware ray-query device; explicitly execute"]
    fn hardware_cache_metadata_moves_empty_and_abandoned_encoder_reach_pixels() {
        let gpu = pollster::block_on(Gpu::request_for(
            &wgpu::Instance::default(),
            None,
            crate::RenderingPath::HardwareRaytrace,
        ))
        .unwrap();
        eprintln!("hardware backend GPU {:?}", gpu.adapter.get_info());
        let mut rt = HardwareRt::new(&gpu).unwrap();
        let camera = glam::Mat4::perspective_rh(50f32.to_radians(), 1., 0.1, 10.)
            * glam::Mat4::look_at_rh(glam::Vec3::new(0., 0., 2.), glam::Vec3::ZERO, glam::Vec3::Y);
        let inverse = camera.inverse().to_cols_array_2d();
        let mut triangle = RtTriangle {
            a: [-0.8, -0.8, 0.],
            b: [0.8, -0.8, 0.],
            c: [0., 0.8, 0.],
            normal: [0., 0., 1.],
            color: [1., 0., 0., 1.],
        };
        let mut abandoned = gpu.device.create_command_encoder(&Default::default());
        rt.encode_primary(
            &gpu,
            &[triangle.clone()],
            inverse,
            [32, 32],
            [0.01, 0.02, 0.03, 1.],
            &mut abandoned,
        )
        .unwrap();
        drop(abandoned);
        assert_eq!(rt.stats().builds_encoded, 1);
        let shot = |rt: &mut HardwareRt, triangles: &[RtTriangle]| {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let texture = rt
                .encode_primary(
                    &gpu,
                    triangles,
                    inverse,
                    [32, 32],
                    [0.01, 0.02, 0.03, 1.],
                    &mut encoder,
                )
                .unwrap();
            read_output(&gpu, &texture, encoder, rt)[16 * 32 + 16]
        };
        let red = shot(&mut rt, &[triangle.clone()]);
        assert!(red[0] > 0.3 && red[1] < 0.01, "red hardware hit {red:?}");
        assert_eq!(rt.stats().builds_encoded, 2);
        triangle.color = [0., 1., 0., 1.];
        let green = shot(&mut rt, &[triangle.clone()]);
        assert!(green[1] > 0.3 && green[0] < 0.01);
        assert_eq!(rt.stats().builds_encoded, 2, "metadata must not rebuild AS");
        for position in [&mut triangle.a, &mut triangle.b, &mut triangle.c] {
            position[0] += 5.;
        }
        let moved = shot(&mut rt, &[triangle]);
        assert!((moved[2] - 0.03).abs() < 0.001);
        assert_eq!(rt.stats().builds_encoded, 3);
        let empty = shot(&mut rt, &[]);
        assert!((empty[2] - 0.03).abs() < 0.001);
        assert_eq!(rt.stats().triangles, 0);
    }
    #[test]
    #[ignore = "requires actual hardware ray-query device; explicitly execute"]
    fn hardware_sun_visibility_preserves_ao_and_empty_casters_clear_only_visibility() {
        let gpu = pollster::block_on(Gpu::request_for(
            &wgpu::Instance::default(),
            None,
            crate::RenderingPath::HardwareRaytrace,
        ))
        .unwrap();
        let mut rt = HardwareRt::new(&gpu).unwrap();
        let size = 16;
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test prepass depth"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let normals = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test prepass normals"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let ao = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test AO"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let normal_data: Vec<u8> = (0..size * size)
            .flat_map(|_| {
                [0f32, 0., 1., 1.]
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            })
            .collect();
        for (texture, data, bpp) in [
            (&normals, normal_data, 8),
            (
                &ao,
                (0..size * size)
                    .flat_map(|_| [64u8, 0, 0, 255])
                    .collect::<Vec<_>>(),
                4,
            ),
        ] {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * bpp),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view =
            glam::Mat4::look_at_rh(glam::Vec3::new(0., 0., 2.), glam::Vec3::ZERO, glam::Vec3::Y);
        let camera = glam::Mat4::perspective_rh(50f32.to_radians(), 1., 0.1, 10.) * view;
        let plane = camera * glam::Vec4::new(0., 0., 0., 1.);
        let plane_depth = plane.z / plane.w;
        let depth_view = depth.create_view(&Default::default());
        let normals_view = normals.create_view(&Default::default());
        let ao_view = ao.create_view(&Default::default());
        let shot = |rt: &mut HardwareRt, triangles: &[RtTriangle]| {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("test plane depth"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(plane_depth),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            let texture = rt
                .encode_sun_shadows(
                    &gpu,
                    triangles,
                    camera.inverse().to_cols_array_2d(),
                    view.inverse().to_cols_array_2d(),
                    [0., 0., 1.],
                    [size, size],
                    &depth_view,
                    &normals_view,
                    &ao_view,
                    &mut encoder,
                )
                .unwrap();
            read_output(&gpu, &texture, encoder, rt)[(size * size / 2 + size / 2) as usize]
        };
        let caster = RtTriangle {
            a: [-0.8, -0.8, 1.],
            b: [0.8, -0.8, 1.],
            c: [0., 0.8, 1.],
            normal: [0., 0., 1.],
            color: [1.; 4],
        };
        let baseline = shot(&mut rt, &[]);
        assert!((baseline[1] - 1.).abs() < 0.001);
        eprintln!("hardware shadow empty-caster compute completed");
        let mut build = gpu.device.create_command_encoder(&Default::default());
        rt.prepare(&gpu, &[caster.clone()], &mut build).unwrap();
        let build_output = target(&gpu, [size, size], "AS build probe").unwrap();
        clear(&mut build, &build_output, [1.; 4]);
        let built = read_output(&gpu, &build_output, build, &mut rt);
        assert!((built[0][0] - 1.).abs() < 0.001);
        eprintln!("hardware native BLAS/TLAS build completed");
        let shadow = shot(&mut rt, &[caster]);
        assert!(
            (shadow[0] - 64. / 255.).abs() < 0.001 && shadow[1] < 0.01,
            "packed AO/shadow {shadow:?}"
        );
        let empty = shot(&mut rt, &[]);
        assert!(
            (empty[0] - 64. / 255.).abs() < 0.001 && (empty[1] - 1.).abs() < 0.001,
            "empty scene retains AO {empty:?}"
        );
    }
}
