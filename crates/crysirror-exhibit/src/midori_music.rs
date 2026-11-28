// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 ordinary 44.1 kHz stereo BGM owner and literal loop transport.
use crate::{
    branch::number,
    midori_sound_capture::Lane,
    registers::{Bank, Change, Registers, Value},
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Track {
    pub id: i32,
    fade: u32,
    levels: [i32; 2],
    pub loop_start: u64,
}

impl Track {
    pub fn path(self) -> Result<VfsPath> {
        Ok(VfsPath::new(
            Mount::Asset,
            &format!("res/s/m/m{:02}.ogg", self.id),
        )?)
    }

    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw
            .splitn(10, |&b| b == b',')
            .map(number)
            .collect::<Result<_>>()?;
        let [id, fade, left, right, 1, 0, 0, start, -1] = fields.as_slice() else {
            bail!("unsupported Midori BGM transport variant");
        };
        ensure!(
            (1..=17).contains(id)
                && (0..=600_000).contains(fade)
                && (0..=256).contains(left)
                && (0..=256).contains(right)
                && (0..=i32::MAX / 4).contains(start),
            "unsupported Midori BGM literal operands"
        );
        // cmdPlayMusic multiplies decoded stereo frame indices by four.
        // a13 (startup) and a15 (loop end) are zero for these authored
        // records after native clamping; a14 (loop restart) is byte-counted.
        Ok(Self {
            id: *id,
            fade: *fade as u32,
            levels: [*left, *right],
            loop_start: *start as u64,
        })
    }
}

pub(super) fn stop_duration(ms: i32, wait: i32) -> Result<u32> {
    ensure!(
        wait == 0 && (0..=600_000).contains(&ms),
        "unsupported Midori BGM stop mode"
    );
    Ok(ms as u32)
}

#[derive(Clone, Copy)]
struct Live {
    track: Track,
    lane: u32,
}

struct Preparing {
    ticket: u64,
    candidate: Live,
    fade: u32,
}

#[derive(Default)]
pub(super) struct Music {
    audible: Option<Live>,
    requested: Lane,
    last_lane: Option<u32>,
    preparing: Option<Preparing>,
    retiring: Option<u32>,
}

fn retire(host: &mut dyn HostServices, c: ClientId, lane: u32, fade: u32) -> Result<()> {
    host.submit_audio(
        c,
        AudioRequest::Stop {
            channel: lane,
            fade_ms: fade as f32,
        },
    )?;
    Ok(())
}

impl Music {
    pub fn preparing(&self) -> bool {
        self.preparing.is_some()
    }

    pub fn ids(&self) -> [Option<i32>; 2] {
        [
            (self.requested.id >= 0).then_some(self.requested.id),
            self.audible.map(|live| live.track.id),
        ]
    }

    pub fn live_volume(&mut self) -> Option<(u32, &mut Lane)> {
        let live = self.audible?;
        Some((live.lane, &mut self.requested))
    }

    pub fn capture_lane(&self) -> Option<Lane> {
        (self.preparing.is_none() && self.retiring.is_none()).then_some(self.requested)
    }

    pub fn play(
        &mut self,
        host: &mut dyn HostServices,
        c: ClientId,
        regs: &Registers,
        track: Track,
        skip_fade: bool,
    ) -> Result<bool> {
        ensure!(
            self.preparing.is_none() && self.retiring.is_none(),
            "overlapping Midori BGM request"
        );
        if self.audible.is_some_and(|live| live.track.id == track.id) {
            return Ok(false);
        }
        if regs.read(Value::System(30))? & 0x1_0000 != 0 {
            self.requested = Lane::music(track.id, track.levels);
            return Ok(false);
        }
        let lane = if self.last_lane == Some(0) { 1 } else { 0 };
        let fade = if skip_fade { 0 } else { track.fade };
        let ticket = host.begin_audio(
            c,
            AudioRequest::PlayAsset {
                channel: lane,
                source: AudioAsset {
                    path: track.path()?,
                    frame_limit: None,
                },
                intro: None,
                parameters: AudioParameters {
                    bus: PcmBus::Music,
                    looping: true,
                    loop_start: track.loop_start,
                    fade_ms: fade as f32,
                    pcm16_gain: Some(AudioPcm16Gain {
                        stereo: packed_stereo(regs, 31, track.levels)?,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
        )?;
        self.preparing = Some(Preparing {
            ticket,
            candidate: Live { track, lane },
            fade,
        });
        Ok(true)
    }

    pub fn stop(
        &mut self,
        host: &mut dyn HostServices,
        c: ClientId,
        fade: u32,
        skip_fade: bool,
    ) -> Result<bool> {
        ensure!(
            self.preparing.is_none() && self.retiring.is_none(),
            "overlapping Midori BGM stop"
        );
        if let Some(live) = self.audible {
            let fade = if skip_fade { 0 } else { fade };
            retire(host, c, live.lane, fade)?;
            self.retiring = (fade != 0).then_some(live.lane);
            self.audible = None;
        }
        self.requested = Lane::stop_music();
        Ok(self.retiring.is_some())
    }

    pub fn poll(
        &mut self,
        host: &mut dyn HostServices,
        c: ClientId,
        regs: &mut Registers,
    ) -> Result<bool> {
        if let Some(pending) = &self.preparing {
            let status = host.audio_preparation(c, pending.ticket)?;
            if status.phase == AudioPreparePhase::Pending {
                return Ok(false);
            }
            host.release_audio(c, pending.ticket)?;
            let pending = self
                .preparing
                .take()
                .context("missing Midori music preparation")?;
            if status.phase == AudioPreparePhase::Failed {
                bail!("Midori BGM preparation failed: {:?}", status.error);
            }
            let previous = self.audible.replace(pending.candidate);
            let track = pending.candidate.track;
            self.requested = Lane::music(track.id, track.levels);
            self.last_lane = Some(pending.candidate.lane);
            regs.change(Change::Experience(
                Bank::Sound,
                vec![pending.candidate.track.id],
            ))?;
            if let Some(previous) = previous {
                retire(host, c, previous.lane, pending.fade)?;
                if pending.fade != 0 {
                    self.retiring = Some(previous.lane);
                    return Ok(false);
                }
            }
        }
        if let Some(lane) = self.retiring {
            if host
                .audio_info(c, lane)?
                .is_some_and(|info| info.stopping && info.fade_out_frames > 0)
            {
                return Ok(false);
            }
            self.retiring = None;
        }
        Ok(true)
    }
}
