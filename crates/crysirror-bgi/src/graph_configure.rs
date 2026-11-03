//! Native subclass operands become reusable image operation plans.
use super::*;
impl BgiVm {
    pub(super) fn configure_native_sprite_pair(&mut self, call: &NativeCall) -> Result<()> {
        let a = &call.args;
        let object = a[0];
        self.graph.require(object)?;
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Sprite),
            "BGI invalid sprite handle"
        );
        for id in [a[3], a[4]] {
            target_checks::bitmap(id)?;
            self.bitmap(id)?;
        }
        let primary = self.bitmap(a[3])?.1.clone();
        let secondary = self.bitmap(a[4])?.1.clone();
        let (mode, blend, alpha, priority, operation) = if call.id == 0x58 {
            target_checks::mixing_ratio(a[5])?;
            target_checks::transparency(a[6])?;
            target_checks::priority(a[7])?;
            ensure!(
                primary.size == secondary.size,
                "BGI dual sprite bitmap sizes differ"
            );
            ensure!(
                primary.format == secondary.format && matches!(primary.format, 1 | 2),
                "BGI dual sprite native pixel formats"
            );
            (
                1,
                1,
                a[6],
                a[7],
                Raster::Display {
                    spec: DisplayRaster::Crossfade { amount: a[5] },
                },
            )
        } else {
            target_checks::priority(a[9])?;
            target_checks::effect_level(a[8])?;
            target_checks::effect_mode(a[7])?;
            target_checks::effect_level(a[6])?;
            ensure!(
                secondary.format == 3,
                "BGI sprite filtering bitmap is not grayscale"
            );
            (
                3,
                a[7],
                a[8],
                a[9],
                Raster::Display {
                    spec: DisplayRaster::ThresholdMask {
                        shift: a[5] as u32 & 31,
                        threshold: a[6] as u32,
                    },
                },
            )
        };
        self.graph.projections.remove(&object);
        self.graph.sprite_affine.remove(&object);
        self.graph.raster_offsets.remove(&object);
        self.graph.images.remove(object);
        self.graph.frames.remove(&object);
        self.graph.sprite_wave_args.remove(&object);
        if mode == 3 {
            self.graph
                .images
                .mode3_generation
                .insert(object, (a[3] as u32, primary.generation));
        }
        self.graph.transforms.remove(&object);
        self.graph.position(object, a[1], a[2]);
        let p = self.graph.objects.entry(object).or_default();
        p.format_resource = Some(a[3]);
        p.blend_mode = blend;
        p.native.blend_mode = blend;
        p.set_alpha_parameter(alpha);
        p.native.priority = priority as u32;
        p.named_properties.insert("target-object-mode".into(), mode);
        if mode == 1 {
            p.named_properties.insert("mode1-selector".into(), a[8]);
            p.named_properties.insert("mode1-amount".into(), a[5]);
        } else {
            p.native.fixed_parameter_16_16 = a[6].wrapping_shl(16);
            p.named_properties
                .insert("mode3-mask-resource".into(), a[4]);
        }
        self.sprites
            .get_mut(&(object as u32))
            .context("BGI sprite slot")?
            .bitmap = Some(a[3] as u32);
        self.graph.sort(object);
        self.set_graph_plan(
            object,
            GraphPlan {
                inputs: vec![GraphInput::Bitmap(a[3]), GraphInput::Bitmap(a[4])],
                quad: None,
                source_format: None,
                warp: None,
                compound: None,
                projection: None,
                operation: Some(operation),
                format: primary.format,
                force_opaque: mode == 1 && primary.format == 1,
                filters: Vec::new(),
            },
        );
        Ok(())
    }
    pub(super) fn configure_native_frame_background(
        &mut self,
        vm: &Machine,
        call: &NativeCall,
    ) -> Result<()> {
        let a = &call.args;
        let count = usize::try_from(a[0])?;
        ensure!(
            (2..=32).contains(&count),
            "BGI invalid background frame count"
        );
        let bytes = vm.memory(a[1] as u32, count * 4)?;
        let mut frames = Vec::with_capacity(count);
        for bytes in bytes.chunks_exact(4) {
            let id = i32::from_le_bytes(bytes.try_into().unwrap());
            let info = self.bitmap(id)?.1;
            ensure!(
                info.size == self.graph.screen && matches!(info.format, 1 | 2),
                "BGI background frame size/format"
            );
            frames.push((id, info.generation));
        }
        self.graph.filters.remove(&0);
        self.graph.images.remove(0);
        self.graph.transforms.remove(&0);
        let mut properties = Properties::default();
        let mut background = NativeBackgroundState::new(BackClass::BackD);
        background.active = self.graph.renderer_options[1];
        properties.background = Some(background);
        properties.native.priority = self.graph.default_priority as u32;
        properties.native.draw_enabled = u32::from(self.graph.renderer_options[0] != 0);
        properties.set_alpha_parameter(a[2]);
        properties
            .named_properties
            .insert("target-object-mode".into(), BackClass::BackD as i32);
        self.graph.objects.insert(0, properties);
        self.graph.tree.replace_kind(0, Kind::BackD);
        self.graph.frames.insert(0, frames);
        self.graph.background_args.insert(0, (0x44, a.clone()));
        self.graph.sort(0);
        self.rebuild_backdrop_plan(0)
    }
    pub(super) fn configure_native_backdrop(&mut self, call: &NativeCall) -> Result<()> {
        let a = &call.args;
        self.graph.frames.remove(&0);
        self.graph.filters.remove(&0);
        let class = BackClass::for_graph90_selector(call.id).context("BGI backdrop class")?;
        let primary = self.bitmap(a[0])?.1.clone();
        ensure!(matches!(primary.format, 1 | 2), "BGI backdrop pixel format");
        if !matches!(call.id, 0x48 | 0x49) {
            ensure!(
                primary.size == self.graph.screen,
                "BGI backdrop does not match native screen size"
            );
        }
        let previous = self
            .graph
            .objects
            .get(&0)
            .and_then(|p| p.background)
            .map(|b| b.class);
        if previous != Some(class) {
            self.graph.images.remove(0);
            self.graph.transforms.remove(&0);
            let mut p = Properties::default();
            let mut background = NativeBackgroundState::new(class);
            background.active = self.graph.renderer_options[1];
            p.background = Some(background);
            p.native.priority = self.graph.default_priority as u32;
            p.native.draw_enabled = u32::from(self.graph.renderer_options[0] != 0);
            self.graph.objects.insert(0, p);
        }
        self.graph.tree.replace_kind(0, class.display_kind());
        let p = self.graph.objects.entry(0).or_default();
        p.format_resource = Some(a[0]);
        p.named_properties
            .insert("target-object-mode".into(), class as i32);
        let alpha = match call.id {
            0x41 | 0x46 => Some(a[2]),
            0x45 => Some(a[3]),
            0x47 => Some(a[4]),
            0x4a => Some(a[3]),
            _ => None,
        };
        if let Some(alpha) = alpha {
            target_checks::effect_level(alpha)?;
            p.set_alpha_parameter(alpha);
        }
        match call.id {
            0x45 | 0x47 => {
                let field_ids = if call.id == 0x45 {
                    vec![a[1], a[2]]
                } else {
                    vec![a[1]]
                };
                let mut bindings = vec![(a[0], primary.generation)];
                for id in field_ids.into_iter().filter(|id| *id != -1) {
                    let b = self.bitmap(id)?.1;
                    let expected = if call.id == 0x45 { 4 } else { 6 };
                    ensure!(
                        b.format == expected
                            && b.size[0] >= self.graph.screen[0]
                            && b.size[1] >= self.graph.screen[1],
                        "BGI background vector field size/format"
                    );
                    ensure!(
                        self.graph.fields.contains_key(&(id as u32)),
                        "BGI background field resource missing"
                    );
                    bindings.push((id, b.generation));
                }
                if call.id == 0x47 {
                    ensure!(a[2] > 0, "BGI invalid ripple maximum distance");
                    self.graph
                        .waves
                        .get(&a[3])
                        .context("BGI wave slot absent")?
                        .signed_samples(a[2], 0, a[4])?;
                }
                self.graph.frames.insert(0, bindings);
            }
            0x42 => {
                let mut frames = Vec::with_capacity(4);
                for &id in &a[..4] {
                    let bitmap = self.bitmap(id)?.1;
                    ensure!(
                        bitmap.size == self.graph.screen && matches!(bitmap.format, 1 | 2),
                        "BGI four-cell backdrop bitmap size/format"
                    );
                    frames.push((id, bitmap.generation));
                }
                ensure!(
                    a[4] >= 0
                        && a[5] >= 0
                        && a[4] as u32 <= self.graph.screen[0]
                        && a[5] as u32 <= self.graph.screen[1],
                    "BGI invalid four-cell backdrop coordinates"
                );
                let background = self
                    .graph
                    .objects
                    .get_mut(&0)
                    .unwrap()
                    .background
                    .as_mut()
                    .unwrap();
                background.set_position(
                    a[4],
                    a[5],
                    self.graph.screen[0] as i32,
                    self.graph.screen[1] as i32,
                );
                self.graph.frames.insert(0, frames);
            }
            0x41 => {
                if !matches!(a[1], 0x7000 | 0x7001) {
                    let other = self.bitmap(a[1])?.1;
                    ensure!(
                        other.size == self.graph.screen && matches!(other.format, 1 | 2),
                        "BGI secondary backdrop size/format"
                    );
                }
            }
            0x46 => ensure!(matches!(a[1], 0 | 1), "BGI invalid gradient type"),
            0x48 => {
                ensure!(
                    a[3] as u32 >= 2 && a[4] as u32 >= 2,
                    "BGI invalid backdrop source extent"
                );
                let p = self.graph.objects.get_mut(&0).unwrap();
                if let Some(background) = p.background.as_mut() {
                    background.set_position(
                        a[1].wrapping_shl(16),
                        a[2].wrapping_shl(16),
                        self.graph.screen[0] as i32,
                        self.graph.screen[1] as i32,
                    );
                }
            }
            0x49 => ensure!(a[1] != 0, "BGI invalid backdrop scale"),
            0x4a => {
                ensure!(
                    a[2] == 0 && a[4] as u32 <= 1,
                    "BGI invalid mosaic style/coupling"
                );
                if !matches!(a[1], 0x7000 | 0x7001) {
                    let other = self.bitmap(a[1])?.1;
                    ensure!(other.size == self.graph.screen, "BGI secondary mosaic size");
                }
            }
            _ => (),
        }
        if call.id == 0x47 {
            let p = self.graph.objects.get_mut(&0).unwrap();
            p.named_properties
                .insert("target-auxiliary-resource".into(), a[1]);
            p.named_properties
                .insert("target-record-count".into(), a[2]);
            p.named_properties
                .insert("target-record-pointer".into(), a[3]);
        }
        self.graph.background_args.insert(0, (call.id, a.clone()));
        self.graph.sort(0);
        self.rebuild_backdrop_plan(0)?;
        Ok(())
    }
    pub(super) fn rebuild_backdrop_plan(&mut self, object: i32) -> Result<()> {
        if self
            .graph
            .background_args
            .get(&object)
            .is_some_and(|(id, _)| *id == 0x43)
        {
            return self.rebuild_native_backf(object);
        }
        let Some((selector, a)) = self.graph.background_args.get(&object).cloned() else {
            return Ok(());
        };
        let p = &self.graph.objects[&object];
        let alpha = p.alpha_parameter();
        let format = if selector == 0x44 {
            1
        } else {
            self.bitmap(a[0])?.1.format
        };
        let screen = self.graph.screen;
        let bitmap = |id| GraphInput::Bitmap(id);
        let pair = |operation, inputs| GraphPlan {
            inputs,
            quad: None,
            source_format: None,
            warp: None,
            compound: None,
            projection: None,
            operation: Some(operation),
            format,
            force_opaque: true,
            filters: Vec::new(),
        };
        if selector == 0x44 {
            let frames = self
                .graph
                .frames
                .get(&object)
                .context("BGI frame background binding")?;
            if frames.iter().any(|&(id, generation)| {
                self.bitmaps
                    .get(&(id as u32))
                    .is_none_or(|b| b.generation != generation)
            }) {
                self.graph.images.expired.insert(object);
                return Ok(());
            }
            if let Some(&(bitmap, _)) = frames.get(alpha as u32 as usize) {
                let format = self.bitmap(bitmap)?.1.format;
                self.graph.images.expired.remove(&object);
                let mut plan = GraphPlan::bitmap(bitmap, format);
                plan.force_opaque = true;
                self.set_graph_plan(object, plan);
            } else {
                self.graph.images.expired.insert(object);
            }
            return Ok(());
        }
        self.graph.images.expired.remove(&object);
        let plan = match selector {
            0x40 => GraphPlan::bitmap(a[0], format),
            0x42 => {
                let (x, y) = p
                    .background
                    .and_then(|b| b.integer_position())
                    .unwrap_or((a[4], a[5]));
                ensure!(
                    x >= 0 && y >= 0 && x as u32 <= screen[0] && y as u32 <= screen[1],
                    "BGI four-cell backdrop coordinates"
                );
                GraphPlan {
                    inputs: a[..4].iter().copied().map(GraphInput::Bitmap).collect(),
                    operation: None,
                    quad: Some([x as u32, y as u32, screen[0], screen[1]]),
                    source_format: None,
                    warp: None,
                    compound: None,
                    projection: None,
                    format,
                    force_opaque: format == 1,
                    filters: Vec::new(),
                }
            }

            0x41 => {
                let secondary = match a[1] {
                    0x7000 => GraphInput::Solid(screen, [0, 0, 0, 255]),
                    0x7001 => GraphInput::Solid(screen, [255; 4]),
                    id => bitmap(id),
                };
                let mode = match a[1] {
                    0x7000 => 0xc0,
                    0x7001 => 0xc1,
                    _ => 0xf0,
                };
                pair(
                    Raster::Blit {
                        position: [0; 2],
                        mode,
                        parameter: alpha.clamp(0, 256),
                    },
                    vec![secondary, bitmap(a[0])],
                )
            }
            0x45 | 0x47 => self.native_background_field_plan(object, selector, &a, format)?,
            0x46 => pair(
                Raster::Display {
                    spec: DisplayRaster::HorizontalBlur {
                        radius: (alpha as u32).min(255) as u8,
                        clamp: a[1] != 0,
                    },
                },
                vec![bitmap(a[0]), bitmap(a[0])],
            ),
            0x48 => {
                let (x, y) = p
                    .background
                    .and_then(|b| b.integer_position())
                    .unwrap_or((a[1].wrapping_shl(16), a[2].wrapping_shl(16)));
                let (tx, ty, tw, th) = p
                    .properties
                    .get(&0x102)
                    .map(|&(xy, wh)| {
                        (
                            (xy as i16) as i32,
                            ((xy >> 16) as i16) as i32,
                            (wh as u16) as i32,
                            (wh as u32 >> 16) as i32,
                        )
                    })
                    .unwrap_or((x >> 16, y >> 16, a[3], a[4]));
                let t = alpha as u32 as f64 / 256.;
                let w = a[3] as u32 as f64 + tw.wrapping_sub(a[3]) as f64 * t;
                let h = a[4] as u32 as f64 + th.wrapping_sub(a[4]) as f64 * t;
                let ox = (x as f64 + tx.wrapping_shl(16).wrapping_sub(x) as f64 * t) / 65536.;
                let oy = (y as f64 + ty.wrapping_shl(16).wrapping_sub(y) as f64 * t) / 65536.;
                pair(
                    Raster::Display {
                        spec: DisplayRaster::StretchBackdrop {
                            rect: [ox, oy, w, h],
                        },
                    },
                    vec![GraphInput::Solid(screen, [0; 4]), bitmap(a[0])],
                )
            }
            0x49 => {
                let (scale, angle) = p.properties.get(&0x101).copied().unwrap_or((a[1], a[2]));
                let (ds, da) = p.properties.get(&0x80).copied().unwrap_or_default();
                let alpha = alpha as u32;
                let mut scale = scale as u32;
                if ds != 0 && alpha != 0 {
                    let wave = (alpha as f64 * std::f64::consts::PI / 512.).sin() * 256.;
                    let sign = if ds < 0 { 1. } else { -1. };
                    let magnitude = ds.unsigned_abs() as f64;
                    let ratio = sign * (256. - wave) / 256.
                        + ds as f64 / (magnitude - (magnitude - 65536.) * wave / 256.);
                    scale = (ratio * 65536. + scale as f64) as i64 as u32;
                }
                let angle =
                    angle.wrapping_add(((da as u64).wrapping_mul(u64::from(alpha)) >> 8) as i32);
                pair(
                    Raster::Display {
                        spec: DisplayRaster::RotateBackdrop { scale, angle },
                    },
                    vec![GraphInput::Solid(screen, [0; 4]), bitmap(a[0])],
                )
            }
            0x4a => {
                let solid = matches!(a[1], 0x7000 | 0x7001).then_some(a[1] as u32);
                let secondary = if let Some(color) = solid {
                    GraphInput::Solid(
                        screen,
                        if color == 0x7001 {
                            [255; 4]
                        } else {
                            [0, 0, 0, 255]
                        },
                    )
                } else {
                    bitmap(a[1])
                };
                pair(
                    Raster::Display {
                        spec: DisplayRaster::Mosaic {
                            amount: alpha.clamp(0, 256),
                            secondary: solid,
                            coupled: a[4] == 1,
                        },
                    },
                    vec![bitmap(a[0]), secondary],
                )
            }
            _ => bail!("BGI backdrop selector {selector:02x} has no engine plan"),
        };
        self.set_graph_plan(object, plan);
        Ok(())
    }
    fn native_background_field_plan(
        &self,
        object: i32,
        selector: u16,
        a: &[i32],
        format: i32,
    ) -> Result<GraphPlan> {
        use crysirror_core::image_processing::FieldWarpSpec;
        let p = &self.graph.objects[&object];
        let spec = if selector == 0x45 {
            FieldWarpSpec::Displacement {
                first: self
                    .graph
                    .fields
                    .get(&(a[1] as u32))
                    .cloned()
                    .context("BGI first displacement field")?,
                second: if a[2] == -1 {
                    None
                } else {
                    Some(
                        self.graph
                            .fields
                            .get(&(a[2] as u32))
                            .cloned()
                            .context("BGI second displacement field")?,
                    )
                },
                amount: p.alpha_parameter().clamp(0, 256) as u16,
                bilinear: a[4] != 0,
            }
        } else {
            let offset = p.properties.get(&0x100).map_or(0, |v| v.1);
            let origin = p
                .background
                .and_then(|b| b.integer_position())
                .map_or([0; 2], |(x, y)| [x, y]);
            FieldWarpSpec::Wave {
                field: self
                    .graph
                    .fields
                    .get(&(a[1] as u32))
                    .cloned()
                    .context("BGI ripple field")?,
                samples: self
                    .graph
                    .waves
                    .get(&a[3])
                    .context("BGI wave slot absent")?
                    .signed_samples(a[2], offset, p.alpha_parameter())?,
                origin,
                wrap: false,
                bilinear: p.properties.get(&255).is_some_and(|v| v.0 != 0),
            }
        };
        spec.validate(self.client, self.graph.screen)?;
        Ok(GraphPlan {
            inputs: vec![GraphInput::Bitmap(a[0])],
            operation: None,
            quad: None,
            source_format: None,
            warp: Some((self.graph.screen, spec)),
            compound: None,
            projection: None,
            format,
            force_opaque: true,
            filters: Vec::new(),
        })
    }
}
