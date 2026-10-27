// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Sparse reflection plans with single-owner history and bounded tap crossfades.
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
/// Immutable, worker-prepared tap plan. Publication does not reset sample history.
pub struct ReflectionPlan {
    paths: Arc<ReflectionPaths>,
    taps: Vec<Tap>,
    rate: u32,
    latency: usize,
    longest: usize,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl ReflectionPlan {
    pub fn prepare(
        rate: u32,
        paths: ReflectionPaths,
        direct_latency_frames: usize,
    ) -> Result<Self> {
        paths.query.validate()?;
        ensure!(
            (8000..=192000).contains(&rate)
                && direct_latency_frames <= 256
                && paths.paths.len() <= paths.query.options.max_paths,
            "reflection PCM format/path count/reference latency"
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
        Ok(Self {
            paths: Arc::new(paths),
            taps,
            rate,
            latency: direct_latency_frames,
            longest,
            storage: None,
        })
    }
    pub fn paths(&self) -> &ReflectionPaths {
        &self.paths
    }
    pub fn direct_latency_frames(&self) -> usize {
        self.latency
    }
    pub fn resident_bytes(&self) -> usize {
        self.taps.capacity() * core::mem::size_of::<Tap>()
            + self.paths.paths.capacity()
                * core::mem::size_of::<crate::reflections::ReflectionPath>()
            + core::mem::size_of::<ReflectionPaths>()
    }
    pub fn retain_storage(&mut self, receipt: Arc<dyn Send + Sync>) {
        self.storage = Some(receipt);
    }
    fn tail_frames(&self) -> usize {
        if self.taps.is_empty() {
            0
        } else {
            self.longest + (self.rate as usize).div_ceil(100)
        }
    }
    fn output(&self, ring: &[[f32; 3]], at: usize, filled: usize) -> [f32; 2] {
        let read = |delay: usize| {
            if delay > filled {
                [0.; 3]
            } else {
                ring[(at + ring.len() - delay) % ring.len()]
            }
        };
        let mut out = [0.; 2];
        for tap in &self.taps {
            let a = read(tap.delay);
            let b = read(tap.delay + 1);
            let v = (0..3)
                .map(|i| (a[i] + (b[i] - a[i]) * tap.fraction) * tap.gains[i])
                .sum::<f32>();
            out[0] += v * tap.pan[0];
            out[1] += v * tap.pan[1];
        }
        let gain = self.paths.query.options.wet_gain;
        [out[0] * gain, out[1] * gain]
    }
}
/// Mutable single-owner delay history. Tracked mode reserves the configured
/// maximum delay once and keeps history even when the current plan has no paths.
pub struct EarlyReflections {
    current: ReflectionPlan,
    previous: Option<ReflectionPlan>,
    splitter: BandSplitter,
    ring: Vec<[f32; 3]>,
    at: usize,
    filled: usize,
    transition_total: usize,
    transition_left: usize,
    tracked: bool,
    audible: bool,
    fade_from_silence: bool,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl EarlyReflections {
    pub fn prepare(rate: u32, paths: ReflectionPaths, latency: usize) -> Result<Self> {
        Self::from_plan(ReflectionPlan::prepare(rate, paths, latency)?, false)
    }
    pub fn prepare_tracked(rate: u32, paths: ReflectionPaths, latency: usize) -> Result<Self> {
        Self::from_plan(ReflectionPlan::prepare(rate, paths, latency)?, true)
    }
    fn from_plan(plan: ReflectionPlan, tracked: bool) -> Result<Self> {
        let capacity = if tracked {
            (plan.paths.query.options.max_delay_seconds as f64 * plan.rate as f64).ceil() as usize
                + 256
                + 2
        } else {
            plan.longest + 2
        };
        Ok(Self {
            splitter: BandSplitter::new(plan.rate)?,
            ring: vec![[0.; 3]; capacity],
            current: plan,
            previous: None,
            at: 0,
            filled: 0,
            transition_total: 0,
            transition_left: 0,
            tracked,
            audible: true,
            fade_from_silence: false,
            storage: None,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.current.rate
    }
    pub fn paths(&self) -> &ReflectionPaths {
        self.current.paths()
    }
    pub fn direct_latency_frames(&self) -> usize {
        self.current.latency
    }
    pub fn tail_frames(&self) -> usize {
        self.current.tail_frames().max(
            self.previous
                .as_ref()
                .filter(|_| self.transition_left > 0)
                .map_or(0, ReflectionPlan::tail_frames),
        )
    }
    pub fn resident_bytes(&self) -> usize {
        self.ring.capacity() * core::mem::size_of::<[f32; 3]>()
            + self.current.resident_bytes()
            + self
                .previous
                .as_ref()
                .map_or(0, ReflectionPlan::resident_bytes)
    }
    pub fn history_capacity_frames(&self) -> usize {
        self.ring.len()
    }
    pub fn is_tracked(&self) -> bool {
        self.tracked
    }
    pub fn set_audible(&mut self, audible: bool) {
        self.audible = audible;
    }
    pub fn can_publish_plan(&self) -> bool {
        self.transition_left == 0
    }
    /// Returns retired metadata for owner-side disposal. At most two plans render.
    pub fn publish_plan(
        &mut self,
        plan: ReflectionPlan,
        crossfade_ms: u32,
    ) -> Result<Option<ReflectionPlan>> {
        ensure!(
            self.tracked && self.can_publish_plan(),
            "reflection transition is pending"
        );
        ensure!(
            plan.rate == self.sample_rate() && plan.longest + 2 <= self.ring.len(),
            "reflection plan exceeds history format/capacity"
        );
        ensure!(
            (1..=200).contains(&crossfade_ms),
            "reflection crossfade duration"
        );
        let retired = self.previous.take();
        let old = std::mem::replace(&mut self.current, plan);
        self.previous = Some(old);
        self.transition_total = (self.sample_rate() as usize * crossfade_ms as usize)
            .div_ceil(1000)
            .max(1);
        self.transition_left = self.transition_total;
        self.fade_from_silence = !self.audible;
        self.audible = true;
        Ok(retired)
    }
    /// The sample loop never frees previous plans. Collect after a completed fade.
    pub fn retire_plan(&mut self) -> Option<ReflectionPlan> {
        if self.transition_left == 0 {
            self.previous.take()
        } else {
            None
        }
    }
    pub fn retain_storage(&mut self, receipt: Arc<dyn Send + Sync>) {
        self.storage = Some(receipt);
    }
    pub fn reset(&mut self) {
        self.at = 0;
        self.filled = 0;
        self.splitter.reset();
    }
    pub fn process_frame(&mut self, input: f32) -> Result<[f32; 2]> {
        ensure!(input.is_finite(), "nonfinite reflection input");
        if !self.tracked && self.current.taps.is_empty() {
            return Ok([0.; 2]);
        }
        self.ring[self.at] = self.splitter.process(input);
        let mut out = self.current.output(&self.ring, self.at, self.filled);
        if self.transition_left > 0 {
            let old = self
                .previous
                .as_ref()
                .unwrap()
                .output(&self.ring, self.at, self.filled);
            let old = if self.fade_from_silence { [0.; 2] } else { old };
            let t = (self.transition_total - self.transition_left) as f32
                / self.transition_total as f32;
            out = [
                old[0] + (out[0] - old[0]) * t,
                old[1] + (out[1] - old[1]) * t,
            ];
            self.transition_left -= 1;
        }
        self.at = (self.at + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len() - 1);
        ensure!(
            out.iter().all(|x| x.is_finite()),
            "nonfinite reflection output"
        );
        Ok(if self.audible { out } else { [0.; 2] })
    }
}
