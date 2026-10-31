// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Immutable piecewise nearest-neighbour sampling with explicit16.16 steps and
//! byte-exact /256 mixing. Useful for quadrant zoom, mirrored tiles and fixed
//! raster transforms; no native VM, device or external pointer is involved.
use crate::{effects::MAX_EFFECT_PIXELS, ImageSurface};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedSampleRegion {
    pub destination_origin: [i32; 2],
    /// Each destination axis advances one pixel forward or backward.
    pub destination_step: [i8; 2],
    pub size: [u32; 2],
    /// Source texel is arithmetic floor(q16/65536). The original immutable
    /// input is sampled even when earlier regions have written this texel.
    pub source_origin_q16: [i64; 2],
    pub source_step_q16: [i64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResampleAlpha {
    Weighted,
    PreserveDestination,
    Opaque,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiecewiseResampleSpec {
    /// Regions execute in order; overlap mixes against the staged destination.
    pub regions: Vec<FixedSampleRegion>,
    /// These are independent floor-quantized weights, not inferred complements.
    pub source_weight256: u16,
    pub destination_weight256: u16,
    pub alpha: ResampleAlpha,
}
impl PiecewiseResampleSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.regions.len() <= 64, "piecewise sample region limit");
        ensure!(
            u32::from(self.source_weight256) + u32::from(self.destination_weight256) <= 256,
            "piecewise sample normalized /256 weights"
        );
        let mut writes = 0u64;
        for r in &self.regions {
            ensure!(
                r.destination_step.iter().all(|n| matches!(n, -1 | 1)),
                "piecewise sample destination step"
            );
            ensure!(
                r.size.iter().all(|n| (1..=8192).contains(n)),
                "piecewise sample region dimensions"
            );
            writes = writes
                .checked_add(u64::from(r.size[0]) * u64::from(r.size[1]))
                .context("piecewise sample write overflow")?;
            ensure!(writes <= MAX_EFFECT_PIXELS, "piecewise sample work budget");
            for axis in 0..2 {
                r.source_step_q16[axis]
                    .checked_mul(i64::from(r.size[axis] - 1))
                    .and_then(|n| r.source_origin_q16[axis].checked_add(n))
                    .context("piecewise sample fixed coordinate overflow")?;
            }
        }
        Ok(())
    }
    pub fn apply(
        &self,
        input: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        input.validate()?;
        ensure!(
            u64::from(input.width) * u64::from(input.height) <= MAX_EFFECT_PIXELS,
            "piecewise sample image budget"
        );
        let dimensions = [i64::from(input.width), i64::from(input.height)];
        // Endpoints bound each monotonic axis. Reject the entire operation
        // before any output allocation or traversal, rather than clamping a
        // malformed caller profile into an unrelated effect.
        for r in &self.regions {
            for axis in 0..2 {
                let count = i64::from(r.size[axis] - 1);
                let d0 = i64::from(r.destination_origin[axis]);
                let d1 = d0 + count * i64::from(r.destination_step[axis]);
                ensure!(
                    d0.min(d1) >= 0 && d0.max(d1) < dimensions[axis],
                    "piecewise sample destination extent"
                );
                let s0 = r.source_origin_q16[axis];
                let s1 = s0 + count * r.source_step_q16[axis];
                ensure!(
                    s0.min(s1) >= 0 && s0.max(s1) < dimensions[axis] * 65536,
                    "piecewise sample source extent"
                );
            }
        }
        check()?;
        let mut output = input.clone();
        for r in &self.regions {
            for row in 0..r.size[1] {
                check()?;
                let dy = i64::from(r.destination_origin[1])
                    + i64::from(row) * i64::from(r.destination_step[1]);
                let sy = (r.source_origin_q16[1] + i64::from(row) * r.source_step_q16[1]) >> 16;
                for column in 0..r.size[0] {
                    let dx = i64::from(r.destination_origin[0])
                        + i64::from(column) * i64::from(r.destination_step[0]);
                    let sx =
                        (r.source_origin_q16[0] + i64::from(column) * r.source_step_q16[0]) >> 16;
                    let si = ((sy * i64::from(input.width) + sx) * 4) as usize;
                    let di = ((dy * i64::from(input.width) + dx) * 4) as usize;
                    for channel in 0..if self.alpha == ResampleAlpha::Weighted {
                        4
                    } else {
                        3
                    } {
                        output.rgba[di + channel] = ((u32::from(input.rgba[si + channel])
                            * u32::from(self.source_weight256)
                            + u32::from(output.rgba[di + channel])
                                * u32::from(self.destination_weight256))
                            >> 8) as u8;
                    }
                    if self.alpha == ResampleAlpha::Opaque {
                        output.rgba[di + 3] = 255;
                    }
                }
            }
        }
        check()?;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> PiecewiseResampleSpec {
        PiecewiseResampleSpec {
            regions: vec![FixedSampleRegion {
                destination_origin: [0, 0],
                destination_step: [1, 1],
                size: [4, 1],
                source_origin_q16: [32768, 0],
                source_step_q16: [32768, 0],
            }],
            source_weight256: 256,
            destination_weight256: 0,
            alpha: ResampleAlpha::PreserveDestination,
        }
    }
    fn image() -> ImageSurface {
        ImageSurface {
            width: 4,
            height: 1,
            rgba: vec![
                10, 20, 30, 1, 40, 50, 60, 2, 70, 80, 90, 3, 100, 110, 120, 4,
            ],
        }
    }
    #[test]
    fn exact_fractional_steps_mirrored_destinations_and_independent_weights() {
        let input = image();
        let out = spec().apply(&input, &|| Ok(())).unwrap();
        assert_eq!(
            out.rgba,
            [10, 20, 30, 1, 40, 50, 60, 2, 40, 50, 60, 3, 70, 80, 90, 4]
        );
        let mut s = spec();
        s.regions[0].destination_origin = [3, 0];
        s.regions[0].destination_step = [-1, 1];
        s.alpha = ResampleAlpha::Weighted;
        assert_eq!(
            s.apply(&input, &|| Ok(())).unwrap().rgba,
            [70, 80, 90, 3, 40, 50, 60, 2, 40, 50, 60, 2, 10, 20, 30, 1]
        );
        let white = ImageSurface {
            width: 4,
            height: 1,
            rgba: vec![255; 16],
        };
        s.source_weight256 = 100;
        s.destination_weight256 = 155;
        s.alpha = ResampleAlpha::Opaque;
        assert_eq!(
            s.apply(&white, &|| Ok(())).unwrap().rgba,
            [254, 254, 254, 255].repeat(4)
        );
        let effect = crate::effects::ImageEffect::PiecewiseResample { spec: s };
        let decoded = serde_json::from_slice(&serde_json::to_vec(&effect).unwrap()).unwrap();
        assert_eq!(
            white.apply_effect(&decoded).unwrap().rgba,
            [254, 254, 254, 255].repeat(4)
        );
        assert_eq!(input, image());
    }
    #[test]
    fn endpoint_validation_and_mid_traversal_cancel_preserve_inputs() {
        let input = image();
        let mut s = spec();
        s.regions[0].source_origin_q16[0] = -1;
        assert!(s.apply(&input, &|| Ok(())).is_err());
        s = spec();
        s.regions[0].destination_step[0] = 0;
        assert!(s.validate().is_err());
        s = spec();
        s.regions[0].source_step_q16[0] = i64::MAX;
        assert!(s.validate().is_err());
        s = spec();
        s.source_weight256 = 257;
        assert!(s.validate().is_err());
        let count = std::cell::Cell::new(0);
        let s = spec();
        assert!(s
            .apply(&input, &|| {
                count.set(count.get() + 1);
                ensure!(count.get() < 2, "cancelled");
                Ok(())
            })
            .is_err());
        assert_eq!(input, image());
    }
}
