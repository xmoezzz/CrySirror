// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Persistent, allocation-free three-band direct-path DSP with smoothed gains.
use anyhow::{Result, ensure};
/// Complementary one-pole low/mid/high split at 800/8000 Hz. The three
/// components sum back to the input; prepare once for the output sample rate.
pub struct BandSplitter {
    low: f32,
    high_low: f32,
    alpha: [f32; 2],
}
impl BandSplitter {
    pub fn new(sample_rate: u32) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&sample_rate),
            "acoustic sample rate"
        );
        let rate = sample_rate as f32;
        Ok(Self {
            low: 0.,
            high_low: 0.,
            alpha: [800f32, 8000.]
                .map(|f| 1. - (-std::f32::consts::TAU * f.min(rate * 0.45) / rate).exp()),
        })
    }
    pub fn reset(&mut self) {
        self.low = 0.;
        self.high_low = 0.;
    }
    pub fn process(&mut self, input: f32) -> [f32; 3] {
        self.low += self.alpha[0] * (input - self.low);
        self.high_low += self.alpha[1] * (input - self.high_low);
        [self.low, self.high_low - self.low, input - self.high_low]
    }
}
pub struct BandFilter {
    splitter: BandSplitter,
    current: [f32; 3],
    target: [f32; 3],
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
            splitter: BandSplitter::new(sample_rate)?,
            current: [1.; 3],
            target: [1.; 3],
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
        self.splitter.reset();
    }
    pub fn process(&mut self, input: f32) -> f32 {
        let bands = self.splitter.process(input);
        for (gain, target) in self.current.iter_mut().zip(self.target) {
            *gain += self.smooth * (target - *gain);
        }
        self.current[0] * bands[0] + self.current[1] * bands[1] + self.current[2] * bands[2]
    }
}
