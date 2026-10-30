// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Single original KOE slot: native identity/config/waits and shared resource
//! handles only. Compression, decoder storage, voices and fades belong to engine.
use crate::{
    audio::AudioHub,
    audio_queue::Queue,
    bgm_bridge::Operation,
    runtime::{
        CommandContext, Value, constants,
        forms::{
            codes::{elm_value as op, syscom_op as cfg},
            prop_access,
        },
        wait::AudioWait,
    },
};
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{AudioPlaybackInfo, AudioRequest, AudioResourceHandle};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub(crate) const CHANNEL: u32 = 3;
pub(crate) const GROUP: &str = "siglus.koe";
#[derive(Clone, Serialize, Deserialize)]
pub struct KoeEngine {
    volume: u8,
    current: Option<i64>,
    rate: u16,
    playing: bool,
    #[serde(skip)]
    initialized: bool,
    #[serde(skip)]
    pub(crate) pending: Queue,
    #[serde(skip)]
    pub(crate) bindings: BTreeMap<(i64, u16), AudioResourceHandle>,
}
impl Default for KoeEngine {
    fn default() -> Self {
        Self {
            volume: 255,
            current: None,
            rate: 100,
            playing: false,
            initialized: false,
            pending: Default::default(),
            bindings: Default::default(),
        }
    }
}
impl KoeEngine {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            (100..=400).contains(&self.rate)
                && self.current.is_none_or(|n| n >= 0)
                && (!self.playing || self.current.is_some()),
            "invalid saved Siglus voice declaration"
        );
        Ok(())
    }
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub(crate) fn set_character_gain(&mut self, value: u8) -> Result<()> {
        self.pending
            .push(Operation::Control(AudioRequest::SetGain {
                channel: CHANNEL,
                gain: f32::from(value) / 255.,
                fade_ms: 0.,
            }))?;
        Ok(())
    }
    pub(crate) fn current_no(&self) -> i64 {
        self.current.unwrap_or(-1)
    }
    pub(crate) fn rate(&self) -> u16 {
        self.rate
    }
    pub fn set_volume_raw(&mut self, audio: &mut AudioHub, v: u8) -> Result<()> {
        self.set_volume_raw_fade(audio, v, 0)
    }
    pub fn set_volume_raw_fade(&mut self, _: &mut AudioHub, v: u8, ms: i64) -> Result<()> {
        ensure!(ms.max(0) <= 600000, "voice fade exceeds engine limit");
        self.pending
            .push(Operation::Control(AudioRequest::SetMixGroupGain {
                name: GROUP.into(),
                gain: f32::from(v) / 255.,
                fade_ms: ms.max(0) as f32,
            }))?;
        self.volume = v;
        self.initialized = true;
        Ok(())
    }
    pub fn stop(&mut self, fade: Option<i64>) -> Result<()> {
        let ms = fade.unwrap_or(0).max(0);
        ensure!(ms <= 600000, "voice stop fade exceeds engine limit");
        if self.current.is_some() {
            self.pending.push(Operation::Control(AudioRequest::Stop {
                channel: CHANNEL,
                fade_ms: ms as f32,
            }))?;
        }
        self.playing = false;
        Ok(())
    }
    pub fn is_playing_any(&mut self) -> bool {
        self.playing
    }
    pub(crate) fn play(&mut self, no: i64, rate: u16) -> Result<()> {
        // Original reinitializes/stops the old slot before resource work, even
        // when the following resource fails or a negative number is requested.
        self.pending.push(Operation::Control(AudioRequest::Stop {
            channel: CHANNEL,
            fade_ms: 0.,
        }))?;
        self.current = None;
        self.playing = false;
        self.rate = rate.clamp(100, 400);
        if no >= 0 {
            if !self.initialized {
                self.set_volume_raw(&mut AudioHub, self.volume)?;
            }
            self.pending.push(Operation::Speech {
                number: no,
                percent: self.rate,
            })?;
            self.current = Some(no);
            self.playing = true;
        }
        Ok(())
    }
    pub(crate) fn sync(&mut self, info: Option<AudioPlaybackInfo>) -> Result<()> {
        let Some(i) = info.filter(|i| i.source_playing) else {
            self.playing = false;
            return Ok(());
        };
        let frames = i
            .source_frames
            .context("Siglus voice needs a measured source boundary")?;
        ensure!(
            i.source_rate > 0 && i.output_rate > 0,
            "invalid shared voice clock"
        );
        self.playing = self.current.is_some()
            && !i.paused
            && i.fade_out_frames == 0
            && u128::from(i.age_frames) * 1000 / u128::from(i.output_rate)
                < u128::from(frames) * 1000 / u128::from(i.source_rate);
        Ok(())
    }
    pub(crate) fn rebind(&mut self) -> Result<()> {
        self.pending = Default::default();
        self.bindings.clear();
        self.initialized = false;
        if self.playing {
            self.play(
                self.current.context("saved voice missing identity")?,
                self.rate,
            )?;
        } else if self.volume != 255 {
            self.set_volume_raw(&mut AudioHub, self.volume)?;
        }
        Ok(())
    }
}
fn rate(ctx: &CommandContext, explicit: Option<bool>) -> u16 {
    let o = &ctx.globals.syscom.original_config;
    let auto = ctx.globals.script.auto_mode_flag || ctx.globals.syscom.auto_mode.onoff;
    let enabled = explicit.unwrap_or_else(|| {
        let (key, fallback) = if auto {
            (cfg::GET_JITAN_AUTO_MODE_ONOFF, o.jitan_auto_mode_onoff)
        } else {
            (cfg::GET_JITAN_NORMAL_ONOFF, o.jitan_normal_onoff)
        };
        ctx.globals
            .syscom
            .config_int
            .get(&key)
            .copied()
            .unwrap_or(i64::from(fallback))
            != 0
    });
    if enabled {
        ctx.globals
            .syscom
            .config_int
            .get(&cfg::GET_JITAN_SPEED)
            .copied()
            .unwrap_or(o.jitan_speed)
            .clamp(100, 400) as u16
    } else {
        100
    }
}
fn named(args: &[Value], id: i32) -> Option<i64> {
    args.iter().find_map(|v| {
        if let Value::NamedArg { id: n, value } = v {
            if *n == id {
                return value.as_i64();
            }
        }
        None
    })
}
fn positional(args: &[Value], n: usize) -> Option<i64> {
    args.iter()
        .filter(|v| !matches!(v, Value::NamedArg { .. }))
        .filter_map(Value::as_i64)
        .nth(n)
}
fn remember(ctx: &mut CommandContext, no: i64, chara: i64, ex: bool) {
    ctx.globals.sound_routing.koe_chara_no = chara;
    ctx.globals.sound_routing.koe_ex_flag = ex;
    if !ex {
        ctx.globals.script.cur_koe_no = no;
        ctx.globals.script.cur_chr_no = chara;
        ctx.globals.syscom.replay_koe = Some((no, chara));
        ctx.globals.sound_routing.bgmfade2_need_flag = true;
    }
    let props = ctx
        .globals
        .int_props
        .entry(constants::fm::GLOBAL as u32)
        .or_default();
    props.insert(op::GLOBAL_KOE_CHECK_GET_KOE_NO, no);
    props.insert(op::GLOBAL_KOE_CHECK_GET_CHARA_NO, chara);
    props.insert(op::GLOBAL_KOE_CHECK_IS_EX_KOE, i64::from(ex));
}
pub(crate) fn dispatch(ctx: &mut CommandContext, id: u32, args: &[Value]) -> Result<bool> {
    let id = id as i32;
    let returns = prop_access::current_vm_meta(ctx).1.unwrap_or(0) != 0;
    match id {
        op::GLOBAL_KOE
        | op::GLOBAL_EXKOE
        | op::GLOBAL_KOE_PLAY_WAIT
        | op::GLOBAL_KOE_PLAY_WAIT_KEY
        | op::GLOBAL_EXKOE_PLAY_WAIT
        | op::GLOBAL_EXKOE_PLAY_WAIT_KEY => {
            let ex = matches!(
                id,
                op::GLOBAL_EXKOE | op::GLOBAL_EXKOE_PLAY_WAIT | op::GLOBAL_EXKOE_PLAY_WAIT_KEY
            );
            let no = if ex {
                named(args, 0).or_else(|| positional(args, 0))
            } else {
                positional(args, 0)
            }
            .unwrap_or(0);
            let chara = if ex {
                named(args, 1).or_else(|| positional(args, 1))
            } else {
                positional(args, 1)
            }
            .unwrap_or(-1);
            if !ex {
                ctx.request_read_flag_no();
            }
            remember(ctx, no, chara, ex);
            let percent = rate(ctx, ex.then(|| named(args, 4).unwrap_or(0) != 0));
            ctx.koe.play(no, percent)?;
            crate::character_voice::refresh(ctx)?;
            let direct = matches!(id, op::GLOBAL_KOE | op::GLOBAL_EXKOE);
            let wait = !direct || (ex && named(args, 2).unwrap_or(0) != 0);
            if wait {
                let key = if direct {
                    named(args, 3).unwrap_or(0) != 0
                } else {
                    matches!(
                        id,
                        op::GLOBAL_KOE_PLAY_WAIT_KEY | op::GLOBAL_EXKOE_PLAY_WAIT_KEY
                    )
                };
                ctx.wait
                    .wait_audio_with_return(AudioWait::KoeAny, key, returns);
            } else if returns {
                ctx.push(Value::Int(0));
            }
        }
        op::GLOBAL_KOE_STOP => ctx.koe.stop(args.first().and_then(Value::as_i64))?,
        op::GLOBAL_KOE_WAIT | op::GLOBAL_KOE_WAIT_KEY => ctx.wait.wait_audio_with_return(
            AudioWait::KoeAny,
            id == op::GLOBAL_KOE_WAIT_KEY,
            returns,
        ),
        op::GLOBAL_KOE_CHECK => {
            let playing = ctx.koe.is_playing_any();
            ctx.push(Value::Int(i64::from(playing)));
        }
        op::GLOBAL_KOE_CHECK_GET_KOE_NO
        | op::GLOBAL_KOE_CHECK_GET_CHARA_NO
        | op::GLOBAL_KOE_CHECK_IS_EX_KOE => ctx.push(Value::Int(
            ctx.globals
                .int_props
                .get(&(constants::fm::GLOBAL as u32))
                .and_then(|p| p.get(&id))
                .copied()
                .unwrap_or(0),
        )),
        op::GLOBAL_KOE_SET_VOLUME => ctx.koe.set_volume_raw_fade(
            &mut ctx.audio,
            args.first()
                .and_then(Value::as_i64)
                .unwrap_or(255)
                .clamp(0, 255) as u8,
            args.get(1).and_then(Value::as_i64).unwrap_or(0),
        )?,
        op::GLOBAL_KOE_SET_VOLUME_MAX | op::GLOBAL_KOE_SET_VOLUME_MIN => {
            ctx.koe.set_volume_raw_fade(
                &mut ctx.audio,
                if id == op::GLOBAL_KOE_SET_VOLUME_MAX {
                    255
                } else {
                    0
                },
                args.first().and_then(Value::as_i64).unwrap_or(0),
            )?
        }
        op::GLOBAL_KOE_GET_VOLUME => ctx.push(Value::Int(i64::from(ctx.koe.volume_raw()))),
        _ => return Ok(false),
    }
    Ok(true)
}
/// Only the original JITAN config subset; other SYSCOM commands remain unsupported.
pub(crate) fn config(ctx: &mut CommandContext, id: u32, args: &[Value]) -> Result<bool> {
    if id != ctx.ids.form_global_syscom {
        return Ok(false);
    }
    let Some(op) = prop_access::current_op_from_ctx_or_args(ctx, args) else {
        return Ok(false);
    };
    if crate::character_voice::dispatch(ctx, op, args)? {
        return Ok(true);
    }
    let (key, action) = match op {
        cfg::SET_JITAN_NORMAL_ONOFF => (cfg::GET_JITAN_NORMAL_ONOFF, 0),
        cfg::SET_JITAN_NORMAL_ONOFF_DEFAULT => (cfg::GET_JITAN_NORMAL_ONOFF, 1),
        cfg::GET_JITAN_NORMAL_ONOFF => (cfg::GET_JITAN_NORMAL_ONOFF, 2),
        cfg::SET_JITAN_AUTO_MODE_ONOFF => (cfg::GET_JITAN_AUTO_MODE_ONOFF, 0),
        cfg::SET_JITAN_AUTO_MODE_ONOFF_DEFAULT => (cfg::GET_JITAN_AUTO_MODE_ONOFF, 1),
        cfg::GET_JITAN_AUTO_MODE_ONOFF => (cfg::GET_JITAN_AUTO_MODE_ONOFF, 2),
        cfg::SET_JITAN_KOE_REPLAY_ONOFF => (cfg::GET_JITAN_KOE_REPLAY_ONOFF, 0),
        cfg::SET_JITAN_KOE_REPLAY_ONOFF_DEFAULT => (cfg::GET_JITAN_KOE_REPLAY_ONOFF, 1),
        cfg::GET_JITAN_KOE_REPLAY_ONOFF => (cfg::GET_JITAN_KOE_REPLAY_ONOFF, 2),
        cfg::SET_JITAN_SPEED => (cfg::GET_JITAN_SPEED, 0),
        cfg::SET_JITAN_SPEED_DEFAULT => (cfg::GET_JITAN_SPEED, 1),
        cfg::GET_JITAN_SPEED => (cfg::GET_JITAN_SPEED, 2),
        _ => return Ok(false),
    };
    let fallback = if key == cfg::GET_JITAN_SPEED { 100 } else { 0 };
    if action == 2 {
        ctx.push(Value::Int(
            ctx.globals
                .syscom
                .config_int
                .get(&key)
                .copied()
                .unwrap_or(fallback),
        ));
    } else {
        let value = if action == 1 {
            fallback
        } else {
            let value = args.first().and_then(Value::as_i64).unwrap_or(0);
            if key == cfg::GET_JITAN_SPEED {
                value.clamp(100, 300)
            } else {
                i64::from(value != 0)
            }
        };
        ctx.globals.syscom.config_int.insert(key, value);
    }
    Ok(true)
}
