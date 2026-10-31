// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Arbitrary encoded-byte RGB lookup. This is not a colour-space conversion:
//! each table addresses one stored straight RGB channel, including hidden RGB
//! at zero alpha. Alpha never changes. ImageSurface/ImageResource carry straight
//! RGBA; premultiplied input is explicitly rejected because they carry no
//! association metadata. No VM policy, file parser or device lives here.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbLookupProfile {
    /// Lookup every straight RGB byte, including hidden RGB at zero alpha.
    StraightRgba,
    /// Recognized for explicit rejection: this pipeline cannot publish premultiplied
    /// bytes as an ordinary straight ImageSurface. No implicit unpremultiplication.
    PremultipliedRgba,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RgbLookup {
    pub red: Vec<u8>,
    pub green: Vec<u8>,
    pub blue: Vec<u8>,
    pub profile: RgbLookupProfile,
}
impl RgbLookup {
    /// Exact encoded-byte weighted colour transform. Each contribution is
    /// truncated separately before addition; alpha remains unchanged, as for
    /// every RGB lookup. This admits legacy Q15/Q8 profiles without replacing
    /// them with floating alpha interpolation. No image allocation occurs here.
    pub fn weighted_color_fixed(
        color: [u8; 3],
        source_weight: u32,
        color_weight: u32,
        fractional_bits: u8,
    ) -> Result<Self> {
        ensure!(fractional_bits <= 16, "RGB weighted fractional bits");
        let scale = 1u32 << fractional_bits;
        ensure!(
            source_weight <= scale
                && color_weight <= scale
                && source_weight + color_weight <= scale,
            "RGB weighted contribution bounds"
        );
        let table = |color: u8| {
            (0u32..=255)
                .map(|source| {
                    ((source * source_weight) / scale + (u32::from(color) * color_weight) / scale)
                        as u8
                })
                .collect()
        };
        Ok(Self {
            red: table(color[0]),
            green: table(color[1]),
            blue: table(color[2]),
            profile: RgbLookupProfile::StraightRgba,
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.profile == RgbLookupProfile::StraightRgba,
            "premultiplied RGB lookup profile is unsupported for straight ImageSurface resources"
        );
        ensure!(
            self.red.len() == 256 && self.green.len() == 256 && self.blue.len() == 256,
            "RGB lookup requires exactly three 256-byte tables"
        );
        Ok(())
    }
    /// Immutable all-or-error output, <=4M pixels and 768 table bytes. The worker
    /// checks cancellation between rows; no result escapes on failure. ImageJob's
    /// existing memory reservation and owner completion/publication still apply.
    pub fn apply(
        &self,
        input: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        input.validate()?;
        ensure!(
            u64::from(input.width) * u64::from(input.height) <= crate::effects::MAX_EFFECT_PIXELS,
            "RGB lookup pixel budget"
        );
        check()?;
        let mut out = input.clone();
        let tables = [&self.red, &self.green, &self.blue];
        for row in out.rgba.chunks_exact_mut(input.width as usize * 4) {
            check()?;
            for p in row.chunks_exact_mut(4) {
                for c in 0..3 {
                    p[c] = tables[c][p[c] as usize];
                }
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_weights_truncate_each_contribution_and_preserve_alpha() {
        let input = ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![1, 255, 101, 0, 255, 255, 255, 255],
        };
        let lut = RgbLookup::weighted_color_fixed([1, 255, 101], 16384, 16384, 15).unwrap();
        let out = lut.apply(&input, &|| Ok(())).unwrap();
        assert_eq!(out.rgba, [0, 254, 100, 0, 127, 254, 177, 255]);
        let q15 = RgbLookup::weighted_color_fixed([0; 3], 32767, 0, 15).unwrap();
        assert_eq!(q15.red[255], 254);
        assert!(RgbLookup::weighted_color_fixed([255; 3], 32768, 1, 15).is_err());
        assert!(RgbLookup::weighted_color_fixed([0; 3], 0, 0, 17).is_err());
    }
    fn lookup(profile: RgbLookupProfile) -> RgbLookup {
        RgbLookup {
            red: (0..=255).map(|x| 255 - x).collect(),
            green: (0..=255).map(|x| x ^ 0x55).collect(),
            blue: (0..=255)
                .map(|x| if x % 2 == 0 { 17 } else { 231 })
                .collect(),
            profile,
        }
    }
    #[test]
    fn arbitrary_independent_tables_preserve_all_alpha_and_hidden_rgb() {
        let image = ImageSurface {
            width: 3,
            height: 1,
            rgba: vec![2, 3, 4, 0, 7, 8, 9, 128, 255, 0, 1, 255],
        };
        let effect = crate::effects::ImageEffect::RgbLookup {
            spec: lookup(RgbLookupProfile::StraightRgba),
        };
        let out = image.apply_effect(&effect).unwrap();
        assert_eq!(
            out.rgba,
            [253, 86, 17, 0, 248, 93, 231, 128, 0, 85, 231, 255]
        );
        assert_eq!(image.rgba[0], 2);
        let json = serde_json::to_vec(&effect).unwrap();
        let again: crate::effects::ImageEffect = serde_json::from_slice(&json).unwrap();
        assert_eq!(image.apply_effect(&again).unwrap().rgba, out.rgba);
    }
    #[test]
    fn public_surface_effect_rejects_premultiplied_representation() {
        let image = ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![32, 64, 128, 128, 0, 0, 0, 0],
        };
        let effect = crate::effects::ImageEffect::RgbLookup {
            spec: lookup(RgbLookupProfile::PremultipliedRgba),
        };
        assert!(image
            .apply_effect(&effect)
            .unwrap_err()
            .to_string()
            .contains("premultiplied"));
        assert_eq!(image.rgba, [32, 64, 128, 128, 0, 0, 0, 0]);
    }
    #[test]
    fn bad_table_or_profile_cannot_modify_source() {
        let image = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![129, 2, 3, 128],
        };
        let before = image.rgba.clone();
        assert!(lookup(RgbLookupProfile::PremultipliedRgba)
            .apply(&image, &|| Ok(()))
            .is_err());
        let mut invalid = lookup(RgbLookupProfile::StraightRgba);
        invalid.red.pop();
        assert!(invalid.apply(&image, &|| Ok(())).is_err());
        assert_eq!(image.rgba, before);
        let mut value = serde_json::to_value(lookup(RgbLookupProfile::StraightRgba)).unwrap();
        value["profile"] = "implicit_auto".into();
        assert!(serde_json::from_value::<RgbLookup>(value).is_err());
    }
    #[test]
    fn cancellation_drops_result_and_keeps_input_immutable() {
        use std::cell::Cell;
        let image = ImageSurface {
            width: 1,
            height: 3,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        };
        let before = image.rgba.clone();
        let checks = Cell::new(0);
        let r = lookup(RgbLookupProfile::StraightRgba).apply(&image, &|| {
            checks.set(checks.get() + 1);
            ensure!(checks.get() < 3, "cancelled");
            Ok(())
        });
        assert!(r.is_err());
        assert_eq!(image.rgba, before);
    }
}
