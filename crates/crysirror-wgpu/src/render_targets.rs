// SPDX-License-Identifier: MPL-2.0
//! Mutable GPU-only bitmap destinations. Each allocation is retained by the
//! shared frame's generation, including while its presentation is hidden.
use super::*;
use crysirror_scene::layer::render_target::RenderTargetBinding as Target;

impl Renderer {
    /// Prepare pending target writes using the normal GPU pipeline, without a
    /// CPU screenshot. Revision receipts suppress their later presentation
    /// replay; unrelated main-frame pixels are not part of a target readback.
    pub fn flush_target_writes(
        &mut self,
        _scene: &SceneState,
        assets: &mut Assets,
        canvas: [u32; 2],
        background: [u8; 4],
    ) -> Result<()> {
        ensure!(
            canvas
                .iter()
                .all(|v| *v > 0 && *v <= self.gpu.device.limits().max_texture_dimension_2d),
            "target preparation canvas"
        );
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target preparation frame"),
            size: wgpu::Extent3d {
                width: canvas[0],
                height: canvas[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let protected = self.frame_cache_keys(assets, &self.engine_frame)?;
        let retained = std::mem::take(&mut self.engine_frame);
        let previous_keys = self.capture_texture_keys.clone();
        let previous_scene_keys = self.scene_texture_keys.clone();
        self.capture_texture_keys.extend(protected);
        self.capture_texture_keys
            .extend(previous_scene_keys.iter().cloned());
        self.engine_frame = retained
            .iter()
            .filter(|p| p.output.is_some() || p.target.is_some())
            .cloned()
            .map(|mut p| {
                if p.output.is_none() {
                    p.opacity = 0;
                }
                p
            })
            .collect();
        let previous_flush = std::mem::replace(&mut self.flushing_target_writes, true);
        let result = self.render(
            &SceneState::default(),
            assets,
            RenderTarget {
                view: &view,
                size: canvas,
            },
            canvas,
            background,
        );
        self.flushing_target_writes = previous_flush;
        self.engine_frame = retained;
        self.capture_texture_keys = previous_keys;
        self.scene_texture_keys = previous_scene_keys;
        result
    }
    /// Queue a frozen GPU copy before a worker maps bytes. A pending source
    /// dependency retries, and no live attachment is ever handed to a worker.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_target_readback(
        &mut self,
        assets: &mut Assets,
        target: &Target,
    ) -> Result<Option<Arc<dyn crysirror_core::service::AssetReader>>> {
        let source = Self::target_key(assets, target);
        if self.pending_target_outputs.contains(&source) {
            return Ok(None);
        }
        let Some(texture) = self.textures.get(&source) else {
            return Ok(None);
        };
        let source = texture.texture.clone();
        ensure!(
            [source.width(), source.height()] == target.spec.size,
            "target readback dimensions changed"
        );
        self.target_readback_serial = self
            .target_readback_serial
            .checked_add(1)
            .context("target readback identity exhausted")?;
        let key = format!(
            "target-readback:{}:{}",
            assets.store_id(),
            self.target_readback_serial
        );
        self.target_texture(key.clone(), target.spec.size)?;
        let copy = self.textures[&key].texture.clone();
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("freeze target image"),
            });
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            copy.as_image_copy(),
            source.size(),
        );
        self.gpu.queue.submit(Some(encoder.finish()));
        let reader = self.texture_readback_reader(copy, target.spec.size);
        self.snapshot_readers
            .entry(key)
            .or_default()
            .push(Arc::downgrade(&reader));
        Ok(Some(reader))
    }
    #[cfg(target_arch = "wasm32")]
    pub fn render_target_readback(
        &mut self,
        _: &mut Assets,
        _: &Target,
    ) -> Result<Option<Arc<dyn crysirror_core::service::AssetReader>>> {
        anyhow::bail!("browser target readback requires a presenter-owned asynchronous transfer")
    }
    pub(super) fn target_key(assets: &Assets, target: &Target) -> String {
        format!(
            "target:{}:{}:{}:{}",
            assets.store_id(),
            target.owner,
            target.id,
            target.generation
        )
    }
    pub(super) fn target_texture(&mut self, key: String, size: [u32; 2]) -> Result<()> {
        if self.textures.contains_key(&key) {
            return Ok(());
        }
        ensure!(
            size.iter()
                .all(|v| *v > 0 && *v <= self.gpu.device.limits().max_texture_dimension_2d),
            "persistent GPU target dimensions"
        );
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        ensure!(
            self.textures.values().map(|t| t.bytes).sum::<u64>() + bytes <= 256 * 1024 * 1024,
            "persistent GPU target/cache byte budget"
        );
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("persistent painter destination"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("persistent destination sampling"),
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
                texture,
                view,
                bind,
                bytes,
            },
        );
        Ok(())
    }
    pub(super) fn prepare_render_targets(&mut self, assets: &mut Assets) -> Result<()> {
        let mut targets = BTreeMap::new();
        for p in &self.engine_frame {
            for target in p.target.iter().chain(p.output.iter()) {
                targets.insert(Self::target_key(assets, target), target.clone());
            }
        }
        // Retire superseded generations before reserving replacements. Queued
        // wgpu command buffers retain old textures through their submissions.
        self.retire_unused_textures(assets)?;
        for (key, target) in targets {
            if self.textures.contains_key(&key) {
                continue;
            }
            let seed = if let Some(asset) = &target.spec.initial {
                let seed = Self::image_key(assets, target.owner, asset)?;
                if !self.ensure_image(assets, target.owner, asset, &seed)? {
                    continue;
                }
                Some(seed)
            } else {
                None
            };
            self.target_texture(key.clone(), target.spec.size)?;
            let mut encoder =
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("initialize retained bitmap"),
                    });
            let shade = geometry::tint(target.spec.clear);
            let vertices = geometry::quad(
                Rect {
                    x: 0,
                    y: 0,
                    width: target.spec.size[0],
                    height: target.spec.size[1],
                },
                target.spec.size,
                [255; 4],
            );
            let buffer = self
                .gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("retained bitmap seed"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("initial bitmap contents"),
                    multiview_mask: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.textures[&key].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: shade[0] as f64,
                                g: shade[1] as f64,
                                b: shade[2] as f64,
                                a: shade[3] as f64,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let Some(seed) = &seed {
                    pass.set_pipeline(&self.draw_pipelines.ray_composite_pipeline);
                    pass.set_bind_group(0, &self.textures[seed].bind, &[]);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..6, 0..1);
                }
            }
            self.gpu.queue.submit(Some(encoder.finish()));
        }
        // A same-target read must never bind an attachment as its own texture.
        // Reserve one scratch bitmap per such target, reused at ordering fences.
        let feedback: Vec<_> = self
            .engine_frame
            .iter()
            .filter_map(|p| {
                let (Some(source), Some(output)) = (&p.target, &p.output) else {
                    return None;
                };
                (source == output).then(|| {
                    (
                        format!("{}:read", Self::target_key(assets, source)),
                        source.spec.size,
                    )
                })
            })
            .collect();
        for (key, size) in feedback {
            self.target_texture(key, size)?;
        }
        Ok(())
    }
}
