// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared client bus sends and return routing for prepared late reverberation.
use crate::mixer::Bus;
use anyhow::{Result, ensure};
use crysirror_acoustics::reverb::{LateReverb, ReverbSpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One post-fader bus send. Muted input buses send silence; Master is excluded.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReverbSend {
    /// Source bus, independent of the effect's return bus.
    pub bus: Bus,
    /// Send amplitude, 0..1.
    pub gain: f32,
}
/// Authored late-reverb routing. The return never feeds back into its own send.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReverbSettings {
    /// Parameters prepared on a worker, separate from geometry simulation.
    pub room: ReverbSpec,
    /// At most five unique non-Master bus sends. Empty sends disable input.
    pub sends: Vec<ReverbSend>,
    /// Return bus whose volume/mute and Music ducking affect the wet output.
    pub return_bus: Bus,
    /// Return amplitude, 0..1. Dry playback remains unchanged.
    pub wet_gain: f32,
}
impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            room: Default::default(),
            sends: vec![ReverbSend {
                bus: Bus::Sfx,
                gain: 0.2,
            }],
            return_bus: Bus::Ambient,
            wet_gain: 1.,
        }
    }
}
impl ReverbSettings {
    pub fn validate(&self) -> Result<()> {
        self.room.validate()?;
        ensure!(
            self.return_bus != Bus::Master
                && self.wet_gain.is_finite()
                && (0. ..=1.).contains(&self.wet_gain),
            "invalid reverb return"
        );
        ensure!(self.sends.len() <= 5, "reverb send count");
        let mut buses = BTreeSet::new();
        for send in &self.sends {
            ensure!(
                send.bus != Bus::Master
                    && send.gain.is_finite()
                    && (0. ..=1.).contains(&send.gain)
                    && buses.insert(send.bus),
                "invalid or duplicate reverb send"
            );
        }
        Ok(())
    }
}
pub struct PreparedReverb {
    pub(crate) dsp: LateReverb,
    pub(crate) settings: ReverbSettings,
    pub(crate) input: [f32; 2],
}
impl PreparedReverb {
    pub fn prepare(rate: u32, settings: ReverbSettings) -> Result<Self> {
        settings.validate()?;
        Ok(Self {
            dsp: LateReverb::prepare(rate, settings.room.clone())?,
            settings,
            input: [0.; 2],
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.dsp.sample_rate()
    }
    pub fn settings(&self) -> &ReverbSettings {
        &self.settings
    }
    /// Reservations survive publication until actual DSP disposal.
    pub fn retain_storage(&mut self, receipt: std::sync::Arc<dyn Send + Sync>) {
        self.dsp.retain_storage(receipt);
    }
    pub(crate) fn send(&self, bus: Bus) -> f32 {
        self.settings
            .sends
            .iter()
            .find(|s| s.bus == bus)
            .map_or(0., |s| s.gain)
    }
}
