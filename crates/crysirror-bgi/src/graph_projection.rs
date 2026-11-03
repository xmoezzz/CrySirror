//! Native projected sprite state over shared geometry, pyramids and image jobs.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImagePyramidResource, ImageResource};
use crysirror_image::{
    effects::ImageEffect,
    fixed_projection::{FixedSpriteDynamics, FixedSpriteProjection},
    integer_raster::{
        IntegerRaster,
        perspective_blit::{self, PerspectiveBlit},
    },
    pyramid::RowDeform,
};
#[derive(Default)]
pub(super) struct ProjectionState {
    pub args: Vec<i32>,
    pyramids: Vec<Option<Arc<ImagePyramidResource>>>,
    jobs: Vec<Option<Task<Arc<ImagePyramidResource>>>>,
    pub dirty_rect: Option<[i32; 4]>,
}
#[derive(Clone)]
pub(super) struct ProjectionPlan {
    level: u8,
    source_format: i32,
    transition: i32,
    deform: Option<RowDeform>,
    raster: Raster,
    size: [u32; 2],
}
impl BgiVm {
    pub(super) fn configure_native_projection(&mut self, call: &NativeCall) -> Result<()> {
        let a = &call.args;
        let object = a[0];
        let mode = if call.id == 0x5c { 5 } else { 6 };
        let blend = if mode == 5 { a[14] } else { a[17] };
        let alpha = if mode == 5 { a[15] } else { a[18] };
        let priority = *a.last().unwrap();
        target_checks::bitmap(a[4])?;
        target_checks::mixing_ratio(a[6])?;
        target_checks::effect_mode(blend)?;
        target_checks::effect_level(alpha)?;
        target_checks::priority(priority)?;
        self.graph.require(object)?;
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Sprite),
            "BGI projected sprite handle"
        );
        let primary = self.bitmap(a[4])?.1.clone();
        let secondary = if a[5] == -1 {
            None
        } else {
            Some(self.bitmap(a[5])?.1.clone())
        };
        let p = self.graph.objects.entry(object).or_default();
        p.native.fixed_position_updates_integer_position = 0;
        let round = |v: i32| {
            if p.native.fixed_position_rounding_enabled != 0
                && (a[3] == 0 || p.native.fixed_position_rounding_mode == 1)
            {
                v.wrapping_add(0x8000) & !0xffff
            } else {
                v
            }
        };
        let fixed_x = round(a[1]);
        let fixed_y = round(a[2]);
        p.native.fixed_position_x_16_16 = fixed_x;
        p.native.fixed_position_y_16_16 = fixed_y;
        p.native.fixed_position_z_16_16 = a[3];
        p.native.blend_mode = blend;
        p.blend_mode = blend;
        p.set_alpha_parameter(alpha);
        p.native.priority = priority as u32;
        if secondary
            .as_ref()
            .is_some_and(|b| b.size != primary.size || b.format != primary.format)
        {
            if self.graph.projections.contains_key(&object) {
                self.rebuild_native_projection(object)?;
            }
            return Ok(());
        }
        ensure!(
            matches!(primary.format, 1 | 2),
            "BGI projected sprite pixel format"
        );
        self.graph.images.remove(object);
        self.graph.sprite_affine.remove(&object);
        self.graph.sprite_wave_args.remove(&object);
        self.graph.raster_offsets.remove(&object);
        self.graph.transforms.insert(object, Default::default());
        let p = self.graph.objects.entry(object).or_default();
        p.format_resource = Some(a[4]);
        p.named_properties.insert("target-object-mode".into(), mode);
        for key in [0x40, 0x41, 0x42, 0x43, 0x80, 0x81, 0x82, 0x83, 0x8f] {
            p.properties.remove(&key);
        }
        p.named_properties
            .insert(format!("mode{mode}-secondary-parameter"), a[7]);
        p.named_properties
            .insert(format!("mode{mode}-transition-current"), a[6]);
        let mut bindings = vec![(a[4], primary.generation)];
        if let Some(b) = secondary {
            bindings.push((a[5], b.generation));
        }
        self.graph.frames.insert(object, bindings);
        self.sprites
            .get_mut(&(object as u32))
            .context("BGI projected sprite slot")?
            .bitmap = Some(a[4] as u32);
        self.graph.projections.insert(
            object,
            ProjectionState {
                args: a.clone(),
                ..Default::default()
            },
        );
        self.rebuild_native_projection(object)
    }
    pub(super) fn rebuild_native_projection(&mut self, object: i32) -> Result<()> {
        let Some(state) = self.graph.projections.get(&object) else {
            return Ok(());
        };
        let a = state.args.clone();
        let p = &self.graph.objects[&object];
        let n = p.native;
        let mode = p
            .named_properties
            .get("target-object-mode")
            .copied()
            .unwrap_or(0);
        let b = self.bitmap(a[4])?.1;
        let source_format = b.format;
        let source_size = b.size;
        let pair = |id| p.properties.get(&id).copied().unwrap_or_default();
        let (bx, by) = p.properties.get(&0x42).copied().unwrap_or((65536, 0));
        let bx = if bx == 0 { 1 } else { bx };
        let coupled = by != 0;
        let by = if by == 0 { bx } else { by };
        let (packed, amplitude) = pair(0x60);
        let dynamics = FixedSpriteDynamics {
            progress_8_24: n.fixed_parameter_16_16,
            offset_delta_x_16_16: pair(0x80).0,
            offset_delta_y_16_16: pair(0x80).1,
            rotation_delta_16_16: pair(0x81).0,
            scale_delta_x_16_16: pair(0x82).0,
            scale_delta_y_16_16: pair(0x82).1,
            uniform_scale_delta_16_16: pair(0x81).1,
            curve: graph_effector::fixed_curve_shape(pair(0x8f).0),
            base_scale_x_16_16: bx,
            base_scale_y_16_16: by,
            coupled_scale: coupled,
            width_adjust_16_16: if (packed as u32 >> 16) != 0 {
                amplitude
            } else {
                0
            },
        };
        let vectors = self
            .graph
            .fixed_vectors
            .get(&object)
            .copied()
            .unwrap_or([[0; 3]; 2]);
        let resolved = std::array::from_fn::<_, 3, _>(|i| {
            [
                n.fixed_position_x_16_16,
                n.fixed_position_y_16_16,
                n.fixed_position_z_16_16,
            ][i]
                .wrapping_add(vectors[0][i])
                .wrapping_add(vectors[1][i])
        });
        let anchor = p.properties.get(&0x40).copied().unwrap_or((a[8], a[9]));
        let node = FixedSpriteProjection {
            position_x: resolved[0],
            position_y: resolved[1],
            position_z: resolved[2],
            anchor_x: anchor.0,
            anchor_y: anchor.1,
            rotation: p.properties.get(&0x41).map_or(a[10], |p| p.0),
            perspective: a[11],
            project_position: a[12] != 0,
            interpolation: a[13] != 0,
        };
        let transform_parameters = if mode == 5 {
            node.mode5_transform_parameters(dynamics)?
        } else {
            let v = self.native_mode6_parameters(&a)?;
            [v[0], v[1], v[4], v[5], v[6]]
        };
        let scale = if mode == 5 {
            transform_parameters[3] as u32
        } else {
            perspective_ratio(resolved[2], a[14] as u32)
        };
        let mut level = crysirror_image::pyramid::select_level(scale);
        let mut available = 0;
        let mut mipped = source_size;
        while available < level && mipped[0] >= 2 && mipped[1] >= 2 {
            mipped = [mipped[0].div_ceil(2), mipped[1].div_ceil(2)];
            available += 1;
        }
        level = available;
        let deform = if packed as u32 >> 16 != 0 && amplitude != 0 {
            Some(RowDeform {
                wavelength: (packed as u32 >> 16) >> level,
                phase: (packed as u32 & 65535) >> level,
                amplitude,
            })
        } else {
            None
        };
        let prepared_size = deform
            .as_ref()
            .map_or(Ok(mipped), |d| d.output_size(mipped))?;
        let anchor_shift =
            (prepared_size[0].wrapping_sub(mipped[0]).wrapping_shl(15) & 0xffff0000) as i32;
        let (size, raster, ordinary, raster_offset) = if mode == 5 {
            let g = node.screen_geometry(
                source_size[0] as f32,
                source_size[1] as f32,
                self.graph.screen[0] as f32,
                self.graph.screen[1] as f32,
                (self.graph.center[0], self.graph.center[1]),
                n.use_graph_center != 0,
                dynamics,
            )?;
            let size = [g.width as u32, g.height as u32];
            ensure!(
                size[0] >= 2 && size[1] >= 2,
                "BGI projected sprite extent below two"
            );
            let [ax, ay, angle, sx, sy] = node.mode5_transform_parameters(dynamics)?;
            let mut ax = (ax >> level).wrapping_add(anchor_shift);
            let mut ay = ay >> level;
            let sx = sx.wrapping_shl(level);
            let sy = sy.wrapping_shl(level);
            if level > 0 || deform.is_some() {
                let mut temp = sx as u32;
                let mut j = 1;
                while temp < 65536 && temp != 0 && j < 32 {
                    let delta = if temp >= 32768 {
                        (65536 - temp) >> j
                    } else {
                        32768 >> j
                    };
                    ax = ax.wrapping_add(delta as i32);
                    ay = ay.wrapping_add(delta as i32);
                    temp = temp.wrapping_mul(2);
                    j += 1;
                }
            }
            let project = |v: i32| {
                if node.project_position {
                    ((i64::from(v) * i64::from(node.perspective_scale_fixed())) >> 16) as i32
                } else {
                    v
                }
            };
            let destination = [
                (g.origin_offset_x as i32)
                    .wrapping_shl(16)
                    .wrapping_add(project(node.position_x) & 65535),
                (g.origin_offset_y as i32)
                    .wrapping_shl(16)
                    .wrapping_add(project(node.position_y) & 65535),
            ];
            (
                size,
                Raster::Display {
                    spec: DisplayRaster::InverseAffine {
                        destination,
                        transform: [ax, ay, angle, sx, sy],
                        bilinear: node.interpolation,
                    },
                },
                [g.object_x as i32, g.object_y as i32],
                [-(g.origin_offset_x as i32), -(g.origin_offset_y as i32)],
            )
        } else {
            let [ox, oy, pitch, heading, bank, sx, sy] = self.native_mode6_parameters(&a)?;
            let ox = (ox >> level).wrapping_add(anchor_shift);
            let oy = oy >> level;
            let sx = sx.wrapping_shl(level);
            let sy = sy.wrapping_shl(level);
            let center = if n.use_graph_center != 0
                && self.graph.center[0] >= 0
                && self.graph.center[1] >= 0
                && (self.graph.center[0] as u32) < self.graph.screen[0]
                && (self.graph.center[1] as u32) < self.graph.screen[1]
            {
                self.graph.center
            } else {
                [
                    self.graph.screen[0] as i32 / 2,
                    self.graph.screen[1] as i32 / 2,
                ]
            };
            let (dest_x, dest_y, translate_x, translate_y) = if a[15] != 0 {
                (
                    center[0].wrapping_shl(16),
                    center[1].wrapping_shl(16),
                    resolved[0],
                    resolved[1],
                )
            } else {
                (
                    center[0].wrapping_shl(16).wrapping_add(resolved[0]),
                    center[1].wrapping_shl(16).wrapping_add(resolved[1]),
                    0,
                    0,
                )
            };
            let transform = PerspectiveBlit {
                dest_x,
                dest_y,
                anchor_x: ox,
                anchor_y: oy,
                scale_x: sx,
                scale_y: sy,
                translate_x,
                translate_y,
                translate_z: resolved[2],
                pitch,
                heading,
                bank,
                order: a[13],
                distance: a[14],
                mode: 0,
                parameter: 0,
            };
            let corners =
                perspective_blit::project_corners(prepared_size[0], prepared_size[1], &transform);
            let [left, top, right, bottom] = perspective_blit::bounds(&corners);
            let size = [
                u32::try_from(i64::from(right) - i64::from(left) + 1)?,
                u32::try_from(i64::from(bottom) - i64::from(top) + 1)?,
            ];
            ensure!(
                size[0] >= 2 && size[1] >= 2,
                "BGI projected sprite extent below two"
            );
            (
                size,
                Raster::Display {
                    spec: DisplayRaster::PerspectiveRaster {
                        transform,
                        origin: [left, top],
                    },
                },
                [left, top],
                [0; 2],
            )
        };
        crysirror_image::rgba_len(size[0], size[1])?;
        let transition = p
            .named_properties
            .get(&format!("mode{mode}-transition-current"))
            .copied()
            .unwrap_or(a[6]);
        let p = self.graph.objects.entry(object).or_default();
        for (i, value) in transform_parameters.into_iter().enumerate() {
            p.named_properties
                .insert(format!("projected-transform-{i}"), value);
        }
        p.native.position_x = ordinary[0];
        p.native.position_y = ordinary[1];
        p.named_properties
            .insert(format!("mode{mode}-projected-width"), size[0] as i32);
        p.named_properties
            .insert(format!("mode{mode}-projected-height"), size[1] as i32);
        self.graph.raster_offsets.insert(object, raster_offset);
        self.graph.sort(object);
        self.set_graph_plan(
            object,
            GraphPlan {
                inputs: [a[4], a[5]]
                    .into_iter()
                    .filter(|id| *id != -1)
                    .map(GraphInput::Bitmap)
                    .collect(),
                operation: None,
                quad: None,
                source_format: None,
                compound: None,
                warp: None,
                projection: Some(ProjectionPlan {
                    level: level as u8,
                    source_format,
                    transition,
                    deform,
                    raster,
                    size,
                }),
                format: 2,
                force_opaque: false,
                filters: Vec::new(),
            },
        );
        Ok(())
    }
    pub(super) fn projection_image(
        &mut self,
        object: i32,
        plan: &ProjectionPlan,
        sources: Vec<Arc<dyn ImageResource>>,
    ) -> Result<Option<ImageInput>> {
        let cached = self
            .graph
            .projections
            .values()
            .flat_map(|s| &s.pyramids)
            .filter_map(Option::as_ref)
            .map(|p| p.byte_len())
            .sum::<usize>();
        let initializing = self
            .graph
            .projections
            .get(&object)
            .is_some_and(|s| s.pyramids.is_empty());
        if initializing {
            let requested = sources
                .iter()
                .map(|source| {
                    let size = source.size();
                    size[0] as usize * size[1] as usize * 6
                })
                .sum::<usize>();
            ensure!(
                cached
                    .checked_add(requested)
                    .is_some_and(|bytes| bytes <= 256 * 1024 * 1024),
                "BGI retained image pyramid budget"
            );
        }
        let pool = self.pool.clone();
        let state = self
            .graph
            .projections
            .get_mut(&object)
            .context("BGI projected sprite state")?;
        if state.pyramids.is_empty() {
            state.pyramids = (0..sources.len()).map(|_| None).collect();
            state.jobs = (0..sources.len()).map(|_| None).collect();
        }
        for i in 0..sources.len() {
            if state.pyramids[i].is_none() && state.jobs[i].is_none() {
                let source = sources[i].clone();
                state.jobs[i] = Some(pool.spawn(move |cancel| {
                    crysirror_image::pyramid::build(source, &|| cancel.check())
                })?);
            }
        }
        if state.jobs.iter().all(Option::is_none) && state.pyramids.iter().all(Option::is_some) {
            if let Some(rect) = state.dirty_rect.take() {
                for i in 0..sources.len() {
                    let pyramid = state.pyramids[i].as_ref().unwrap().clone();
                    let source = sources[i].clone();
                    state.jobs[i] = Some(pool.spawn(move |cancel| {
                        crysirror_image::pyramid::update(&pyramid, source, rect, &|| cancel.check())
                    })?);
                }
            }
        }
        for i in 0..state.jobs.len() {
            if let Some(job) = &mut state.jobs[i] {
                if let Some(pyramid) = job.try_take()? {
                    state.pyramids[i] = Some(pyramid);
                    state.jobs[i] = None;
                }
            }
        }
        if state.jobs.iter().any(Option::is_some) || state.dirty_rect.is_some() {
            return Ok(None);
        }
        let input = |i: usize| -> ImageInput {
            if plan.level == 0 {
                ImageInput::Resource(sources[i].clone())
            } else {
                ImageInput::PyramidLevel {
                    pyramid: state.pyramids[i].as_ref().unwrap().clone(),
                    level: plan.level,
                }
            }
        };
        let mut source = input(0);
        if sources.len() == 2 {
            source = ImageInput::Process {
                input: Box::new(ImageInput::Raster {
                    destination: Box::new(source),
                    source: Box::new(input(1)),
                }),
                effects: serde_json::to_vec(&vec![ImageEffect::IntegerRaster {
                    spec: IntegerRaster {
                        destination_format: plan.source_format as u32,
                        source_format: plan.source_format as u32,
                        raster: Raster::Display {
                            spec: DisplayRaster::Crossfade {
                                amount: plan.transition,
                            },
                        },
                    },
                }])?
                .into(),
            };
        }
        if plan.source_format == 1 {
            source = ImageInput::Process {
                input: Box::new(source),
                effects: serde_json::to_vec(&vec![ImageEffect::OverrideAlpha { alpha: 255 }])?
                    .into(),
            };
        }
        if let Some(spec) = &plan.deform {
            source = ImageInput::Process {
                input: Box::new(source),
                effects: serde_json::to_vec(&vec![ImageEffect::RowDeform { spec: *spec }])?.into(),
            };
        }
        Ok(Some(ImageInput::Process {
            input: Box::new(ImageInput::Raster {
                destination: Box::new(ImageInput::Solid {
                    size: plan.size,
                    color: [0; 4],
                }),
                source: Box::new(source),
            }),
            effects: serde_json::to_vec(&vec![ImageEffect::IntegerRaster {
                spec: IntegerRaster {
                    destination_format: 2,
                    source_format: 2,
                    raster: plan.raster.clone(),
                },
            }])?
            .into(),
        }))
    }
}
fn perspective_ratio(z: i32, focal: u32) -> u32 {
    if z == 0 || focal == 0 {
        65536
    } else if z < 0 {
        (((u64::from(focal) << 16) + u64::from(z.unsigned_abs())) / u64::from(focal)) as u32
    } else {
        ((u64::from(focal) << 32) / ((u64::from(focal) << 16) + z as u64)) as u32
    }
}

impl BgiVm {
    fn native_mode6_parameters(&self, a: &[i32]) -> Result<[i32; 7]> {
        if a.len() != 20 {
            bail!("BGI mode6 argument count");
        }
        let properties = self.graph.objects.get(&a[0]).context("BGI mode6 object")?;
        let native = properties.native;
        let pair = |id| properties.properties.get(&id).copied().unwrap_or((0, 0));

        let progress = native.fixed_parameter_16_16;
        let advance = |delta: i32| ((i64::from(delta) * i64::from(progress as u32)) >> 24) as i32;
        let curve = crysirror_image::progress_curve::sample_fixed_progress(
            graph_effector::fixed_curve_shape(pair(0x8f).0),
            progress,
        )?;
        let angle_delta = |delta: i32| ((i64::from(delta) * i64::from(curve)) >> 16) as i32;
        let anchor = properties
            .properties
            .get(&0x40)
            .copied()
            .unwrap_or((a[8], a[9]));
        let pitch_heading = properties
            .properties
            .get(&0x43)
            .copied()
            .unwrap_or((a[10], a[11]));
        let bank = properties.properties.get(&0x41).map_or(a[12], |p| p.0);
        // 429FD0 shares the two-scale equation with mode 5. Rotation has
        // three independent coefficients in mode 6 (83 pitch/heading,81 bank).
        let (base_x, base_y) = properties
            .properties
            .get(&0x42)
            .copied()
            .unwrap_or((65536, 0));
        let base_x = if base_x == 0 { 1 } else { base_x };
        let coupled = base_y != 0;
        let base_y = if base_y == 0 { base_x } else { base_y };
        let scale_delta = pair(0x82);
        let uniform = pair(0x81).1;
        let product = |a: i32, b: i32| ((u64::from(a as u32) * u64::from(b as u32)) >> 16) as i32;
        let scales = if coupled {
            [
                product(
                    base_x,
                    65536i32.wrapping_add(advance(scale_delta.0.wrapping_add(uniform))),
                ),
                product(
                    base_y,
                    65536i32.wrapping_add(advance(scale_delta.1.wrapping_add(uniform))),
                ),
            ]
        } else {
            [
                product(
                    65536i32.wrapping_add(advance(scale_delta.0)),
                    base_x.wrapping_add(advance(uniform)),
                ),
                product(
                    65536i32.wrapping_add(advance(scale_delta.1)),
                    base_y.wrapping_add(advance(uniform)),
                ),
            ]
        };
        let offset_x = anchor
            .0
            .wrapping_shl(16)
            .wrapping_add(advance(pair(0x80).0));
        let offset_y = anchor
            .1
            .wrapping_shl(16)
            .wrapping_add(advance(pair(0x80).1));
        let pitch = pitch_heading.0.wrapping_add(angle_delta(pair(0x83).0));
        let heading = pitch_heading.1.wrapping_add(angle_delta(pair(0x83).1));
        let bank = bank.wrapping_add(angle_delta(pair(0x81).0));
        Ok([
            offset_x, offset_y, pitch, heading, bank, scales[0], scales[1],
        ])
    }
}

impl BgiVm {
    pub(super) fn sync_native_projection_fixed(&mut self, object: i32, raw: i32) -> Result<()> {
        if !self.graph.projections.contains_key(&object) {
            return Ok(());
        }
        let p = self.graph.objects.entry(object).or_default();
        let mode = p.named_properties["target-object-mode"];
        if p.named_properties
            .get(&format!("mode{mode}-secondary-parameter"))
            == Some(&3)
        {
            p.named_properties.insert(
                format!("mode{mode}-transition-current"),
                (raw as u32 >> 16) as i32,
            );
        }
        self.rebuild_native_projection(object)
    }
    pub(super) fn native_sprite_property(
        &mut self,
        object: i32,
        property: u32,
        value: i32,
        extra: i32,
    ) -> Result<()> {
        let mode = self
            .graph
            .objects
            .get(&object)
            .and_then(|p| p.named_properties.get("target-object-mode"))
            .copied()
            .unwrap_or(0);
        if property == 0x10 {
            return self.replace_native_sprite_source(object, value);
        }
        let transformed = matches!(mode, 2 | 5 | 6);
        let store = match property {
            0x40 | 0x41 | 0x42 | 0x80 | 0x81 | 0x82 => transformed,
            0x43 | 0x83 => mode == 6,
            0x60 | 0x8f => true,
            _ => bail!("BGI unsupported sprite property"),
        };
        if store {
            self.graph
                .objects
                .entry(object)
                .or_default()
                .properties
                .insert(property, (value, extra));
        }
        if matches!(property, 0x41 | 0x42) && matches!(mode, 5 | 6) {
            self.rebuild_native_projection(object)?;
        }
        Ok(())
    }
    pub(super) fn replace_native_sprite_source(&mut self, object: i32, bitmap: i32) -> Result<()> {
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Sprite),
            "BGI bitmap replacement requires sprite"
        );
        let mode = self
            .graph
            .objects
            .get(&object)
            .and_then(|p| p.named_properties.get("target-object-mode"))
            .copied()
            .unwrap_or(0);
        if !matches!(mode, 0 | 2 | 5 | 6) {
            return Ok(());
        }
        let b = self.bitmap(bitmap)?.1.clone();
        self.sprites
            .get_mut(&(object as u32))
            .context("BGI sprite slot")?
            .bitmap = Some(bitmap as u32);
        self.graph
            .objects
            .entry(object)
            .or_default()
            .format_resource = Some(bitmap);
        if mode == 0 {
            self.graph.images.remove(object);
            let plan = GraphPlan::bitmap(bitmap, b.format);
            if self.graph.objects[&object]
                .properties
                .keys()
                .any(|key| key & 0xff00 == 0x1100)
            {
                self.set_graph_plan(object, plan);
            }
            return Ok(());
        }
        if mode == 2 {
            let p = &self.graph.objects[&object];
            if let Some(meta) = self.graph.sprite_affine.get_mut(&object) {
                if let Some(&(x, y)) = p.properties.get(&0x40) {
                    meta[0] = x;
                    meta[1] = y;
                }
                if let Some(&(angle, _)) = p.properties.get(&0x41) {
                    meta[2] = angle;
                }
            }
            return self.rebuild_native_sprite_affine(object);
        }
        self.replace_native_projection_source(object, bitmap)
    }
    pub(super) fn replace_native_projection_source(
        &mut self,
        object: i32,
        bitmap: i32,
    ) -> Result<()> {
        let b = self.bitmap(bitmap)?.1.clone();
        ensure!(
            matches!(b.format, 1 | 2),
            "BGI projected replacement colour format"
        );
        let state = self
            .graph
            .projections
            .get_mut(&object)
            .context("BGI projected replacement state")?;
        state.args[4] = bitmap;
        state.args[5] = -1;
        state.args[6] = 0;
        state.args[7] = -1;
        state.pyramids.clear();
        state.jobs.clear();
        state.dirty_rect = None;
        self.graph
            .frames
            .insert(object, vec![(bitmap, b.generation)]);
        let p = self.graph.objects.entry(object).or_default();
        let mode = p.named_properties["target-object-mode"];
        p.named_properties
            .insert(format!("mode{mode}-transition-current"), 0);
        p.named_properties
            .insert(format!("mode{mode}-secondary-parameter"), -1);
        self.rebuild_native_projection(object)
    }
}
