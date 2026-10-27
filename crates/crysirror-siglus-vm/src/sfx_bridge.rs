// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original PCM/global SE values and resource policy, over CrySirror group and
//! channel APIs. No source buffers, sample clocks, envelopes or decoder caches.
use crate::{
    audio::AudioHub,
    audio_queue::Queue,
    bgm_bridge::Operation,
    runtime::{
        CommandContext, Value,
        forms::{
            codes::{pcm_op, se_op},
            prop_access,
        },
        wait::AudioWait,
    },
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioPlaybackInfo, AudioRequest},
};
use serde::{Deserialize, Serialize};
pub(crate) const PCM_CHANNEL: u32 = 1;
pub(crate) const SE_CHANNEL: u32 = 2;
pub(crate) const PCM_GROUP: &str = "siglus.pcm";
pub(crate) const SE_GROUP: &str = "siglus.se";
#[derive(Clone, Serialize, Deserialize)]
struct State {
    volume: u8,
    current: Option<String>,
    playing: bool,
    #[serde(skip)]
    initialized: bool,
    #[serde(skip)]
    pending: Queue,
}
impl Default for State {
    fn default() -> Self {
        Self {
            volume: 255,
            current: None,
            playing: false,
            initialized: false,
            pending: Default::default(),
        }
    }
}
pub(crate) fn duration(n: i64) -> Result<f32> {
    ensure!(
        n.max(0) <= 600000,
        "Siglus sound duration exceeds engine limit"
    );
    Ok(n.max(0) as f32)
}
pub(crate) fn key(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty() && s.len() <= 1024,
        "invalid Siglus sound alias"
    );
    let p = VfsPath::new(Mount::Asset, s).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        p.relative() == s,
        "Siglus sound needs a canonical resource alias"
    );
    Ok(())
}
impl State {
    fn validate(&self) -> Result<()> {
        if let Some(s) = &self.current {
            key(s)?;
        }
        ensure!(
            !self.playing || self.current.is_some(),
            "saved native sound lacks declaration"
        );
        Ok(())
    }
    fn volume(&mut self, group: &str, v: u8, fade: i64) -> Result<()> {
        let fade_ms = duration(fade)?;
        self.pending
            .push(Operation::Control(AudioRequest::SetMixGroupGain {
                name: group.into(),
                gain: f32::from(v) / 255.,
                fade_ms,
            }))?;
        self.volume = v;
        self.initialized = true;
        Ok(())
    }
    fn play(&mut self, channel: u32, group: &str, k: &str) -> Result<()> {
        key(k)?;
        if !self.initialized {
            self.volume(group, self.volume, 0)?;
        }
        self.pending.push(Operation::Start {
            channel,
            key: k.into(),
            group: group.into(),
        })?;
        self.current = Some(k.into());
        self.playing = true;
        Ok(())
    }
    fn stop(&mut self, channel: u32, fade: Option<i64>) -> Result<()> {
        let fade_ms = duration(fade.unwrap_or(0))?;
        if self.current.is_some() {
            self.pending
                .push(Operation::Control(AudioRequest::Stop { channel, fade_ms }))?;
        }
        self.current = None;
        self.playing = false;
        Ok(())
    }
    fn sync(&mut self, info: Option<AudioPlaybackInfo>) -> Result<()> {
        if self.current.is_none() {
            self.playing = false;
            return Ok(());
        }
        let Some(i) = info.filter(|i| i.source_playing) else {
            self.current = None;
            self.playing = false;
            return Ok(());
        };
        ensure!(
            i.source_rate > 0 && i.output_rate > 0,
            "invalid shared audio clock"
        );
        let frames = i
            .source_frames
            .context("Siglus sound wait needs a measured engine source length")?;
        // Original SfxEngine's WAV duration truncates to integer milliseconds.
        // The engine's publication age supplies the clock; no VM timer/cache.
        let duration_ms = u128::from(frames) * 1000 / u128::from(i.source_rate);
        let elapsed_ms = u128::from(i.age_frames) * 1000 / u128::from(i.output_rate);
        self.playing = elapsed_ms < duration_ms
            && !i.paused
            && i.fade_out_frames == 0
            && i.resume_delay_frames == 0;
        if elapsed_ms >= duration_ms {
            self.current = None;
        }
        Ok(())
    }
    fn rebind(&mut self, channel: u32, group: &str) -> Result<()> {
        self.initialized = false;
        self.pending = Default::default();
        if self.volume != 255 || self.playing {
            self.volume(group, self.volume, 0)?;
        }
        if self.playing {
            let k = self
                .current
                .clone()
                .context("native sound declaration missing")?;
            self.play(channel, group, &k)?;
        }
        Ok(())
    }
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct PcmEngine {
    state: State,
    pub(crate) channels: crate::pcmch_bridge::Channels,
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct SeEngine {
    state: State,
}
impl PcmEngine {
    pub fn volume_raw(&self) -> u8 {
        self.state.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> Result<()> {
        self.state.volume(PCM_GROUP, v, 0)
    }
    pub fn stop(&mut self, fade: Option<i64>) -> Result<()> {
        self.state.stop(PCM_CHANNEL, fade)
    }
    pub fn stop_all(&mut self, fade: Option<i64>) -> Result<()> {
        self.stop(fade)?;
        self.channels.stop_all(fade.unwrap_or(0))
    }
    pub fn is_playing_any(&mut self) -> bool {
        self.state.playing || self.channels.any()
    }
    pub fn is_playing_slot(&mut self, ch: usize) -> bool {
        self.channels.playing(ch)
    }
    pub fn is_fading_slot(&mut self, ch: usize) -> bool {
        self.channels.fading(ch)
    }
    pub fn slot_volume_raw(&self, ch: usize) -> i32 {
        self.channels.volume(ch)
    }
    pub fn slot_resume_delay_ms(&self, ch: usize) -> i64 {
        self.channels.delay(ch)
    }
    pub(crate) fn ensure_group(&mut self) -> Result<()> {
        if !self.state.initialized {
            self.state.volume(PCM_GROUP, self.state.volume, 0)?;
        }
        Ok(())
    }
    pub(crate) fn has_source(&self) -> bool {
        self.state.current.is_some()
    }
    pub(crate) fn sync(&mut self, i: Option<AudioPlaybackInfo>) -> Result<()> {
        self.state.sync(i)
    }
    pub(crate) fn rebind(&mut self) -> Result<()> {
        self.state.rebind(PCM_CHANNEL, PCM_GROUP)?;
        if !self.channels.active().is_empty() {
            self.ensure_group()?;
        }
        self.channels.rebind()
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.state.validate()?;
        self.channels.validate()
    }
    pub(crate) fn pending(&self) -> bool {
        !self.state.pending.is_empty() || self.channels.pending()
    }
    pub(crate) fn take(&mut self) -> Queue {
        let mut queue = std::mem::take(&mut self.state.pending);
        queue.append(self.channels.take());
        queue
    }
}
impl SeEngine {
    pub fn volume_raw(&self) -> u8 {
        self.state.volume
    }
    pub fn set_volume_raw(&mut self, _: &mut AudioHub, v: u8) -> Result<()> {
        self.state.volume(SE_GROUP, v, 0)
    }
    pub fn stop(&mut self, fade: Option<i64>) -> Result<()> {
        self.state.stop(SE_CHANNEL, fade)
    }
    pub fn is_playing_any(&mut self) -> bool {
        self.state.playing
    }
    pub(crate) fn has_source(&self) -> bool {
        self.state.current.is_some()
    }
    pub(crate) fn sync(&mut self, i: Option<AudioPlaybackInfo>) -> Result<()> {
        self.state.sync(i)
    }
    pub(crate) fn rebind(&mut self) -> Result<()> {
        self.state.rebind(SE_CHANNEL, SE_GROUP)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.state.validate()
    }
    pub(crate) fn pending(&self) -> bool {
        !self.state.pending.is_empty()
    }
    pub(crate) fn take(&mut self) -> Queue {
        std::mem::take(&mut self.state.pending)
    }
}
fn integer(a: &[Value], i: usize, default: i64) -> i64 {
    a.get(i).and_then(Value::as_i64).unwrap_or(default)
}
pub(crate) fn pcm(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let op = prop_access::current_op_from_ctx_or_args(ctx, args)
        .context("Siglus PCM opcode required")?;
    let args = prop_access::params_without_op(ctx, args);
    match op {
        pcm_op::PLAY => {
            let k = args.first().and_then(Value::as_str).context(
                "Siglus PCM.PLAY resource alias required; property fallback unsupported",
            )?;
            ctx.pcm.state.play(PCM_CHANNEL, PCM_GROUP, k)?;
        }
        pcm_op::STOP => ctx.pcm.stop(None)?,
        _ => bail!("unsupported Siglus PCM opcode {op}"),
    }
    Ok(true)
}
pub(crate) fn se(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let op =
        prop_access::current_op_from_ctx_or_args(ctx, args).context("Siglus SE opcode required")?;
    let ret = prop_access::current_vm_meta(ctx).1.unwrap_or(0) != 0;
    let args = prop_access::params_without_op(ctx, args);
    match op {
        se_op::PLAY_BY_FILE_NAME => {
            let k = args
                .first()
                .and_then(Value::as_str)
                .context("Siglus SE file alias required")?;
            ctx.se.state.play(SE_CHANNEL, SE_GROUP, k)?;
        }
        se_op::PLAY | se_op::PLAY_BY_SE_NO => {
            let no = args
                .first()
                .and_then(Value::as_i64)
                .context("Siglus SE number required")?;
            let cfg = ctx
                .tables
                .gameexe
                .as_ref()
                .context("Siglus SE numbered playback needs Gameexe SE table")?;
            let count = cfg.get_usize("SE.CNT").unwrap_or(16).clamp(8, 256);
            ensure!(
                no >= 0 && (no as u64) < count as u64,
                "Siglus SE number outside native table"
            );
            let k = cfg
                .get_indexed_unquoted("SE", no as usize)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("Siglus SE table entry missing")?
                .to_owned();
            ctx.se.state.play(SE_CHANNEL, SE_GROUP, &k)?;
        }
        se_op::PLAY_BY_KOE_NO => {
            let no = args
                .first()
                .and_then(Value::as_i64)
                .context("Siglus SE voice number required")?;
            ensure!(no >= 0, "negative Siglus SE voice number");
            // Explicit CrySirror alias profile; no native pack/path discovery.
            ctx.se
                .state
                .play(SE_CHANNEL, SE_GROUP, &format!("koe/{no}"))?;
        }
        se_op::STOP => ctx.se.stop(Some(integer(args, 0, 0)))?,
        se_op::SET_VOLUME | se_op::SET_VOLUME_MAX | se_op::SET_VOLUME_MIN => {
            let v = match op {
                se_op::SET_VOLUME => args
                    .first()
                    .and_then(Value::as_i64)
                    .context("Siglus SE volume required")?
                    .clamp(0, 255) as u8,
                se_op::SET_VOLUME_MAX => 255,
                _ => 0,
            };
            let fade = integer(args, usize::from(op == se_op::SET_VOLUME), 0);
            ctx.se.state.volume(SE_GROUP, v, fade)?;
        }
        se_op::GET_VOLUME => ctx.push(Value::Int(i64::from(ctx.se.volume_raw()))),
        se_op::CHECK => {
            let playing = ctx.se.is_playing_any();
            ctx.push(Value::Int(i64::from(playing)));
        }
        se_op::WAIT | se_op::WAIT_KEY => {
            ctx.wait.wait_audio(AudioWait::SeAny, op == se_op::WAIT_KEY);
            if ret {
                ctx.push(Value::Int(0));
            }
        }
        _ => bail!("unsupported Siglus SE opcode {op}"),
    }
    Ok(true)
}
