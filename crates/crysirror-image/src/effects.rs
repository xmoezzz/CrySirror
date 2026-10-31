// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared image processing for bitmap, UI, sprite and texture resources. These
//! are destructive resource operations, separate from display-only layer filters.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// CPU pixel limit bounds scratch memory independently of the decoder budget.
pub const MAX_EFFECT_PIXELS: u64 = 4_194_304;
/// Reusable image operations. All channels are straight RGBA bytes; RGB-only
/// transforms preserve alpha. Effects never silently select a different effect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageEffect {
    /// Replace coverage while retaining all stored RGB bytes. Useful for
    /// opaque XRGB resources and explicitly authored mask normalisation.
    OverrideAlpha {
        alpha: u8,
    },
    /// Arbitrary straight RGB tables; premultiplied profiles fail before admission.
    RgbLookup {
        spec: crate::rgb_lut::RgbLookup,
    },
    /// Explicit grouped luminance rounding and integer /256 colour mixture.
    FixedLuma {
        spec: crate::fixed_luma::FixedLumaSpec,
    },
    /// Fixed16.16 piecewise nearest sampling and explicit /256 mixtures.
    PiecewiseResample {
        spec: crate::piecewise_resample::PiecewiseResampleSpec,
    },
    /// Bounded generic stripe/stagger mask for retained transitions.
    StripeReveal {
        spec: crate::sprite_alpha::StripeReveal,
    },
    /// Extract reusable sprite transparency from decoded resource pixels.
    SpriteAlpha {
        spec: crate::sprite_alpha::SpriteAlpha,
    },
    /// Two-input whole-pixel row copy; preserves uncovered destination pixels.
    SineRowCopy {
        spec: crate::row_copy::SineRowCopy,
    },
    /// Two-input clipped coverage multiplication with optional exterior clear.
    AlphaClipRect { spec:crate::alpha_clip::AlphaClipRect },
    Transition {
        spec: crate::transition::ImageTransition,
    },
    /// Engine vector path on the existing image with source-over composition.
    Vector {
        path: crate::vector::VectorPath,
    },
    /// Map the whole source into a convex quad, keeping canvas dimensions.
    Perspective {
        quad: [[f32; 2]; 4],
    },

    /// Per-channel pow(value/255,1/gamma), floor/ceiling in byte units.
    /// Fully transparent pixels keep hidden RGB, matching the Krkr gamma profile.
    Gamma {
        gamma: [f32; 3],
        floor: [i32; 3],
        ceiling: [i32; 3],
    },
    /// Integer luminance: (54R+183G+19B)/256, truncated.
    Grayscale,
    /// CxImage brightness/contrast LUT (brightness -255..255, contrast -100..100).
    BrightnessContrast {
        brightness: i32,
        contrast: i32,
    },
    /// CxImage HSL colorize: replace hue/saturation (0..255), keep lightness.
    Colorize {
        hue: u8,
        saturation: u8,
        blend: f64,
    },
    /// HSL modulate: hue degrees, saturation/luminance percent (-100..100).
    HslModulate {
        hue: i32,
        saturation: i32,
        luminance: i32,
    },
    /// Grey white noise replacing RGB; alpha is unchanged.
    WhiteNoise {
        seed: u32,
    },
    Invert,
    /// Box blur of `rect` (x, y, w, h; whole image when absent): each pixel
    /// becomes the truncated mean of the window `area` = [left, top, right,
    /// bottom] offsets (inclusive, containing 0) around it, with the window
    /// clipped to the image and the divisor counting only covered pixels.
    /// `alpha_weighted` averages colour premultiplied by alpha and
    /// un-premultiplies (krkrz `DoBoxBlurForAlpha`); otherwise all four
    /// channels average independently (`DoBoxBlur`).
    BoxBlur {
        #[serde(default)]
        rect: Option<[u32; 4]>,
        area: [i32; 4],
        #[serde(default)]
        alpha_weighted: bool,
    },
    /// Separable Gaussian on premultiplied floating RGBA, clamped image edges.
    GaussianBlur {
        sigma: f32,
    },
    /// Deterministic signed uniform RGB noise. Alpha is unchanged.
    Noise {
        seed: u32,
        amplitude: u8,
        monochrome: bool,
    },
    Flip {
        horizontal: bool,
        vertical: bool,
    },
    /// Whole-pixel row displacement: round(sin(phase+y*frequency)*amplitude).
    /// Source edges clamp. Phase/frequency are radians, amplitude is pixels.
    Raster {
        amplitude: f32,
        frequency: f32,
        phase: f32,
    },
}
impl ImageEffect {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::RgbLookup { spec } => spec.validate()?,
            Self::FixedLuma { spec } => spec.validate()?,
            Self::PiecewiseResample { spec } => spec.validate()?,
            Self::StripeReveal { spec } => spec.validate()?,
            Self::SpriteAlpha { spec } => spec.validate()?,
            Self::SineRowCopy { .. } => (),
            Self::AlphaClipRect {spec} => spec.validate()?,
            Self::Transition { spec } => spec.validate()?,
            Self::Vector { path } => path.validate()?,
            Self::Perspective { quad } => crate::warp::validate_quad(quad)?,
            Self::Gamma {
                gamma,
                floor,
                ceiling,
            } => {
                ensure!(
                    gamma
                        .iter()
                        .all(|g| g.is_finite() && (0.01..=100.).contains(g)),
                    "gamma must be finite in 0.01..100"
                );
                ensure!(
                    floor
                        .iter()
                        .chain(ceiling)
                        .all(|v| (-65535..=65535).contains(v)),
                    "gamma ramp bounds"
                );
            }
            Self::BrightnessContrast {
                brightness,
                contrast,
            } => ensure!(
                (-65535..=65535).contains(brightness) && (-65535..=65535).contains(contrast),
                "brightness/contrast bounds"
            ),
            Self::Colorize { blend, .. } => ensure!(blend.is_finite(), "colorize blend"),
            Self::HslModulate {
                hue,
                saturation,
                luminance,
            } => ensure!(
                [hue, saturation, luminance]
                    .iter()
                    .all(|v| (-65535..=65535).contains(*v)),
                "modulate bounds"
            ),
            Self::BoxBlur { area, .. } => {
                ensure!(
                    area[0] <= 0 && area[1] <= 0 && area[2] >= 0 && area[3] >= 0,
                    "box blur area must contain the centre pixel"
                );
                ensure!(area.iter().all(|v| v.abs() <= 8192), "box blur area bounds");
            }
            Self::GaussianBlur { sigma } => ensure!(
                sigma.is_finite() && (0.0..=16.).contains(sigma),
                "blur sigma must be 0..16"
            ),
            Self::Raster {
                amplitude,
                frequency,
                phase,
            } => ensure!(
                [amplitude, frequency, phase]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 65536.),
                "raster parameters"
            ),
            _ => (),
        }
        Ok(())
    }
}
impl ImageSurface {
    pub fn apply_effect(&self, effect: &ImageEffect) -> Result<Self> {
        self.apply_effect_cancellable(effect, &|| Ok(()))
    }
    /// The input remains immutable on error/cancellation. Jobs call the fence
    /// between rows/passes; no worker ever mutates a published surface in place.
    pub fn apply_effect_cancellable(
        &self,
        effect: &ImageEffect,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        self.validate()?;
        effect.validate()?;
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= MAX_EFFECT_PIXELS,
            "image effect pixel budget"
        );
        check()?;
        if matches!(
            effect,
            ImageEffect::Transition { .. } | ImageEffect::SineRowCopy { .. } | ImageEffect::AlphaClipRect {..}
        ) {
            anyhow::bail!("transition requires a two-image context")
        }
        if let ImageEffect::RgbLookup { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::FixedLuma { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::PiecewiseResample { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::StripeReveal { spec } = effect {
            return self.stripe_reveal(spec, check);
        }
        if let ImageEffect::SpriteAlpha { spec } = effect {
            return self.sprite_alpha(spec, check);
        }
        if let ImageEffect::Vector { path } = effect {
            return self.draw_vector(path, check);
        }
        if let ImageEffect::Perspective { quad } = effect {
            return self.perspective(*quad, check);
        }
        if let ImageEffect::GaussianBlur { sigma } = effect {
            return self.gaussian(*sigma, check);
        }
        if let ImageEffect::BoxBlur {
            rect,
            area,
            alpha_weighted,
        } = effect
        {
            return self.box_blur(*rect, *area, *alpha_weighted, check);
        }
        let mut out = self.clone();
        let w = self.width as usize;
        let h = self.height as usize;
        let mut seed = match effect {
            ImageEffect::Noise { seed, .. } | ImageEffect::WhiteNoise { seed } => *seed,
            _ => 0,
        };
        if seed == 0 {
            seed = 0x9e3779b9;
        }
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut lut = [[0u8; 256]; 3];
        if let ImageEffect::BrightnessContrast {
            brightness,
            contrast,
        } = effect
        {
            lut = [crate::cximage::brightness_contrast_lut(*brightness, *contrast); 3];
        }

        if let ImageEffect::Gamma {
            gamma,
            floor,
            ceiling,
        } = effect
        {
            for c in 0..3 {
                for v in 0..256 {
                    lut[c][v] = (((v as f64 / 255.).powf(1. / f64::from(gamma[c]))
                        * f64::from(ceiling[c] - floor[c])
                        + 0.5
                        + f64::from(floor[c]))
                    .trunc())
                    .clamp(0., 255.) as u8;
                }
            }
        }
        for y in 0..h {
            check()?;
            for x in 0..w {
                let at = (y * w + x) * 4;
                let p = &self.rgba[at..at + 4];
                match effect {
                    ImageEffect::OverrideAlpha { alpha } => out.rgba[at + 3] = *alpha,
                    ImageEffect::Gamma { .. } => {
                        if p[3] != 0 {
                            for c in 0..3 {
                                out.rgba[at + c] = lut[c][p[c] as usize];
                            }
                        }
                    }
                    // Native operates on BGRA without consulting alpha.
                    ImageEffect::BrightnessContrast { .. } => {
                        for c in 0..3 {
                            out.rgba[at + c] = lut[c][p[c] as usize];
                        }
                    }
                    ImageEffect::Colorize {
                        hue,
                        saturation,
                        blend,
                    } => {
                        let v =
                            crate::cximage::colorize([p[0], p[1], p[2]], *hue, *saturation, *blend);
                        out.rgba[at..at + 3].copy_from_slice(&v);
                    }
                    ImageEffect::HslModulate {
                        hue,
                        saturation,
                        luminance,
                    } => {
                        let v = crate::cximage::modulate(
                            [p[0], p[1], p[2]],
                            *hue,
                            *saturation,
                            *luminance,
                        );
                        out.rgba[at..at + 3].copy_from_slice(&v);
                    }
                    ImageEffect::WhiteNoise { .. } => {
                        let n = (random() % 256) as u8;
                        out.rgba[at..at + 3].fill(n);
                    }
                    ImageEffect::Grayscale => {
                        let v =
                            ((u32::from(p[0]) * 54 + u32::from(p[1]) * 183 + u32::from(p[2]) * 19)
                                >> 8) as u8;
                        out.rgba[at..at + 3].fill(v);
                    }
                    ImageEffect::Invert => {
                        for c in 0..3 {
                            out.rgba[at + c] = 255 - p[c];
                        }
                    }
                    ImageEffect::Noise {
                        amplitude,
                        monochrome,
                        ..
                    } => {
                        let a = i32::from(*amplitude);
                        let mut noise = (random() % (2 * a + 1) as u32) as i32 - a;
                        for c in 0..3 {
                            if c > 0 && !*monochrome {
                                noise = (random() % (2 * a + 1) as u32) as i32 - a;
                            }
                            out.rgba[at + c] = (i32::from(p[c]) + noise).clamp(0, 255) as u8;
                        }
                    }
                    ImageEffect::Flip {
                        horizontal,
                        vertical,
                    } => {
                        let sx = if *horizontal { w - 1 - x } else { x };
                        let sy = if *vertical { h - 1 - y } else { y };
                        let from = (sy * w + sx) * 4;
                        out.rgba[at..at + 4].copy_from_slice(&self.rgba[from..from + 4]);
                    }
                    ImageEffect::Raster {
                        amplitude,
                        frequency,
                        phase,
                    } => {
                        let dx = ((f64::from(*phase) + y as f64 * f64::from(*frequency)).sin()
                            * f64::from(*amplitude))
                        .round() as i64;
                        let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                        let from = (y * w + sx) * 4;
                        out.rgba[at..at + 4].copy_from_slice(&self.rgba[from..from + 4]);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(out)
    }
    fn box_blur(
        &self,
        rect: Option<[u32; 4]>,
        area: [i32; 4],
        alpha_weighted: bool,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let (w, h) = (self.width as usize, self.height as usize);
        let [rx, ry, rw, rh] = rect.unwrap_or([0, 0, self.width, self.height]);
        let x0 = (rx as usize).min(w);
        let y0 = (ry as usize).min(h);
        let x1 = (rx as usize).saturating_add(rw as usize).min(w);
        let y1 = (ry as usize).saturating_add(rh as usize).min(h);
        if area == [0; 4] || x0 >= x1 || y0 >= y1 {
            return Ok(self.clone());
        }
        // Summed-area table of the summed channel values ([R, G, B, A]; with
        // alpha weighting the colour terms are `c * (a + a/128) >> 8`).
        let stride = w + 1;
        let mut sat = vec![[0u64; 4]; stride * (h + 1)];
        for y in 0..h {
            check()?;
            let mut row = [0u64; 4];
            for x in 0..w {
                let p = &self.rgba[(y * w + x) * 4..][..4];
                let v = if alpha_weighted {
                    let adj = u32::from(p[3]) + (u32::from(p[3]) >> 7);
                    [0, 1, 2].map(|c| u64::from(u32::from(p[c]) * adj >> 8))
                } else {
                    [0, 1, 2].map(|c| u64::from(p[c]))
                };
                row[0] += v[0];
                row[1] += v[1];
                row[2] += v[2];
                row[3] += u64::from(p[3]);
                let above = sat[y * stride + x + 1];
                sat[(y + 1) * stride + x + 1] = [0, 1, 2, 3].map(|c| above[c] + row[c]);
            }
        }
        let mut out = self.clone();
        for y in y0..y1 {
            check()?;
            let top = (y as i64 + i64::from(area[1])).max(0) as usize;
            let bottom = (y as i64 + i64::from(area[3])).min(h as i64 - 1) as usize + 1;
            for x in x0..x1 {
                let left = (x as i64 + i64::from(area[0])).max(0) as usize;
                let right = (x as i64 + i64::from(area[2])).min(w as i64 - 1) as usize + 1;
                let n = ((right - left) * (bottom - top)) as u64;
                let s = |c: usize| {
                    sat[bottom * stride + right][c] + sat[top * stride + left][c]
                        - sat[top * stride + right][c]
                        - sat[bottom * stride + left][c]
                };
                let mean = [0, 1, 2, 3].map(|c| s(c) / n);
                let px = &mut out.rgba[(y * w + x) * 4..][..4];
                if alpha_weighted {
                    let a = mean[3];
                    for c in 0..3 {
                        px[c] = if a == 0 {
                            0
                        } else {
                            (mean[c] * 255 / a).min(255) as u8
                        };
                    }
                    px[3] = a as u8;
                } else {
                    for c in 0..4 {
                        px[c] = mean[c] as u8;
                    }
                }
            }
        }
        Ok(out)
    }
    fn gaussian(&self, sigma: f32, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        if sigma == 0. {
            return Ok(self.clone());
        }
        let radius = (sigma * 3.).ceil() as i32;
        let mut kernel = (-radius..=radius)
            .map(|x| (-0.5 * (x as f64 / f64::from(sigma)).powi(2)).exp())
            .collect::<Vec<_>>();
        let total = kernel.iter().sum::<f64>();
        for k in &mut kernel {
            *k /= total;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let mut a = vec![[0f32; 4]; w * h];
        let mut b = a.clone();
        for (p, rgba) in a.iter_mut().zip(self.rgba.chunks_exact(4)) {
            let alpha = rgba[3] as f32 / 255.;
            *p = [
                rgba[0] as f32 * alpha,
                rgba[1] as f32 * alpha,
                rgba[2] as f32 * alpha,
                rgba[3] as f32,
            ];
        }
        for vertical in [false, true] {
            for y in 0..h {
                check()?;
                for x in 0..w {
                    let mut v = [0f64; 4];
                    for (i, &k) in kernel.iter().enumerate() {
                        let d = i as i32 - radius;
                        let sx = if vertical {
                            x
                        } else {
                            (x as i32 + d).clamp(0, w as i32 - 1) as usize
                        };
                        let sy = if vertical {
                            (y as i32 + d).clamp(0, h as i32 - 1) as usize
                        } else {
                            y
                        };
                        for c in 0..4 {
                            v[c] += f64::from(a[sy * w + sx][c]) * k;
                        }
                    }
                    b[y * w + x] = v.map(|n| n as f32);
                }
            }
            std::mem::swap(&mut a, &mut b);
        }
        let mut out = self.clone();
        for (p, v) in out.rgba.chunks_exact_mut(4).zip(a) {
            let alpha = v[3];
            p[3] = alpha.round().clamp(0., 255.) as u8;
            for c in 0..3 {
                p[c] = if alpha > 0. {
                    (v[c] * 255. / alpha).round().clamp(0., 255.) as u8
                } else {
                    0
                };
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(w: u32, h: u32, px: &[[u8; 4]]) -> ImageSurface {
        ImageSurface {
            width: w,
            height: h,
            rgba: px.concat(),
        }
    }

    #[test]
    fn box_blur_truncates_the_clipped_window_mean() {
        // 3x1 row, 3x1 window: edge pixels average two neighbours.
        let s = surface(3, 1, &[[0, 0, 0, 255], [9, 30, 100, 255], [20, 0, 0, 255]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: None,
                area: [-1, 0, 1, 0],
                alpha_weighted: false,
            })
            .unwrap();
        assert_eq!(out.rgba, [4, 15, 50, 255, 9, 10, 33, 255, 14, 15, 50, 255]);
    }

    #[test]
    fn box_blur_rect_limits_writes_not_reads() {
        let s = surface(2, 1, &[[100, 0, 0, 255], [0, 0, 0, 255]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: Some([1, 0, 1, 1]),
                area: [-1, 0, 1, 0],
                alpha_weighted: false,
            })
            .unwrap();
        assert_eq!(out.rgba, [100, 0, 0, 255, 50, 0, 0, 255]);
    }

    #[test]
    fn alpha_weighted_box_blur_ignores_transparent_colour() {
        let s = surface(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 0]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: None,
                area: [-1, 0, 1, 0],
                alpha_weighted: true,
            })
            .unwrap();
        // Mean alpha 127; premultiplied red 255*256>>8 = 255 -> 127 -> 255*127/127.
        assert_eq!(&out.rgba[0..4], &[255, 0, 0, 127]);
    }
}
