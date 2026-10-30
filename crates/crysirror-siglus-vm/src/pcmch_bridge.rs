// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original channel declarations and wait/save policy. All resources, clocks,
//! fades, pauses and scheduled actions execute in CrySirror's channel service.
use crate::{
    audio_queue::Queue,
    bgm_bridge::Operation,
    runtime::{
        CommandContext, Value,
        forms::codes::{self, pcmch_op as op},
        globals::PcmChPersistentState,
        wait::AudioWait,
    },
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::{
    AudioParameters, AudioPlaybackInfo, AudioRequest, AudioResumePolicy, AudioResumeSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const MAX_CHANNELS: usize = 256;
pub(crate) fn channel(ch: usize) -> u32 {
    4 + ch as u32
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    #[default]
    Stopped,
    Playing,
    Ready,
    Paused,
    Stopping,
}
#[derive(Clone, Serialize, Deserialize)]
struct Slot {
    key: Option<String>,
    looping: bool,
    mode: Mode,
    volume: u8,
    delay: i64,
    #[serde(skip)]
    pending: Queue,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            key: None,
            looping: false,
            mode: Mode::Stopped,
            volume: 255,
            delay: 0,
            pending: Queue::default(),
        }
    }
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Channels(BTreeMap<usize, Slot>);
fn ms(n: i64) -> Result<f32> {
    super::sfx_bridge::duration(n)
}
impl Slot {
    fn clear(&mut self) {
        self.key = None;
        self.looping = false;
        self.mode = Mode::Stopped;
        self.delay = 0;
    }
    fn start(&mut self, ch: usize, key: &str, looping: bool, fade: i64, ready: bool) -> Result<()> {
        super::sfx_bridge::key(key)?;
        let fade_ms = ms(fade)?;
        self.pending.push(Operation::Channel {
            channel: channel(ch),
            key: key.into(),
            parameters: AudioParameters {
                // PCMCH script volume is independent of GLOBAL.PCM volume.
                mix_group: None,
                gain: if fade_ms > 0. && !ready {
                    0.
                } else {
                    f32::from(self.volume) / 255.
                },
                looping,
                paused: ready,
                fade_ms: 0.,
                ..Default::default()
            },
        })?;
        if fade_ms > 0. && !ready {
            // Original PLAY's fade is the gain parameter, independently of
            // the pause/stop transport envelope. A later volume call replaces it.
            self.pending
                .push(Operation::Control(AudioRequest::SetGain {
                    channel: channel(ch),
                    gain: f32::from(self.volume) / 255.,
                    fade_ms,
                }))?;
        }
        self.key = Some(key.into());
        self.looping = looping;
        self.mode = if ready { Mode::Ready } else { Mode::Playing };
        self.delay = 0;
        Ok(())
    }
    fn stop(&mut self, ch: usize, fade: i64) -> Result<()> {
        let fade_ms = ms(fade)?;
        if self.key.is_some() {
            self.pending.push(Operation::Control(AudioRequest::Stop {
                channel: channel(ch),
                fade_ms,
            }))?;
            self.mode = Mode::Stopping;
            self.delay = 0;
        }
        if fade_ms == 0. {
            self.clear();
        }
        Ok(())
    }
}
impl Channels {
    fn slot(&mut self, ch: usize) -> Result<&mut Slot> {
        ensure!(ch < MAX_CHANNELS, "Siglus PCMCH index exceeds 256");
        Ok(self.0.entry(ch).or_default())
    }
    pub(crate) fn active(&self) -> Vec<usize> {
        self.0
            .iter()
            .filter_map(|(ch, s)| s.key.as_ref().map(|_| *ch))
            .collect()
    }
    pub(crate) fn playing(&self, ch: usize) -> bool {
        self.0.get(&ch).is_some_and(|s| s.mode == Mode::Playing)
    }
    pub(crate) fn fading(&self, ch: usize) -> bool {
        self.0.get(&ch).is_some_and(|s| s.mode == Mode::Stopping)
    }
    pub(crate) fn any(&self) -> bool {
        self.0.values().any(|s| s.mode == Mode::Playing)
    }
    pub(crate) fn volume(&self, ch: usize) -> i32 {
        self.0.get(&ch).map_or(255, |s| i32::from(s.volume))
    }
    pub(crate) fn delay(&self, ch: usize) -> i64 {
        self.0.get(&ch).map_or(0, |s| s.delay)
    }
    pub(crate) fn pending(&self) -> bool {
        self.0.values().any(|s| !s.pending.is_empty())
    }
    pub(crate) fn take(&mut self) -> Queue {
        let mut queue = Queue::default();
        for s in self.0.values_mut() {
            queue.append(std::mem::take(&mut s.pending));
        }
        queue
    }
    pub(crate) fn stop_all(&mut self, fade: i64) -> Result<()> {
        ms(fade)?;
        for (ch, s) in &mut self.0 {
            s.stop(*ch, fade)?;
        }
        Ok(())
    }
    fn set_volume(&mut self, ch: usize, volume: u8, fade: i64) -> Result<()> {
        let fade_ms = ms(fade)?;
        let s = self.slot(ch)?;
        if s.key.is_some() {
            s.pending.push(Operation::Control(AudioRequest::SetGain {
                channel: channel(ch),
                gain: f32::from(volume) / 255.,
                fade_ms,
            }))?;
        }
        s.volume = volume;
        Ok(())
    }
    pub(crate) fn sync(&mut self, ch: usize, info: Option<AudioPlaybackInfo>) -> Result<()> {
        let Some(s) = self.0.get_mut(&ch) else {
            return Ok(());
        };
        let Some(i) = info.filter(|i| i.source_playing || i.stopping) else {
            s.clear();
            return Ok(());
        };
        ensure!(
            i.source_rate > 0 && i.output_rate > 0,
            "invalid shared PCMCH clock"
        );
        s.delay = (u128::from(i.resume_delay_frames) * 1000 / u128::from(i.output_rate)).min(600000)
            as i64;
        if s.mode == Mode::Stopping {
            if !i.stopping {
                s.clear();
            }
            return Ok(());
        }
        let len = i
            .source_frames
            .context("Siglus PCMCH needs measured source length")?;
        let elapsed = u128::from(i.transport_frames) * 1000 / u128::from(i.output_rate);
        let duration = u128::from(len) * 1000 / u128::from(i.source_rate);
        if !s.looping && !i.paused && s.mode != Mode::Ready && elapsed >= duration {
            s.clear();
        } else if s.mode == Mode::Paused && !i.paused && !i.pausing && i.resume_delay_frames == 0 {
            s.mode = Mode::Playing;
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.0.len() <= MAX_CHANNELS,
            "saved PCMCH list exceeds limit"
        );
        for (ch, s) in &self.0 {
            ensure!(
                *ch < MAX_CHANNELS && (0..=600000).contains(&s.delay),
                "invalid saved PCMCH state"
            );
            ensure!(
                (s.mode == Mode::Stopped) == s.key.is_none(),
                "saved PCMCH declaration/mode mismatch"
            );
            if let Some(key) = &s.key {
                super::sfx_bridge::key(key)?;
            }
        }
        Ok(())
    }
    pub(crate) fn rebind(&mut self) -> Result<()> {
        for (ch, s) in &mut self.0 {
            s.pending = Queue::default();
            let Some(key) = s.key.clone() else { continue };
            let mode = s.mode;
            let delay = s.delay;
            // Runtime snapshots reconstruct cues, never encoded PCM/DSP history.
            // A pending stop has no retained source after reconstruction.
            if mode == Mode::Stopping {
                s.clear();
                continue;
            }
            s.start(
                *ch,
                &key,
                s.looping,
                0,
                matches!(mode, Mode::Ready | Mode::Paused),
            )?;
            if mode == Mode::Paused {
                s.mode = Mode::Paused;
                if delay > 0 {
                    s.pending
                        .push(Operation::Control(AudioRequest::ResumeWithOptions {
                            channel: channel(*ch),
                            delay_ms: ms(delay)?,
                            fade_ms: 0.,
                            settings: AudioResumeSettings {
                                policy: AudioResumePolicy::Preserve,
                                reset_gain: true,
                            },
                        }))?;
                    s.delay = delay;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn inspect(&self) -> serde_json::Value {
        serde_json::Value::Array(self.0.iter().map(|(ch,s)|serde_json::json!({"channel":ch,"resource":s.key,"volume":s.volume,"playing":s.mode==Mode::Playing,"ready":s.mode==Mode::Ready,"paused":s.mode==Mode::Paused,"fading":s.mode==Mode::Stopping,"looping":s.looping,"delay_ms":s.delay})).collect())
    }
}
fn positional(args: &[Value], i: usize) -> Option<&Value> {
    args.iter()
        .filter(|v| !matches!(v, Value::NamedArg { .. }))
        .nth(i)
}
fn integer(args: &[Value], i: usize, default: i64) -> i64 {
    positional(args, i)
        .and_then(Value::as_i64)
        .unwrap_or(default)
}
fn named(args: &[Value], id: i32) -> Option<&Value> {
    args.iter().find_map(|v| match v {
        Value::NamedArg { id: n, value } if *n == id => Some(value.as_ref()),
        _ => None,
    })
}
fn ni(args: &[Value], id: i32, default: i64) -> i64 {
    named(args, id).and_then(Value::as_i64).unwrap_or(default)
}
fn text(args: &[Value], id: i32) -> Option<&str> {
    named(args, id).and_then(Value::as_str)
}
fn count(ctx: &CommandContext) -> usize {
    ctx.tables
        .gameexe
        .as_ref()
        .and_then(|c| c.get_usize("PCMCH.CNT"))
        .unwrap_or(16)
        .min(MAX_CHANNELS)
}
fn check_channel(ctx: &CommandContext, ch: usize) -> Result<()> {
    ensure!(
        ch < count(ctx),
        "Siglus PCMCH index {ch} outside declared channel list"
    );
    Ok(())
}
fn stored(ctx: &mut CommandContext, ch: usize) -> &mut PcmChPersistentState {
    if ctx.globals.pcmch_persistent.len() <= ch {
        ctx.globals
            .pcmch_persistent
            .resize_with(ch + 1, PcmChPersistentState::default);
    }
    &mut ctx.globals.pcmch_persistent[ch]
}
fn routing(ctx: &CommandContext, s: &PcmChPersistentState) -> Result<()> {
    ensure!(
        matches!(s.volume_type, 2 | 3)
            && s.chara_no < 0
            && !s.bgm_fade_target_flag
            && !s.bgm_fade2_target_flag
            && !s.bgm_fade_source_flag,
        "Siglus PCMCH character/category/BGM duck routing unsupported"
    );
    let cfg = &ctx.globals.syscom.original_config;
    ensure!(
        cfg.play_all_sound_check
            && cfg.all_sound_user_volume == 255
            && cfg.play_sound_check[s.volume_type as usize]
            && cfg.sound_user_volume[s.volume_type as usize] == 255,
        "Siglus PCMCH nondefault native system volume routing unsupported"
    );
    Ok(())
}
fn source(ctx: &CommandContext, s: &PcmChPersistentState) -> Result<(String, bool, i64)> {
    if !s.pcm_name.is_empty() {
        return Ok((s.pcm_name.clone(), s.loop_flag, s.fade_in_time));
    }
    if !s.bgm_name.is_empty() {
        if let Some(cfg) = &ctx.tables.gameexe {
            ensure!(
                cfg.indexed_count("BGM") <= 4096,
                "Siglus PCMCH BGM table exceeds 4096 entries"
            );
        }
        let key = ctx
            .tables
            .gameexe
            .as_ref()
            .and_then(|c| {
                (0..c.indexed_count("BGM")).find_map(|i| {
                    let name = c.get_indexed_item_unquoted("BGM", i, 0)?;
                    if name.trim().eq_ignore_ascii_case(s.bgm_name.trim()) {
                        c.get_indexed_item_unquoted("BGM", i, 1)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| s.bgm_name.clone());
        return Ok((key, s.loop_flag, s.fade_in_time));
    }
    if s.koe_no >= 0 {
        return Ok((format!("koe/{}", s.koe_no), s.loop_flag, 0));
    }
    if s.se_no >= 0 {
        let cfg = ctx
            .tables
            .gameexe
            .as_ref()
            .context("Siglus PCMCH SE source needs Gameexe table")?;
        let n = cfg.get_usize("SE.CNT").unwrap_or(16).clamp(8, 256);
        ensure!(
            (s.se_no as u64) < n as u64,
            "Siglus PCMCH SE number outside native table"
        );
        let key = cfg
            .get_indexed_unquoted("SE", s.se_no as usize)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .context("Siglus PCMCH SE table entry missing")?;
        return Ok((key.into(), false, 0));
    }
    bail!("Siglus PCMCH PLAY/READY needs a supported source identity")
}
fn start(ctx: &mut CommandContext, ch: usize, s: &PcmChPersistentState) -> Result<()> {
    routing(ctx, s)?;
    let (key, looping, fade) = source(ctx, s)?;
    ctx.pcm.ensure_group()?;
    ctx.pcm
        .channels
        .slot(ch)?
        .start(ch, &key, looping, fade, s.ready_flag)
}
pub(crate) fn check_event_channel(ctx: &CommandContext, ch: usize) -> Result<()> {
    check_channel(ctx, ch)
}
pub(crate) fn stop_event_channel(ctx: &mut CommandContext, ch: usize) -> Result<()> {
    check_channel(ctx, ch)?;
    ctx.pcm.channels.slot(ch)?.stop(ch, 0)
}
pub(crate) fn play_event(
    ctx: &mut CommandContext,
    ch: usize,
    key: &str,
    event: &crate::runtime::globals::PcmEventState,
) -> Result<()> {
    check_channel(ctx, ch)?;
    let state = PcmChPersistentState {
        pcm_name: key.into(),
        volume_type: i64::from(event.volume_type),
        chara_no: i64::from(event.chara_no),
        bgm_fade_target_flag: event.bgm_fade_target_flag,
        bgm_fade2_target_flag: event.bgm_fade2_target_flag,
        bgm_fade_source_flag: event.bgm_fade_source_flag,
        volume: i64::from(ctx.pcm.slot_volume_raw(ch)),
        ..Default::default()
    };
    start(ctx, ch, &state)?;
    *stored(ctx, ch) = state;
    Ok(())
}
pub(crate) fn restore(
    ctx: &mut CommandContext,
    ch: usize,
    state: PcmChPersistentState,
) -> Result<()> {
    ensure!(ch < MAX_CHANNELS, "native PCMCH save list exceeds 256");
    ctx.pcm
        .channels
        .set_volume(ch, state.volume.clamp(0, 255) as u8, 0)?;
    if state.loop_flag || state.ready_flag || state.delay_time > 0 {
        let mut cue = state.clone();
        cue.fade_in_time = 0;
        if !cue.pcm_name.is_empty() {
            let mut pcm = cue.clone();
            pcm.bgm_name.clear();
            pcm.koe_no = -1;
            pcm.se_no = -1;
            start(ctx, ch, &pcm)?;
        }
        // Original restore is deliberately different from PLAY source priority:
        // PCM first, followed independently by BGM/KOE/SE when also declared.
        cue.pcm_name.clear();
        if !cue.bgm_name.is_empty() || cue.koe_no >= 0 || cue.se_no >= 0 {
            start(ctx, ch, &cue)?;
        }
    }
    *stored(ctx, ch) = state;
    Ok(())
}
pub(crate) fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let call = ctx
        .vm_call
        .as_ref()
        .context("Siglus PCMCH needs native command metadata")?;
    let chain = &call.element;
    ensure!(
        chain.len() == 4
            && chain[1]
                == if ctx.ids.elm_array >= 0 {
                    ctx.ids.elm_array
                } else {
                    codes::ELM_ARRAY
                },
        "unsupported Siglus PCMCH element chain"
    );
    let ch = usize::try_from(chain[2]).context("negative Siglus PCMCH index")?;
    check_channel(ctx, ch)?;
    let op = chain[3];
    let returns = call.ret_form != 0;
    for arg in args {
        if let Value::NamedArg { id, value } = arg {
            ensure!(
                (0..=11).contains(id),
                "unsupported Siglus PCMCH named argument {id}"
            );
            ensure!(
                if matches!(id, 7 | 10) {
                    value.as_str().is_some()
                } else {
                    value.as_i64().is_some()
                },
                "invalid Siglus PCMCH named argument {id}"
            );
        }
    }
    match op {
        op::PLAY | op::PLAY_LOOP | op::PLAY_WAIT | op::READY | op::READY_LOOP => {
            let ready = matches!(op, op::READY | op::READY_LOOP);
            let looping = ni(
                args,
                0,
                i64::from(matches!(op, op::PLAY_LOOP | op::READY_LOOP)),
            ) != 0;
            let fade = if op == op::READY {
                0
            } else {
                ni(args, 2, integer(args, 1, 0))
            };
            let state = PcmChPersistentState {
                pcm_name: text(args, 7)
                    .or_else(|| positional(args, 0).and_then(Value::as_str))
                    .unwrap_or("")
                    .into(),
                bgm_name: text(args, 10).unwrap_or("").into(),
                koe_no: ni(args, 8, -1),
                se_no: ni(args, 9, -1),
                volume_type: ni(args, 3, 2),
                chara_no: ni(args, 6, -1),
                volume: i64::from(ctx.pcm.slot_volume_raw(ch)),
                delay_time: 0,
                fade_in_time: fade,
                loop_flag: looping,
                ready_flag: ready,
                bgm_fade_target_flag: ni(args, 4, 0) != 0,
                bgm_fade2_target_flag: ni(args, 5, 0) != 0,
                bgm_fade_source_flag: ni(args, 11, 0) != 0,
            };
            start(ctx, ch, &state)?;
            *stored(ctx, ch) = state;
            if !ready && ni(args, 1, i64::from(op == op::PLAY_WAIT)) != 0 {
                ctx.wait.wait_audio(AudioWait::PcmSlot(ch as u8), false);
            }
        }
        op::STOP => {
            ctx.pcm.channels.slot(ch)?.stop(ch, integer(args, 0, 0))?;
            stored(ctx, ch).loop_flag = false;
        }
        op::PAUSE => {
            let fade_ms = ms(integer(args, 0, 0))?;
            let s = ctx.pcm.channels.slot(ch)?;
            if matches!(s.mode, Mode::Playing | Mode::Stopping) {
                s.pending.push(Operation::Control(AudioRequest::Pause {
                    channel: channel(ch),
                    paused: true,
                    fade_ms,
                }))?;
                if s.mode != Mode::Stopping {
                    s.mode = Mode::Paused;
                }
            }
        }
        op::RESUME | op::RESUME_WAIT => {
            let fade = integer(args, 0, 0).max(0);
            let delay = if op == op::RESUME {
                ni(args, 0, 0).max(0)
            } else {
                0
            };
            let fade_ms = ms(fade)?;
            let delay_ms = ms(delay)?;
            let s = ctx.pcm.channels.slot(ch)?;
            if matches!(s.mode, Mode::Ready | Mode::Paused) {
                s.pending
                    .push(Operation::Control(AudioRequest::ResumeWithOptions {
                        channel: channel(ch),
                        delay_ms,
                        fade_ms,
                        settings: AudioResumeSettings {
                            policy: AudioResumePolicy::Preserve,
                            reset_gain: true,
                        },
                    }))?;
                s.mode = if delay == 0 {
                    Mode::Playing
                } else {
                    Mode::Paused
                };
                s.delay = delay;
            }
            let p = stored(ctx, ch);
            p.ready_flag = false;
            p.delay_time = delay;
            p.fade_in_time = fade;
            if op == op::RESUME_WAIT {
                ctx.wait.wait_audio(AudioWait::PcmSlot(ch as u8), false);
            }
        }
        op::CHECK => {
            let playing = ctx.pcm.is_playing_slot(ch);
            ctx.push(Value::Int(i64::from(playing)));
        }
        op::WAIT => {
            ctx.wait.wait_audio(AudioWait::PcmSlot(ch as u8), false);
            if returns {
                ctx.push(Value::Int(0));
            }
        }
        op::WAIT_KEY | op::WAIT_FADE | op::WAIT_FADE_KEY => ctx.wait.wait_audio_with_return(
            if op == op::WAIT_KEY {
                AudioWait::PcmSlot(ch as u8)
            } else {
                AudioWait::PcmSlotFade(ch as u8)
            },
            op != op::WAIT_FADE,
            returns,
        ),
        op::SET_VOLUME | op::SET_VOLUME_MAX | op::SET_VOLUME_MIN => {
            let (v, fade) = match op {
                op::SET_VOLUME => (
                    positional(args, 0)
                        .and_then(Value::as_i64)
                        .context("Siglus PCMCH volume required")?
                        .clamp(0, 255) as u8,
                    integer(args, 1, 0),
                ),
                op::SET_VOLUME_MAX => (255, integer(args, 0, 0)),
                _ => (0, integer(args, 0, 0)),
            };
            ctx.pcm.channels.set_volume(ch, v, fade)?;
            stored(ctx, ch).volume = i64::from(v);
        }
        op::GET_VOLUME => {
            let v = ctx.pcm.slot_volume_raw(ch);
            ctx.push(Value::Int(i64::from(v)));
        }
        _ => bail!("unsupported Siglus PCMCH opcode {op}"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_restore_plays_pcm_then_independent_bgm_ignoring_delay_and_fade() {
        let mut ctx = CommandContext::new(".".into());
        restore(
            &mut ctx,
            0,
            PcmChPersistentState {
                pcm_name: "pcm".into(),
                bgm_name: "bgm".into(),
                delay_time: 100,
                fade_in_time: 50,
                volume: 128,
                ..Default::default()
            },
        )
        .unwrap();
        let mut operations = ctx.pcm.take().entries();
        operations.sort_by_key(|v| v.0);
        let cues: Vec<_> = operations
            .iter()
            .filter_map(|(_, op)| match op {
                Operation::Channel {
                    key, parameters, ..
                } => Some((key.as_str(), parameters)),
                _ => None,
            })
            .collect();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].0, "pcm");
        assert_eq!(cues[1].0, "bgm");
        for (_, p) in cues {
            assert_eq!(p.fade_ms, 0.);
            assert!(!p.paused && !p.looping);
            assert_eq!(p.gain, 128. / 255.);
        }
        assert!(!operations.iter().any(|(_, op)| matches!(
            op,
            Operation::Control(
                AudioRequest::Resume { .. } | AudioRequest::ResumeWithOptions { .. }
            )
        )));
        assert_eq!(ctx.pcm.slot_resume_delay_ms(0), 0);
    }
    #[test]
    fn pcmch_se_category_keeps_channel_volume_independent_of_global_pcm_group() {
        let mut ctx = CommandContext::new(".".into());
        ctx.pcm
            .set_volume_raw(&mut crate::audio::AudioHub, 0)
            .unwrap();
        ctx.pcm.channels.set_volume(0, 128, 0).unwrap();
        let state = PcmChPersistentState {
            pcm_name: "cue".into(),
            volume_type: 3,
            ready_flag: true,
            ..Default::default()
        };
        start(&mut ctx, 0, &state).unwrap();
        let queue = ctx.pcm.take().entries();
        let params = queue
            .iter()
            .find_map(|(_, op)| match op {
                Operation::Channel { parameters, .. } => Some(parameters),
                _ => None,
            })
            .unwrap();
        assert_eq!(params.gain, 128. / 255.);
        assert!(params.paused);
        assert_eq!(
            params.mix_group, None,
            "GLOBAL.PCM script gain must not alter a PCMCH source"
        );
        ctx.globals.syscom.original_config.sound_user_volume[3] = 128;
        assert!(
            start(&mut ctx, 0, &state)
                .unwrap_err()
                .to_string()
                .contains("nondefault"),
            "an unmapped system-volume factor is never discarded"
        );
    }
    #[test]
    fn sparse_bgm_indices_are_bounded_before_native_alias_lookup() {
        let mut ctx = CommandContext::new(".".into());
        ctx.tables.gameexe = Some(std::sync::Arc::new(
            siglus_assets::gameexe::GameexeConfig::from_text("#BGM.99999=\"song\",\"cue\",0,1,0\n"),
        ));
        let state = PcmChPersistentState {
            bgm_name: "song".into(),
            ..Default::default()
        };
        assert!(
            source(&ctx, &state)
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
    }
    #[test]
    fn original_nonpersistent_restore_only_restores_volume_and_rejects_live_unknown_routing() {
        let mut ctx = CommandContext::new(".".into());
        restore(
            &mut ctx,
            0,
            PcmChPersistentState {
                pcm_name: "unused".into(),
                volume: 64,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ctx.pcm.slot_volume_raw(0), 64);
        assert!(!ctx.pcm.pending());
        assert_eq!(ctx.pcm.slot_volume_raw(1), 255);
        assert!(
            restore(
                &mut ctx,
                1,
                PcmChPersistentState {
                    pcm_name: "cue".into(),
                    loop_flag: true,
                    bgm_fade_source_flag: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(ctx.pcm.channels.active().is_empty());
    }
}
