//! Integer RGB mixing and1536-step HSV modulation over straight stored bytes.
use crate::{effects::MAX_EFFECT_PIXELS, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorMatrixFixed {
    pub matrix: [[i32; 3]; 3],
    pub offset: [i32; 3],
    pub divisor: u32,
}
impl ColorMatrixFixed {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=65536).contains(&self.divisor),
            "integer colour divisor"
        );
        ensure!(
            self.matrix
                .iter()
                .flatten()
                .chain(&self.offset)
                .all(|n| n.unsigned_abs() <= 1_048_576),
            "integer colour matrix bounds"
        );
        Ok(())
    }
    pub fn apply(
        &self,
        input: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        map(input, check, |rgb| {
            std::array::from_fn(|row| {
                let sum = i64::from(self.offset[row])
                    + self.matrix[row]
                        .iter()
                        .zip(rgb)
                        .map(|(m, v)| i64::from(*m) * i64::from(v))
                        .sum::<i64>();
                (sum / i64::from(self.divisor)).clamp(0, 255) as u8
            })
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegerColorModulation {
    /// Signed phase in a1536-step wheel; replaces hue when replace_hue=true.
    pub hue: i32,
    pub replace_hue: bool,
    pub luminance256: i32,
    /// Zero bypasses chroma mixing; otherwise all three products divide by256.
    pub saturation256: i32,
}
impl IntegerColorModulation {
    pub fn validate(&self) -> Result<()> {
        ensure!((-768..=768).contains(&self.hue), "integer hue phase bounds");
        ensure!(
            (-256..=256).contains(&self.luminance256),
            "integer luminance bounds"
        );
        ensure!(
            (-256..=2560).contains(&self.saturation256),
            "integer saturation bounds"
        );
        Ok(())
    }
    pub fn apply(
        &self,
        input: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        map(input, check, |rgb| {
            let mut rgb = rgb.map(i32::from);
            if self.hue != 0 || self.replace_hue {
                let (h, s, v) = to_hsv(rgb);
                let h = if self.replace_hue {
                    self.hue
                } else {
                    h + self.hue
                };
                rgb = from_hsv(h, s, v, rgb);
            }
            let amount = self.luminance256;
            if amount < 0 {
                rgb = rgb.map(|c| (256 + amount) * c / 256);
            } else if amount > 0 {
                rgb = rgb.map(|c| (255 * amount + (256 - amount) * c) / 256);
            }
            if self.saturation256 != 0 {
                let side = -85 * self.saturation256 / 256;
                let main = 255 - 2 * side;
                rgb = std::array::from_fn(|i| {
                    (main * rgb[i] + side * (rgb[(i + 1) % 3] + rgb[(i + 2) % 3])) / 256
                });
            }
            rgb.map(|c| c.clamp(0, 255) as u8)
        })
    }
}
fn to_hsv([r, g, b]: [i32; 3]) -> (i32, i32, i32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0 {
        return (0, 0, max);
    }
    let mut hue = 0;
    // Independent comparisons preserve the original tie priority.
    if max == r {
        hue = ((g - b) << 8) / delta;
    }
    if max == g {
        hue = ((b - r) << 8) / delta + 512;
    }
    if max == b {
        hue = ((r - g) << 8) / delta + 1024;
    }
    if hue < 0 {
        hue += 1536;
    }
    if hue > 1536 {
        hue -= 1536;
    }
    (hue, (delta << 8) / max, max)
}
fn from_hsv(mut hue: i32, saturation: i32, value: i32, original: [i32; 3]) -> [i32; 3] {
    if hue < 0 {
        hue += 1536;
    }
    if hue > 1536 {
        hue -= 1536;
    }
    let sector = hue / 256;
    let phase = hue % 256;
    let p = value * (255 - saturation) / 256;
    let q = value * (255 - phase * saturation / 256) / 256;
    let t = value * (255 - saturation * (255 - phase) / 256) / 256;
    match sector {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        5 => [value, p, q],
        // Exactly1536 has no native switch case; retain stored input RGB.
        _ => original,
    }
    .map(|v| v.clamp(0, 255))
}
fn map(
    input: &ImageSurface,
    check: &dyn Fn() -> Result<()>,
    f: impl Fn([u8; 3]) -> [u8; 3],
) -> Result<ImageSurface> {
    input.validate()?;
    ensure!(
        u64::from(input.width) * u64::from(input.height) <= MAX_EFFECT_PIXELS,
        "integer colour pixel budget"
    );
    check()?;
    let mut out = input.clone();
    for row in out.rgba.chunks_exact_mut(input.width as usize * 4) {
        check()?;
        for pixel in row.chunks_exact_mut(4) {
            let rgb = f([pixel[0], pixel[1], pixel[2]]);
            pixel[..3].copy_from_slice(&rgb);
        }
    }
    check()?;
    Ok(out)
}

/// Packed RGB scalar operations with signed, truncating /256 interpolation.
/// This differs from arithmetic-shift interpolation for a negative delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum StoredRgbBlend { Normal, Multiply, Add, Subtract, Darken, Lighten }
/// Blend stored RGB bytes independently. The caller chooses coverage, and no
/// transfer to linear light or premultiplication is implicit in this profile.
pub fn blend_stored_rgb(destination:[u8;3],source:[u8;3],coverage:u8,mode:StoredRgbBlend)->[u8;3] {
    let weight=i32::from(coverage)+i32::from(coverage>>7);
    let mix=|from:i32,to:i32|from+((to-from)*weight)/256;
    std::array::from_fn(|i|{
        let (d,s)=(i32::from(destination[i]),i32::from(source[i]));
        let value=match mode {
            StoredRgbBlend::Normal=>mix(d,s),
            StoredRgbBlend::Multiply=>{let m=mix(255,s);(d*(m+(m>>7)))>>8},
            StoredRgbBlend::Add=>d+mix(0,s),
            StoredRgbBlend::Subtract=>d-mix(0,s),
            StoredRgbBlend::Darken=>d.min(mix(255,s)),
            StoredRgbBlend::Lighten=>d.max(mix(0,s)),
        };
        value.clamp(0,255) as u8
    })
}

/// Straight-RGBA 4x5 colour matrix: each output channel is
/// `sum(m[row][c] * in[c]) + m[row][4] * 255` over R, G, B, A, then rounded
/// down and clamped to a byte. This is the general per-pixel colour
/// transform (for example Ren'Py `im.MatrixColor`/`renpy.display.module.colormatrix`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RgbaColorMatrix {
    pub matrix: [[f32; 5]; 4],
}
impl RgbaColorMatrix {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.matrix.iter().flatten().all(|v| v.is_finite() && v.abs() <= 65536.0),
            "RGBA colour matrix bounds"
        );
        Ok(())
    }
    pub fn apply(&self, input: &ImageSurface, check: &dyn Fn() -> Result<()>) -> Result<ImageSurface> {
        self.validate()?;
        input.validate()?;
        let mut out = input.clone();
        for (row, pixels) in out.rgba.chunks_mut(input.width as usize * 4).enumerate() {
            if row % 64 == 0 {
                check()?;
            }
            for px in pixels.chunks_exact_mut(4) {
                let v = [px[0], px[1], px[2], px[3]].map(f32::from);
                for (c, m) in self.matrix.iter().enumerate() {
                    let sum = m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3] * v[3] + m[4] * 255.0;
                    px[c] = sum.floor().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(out)
    }
}
