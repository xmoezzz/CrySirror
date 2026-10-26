// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Sparse fractional-delay early echoes with shared complementary-band history.
use crate::{reflections::ReflectionPaths, spectrum::BandSplitter};
use anyhow::{Result, ensure};
use glam::Vec3;
use std::sync::Arc;
struct Tap {
    delay: usize,
    fraction: f32,
    gains: [f32; 3],
    pan: [f32; 2],
}
/// Worker-prepared immutable tap metadata plus single-owner DSP history.
/// Reads/process/reset perform no geometry, I/O or delay-buffer allocation.
pub struct EarlyReflections {
    paths: Arc<ReflectionPaths>,
    splitter: BandSplitter,
    ring: Vec<[f32; 3]>,
    taps: Vec<Tap>,
    at: usize,
    filled: usize,
    rate: u32,
    tail: usize,
    latency: usize,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl EarlyReflections {
    pub fn prepare(
        rate: u32,
        paths: ReflectionPaths,
        direct_latency_frames: usize,
    ) -> Result<Self> {
        ensure!(direct_latency_frames <= 256, "reflection reference latency");
        paths.query.validate()?;
        ensure!(
            (8000..=192000).contains(&rate) && paths.paths.len() <= paths.query.options.max_paths,
            "reflection PCM format/path count"
        );
        let right = Vec3::from(paths.query.listener_forward)
            .cross(Vec3::from(paths.query.listener_up))
            .normalize();
        let mut taps = Vec::with_capacity(paths.paths.len());
        let mut longest = 0;
        for path in &paths.paths {
            ensure!(
                path.length_metres.is_finite()
                    && path.length_metres >= 0.
                    && path.delay_seconds.is_finite()
                    && path.delay_seconds >= 0.
                    && path.bounce.iter().all(|v| v.is_finite())
                    && path
                        .leg_transmission
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v))
                    && path.crossings <= paths.query.options.propagation.transmission_hits * 2,
                "invalid reflection path metadata"
            );
            ensure!(
                path.excess_delay_seconds.is_finite()
                    && (0. ..=paths.query.options.max_delay_seconds)
                        .contains(&path.excess_delay_seconds)
                    && path
                        .gains
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
                "invalid reflection tap"
            );
            let d = Vec3::from(path.direction);
            ensure!(
                d.is_finite() && d.length_squared().is_finite() && d.length_squared() > 1e-12,
                "invalid reflection arrival direction"
            );
            let pan = d.normalize().dot(right).clamp(-1., 1.);
            let frames =
                path.excess_delay_seconds as f64 * rate as f64 + direct_latency_frames as f64;
            let delay = frames.floor() as usize;
            let fraction = (frames - delay as f64) as f32;
            longest = longest.max(delay + usize::from(fraction > 0.));
            taps.push(Tap {
                delay,
                fraction,
                gains: path.gains,
                pan: [(0.5 * (1. - pan)).sqrt(), (0.5 * (1. + pan)).sqrt()],
            });
        }
        let tail = if taps.is_empty() {
            0
        } else {
            longest + (rate as usize).div_ceil(100)
        };
        Ok(Self {
            paths: Arc::new(paths),
            splitter: BandSplitter::new(rate)?,
            ring: vec![[0.; 3]; longest + 2],
            taps,
            at: 0,
            filled: 0,
            rate,
            tail,
            latency: direct_latency_frames,
            storage: None,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn paths(&self) -> &ReflectionPaths {
        &self.paths
    }
    pub fn direct_latency_frames(&self) -> usize {
        self.latency
    }
    pub fn tail_frames(&self) -> usize {
        self.tail
    }
    pub fn resident_bytes(&self) -> usize {
        self.ring.len() * core::mem::size_of::<[f32; 3]>()
            + self.taps.len() * core::mem::size_of::<Tap>()
    }
    pub fn retain_storage(&mut self, storage: Arc<dyn Send + Sync>) {
        self.storage = Some(storage);
    }
    /// Old cells are inaccessible until overwritten; no large buffer clear.
    pub fn reset(&mut self) {
        self.at = 0;
        self.filled = 0;
        self.splitter.reset();
    }
    pub fn process_frame(&mut self, input: f32) -> Result<[f32; 2]> {
        ensure!(input.is_finite(), "nonfinite reflection input");
        if self.taps.is_empty() {
            return Ok([0.; 2]);
        }
        self.ring[self.at] = self.splitter.process(input);
        let mut out = [0.; 2];
        let len = self.ring.len();
        let read = |delay: usize| -> [f32; 3] {
            if delay > self.filled {
                [0.; 3]
            } else {
                self.ring[(self.at + len - delay) % len]
            }
        };
        for tap in &self.taps {
            let a = read(tap.delay);
            let b = read(tap.delay + 1);
            let value = (0..3)
                .map(|i| (a[i] + (b[i] - a[i]) * tap.fraction) * tap.gains[i])
                .sum::<f32>();
            out[0] += value * tap.pan[0];
            out[1] += value * tap.pan[1];
        }
        self.at = (self.at + 1) % len;
        self.filled = (self.filled + 1).min(len - 1);
        let wet = self.paths.query.options.wet_gain;
        out[0] *= wet;
        out[1] *= wet;
        ensure!(
            out.iter().all(|x| x.is_finite()),
            "nonfinite reflection output"
        );
        Ok(out)
    }
}
