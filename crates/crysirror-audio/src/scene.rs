// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Canonical SceneState voices share worker preparation and the caller's mixer.
use crate::{
    buffered::{BufferedSource, StreamBufferSpec},
    channels::{PreparedChannel, asset, prepare},
    mixer::{Bus, Mixer, VoiceHandle},
};
use anyhow::{Result, ensure};
use crysirror_core::{
    service::{AudioAsset, AudioParameters, ClientStorage, PcmBus},
    state::{ClientId, ObjectId, SceneState},
};
use crysirror_tasks::{
    Cancellation, TaskPool,
    table::{OperationState, OperationTable},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Derived readiness, separate from the canonical voice declaration/save data.
pub enum SceneVoicePhase {
    Stopped,
    Queued,
    Loading,
    Playing,
    Paused,
    Ended,
    Failed,
}
/// Client-scoped readiness product; no worker/device pointers or saved cursors.
#[derive(Clone, Debug, Serialize)]
pub struct SceneVoiceStatus {
    /// Owning client namespace.
    pub client: u32,
    /// Canonical client-local channel.
    pub channel: u32,
    /// Desired logical playback epoch, zero when stopped.
    pub epoch: u64,
    /// Current preparation/playback phase.
    pub phase: SceneVoicePhase,
    /// Ready playback position; not a saved sample cursor.
    pub position_seconds: Option<f64>,
    /// Retained preparation or streaming failure.
    pub error: Option<String>,
}
struct Voice {
    epoch: u64,
    published_epoch: Option<u64>,
    handle: Option<VoiceHandle>,
    ticket: Option<u64>,
    error: Option<String>,
}
struct Charge {
    counter: Arc<AtomicUsize>,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.counter.fetch_sub(STREAM_BYTES, Ordering::Relaxed);
    }
}
const STREAM_BYTES: usize = 4 * 1024 * 1024;
const HOST_BYTES: usize = 512 * 1024 * 1024;
/// Device-free bridge. Hosts supply their scoped storage capabilities and mixer;
/// neither worker results nor mixer handles enter SceneState or native saves.
pub struct SceneAudio {
    voices: BTreeMap<ObjectId, Voice>,
    routes: BTreeMap<ObjectId, Bus>,
    jobs: OperationTable<PreparedChannel>,
    budget: Arc<AtomicUsize>,
    restored: Option<u64>,
    paused: bool,
}
impl Default for SceneAudio {
    fn default() -> Self {
        Self {
            voices: BTreeMap::new(),
            routes: BTreeMap::new(),
            jobs: OperationTable::new(16, 64).unwrap(),
            budget: Arc::new(AtomicUsize::new(0)),
            restored: None,
            paused: false,
        }
    }
}
fn pcm_bus(bus: Bus) -> PcmBus {
    match bus {
        Bus::Music => PcmBus::Music,
        Bus::Sfx => PcmBus::Sfx,
        Bus::Voice => PcmBus::Voice,
        Bus::Ambient => PcmBus::Ambient,
        Bus::Ui => PcmBus::Ui,
        Bus::Master => unreachable!(),
    }
}
fn publish(
    mixer: &mut Mixer,
    v: &mut Voice,
    id: ObjectId,
    gain: u16,
    bus: Bus,
    paused: bool,
    mut ready: PreparedChannel,
) -> Result<()> {
    ready.spec.gain = gain as f32 / 1000.;
    ready.spec.bus = bus;
    ready.spec.paused = paused;
    if let Some(h) = v.handle {
        if mixer.is_playing(id.client.0, h) {
            mixer.stop(id.client.0, h, 0.)?;
        }
    }
    v.handle = Some(mixer.play(id.client.0, ready.source, ready.spec)?);
    v.published_epoch = Some(v.epoch);
    Ok(())
}
impl SceneAudio {
    /// Drop derived work/playback; routing remains host policy across restore.
    pub fn reset(&mut self, mixer: &mut Mixer) {
        self.jobs.clear();
        for (id, v) in &self.voices {
            if let Some(h) = v.handle {
                if mixer.is_playing(id.client.0, h) {
                    let _ = mixer.stop(id.client.0, h, 0.);
                }
            }
        }
        self.voices.clear();
        self.restored = None;
    }
    /// Host routing policy; changing it preserves the source timeline.
    pub fn set_route(&mut self, id: ObjectId, bus: Bus) -> Result<()> {
        ensure!(bus != Bus::Master, "master is not a voice bus");
        self.routes.insert(id, bus);
        Ok(())
    }
    /// Pause existing voices and any voice that publishes while paused.
    pub fn set_paused(&mut self, mixer: &mut Mixer, paused: bool) -> Result<()> {
        self.paused = paused;
        for (id, v) in &self.voices {
            if let Some(h) = v.handle {
                if mixer.is_playing(id.client.0, h) {
                    mixer.pause(id.client.0, h, paused)?;
                }
            }
        }
        Ok(())
    }
    /// Reconcile declarations before collecting results: Stop, replacement and
    /// restore always invalidate stale work before it could publish. Full job
    /// tables/memory budgets leave excess declarations Queued for later frames.
    pub fn sync(
        &mut self,
        mixer: &mut Mixer,
        scene: &SceneState,
        restored: u64,
        pool: &TaskPool,
        nonblocking: bool,
        mut storage: impl FnMut(ClientId) -> Result<Arc<dyn ClientStorage>>,
    ) -> Result<()> {
        if self.restored.is_some_and(|old| old != restored) {
            self.reset(mixer);
        }
        self.restored = Some(restored);
        let stale = self
            .voices
            .keys()
            .filter(|id| !scene.voices().contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        for id in stale {
            let v = self.voices.remove(&id).unwrap();
            if let Some(t) = v.ticket {
                self.jobs.cancel(id.client.0, t)?;
            }
            if let Some(h) = v.handle {
                if mixer.is_playing(id.client.0, h) {
                    mixer.stop(id.client.0, h, 0.)?;
                }
            }
        }
        self.routes.retain(|id, _| scene.voices().contains_key(id));
        for (id, voice) in scene.voices() {
            let epoch = scene.voice_epoch(*id);
            let v = self.voices.entry(*id).or_insert(Voice {
                epoch,
                published_epoch: None,
                handle: None,
                ticket: None,
                error: None,
            });
            if v.epoch != epoch {
                if let Some(t) = v.ticket.take() {
                    self.jobs.cancel(id.client.0, t)?;
                }
                v.epoch = epoch;
                v.error = None;
            }
            // A stopped/stolen/ended handle remains an observed epoch. A new
            // Play command, rather than the declaration persisting, restarts it.
            let current_handle = v.handle;
            if let Some(h) = current_handle {
                if mixer.is_playing(id.client.0, h) {
                    mixer.set_gain(id.client.0, h, voice.gain_milli as f32 / 1000.)?;
                    mixer.set_voice_bus(
                        id.client.0,
                        h,
                        self.routes.get(id).copied().unwrap_or_default(),
                    )?;
                }
            }
            // Track publication epoch separately from the currently desired one.
            let needs = v.published_epoch != Some(epoch);
            if needs && v.ticket.is_none() && v.error.is_none() {
                if nonblocking && self.jobs.admit(id.client.0).is_err() {
                    continue;
                }
                if self
                    .budget
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                        n.checked_add(STREAM_BYTES).filter(|n| *n <= HOST_BYTES)
                    })
                    .is_err()
                {
                    continue;
                }
                let charge = Arc::new(Charge {
                    counter: self.budget.clone(),
                });
                let authority = storage(id.client)?;
                let input = AudioAsset {
                    path: voice.asset.clone(),
                    frame_limit: None,
                };
                let p = AudioParameters {
                    gain: voice.gain_milli as f32 / 1000.,
                    looping: voice.looping,
                    bus: pcm_bus(self.routes.get(id).copied().unwrap_or_default()),
                    ..Default::default()
                };
                let workers = pool.clone();
                let work = move |cancel: Cancellation| -> Result<PreparedChannel> {
                    cancel.check()?;
                    let mut ready = prepare(asset(input, 1., authority.as_ref())?, None, p)?;
                    if nonblocking {
                        let mut src = BufferedSource::prepare_cancellable(
                            ready.source,
                            workers,
                            StreamBufferSpec::default(),
                            None,
                            &cancel,
                        )?;
                        src.retain_storage(charge)?;
                        ready.source = Box::new(src);
                    }
                    cancel.check()?;
                    Ok(ready)
                };
                if nonblocking {
                    v.ticket = Some(self.jobs.submit_deferred(
                        id.client.0,
                        restored,
                        "scene_audio",
                        pool,
                        work,
                    )?);
                } else {
                    let result = work(Cancellation::default()).and_then(|ready| {
                        publish(
                            mixer,
                            v,
                            *id,
                            voice.gain_milli,
                            self.routes.get(id).copied().unwrap_or_default(),
                            self.paused,
                            ready,
                        )
                    });
                    if let Err(e) = result {
                        v.error = Some(format!("{e:#}"));
                    }
                }
            }
        }
        let ids = self.voices.keys().copied().collect::<Vec<_>>();
        let mut failure = None;
        for id in ids {
            let v = self.voices.get_mut(&id).unwrap();
            if let Some(t) = v.ticket {
                let status = self.jobs.status(id.client.0, restored, t)?;
                if status.state == OperationState::Pending {
                    continue;
                }
                v.ticket = None;
                if status.state == OperationState::Failed {
                    v.error = status.error;
                    self.jobs.cancel(id.client.0, t)?;
                } else {
                    let ready = self.jobs.take(id.client.0, restored, t)?.unwrap();
                    let declaration = &scene.voices()[&id];
                    if let Err(e) = publish(
                        mixer,
                        v,
                        id,
                        declaration.gain_milli,
                        self.routes.get(&id).copied().unwrap_or_default(),
                        self.paused,
                        ready,
                    ) {
                        v.error = Some(format!("{e:#}"));
                    }
                }
            }
            if let Some(e) = &v.error {
                failure = Some(format!(
                    "canonical audio client {} channel {}: {e}",
                    id.client.0, id.local
                ));
            }
        }

        if let Some(e) = failure {
            anyhow::bail!("{e}");
        }
        Ok(())
    }
    /// Inspect the current declaration and last collected playback state.
    pub fn status(&self, mixer: &Mixer, scene: &SceneState, id: ObjectId) -> SceneVoiceStatus {
        let epoch = scene.voice_epoch(id);
        let mut out = SceneVoiceStatus {
            client: id.client.0,
            channel: id.local,
            epoch,
            phase: SceneVoicePhase::Stopped,
            position_seconds: None,
            error: None,
        };
        if !scene.voices().contains_key(&id) {
            return out;
        }
        out.phase = SceneVoicePhase::Queued;
        if let Some(v) = self.voices.get(&id).filter(|v| v.epoch == epoch) {
            if v.error.is_none() && v.published_epoch == Some(epoch) {
                if let Some(error) = v
                    .handle
                    .and_then(|h| mixer.source_status(id.client.0, h).ok().flatten())
                    .and_then(|s| s.error)
                {
                    out.phase = SceneVoicePhase::Failed;
                    out.error = Some(error);
                    return out;
                }
            }
            out.error = v.error.clone();
            out.phase = if v.error.is_some() {
                SceneVoicePhase::Failed
            } else if v.ticket.is_some() {
                SceneVoicePhase::Loading
            } else if v.published_epoch != Some(epoch) {
                SceneVoicePhase::Queued
            } else if let Some(h) = v.handle {
                if mixer.is_playing(id.client.0, h) {
                    out.position_seconds = mixer.position(id.client.0, h).ok();
                    if mixer.spec(id.client.0, h).is_ok_and(|s| s.paused) {
                        SceneVoicePhase::Paused
                    } else {
                        SceneVoicePhase::Playing
                    }
                } else {
                    SceneVoicePhase::Ended
                }
            } else {
                SceneVoicePhase::Queued
            };
        }
        out
    }
}
