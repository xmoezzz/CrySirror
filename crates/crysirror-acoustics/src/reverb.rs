// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Worker-prepared parametric late reverberation over FunDSP; no geometry claim.
use anyhow::{Result, ensure};
use fundsp::audiounit::AudioUnit;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Authored late-reverb parameters. These do not infer room dimensions or RT60
/// from scene geometry. The upstream decay parameter is approximate.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReverbSpec {
    /// FDN delay scale in metres, 1..50; not an acoustic-scene bounding box.
    pub room_size_metres: f64,
    /// Upstream approximate time to -60 dB in seconds, 0.1..20.
    pub decay_seconds: f64,
    /// High-frequency damping, 0..1.
    pub damping: f64,
}
impl Default for ReverbSpec {
    fn default() -> Self {
        Self {
            room_size_metres: 10.,
            decay_seconds: 1.5,
            damping: 0.5,
        }
    }
}
impl ReverbSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.room_size_metres.is_finite() && (1. ..=50.).contains(&self.room_size_metres),
            "reverb room size must be 1..50 metres"
        );
        ensure!(
            self.decay_seconds.is_finite() && (0.1..=20.).contains(&self.decay_seconds),
            "reverb decay must be 0.1..20 seconds"
        );
        ensure!(
            self.damping.is_finite() && (0. ..=1.).contains(&self.damping),
            "reverb damping must be 0..1"
        );
        Ok(())
    }
}
/// Mutable DSP history is owned by one mixer. Construction, delay allocation and
/// sample-rate setup belong on a worker; per-frame processing does not allocate.
pub struct LateReverb {
    graph: Box<dyn AudioUnit>,
    rate: u32,
    spec: ReverbSpec,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl LateReverb {
    pub fn prepare(rate: u32, spec: ReverbSpec) -> Result<Self> {
        spec.validate()?;
        ensure!((8000..=192000).contains(&rate), "reverb sample rate");
        let mut graph: Box<dyn AudioUnit> = Box::new(fundsp::prelude::reverb_stereo(
            spec.room_size_metres,
            spec.decay_seconds,
            spec.damping,
        ));
        graph.set_sample_rate(rate as f64);
        graph.allocate();
        Ok(Self {
            graph,
            rate,
            spec,
            storage: None,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn spec(&self) -> &ReverbSpec {
        &self.spec
    }
    pub fn retain_storage(&mut self, receipt: Arc<dyn Send + Sync>) {
        self.storage = Some(receipt);
    }
    pub fn process_frame(&mut self, frame: [f32; 2]) -> Result<[f32; 2]> {
        ensure!(
            frame.iter().all(|x| x.is_finite()),
            "nonfinite reverb input"
        );
        let (l, r) = self.graph.filter_stereo(frame[0], frame[1]);
        ensure!(l.is_finite() && r.is_finite(), "nonfinite reverb output");
        Ok([l, r])
    }
}
