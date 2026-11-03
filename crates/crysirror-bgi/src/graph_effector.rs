//! Native effector policy. Every sampler and byte blend runs in engine workers.
use super::*;
use crysirror_core::image_processing::{FieldWarpSpec, ImageResource};
pub(super) struct NativeEffector {
    pub mode: i32,
    maps: [i32; 2],
    parameters: [i32; 9],
    bindings: Vec<(i32, u64)>,
    pub transform: [i32; 5],
    waves: Arc<[i16]>,
    pub feedback: Option<Arc<dyn ImageResource>>,
    pub feedback_at: Option<u64>,
}
impl NativeEffector {
    pub fn bitmap_dependencies(&self) -> impl Iterator<Item = u32> + '_ {
        self.bindings.iter().map(|(bitmap, _)| *bitmap as u32)
    }
    pub fn uses_bitmap(&self, id: u32) -> bool {
        self.bindings.iter().any(|(bitmap, _)| *bitmap as u32 == id)
    }
    pub fn descriptor(&self) -> Value {
        json!({"mode":self.mode,"maps":self.maps,"parameters":self.parameters,"bindings":self.bindings,"transform":self.transform})
    }
}
pub(super) fn handles(group: u8, id: u16) -> bool {
    group == 0x91 && matches!(id, 0x60 | 0x61 | 0x64..=0x69)
}
impl BgiVm {
    pub(super) fn native_effector_call(
        &mut self,
        host: &mut dyn HostServices,
        call: &NativeCall,
    ) -> Result<Option<i32>> {
        let a = &call.args;
        if call.id == 0x60 {
            let id = self.graph.allocate(0x91000000, 8, Kind::Effector);
            ensure!(id != 0, "BGI effector pool exhausted");
            return Ok(Some(id));
        }
        let object = a[0];
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Effector),
            "BGI invalid effector handle"
        );
        if call.id == 0x61 {
            if let Some(parent) = self.graph.tree.parent(object) {
                if let Some(members) = self.graph.members.get_mut(&parent) {
                    members.remove(&object);
                }
            }
            self.hide_graph_object(host, object)?;
            self.graph.images.remove(object);
            self.graph.filters.remove(&object);
            self.graph.fixed_vectors.remove(&object);
            self.graph.raster_offsets.remove(&object);
            self.graph.tree.remove(object);
            self.graph.objects.remove(&object);
            return Ok(None);
        }
        if call.id == 0x64 {
            let p = self.graph.objects.entry(object).or_default();
            p.native.draw_enabled = u32::from(a[1] != 0);
            p.named_properties
                .insert("effector-draw-explicit".into(), 1);
            return Ok(None);
        }
        let mode = i32::from(call.id - 0x65);
        let priority = *a.last().unwrap();
        let alpha = a[a.len() - 2];
        target_checks::priority(priority)?;
        target_checks::effect_level(alpha)?;
        let maps = match call.id {
            0x65 => [a[1], a[2]],
            0x67 => [a[1], -1],
            _ => [-1; 2],
        };
        let mut bindings = Vec::new();
        for (index, bitmap) in maps.into_iter().enumerate() {
            if bitmap == -1 && (index == 1 || !matches!(call.id, 0x65 | 0x67)) {
                continue;
            }
            let b = self.bitmap(bitmap)?.1;
            let format = if mode == 2 { 6 } else { 4 };
            ensure!(
                b.format == format
                    && b.size[0] >= self.graph.screen[0]
                    && b.size[1] >= self.graph.screen[1]
                    && (mode != 2 || b.size == self.graph.screen),
                "BGI effector field size/format"
            );
            ensure!(
                self.graph.fields.contains_key(&(bitmap as u32)),
                "BGI effector field resource missing"
            );
            bindings.push((bitmap, b.generation));
        }
        if call.id == 0x66 {
            ensure!((0..2).contains(&a[1]), "BGI invalid effector blur type");
        }
        if call.id == 0x68 {
            ensure!(a[4] != 0 && a[5] != 0, "BGI invalid effector scale");
        }
        let waves = if mode == 2 {
            ensure!(a[2] != 0, "BGI zero effector wave distance");
            self.graph
                .waves
                .get(&a[3])
                .context("BGI effector wave slot")?
                .signed_samples(a[2], 0, alpha)?
        } else {
            Arc::from([])
        };
        let parameters = match call.id {
            0x65 => [a[3], a[4], 0, 0, 0, 0, 0, 0, 0],
            0x66 => [a[1], a[2], 0, 0, 0, 0, 0, 0, 0],
            0x67 => [a[2], a[3], a[4], 0, 0, 0, 0, 0, 0],
            0x68 => [a[1], a[2], a[3], a[4], a[5], a[6], a[7], 0, 0],
            _ => [alpha, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let p = self.graph.objects.entry(object).or_default();
        p.native.priority = priority as u32;
        if !p.named_properties.contains_key("effector-draw-explicit") {
            p.native.draw_enabled = 1;
        }
        p.set_alpha_parameter(alpha);
        if mode == 3 {
            for id in [0x80, 0x81, 0x82, 0x8f] {
                p.properties.remove(&id);
            }
            p.native.fixed_parameter_16_16 = 0;
        }
        self.graph.filters.insert(
            object,
            graph_filter::NativeFilter {
                effector: Some(NativeEffector {
                    mode,
                    maps,
                    parameters,
                    bindings,
                    transform: [0, 0, 0, 65536, 65536],
                    waves,
                    feedback: None,
                    feedback_at: Some(host.now_ms()),
                }),
                ..Default::default()
            },
        );
        self.sync_native_effector(object)?;
        self.graph.sort(object);
        self.publish_graph(host)?;
        self.rebuild_native_effector_plan(object)?;
        Ok(None)
    }
    pub(super) fn sync_native_effector(&mut self, object: i32) -> Result<()> {
        let Some(f) = self.graph.filters.get(&object) else {
            return Ok(());
        };
        let Some(e) = &f.effector else {
            return Ok(());
        };
        let p = &self.graph.objects[&object];
        let params = e.parameters;
        let (transform, waves) = if e.mode == 3 {
            let progress = p.native.fixed_parameter_16_16;
            let pair = |id| p.properties.get(&id).copied().unwrap_or_default();
            let curve = effector_curve(pair(0x8f).0, progress)?;
            let alpha = p.alpha_parameter() as u32;
            let delta = |value: i32| ((i64::from(value) * i64::from(progress as u32)) >> 24) as i32;
            let scale = |base: i32, coefficient: i32| {
                let change = (i128::from(coefficient) * i128::from(progress as u32)) >> 24;
                ((((i128::from(base as u32) + change - 65536) * i128::from(alpha)) >> 8) as i32)
                    .wrapping_add(65536)
            };
            let angle = ((i128::from(params[2])
                + ((i128::from(pair(0x81).0) * i128::from(curve)) >> 16))
                * i128::from(alpha)
                >> 8) as i32;
            (
                Some([
                    params[0].wrapping_add(delta(pair(0x80).0)),
                    params[1].wrapping_add(delta(pair(0x80).1)),
                    angle,
                    scale(params[3], pair(0x82).0),
                    scale(params[4], pair(0x82).1),
                ]),
                None,
            )
        } else if e.mode == 2 {
            let offset = p.properties.get(&0x100).map_or(0, |v| v.1);
            (
                None,
                Some(
                    self.graph
                        .waves
                        .get(&params[1])
                        .context("BGI effector wave slot")?
                        .signed_samples(params[0], offset, p.alpha_parameter())?,
                ),
            )
        } else {
            (None, None)
        };
        let e = self
            .graph
            .filters
            .get_mut(&object)
            .unwrap()
            .effector
            .as_mut()
            .unwrap();
        if let Some(transform) = transform {
            e.transform = transform;
        }
        if let Some(waves) = waves {
            e.waves = waves;
        }
        self.graph.filters.get_mut(&object).unwrap().signature = None;
        Ok(())
    }
    pub(super) fn rebuild_native_effector_plan(&mut self, object: i32) -> Result<()> {
        let f = &self.graph.filters[&object];
        let e = f.effector.as_ref().context("BGI effector state")?;
        let p = &self.graph.objects[&object];
        let signature = self.native_filter_signature(object)?;
        let alpha = p.transparency_parameter();
        let size = self.graph.screen;
        let cut = [
            p.native.priority as i32,
            self.graph
                .tree
                .chain_position(object)
                .context("BGI effector order")? as i32,
        ];
        let mut plan = GraphPlan {
            inputs: vec![GraphInput::Prefix(size, cut)],
            operation: None,
            quad: None,
            source_format: None,
            warp: None,
            compound: None,
            projection: None,
            format: 1,
            force_opaque: true,
            filters: Vec::new(),
        };
        // Native allocation checks fail closed to an identity prefix; a
        // recycled bitmap number must never bind the replacement field.
        let current = e.bindings.iter().all(|(id, g)| {
            self.bitmaps
                .get(&(*id as u32))
                .is_some_and(|b| b.generation == *g)
        });
        let filter = p.properties.get(&255).map(|v| v.0);
        if current {
            match e.mode {
                0 => {
                    plan.warp = Some((
                        size,
                        FieldWarpSpec::Displacement {
                            first: self
                                .graph
                                .fields
                                .get(&(e.maps[0] as u32))
                                .cloned()
                                .context("BGI effector displacement")?,
                            second: if e.maps[1] == -1 {
                                None
                            } else {
                                Some(
                                    self.graph
                                        .fields
                                        .get(&(e.maps[1] as u32))
                                        .cloned()
                                        .context("BGI effector displacement")?,
                                )
                            },
                            amount: alpha as u16,
                            bilinear: filter.unwrap_or(e.parameters[1]) != 0,
                        },
                    ))
                }
                1 => {
                    plan.operation = Some(Raster::Display {
                        spec: DisplayRaster::HorizontalBlur {
                            radius: (alpha as u32).min(255) as u8,
                            clamp: e.parameters[0] != 0,
                        },
                    })
                }
                2 => {
                    plan.warp = Some((
                        size,
                        FieldWarpSpec::Wave {
                            field: self
                                .graph
                                .fields
                                .get(&(e.maps[0] as u32))
                                .cloned()
                                .context("BGI effector vector")?,
                            samples: e.waves.clone(),
                            origin: [0; 2],
                            wrap: false,
                            bilinear: filter.unwrap_or(0) != 0,
                        },
                    ))
                }
                3 => {
                    plan.operation = Some(Raster::Display {
                        spec: DisplayRaster::InverseAffine {
                            destination: [
                                ((size[0] & !1) << 15) as i32,
                                ((size[1] & !1) << 15) as i32,
                            ],
                            transform: e.transform,
                            bilinear: filter.unwrap_or(e.parameters[5]) != 0,
                        },
                    })
                }
                4 => {
                    plan.inputs.push(
                        e.feedback
                            .as_ref()
                            .map_or(GraphInput::Solid(size, [0; 4]), |image| {
                                GraphInput::Frozen(image.clone())
                            }),
                    );
                    plan.operation = Some(Raster::Blit {
                        position: [0; 2],
                        mode: 1,
                        parameter: 256 - alpha,
                    });
                }
                _ => bail!("BGI invalid effector mode"),
            }
        }
        self.set_graph_plan(object, plan);
        self.graph.filters.get_mut(&object).unwrap().signature = Some(signature);
        Ok(())
    }
}
fn effector_curve(curve: i32, progress: i32) -> Result<i32> {
    crysirror_image::progress_curve::sample_fixed_progress(fixed_curve_shape(curve), progress)
}
pub(super) fn fixed_curve_shape(curve: i32) -> crysirror_image::progress_curve::FixedProgressShape {
    use crysirror_image::progress_curve::FixedProgressShape as Shape;
    match curve {
        1 => Shape::HalfCosine,
        2 => Shape::SineOut,
        3 => Shape::SineIn,
        4..=15 => {
            let power = match curve {
                4 | 5 => 2.,
                6 | 7 => 2.5,
                8 | 9 => 3.,
                10 | 11 => 4.,
                12 | 13 => 5.,
                14 | 15 => 6.,
                _ => unreachable!(),
            };
            if curve & 1 == 0 {
                Shape::PowerIn(power)
            } else {
                Shape::PowerOut(power)
            }
        }
        _ => Shape::Linear,
    }
}

impl BgiVm {
    pub(super) fn graph_effector_property(
        &mut self,
        object: i32,
        property: u32,
        value: i32,
        extra: i32,
    ) -> Result<()> {
        let e = self
            .graph
            .filters
            .get(&object)
            .and_then(|f| f.effector.as_ref())
            .context("BGI effector property target")?;
        if matches!(property, 0x80 | 0x81 | 0x82) && e.mode != 3 {
            return Ok(());
        }
        if property == 0x100 {
            if e.mode != 2 {
                return Ok(());
            }
            let p = self
                .graph
                .objects
                .get(&object)
                .context("BGI effector object")?;
            let alpha = if (value as u32) > 256 {
                p.alpha_parameter()
            } else {
                value
            };
            self.graph
                .waves
                .get(&e.parameters[1])
                .context("BGI effector wave slot (254)")?
                .signed_samples(e.parameters[0], extra, alpha)?;
            let p = self.graph.objects.entry(object).or_default();
            p.properties.insert(property, (alpha, extra));
            p.set_alpha_parameter(alpha);
            self.sync_native_effector(object)?;
        } else {
            self.graph
                .objects
                .entry(object)
                .or_default()
                .properties
                .insert(property, (value, extra));
            if property == 255 {
                self.graph.filters.get_mut(&object).unwrap().signature = None;
            }
            // Coefficients intentionally become visible only after SetAlpha or
            // the fixed-parameter virtual; merely storing them never rebuilds.
        }
        Ok(())
    }
}
