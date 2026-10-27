// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Siglus BGM values/waits translated to shared channel transport. No decoder,
//! clocked mixer, sample buffer, envelope algorithm or output device lives here.
use crate::{
    audio::AudioHub,
    runtime::{
        CommandContext, Value,
        forms::{codes::bgm_op as op, prop_access},
        wait::AudioWait,
    },
};
use anyhow::{Result, bail, ensure};
use crysirror_core::service::{
    AudioParameters, AudioPlaybackInfo, AudioRequest, AudioResourceHandle,
};
use serde::{Deserialize, Serialize};

pub(crate) const CHANNEL: u32 = 0;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Play {
    pub name: String,
    pub looping: bool,
    pub fade_in: i64,
    pub fade_out: i64,
    pub start: i64,
    pub paused: bool,
    pub delay: i64,
}
#[derive(Clone)]
pub(crate) struct Binding {
    pub resource: AudioResourceHandle,
    pub parameters: AudioParameters,
}
#[derive(Clone)]
pub(crate) enum Operation {
    Play(Play),
    Retain(Play),
    Replay { fade: f32, delay: f32 },
    Control(AudioRequest),
}
#[derive(Clone, Serialize, Deserialize)]
pub struct BgmEngine {
    volume: u8,
    name: Option<String>,
    looping: bool,
    paused: bool,
    state: i32,
    position: u64,
    initial_position: u64,
    delay: i64,
    pub(crate) current: Option<Play>,
    #[serde(skip)]
    pub(crate) binding: Option<Binding>,
    #[serde(skip)]
    pub(crate) pending: Vec<Operation>,
}
impl Default for BgmEngine {
    fn default() -> Self {
        Self {
            volume: 255,
            name: None,
            looping: false,
            paused: false,
            state: 0,
            position: 0,
            initial_position: 0,
            delay: 0,
            current: None,
            binding: None,
            pending: vec![],
        }
    }
}
fn milliseconds(n: i64) -> Result<f32> {
    ensure!(
        n.max(0) <= 600000,
        "Siglus BGM duration exceeds engine limit"
    );
    Ok(n.max(0) as f32)
}
impl BgmEngine {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            (0..=3).contains(&self.state) && (0..=600000).contains(&self.delay),
            "invalid saved Siglus BGM transport state"
        );
        ensure!(
            self.state == 0 || self.current.is_some(),
            "saved Siglus BGM has no declaration"
        );
        if let Some(play) = &self.current {
            ensure!(
                !play.name.is_empty()
                    && play.name.len() <= 1024
                    && play.name == play.name.trim().to_ascii_lowercase(),
                "invalid saved Siglus BGM name"
            );
            milliseconds(play.fade_in)?;
            milliseconds(play.fade_out)?;
            milliseconds(play.delay)?;
            ensure!(
                self.name.as_ref().is_none_or(|name| name == &play.name),
                "saved Siglus BGM name/declaration mismatch"
            );
        } else {
            ensure!(
                self.name.is_none(),
                "saved Siglus BGM name has no declaration"
            );
        }
        Ok(())
    }
    pub fn volume_raw(&self) -> u8 {
        self.volume
    }
    pub fn current_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn save_delay_time_ms(&self) -> i64 {
        self.delay
    }
    pub fn save_loop_flag(&self) -> bool {
        self.looping
    }
    pub fn save_pause_flag(&self) -> bool {
        self.paused
    }
    pub fn is_playing(&mut self) -> bool {
        self.state == 1
    }
    pub fn is_fade_out_doing(&mut self) -> bool {
        self.state == 2
    }
    pub(crate) fn sync(&mut self, info: Option<AudioPlaybackInfo>) {
        match info.filter(|i| i.source_playing) {
            Some(i) => {
                self.state = if i.paused {
                    3
                } else if i.fade_out_frames > 0 {
                    2
                } else {
                    1
                };
                self.position = i.position_frames;
                self.delay =
                    (i.resume_delay_frames.saturating_mul(1000) / u64::from(i.output_rate)) as i64;
            }
            None => {
                self.state = 0;
                self.position = self.initial_position;
                self.delay = 0;
            }
        }
    }
    pub fn stop(&mut self) -> Result<()> {
        self.stop_fade(0)
    }
    fn stop_fade(&mut self, duration: i64) -> Result<()> {
        let fade_ms = milliseconds(duration)?;
        if self.current.is_some() {
            self.pending.push(Operation::Control(AudioRequest::Stop {
                channel: CHANNEL,
                fade_ms,
            }));
        }
        self.name = None;
        self.looping = false;
        self.paused = false;
        self.delay = 0;
        if self.state != 3 {
            self.state = if fade_ms > 0. && self.state != 0 {
                2
            } else {
                0
            };
        }
        Ok(())
    }
    pub fn set_volume_raw(&mut self, audio: &mut AudioHub, v: u8) -> Result<()> {
        self.volume_fade(audio, v, 0)
    }
    fn volume_fade(&mut self, _: &mut AudioHub, v: u8, duration: i64) -> Result<()> {
        let fade_ms = milliseconds(duration)?;
        self.volume = v;
        if self.current.is_some() {
            self.pending.push(Operation::Control(AudioRequest::SetGain {
                channel: CHANNEL,
                gain: f32::from(v) / 255.,
                fade_ms,
            }));
        }
        Ok(())
    }
    pub fn play_name_script(
        &mut self,
        _: &mut AudioHub,
        name: &str,
        looping: bool,
        fade_in: i64,
        fade_out: i64,
        start: i64,
        paused: bool,
        delay: i64,
    ) -> Result<()> {
        milliseconds(fade_in)?;
        milliseconds(fade_out)?;
        milliseconds(delay)?;
        let name = name.trim().to_ascii_lowercase();
        ensure!(
            !name.is_empty() && name.len() <= 1024,
            "invalid Siglus BGM name"
        );
        if self.name.as_deref() == Some(&name) && self.looping && looping {
            return Ok(());
        }
        let play = Play {
            name: name.clone(),
            looping,
            fade_in,
            fade_out,
            start,
            paused,
            delay,
        };
        self.pending.push(Operation::Play(play.clone()));
        self.current = Some(play);
        self.name = Some(name);
        self.looping = looping;
        self.paused = paused;
        self.delay = delay.max(0);
        self.position = start.max(0) as u64;
        self.initial_position = self.position;
        self.state = if paused || delay > 0 { 3 } else { 1 };
        Ok(())
    }
    fn pause(&mut self, duration: i64) -> Result<()> {
        let fade_ms = milliseconds(duration)?;
        self.paused = true;
        self.delay = 0;
        if self.state != 0 && self.state != 3 {
            self.pending.push(Operation::Control(AudioRequest::Pause {
                channel: CHANNEL,
                paused: true,
                fade_ms,
            }));
            self.state = if fade_ms > 0. { 2 } else { 3 };
        }
        Ok(())
    }
    fn resume(&mut self, fade: i64, delay: i64) -> Result<()> {
        let fade_ms = milliseconds(fade)?;
        let delay_ms = milliseconds(delay)?;
        if self.current.is_some() {
            if self.state == 0 {
                self.pending.push(Operation::Replay {
                    fade: fade_ms,
                    delay: delay_ms,
                });
            } else {
                self.pending.push(Operation::Control(AudioRequest::Resume {
                    channel: CHANNEL,
                    delay_ms,
                    fade_ms,
                }));
            }
            self.paused = false;
            self.delay = delay.max(0);
            self.state = if delay > 0 { 3 } else { 1 };
        }
        Ok(())
    }
    pub(crate) fn set_initial_position(&mut self, position: u64) {
        self.initial_position = position;
    }
    /// Recreate derived playback only after a candidate restore is committed.
    pub(crate) fn rebind(&mut self) {
        self.binding = None;
        if let Some(mut play) = self.current.clone() {
            if self.state != 0 && self.name.is_some() {
                play.paused = self.paused || (self.state == 3 && self.delay == 0);
                play.delay = self.delay;
                play.fade_in = 0;
                play.fade_out = 0;
                self.pending.push(Operation::Play(play));
            } else {
                // A stopped slot retains source metadata; restoring it must not
                // create an audible voice merely to reconstruct the resource.
                self.pending.push(Operation::Retain(play));
            }
        }
    }
    pub(crate) fn check_state(&self) -> i32 {
        self.state
    }
}
fn integer(a: &[Value], index: usize, default: i64) -> i64 {
    a.get(index).and_then(Value::as_i64).unwrap_or(default)
}
fn named(a: &[Value], id: i32) -> Option<&Value> {
    a.iter().find_map(|v| match v {
        Value::NamedArg { id: n, value } if *n == id => Some(value.as_ref()),
        _ => None,
    })
}
fn ni(a: &[Value], id: i32) -> Option<i64> {
    named(a, id).and_then(Value::as_i64)
}
pub(crate) fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let operation = prop_access::current_op_from_ctx_or_args(ctx, args)
        .ok_or_else(|| anyhow::anyhow!("Siglus BGM opcode required"))?;
    let ret = prop_access::current_vm_meta(ctx).1.unwrap_or(0) != 0;
    match operation {
        op::PLAY | op::PLAY_ONESHOT | op::PLAY_WAIT | op::READY | op::READY_ONESHOT => {
            let name = named(args, 0)
                .and_then(Value::as_str)
                .or_else(|| args.first().and_then(Value::as_str))
                .ok_or_else(|| anyhow::anyhow!("Siglus BGM name required"))?
                .to_owned();
            let mut looping = operation == op::PLAY || operation == op::READY;
            let mut wait = operation == op::PLAY_WAIT;
            let ready = operation == op::READY || operation == op::READY_ONESHOT;
            let mut fade_in = if operation == op::READY {
                0
            } else {
                integer(args, 1, 0)
            };
            let mut fade_out = if operation == op::READY {
                integer(args, 1, 100)
            } else {
                integer(args, 2, 0)
            };
            if let Some(v) = ni(args, 1) {
                looping = v != 0;
            }
            if let Some(v) = ni(args, 2) {
                wait = v != 0;
            }
            if let Some(v) = ni(args, 4) {
                fade_in = v;
            }
            if let Some(v) = ni(args, 5) {
                fade_out = v;
            }
            if wait {
                looping = false;
            }
            ctx.bgm.play_name_script(
                &mut ctx.audio,
                &name,
                looping,
                fade_in,
                fade_out,
                ni(args, 3).unwrap_or(-1),
                ready,
                0,
            )?;
            if wait {
                ctx.wait.wait_audio(AudioWait::Bgm, false);
            }
        }
        op::STOP => ctx.bgm.stop_fade(integer(args, 0, 100))?,
        op::PAUSE => ctx.bgm.pause(integer(args, 0, 100))?,
        op::RESUME | op::RESUME_WAIT => {
            ctx.bgm.resume(
                integer(args, 0, 0),
                if operation == op::RESUME {
                    ni(args, 0).unwrap_or(0)
                } else {
                    0
                },
            )?;
            if operation == op::RESUME_WAIT {
                ctx.wait.wait_audio(AudioWait::Bgm, false);
            }
        }
        op::WAIT | op::WAIT_KEY => {
            ctx.wait
                .wait_audio_with_return(AudioWait::Bgm, operation == op::WAIT_KEY, ret)
        }
        op::WAIT_FADE | op::WAIT_FADE_KEY => {
            ctx.wait
                .wait_audio_with_return(AudioWait::BgmFade, operation == op::WAIT_FADE_KEY, ret)
        }
        op::SET_VOLUME => ctx.bgm.volume_fade(
            &mut ctx.audio,
            args.first()
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Siglus BGM.SET_VOLUME argument required; property fallback unsupported"
                    )
                })?
                .clamp(0, 255) as u8,
            integer(args, 1, 0),
        )?,
        op::SET_VOLUME_MAX => ctx
            .bgm
            .volume_fade(&mut ctx.audio, 255, integer(args, 0, 0))?,
        op::SET_VOLUME_MIN => ctx
            .bgm
            .volume_fade(&mut ctx.audio, 0, integer(args, 0, 0))?,
        op::GET_VOLUME => ctx.push(Value::Int(i64::from(ctx.bgm.volume_raw()))),
        op::GET_REGIST_NAME => ctx.push(Value::Str(ctx.bgm.current_name().unwrap_or("").into())),
        op::GET_PLAY_POS => ctx.push(Value::Int(ctx.bgm.position.min(i64::MAX as u64) as i64)),
        op::CHECK => ctx.push(Value::Int(i64::from(ctx.bgm.state))),
        _ => bail!("unsupported Siglus BGM opcode {operation}"),
    }
    Ok(true)
}

pub(crate) fn dispatch_table(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    use crate::runtime::forms::codes::bgm_table_op as table;
    let operation = prop_access::current_op_from_ctx_or_args(ctx, args)
        .ok_or_else(|| anyhow::anyhow!("Siglus BGMTABLE opcode required"))?;
    let count = ctx
        .tables
        .gameexe
        .as_ref()
        .map(|cfg| {
            cfg.get_usize("BGM.CNT")
                .unwrap_or_else(|| cfg.indexed_count("BGM"))
        })
        .unwrap_or(0);
    ensure!(count <= 4096, "Siglus BGM table exceeds 4096 entries");
    ctx.globals.bgm_table_flags.resize(
        count.max(ctx.globals.bgm_table_flags.len()),
        ctx.globals.bgm_table_all_flag,
    );
    match operation {
        table::GET_COUNT => ctx.push(Value::Int(
            count
                .max(ctx.globals.bgm_table_flags.len())
                .max(ctx.globals.bgm_table_listened.len()) as i64,
        )),
        table::SET_ALL_FLAG => {
            let value = integer(args, 0, 0) != 0;
            ctx.globals.bgm_table_all_flag = value;
            ctx.globals.bgm_table_flags.fill(value);
            for flag in ctx.globals.bgm_table_listened.values_mut() {
                *flag = value;
            }
        }
        table::GET_LISTEN_BY_NAME | table::SET_LISTEN_CURRENT => {
            if args.first().and_then(Value::as_str).is_none() {
                if operation == table::GET_LISTEN_BY_NAME {
                    ctx.push(Value::Int(-1));
                }
                return Ok(true);
            }
            let name = args
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let index = (0..count).find(|i| {
                ctx.tables
                    .gameexe
                    .as_ref()
                    .and_then(|cfg| {
                        cfg.get_indexed_item_unquoted("BGM", *i, 0)
                            .or_else(|| cfg.get_indexed_field_unquoted("BGM", *i, "REGIST_NAME"))
                            .or_else(|| cfg.get_indexed_unquoted("BGM", *i))
                    })
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case(&name))
            });
            if operation == table::GET_LISTEN_BY_NAME {
                ctx.push(Value::Int(
                    index.map_or(-1, |i| i64::from(ctx.globals.bgm_table_flags[i])),
                ));
            } else if let Some(i) = index {
                let value = integer(args, 1, 0) != 0;
                ctx.globals.bgm_table_flags[i] = value;
                ctx.globals.bgm_table_listened.insert(name, value);
            }
        }
        _ => bail!("unsupported Siglus BGMTABLE opcode {operation}"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_transport_rejects_invalid_declarations_before_rebinding_resources() {
        let mut bgm = BgmEngine::default();
        bgm.state = 1;
        assert!(bgm.validate().is_err());
        bgm.current = Some(Play {
            name: "song".into(),
            looping: true,
            fade_in: 0,
            fade_out: 0,
            start: -1,
            paused: false,
            delay: 0,
        });
        bgm.name = Some("song".into());
        bgm.validate().unwrap();
        bgm.delay = 600001;
        assert!(bgm.validate().is_err());
        bgm.delay = 0;
        bgm.current.as_mut().unwrap().fade_out = 600001;
        assert!(bgm.validate().is_err());
        bgm.current.as_mut().unwrap().fade_out = 0;
        bgm.name = Some("other".into());
        assert!(bgm.validate().is_err());
        bgm.name = Some("song".into());
        bgm.state = 4;
        assert!(bgm.validate().is_err());
    }
}
