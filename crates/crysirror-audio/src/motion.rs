// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Reusable finite-difference pose capture. Owner clocks, never wall time.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
/// Automatic pose-inference limits in the engine world metre convention.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MotionSettings {
    /// Longer frame gaps reset inference, default 250 ms, allowed 1..60000.
    pub max_interval_ms: u64,
    /// Larger world displacement resets inference, default 50 m, range 0.001..1000.
    pub teleport_distance: f32,
}
impl Default for MotionSettings {
    fn default() -> Self {
        Self {
            max_interval_ms: 250,
            teleport_distance: 50.,
        }
    }
}
impl MotionSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (1..=60000).contains(&self.max_interval_ms)
                && self.teleport_distance.is_finite()
                && (0.001..=1000.).contains(&self.teleport_distance),
            "invalid audio motion capture settings"
        );
        Ok(())
    }
}
/// Observable reason for a motion capture velocity or reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionStatus {
    /// No previous pose is available.
    Initial,
    /// Finite difference over a positive bounded frame interval.
    Inferred,
    /// No displacement over a positive frame interval.
    Stationary,
    /// Unchanged zero-duration pose retains the previous velocity.
    ZeroInterval,
    /// Teleport threshold or changed zero-duration pose reset velocity.
    Discontinuity,
    /// The frame interval exceeded the configured inference limit.
    Gap,
    /// Caller-provided velocity bypassed inference.
    Explicit,
}
/// Last owner capture result; independent of source/decoder playback clocks.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MotionEstimate {
    /// Derived or explicit world velocity in metres/second.
    pub velocity: [f32; 3],
    /// Why velocity was inferred, retained or reset at this capture.
    pub status: MotionStatus,
}
impl Default for MotionEstimate {
    fn default() -> Self {
        Self {
            velocity: [0.; 3],
            status: MotionStatus::Initial,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct MotionTracker {
    position: Option<[f32; 3]>,
    estimate: MotionEstimate,
}
impl MotionTracker {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn observe(
        &mut self,
        position: [f32; 3],
        dt_ms: u64,
        explicit: Option<[f32; 3]>,
        settings: MotionSettings,
    ) -> Result<MotionEstimate> {
        settings.validate()?;
        ensure!(
            crate::doppler::vector(position) && explicit.is_none_or(crate::doppler::vector),
            "invalid audio motion vector"
        );
        let mut next = MotionEstimate::default();
        if let Some(old) = self.position {
            let delta: [f64; 3] =
                std::array::from_fn(|i| f64::from(position[i]) - f64::from(old[i]));
            let distance = delta.iter().map(|x| x * x).sum::<f64>().sqrt();
            if dt_ms > settings.max_interval_ms {
                next.status = MotionStatus::Gap;
            } else if distance > f64::from(settings.teleport_distance)
                || (dt_ms == 0 && distance > 0.)
            {
                next.status = MotionStatus::Discontinuity;
            } else if dt_ms == 0 {
                next = self.estimate;
                next.status = MotionStatus::ZeroInterval;
            } else {
                next.velocity = std::array::from_fn(|i| (delta[i] * 1000. / dt_ms as f64) as f32);
                next.status = if distance == 0. {
                    MotionStatus::Stationary
                } else {
                    MotionStatus::Inferred
                };
            }
        }
        if let Some(v) = explicit {
            next.velocity = v;
            next.status = MotionStatus::Explicit;
        }
        self.position = Some(position);
        self.estimate = next;
        Ok(next)
    }
}
