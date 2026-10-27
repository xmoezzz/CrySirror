// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-captured world velocities; mixer workers own the actual Doppler ramp.
use anyhow::{Context, Result};
use crysirror_audio::{
    mixer::{Listener, Spatial, VoiceHandle},
    motion::{MotionEstimate, MotionSettings, MotionTracker},
};
use std::collections::BTreeMap;
type Key = (u32, u32, u64);
fn key(h: VoiceHandle) -> Key {
    (h.client, h.slot, h.generation)
}
#[derive(Default)]
struct Capture {
    tracker: MotionTracker,
    last: MotionEstimate,
}
#[derive(Default)]
pub(crate) struct MotionRuntime {
    settings: BTreeMap<u32, MotionSettings>,
    listeners: BTreeMap<u32, Capture>,
    sources: BTreeMap<Key, Capture>,
}
impl MotionRuntime {
    pub(crate) fn settings(&self, client: u32) -> MotionSettings {
        self.settings.get(&client).copied().unwrap_or_default()
    }
    pub(crate) fn set_settings(&mut self, client: u32, settings: MotionSettings) -> Result<()> {
        settings.validate()?;
        self.settings.insert(client, settings);
        self.reset(client);
        Ok(())
    }
    pub(crate) fn reset(&mut self, client: u32) {
        self.listeners.remove(&client);
        self.sources.retain(|k, _| k.0 != client);
    }
    pub(crate) fn reset_listener(&mut self, client: u32) {
        self.listeners.remove(&client);
    }
    pub(crate) fn clear(&mut self) {
        self.listeners.clear();
        self.sources.clear();
    }
    pub(crate) fn listener(
        &mut self,
        client: u32,
        mut listener: Listener,
        dt: u64,
    ) -> Result<Listener> {
        let settings = self.settings(client);
        let c = self.listeners.entry(client).or_default();
        c.last = c
            .tracker
            .observe(listener.position, dt, listener.velocity, settings)?;
        listener.velocity = Some(c.last.velocity);
        Ok(listener)
    }
    pub(crate) fn source(&mut self, h: VoiceHandle, spatial: &mut Spatial, dt: u64) -> Result<()> {
        if spatial.doppler.is_none() {
            self.sources.remove(&key(h));
            return Ok(());
        }
        let settings = self.settings(h.client);
        let c = self.sources.entry(key(h)).or_default();
        c.last = c
            .tracker
            .observe(spatial.position, dt, spatial.velocity, settings)?;
        spatial.velocity = Some(c.last.velocity);
        Ok(())
    }
    pub(crate) fn retain(&mut self, alive: impl Fn(VoiceHandle) -> bool) {
        self.sources.retain(|&(client, slot, generation), _| {
            alive(VoiceHandle {
                client,
                slot,
                generation,
            })
        });
    }
    fn info(&self, h: VoiceHandle) -> serde_json::Value {
        serde_json::json!({"source":self.sources.get(&key(h)).map(|c|c.last),"listener":self.listeners.get(&h.client).map(|c|c.last)})
    }
}
pub(crate) fn call(
    cx: &mut crate::EngineContext,
    name: &str,
    args: &[crate::Value],
) -> Result<Option<crate::Value>> {
    let client = cx.state()?.client.0;
    match name {
        "motionSettings" => Ok(Some(crate::Value::from_json(serde_json::to_value(
            cx.state()?.mixer_audio.motion.settings(client),
        )?))),
        "setMotionSettings" => {
            let v = args.first().context("motion settings required")?.to_json();
            let settings: MotionSettings = serde_json::from_value(v)?;
            cx.state()?
                .mixer_audio
                .motion
                .set_settings(client, settings)?;
            Ok(Some(crate::Value::Null))
        }
        "resetMotion" => {
            cx.state()?.mixer_audio.motion.reset(client);
            Ok(Some(crate::Value::Null))
        }
        "dopplerVoice" => {
            let h: VoiceHandle =
                serde_json::from_value(args.first().context("voice handle required")?.to_json())?;
            let a = &cx.state()?.mixer_audio;
            let info = a.mixer.doppler_info(client, h)?;
            let mut value = serde_json::to_value(info)?;
            value["motion"] = a.motion.info(h);
            Ok(Some(crate::Value::from_json(value)))
        }
        _ => Ok(None),
    }
}
