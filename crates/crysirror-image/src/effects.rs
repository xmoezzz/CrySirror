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
    /// Two-input whole-pixel row copy; preserves uncovered destination pixels.
    SineRowCopy {
        spec: crate::row_copy::SineRowCopy,
    },
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
    Invert,
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
            Self::SineRowCopy { .. } => (),
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
            ImageEffect::Transition { .. } | ImageEffect::SineRowCopy { .. }
        ) {
            anyhow::bail!("transition requires a two-image context")
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
        let mut out = self.clone();
        let w = self.width as usize;
        let h = self.height as usize;
        let mut seed = match effect {
            ImageEffect::Noise { seed, .. } => *seed,
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
                    ImageEffect::Gamma { .. } => {
                        if p[3] != 0 {
                            for c in 0..3 {
                                out.rgba[at + c] = lut[c][p[c] as usize];
                            }
                        }
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
