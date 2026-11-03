//! Native field handles/wave slots; numerical work lives in crysirror-image.
use super::*;
use crysirror_core::image_processing::{VectorFieldResource, VectorFieldSamples};
use crysirror_image::vector_field::{self, FieldPattern, WaveTable};
pub(super) enum FieldWork {
    Map {
        bitmap: u32,
        generation: u64,
        task: Task<Arc<VectorFieldResource>>,
    },
    Wave {
        slot: i32,
        task: Task<WaveTable>,
    },
}
pub(super) fn handles(group: u8, id: u16) -> bool {
    matches!(
        (group, id),
        (0x91, 0x10..=0x17) | (0x92, 0x00 | 0x01 | 0x10 | 0x11)
    )
}
impl BgiVm {
    pub(crate) fn graph_field_bitmap(&self, bitmap: u32) -> Option<Arc<VectorFieldResource>> {
        self.graph.fields.get(&bitmap).cloned()
    }
    pub(crate) fn register_graph_field_bitmap(
        &mut self,
        bitmap: u32,
        size: [u32; 2],
        format: i32,
    ) -> Result<()> {
        if matches!(format, 4 | 6) {
            self.graph
                .fields
                .insert(bitmap, vector_field::blank(self.client, size, format)?);
        } else {
            self.graph.fields.remove(&bitmap);
        }
        self.invalidate_graph_bitmap(bitmap);
        Ok(())
    }
    pub(crate) fn clear_graph_field_bitmap(&mut self, bitmap: u32) -> Result<bool> {
        let Some(field) = self.graph.fields.get(&bitmap) else {
            return Ok(false);
        };
        let format = match field.samples {
            VectorFieldSamples::Displacement(_) => 4,
            VectorFieldSamples::DirectionDistance(_) => 6,
        };
        let field = vector_field::blank(self.client, field.size, format)?;
        self.graph.fields.insert(bitmap, field);
        self.invalidate_graph_bitmap(bitmap);
        Ok(true)
    }
    pub(crate) fn release_graph_field_bitmap(&mut self, bitmap: u32) {
        self.graph.fields.remove(&bitmap);
        self.invalidate_graph_bitmap(bitmap);
    }
    pub(crate) fn clone_graph_field_bitmap(
        &mut self,
        destination: u32,
        source: u32,
    ) -> Result<bool> {
        let Some(field) = self.graph.fields.get(&source).cloned() else {
            self.graph.fields.remove(&destination);
            return Ok(false);
        };
        self.graph.fields.insert(destination, field);
        self.invalidate_graph_bitmap(destination);
        Ok(true)
    }
    pub(crate) fn replace_graph_field_bitmap(
        &mut self,
        bitmap: u32,
        field: Arc<VectorFieldResource>,
    ) -> Result<()> {
        field.validate(self.client)?;
        let (_, b) = self.bitmap(bitmap as i32)?;
        ensure!(
            b.size == field.size
                && matches!(
                    (&field.samples, b.format),
                    (VectorFieldSamples::Displacement(_), 4)
                        | (VectorFieldSamples::DirectionDistance(_), 6)
                ),
            "BGI replacement field layout"
        );
        self.graph.fields.insert(bitmap, field);
        self.invalidate_graph_bitmap(bitmap);
        Ok(())
    }
    pub(super) fn native_field_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        let request = vm.request_id().context("BGI field request")?;
        let a = &call.args;
        if !self.graph.field_jobs.contains_key(&request) {
            let work = if call.group == 0x92 && matches!(call.id, 0 | 1) {
                let slot = a[0];
                ensure!((0..8).contains(&slot), "BGI invalid wave slot");
                let args = a.clone();
                let id = call.id;
                FieldWork::Wave {
                    slot,
                    task: self.pool.spawn(move |cancel| {
                        cancel.check()?;
                        let wave = if id == 0 {
                            WaveTable::compact(args[1], args[2], args[3], args[4])?
                        } else {
                            WaveTable::extended(
                                args[1], args[2], args[3], args[4], args[5], args[6],
                            )?
                        };
                        cancel.check()?;
                        Ok(wave)
                    })?,
                }
            } else {
                let (bitmap, b) = self.bitmap(a[0])?;
                let generation = b.generation;
                let expected = if call.group == 0x91 { 4 } else { 6 };
                ensure!(b.format == expected, "BGI vector field bitmap format");
                let field = self
                    .graph
                    .fields
                    .get(&bitmap)
                    .cloned()
                    .context("BGI field resource not allocated")?;
                let pattern = if call.group == 0x91 {
                    match call.id {
                        0x10 => ensure!(a[3] != 0 && a[4] != 0, "BGI invalid field stretch extent"),
                        0x12 => ensure!(a[3] != 0, "BGI zero radial field period"),
                        0x14 | 0x15 => {
                            ensure!(a[3] != 0 && a[4] != 0, "BGI invalid field curvature/radius")
                        }
                        _ => (),
                    }
                    FieldPattern::Displacement {
                        kind: call.id as u8,
                        parameters: a[1..].to_vec(),
                    }
                } else if call.id == 0x10 {
                    ensure!((0..2).contains(&a[1]), "BGI radial vector mode");
                    FieldPattern::Radial {
                        mode: a[1],
                        center: [a[2], a[3]],
                        period: a[4],
                    }
                } else {
                    ensure!((0..4).contains(&a[1]), "BGI axis vector mode");
                    FieldPattern::Axis { mode: a[1] }
                };
                FieldWork::Map {
                    bitmap,
                    generation,
                    task: self.pool.spawn(move |cancel| {
                        vector_field::generate(&field, &pattern, &|| cancel.check())
                    })?,
                }
            };
            self.graph.field_jobs.insert(request, work);
        }
        let work = self.graph.field_jobs.remove(&request).unwrap();
        match work {
            FieldWork::Map {
                bitmap,
                generation,
                mut task,
            } => {
                let Some(field) = task.try_take()? else {
                    self.graph.field_jobs.insert(
                        request,
                        FieldWork::Map {
                            bitmap,
                            generation,
                            task,
                        },
                    );
                    return Ok(false);
                };
                ensure!(
                    self.bitmaps
                        .get(&bitmap)
                        .is_some_and(|b| b.generation == generation),
                    "BGI vector allocation changed during generation"
                );
                self.replace_graph_field_bitmap(bitmap, field)?;
            }
            FieldWork::Wave { slot, mut task } => {
                let Some(wave) = task.try_take()? else {
                    self.graph
                        .field_jobs
                        .insert(request, FieldWork::Wave { slot, task });
                    return Ok(false);
                };
                self.graph.waves.insert(slot, wave);
                let objects = self
                    .graph
                    .background_args
                    .iter()
                    .filter_map(|(&id, (selector, _))| (*selector == 0x47).then_some(id))
                    .collect::<Vec<_>>();
                self.graph.field_rebuild.extend(objects);
                self.graph
                    .field_rebuild
                    .extend(self.graph.sprite_wave_args.keys().copied());
            }
        }
        self.graph_tick(host)?;
        vm.commit_call(&[])?;
        self.calls += 1;
        Ok(true)
    }
}

impl BgiVm {
    pub(super) fn configure_native_sprite_wave(&mut self, call: &NativeCall) -> Result<()> {
        let a = &call.args;
        let object = a[0];
        self.graph.require(object)?;
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Sprite),
            "BGI invalid sprite handle"
        );
        target_checks::bitmap(a[3])?;
        target_checks::bitmap(a[4])?;
        target_checks::transparency(a[8])?;
        target_checks::priority(a[9])?;
        let source = self.bitmap(a[3])?.1;
        let vector = self.bitmap(a[4])?.1;
        ensure!(
            matches!(source.format, 1 | 2) && vector.format == 6 && source.size == vector.size,
            "BGI sprite direction/distance map size/format"
        );
        let bindings = vec![(a[3], source.generation), (a[4], vector.generation)];
        ensure!(a[5] > 0, "BGI sprite maximum wave distance");
        self.graph
            .waves
            .get(&a[6])
            .context("BGI sprite wave slot absent")?
            .signed_samples(a[5], 0, a[7])?;
        self.graph.projections.remove(&object);
        self.graph.sprite_affine.remove(&object);
        self.graph.raster_offsets.remove(&object);
        self.graph.images.remove(object);
        self.graph.transforms.remove(&object);
        self.graph.frames.insert(object, bindings);
        self.graph.sprite_wave_args.insert(object, a.clone());
        self.graph.position(object, a[1], a[2]);
        let p = self.graph.objects.entry(object).or_default();
        p.format_resource = Some(a[3]);
        p.blend_mode = 1;
        p.native.blend_mode = 1;
        p.native.priority = a[9] as u32;
        p.set_alpha_parameter(a[7]);
        p.mask_alpha = a[8];
        p.native.mask_alpha = a[8];
        p.named_properties.insert("target-object-mode".into(), 4);
        p.named_properties
            .insert("mode4-effect-resource".into(), a[4]);
        p.named_properties.insert("mode4-record-count".into(), a[5]);
        p.named_properties
            .insert("mode4-record-pointer".into(), a[6]);
        self.sprites
            .get_mut(&(object as u32))
            .context("BGI sprite slot")?
            .bitmap = Some(a[3] as u32);
        self.graph.sort(object);
        self.rebuild_native_sprite_wave(object)
    }
    pub(super) fn rebuild_native_sprite_wave(&mut self, object: i32) -> Result<()> {
        use crysirror_core::image_processing::FieldWarpSpec;
        let a = self
            .graph
            .sprite_wave_args
            .get(&object)
            .context("BGI sprite wave binding")?;
        let p = &self.graph.objects[&object];
        let b = self.bitmap(a[3])?.1;
        let field = self
            .graph
            .fields
            .get(&(a[4] as u32))
            .cloned()
            .context("BGI sprite vector field")?;
        let offset = p.properties.get(&0x100).map_or(0, |v| v.1);
        let samples = self
            .graph
            .waves
            .get(&a[6])
            .context("BGI sprite wave slot")?
            .signed_samples(a[5], offset, p.alpha_parameter())?;
        let spec = FieldWarpSpec::Wave {
            field,
            samples,
            origin: [0; 2],
            wrap: true,
            bilinear: false,
        };
        spec.validate(self.client, b.size)?;
        let plan = GraphPlan {
            inputs: vec![GraphInput::Bitmap(a[3])],
            operation: None,
            quad: None,
            source_format: None,
            warp: Some((b.size, spec)),
            compound: None,
            projection: None,
            format: b.format,
            force_opaque: b.format == 1,
            filters: Vec::new(),
        };
        self.set_graph_plan(object, plan);
        Ok(())
    }
    pub(super) fn graph_wave_window(&mut self, object: i32, alpha: i32, offset: i32) -> Result<()> {
        let p = self.graph.objects.get(&object).context("BGI wave object")?;
        let back = p.background.is_some_and(|b| b.class == BackClass::BackRpl);
        let args = if back {
            self.graph.background_args.get(&object).map(|v| &v.1)
        } else {
            self.graph.sprite_wave_args.get(&object)
        };
        let Some(args) = args else {
            return Ok(());
        };
        let (count, slot) = if back {
            (args[2], args[3])
        } else {
            (args[5], args[6])
        };
        let alpha = if (alpha as u32) <= 256 {
            alpha
        } else {
            p.alpha_parameter()
        };
        self.graph
            .waves
            .get(&slot)
            .context("BGI invalid wave slot (254)")?
            .signed_samples(count, offset, alpha)?;
        let p = self.graph.objects.entry(object).or_default();
        p.set_alpha_parameter(alpha);
        p.properties.insert(0x100, (alpha, offset));
        if back {
            self.rebuild_backdrop_plan(object)
        } else {
            self.rebuild_native_sprite_wave(object)
        }
    }
}
