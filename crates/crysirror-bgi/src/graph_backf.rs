//! BackF native resource/origin policy expressed as a shared image graph.
use super::*;
use crysirror_core::image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput};
#[derive(Clone)]
pub(super) struct BackFPlan {
    size: [u32; 2],
    primary_size: [u32; 2],
    secondary_size: [u32; 2],
    primary_origin: [i32; 2],
    secondary_origin: [i32; 2],
    primary_format: i32,
    secondary_format: i32,
    secondary: i32,
    mask: Option<crysirror_image::stored_mask::StoredMaskBlend>,
    alpha: i32,
}
impl BackFPlan {
    pub fn input(&self, inputs: Vec<ImageInput>) -> Result<ImageInput> {
        ensure!(inputs.len() == 4, "BGI BackF image input count");
        let mut inputs = inputs.into_iter();
        let mut base = inputs.next().unwrap();
        let primary = inputs.next().unwrap();
        let secondary = inputs.next().unwrap();
        let mask = inputs.next().unwrap();
        match self.secondary {
            0x7000 | 0x7001 => {
                base = ImageInput::Solid {
                    size: self.size,
                    color: if self.secondary == 0x7001 {
                        [255; 4]
                    } else {
                        [0, 0, 0, 255]
                    },
                }
            }
            -1 | 0x7fff => (),
            _ => {
                base = ImageInput::Composite {
                    base: Box::new(base),
                    layers: vec![ImageCompositeLayer {
                        input: Box::new(secondary),
                        source: [
                            0,
                            0,
                            self.secondary_size[0] as i32,
                            self.secondary_size[1] as i32,
                        ],
                        destination: self.secondary_origin.map(i32::wrapping_neg),
                        clip: None,
                        opacity: 255,
                        math: ImageCompositeMath::IntegerStored {
                            destination_format: 1,
                            source_format: self.secondary_format as u8,
                            mode: 128,
                            parameter: 0,
                        },
                    }],
                }
            }
        }
        if let Some(spec) = &self.mask {
            // The original masked primary path is a defined no-op for ARGB;
            // its secondary/prefix result remains visible.
            if self.primary_format != 1 {
                return Ok(base);
            }
            return Ok(ImageInput::Process {
                input: Box::new(ImageInput::Ternary {
                    destination: Box::new(base),
                    first: Box::new(primary),
                    second: Box::new(mask),
                }),
                effects: serde_json::to_vec(&vec![
                    crysirror_image::effects::ImageEffect::StoredMaskBlend { spec: spec.clone() },
                ])?
                .into(),
            });
        }
        let mode = match self.secondary {
            0x7001 => 0xc1,
            0x7000 => 0xc0,
            -1 | 0x7fff => {
                if self.alpha == 0 {
                    128
                } else {
                    0xc0
                }
            }
            _ => 1,
        };
        Ok(ImageInput::Composite {
            base: Box::new(base),
            layers: vec![ImageCompositeLayer {
                input: Box::new(primary),
                source: [
                    0,
                    0,
                    self.primary_size[0] as i32,
                    self.primary_size[1] as i32,
                ],
                destination: self.primary_origin.map(i32::wrapping_neg),
                clip: None,
                opacity: 255,
                math: ImageCompositeMath::IntegerStored {
                    destination_format: 1,
                    source_format: self.primary_format as u8,
                    mode,
                    parameter: self.alpha,
                },
            }],
        })
    }
}
impl BgiVm {
    pub(super) fn configure_native_backf(&mut self, call: &NativeCall) -> Result<()> {
        use crate::native_background::NativeBackgroundSecondaryResource as Secondary;
        let a = &call.args;
        target_checks::effect_level(a[8])?;
        let primary = self.bitmap(a[2])?.1.clone();
        ensure!(
            matches!(primary.format, 1 | 2),
            "BGI BackF primary colour format"
        );
        let secondary = if matches!(a[5], -1 | 0x7000 | 0x7001 | 0x7fff) {
            None
        } else {
            let b = self.bitmap(a[5])?.1.clone();
            ensure!(
                matches!(b.format, 1 | 2),
                "BGI BackF secondary colour format"
            );
            Some(b)
        };
        let previous = self
            .graph
            .objects
            .get(&0)
            .and_then(|p| p.background)
            .map(|b| b.class);
        if previous != Some(BackClass::BackF) {
            self.graph.images.remove(0);
            self.graph.transforms.remove(&0);
            let mut p = Properties::default();
            let mut b = NativeBackgroundState::new(BackClass::BackF);
            b.active = self.graph.renderer_options[1];
            p.background = Some(b);
            p.native.priority = self.graph.default_priority as u32;
            p.native.draw_enabled = u32::from(self.graph.renderer_options[0] != 0);
            p.blend_mode = 1;
            p.native.blend_mode = 1;
            self.graph.objects.insert(0, p);
        }
        self.graph.tree.replace_kind(0, Kind::BackF);
        let mut bindings = vec![(a[2], primary.generation)];
        if let Some(b) = &secondary {
            bindings.push((a[5], b.generation));
        }
        let p = self.graph.objects.entry(0).or_default();
        p.format_resource = Some(a[2]);
        p.named_properties
            .insert("target-object-mode".into(), BackClass::BackF as i32);
        let b = p.background.as_mut().unwrap();
        b.set_position(
            a[0],
            a[1],
            self.graph.screen[0] as i32,
            self.graph.screen[1] as i32,
        );
        b.resource_binding = Some((a[2], primary.generation));
        b.secondary_resource_binding = Some(secondary.as_ref().map_or(
            Secondary::Sentinel(a[5]),
            |b| Secondary::Bitmap {
                handle: a[5],
                generation: b.generation,
            },
        ));
        let backf = b.backf.as_mut().unwrap();
        backf.secondary_x = a[3];
        backf.secondary_y = a[4];
        // Native construction commits primary and secondary before validating
        // the optional grayscale mask, and writes alpha only after it succeeds.
        let mask = if a[6] == -1 {
            None
        } else {
            let b = self.bitmap(a[6])?.1;
            ensure!(b.format == 3, "BGI BackF mask is not grayscale");
            Some((a[6], b.generation))
        };
        let p = self.graph.objects.entry(0).or_default();
        let backf = p.background.as_mut().unwrap().backf.as_mut().unwrap();
        backf.mask_resource_binding = mask;
        if mask.is_some() {
            backf.mask_parameter = a[7];
        }
        p.set_alpha_parameter(a[8]);
        if let Some(mask) = mask {
            bindings.push(mask);
        }
        self.graph.frames.insert(0, bindings);
        self.graph.background_args.insert(0, (0x43, a.clone()));
        self.graph.filters.entry(0).or_default();
        self.graph.sort(0);
        self.rebuild_native_backf(0)
    }
    pub(super) fn rebuild_native_backf(&mut self, object: i32) -> Result<()> {
        let a = &self
            .graph
            .background_args
            .get(&object)
            .context("BGI BackF arguments")?
            .1;
        let p = &self.graph.objects[&object];
        let b = p.background.context("BGI BackF class")?;
        let f = b.backf.context("BGI BackF state")?;
        let primary = self.bitmap(a[2])?.1;
        let secondary = b
            .secondary_resource_binding
            .context("BGI BackF secondary binding")?;
        let placement = self.native_filter_origin(object)?;
        let primary_origin = b.integer_position().map_or([0; 2], |(x, y)| [x, y]);
        let primary_origin = [
            primary_origin[0].wrapping_sub(placement[0]),
            primary_origin[1].wrapping_sub(placement[1]),
        ];
        let alpha = p.transparency_parameter();
        let control = if f.mask_control_enabled == 0 {
            0
        } else {
            let fixed = (p.native.fixed_parameter_16_16 as u32 >> 16) as i32;
            match f.mask_control_mode {
                1 => 256 - alpha,
                2 => fixed,
                3 => 256 - fixed,
                _ => alpha,
            }
        };
        let mask = f
            .mask_resource_binding
            .map(|_| crysirror_image::stored_mask::StoredMaskBlend {
                source_origin: primary_origin,
                mask_origin: placement.map(i32::wrapping_neg),
                parameter: f.mask_parameter,
                transparency: alpha as u16,
                control: control.clamp(0, 256) as u16,
                mask_channel: 0,
            });
        let (secondary_id, secondary_size, secondary_format, secondary_input) = match secondary {
            crate::native_background::NativeBackgroundSecondaryResource::Bitmap {
                handle, ..
            } => {
                let b = self.bitmap(handle)?.1;
                (handle, b.size, b.format, GraphInput::Bitmap(handle))
            }
            crate::native_background::NativeBackgroundSecondaryResource::Sentinel(id) => (
                id,
                self.graph.screen,
                1,
                GraphInput::Solid(self.graph.screen, [0; 4]),
            ),
        };
        let compound = BackFPlan {
            size: self.graph.screen,
            primary_size: primary.size,
            secondary_size,
            primary_origin,
            secondary_origin: [
                f.secondary_x.wrapping_sub(placement[0]),
                f.secondary_y.wrapping_sub(placement[1]),
            ],
            primary_format: primary.format,
            secondary_format,
            secondary: secondary_id,
            mask,
            alpha,
        };
        let cut = [
            p.native.priority as i32,
            self.graph
                .tree
                .chain_position(object)
                .context("BGI BackF order")? as i32,
        ];
        let inputs = vec![
            GraphInput::Prefix(self.graph.screen, cut),
            GraphInput::Bitmap(a[2]),
            secondary_input,
            f.mask_resource_binding
                .map_or(GraphInput::Solid(self.graph.screen, [0; 4]), |(id, _)| {
                    GraphInput::Bitmap(id)
                }),
        ];
        self.set_graph_plan(
            object,
            GraphPlan {
                inputs,
                operation: None,
                quad: None,
                source_format: None,
                warp: None,
                compound: Some(compound),
                projection: None,
                format: 1,
                force_opaque: true,
                filters: Vec::new(),
            },
        );
        let signature = self.native_filter_signature(object)?;
        self.graph.filters.get_mut(&object).unwrap().signature = Some(signature);
        Ok(())
    }
}
