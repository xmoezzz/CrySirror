// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Atomic live-source retirement and external-update logarithmic release.
use crate::{channels::ChannelAudio, mixer::VoiceHandle};
use anyhow::{ensure, Context, Result};
pub use crysirror_core::service::{
    AudioReleaseHandle as Handle, AudioReleaseInfo as Info, AudioReleasePhase as Phase,
    AudioReleaseSpec as Spec,
};
use crysirror_core::{service::AudioRequest, state::ClientId};
use std::collections::BTreeMap;
#[derive(Default)]
pub(crate) struct Releases {
    entries: BTreeMap<u64, Entry>,
}
struct Entry {
    owner: u32,
    spec: Spec,
    info: Info,
    voice: Option<VoiceHandle>,
    current: f32,
    target: f32,
    step: f32,
}
impl Entry {
    fn advance(&mut self, mixer: &mut crate::mixer::Mixer) -> Result<()> {
        if matches!(self.info.phase, Phase::Complete | Phase::Cancelled) {
            self.info.completed_updates = self.info.submitted_updates;
            return Ok(());
        }
        let h = self.voice.context("release voice missing")?;
        while self.info.completed_updates < self.info.submitted_updates {
            self.info.completed_updates += 1;
            self.current += self.step;
            let radius = self.step.abs();
            if f64::from(self.target) >= f64::from(self.current) - f64::from(radius)
                && f64::from(self.target) <= f64::from(self.current) + f64::from(radius)
            {
                self.current = self.target;
            }
            if self.current == self.target || self.step == 0. {
                mixer.stop(self.owner, h, 0.)?;
                self.voice = None;
                self.info.phase = Phase::Complete;
                self.info.volume_db100 = Some(self.target as i32);
                self.info.stopped_at_update = Some(self.info.completed_updates);
                self.info.completed_updates = self.info.submitted_updates;
                break;
            }
            let db = self.current as i32;
            mixer.release_db100(self.owner, h, db)?;
            self.info.volume_db100 = Some(db);
        }
        Ok(())
    }
}
impl ChannelAudio {
    /// Retire source into a separate nonlooping, running tail. Prepared source
    /// intents on both channels are invalidated; live PCM/decoder cursor is moved.
    pub fn begin_audio_release(&mut self, _client: u32, spec: Spec) -> Result<Handle> {
        spec.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::bail!("audio stepped release producer is not yet verified; native control remains unsupported")
    }
    // Incomplete producer retained for the next owner task. Do not expose this
    // admission path until offline PCM and worker-ownership oracles pass.
    #[allow(dead_code)]
    fn admit_release_unverified(&mut self, client: u32, spec: Spec) -> Result<Handle> {
        spec.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(self.clients.contains(&client), "release client missing");
        ensure!(
            self.releases.entries.len() < 1024
                && self
                    .releases
                    .entries
                    .values()
                    .filter(|e| e.owner == client)
                    .count()
                    < 128,
            "release capacity"
        );
        if !spec.release_tail_when_source_missing {
            ensure!(
                self.voices
                    .get(&(client, spec.source_channel))
                    .is_some_and(|&h| self.mixer.is_playing(client, h)),
                "release source missing"
            );
        }
        for channel in [spec.source_channel, spec.tail_channel] {
            self.cancel_pending_control(
                client,
                &AudioRequest::Stop {
                    channel,
                    fade_ms: 0.,
                },
            )?;
        }
        let generation = crysirror_tasks::table::next_operation_id()?;
        self.releases.entries.insert(
            generation,
            Entry {
                owner: client,
                spec,
                info: Info {
                    phase: Phase::Pending,
                    submitted_updates: 0,
                    completed_updates: 0,
                    volume_db100: None,
                    source_position_frames: None,
                    stopped_at_update: None,
                },
                voice: None,
                current: 0.,
                target: 0.,
                step: 0.,
            },
        );
        // Admission is distinct from publication. A worker lease remains Pending.
        self.poll_releases()?;
        Ok(Handle {
            client: ClientId(client),
            generation,
        })
    }
    pub fn advance_audio_release(&mut self, client: u32, h: Handle, elapsed: u64) -> Result<()> {
        ensure!(h.client.0 == client, "foreign audio release handle");
        let e = self
            .releases
            .entries
            .get_mut(&h.generation)
            .filter(|e| e.owner == client)
            .context("stale audio release handle")?;
        ensure!(
            elapsed >= e.info.submitted_updates && elapsed <= 1_000_000_000,
            "release elapsed backwards/out of range"
        );
        e.info.submitted_updates = elapsed;
        self.poll_releases()
    }
    pub fn audio_release_info(&mut self, client: u32, h: Handle) -> Result<Info> {
        ensure!(h.client.0 == client, "foreign audio release handle");
        self.poll_releases()?;
        Ok(self
            .releases
            .entries
            .get(&h.generation)
            .filter(|e| e.owner == client)
            .context("stale audio release handle")?
            .info)
    }
    pub fn release_audio_release(&mut self, client: u32, h: Handle) -> Result<()> {
        let i = self.audio_release_info(client, h)?;
        ensure!(
            matches!(i.phase, Phase::Complete | Phase::Cancelled),
            "release producer still active"
        );
        self.releases.entries.remove(&h.generation);
        Ok(())
    }
    /// Restore requires rebuilding every live source and producer first.
    pub fn has_release_state(&self) -> bool {
        self.releases
            .entries
            .values()
            .any(|e| matches!(e.info.phase, Phase::Pending | Phase::Active))
    }
    pub fn poll_releases(&mut self) -> Result<()> {
        if self.mixer.is_rendering() {
            return Ok(());
        }
        let ids = self.releases.entries.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let mut e = self.releases.entries.remove(&id).unwrap();
            let result = (|| -> Result<()> {
                if e.info.phase == Phase::Pending {
                    let src = self
                        .voices
                        .get(&(e.owner, e.spec.source_channel))
                        .copied()
                        .filter(|&h| self.mixer.is_playing(e.owner, h));
                    let dst = self
                        .voices
                        .get(&(e.owner, e.spec.tail_channel))
                        .copied()
                        .filter(|&h| self.mixer.is_playing(e.owner, h));
                    let chosen = src.or(if e.spec.release_tail_when_source_missing {
                        dst
                    } else {
                        None
                    });
                    if let Some(h) = chosen {
                        let (start, position) =
                            self.mixer
                                .release_start(e.owner, h, e.spec.freeze_group_gain)?;
                        let target = if start >= e.spec.threshold_db100 {
                            e.spec.target_above_db100
                        } else {
                            e.spec.target_below_db100
                        };
                        ensure!(
                            target <= start,
                            "release target would amplify captured gain"
                        );
                        if let Some(old) = dst.filter(|old| Some(*old) != src && src.is_some()) {
                            self.mixer.stop(e.owner, old, 0.)?;
                        }
                        for old in self.releases.entries.values_mut().filter(|old| {
                            old.owner == e.owner
                                && old.spec.tail_channel == e.spec.tail_channel
                                && old.info.phase == Phase::Active
                        }) {
                            old.info.phase = Phase::Cancelled;
                            old.voice = None;
                        }
                        self.mixer.capture_release(e.owner, h, &e.spec)?;
                        if src.is_some() {
                            self.voices.remove(&(e.owner, e.spec.source_channel));
                            self.voices.insert((e.owner, e.spec.tail_channel), h);
                        }
                        e.voice = Some(h);
                        e.current = start as f32;
                        e.target = target as f32;
                        e.step = ((f64::from(target) - f64::from(start)) / f64::from(e.spec.steps))
                            as f32;
                        e.info.phase = Phase::Active;
                        e.info.volume_db100 = Some(start);
                        e.info.source_position_frames = Some(position);
                        if start == -10000 {
                            self.mixer.stop(e.owner, h, 0.)?;
                            e.voice = None;
                            e.info.phase = Phase::Complete;
                            e.info.stopped_at_update = Some(0);
                        }
                    } else {
                        ensure!(
                            e.spec.release_tail_when_source_missing,
                            "release source disappeared before publication"
                        );
                        e.info.phase = Phase::Complete;
                        e.info.stopped_at_update = Some(0);
                    }
                }
                if e.info.phase == Phase::Active {
                    let h = e.voice.unwrap();
                    if self.voices.get(&(e.owner, e.spec.tail_channel)) != Some(&h)
                        || !self.mixer.is_playing(e.owner, h)
                    {
                        e.voice = None;
                        e.info.phase = Phase::Cancelled;
                    } else if !self.mixer.source_is_playing(e.owner, h) {
                        self.mixer.stop(e.owner, h, 0.)?;
                        e.voice = None;
                        e.info.phase = Phase::Complete;
                        e.info.stopped_at_update = Some(e.info.completed_updates);
                    } else {
                        e.info.source_position_frames =
                            Some(self.mixer.playback_info(e.owner, h)?.position_frames);
                    }
                }
                if e.info.phase != Phase::Pending {
                    e.advance(&mut self.mixer)?;
                }
                Ok(())
            })();
            self.releases.entries.insert(id, e);
            result?;
        }
        Ok(())
    }
    pub(super) fn cancel_release_channel(&mut self, client: u32, channel: u32) {
        for e in self.releases.entries.values_mut().filter(|e| {
            e.owner == client
                && e.spec.tail_channel == channel
                && matches!(e.info.phase, Phase::Pending | Phase::Active)
        }) {
            e.info.phase = Phase::Cancelled;
            e.voice = None;
        }
    }
}
