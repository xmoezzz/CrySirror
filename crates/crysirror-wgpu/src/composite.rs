// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Isolated engine subtree rendering. Uses the same renderer for both inputs;
//! child opacity is applied before the crossfade, never independently faded.
use super::*;
use wgpu::util::DeviceExt;
pub(super) struct Compositor {
    renderer: Box<Renderer>,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}
impl Compositor {
    fn new(gpu: Arc<Gpu>, format: wgpu::TextureFormat) -> Self {
        let d = &gpu.device;
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("subtree crossfade"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Rule image of a universal wipe (1x1 white when unused).
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
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("isolated crossfade"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("subtree crossfade"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subtree crossfade"),
            layout: Some(&pl),
            cache: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
        });
        Self {
            renderer: Box::new(Renderer::new(gpu.clone(), format)),
            layout,
            pipeline,
        }
    }
}
impl Renderer {
    pub(super) fn prepare_composites(
        &mut self,
        assets: &mut Assets,
        canvas: [u32; 2],
    ) -> Result<()> {
        let composites: Vec<_> = self
            .engine_frame
            .iter()
            .filter_map(|p| p.composite.as_ref().map(|f| (p.owner, p.id, f.clone())))
            .collect();
        if composites.is_empty() {
            if !self
                .engine_frame
                .iter()
                .any(|p| matches!(p.paint, Paint::Snapshot { .. }))
            {
                self.compositor = None;
            }
            return Ok(());
        }
        ensure!(composites.len() <= 8, "subtree composition limit");
        let bytes_per_pixel = self
            .format
            .block_copy_size(None)
            .context("unsupported composite format")? as u64;
        let pixel_count = u64::from(canvas[0]) * u64::from(canvas[1]);
        ensure!(
            canvas
                .iter()
                .all(|n| *n > 0 && *n <= self.gpu.device.limits().max_texture_dimension_2d),
            "composite canvas exceeds device dimensions"
        );
        // All retained products, two isolated inputs and the child renderer's
        // depth target share one explicit working-set budget.
        let work_bytes = pixel_count * (bytes_per_pixel * (composites.len() as u64 + 2) + 4);
        ensure!(
            work_bytes <= 128 * 1024 * 1024,
            "composite working set exceeds 128 MiB"
        );
        let mut c = self
            .compositor
            .take()
            .unwrap_or_else(|| Compositor::new(self.gpu.clone(), self.format));
        let result = (|| {
            let mut products = BTreeMap::new();
            for (owner, id, frame) in composites {
                ensure!(
                    frame.progress.is_finite() && (0.0..=1.0).contains(&frame.progress),
                    "invalid composite progress"
                );
                ensure!(
                    frame
                        .before
                        .iter()
                        .chain(&frame.after)
                        .all(|p| p.composite.is_none() && p.owner == owner),
                    "nested or foreign-client compositor input"
                );
                // The rule image is an ordinary immutable image texture.
                // Until an asynchronously decoded rule is ready the wipe has
                // not visibly started: show the before input.
                let mut rule_ready = false;
                let mut rule_key = "solid:white".to_string();
                if let Some((r, _)) = &frame.rule {
                    let key = Self::image_key(assets, owner, &r.asset)?;
                    if self.ensure_image(assets, owner, &r.asset, &key)? {
                        rule_key = key;
                        rule_ready = true;
                    }
                }
                if rule_key == "solid:white" && !self.textures.contains_key("solid:white") {
                    self.upload("solid:white".into(), 1, 1, &[255; 4])?;
                }
                let asset = format!("__crysirror_composite/{id}");
                ensure!(
                    assets.surface(ClientId(owner), &asset).is_none(),
                    "composite name collides with a dynamic surface"
                );
                // Match prepare's ordinary Paint::Image cache namespace.
                let key = format!("image:{}", assets.key(ClientId(owner), &asset)?);
                ensure!(!products.contains_key(&key), "duplicate composite product");
                let cached: u64 = self.textures.values().map(|t| t.bytes).sum();
                let child_cached: u64 = c.renderer.textures.values().map(|t| t.bytes).sum();
                ensure!(
                    cached + child_cached + work_bytes <= 256 * 1024 * 1024,
                    "composite and texture caches exceed 256 MiB"
                );
                let texture = |label| {
                    self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: canvas[0],
                            height: canvas[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                };
                let a = texture("subtree before");
                let b = texture("subtree after");
                let output = texture("subtree crossfade product");
                let av = a.create_view(&Default::default());
                let bv = b.create_view(&Default::default());
                let view = output.create_view(&Default::default());
                for (primitives, target) in [(&frame.before, &av), (&frame.after, &bv)] {
                    c.renderer.set_engine_frame(primitives.clone())?;
                    c.renderer.render(
                        &SceneState::default(),
                        assets,
                        RenderTarget {
                            view: target,
                            size: canvas,
                        },
                        canvas,
                        [0; 4],
                    )?;
                }
                let uniform =
                    self.gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("crossfade progress"),
                            contents: bytemuck::cast_slice(&{
                                let (vague, on, rect) = match &frame.rule {
                                    Some((r, rect)) if rule_ready => (r.vague as f32, 1., *rect),
                                    _ => (0., 0., [0., 0., 1., 1.]),
                                };
                                let pending = frame.rule.is_some() && !rule_ready;
                                [
                                    if pending { 0. } else { frame.progress.clamp(0., 1.) },
                                    vague,
                                    on,
                                    0.,
                                    rect[0],
                                    rect[1],
                                    rect[2].max(1.),
                                    rect[3].max(1.),
                                ]
                            }),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let bind = self
                    .gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("crossfade inputs"),
                        layout: &c.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&av),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&bv),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(
                                    &self.textures[&rule_key].view,
                                ),
                            },
                        ],
                    });
                let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("isolated crossfade"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&c.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                self.gpu.queue.submit(Some(encoder.finish()));
                ensure!(
                    self.textures.values().map(|t| t.bytes).sum::<u64>()
                        + c.renderer.textures.values().map(|t| t.bytes).sum::<u64>()
                        + work_bytes
                        <= 256 * 1024 * 1024,
                    "composite and texture caches exceed 256 MiB"
                );
                let bind = self
                    .gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("composite layer"),
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
                products.insert(
                    key,
                    Texture {
                        bind,
                        view,
                        _texture: output,
                        bytes: pixel_count * bytes_per_pixel,
                    },
                );
            }
            self.textures
                .retain(|key, _| !key.contains(":__crysirror_composite/"));
            self.textures.extend(products);
            Ok(())
        })();
        // Failed preparation must not strand newly loaded child textures and
        // make every later frame fail the same budget check.
        self.compositor = if result.is_ok() { Some(c) } else { None };
        result
    }
}

impl Renderer {
    pub(super) fn snapshot_key(
        assets: &Assets,
        owner: u32,
        size: [u32; 2],
        layers: &[crysirror_scene::layer::LayerSpec],
    ) -> Result<String> {
        let definition = serde_json::to_vec(&(size, layers))?;
        ensure!(
            definition.len() <= 4 * 1024 * 1024,
            "snapshot definition exceeds 4 MiB"
        );
        Ok(format!(
            "snapshot:{}:{owner}:{}",
            assets.store_id(),
            blake3::hash(&definition)
        ))
    }

    /// Frozen engine resources only. A namespace-shaped string is not proof:
    /// generated_for must find the bytes in the owner's immutable resource map.
    fn snapshot_dependencies(
        assets: &mut Assets,
        owner: u32,
        layers: &[crysirror_scene::layer::LayerSpec],
    ) -> Result<Option<Vec<crysirror_runtime::assets::AssetLease>>> {
        let mut names = BTreeSet::new();
        for layer in layers {
            match &layer.paint {
                Paint::Solid { .. } => {}
                Paint::Image { asset, .. } => {
                    names.insert(asset.as_str());
                }
                Paint::Text { .. } | Paint::GlyphRun { .. } => {
                    anyhow::bail!("snapshot font requires an immutable captured font handle")
                }
                Paint::Puppet { .. } | Paint::Snapshot { .. } => {
                    anyhow::bail!("dynamic or nested snapshot dependency")
                }
            }
            if let Some(filter) = &layer.filter {
                if let Some(mask) = &filter.mask {
                    names.insert(mask.asset.as_str());
                }
                if let Some(curve) = &filter.tone_curve {
                    names.insert(curve.asset.as_str());
                }
            }
        }
        let mut ready = true;
        let mut pins = Vec::new();
        for name in names {
            let bytes = assets
                .generated_for(ClientId(owner), name)?
                .with_context(|| {
                    format!("snapshot requires a published immutable engine image: {name}")
                })?;
            // Native immutable image publication uses self-contained PNG. Other
            // codecs can read mutable companion files even when their main
            // bytes are immutable; those need a captured dependency graph.
            ensure!(
                bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "snapshot immutable image profile requires self-contained PNG: {name}"
            );
            // Preflight every dependency before rasterization. In nonblocking
            // mode None means pending, never a successful blank snapshot.
            pins.push(assets.lease(
                ClientId(owner),
                name,
                crysirror_runtime::assets::AssetKind::Image,
            )?);
            ready &= assets.image_for(ClientId(owner), name)?.is_some();
        }
        Ok(ready.then_some(pins))
    }

    pub(super) fn prepare_snapshots(&mut self, assets: &mut Assets) -> Result<()> {
        let snapshots: Vec<_> = self
            .engine_frame
            .iter()
            .filter_map(|p| match &p.paint {
                Paint::Snapshot { size, layers } => Some((p.owner, *size, layers.clone())),
                _ => None,
            })
            .collect();
        if snapshots.is_empty() {
            return Ok(());
        }
        ensure!(snapshots.len() <= 64, "snapshot frame count limit");
        ensure!(
            snapshots
                .iter()
                .map(|(_, _, layers)| layers.len())
                .sum::<usize>()
                <= 32768,
            "snapshot frame node budget"
        );
        let pixel_bytes = self
            .format
            .block_copy_size(None)
            .context("snapshot color format")? as u64;
        let mut c = self
            .compositor
            .take()
            .unwrap_or_else(|| Compositor::new(self.gpu.clone(), self.format));
        let result = (|| {
            for (owner, size, layers) in snapshots {
                let paint = Paint::Snapshot {
                    size,
                    layers: layers.clone(),
                };
                paint.validate()?;
                let key = Self::snapshot_key(assets, owner, size, &layers)?;
                if self.textures.contains_key(&key) {
                    continue;
                }
                ensure!(
                    size.iter()
                        .all(|n| *n > 0 && *n <= self.gpu.device.limits().max_texture_dimension_2d),
                    "snapshot size exceeds device limits"
                );
                let pixels = u64::from(size[0]) * u64::from(size[1]);
                let work_bytes = pixels * (pixel_bytes * 2 + 4);
                ensure!(
                    work_bytes <= 128 * 1024 * 1024,
                    "snapshot working set exceeds 128 MiB"
                );
                ensure!(
                    self.textures.values().map(|t| t.bytes).sum::<u64>()
                        + c.renderer.textures.values().map(|t| t.bytes).sum::<u64>()
                        + work_bytes
                        <= 256 * 1024 * 1024,
                    "snapshot and texture caches exceed 256 MiB"
                );
                let Some(_pins) = Self::snapshot_dependencies(assets, owner, &layers)? else {
                    continue;
                };
                let mut isolated = crysirror_scene::layer::LayerSystem::default();
                isolated.install(ClientId(owner), &layers)?;
                c.renderer.set_engine_frame(isolated.frame(size)?)?;
                let make = |label| {
                    self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                };
                let premultiplied = make("immutable snapshot isolated input");
                let input = premultiplied.create_view(&Default::default());
                c.renderer.render(
                    &SceneState::default(),
                    assets,
                    RenderTarget { view: &input, size },
                    size,
                    [0; 4],
                )?;
                ensure!(
                    self.textures.values().map(|t| t.bytes).sum::<u64>()
                        + c.renderer.textures.values().map(|t| t.bytes).sum::<u64>()
                        + work_bytes
                        <= 256 * 1024 * 1024,
                    "snapshot and texture caches exceed 256 MiB"
                );
                let output = make("immutable snapshot straight alpha");
                let view = output.create_view(&Default::default());
                let uniform =
                    self.gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("snapshot alpha conversion"),
                            contents: bytemuck::cast_slice(&[0.0f32; 4]),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                // Reuse the tested isolated compositor's alpha conversion with
                // identical endpoints. There is no animation or screen capture.
                let bind = self
                    .gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("snapshot isolated input"),
                        layout: &c.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&input),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&input),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: uniform.as_entire_binding(),
                            },
                        ],
                    });
                let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("snapshot alpha conversion"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&c.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                self.gpu.queue.submit(Some(encoder.finish()));
                let bind = self
                    .gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("immutable snapshot texture"),
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
                        _texture: output,
                        bytes: pixels * pixel_bytes,
                    },
                );
                self.snapshot_rasterizations = self.snapshot_rasterizations.saturating_add(1);
            }
            Ok(())
        })();
        self.compositor = if result.is_ok() { Some(c) } else { None };
        result
    }
}
