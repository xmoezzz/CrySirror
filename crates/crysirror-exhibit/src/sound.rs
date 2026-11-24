// SPDX-License-Identifier: MPL-2.0
//! Native RLD v1 music, loop-effect and one-shot channel policy.
use crate::registers::{Bank, Change, Registers, Value};
use crate::voice;
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::{InputFrame, buttons};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AudioAsset, AudioParameters, AudioPcm16Gain, AudioPcm16GainUpdate, AudioPreparePhase,
        AudioRequest, HostServices, PcmBus,
    },
    state::ClientId,
};
use crysirror_tasks::TaskPool;

// Retouch's 257-entry amplitude curve, indexed after applying the system
// channel volume. Values at either endpoint are valid coefficients.
const LEVELS: [u16; 257] = [
    0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 6, 7, 7, 8, 9, 9, 10,
    10, 11, 11, 12, 13, 13, 14, 15, 16, 16, 17, 18, 19, 20, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43, 44, 45, 46, 47, 49, 50, 51, 52, 54, 55, 56,
    57, 59, 60, 61, 63, 64, 65, 67, 68, 70, 71, 72, 74, 75, 77, 78, 80, 81, 82, 84, 85, 87, 88, 90,
    91, 93, 94, 96, 97, 99, 100, 102, 104, 105, 107, 108, 110, 111, 113, 114, 116, 118, 119, 121,
    122, 124, 125, 127, 129, 130, 132, 133, 135, 136, 138, 139, 141, 143, 144, 146, 147, 149, 150,
    152, 153, 155, 157, 158, 160, 161, 163, 164, 166, 167, 169, 170, 172, 173, 175, 176, 177, 179,
    180, 182, 183, 185, 186, 187, 189, 190, 192, 193, 194, 196, 197, 198, 200, 201, 202, 203, 205,
    206, 207, 208, 210, 211, 212, 213, 214, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226,
    227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 237, 238, 239, 240, 241, 241, 242, 243,
    244, 244, 245, 246, 246, 247, 247, 248, 248, 249, 250, 250, 251, 251, 251, 252, 252, 253, 253,
    253, 254, 254, 254, 255, 255, 255, 255, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256,
    256,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Sound {
    Environment {
        id: i32,
        fade: i32,
        levels: [i32; 2],
    },
    StopEnvironment(i32),
    Music {
        id: i32,
        fade: i32,
        levels: [i32; 2],
        looping: bool,
    },
    Loop {
        id: i32,
        fade: i32,
        levels: [i32; 2],
    },
    Effect {
        id: i32,
        levels: [i32; 2],
        slot: i32,
        wait: bool,
    },
    Voice {
        id: i32,
        levels: [i32; 2],
        wait: bool,
    },
    LeaveVoice(i32),
    StopMusic {
        fade: i32,
        wait: bool,
    },
    StopLoop(i32),
    LeaveEffect {
        fade: i32,
        slot: i32,
    },
}
pub(super) fn decode(op: u16, flags: u8, args: &[i32]) -> Result<Sound> {
    Ok(match (op, flags, args) {
        (0x34, 0, [id, fade, l, r]) => Sound::Environment {
            id: *id,
            fade: *fade,
            levels: [*l, *r],
        },
        (0x35, 0, [fade]) => Sound::StopEnvironment(*fade),
        (0x32, 0, [id, fade, l, r, looping, 0]) => Sound::Music {
            id: *id,
            fade: *fade,
            levels: [*l, *r],
            looping: *looping != 0,
        },
        (0x33, 0, [fade, wait]) => Sound::StopMusic {
            fade: *fade,
            wait: *wait != 0,
        },
        (0x36, 0, [id, fade, l, r]) => Sound::Loop {
            id: *id,
            fade: *fade,
            levels: [*l, *r],
        },
        (0x37, 0, [fade]) => Sound::StopLoop(*fade),
        (0x38 | 0x3a, 0x10, [id, l, r, _, 1]) => Sound::Voice {
            id: *id,
            levels: [*l, *r],
            wait: op == 0x3a,
        },
        (0x39, 0x10, [fade, _, 1]) => Sound::LeaveVoice(*fade),
        (0x38 | 0x3a, 0, [id, l, r, slot]) | (0x38 | 0x3a, 0x10, [id, l, r, slot, 0]) => {
            Sound::Effect {
                id: *id,
                levels: [*l, *r],
                slot: *slot,
                wait: op == 0x3a,
            }
        }
        (0x39, 0, [fade, slot]) | (0x39, 0x10, [fade, slot, 0]) => Sound::LeaveEffect {
            fade: *fade,
            slot: *slot,
        },
        _ => bail!("unsupported v1 audio opcode {op:#x} flags {flags:#x} operands {args:?}"),
    })
}

/// The ordinary 2011 PCM effect commands, not its voice/extension modes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Effect2011 {
    Play {
        id: i32,
        levels: [i32; 2],
        slot: i32,
        sync: bool,
    },
    Leave {
        slot: i32,
        wait: bool,
    },
}

pub(super) fn effect2011(op: u16, flags: u8, args: &[i32]) -> Result<Effect2011> {
    match (op, flags, args) {
        (0x38 | 0x3a, 0x10, [id, l, r, slot, 0])
            if *id >= 0
                && (0..=256).contains(l)
                && (0..=256).contains(r)
                && (0..=2).contains(slot) =>
        {
            Ok(Effect2011::Play {
                id: *id,
                levels: [*l, *r],
                slot: *slot,
                sync: op == 0x3a,
            })
        }
        (0x39, 0x10, [fade @ (-1 | 0), slot, 0]) if (0..=2).contains(slot) => {
            Ok(Effect2011::Leave {
                slot: *slot,
                wait: *fade == 0,
            })
        }
        _ => bail!("unsupported ExHibit 2011 effect opcode {op:#x} mode {flags:#x}"),
    }
}
const MUSIC: u32 = 0;

#[derive(Clone, Copy)]
enum Finish {
    Continue,
    WaitForEnd(u32),
    WaitEffect2011,
    WaitFade(u32),
}
struct Pending {
    ticket: u64,
    channel: u32,
    finish: Finish,
    seen: Option<i32>,
}
enum FadeWait {
    Start { channel: u32, ms: u32 },
    Stop(u32),
}

struct EffectWait2011 {
    channel: u32,
    began: u64,
    next: u64,
}

impl EffectWait2011 {
    fn new(channel: u32, now: u64) -> Self {
        Self {
            channel,
            began: now,
            next: now,
        }
    }

    fn tick(&mut self, h: &mut dyn HostServices, c: ClientId, input: InputFrame) -> Result<bool> {
        let now = h.now_ms();
        if now < self.next {
            return Ok(false);
        }
        let age = now.saturating_sub(self.began);
        // Native effect wait checks Ctrl on entry and then in its 100 ms
        // wait loop. The first other input check follows its sixth sleep.
        let valid_input = if h.ui_input_owned(c).unwrap_or(true) {
            0
        } else {
            input.buttons_down
        };
        let secondary = buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT;
        if valid_input & buttons::CTRL != 0
            || age >= 600 && valid_input & secondary != 0
            || !h.audio_playing(c, self.channel)?
        {
            h.submit_audio(
                c,
                AudioRequest::Stop {
                    channel: self.channel,
                    fade_ms: 0.,
                },
            )?;
            return Ok(true);
        }
        let ticks = (age / 100).saturating_add(1);
        self.next = self.began.saturating_add(ticks.saturating_mul(100));
        Ok(false)
    }
}
pub(super) struct Audio {
    pub scene: [i32; 12],
    pub environment: crate::environment::Environment,
    music: Option<i32>,
    music_pcm16: [u32; 2],
    loop_effect: Option<i32>,
    loading: Option<Pending>,
    waiting_end: Option<u32>,
    effect_wait_2011: Option<EffectWait2011>,
    fade: Option<FadeWait>,
    voice: Option<voice::Voice>,
    mark_after_wait: Option<i32>,
}
impl Default for Audio {
    fn default() -> Self {
        Self {
            environment: Default::default(),
            scene: [0, -1, 0, 0, -1, 0, 0, -1, -1, -1, 214, 214],
            music: None,
            music_pcm16: [0; 2],
            loop_effect: None,
            loading: None,
            waiting_end: None,
            effect_wait_2011: None,
            fade: None,
            voice: None,
            mark_after_wait: None,
        }
    }
}
fn effect_channel(slot: i32) -> u32 {
    if slot == 1 { 15 } else { 14 }
}
pub(super) fn stereo(regs: &Registers, base: i32, levels: [i32; 2]) -> Result<[u32; 2]> {
    let mut stereo = [0; 2];
    for (i, level) in levels.into_iter().enumerate() {
        let n = level.wrapping_mul(regs.read(Value::System(base + i as i32))?) / 256;
        ensure!(n >= 0, "unsupported retained stereo level");
        stereo[i] = u32::from(LEVELS[n.min(256) as usize]);
    }
    Ok(stereo)
}
/// Packed 2011 master gain: high half left, low half right.
pub(super) fn packed_stereo(regs: &Registers, slot: i32, levels: [i32; 2]) -> Result<[u32; 2]> {
    let packed = regs.read(Value::System(slot))? as u32;
    let master = [packed >> 16, packed & 0xffff].map(|x| x as i32);
    Ok(std::array::from_fn(|i| {
        let corrected = master[i].wrapping_mul(levels[i]) / 256;
        u32::from(LEVELS[corrected.clamp(0, 256) as usize])
    }))
}
impl Audio {
    pub fn scene_music_level(
        &self,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<Option<[u32; 2]>> {
        Ok((self.music.is_some() && h.audio_playing(c, MUSIC)?).then_some(self.music_pcm16))
    }

    pub fn set_scene_music_level(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        level: [u32; 2],
    ) -> Result<()> {
        h.update_audio_pcm16_gain(c, MUSIC, AudioPcm16GainUpdate::Stereo(level))?;
        self.music_pcm16 = level;
        Ok(())
    }

    pub fn stop_scene_music(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<()> {
        self.scene[1..4].copy_from_slice(&[-1, 0, 0]);
        self.scene[0] &= !3;
        if self.music.take().is_some() {
            self.stop(h, c, MUSIC, 0, false)?;
        }
        self.music_pcm16 = [0; 2];
        Ok(())
    }

    pub fn preparation_pending(&self) -> bool {
        self.loading.is_some() || self.voice.is_some() || self.environment.preparing()
    }
    fn finish(&mut self, channel: u32, action: Finish, now: u64) {
        match action {
            Finish::Continue => {}
            Finish::WaitForEnd(ch) => self.waiting_end = Some(ch),
            Finish::WaitEffect2011 => {
                self.effect_wait_2011 = Some(EffectWait2011::new(channel, now))
            }
            Finish::WaitFade(ms) => self.fade = Some(FadeWait::Start { channel, ms }),
        }
    }
    pub fn poll(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        r: &mut Registers,
        input: InputFrame,
    ) -> Result<bool> {
        if !self.environment.poll(h, c)? {
            return Ok(false);
        }
        if let Some(v) = &mut self.voice {
            if !v.poll(h, c)? {
                return Ok(false);
            }
            if v.wait {
                self.waiting_end = Some(voice::CHANNEL);
                self.mark_after_wait = Some(v.id);
            } else {
                r.mark_message(v.id)?;
            }
            self.voice = None;
        }
        if let Some(p) = &self.loading {
            let state = h.audio_preparation(c, p.ticket)?;
            if matches!(state.phase, AudioPreparePhase::Pending) {
                return Ok(false);
            }
            h.release_audio(c, p.ticket)?;
            let p = self
                .loading
                .take()
                .context("missing ExHibit sound request")?;
            if matches!(state.phase, AudioPreparePhase::Failed) {
                bail!("ExHibit sound preparation failed: {:?}", state.error)
            }
            if let Some(id) = p.seen {
                r.change(Change::Experience(Bank::Sound, vec![id]))?;
            }
            self.finish(p.channel, p.finish, h.now_ms());
        }
        if let Some(fade) = &self.fade {
            let pending = match *fade {
                FadeWait::Start { channel, ms } => h.audio_info(c, channel)?.is_some_and(|a| {
                    a.age_frames < (u64::from(ms) * u64::from(a.output_rate)).div_ceil(1000)
                }),
                FadeWait::Stop(channel) => h
                    .audio_info(c, channel)?
                    .is_some_and(|a| a.stopping && a.fade_out_frames > 0),
            };
            if pending {
                return Ok(false);
            }
            self.fade = None;
        }
        if let Some(ch) = self.waiting_end {
            let mask = buttons::CTRL | buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT;
            if (input.buttons_down | input.buttons_pressed) & mask == 0 && h.audio_playing(c, ch)? {
                return Ok(false);
            }
            h.submit_audio(
                c,
                AudioRequest::Stop {
                    channel: ch,
                    fade_ms: 0.,
                },
            )?;
            self.waiting_end = None;
            if let Some(id) = self.mark_after_wait.take() {
                r.mark_message(id)?;
            }
        }
        if let Some(wait) = &mut self.effect_wait_2011 {
            if !wait.tick(h, c, input)? {
                return Ok(false);
            }
            self.effect_wait_2011 = None;
        }
        Ok(true)
    }
    pub fn issue_2011_effect(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        r: &Registers,
        cmd: Effect2011,
        skip: bool,
    ) -> Result<bool> {
        let slot = match cmd {
            Effect2011::Play { slot, .. } | Effect2011::Leave { slot, .. } => slot,
        };
        // Midori has three native lanes; 14/15 preserve the v1 host slot
        // IDs, and 16 is a separate lane not shared with voice or ambience.
        let channel = 14 + u32::try_from(slot)?;
        let stop = |h: &mut dyn HostServices| -> Result<()> {
            if h.audio_playing(c, channel)? {
                h.submit_audio(
                    c,
                    AudioRequest::Stop {
                        channel,
                        fade_ms: 0.,
                    },
                )?;
            }
            Ok(())
        };
        match cmd {
            Effect2011::Leave { wait, .. } => {
                if wait && !skip && h.audio_playing(c, channel)? {
                    self.effect_wait_2011 = Some(EffectWait2011::new(channel, h.now_ms()));
                    return Ok(true);
                } else {
                    stop(h)?;
                }
            }
            Effect2011::Play {
                id, levels, sync, ..
            } => {
                if skip || r.read(Value::System(30))? & 0x2_0000 != 0 {
                    return Ok(false);
                }
                let gain = packed_stereo(r, 33, levels)?;
                let source = AudioAsset {
                    path: VfsPath::new(Mount::Asset, &format!("res/s/e/{id:03}.wav"))?,
                    frame_limit: None,
                };
                stop(h)?;
                let ticket = h.begin_audio(
                    c,
                    AudioRequest::PlayAsset {
                        channel,
                        source,
                        intro: None,
                        parameters: AudioParameters {
                            bus: PcmBus::Sfx,
                            pcm16_gain: Some(AudioPcm16Gain {
                                stereo: gain,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    },
                )?;
                self.loading = Some(Pending {
                    ticket,
                    channel,
                    finish: if sync {
                        Finish::WaitEffect2011
                    } else {
                        Finish::Continue
                    },
                    seen: None,
                });
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn issue(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        r: &Registers,
        worker: &TaskPool,
        s: Sound,
    ) -> Result<()> {
        const LOOP: u32 = 12;
        let (channel, id, path, bus, levels, base, fade, looping, finish, seen) = match s {
            Sound::Environment { id, fade, levels } => {
                return self
                    .environment
                    .play(h, c, r, id, fade, levels, &mut self.scene);
            }
            Sound::StopEnvironment(fade) => {
                self.fade = self
                    .environment
                    .stop(h, c, fade, &mut self.scene)?
                    .map(FadeWait::Stop);
                return Ok(());
            }
            Sound::Voice { id, levels, wait } => {
                if id >= 0 {
                    self.voice = Some(voice::Voice::begin(
                        h,
                        worker,
                        c,
                        id,
                        wait,
                        stereo(r, 38, levels)?,
                    )?);
                } else if wait {
                    self.waiting_end = Some(voice::CHANNEL);
                }
                return Ok(());
            }
            Sound::LeaveVoice(fade) => return self.stop(h, c, voice::CHANNEL, fade, fade == 0),
            Sound::Music {
                id,
                fade,
                levels,
                looping,
            } => {
                if id < 0 || self.music == Some(id) {
                    return Ok(());
                }
                self.scene[1..4].copy_from_slice(&[id, levels[0], levels[1]]);
                self.scene[0] = (self.scene[0] & !3) | i32::from(looping);
                if r.read(Value::System(31))? != 0 {
                    return Ok(());
                }
                let replace = self.music.is_some();
                (
                    MUSIC,
                    id,
                    format!("res/s/m/m{id:02}.ogg"),
                    PcmBus::Music,
                    levels,
                    32,
                    fade,
                    looping,
                    if replace && fade > 0 {
                        Finish::WaitFade(fade as u32)
                    } else {
                        Finish::Continue
                    },
                    Some(id),
                )
            }
            Sound::Loop { id, fade, levels } => {
                if id < 0 || self.loop_effect == Some(id) {
                    return Ok(());
                }
                self.scene[4..7].copy_from_slice(&[id, levels[0], levels[1]]);
                if r.read(Value::System(37))? != 0 {
                    self.loop_effect = Some(id);
                    return Ok(());
                }
                (
                    LOOP,
                    id,
                    format!("res/s/e/{id:03}.wav"),
                    PcmBus::Sfx,
                    levels,
                    38,
                    fade,
                    true,
                    if fade > 0 {
                        Finish::WaitFade(fade as u32)
                    } else {
                        Finish::Continue
                    },
                    None,
                )
            }
            Sound::Effect {
                id,
                slot,
                levels,
                wait,
            } => {
                if id < 0 || r.read(Value::System(37))? != 0 {
                    return Ok(());
                }
                let ch = effect_channel(slot);
                (
                    ch,
                    id,
                    format!("res/s/e/{id:03}.wav"),
                    PcmBus::Sfx,
                    levels,
                    38,
                    0,
                    false,
                    if wait {
                        Finish::WaitForEnd(ch)
                    } else {
                        Finish::Continue
                    },
                    None,
                )
            }
            Sound::StopMusic { fade, wait } => {
                self.scene[1..4].copy_from_slice(&[-1, 0, 0]);
                self.scene[0] &= !3;
                if self.music.take().is_some() {
                    self.stop(h, c, MUSIC, fade, wait)?;
                }
                return Ok(());
            }
            Sound::StopLoop(fade) => {
                self.scene[4..7].copy_from_slice(&[-1, 0, 0]);
                if self.loop_effect.take().is_some() {
                    self.stop(h, c, LOOP, fade, false)?;
                }
                return Ok(());
            }
            Sound::LeaveEffect { slot, fade } => {
                return self.stop(h, c, effect_channel(slot), fade, fade == 0);
            }
        };
        let pcm16 = stereo(r, base, levels)?;
        let parameters = AudioParameters {
            bus,
            looping,
            fade_ms: fade.max(0) as f32,
            replace_fade_ms: fade.max(0) as f32,
            pcm16_gain: Some(AudioPcm16Gain {
                stereo: pcm16,
                ..Default::default()
            }),
            ..Default::default()
        };
        let ticket = h.begin_audio(
            c,
            AudioRequest::PlayAsset {
                channel,
                source: AudioAsset {
                    path: VfsPath::new(Mount::Asset, &path)?,
                    frame_limit: None,
                },
                intro: None,
                parameters,
            },
        )?;
        if channel == MUSIC {
            self.music = Some(id);
            self.music_pcm16 = pcm16;
        }
        if channel == LOOP {
            self.loop_effect = Some(id)
        }
        self.loading = Some(Pending {
            ticket,
            channel,
            finish,
            seen,
        });
        Ok(())
    }
    fn stop(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        channel: u32,
        fade: i32,
        wait: bool,
    ) -> Result<()> {
        if wait {
            self.waiting_end = Some(channel);
        } else if h.audio_playing(c, channel)? {
            h.submit_audio(
                c,
                AudioRequest::Stop {
                    channel,
                    fade_ms: fade.max(0) as f32,
                },
            )?;
            if fade > 0 {
                self.fade = Some(FadeWait::Stop(channel));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::native_state_tests::fresh;
    use crysirror_core::{
        Error,
        service::{AudioPreparation, FileSystem},
        state::Command,
    };
    #[derive(Default)]
    pub(crate) struct Host {
        pub(crate) requests: Vec<AudioRequest>,
        ready: bool,
        failing_preparation: bool,
        playing: bool,
        released: Vec<u64>,
        now: u64,
        ui_owns_input: bool,
        retiring_frames: Option<(u32, u64)>,
    }
    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn submit(&mut self, _: ClientId, _: &[Command]) -> crysirror_core::Result<()> {
            Ok(())
        }
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn ui_input_owned(&self, _: ClientId) -> crysirror_core::Result<bool> {
            Ok(self.ui_owns_input)
        }
        fn audio_playing(&self, _: ClientId, _: u32) -> crysirror_core::Result<bool> {
            Ok(self.playing)
        }
        fn audio_info(
            &self,
            _: ClientId,
            channel: u32,
        ) -> crysirror_core::Result<Option<crysirror_core::service::AudioPlaybackInfo>> {
            Ok(self
                .retiring_frames
                .filter(|x| x.0 == channel)
                .map(|(_, frames)| crysirror_core::service::AudioPlaybackInfo {
                    source_peak: 0.,
                    source_taps: [0.; 4],
                    balance: Default::default(),
                    source_playing: true,
                    first_pass_complete: false,
                    paused: false,
                    retain_on_finish: false,
                    transport_active: true,
                    looping: true,
                    loop_passes: None,
                    swap_channels: false,
                    age_frames: 0,
                    transport_frames: 0,
                    stopping: frames > 0,
                    pausing: false,
                    position_frames: 0,
                    source_rate: 22050,
                    source_frames: None,
                    output_rate: 48000,
                    fade_out_frames: frames,
                    resume_delay_frames: 0,
                    gain: 1.,
                    gain_target: 0.,
                    gain_fade_frames: 0,
                    gain_stop_after_fade: false,
                }))
        }
        fn begin_audio(&mut self, _: ClientId, r: AudioRequest) -> crysirror_core::Result<u64> {
            self.requests.push(r);
            Ok(self.requests.len() as u64)
        }
        fn submit_audio(&mut self, _: ClientId, r: AudioRequest) -> crysirror_core::Result<()> {
            self.requests.push(r);
            Ok(())
        }
        fn audio_preparation(
            &mut self,
            _: ClientId,
            _: u64,
        ) -> crysirror_core::Result<AudioPreparation> {
            Ok(AudioPreparation {
                phase: if self.failing_preparation {
                    AudioPreparePhase::Failed
                } else if self.ready {
                    AudioPreparePhase::Ready
                } else {
                    AudioPreparePhase::Pending
                },
                channel: 0,
                error: None,
            })
        }
        fn release_audio(&mut self, _: ClientId, t: u64) -> crysirror_core::Result<()> {
            self.released.push(t);
            Ok(())
        }
    }
    #[test]
    fn midori_audio_capture_serializes_requested_lanes_not_audible_gain() -> Result<()> {
        use crate::{midori_ambient, midori_music, midori_sound_capture::SoundCapture};
        let c = ClientId(8);
        let (mut host, mut regs) = (Host::default(), fresh(3)?);
        let (mut music, mut ambience) = (
            midori_music::Music::default(),
            midori_ambient::Ambient::default(),
        );
        let capture =
            |bgm: &midori_music::Music, env: &midori_ambient::Ambient| -> Result<String> {
                let native = SoundCapture::from_owners(bgm, env)
                    .context("unsettled Midori audio capture")?;
                assert_eq!(native.normal, (61, 0x80_0000));
                assert_eq!(native.text.0, 204);
                Ok(native.text.1)
            };
        let initially = "0,-1,2.214.214,0,0";
        let expected = |m: &str, e: &str| format!("5;{m};{initially};{e};{initially};{initially}");
        assert_eq!(capture(&music, &ambience)?, expected(initially, initially));
        assert!(regs.text(204)?.is_empty());
        regs.set_normal(61, 0x0123)?;
        regs.set_system(30, 0x10000)?;
        regs.set_system(31, 0x0080_0040)?;
        regs.set_system(32, 0x0020_00c0)?;
        let track = midori_music::Track::parse(b"8,0,144,96,1,0,0,294981,-1")?;
        assert!(!music.play(&mut host, c, &regs, track, false)?);
        assert_eq!(music.ids(), [Some(8), None]);
        let requested_music = "0,8,2.144.96,65,0";
        assert_eq!(
            capture(&music, &ambience)?,
            expected(requested_music, initially)
        );
        assert!(host.requests.is_empty());

        ambience.define(midori_ambient::Definition::parse(
            b"2,50,2,0,192,96,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1",
        )?);
        let play_env = midori_ambient::Command::parse(0x34, 0, &[2, 0, -1, 64])?;
        assert!(ambience.issue(&mut host, c, &regs, play_env, false)?);
        assert!(SoundCapture::from_owners(&music, &ambience).is_none());
        host.ready = true;
        assert!(ambience.poll(&mut host, c)?);
        let requested_ambience = "0,2,2.-1.64,512,0";
        assert_eq!(
            capture(&music, &ambience)?,
            expected(requested_music, requested_ambience)
        );

        // A worker error discards its unpublished request rather than making
        // it look as though the failed source changed the captured scene.
        ambience.define(midori_ambient::Definition::parse(
            b"3,20,2,0,96,128,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1",
        )?);
        host.failing_preparation = true;
        let unavailable_env = midori_ambient::Command::parse(0x34, 0, &[3, 0, 88, 143])?;
        assert!(ambience.issue(&mut host, c, &regs, unavailable_env, false)?);
        assert!(SoundCapture::from_owners(&music, &ambience).is_none());
        assert!(ambience.poll(&mut host, c).is_err());
        host.failing_preparation = false;
        assert_eq!(
            capture(&music, &ambience)?,
            expected(requested_music, requested_ambience)
        );

        // The admitted one-shot lane is outside the five native SoundInfo
        // descriptors and may be active without changing the capture.
        let mut se = Audio::default();
        assert!(se.issue_2011_effect(
            &mut host,
            c,
            &regs,
            effect_record(0x38, &[19, 256, 256, 0, 0])?,
            false
        )?);
        assert_eq!(
            capture(&music, &ambience)?,
            expected(requested_music, requested_ambience)
        );
        assert!(se.poll(&mut host, c, &mut regs, InputFrame::default())?);
        assert!(!music.stop(&mut host, c, 800, false)?); // muted, never published
        assert!(ambience.issue(
            &mut host,
            c,
            &regs,
            midori_ambient::Command::Stop(800),
            false
        )?);
        assert!(SoundCapture::from_owners(&music, &ambience).is_none());
        host.retiring_frames = Some((4, 100));
        assert!(!ambience.poll(&mut host, c)?);
        assert!(SoundCapture::from_owners(&music, &ambience).is_none());
        host.retiring_frames = None;
        assert!(ambience.poll(&mut host, c)?);
        assert_eq!(
            capture(&music, &ambience)?,
            expected("0,-1,2.0.0,64,0", "0,-1,2.-1.-1,512,0")
        );
        // The isolated capture never applies either register while the whole
        // blockEntry contract is still incomplete.
        assert_eq!(regs.read(Value::Local(61))?, 0x0123);
        assert!(regs.text(204)?.is_empty());
        Ok(())
    }

    #[test]
    fn midori_music_rld_starts_once_keeps_loop_frame_and_waits_only_for_retirement() -> Result<()> {
        use crate::{Action, Script, midori_music::Music};
        let mut rld = b"\0DLR".to_vec();
        for word in [3u32, 12, 3] {
            rld.extend(word.to_le_bytes());
        }
        let records: [&[u8]; 2] = [
            b"2,1500,144,96,1,0,0,294100,-1\0",
            b"7,1500,144,144,1,0,0,770697,-1\0",
        ];
        for csv in records {
            rld.extend([0x32, 0, 0, 0x31]);
            rld.extend(csv);
        }
        rld.extend([0x33, 0, 2, 0]);
        rld.extend(800i32.to_le_bytes());
        rld.extend(0i32.to_le_bytes());
        let mut script = Script::new(rld, 3)?;
        let Action::MidoriMusic(first) = script.next()? else {
            bail!("expected first music")
        };
        let Action::MidoriMusic(second) = script.next()? else {
            bail!("expected replacement music")
        };
        let Action::StopMidoriMusic(stop) = script.next()? else {
            bail!("expected stop music")
        };
        let (c, mut h, mut r, mut music) =
            (ClientId(5), Host::default(), fresh(3)?, Music::default());
        r.set_system(31, 0x0080_0040)?;
        r.set_system(32, 0)?;
        r.set_system(30, 0x10000)?;
        assert!(!music.play(&mut h, c, &r, first, false)?);
        assert_eq!(music.ids(), [Some(2), None]);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 2))?, 0);
        assert!(h.requests.is_empty());
        // Ambient mute does not silence BGM, and skip does not remove the loop.
        r.set_system(30, 0x40000)?;
        assert!(music.play(&mut h, c, &r, first, true)?);
        let AudioRequest::PlayAsset {
            channel,
            source,
            intro,
            parameters,
        } = &h.requests[0]
        else {
            bail!("expected music asset")
        };
        assert_eq!(*channel, 0);
        assert_eq!(source.path.relative(), "res/s/m/m02.ogg");
        assert!(intro.is_none() && matches!(parameters.bus, PcmBus::Music) && parameters.looping);
        assert_eq!(parameters.loop_start, 294100);
        assert_eq!(parameters.loop_end, None);
        assert_eq!(parameters.fade_ms, 0.);
        assert_eq!(
            parameters.pcm16_gain.unwrap().stereo,
            [u32::from(LEVELS[72]), u32::from(LEVELS[24])]
        );
        assert!(!music.poll(&mut h, c, &mut r)?);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 2))?, 0);
        h.ready = true;
        assert!(music.poll(&mut h, c, &mut r)?);
        assert_eq!(music.ids(), [Some(2), Some(2)]);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 2))?, 1);
        assert!(!music.play(&mut h, c, &r, first, false)?);
        h.ready = false;
        assert!(music.play(&mut h, c, &r, second, false)?);
        assert!(
            matches!(&h.requests[1], AudioRequest::PlayAsset {channel:1, parameters, ..}
            if parameters.fade_ms == 1500. && parameters.loop_start == 770697)
        );
        assert!(!music.poll(&mut h, c, &mut r)?);
        assert_eq!(music.ids(), [Some(2), Some(2)]);
        assert_eq!(h.requests.len(), 2);
        h.ready = true;
        assert!(!music.poll(&mut h, c, &mut r)?);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 7))?, 1);
        assert!(matches!(
            h.requests[2],
            AudioRequest::Stop {
                channel: 0,
                fade_ms: 1500.
            }
        ));
        h.retiring_frames = Some((0, 1));
        assert!(!music.poll(&mut h, c, &mut r)?);
        h.retiring_frames = None;
        assert!(music.poll(&mut h, c, &mut r)?);
        assert!(music.stop(&mut h, c, stop, false)?);
        assert!(matches!(
            h.requests[3],
            AudioRequest::Stop {
                channel: 1,
                fade_ms: 800.
            }
        ));
        h.retiring_frames = Some((1, 1));
        assert!(!music.poll(&mut h, c, &mut r)?);
        h.retiring_frames = None;
        assert!(music.poll(&mut h, c, &mut r)?);
        assert_eq!(music.ids(), [None, None]);
        assert!(!music.stop(&mut h, c, stop, false)?);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 2))?, 1);
        assert!(crate::midori_music::Track::parse(b"2,0,144,144,1,0,0,R1293,R1294").is_err());
        assert!(crate::midori_music::Track::parse(b"2,0,144,144,1,0,0,100,110").is_err());
        assert!(crate::midori_music::stop_duration(400, 1).is_err());
        Ok(())
    }

    #[test]
    fn midori_ambient_rld_uses_its_own_banks_and_waits_for_retirement() -> Result<()> {
        use crate::{Action, Script, midori_ambient::Ambient};
        let mut bytes = b"\0DLR".to_vec();
        for n in [3u32, 12, 5] {
            bytes.extend(n.to_le_bytes());
        }
        let definitions: [&[u8]; 2] = [
            b"1,19,2,0,128,128,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1\0",
            b"2,21,2,0,192,96,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1\0",
        ];
        for def in definitions {
            bytes.extend([0x46, 0, 0, 1]);
            bytes.extend(def);
        }
        for (id, fade) in [(1i32, 170i32), (2, 250)] {
            bytes.extend([0x34, 0, 4, 0]);
            for v in [id, fade, -1, 64] {
                bytes.extend(v.to_le_bytes());
            }
        }
        bytes.extend([0x35, 0, 1, 0]);
        bytes.extend(800i32.to_le_bytes());
        let mut script = Script::new(bytes, 3)?;
        let (c, mut h, mut ambience, mut r) =
            (ClientId(7), Host::default(), Ambient::default(), fresh(3)?);
        r.set_system(32, 0x0080_0060)?;
        r.set_system(33, 0)?;
        for _ in 0..2 {
            let Action::DefineMidoriAmbient(def) = script.next()? else {
                bail!("expected ambience definition")
            };
            ambience.define(def);
        }
        let Action::MidoriAmbient(first) = script.next()? else {
            bail!("expected ambience command")
        };
        let Action::MidoriAmbient(second) = script.next()? else {
            bail!("expected ambience command")
        };
        let Action::MidoriAmbient(stop) = script.next()? else {
            bail!("expected ambience stop")
        };
        r.set_system(30, 0x40000)?;
        assert!(!ambience.issue(&mut h, c, &r, first, false)?);
        assert!(h.requests.is_empty());
        // The one-shot mute group must not suppress ambience. Skip affects the fade,
        // not admission, gain or looping.
        r.set_system(30, 0x20000)?;
        assert!(ambience.issue(&mut h, c, &r, first, true)?);
        let AudioRequest::PlayAsset {
            channel,
            source,
            parameters,
            ..
        } = &h.requests[0]
        else {
            bail!("expected ambience request")
        };
        assert_eq!(*channel, 4);
        assert_eq!(source.path.relative(), "res/s/e/019.wav");
        assert!(matches!(parameters.bus, PcmBus::Ambient));
        assert!(parameters.looping);
        assert_eq!(parameters.fade_ms, 0.);
        assert_eq!(
            parameters.pcm16_gain.unwrap().stereo,
            [u32::from(LEVELS[64]), u32::from(LEVELS[24])]
        );
        assert!(!ambience.poll(&mut h, c)?);
        h.ready = true;
        assert!(ambience.poll(&mut h, c)?);
        assert!(!ambience.issue(&mut h, c, &r, first, false)?);
        h.ready = false;
        assert!(ambience.issue(&mut h, c, &r, second, false)?);
        assert!(
            matches!(&h.requests[1], AudioRequest::PlayAsset { channel:8, parameters, .. } if parameters.fade_ms == 250.)
        );
        assert!(!ambience.poll(&mut h, c)?);
        assert_eq!(h.requests.len(), 2);
        h.ready = true;
        assert!(!ambience.poll(&mut h, c)?);
        assert!(matches!(
            h.requests[2],
            AudioRequest::Stop {
                channel: 4,
                fade_ms: 250.
            }
        ));
        h.retiring_frames = Some((4, 40));
        assert!(!ambience.poll(&mut h, c)?);
        h.retiring_frames = None;
        assert!(ambience.poll(&mut h, c)?);
        assert!(ambience.issue(&mut h, c, &r, stop, false)?);
        assert!(matches!(
            h.requests[3],
            AudioRequest::Stop {
                channel: 8,
                fade_ms: 800.
            }
        ));
        h.retiring_frames = Some((8, 1));
        assert!(!ambience.poll(&mut h, c)?);
        h.retiring_frames = None;
        assert!(ambience.poll(&mut h, c)?);
        assert!(!ambience.issue(&mut h, c, &r, stop, false)?);
        assert_eq!(h.requests.len(), 4);
        assert!(ambience.issue(&mut h, c, &r, first, false)?);
        assert!(
            matches!(&h.requests[4], AudioRequest::PlayAsset { channel:4, parameters, .. } if parameters.fade_ms == 170.)
        );
        assert!(ambience.poll(&mut h, c)?);
        assert!(crate::midori_ambient::Command::parse(0x34, 0, &[1, 1, -2, 32]).is_err());
        assert!(
            crate::midori_ambient::Definition::parse(
                b"1,19,4,30,128,128,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1"
            )
            .is_err()
        );
        Ok(())
    }

    fn effect_record(op: u16, args: &[i32]) -> Result<Effect2011> {
        let mut data = b"\0DLR".to_vec();
        for n in [3u32, 12, 1] {
            data.extend(n.to_le_bytes());
        }
        data.extend(op.to_le_bytes());
        data.extend([args.len().try_into()?, 0x10]);
        data.extend(args.iter().flat_map(|&v| v.to_le_bytes()));
        let mut script = crate::Script::new(data, 3)?;
        let crate::Action::MidoriEffect(cmd) = script.next()? else {
            bail!("expected a Midori-only WAV effect");
        };
        Ok(cmd)
    }

    #[test]
    fn midori_async_effect_uses_own_slot_gain_and_suppression() -> Result<()> {
        let (c, mut h, mut a, mut r) = (ClientId(4), Host::default(), Audio::default(), fresh(3)?);
        r.set_system(30, 0)?;
        r.set_system(33, 0x00c0_0060)?;
        r.set_system(37, 1)?;
        let cmd = effect_record(0x38, &[19, 256, 128, 1, 0])?;
        assert!(a.issue_2011_effect(&mut h, c, &r, cmd, false)?);
        let AudioRequest::PlayAsset {
            channel,
            source,
            parameters,
            ..
        } = &h.requests[0]
        else {
            bail!("expected WAV effect");
        };
        assert_eq!(*channel, 15);
        assert_eq!(source.path.relative(), "res/s/e/019.wav");
        assert_eq!(
            parameters.pcm16_gain.unwrap().stereo,
            [u32::from(LEVELS[192]), u32::from(LEVELS[48])]
        );
        assert!(!parameters.looping);
        assert!(!a.poll(&mut h, c, &mut r, InputFrame::default())?);
        h.ready = true;
        h.playing = true;
        assert!(a.poll(&mut h, c, &mut r, InputFrame::default())?);
        assert_eq!(h.released, [1]);
        r.set_system(30, 0x20000)?;
        assert!(!a.issue_2011_effect(&mut h, c, &r, cmd, false)?);
        assert_eq!(h.requests.len(), 1);
        r.set_system(30, 0)?;
        assert!(!a.issue_2011_effect(&mut h, c, &r, cmd, true)?);
        assert_eq!(h.requests.len(), 1);
        assert!(a.issue_2011_effect(&mut h, c, &r, cmd, false)?);
        assert!(matches!(
            h.requests[1],
            AudioRequest::Stop {
                channel: 15,
                fade_ms: 0.
            }
        ));
        assert!(matches!(
            h.requests[2],
            AudioRequest::PlayAsset { channel: 15, .. }
        ));
        assert!(effect_record(0x38, &[19, 256, 128, 1, 2817]).is_err());
        assert!(effect_record(0x39, &[0, 1, 1]).is_err());
        Ok(())
    }

    #[test]
    fn midori_sync_and_leave_distinguish_wait_stop_and_owned_input() -> Result<()> {
        let (c, mut h, mut a, mut r) = (ClientId(4), Host::default(), Audio::default(), fresh(3)?);
        r.set_system(30, 0)?;
        h.ready = true;
        let held = |buttons_down| InputFrame {
            buttons_down,
            ..Default::default()
        };
        assert!(a.issue_2011_effect(
            &mut h,
            c,
            &r,
            effect_record(0x3a, &[19, 256, 256, 0, 0])?,
            false
        )?);
        h.playing = true;
        assert!(!a.poll(&mut h, c, &mut r, held(buttons::MOUSE_LEFT))?);
        for now in [100, 200, 300, 400, 500, 599] {
            h.now = now;
            assert!(!a.poll(&mut h, c, &mut r, held(buttons::MOUSE_LEFT))?);
        }
        h.now = 600;
        h.ui_owns_input = true;
        assert!(!a.poll(&mut h, c, &mut r, held(buttons::MOUSE_LEFT | buttons::CTRL))?);
        h.now = 700;
        h.ui_owns_input = false;
        assert!(a.poll(&mut h, c, &mut r, held(buttons::MOUSE_LEFT))?);
        assert!(matches!(
            h.requests.last(),
            Some(AudioRequest::Stop {
                channel: 14,
                fade_ms: 0.
            })
        ));

        assert!(a.issue_2011_effect(&mut h, c, &r, effect_record(0x39, &[0, 1, 0])?, false)?);
        assert!(a.poll(&mut h, c, &mut r, held(buttons::CTRL))?);
        assert!(matches!(
            h.requests.last(),
            Some(AudioRequest::Stop {
                channel: 15,
                fade_ms: 0.
            })
        ));
        assert!(!a.issue_2011_effect(&mut h, c, &r, effect_record(0x39, &[-1, 2, 0])?, false)?);
        assert!(matches!(
            h.requests.last(),
            Some(AudioRequest::Stop {
                channel: 16,
                fade_ms: 0.
            })
        ));
        assert!(a.poll(&mut h, c, &mut r, held(0))?);
        Ok(())
    }
    #[test]
    fn bgm_publishes_native_stereo_then_marks_experience_only_once() -> Result<()> {
        let (c, mut h, mut a, mut r) = (ClientId(7), Host::default(), Audio::default(), fresh(1)?);
        let pool = TaskPool::new("exhibit-test", 1, 1)?;
        let cmd = decode(0x32, 0, &[5, 0, 256, 0, 1, 0])?;
        a.issue(&mut h, c, &r, &pool, cmd)?;
        let AudioRequest::PlayAsset {
            channel,
            source,
            parameters,
            ..
        } = &h.requests[0]
        else {
            bail!("expected music")
        };
        assert_eq!(*channel, 0);
        assert_eq!(source.path.relative(), "res/s/m/m05.ogg");
        assert_eq!(parameters.pcm16_gain.unwrap().stereo, [81, 0]);
        assert!(parameters.looping);
        assert!(!a.poll(&mut h, c, &mut r, InputFrame::default())?);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 5))?, 0);
        h.ready = true;
        assert!(a.poll(&mut h, c, &mut r, InputFrame::default())?);
        a.issue(&mut h, c, &r, &pool, cmd)?;
        assert_eq!(h.released, [1]);
        assert_eq!(h.requests.len(), 1);
        assert_eq!(r.read(Value::Seen(Bank::Sound, 5))?, 1);
        Ok(())
    }
    #[test]
    fn blocking_effect_uses_the_selected_channel_and_stops_on_native_skip_input() -> Result<()> {
        let (c, mut h, mut a, mut r) = (ClientId(4), Host::default(), Audio::default(), fresh(1)?);
        let pool = TaskPool::new("exhibit-test", 1, 1)?;
        h.ready = true;
        h.playing = true;
        a.issue(&mut h, c, &r, &pool, decode(0x3a, 0, &[19, 256, 128, 1])?)?;
        assert!(!a.poll(&mut h, c, &mut r, InputFrame::default())?);
        assert!(a.poll(
            &mut h,
            c,
            &mut r,
            InputFrame {
                buttons_down: buttons::CTRL,
                ..Default::default()
            }
        )?);
        assert!(
            matches!(h.requests.last(),Some(AudioRequest::Stop{channel:15,fade_ms}) if *fade_ms==0.)
        );
        assert_eq!(effect_channel(-1), 14);
        Ok(())
    }
    #[test]
    fn four_lane_environment_uses_lane_defaults_and_only_the_base_fade() -> Result<()> {
        let mut e = crate::environment::Environment::default();
        let def = crate::environment::Definition::parse(
            b"7,18,2,0,144,144,16,0,0,256,256,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1",
        )?;
        e.define(def)?;
        let (c, mut h, r) = (ClientId(1), Host::default(), fresh(1)?);
        let mut scene = [0; 12];
        e.play(&mut h, c, &r, 7, 120, [256, -1], &mut scene)?;
        assert_eq!(&scene[7..10], &[7, 256, -1]);
        assert_eq!(h.requests.len(), 2);
        for (i, (ch, path, looping, fade, levels)) in [
            (4, "res/s/e/018.wav", true, 120., [256, 144]),
            (5, "res/s/e/016.wav", false, 0., [256, 256]),
        ]
        .into_iter()
        .enumerate()
        {
            let AudioRequest::PlayAsset {
                channel,
                source,
                parameters,
                ..
            } = &h.requests[i]
            else {
                bail!("expected lane source")
            };
            assert_eq!(
                (
                    *channel,
                    source.path.relative(),
                    parameters.looping,
                    parameters.fade_ms
                ),
                (ch, path, looping, fade)
            );
            assert_eq!(
                parameters.pcm16_gain.unwrap().stereo,
                stereo(&r, 35, levels)?
            );
        }
        Ok(())
    }
}
