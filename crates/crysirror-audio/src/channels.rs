// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Client channel service over the shared streaming decoder and PCM mixer.
//! Adapters supply decoded container policy, never devices or ambient paths.
use crate::{
    decoder::{AudioSource, StereoFrame},
    mixer::{Bus, Ducking, LoopPoints, Mixer, MixerConfig, VoiceHandle, VoiceSpec},
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::Mount,
    service::{AudioAsset, AudioParameters, AudioRequest, ClientStorage, EncodedAudio, PcmBus},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn asset(
    input: AudioAsset,
    gain: f32,
    storage: &dyn ClientStorage,
) -> Result<Box<dyn AudioSource>> {
    ensure!(
        input.path.mount() == Mount::Asset,
        "audio input requires asset authority"
    );
    let extension = input.path.relative().rsplit_once('.').map(|v| v.1);
    let reader = storage
        .open_asset(&input.path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        reader.len().map_err(|e| anyhow::anyhow!("{e}"))? <= 512 * 1024 * 1024,
        "encoded stream size limit"
    );
    wrap_source(
        crate::decoder::open_asset(reader, extension)?,
        input.frame_limit,
        gain,
    )
}
struct Source {
    inner: Box<dyn AudioSource>,
    limit: Option<u64>,
    at: u64,
    gain: f32,
}
impl AudioSource for Source {
    fn is_nonblocking(&self) -> bool {
        self.inner.is_nonblocking()
    }
    fn read_position(&self) -> Option<u64> {
        Some(self.at)
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn frame_count(&self) -> Option<u64> {
        self.limit
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let limit = self.limit.map_or(out.len(), |n| {
            out.len()
                .min(n.saturating_sub(self.at).min(usize::MAX as u64) as usize)
        });
        if limit == 0 {
            return Ok(0);
        }
        let count = self.inner.read(&mut out[..limit])?;
        ensure!(count <= limit, "audio source exceeded requested length");
        for frame in &mut out[..count] {
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
        self.at += count as u64;
        Ok(count)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(
            self.limit.is_none_or(|n| frame <= n),
            "audio seek beyond declared samples"
        );
        self.inner.seek(frame)?;
        self.at = frame;
        Ok(())
    }
}
pub(super) fn source(encoded: EncodedAudio, gain: f32) -> Result<Box<dyn AudioSource>> {
    wrap_source(
        crate::decoder::open_memory(encoded.bytes, None)?,
        encoded.frame_limit,
        gain,
    )
}
pub(super) fn wrap_source(
    stream: Box<dyn AudioSource>,
    frame_limit: Option<u64>,
    gain: f32,
) -> Result<Box<dyn AudioSource>> {
    let limit = match (frame_limit, stream.frame_count()) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    ensure!(limit != Some(0), "empty audio source");
    Ok(Box::new(Source {
        inner: stream,
        limit,
        at: 0,
        gain,
    }))
}
struct Joined {
    intro: Box<dyn AudioSource>,
    tail: Box<dyn AudioSource>,
    intro_len: u64,
    len: u64,
    at: u64,
}
impl AudioSource for Joined {
    fn sample_rate(&self) -> u32 {
        self.intro.sample_rate()
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.len)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let mut count = 0;
        while count < out.len() && self.at < self.len {
            let in_intro = self.at < self.intro_len;
            let remaining = if in_intro {
                self.intro_len - self.at
            } else {
                self.len - self.at
            };
            let wanted = (out.len() - count).min(remaining.min(usize::MAX as u64) as usize);
            let n = if in_intro {
                self.intro.read(&mut out[count..count + wanted])?
            } else {
                self.tail.read(&mut out[count..count + wanted])?
            };
            ensure!(
                n > 0 && n <= wanted,
                "audio ended before its declared segment boundary"
            );
            self.at += n as u64;
            count += n;
        }
        Ok(count)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(frame <= self.len, "audio sequence seek beyond source");
        if frame < self.intro_len {
            self.intro.seek(frame)?;
            self.tail.seek(0)?;
        } else {
            self.tail.seek(frame - self.intro_len)?;
        }
        self.at = frame;
        Ok(())
    }
}
pub(crate) struct PreparedChannel {
    pub source: Box<dyn AudioSource>,
    pub spec: VoiceSpec,
    pub start: u64,
    pub pan: f32,
    pub replace_fade_ms: f32,
}
pub(super) fn prepare(
    encoded: Box<dyn AudioSource>,
    intro: Option<Box<dyn AudioSource>>,
    p: AudioParameters,
) -> Result<PreparedChannel> {
    ensure!(
        p.decode_gain.is_finite() && (0.0..=16.).contains(&p.decode_gain),
        "invalid decoded audio gain"
    );
    ensure!(
        p.pan.is_finite() && (0.0..=1.).contains(&p.pan),
        "invalid stereo pan"
    );
    let mut tail = encoded;
    let mut loop_start = p.loop_start;
    if let Some(intro) = intro {
        let intro = intro;
        ensure!(
            intro.sample_rate() == tail.sample_rate(),
            "audio sequence rates differ"
        );
        let intro_len = intro
            .frame_count()
            .context("intro sample count unavailable")?;
        let len = intro_len
            .checked_add(
                tail.frame_count()
                    .context("loop sample count unavailable")?,
            )
            .context("audio sequence length overflow")?;
        loop_start = intro_len
            .checked_add(loop_start)
            .context("loop offset overflow")?;
        tail = Box::new(Joined {
            intro,
            tail,
            intro_len,
            len,
            at: 0,
        });
    }
    let mut start = match p.start_frame {
        Some(frame) => frame,
        None => {
            p.start_ms
                .checked_mul(tail.sample_rate() as u64)
                .context("audio start position overflow")?
                / 1000
        }
    };
    let mut looping = p.looping;
    if p.clamp_range {
        let end = tail
            .frame_count()
            .context("clamped audio range needs a known sample count")?;
        start = start.min(end);
        loop_start = loop_start.min(end);
        looping &= loop_start < end;
        if start >= end && looping {
            start = loop_start;
        }
        ensure!(start < end, "empty clamped audio range");
    }
    let loop_points = if looping && loop_start > 0 {
        Some(LoopPoints {
            start: loop_start,
            end: tail
                .frame_count()
                .context("nonzero loop point needs a known sample count")?,
        })
    } else {
        None
    };
    let bus = match p.bus {
        PcmBus::Music => Bus::Music,
        PcmBus::Sfx => Bus::Sfx,
        PcmBus::Voice => Bus::Voice,
        PcmBus::Ambient => Bus::Ambient,
        PcmBus::Ui => Bus::Ui,
    };
    let spec = VoiceSpec {
        mix_group: p.mix_group.clone(),
        bus,
        gain: p.gain,
        rate: p.rate,
        looping,
        loop_points,
        fade_in_ms: p.fade_ms,
        paused: p.paused,
        ..Default::default()
    };
    spec.validate()?;
    ensure!(
        p.replace_fade_ms.is_finite() && (0. ..=600000.).contains(&p.replace_fade_ms),
        "audio replacement fade duration"
    );
    ensure!(
        tail.frame_count().is_none_or(|len| start <= len),
        "audio start position exceeds source"
    );
    if start > 0 {
        tail.seek(start)?;
    }
    Ok(PreparedChannel {
        source: tail,
        spec,
        start,
        pan: p.pan,
        replace_fade_ms: p.replace_fade_ms,
    })
}
pub struct ChannelAudio {
    pub sequences: crate::sequence::Sequences,
    pub mixer: Mixer,
    pub resources: crate::resources::AudioResources,
    pub(super) voices: BTreeMap<(u32, u32), VoiceHandle>,
    pub(super) clients: BTreeSet<u32>,
    pub(super) output: Vec<StereoFrame>,
    pub(super) remainder: u64,
    pub(super) jobs: super::channel_async::ChannelJobs,
}
impl Default for ChannelAudio {
    fn default() -> Self {
        Self {
            sequences: Default::default(),
            mixer: Mixer::new(MixerConfig {
                max_voices: 256,
                ..Default::default()
            })
            .expect("valid channel mixer"),
            resources: Default::default(),
            voices: BTreeMap::new(),
            clients: BTreeSet::new(),
            output: Vec::new(),
            remainder: 0,
            jobs: Default::default(),
        }
    }
}
impl ChannelAudio {
    pub fn playback_info(
        &self,
        client: u32,
        channel: u32,
    ) -> Result<Option<crysirror_core::service::AudioPlaybackInfo>> {
        self.voices
            .get(&(client, channel))
            .filter(|h| self.mixer.is_playing(client, **h))
            .map(|h| self.mixer.playback_info(client, *h))
            .transpose()
    }
    /// Authoritative, client-scoped channel state from the shared mixer.
    pub fn is_playing(&self, client: u32, channel: u32) -> bool {
        self.voices
            .get(&(client, channel))
            .is_some_and(|handle| self.mixer.source_is_playing(client, *handle))
    }

    /// Publish a shared echo resource on a scoped channel, with native completion
    /// still defined by source EOF rather than the retained echo tail.
    pub fn set_reflections(
        &mut self,
        client: u32,
        channel: u32,
        effect: Option<crysirror_acoustics::reflection_dsp::EarlyReflections>,
    ) -> Result<()> {
        let handle = *self
            .voices
            .get(&(client, channel))
            .context("audio channel not found")?;
        self.mixer.set_reflections(client, handle, effect)?;
        Ok(())
    }
    pub fn reflection_plan_ready(&self, client: u32, channel: u32) -> Result<bool> {
        let h = *self
            .voices
            .get(&(client, channel))
            .context("audio channel not found")?;
        self.mixer.reflection_plan_ready(client, h)
    }
    /// Publish a worker-prepared moving-path plan while keeping channel history.
    pub fn publish_reflection_plan(
        &mut self,
        client: u32,
        channel: u32,
        plan: crysirror_acoustics::reflection_dsp::ReflectionPlan,
        crossfade_ms: u32,
    ) -> Result<Option<crysirror_acoustics::reflection_dsp::ReflectionPlan>> {
        let h = *self
            .voices
            .get(&(client, channel))
            .context("audio channel not found")?;
        self.mixer
            .publish_reflection_plan(client, h, plan, crossfade_ms)
    }
    pub fn set_ducking(&mut self, client: u32, config: Ducking) -> Result<()> {
        self.mixer.set_ducking(client, config)?;
        self.clients.insert(client);
        Ok(())
    }
    pub fn submit(&mut self, client: u32, request: AudioRequest) -> Result<()> {
        self.mixer.idle()?;
        if self.clients.insert(client) {
            // Native VMs retain their own ducking policy. Users can explicitly
            // configure the shared mixer; playback does not invent a new fade.
            self.mixer.set_ducking(
                client,
                Ducking {
                    gain: 1.,
                    ..Default::default()
                },
            )?;
        }
        self.voices
            .retain(|_, h| self.mixer.is_playing(h.client, *h));
        self.cancel_pending_control(client, &request)?;
        match request {
            AudioRequest::SetMixGroupGain {
                name,
                gain,
                fade_ms,
            } => {
                self.mixer
                    .set_mix_group_gain(client, &name, gain, fade_ms)?;
            }
            AudioRequest::SetMixGroup {
                name,
                settings,
                fade_ms,
            } => {
                self.mixer.set_mix_group(client, &name, settings, fade_ms)?;
            }
            AudioRequest::PlayPcm {
                channel,
                source,
                parameters: p,
            } => {
                if !p.restart && self.is_playing(client, channel) {
                    return Ok(());
                }
                let ready = prepare(
                    wrap_source(crate::decoder::open_pcm(source), None, p.decode_gain)?,
                    None,
                    p,
                )?;
                self.publish(client, channel, ready)?;
            }
            AudioRequest::PlayAsset { .. } | AudioRequest::PlayResource { .. } => {
                anyhow::bail!("asset channel playback requires begin preparation")
            }
            AudioRequest::Play {
                channel,
                source: encoded,
                intro,
                parameters: p,
            } => {
                if !p.restart && self.voices.contains_key(&(client, channel)) {
                    return Ok(());
                }
                self.play(client, channel, encoded, intro, p)?;
            }
            AudioRequest::Stop { channel, fade_ms } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.stop(client, h, fade_ms)?;
                }
            }
            AudioRequest::Pause {
                channel,
                paused,
                fade_ms,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.pause_fade(client, h, paused, fade_ms)?;
                }
            }
            AudioRequest::Resume {
                channel,
                delay_ms,
                fade_ms,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.schedule_resume(client, h, delay_ms, fade_ms)?;
                }
            }
            AudioRequest::ResumeWithOptions {
                channel,
                delay_ms,
                fade_ms,
                settings,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer
                        .schedule_resume_with_settings(client, h, delay_ms, fade_ms, settings)?;
                }
            }
            AudioRequest::SetGain {
                channel,
                gain,
                fade_ms,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.fade_gain(client, h, gain, fade_ms)?;
                }
            }
            AudioRequest::SetLooped {
                channel,
                looping,
                loop_start,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.set_looped(client, h, looping, loop_start)?;
                }
            }
            AudioRequest::SetPan {
                channel,
                pan,
                fade_ms,
            } => {
                if let Some(&h) = self
                    .voices
                    .get(&(client, channel))
                    .filter(|h| self.mixer.is_playing(client, **h))
                {
                    self.mixer.set_pan(client, h, pan, fade_ms)?;
                }
            }
        }
        Ok(())
    }
    fn play(
        &mut self,
        client: u32,
        channel: u32,
        encoded: EncodedAudio,
        intro: Option<EncodedAudio>,
        p: AudioParameters,
    ) -> Result<()> {
        let prepared = prepare(
            source(encoded, p.decode_gain)?,
            intro.map(|v| source(v, p.decode_gain)).transpose()?,
            p,
        )?;
        self.publish(client, channel, prepared)
    }
    pub(crate) fn publish(
        &mut self,
        client: u32,
        channel: u32,
        prepared: PreparedChannel,
    ) -> Result<()> {
        let PreparedChannel {
            source: tail,
            spec,
            start,
            pan,
            replace_fade_ms,
        } = prepared;
        let previous = self
            .voices
            .get(&(client, channel))
            .copied()
            .filter(|h| self.mixer.is_playing(client, *h));
        ensure!(
            previous.is_some() || self.mixer.stats().active < 256,
            "shared channel voice capacity"
        );
        let h = if let Some(old) = previous.filter(|_| replace_fade_ms == 0.) {
            self.mixer.replace(client, old, tail, spec)?
        } else {
            ensure!(
                self.mixer.has_spare_voice(client),
                "channel crossfade requires a spare voice"
            );
            let handle = self.mixer.play(client, tail, spec)?;
            if let Some(old) = previous {
                self.mixer.stop(client, old, replace_fade_ms)?;
            }
            handle
        };
        self.mixer.prepared_position(client, h, start)?;
        self.mixer.set_pan(client, h, pan, 0.)?;
        self.voices.insert((client, channel), h);
        Ok(())
    }
    /// Owner control phase, shared by synchronous and worker frame paths.
    pub fn prepare_render(&mut self, delta_ms: u64) -> Result<usize> {
        self.mixer.idle()?;
        self.poll_preparations()?;
        for (&(client, _), &h) in &self.voices {
            if self.mixer.is_playing(client, h) {
                self.mixer.retire_reflection_plan(client, h)?;
            }
        }
        // An idle synchronous/offline runtime may advance a long simulation
        // interval without producing PCM. Preserve that contract; active audio
        // and worker frame admission still enforce their two-second bounds.
        if self.clients.is_empty() && !self.mixer.requires_render() {
            self.output.clear();
            return Ok(0);
        }
        let numerator = delta_ms
            .checked_mul(48_000)
            .and_then(|v| v.checked_add(self.remainder))
            .context("channel audio clock overflow")?;
        let count = numerator / 1000;
        ensure!(count <= 96_000, "channel audio frame exceeds two seconds");
        self.remainder = numerator % 1000;
        self.output.clear();
        Ok(count as usize)
    }
    pub fn render(&mut self, delta_ms: u64) -> Result<()> {
        let count = self.prepare_render(delta_ms)?;
        self.output.resize(count, [0.; 2]);
        for block in self.output.chunks_mut(4096) {
            self.mixer.render(block)?;
        }
        Ok(())
    }
    /// Publish only after a matching worker job restored this mixer.
    pub fn publish_output(&mut self, pcm: Vec<StereoFrame>) -> Result<()> {
        self.mixer.idle()?;
        ensure!(pcm.len() <= 96_000, "channel output extent");
        self.output = pcm;
        Ok(())
    }
    pub fn take_output(&mut self) -> Vec<StereoFrame> {
        std::mem::take(&mut self.output)
    }
}

#[cfg(test)]
mod prepared_cursor_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Source {
        at: u64,
        seeks: Arc<AtomicUsize>,
    }
    impl AudioSource for Source {
        fn sample_rate(&self) -> u32 {
            48000
        }
        fn frame_count(&self) -> Option<u64> {
            Some(100)
        }
        fn read_position(&self) -> Option<u64> {
            Some(self.at)
        }
        fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
            let n = out.len().min((100 - self.at) as usize);
            for frame in &mut out[..n] {
                *frame = [self.at as f32 / 100.; 2];
                self.at += 1;
            }
            Ok(n)
        }
        fn seek(&mut self, frame: u64) -> Result<()> {
            self.seeks.fetch_add(1, Ordering::SeqCst);
            self.at = frame;
            Ok(())
        }
    }
    #[test]
    fn worker_prepared_cursor_does_not_seek_again_on_publication_or_first_mix() {
        let seeks = Arc::new(AtomicUsize::new(0));
        let prepared = prepare(
            Box::new(Source {
                at: 0,
                seeks: seeks.clone(),
            }),
            None,
            AudioParameters {
                start_frame: Some(7),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(seeks.load(Ordering::SeqCst), 1);
        let mut channels = ChannelAudio::default();
        channels.publish(1, 0, prepared).unwrap();
        let mut output = [[0.; 2]; 1];
        channels.mixer.render(&mut output).unwrap();
        assert!((output[0][0] - 0.07).abs() < 1e-6);
        assert_eq!(seeks.load(Ordering::SeqCst), 1);
    }
}
