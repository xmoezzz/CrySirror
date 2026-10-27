// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared two-image crossfade, rule reveal and blur-crossfade resource operations.
use crate::{effects::ImageEffect, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// A resource transition at explicit progress. A mask reveals bright pixels
/// first; softness is the normalized reveal ramp. Endpoints preserve exact input
/// bytes. Blur grows toward the midpoint; this is an engine profile, not an
/// alias for every legacy transition provider's arithmetic.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageTransition {
    pub progress: f32,
    pub softness: f32,
    pub blur: f32,
}
impl ImageTransition {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.progress.is_finite() && (0.0..=1.).contains(&self.progress),
            "transition progress"
        );
        ensure!(
            self.softness.is_finite() && (0.0..=1.).contains(&self.softness),
            "transition softness"
        );
        ensure!(
            self.blur.is_finite() && (0.0..=16.).contains(&self.blur),
            "transition blur"
        );
        Ok(())
    }
}
impl ImageSurface {
    pub fn transition_to(
        &self,
        other: &Self,
        mask: Option<&Self>,
        spec: &ImageTransition,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        self.validate()?;
        other.validate()?;
        spec.validate()?;
        ensure!(
            [self.width, self.height] == [other.width, other.height],
            "transition image dimensions differ"
        );
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= crate::effects::MAX_EFFECT_PIXELS,
            "transition pixel budget"
        );
        if let Some(m) = mask {
            m.validate()?;
            ensure!(
                [m.width, m.height] == [self.width, self.height],
                "transition mask dimensions differ"
            );
        }
        check()?;
        if spec.progress == 0. {
            return Ok(self.clone());
        }
        if spec.progress == 1. {
            return Ok(other.clone());
        }
        let sigma = spec.blur * 2. * spec.progress.min(1. - spec.progress);
        let a = self.apply_effect_cancellable(&ImageEffect::GaussianBlur { sigma }, check)?;
        let b = other.apply_effect_cancellable(&ImageEffect::GaussianBlur { sigma }, check)?;
        let mut out = self.clone();
        for y in 0..self.height as usize {
            check()?;
            for x in 0..self.width as usize {
                let at = (y * self.width as usize + x) * 4;
                let t = if let Some(mask) = mask {
                    let p = &mask.rgba[at..at + 4];
                    let lum =
                        (f64::from(p[0]) * 54. + f64::from(p[1]) * 183. + f64::from(p[2]) * 19.)
                            / (256. * 255.);
                    let threshold = 1. - lum;
                    let p = f64::from(spec.progress);
                    if spec.softness == 0. {
                        if p >= threshold {
                            1.
                        } else {
                            0.
                        }
                    } else {
                        ((p * (1. + f64::from(spec.softness)) - threshold)
                            / f64::from(spec.softness))
                        .clamp(0., 1.)
                    }
                } else {
                    f64::from(spec.progress)
                };
                let aa = f64::from(a.rgba[at + 3]);
                let ba = f64::from(b.rgba[at + 3]);
                let alpha = aa * (1. - t) + ba * t;
                out.rgba[at + 3] = alpha.round().clamp(0., 255.) as u8;
                for c in 0..3 {
                    out.rgba[at + c] = if alpha > 0. {
                        ((f64::from(a.rgba[at + c]) * aa * (1. - t)
                            + f64::from(b.rgba[at + c]) * ba * t)
                            / alpha)
                            .round()
                            .clamp(0., 255.) as u8
                    } else {
                        0
                    };
                }
            }
        }
        Ok(out)
    }
}
