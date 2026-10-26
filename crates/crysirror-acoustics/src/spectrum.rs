// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Persistent, allocation-free three-band direct-path DSP with smoothed gains.
use anyhow::{Result, ensure};
pub struct BandFilter {
    low: f32,
    high_low: f32,
    current: [f32; 3],
    target: [f32; 3],
    alpha: [f32; 2],
    smooth: f32,
}
impl BandFilter {
    pub fn new(sample_rate: u32) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&sample_rate),
            "acoustic sample rate"
        );
        let rate = sample_rate as f32;
        Ok(Self {
            low: 0.,
            high_low: 0.,
            current: [1.; 3],
            target: [1.; 3],
            alpha: [800f32, 8000.]
                .map(|f| 1. - (-std::f32::consts::TAU * f.min(rate * 0.45) / rate).exp()),
            smooth: 1. - (-1. / (rate * 0.01)).exp(),
        })
    }
    /// Initialize without a unity-gain burst while the first query is pending.
    pub fn with_gains(sample_rate: u32, gains: [f32; 3]) -> Result<Self> {
        let mut filter = Self::new(sample_rate)?;
        filter.set_gains(gains)?;
        filter.current = gains;
        Ok(filter)
    }
    pub fn set_gains(&mut self, gains: [f32; 3]) -> Result<()> {
        ensure!(
            gains
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.).contains(v)),
            "invalid acoustic band gains"
        );
        self.target = gains;
        Ok(())
    }
    /// Reset sample history for a discontinuous seek, retaining gain controls.
    pub fn reset_history(&mut self) {
        self.low = 0.;
        self.high_low = 0.;
    }
    pub fn process(&mut self, input: f32) -> f32 {
        self.low += self.alpha[0] * (input - self.low);
        self.high_low += self.alpha[1] * (input - self.high_low);
        for (gain, target) in self.current.iter_mut().zip(self.target) {
            *gain += self.smooth * (target - *gain);
        }
        self.current[0] * self.low
            + self.current[1] * (self.high_low - self.low)
            + self.current[2] * (input - self.high_low)
    }
}
