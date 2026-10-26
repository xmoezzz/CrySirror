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
    let loop_points = if p.looping && loop_start > 0 {
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
        bus,
        gain: p.gain,
        rate: p.rate,
        looping: p.looping,
        loop_points,
        fade_in_ms: p.fade_ms,
        ..Default::default()
    };
    spec.validate()?;
    let start = p
        .start_ms
        .checked_mul(tail.sample_rate() as u64)
        .context("audio start position overflow")?
        / 1000;
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
    })
}
pub struct ChannelAudio {
    pub mixer: Mixer,
    pub(super) voices: BTreeMap<(u32, u32), VoiceHandle>,
    pub(super) clients: BTreeSet<u32>,
    pub(super) output: Vec<StereoFrame>,
    pub(super) remainder: u64,
    pub(super) jobs: super::channel_async::ChannelJobs,
}
impl Default for ChannelAudio {
    fn default() -> Self {
        Self {
            mixer: Mixer::new(MixerConfig {
                max_voices: 256,
                ..Default::default()
            })
            .expect("valid channel mixer"),
            voices: BTreeMap::new(),
            clients: BTreeSet::new(),
            output: Vec::new(),
            remainder: 0,
            jobs: Default::default(),
        }
    }
}
impl ChannelAudio {
    /// Authoritative, client-scoped channel state from the shared mixer.
    pub fn is_playing(&self, client: u32, channel: u32) -> bool {
        self.voices
            .get(&(client, channel))
            .is_some_and(|handle| self.mixer.is_playing(client, *handle))
    }

    pub fn set_ducking(&mut self, client: u32, config: Ducking) -> Result<()> {
        self.mixer.set_ducking(client, config)?;
        self.clients.insert(client);
        Ok(())
    }
    pub fn submit(&mut self, client: u32, request: AudioRequest) -> Result<()> {
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
            AudioRequest::PlayAsset { .. } => {
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
                if let Some(&h) = self.voices.get(&(client, channel)) {
                    self.mixer.stop(client, h, fade_ms)?;
                }
            }
            AudioRequest::Pause {
                channel,
                paused,
                fade_ms,
            } => {
                if let Some(&h) = self.voices.get(&(client, channel)) {
                    self.mixer.pause_fade(client, h, paused, fade_ms)?;
                }
            }
            AudioRequest::SetGain {
                channel,
                gain,
                fade_ms,
            } => {
                if let Some(&h) = self.voices.get(&(client, channel)) {
                    self.mixer.fade_gain(client, h, gain, fade_ms)?;
                }
            }
            AudioRequest::SetLooped {
                channel,
                looping,
                loop_start,
            } => {
                if let Some(&h) = self.voices.get(&(client, channel)) {
                    self.mixer.set_looped(client, h, looping, loop_start)?;
                }
            }
            AudioRequest::SetPan {
                channel,
                pan,
                fade_ms,
            } => {
                if let Some(&h) = self.voices.get(&(client, channel)) {
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
        if let Some(old) = previous {
            self.mixer.stop(client, old, 0.)?;
        }
        let h = self.mixer.play(client, tail, spec)?;
        if start > 0 {
            if self.mixer.begin_seek(client, h, start).is_err() {
                self.mixer.seek(client, h, start)?;
            }
        }
        self.mixer.set_pan(client, h, pan, 0.)?;
        self.voices.insert((client, channel), h);
        Ok(())
    }
    pub fn render(&mut self, delta_ms: u64) -> Result<()> {
        self.poll_preparations()?;
        if self.clients.is_empty() {
            self.output.clear();
            return Ok(());
        }
        let numerator = delta_ms
            .checked_mul(48_000)
            .and_then(|v| v.checked_add(self.remainder))
            .context("channel audio clock overflow")?;
        let count = numerator / 1000;
        ensure!(count <= 96_000, "channel audio frame exceeds two seconds");
        self.remainder = numerator % 1000;
        self.output.resize(count as usize, [0.; 2]);
        for block in self.output.chunks_mut(4096) {
            self.mixer.render(block)?;
        }
        Ok(())
    }
    pub fn take_output(&mut self) -> Vec<StereoFrame> {
        std::mem::take(&mut self.output)
    }
}
