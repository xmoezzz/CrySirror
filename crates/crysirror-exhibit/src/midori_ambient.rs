// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 ordinary single-loop ambience (S30 bit 18 and packed S32).
use crate::{
    branch::number,
    midori_sound_capture::Lane,
    registers::{Registers, Value},
    sound::packed_stereo,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AudioAsset, AudioParameters, AudioPcm16Gain, AudioPreparePhase, AudioRequest, HostServices,
        PcmBus,
    },
    state::ClientId,
};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq)]
pub(super) struct Definition {
    id: i32,
    source: i32,
    levels: [i32; 2],
}

fn valid_level(value: i32) -> bool {
    (0..=256).contains(&value)
}

impl Definition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let args: Vec<_> = raw
            .split(|&b| b == b',')
            .take(22)
            .map(number)
            .collect::<Result<_>>()?;
        ensure!(
            args.len() == 21
                && (0..=999).contains(&args[0])
                && (0..=999).contains(&args[1])
                && args[2..4] == [2, 0]
                && args[4..6].iter().copied().all(valid_level)
                && args[6..].iter().all(|&v| v == -1),
            "unsupported ExHibit 2011 ambient definition"
        );
        Ok(Self {
            id: args[0],
            source: args[1],
            levels: [args[4], args[5]],
        })
    }

    pub fn path(&self) -> Result<VfsPath> {
        Ok(VfsPath::new(
            Mount::Asset,
            &format!("res/s/e/{:03}.wav", self.source),
        )?)
    }

    #[cfg(test)]
    pub fn id(&self) -> i32 {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Command {
    Play {
        id: i32,
        fade: u32,
        overrides: [i32; 2],
    },
    Stop(u32),
}

impl Command {
    pub fn parse(op: u16, flags: u8, args: &[i32]) -> Result<Self> {
        match (op, flags, args) {
            (0x34, 0, [id, fade, l, r])
                if (0..=999).contains(id)
                    && (0..=600_000).contains(fade)
                    && [*l, *r].into_iter().all(|v| v == -1 || valid_level(v)) =>
            {
                Ok(Self::Play {
                    id: *id,
                    fade: *fade as u32,
                    overrides: [*l, *r],
                })
            }
            (0x35, 0, [fade]) if (0..=600_000).contains(fade) => Ok(Self::Stop(*fade as u32)),
            _ => bail!("unsupported ExHibit 2011 ambient command"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Active {
    channel: u32,
    id: i32,
}

struct Preparing {
    ticket: u64,
    voice: Active,
    requested: Lane,
    fade: u32,
}

#[derive(Default)]
pub(super) struct Ambient {
    definitions: BTreeMap<i32, Definition>,
    requested: Lane,
    active: Option<Active>,
    last_bank: Option<u32>,
    preparing: Option<Preparing>,
    retiring: Option<u32>,
}

impl Ambient {
    pub fn define(&mut self, definition: Definition) {
        self.definitions.insert(definition.id, definition);
    }

    pub fn capture_lane(&self) -> Option<Lane> {
        (self.preparing.is_none() && self.retiring.is_none()).then_some(self.requested)
    }

    fn stop(h: &mut dyn HostServices, c: ClientId, channel: u32, ms: u32) -> Result<()> {
        h.submit_audio(
            c,
            AudioRequest::Stop {
                channel,
                fade_ms: ms as f32,
            },
        )?;
        Ok(())
    }

    pub fn issue(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        r: &Registers,
        command: Command,
        skip_fade: bool,
    ) -> Result<bool> {
        ensure!(
            self.preparing.is_none() && self.retiring.is_none(),
            "ExHibit ambient command while pending"
        );
        match command {
            Command::Play {
                id,
                fade,
                overrides,
            } => {
                if self.active.is_some_and(|a| a.id == id) {
                    return Ok(false);
                }
                let definition = self
                    .definitions
                    .get(&id)
                    .context("undefined ExHibit 2011 ambient loop")?;
                if r.read(Value::System(30))? & 0x4_0000 != 0 {
                    self.requested = Lane::ambience(id, overrides);
                    return Ok(false);
                }
                let levels = std::array::from_fn(|i| {
                    if overrides[i] >= 0 {
                        overrides[i]
                    } else {
                        definition.levels[i]
                    }
                });
                let channel = if self.last_bank == Some(4) { 8 } else { 4 };
                let fade = if skip_fade { 0 } else { fade };
                let ticket = h.begin_audio(
                    c,
                    AudioRequest::PlayAsset {
                        channel,
                        source: AudioAsset {
                            path: definition.path()?,
                            frame_limit: None,
                        },
                        intro: None,
                        parameters: AudioParameters {
                            bus: PcmBus::Ambient,
                            looping: true,
                            fade_ms: fade as f32,
                            pcm16_gain: Some(AudioPcm16Gain {
                                stereo: packed_stereo(r, 32, levels)?,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    },
                )?;
                self.preparing = Some(Preparing {
                    ticket,
                    voice: Active { channel, id },
                    requested: Lane::ambience(id, overrides),
                    fade,
                });
                Ok(true)
            }
            Command::Stop(fade) => {
                let Some(active) = self.active else {
                    self.requested = Lane::stop_ambience();
                    return Ok(false);
                };
                let fade = if skip_fade { 0 } else { fade };
                Self::stop(h, c, active.channel, fade)?;
                self.requested = Lane::stop_ambience();
                self.active = None;
                self.retiring = (fade > 0).then_some(active.channel);
                Ok(self.retiring.is_some())
            }
        }
    }

    pub fn poll(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        if let Some(p) = &self.preparing {
            let state = h.audio_preparation(c, p.ticket)?;
            if state.phase == AudioPreparePhase::Pending {
                return Ok(false);
            }
            h.release_audio(c, p.ticket)?;
            let p = self
                .preparing
                .take()
                .context("missing ExHibit ambient request")?;
            if state.phase == AudioPreparePhase::Failed {
                bail!("ExHibit 2011 ambient preparation failed: {:?}", state.error);
            }
            self.requested = p.requested;
            let old = self.active.replace(p.voice);
            self.last_bank = Some(p.voice.channel);
            if let Some(old) = old {
                Self::stop(h, c, old.channel, p.fade)?;
                self.retiring = (p.fade > 0).then_some(old.channel);
                // Observe the stop command after one host mixer turn.
                if self.retiring.is_some() {
                    return Ok(false);
                }
            }
        }
        if let Some(channel) = self.retiring {
            if h.audio_info(c, channel)?
                .is_some_and(|a| a.stopping && a.fade_out_frames > 0)
            {
                return Ok(false);
            }
            self.retiring = None;
        }
        Ok(true)
    }
}
