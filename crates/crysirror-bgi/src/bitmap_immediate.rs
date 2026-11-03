//! Immediate native bitmap policy over CrySirror image graph operations.
use super::*;
use crysirror_core::image_processing::FieldWarpSpec;
use crysirror_image::integer_raster::display::DisplayRaster;
impl BgiVm {
    pub(super) fn alpha_plane_plan(
        &self,
        host: &mut dyn HostServices,
        call: &NativeCall,
    ) -> Result<Option<BitmapWork>> {
        let a = &call.args;
        target_checks::bitmap(a[0])?;
        let primary = self.bitmap(a[3])?.1;
        if a[4] != -1 {
            ensure!(
                self.bitmap(a[4])?.1.format == primary.format,
                "BGI alpha pair formats differ"
            );
        }
        let size = self.graph.screen;
        let phase = if primary.format == 2 {
            Phase::Auxiliary {
                inputs: vec![
                    self.prepare_bitmap(host, a[3])?,
                    self.prepare_bitmap(host, if a[4] == -1 { a[3] } else { a[4] })?,
                ],
                operation: AuxiliaryOperation::AlphaPlanes {
                    size,
                    spec: crysirror_image::alpha_planes::AlphaPlanePair {
                        source_origin: [a[1].wrapping_neg(), a[2].wrapping_neg()],
                        channel: 3,
                        paired: a[4] != -1,
                        amount: a[5] as u16,
                    },
                },
            }
        } else {
            Phase::Process(host.image_metadata(self.client)?.begin_process(
                ImageInput::Solid {
                    size,
                    color: [0, 0, 0, 255],
                },
                serde_json::to_vec(&Vec::<ImageEffect>::new())?.into(),
            )?)
        };
        Ok(Some(BitmapWork {
            reallocate: true,
            outputs: Vec::new(),
            destination: a[0] as u32,
            format: 3,
            auxiliary: [-1; 2],
            phase,
        }))
    }
    pub(super) fn immediate_bitmap_plan(
        &self,
        host: &mut dyn HostServices,
        call: &NativeCall,
    ) -> Result<Option<BitmapWork>> {
        let a = &call.args;
        let target = self.bitmap(a[0])?.1;
        let source_id = match call.id {
            0x19 => a[3],
            0x1c => a[5],
            _ => a[1],
        };
        let source = self.bitmap(source_id)?.1;
        if call.id == 0x19 {
            target_checks::bitmap(a[0])?;
            target_checks::bitmap(source_id)?;
            target_checks::effect_level(a[6])?;
        }
        ensure!(
            target.format == source.format,
            "BGI immediate bitmap formats differ"
        );
        let auxiliary = match call.id {
            0x19 => {
                let mask = self.bitmaps.get(&(a[4] as u32));
                if let Some(mask) = mask {
                    ensure!(
                        mask.format == 3 && mask.size == source.size,
                        "BGI crossfade coverage format/size"
                    );
                }
                if !matches!(target.format, 1 | 2) || mask.is_some() && target.format != 1 {
                    return Ok(None);
                }
                if mask.is_some() {
                    Some((
                        vec![
                            self.prepare_bitmap(host, a[0])?,
                            self.prepare_bitmap(host, source_id)?,
                            self.prepare_bitmap(host, a[4])?,
                        ],
                        AuxiliaryOperation::Mask(crysirror_image::stored_mask::StoredMaskBlend {
                            source_origin: [a[1].wrapping_neg(), a[2].wrapping_neg()],
                            mask_origin: [a[1].wrapping_neg(), a[2].wrapping_neg()],
                            parameter: a[5],
                            transparency: a[6] as u16,
                            control: 0,
                            mask_channel: 0,
                        }),
                    ))
                } else {
                    return Ok(Some(self.raster_plan(
                        host,
                        a[0],
                        source_id,
                        Raster::CrossfadeAt {
                            position: [a[1], a[2]],
                            amount: a[6] as u16,
                        },
                        None,
                    )?));
                }
            }
            0x1a => {
                target_checks::effect_level(a[4])?;
                let first = self
                    .graph_field_bitmap(a[2] as u32)
                    .context("BGI first displacement field")?;
                ensure!(
                    self.bitmap(a[2])?.1.format == 4,
                    "BGI first displacement field format"
                );
                let second = if a[3] == -1 {
                    None
                } else {
                    ensure!(
                        self.bitmap(a[3])?.1.format == 4,
                        "BGI second displacement field format"
                    );
                    let second = self
                        .graph_field_bitmap(a[3] as u32)
                        .context("BGI second displacement field")?;
                    ensure!(
                        second.size == first.size,
                        "BGI displacement fields differ in size"
                    );
                    Some(second)
                };
                let spec = FieldWarpSpec::Displacement {
                    first,
                    second,
                    amount: a[4] as u16,
                    bilinear: a[5] != 0,
                };
                spec.validate(self.client, target.size)?;
                if !matches!(target.format, 1 | 2) {
                    return Ok(None);
                }
                Some((
                    vec![self.prepare_bitmap(host, source_id)?],
                    AuxiliaryOperation::Warp {
                        size: target.size,
                        spec,
                    },
                ))
            }
            0x1b => {
                ensure!(
                    target.size == source.size,
                    "BGI horizontal blur bitmap dimensions differ"
                );
                ensure!((a[2] as u32) <= 1, "BGI horizontal blur gradient type");
                target_checks::effect_level(a[3])?;
                if !matches!(target.format, 1 | 2) {
                    return Ok(None);
                }
                return Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    source_id,
                    Raster::Display {
                        spec: DisplayRaster::HorizontalBlur {
                            radius: (a[3] as u32).min(255) as u8,
                            clamp: a[2] != 0,
                        },
                    },
                    None,
                )?));
            }
            0x1c => {
                ensure!(
                    (a[3] as u32) >= 2 && (a[4] as u32) >= 2,
                    "BGI stretch output range"
                );
                ensure!(
                    (a[8] as u32) >= 2 && (a[9] as u32) >= 2,
                    "BGI stretch reference range"
                );
                if !matches!(target.format, 1 | 2) {
                    return Ok(None);
                }
                return Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    source_id,
                    Raster::StretchCopy {
                        source: [a[6], a[7], a[8], a[9]],
                        destination: [a[1], a[2], a[3], a[4]],
                    },
                    None,
                )?));
            }
            0x1d => {
                ensure!(a[2] != 0, "BGI backdrop scale is zero");
                if !matches!(target.format, 1 | 2) {
                    return Ok(None);
                }
                return Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    source_id,
                    Raster::Display {
                        spec: DisplayRaster::RotateBackdrop {
                            scale: a[2] as u32,
                            angle: a[3],
                        },
                    },
                    None,
                )?));
            }
            _ => bail!("BGI immediate bitmap dispatch"),
        };
        let (inputs, operation) = auxiliary.context("BGI immediate bitmap plan")?;
        Ok(Some(BitmapWork {
            reallocate: false,
            outputs: Vec::new(),
            destination: a[0] as u32,
            format: target.format,
            auxiliary: target.auxiliary,
            phase: Phase::Auxiliary { inputs, operation },
        }))
    }
}
