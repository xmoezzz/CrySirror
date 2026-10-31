// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Checked grouped fixed-point luminance and colour mixing. Quantization groups
//! model SIMD pair-wise arithmetic as well as ordinary sum-before-shift luma;
//! no VM or platform code is required to use this image operation.
use crate::{effects::MAX_EFFECT_PIXELS, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedLumaSpec {
    /// Coefficients in engine RGB order, with sum no greater than 2^bits.
    pub coefficients: [u32; 3],
    /// Equal group IDs are summed before a shift; groups are shifted separately.
    pub groups: [u8; 3],
    pub fractional_bits: u8,
    /// Luminance colour mix weight, in explicit /256 units.
    pub amount256: u16,
}
impl FixedLumaSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.fractional_bits <= 16,
            "fixed luminance fractional bits"
        );
        let scale = 1u32 << self.fractional_bits;
        ensure!(
            self.coefficients.iter().all(|v| *v <= scale)
                && self.coefficients.iter().map(|v| u64::from(*v)).sum::<u64>() <= u64::from(scale),
            "fixed luminance coefficients"
        );
        ensure!(
            self.groups.iter().all(|v| *v < 3),
            "fixed luminance quantization groups"
        );
        ensure!(self.amount256 <= 256, "fixed luminance blend amount");
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
            "fixed luminance pixel budget"
        );
        check()?;
        let mut output = input.clone();
        for row in output.rgba.chunks_exact_mut(input.width as usize * 4) {
            check()?;
            for pixel in row.chunks_exact_mut(4) {
                let mut sums = [0u32; 3];
                for channel in 0..3 {
                    sums[self.groups[channel] as usize] +=
                        u32::from(pixel[channel]) * self.coefficients[channel];
                }
                let luma = sums.iter().map(|v| *v >> self.fractional_bits).sum::<u32>();
                for channel in pixel[..3].iter_mut() {
                    *channel = ((u32::from(*channel) * u32::from(256 - self.amount256)
                        + luma * u32::from(self.amount256))
                        >> 8) as u8;
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
    fn image() -> ImageSurface {
        ImageSurface {
            width: 4,
            height: 1,
            rgba: vec![
                255, 255, 255, 7, 255, 0, 0, 13, 0, 255, 0, 19, 0, 0, 255, 23,
            ],
        }
    }
    #[test]
    fn grouped_rounding_is_explicit_and_retains_alpha() {
        let spec = FixedLumaSpec {
            coefficients: [76, 150, 29],
            groups: [1, 0, 0],
            fractional_bits: 8,
            amount256: 256,
        };
        let out = spec.apply(&image(), &|| Ok(())).unwrap();
        assert_eq!(
            out.rgba,
            [253, 253, 253, 7, 75, 75, 75, 13, 149, 149, 149, 19, 28, 28, 28, 23]
        );
        let ordinary = FixedLumaSpec {
            groups: [0, 0, 0],
            ..spec
        }
        .apply(&image(), &|| Ok(()))
        .unwrap();
        assert_eq!(&ordinary.rgba[..4], &[254, 254, 254, 7]);
        let effect = crate::effects::ImageEffect::FixedLuma { spec };
        let encoded = serde_json::to_vec(&effect).unwrap();
        let decoded = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(image().apply_effect(&decoded).unwrap().rgba, out.rgba);
        let unchanged = FixedLumaSpec {
            amount256: 0,
            ..spec
        }
        .apply(&image(), &|| Ok(()))
        .unwrap();
        assert_eq!(unchanged.rgba, image().rgba);
        let mixed = FixedLumaSpec {
            amount256: 128,
            ..spec
        }
        .apply(&image(), &|| Ok(()))
        .unwrap();
        assert_eq!(&mixed.rgba[4..8], &[165, 37, 37, 13]);
    }
    #[test]
    fn rejects_invalid_profiles_and_cancels_without_changing_input() {
        let base = FixedLumaSpec {
            coefficients: [76, 150, 29],
            groups: [1, 0, 0],
            fractional_bits: 8,
            amount256: 256,
        };
        assert!(FixedLumaSpec {
            amount256: 257,
            ..base
        }
        .validate()
        .is_err());
        assert!(FixedLumaSpec {
            groups: [3, 0, 0],
            ..base
        }
        .validate()
        .is_err());
        assert!(FixedLumaSpec {
            coefficients: [255, 255, 255],
            ..base
        }
        .validate()
        .is_err());
        let image = image();
        let before = image.rgba.clone();
        assert!(base.apply(&image, &|| anyhow::bail!("cancelled")).is_err());
        assert_eq!(image.rgba, before);
    }
}
