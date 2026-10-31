// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Exact fixed-point mixing of immutable image resources on engine workers.
use crate::{effects::MAX_EFFECT_PIXELS, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixedMixRounding {
    SeparateTerms,
    SumThenShift,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixedMixAlpha {
    PreserveLeft,
    Opaque,
    Weighted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedMixSpec {
    pub left_weight: u32,
    pub right_weight: u32,
    pub fractional_bits: u8,
    pub rounding: FixedMixRounding,
    pub alpha: FixedMixAlpha,
}
impl FixedMixSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.fractional_bits <= 16,
            "fixed image mix fractional bits"
        );
        let scale = 1u32 << self.fractional_bits;
        ensure!(
            self.left_weight <= scale
                && self.right_weight <= scale
                && self.left_weight + self.right_weight <= scale,
            "fixed image mix weights"
        );
        Ok(())
    }
    fn channel(&self, left: u8, right: u8) -> u8 {
        let left = u32::from(left) * self.left_weight;
        let right = u32::from(right) * self.right_weight;
        match self.rounding {
            FixedMixRounding::SeparateTerms => {
                ((left >> self.fractional_bits) + (right >> self.fractional_bits)) as u8
            }
            FixedMixRounding::SumThenShift => ((left + right) >> self.fractional_bits) as u8,
        }
    }
    pub fn apply(
        &self,
        left: &ImageSurface,
        right: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        left.validate()?;
        right.validate()?;
        ensure!(
            [left.width, left.height] == [right.width, right.height],
            "fixed image mix size mismatch"
        );
        ensure!(
            u64::from(left.width) * u64::from(left.height) <= MAX_EFFECT_PIXELS,
            "fixed image mix pixel budget"
        );
        check()?;
        let mut output = left.clone();
        for (out, row) in output
            .rgba
            .chunks_exact_mut(left.width as usize * 4)
            .zip(right.rgba.chunks_exact(right.width as usize * 4))
        {
            check()?;
            for (out, right) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                for c in 0..3 {
                    out[c] = self.channel(out[c], right[c]);
                }
                out[3] = match self.alpha {
                    FixedMixAlpha::PreserveLeft => out[3],
                    FixedMixAlpha::Opaque => 255,
                    FixedMixAlpha::Weighted => self.channel(out[3], right[3]),
                };
            }
        }
        check()?;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_rounding_and_normalized_opaque_alpha_are_explicit() {
        let a = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![1, 255, 99, 0],
        };
        let b = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![1, 255, 1, 255],
        };
        let mut spec = FixedMixSpec {
            left_weight: 128,
            right_weight: 128,
            fractional_bits: 8,
            rounding: FixedMixRounding::SumThenShift,
            alpha: FixedMixAlpha::Opaque,
        };
        assert_eq!(
            spec.apply(&a, &b, &|| Ok(())).unwrap().rgba,
            [1, 255, 50, 255]
        );
        spec.rounding = FixedMixRounding::SeparateTerms;
        spec.alpha = FixedMixAlpha::PreserveLeft;
        assert_eq!(
            spec.apply(&a, &b, &|| Ok(())).unwrap().rgba,
            [0, 254, 49, 0]
        );
        spec.left_weight = 257;
        assert!(spec.apply(&a, &b, &|| Ok(())).is_err());
    }
    #[test]
    fn cancellation_keeps_both_resources_immutable() {
        let a = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![10, 20, 30, 255],
        };
        let spec = FixedMixSpec {
            left_weight: 128,
            right_weight: 128,
            fractional_bits: 8,
            rounding: FixedMixRounding::SumThenShift,
            alpha: FixedMixAlpha::Weighted,
        };
        assert!(spec.apply(&a, &a, &|| anyhow::bail!("cancelled")).is_err());
        assert_eq!(a.rgba, [10, 20, 30, 255]);
    }
}
