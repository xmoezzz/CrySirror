// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! GPU-local box filtering of isolated transition inputs and grayscale rules.
use super::*;
use wgpu::util::DeviceExt;
pub(super) struct Filter {
    horizontal: wgpu::ComputePipeline,
    vertical: wgpu::ComputePipeline,
    h_layout: wgpu::BindGroupLayout,
    v_layout: wgpu::BindGroupLayout,
}
pub(super) struct Product {
    pub view: wgpu::TextureView,
    _texture: wgpu::Texture,
}
pub(super) fn target(
    gpu: &Gpu,
    canvas: [u32; 2],
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: canvas[0],
            height: canvas[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    })
}
impl Filter {
    pub fn new(gpu: &Gpu) -> Self {
        let make = |format: wgpu::TextureFormat, name: &str| {
            let layout = gpu
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("transition filter"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
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
                                format,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                    ],
                });
            let source =
                include_str!("shaders/composite_filter.wgsl").replace("__OUTPUT_FORMAT__", name);
            let shader = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("transition box filter"),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            let pipeline_layout =
                gpu.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("transition filter"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipeline = gpu
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("transition box filter"),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some("cs_main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            (pipeline, layout)
        };
        let (horizontal, h_layout) = make(wgpu::TextureFormat::Rgba32Float, "rgba32float");
        let (vertical, v_layout) = make(wgpu::TextureFormat::Rgba16Float, "rgba16float");
        Self {
            horizontal,
            vertical,
            h_layout,
            v_layout,
        }
    }
    fn pass(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
        canvas: [u32; 2],
        rect: [f32; 4],
        radius: [u32; 2],
        axis: u32,
        mode: u32,
        negative: bool,
    ) {
        let values = [
            axis,
            radius[axis as usize],
            radius[0],
            mode,
            rect[0].to_bits(),
            rect[1].to_bits(),
            rect[2].to_bits(),
            rect[3].to_bits(),
            u32::from(negative),
            0,
            0,
            0,
        ];
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("transition filter parameters"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = if axis == 0 {
            &self.h_layout
        } else {
            &self.v_layout
        };
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition filter"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(output),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("transition separable box"),
            timestamp_writes: None,
        });
        pass.set_pipeline(if axis == 0 {
            &self.horizontal
        } else {
            &self.vertical
        });
        pass.set_bind_group(0, &bind, &[]);
        if axis == 0 {
            pass.dispatch_workgroups(canvas[0].div_ceil(64), canvas[1], 1);
        } else {
            pass.dispatch_workgroups(canvas[0], canvas[1].div_ceil(64), 1);
        }
    }
    pub fn process(
        &self,
        gpu: &Gpu,
        inputs: &[(&wgpu::TextureView, [u32; 2])],
        canvas: [u32; 2],
        rect: [f32; 4],
        rule: bool,
        negative: bool,
    ) -> Vec<Product> {
        let temporary = target(
            gpu,
            canvas,
            wgpu::TextureFormat::Rgba32Float,
            "transition horizontal sums",
        );
        let temporary_view = temporary.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut products = Vec::new();
        for (input, radius) in inputs {
            let texture = target(
                gpu,
                canvas,
                wgpu::TextureFormat::Rgba16Float,
                "transition filtered pixels",
            );
            let view = texture.create_view(&Default::default());
            self.pass(
                gpu,
                &mut encoder,
                input,
                &temporary_view,
                canvas,
                rect,
                *radius,
                0,
                if rule { 2 } else { 1 },
                false,
            );
            self.pass(
                gpu,
                &mut encoder,
                &temporary_view,
                &view,
                canvas,
                rect,
                *radius,
                1,
                0,
                negative,
            );
            products.push(Product {
                view,
                _texture: texture,
            });
        }
        gpu.queue.submit(Some(encoder.finish()));
        products
    }
}
