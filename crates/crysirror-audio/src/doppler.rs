// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded direct-path radial-velocity Doppler. Coordinates are metres and
//! velocities metres/second. No propagation delay or retarded-time ray tracing.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
/// Direct radial Doppler controls; opt in through Spatial.doppler.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DopplerSettings {
    /// Velocity exaggeration, 0 disables shifting; default 1, range 0..16.
    pub factor: f32,
    /// Propagation speed in metres/second; default 343.3, range 1..10000.
    pub speed_of_sound: f32,
    /// Symmetric target pitch multiplier bound [1/max_ratio,max_ratio], default 4.
    pub max_ratio: f32,
    /// Output-clock linear transition in ms, default 20, range 0..1000.
    pub smoothing_ms: f32,
}
impl Default for DopplerSettings {
    fn default() -> Self {
        Self {
            factor: 1.,
            speed_of_sound: 343.3,
            max_ratio: 4.,
            smoothing_ms: 20.,
        }
    }
}
impl DopplerSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.factor.is_finite() && (0.0..=16.).contains(&self.factor),
            "Doppler factor must be 0..16"
        );
        ensure!(
            self.speed_of_sound.is_finite() && (1.0..=10000.).contains(&self.speed_of_sound),
            "Doppler sound speed must be 1..10000 m/s"
        );
        ensure!(
            self.max_ratio.is_finite() && (1.0..=4.).contains(&self.max_ratio),
            "Doppler max_ratio must be 1..4"
        );
        ensure!(
            self.smoothing_ms.is_finite() && (0.0..=1000.).contains(&self.smoothing_ms),
            "Doppler smoothing must be 0..1000 ms"
        );
        Ok(())
    }
}
/// Bounded radial calculation with separate velocity and ratio diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DopplerCalculation {
    /// Bounded target multiplier, before interpolation and user pitch/rate.
    pub ratio: f32,
    /// Source velocity projected onto source-to-listener direction, m/s.
    pub source_radial: f32,
    /// Listener velocity on that same direction, m/s (opposite toward source).
    pub listener_radial: f32,
    /// Scaled radial velocity exceeded +/-99% of sound speed.
    pub velocity_clamped: bool,
    /// The physical radial ratio exceeded the configured symmetric pitch bound.
    pub ratio_clamped: bool,
}
pub(crate) fn vector(v: [f32; 3]) -> bool {
    v.iter().all(|v| v.is_finite() && v.abs() <= 1e6)
}
/// OpenAL-style (c-factor*v_listener)/(c-factor*v_source) on the common
/// source-to-listener axis, with explicit bilateral safety and ratio bounds.
pub fn calculate(
    settings: DopplerSettings,
    source: [f32; 3],
    source_velocity: [f32; 3],
    listener: [f32; 3],
    listener_velocity: [f32; 3],
) -> Result<DopplerCalculation> {
    settings.validate()?;
    ensure!(
        [source, source_velocity, listener, listener_velocity]
            .into_iter()
            .all(vector),
        "invalid Doppler position/velocity"
    );
    let d: [f64; 3] = std::array::from_fn(|i| f64::from(listener[i]) - f64::from(source[i]));
    let length = d.iter().map(|v| v * v).sum::<f64>().sqrt();
    let radial = |v: [f32; 3]| {
        if length < 1e-6 {
            0.
        } else {
            (0..3).map(|i| d[i] * f64::from(v[i]) / length).sum::<f64>()
        }
    };
    let s = radial(source_velocity);
    let l = radial(listener_velocity);
    let c = f64::from(settings.speed_of_sound);
    let limit = c * 0.99;
    let raw_s = s * f64::from(settings.factor);
    let raw_l = l * f64::from(settings.factor);
    let vs = raw_s.clamp(-limit, limit);
    let vl = raw_l.clamp(-limit, limit);
    let ratio = (c - vl) / (c - vs);
    let bounded = ratio.clamp(
        1. / f64::from(settings.max_ratio),
        f64::from(settings.max_ratio),
    );
    Ok(DopplerCalculation {
        ratio: bounded as f32,
        source_radial: s as f32,
        listener_radial: l as f32,
        velocity_clamped: vs != raw_s || vl != raw_l,
        ratio_clamped: ratio != bounded,
    })
}
/// Frozen with the mixer frame while DSP runs on a worker.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct DopplerInfo {
    /// Opt-in settings exist on this spatial voice.
    pub enabled: bool,
    /// Current interpolated multiplier at the next output frame.
    pub ratio: f32,
    /// Current pose/velocity target; does not imply audible transition completion.
    pub target: DopplerCalculation,
    /// Target awaits the next render block setup (counters still describe prior setup).
    pub target_pending: bool,
    /// User rate times semitone pitch times interpolated Doppler multiplier.
    pub effective_rate: f32,
    /// Captured world source velocity; zero when absent in standalone Mixer.
    pub source_velocity: [f32; 3],
    /// Captured world listener velocity; zero when absent in standalone Mixer.
    pub listener_velocity: [f32; 3],
    /// Remaining interpolation output frames (including paused/starved time).
    pub smoothing_frames: u64,
}
