// SPDX-License-Identifier: MPL-2.0
//! Bounded software ray/triangle traversal on GPU compute. No hardware RT claim.
use crate::{geometry, Gpu};
use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};
use crysirror_runtime::model::Scene3d;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Triangle {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    normal: [f32; 4],
    color: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    inverse_view_projection: [[f32; 4]; 4],
    background: [f32; 4],
    size_count: [u32; 4],
}
pub(crate) struct RayTracer {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}
impl RayTracer {
    pub fn new(gpu: &Gpu) -> Self {
        let d = &gpu.device;
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raytrace layout"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raytrace pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("software ray/triangle queries"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/raytrace.wgsl").into()),
        });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { cache: None,
            label: Some("compute raytrace"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });
        Self { pipeline, layout }
    }
    pub fn encode(
        &self,
        gpu: &Gpu,
        scene: &Scene3d,
        canvas: [u32; 2],
        background: [u8; 4],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<wgpu::Texture> {
        let pixels = u64::from(canvas[0]) * u64::from(canvas[1]);
        ensure!(
            canvas
                .iter()
                .all(|n| *n <= gpu.device.limits().max_texture_dimension_2d),
            "raytrace target exceeds device dimensions"
        );
        ensure!(
            pixels > 0 && pixels <= 1_048_576,
            "compute raytrace supports at most 1,048,576 pixels"
        );
        let vertices = geometry::triangles(scene)?;
        let count = vertices.len() / 3;
        ensure!(
            count <= 4096 && pixels * (count as u64) <= 256_000_000,
            "compute raytrace triangle/work budget exceeded"
        );
        let mut triangles = Vec::with_capacity(count.max(1));
        for v in vertices.as_chunks::<3>().0 {
            let p = |v: &geometry::Vertex3d| [v.position[0], v.position[1], v.position[2], 0.];
            triangles.push(Triangle {
                a: p(&v[0]),
                b: p(&v[1]),
                c: p(&v[2]),
                normal: [v[0].normal[0], v[0].normal[1], v[0].normal[2], 0.],
                color: v[0].color,
            });
        }
        if triangles.is_empty() {
            triangles.push(Triangle::zeroed());
        }
        let inverse = glam::Mat4::from_cols_array_2d(&geometry::camera(scene, canvas)).inverse();
        ensure!(
            inverse.is_finite(),
            "raytrace camera matrix is not invertible"
        );
        let far = inverse * glam::Vec4::new(0., 0., 1., 1.);
        ensure!(
            far.is_finite() && far.w.abs() > 1e-12,
            "raytrace camera clip ratio exceeds supported f32 precision"
        );
        let params = Params {
            inverse_view_projection: inverse.to_cols_array_2d(),
            background: geometry::tint(background),
            size_count: [canvas[0], canvas[1], count as u32, 0],
        };
        let d = &gpu.device;
        let uniform = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raytrace camera"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let triangles = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raytrace triangles"),
            contents: bytemuck::cast_slice(&triangles),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let texture = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("raytrace linear color"),
            size: wgpu::Extent3d {
                width: canvas[0],
                height: canvas[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raytrace resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: triangles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("primary and shadow rays"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(canvas[0].div_ceil(8), canvas[1].div_ceil(8), 1);
        }
        Ok(texture)
    }
}
