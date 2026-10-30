// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Isolated engine subtree rendering. Uses the same renderer for both inputs;
//! child opacity is applied before the crossfade, never independently faded.
use super::*;
use wgpu::util::DeviceExt;
#[path = "composite_filter.rs"]
mod filter;
use crysirror_scene::layer::SubtreeTransitionEffect;
struct Snapshot {
    before: wgpu::TextureView,
    after: wgpu::TextureView,
    _a: wgpu::Texture,
    _b: wgpu::Texture,
}

pub(super) struct Compositor {
    renderer: Box<Renderer>,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    filter: Option<filter::Filter>,
    snapshots: BTreeMap<(String, u64, [u32; 2]), Snapshot>,
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
            filter: None,
            snapshots: BTreeMap::new(),
        }
    }
}
impl Renderer {
    fn transition_inputs_ready(
        &mut self,
        assets: &mut Assets,
        inputs: &[crysirror_scene::layer::Primitive],
    ) -> Result<bool> {
        let mut ready = true;
        for primitive in inputs {
            if let Some(composite) = &primitive.composite {
                ready &= self.transition_inputs_ready(assets, &composite.before)?;
                ready &= self.transition_inputs_ready(assets, &composite.after)?;
                continue;
            }
            match &primitive.paint {
                Paint::Image { asset, .. } => {
                    let key = Self::sampled_image_key(
                        assets,
                        primitive.owner,
                        asset,
                        primitive.sampling,
                    )?;
                    ready &= self.ensure_sampled_image(
                        assets,
                        primitive.owner,
                        asset,
                        &key,
                        primitive.sampling,
                    )?;
                }
                Paint::Snapshot { size, layers } => {
                    let key = Self::snapshot_key(assets, primitive.owner, *size, layers)?;
                    ready &= self.textures.contains_key(&key);
                }
                _ => {}
            }
            if let Some(filter) = &primitive.filter {
                for asset in filter
                    .mask
                    .as_ref()
                    .map(|m| m.asset.as_str())
                    .into_iter()
                    .chain(filter.tone_curve.as_ref().map(|m| m.asset.as_str()))
                {
                    let key = Self::image_key(assets, primitive.owner, asset)?;
                    ready &= self.ensure_image(assets, primitive.owner, asset, &key)?;
                }
            }
        }
        Ok(ready)
    }
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
            if let Some(compositor) = self.compositor.as_mut() {
                compositor.snapshots.clear();
            }
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
        let effect_bytes: u64 = composites
            .iter()
            .map(|(_, _, frame)| match frame.effect.as_ref() {
                Some((SubtreeTransitionEffect::Blur { dynamic, .. }, _)) => {
                    32 + if !dynamic { 2 * bytes_per_pixel } else { 0 }
                }
                Some((SubtreeTransitionEffect::Dim { .. }, _)) => 24,
                None => 0,
            })
            .sum();
        let work_bytes =
            pixel_count * (bytes_per_pixel * (composites.len() as u64 + 2) + 4 + effect_bytes);
        ensure!(
            work_bytes <= 128 * 1024 * 1024,
            "composite working set exceeds 128 MiB"
        );
        let mut c = self
            .compositor
            .take()
            .unwrap_or_else(|| Compositor::new(self.gpu.clone(), self.format));
        if composites
            .iter()
            .any(|(_, _, frame)| frame.effect.is_some())
            && c.filter.is_none()
        {
            for format in [
                wgpu::TextureFormat::Rgba32Float,
                wgpu::TextureFormat::Rgba16Float,
            ] {
                let usages = self
                    .gpu
                    .adapter
                    .get_texture_format_features(format)
                    .allowed_usages;
                ensure!(
                    usages.contains(
                        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING
                    ),
                    "GPU lacks storage/sample support for subtree filter {format:?}"
                );
            }
            let limits = self.gpu.device.limits();
            ensure!(
                limits.max_compute_workgroup_size_x >= 64
                    && limits.max_compute_invocations_per_workgroup >= 64
                    && limits.max_compute_workgroup_storage_size >= 5120,
                "GPU lacks bounded transition filter workgroup limits"
            );
            c.filter = Some(filter::Filter::new(&self.gpu));
        }
        let desired_snapshots: std::collections::BTreeSet<_> = composites
            .iter()
            .filter_map(|(owner, id, frame)| {
                matches!(
                    frame.effect.as_ref(),
                    Some((SubtreeTransitionEffect::Blur { dynamic: false, .. }, _))
                )
                .then(|| {
                    Self::image_key(assets, *owner, &format!("__crysirror_composite/{id}"))
                        .map(|key| (key, frame.serial, canvas))
                })
            })
            .collect::<Result<_>>()?;
        c.snapshots.retain(|key, _| desired_snapshots.contains(key));
        let result = (|| {
            let mut products = BTreeMap::new();
            let mut active_snapshots = std::collections::BTreeSet::new();
            for (owner, id, frame) in composites {
                ensure!(
                    frame.progress.is_finite() && (0.0..=1.0).contains(&frame.progress),
                    "invalid composite progress"
                );
                // Nested composites (an inner crossfade inside this one's
                // inputs) are prepared by the child renderer's own compositor.
                ensure!(
                    frame
                        .before
                        .iter()
                        .chain(&frame.after)
                        .all(|p| p.owner == owner),
                    "foreign-client compositor input"
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
                let key = Self::image_key(assets, owner, &asset)?;
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
                let snapshot_key = (key.clone(), frame.serial, canvas);
                let static_input = matches!(
                    frame.effect.as_ref(),
                    Some((SubtreeTransitionEffect::Blur { dynamic: false, .. }, _))
                );
                let retained = static_input
                    .then(|| c.snapshots.get(&snapshot_key))
                    .flatten();
                let a = retained
                    .map(|snapshot| snapshot._a.clone())
                    .unwrap_or_else(|| texture("subtree before"));
                let b = retained
                    .map(|snapshot| snapshot._b.clone())
                    .unwrap_or_else(|| texture("subtree after"));
                let output = texture("subtree crossfade product");
                let av = a.create_view(&Default::default());
                let bv = b.create_view(&Default::default());
                let view = output.create_view(&Default::default());

                if static_input {
                    active_snapshots.insert(snapshot_key.clone());
                }
                let inputs_ready = if static_input && !c.snapshots.contains_key(&snapshot_key) {
                    c.renderer.transition_inputs_ready(assets, &frame.before)?
                        && c.renderer.transition_inputs_ready(assets, &frame.after)?
                } else {
                    true
                };
                let capture_inputs = !static_input || !c.snapshots.contains_key(&snapshot_key);
                if capture_inputs {
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
                }
                if static_input && capture_inputs && inputs_ready {
                    c.snapshots.insert(
                        snapshot_key.clone(),
                        Snapshot {
                            before: av.clone(),
                            after: bv.clone(),
                            _a: a.clone(),
                            _b: b.clone(),
                        },
                    );
                }
                let (mut input_a, mut input_b) =
                    if static_input && c.snapshots.contains_key(&snapshot_key) {
                        let snapshot = &c.snapshots[&snapshot_key];
                        (snapshot.before.clone(), snapshot.after.clone())
                    } else {
                        (av.clone(), bv.clone())
                    };
                let mut _processed_inputs = Vec::new();
                let mut _processed_rule = Vec::new();
                let mut rule_view = self.textures[&rule_key].view.clone();
                let mut mode = if frame.rule.is_some() && rule_ready {
                    1.
                } else {
                    0.
                };
                let mut progress = frame.progress;
                let mut phase = 0.;
                if let Some((effect, rect)) = &frame.effect {
                    effect.validate()?;
                    progress = effect.progress(frame.timing[0], frame.timing[1]) as f32;
                    match effect {
                        SubtreeTransitionEffect::Blur { .. } => {
                            let (before, after) = effect.radii(frame.timing[0], frame.timing[1]);
                            _processed_inputs = c.filter.as_ref().unwrap().process(
                                &self.gpu,
                                &[(&input_a, before), (&input_b, after)],
                                canvas,
                                *rect,
                                false,
                                false,
                            );
                            input_a = _processed_inputs[0].view.clone();
                            input_b = _processed_inputs[1].view.clone();
                            mode = 2.;
                        }
                        SubtreeTransitionEffect::Dim {
                            rule,
                            blur,
                            negative,
                            ..
                        } if rule_ready => {
                            _processed_rule = c.filter.as_ref().unwrap().process(
                                &self.gpu,
                                &[(&rule_view, *blur)],
                                canvas,
                                *rect,
                                true,
                                *negative,
                            );
                            rule_view = _processed_rule[0].view.clone();
                            mode = 3.;
                            phase = (effect.progress(frame.timing[0], frame.timing[1])
                                * (255. + f64::from(rule.vague)))
                            .floor() as f32;
                        }
                        SubtreeTransitionEffect::Dim { .. } => {}
                    }
                }
                let uniform =
                    self.gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("crossfade progress"),
                            contents: bytemuck::cast_slice(&{
                                let (vague, _, mut rect) = match &frame.rule {
                                    Some((r, rect)) if rule_ready => (r.vague as f32, 1., *rect),
                                    _ => (0., 0., [0., 0., 1., 1.]),
                                };
                                if let Some((_, effect_rect)) = &frame.effect {
                                    rect = *effect_rect;
                                }
                                let pending =
                                    (frame.rule.is_some() && !rule_ready) || !inputs_ready;
                                [
                                    if pending { 0. } else { progress.clamp(0., 1.) },
                                    vague,
                                    mode,
                                    phase,
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
                                resource: wgpu::BindingResource::TextureView(&input_a),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&input_b),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&rule_view),
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
                        texture: output,
                        bytes: pixel_count * bytes_per_pixel,
                    },
                );
            }
            c.snapshots.retain(|key, _| active_snapshots.contains(key));
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

/// Snapshot draws one frame may reference; their memory is bounded by the
/// working-set and cache budgets checked per snapshot.
const MAX_FRAME_SNAPSHOTS: usize = 512;

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
        // Text is rasterized from the shared fonts: their content digests
        // are part of the key, so replacing a font alias re-keys the snapshot.
        let mut fonts = BTreeSet::new();
        fn collect<'a>(
            layers: &'a [crysirror_scene::layer::LayerSpec],
            out: &mut BTreeSet<&'a str>,
        ) {
            for layer in layers {
                match &layer.paint {
                    Paint::Text { font, .. } | Paint::GlyphRun { font, .. } => {
                        out.insert(font.as_str());
                    }
                    Paint::Snapshot { layers, .. } => collect(layers, out),
                    _ => {}
                }
            }
        }
        collect(layers, &mut fonts);
        let mut hash = blake3::Hasher::new();
        hash.update(&definition);
        for font in fonts {
            hash.update(font.as_bytes());
            hash.update(assets.text.for_client(crysirror_core::state::ClientId(owner)).font_digest(font)?.as_bytes());
        }
        Ok(format!(
            "snapshot:{}:{owner}:{}",
            assets.store_id(),
            hash.finalize()
        ))
    }

    /// Frozen engine resources only. A namespace-shaped string is not proof:
    /// generated_for must find the bytes in the owner's immutable resource map.
    fn snapshot_dependencies(
        plain: &mut std::collections::HashMap<(u32, String), blake3::Hash>,
        assets: &mut Assets,
        owner: u32,
        layers: &[crysirror_scene::layer::LayerSpec],
    ) -> Result<Option<Vec<crysirror_runtime::assets::AssetLease>>> {
        let mut names = BTreeSet::new();
        // Nested snapshot payloads are rasterized first by the isolated
        // renderer; their images are dependencies of this snapshot too.
        let mut all = Vec::new();
        fn flatten<'a>(
            layers: &'a [crysirror_scene::layer::LayerSpec],
            out: &mut Vec<&'a crysirror_scene::layer::LayerSpec>,
        ) {
            for layer in layers {
                if let Paint::Snapshot { layers, .. } = &layer.paint {
                    flatten(layers, out);
                }
                out.push(layer);
            }
        }
        flatten(layers, &mut all);
        for layer in all {
            match &layer.paint {
                Paint::Solid { .. } => {}
                Paint::Image { asset, .. } => {
                    names.insert(asset.as_str());
                }
                // Fonts are pinned by content digest in the snapshot key.
                Paint::Text { font, .. } | Paint::GlyphRun { font, .. } => {
                    assets.text.for_client(crysirror_core::state::ClientId(owner)).font_digest(font)?;
                }
                Paint::Snapshot { .. } => {}
                Paint::Puppet { .. } => anyhow::bail!("dynamic snapshot dependency"),
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
            // Engine-published images must be self-contained PNG (other codecs
            // can read mutable companion files). Images served by a mounted
            // container are immutable while mounted; plain files are refused.
            if !assets.container_backed(ClientId(owner), name) {
                if name.starts_with("__crysirror/") {
                    let bytes =
                        assets
                            .generated_for(ClientId(owner), name)?
                            .with_context(|| {
                                format!(
                                    "snapshot requires a published immutable engine image: {name}"
                                )
                            })?;
                    ensure!(
                        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                        "snapshot immutable image profile requires self-contained PNG: {name}"
                    );
                } else {
                    // A plain project file: only self-contained codecs (no
                    // companion files), pinned by content on first use; a
                    // later change fails visibly instead of reusing a stale
                    // snapshot keyed by name.
                    let bytes = assets.read_for(ClientId(owner), name)?;
                    ensure!(
                        self_contained_image(name, &bytes),
                        "snapshot requires a self-contained image (PNG, JPEG, BMP, g00): {name}"
                    );
                    let hash = blake3::hash(&bytes);
                    let pinned = *plain.entry((owner, name.to_owned())).or_insert(hash);
                    ensure!(pinned == hash, "snapshot image changed on disk: {name}");
                }
            }
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
        // The same frozen content drawn several times is one snapshot.
        let mut seen = BTreeSet::new();
        let mut snapshots = Vec::new();
        for p in &self.engine_frame {
            if let Paint::Snapshot { size, layers } = &p.paint {
                if seen.insert(Self::snapshot_key(assets, p.owner, *size, layers)?) {
                    snapshots.push((p.owner, *size, layers.clone()));
                }
            }
        }
        if snapshots.is_empty() {
            return Ok(());
        }
        ensure!(
            snapshots.len() <= MAX_FRAME_SNAPSHOTS,
            "snapshot frame count limit ({} > {MAX_FRAME_SNAPSHOTS})",
            snapshots.len()
        );
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
                let Some(_pins) = Self::snapshot_dependencies(
                    &mut self.snapshot_plain_images,
                    assets,
                    owner,
                    &layers,
                )?
                else {
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
                        // COPY_SRC: compacted canvases are materialized from it.
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_SRC,
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
                            contents: bytemuck::cast_slice(&[0.0f32; 8]),
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
                            // Rule slot unused here (params.a.z = 0).
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&input),
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
                        texture: output,
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

impl Renderer {
    /// The straight-alpha pixels of a snapshot this renderer has rasterized
    /// (a compacted canvas), or `None` while it has not been drawn yet.
    pub fn read_snapshot(
        &self,
        assets: &Assets,
        owner: u32,
        size: [u32; 2],
        layers: &[crysirror_scene::layer::LayerSpec],
    ) -> Result<Option<image::RgbaImage>> {
        let key = Self::snapshot_key(assets, owner, size, layers)?;
        let Some(texture) = self.textures.get(&key) else {
            return Ok(None);
        };
        self.read_texture(&texture.texture, size[0], size[1])
            .map(Some)
    }

    /// Rasterize one snapshot now, outside a frame (a compacted canvas a
    /// headless host has not drawn yet), and read its pixels back. `None`
    /// while its image dependencies are still loading.
    pub fn materialize_snapshot(
        &mut self,
        assets: &mut Assets,
        owner: u32,
        size: [u32; 2],
        layers: &std::sync::Arc<Vec<crysirror_scene::layer::LayerSpec>>,
    ) -> Result<Option<image::RgbaImage>> {
        let key = Self::snapshot_key(assets, owner, size, layers)?;
        if !self.textures.contains_key(&key) {
            let probe = crysirror_scene::layer::Primitive {
                composite: None,
                owner,
                id: 0,
                sub: 0,
                order: [0, 0],
                size,
                extent: [size[0] as f32, size[1] as f32],
                paint: Paint::Snapshot {
                    size,
                    layers: layers.clone(),
                },
                quad: [[0.; 4]; 4],
                clip: None,
                opacity: 255,
                blend: crysirror_scene::layer::Blend::Normal,
                uv: [0., 0., 1., 1.],
                sampling: Default::default(),
                filter: None,
                puppet: None,
            };
            let saved = std::mem::replace(&mut self.engine_frame, vec![probe]);
            let result = self.prepare_snapshots(assets);
            self.engine_frame = saved;
            result?;
        }
        self.read_snapshot(assets, owner, size, layers)
    }
}

/// Image files whose decode reads no other file.
fn self_contained_image(name: &str, bytes: &[u8]) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"BM")
        || (ext == "g00" && bytes.first().is_some_and(|v| *v <= 2))
}
