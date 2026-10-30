// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Reusable GPU subtree transition descriptions, independent of any native VM.
use super::*;
/// Maximum per-axis radius supported by the bounded GPU tile filter.
pub const MAX_TRANSITION_BLUR_RADIUS: u32 = 128;
/// A scoped, renderer-owned transition between two isolated subtree inputs.
/// Filters use encoded byte-domain pixels, edge replication and bounded radii.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubtreeTransitionEffect {
    /// Increasing blur of the old input and decreasing blur of the new one,
    /// followed by gamma-space byte-weight blending. Static inputs are GPU
    /// snapshots; dynamic inputs are recaptured each frame.
    Blur {
        /// Maximum old-input box radius [horizontal, vertical], in pixels.
        before: [u32; 2],
        /// Maximum new-input radius, decreasing toward zero.
        after: [u32; 2],
        /// Recapture changing inputs every frame; false freezes first ready inputs.
        #[serde(default)]
        dynamic: bool,
    },
    /// Grayscale, clamped-edge box-blurred rule image, optionally inverted,
    /// controlling a rule wipe. Acceleration matches the legacy pow curve.
    Dim {
        /// Client-scoped immutable rule image and wipe softness.
        rule: RuleWipe,
        /// Rule box radius [horizontal, vertical], before optional inversion.
        blur: [u32; 2],
        /// Invert the filtered grayscale level (255 minus the result).
        #[serde(default)]
        negative: bool,
        /// Above one: t^a; below minus one: 1-(1-t)^(-a); otherwise linear.
        #[serde(default = "default_acceleration")]
        acceleration: f64,
    },
}
fn default_acceleration() -> f64 {
    1.
}
impl SubtreeTransitionEffect {
    pub fn validate(&self) -> Result<()> {
        let radius = match self {
            Self::Blur { before, after, .. } => {
                ensure!(
                    before
                        .iter()
                        .chain(after)
                        .all(|r| *r <= MAX_TRANSITION_BLUR_RADIUS),
                    "transition blur radius limit"
                );
                return Ok(());
            }
            Self::Dim {
                rule,
                blur,
                acceleration,
                ..
            } => {
                ensure!(
                    acceleration.is_finite(),
                    "transition acceleration must be finite"
                );
                ensure!(
                    !rule.asset.is_empty() && rule.asset.len() <= 512 && rule.vague <= 4096,
                    "dim rule asset/vague"
                );
                blur
            }
        };
        ensure!(
            radius.iter().all(|r| *r <= MAX_TRANSITION_BLUR_RADIUS),
            "transition blur radius limit"
        );
        Ok(())
    }
    /// Progress curve; blur intentionally stays linear (the native provider
    /// parses accel but never calls its otherwise-defined curve function).
    pub fn progress(&self, elapsed_ms: u64, duration_ms: u64) -> f64 {
        let t = (elapsed_ms as f64 / duration_ms.max(1) as f64).clamp(0., 1.);
        match self {
            Self::Dim { acceleration, .. } if *acceleration > 1. => t.powf(*acceleration),
            Self::Dim { acceleration, .. } if *acceleration < -1. => {
                1. - (1. - t).powf(-*acceleration)
            }
            _ => t,
        }
    }
    /// Radius for a frame, using truncation of the linear native time ratio.
    pub fn radii(&self, elapsed_ms: u64, duration_ms: u64) -> ([u32; 2], [u32; 2]) {
        let t = self.progress(elapsed_ms, duration_ms);
        match self {
            Self::Blur { before, after, .. } => (
                before.map(|n| (f64::from(n) * t).floor() as u32),
                after.map(|n| (f64::from(n) * (1. - t)).floor() as u32),
            ),
            Self::Dim { blur, .. } => (*blur, [0, 0]),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_effect_curves_radii_and_limits() {
        let blur = SubtreeTransitionEffect::Blur {
            before: [32, 16],
            after: [8, 4],
            dynamic: false,
        };
        assert_eq!(blur.radii(250, 1000), ([8, 4], [6, 3]));
        assert_eq!(blur.progress(1000, 1000), 1.);
        let mut dim = SubtreeTransitionEffect::Dim {
            rule: RuleWipe {
                asset: "mask.png".into(),
                vague: 64,
            },
            blur: [16, 8],
            negative: true,
            acceleration: 2.,
        };
        assert_eq!(dim.progress(500, 1000), 0.25);
        assert!(dim.validate().is_ok());
        if let SubtreeTransitionEffect::Dim { acceleration, .. } = &mut dim {
            *acceleration = -2.;
        }
        assert_eq!(dim.progress(500, 1000), 0.75);
        assert!(
            SubtreeTransitionEffect::Blur {
                before: [129, 0],
                after: [0, 0],
                dynamic: true
            }
            .validate()
            .is_err()
        );
    }
}
