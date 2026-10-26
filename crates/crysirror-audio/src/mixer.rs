// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Deterministic, device-free stereo mixer. All controls are client scoped;
//! decoded PCM and decoder handles are derived playback resources, never saves.
use crate::decoder::{AudioSource, Clip, StereoFrame};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Master,
    Music,
    #[default]
    Sfx,
    Voice,
    Ambient,
    Ui,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceHandle {
    pub client: u32,
    pub slot: u32,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusSettings {
    pub gain: f32,
    pub muted: bool,
}
impl Default for BusSettings {
    fn default() -> Self {
        Self {
            gain: 1.,
            muted: false,
        }
    }
}
impl BusSettings {
    fn level(self) -> f32 {
        if self.muted {
            0.
        } else {
            self.gain
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ducking {
    pub gain: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}
impl Default for Ducking {
    fn default() -> Self {
        Self {
            gain: 0.25,
            attack_ms: 30.,
            release_ms: 250.,
        }
    }
}
#[derive(Clone, Debug)]
pub struct MixerConfig {
    pub sample_rate: u32,
    pub max_voices: usize,
    pub seed: u64,
}
impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            max_voices: 64,
            seed: 1,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}
impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            forward: [0., 0., -1.],
            up: [0., 1., 0.],
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Attenuation {
    Linear,
    Inverse,
    #[default]
    InverseSquare,
    Custom {
        points: Vec<[f32; 2]>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spatial {
    pub position: [f32; 3],
    pub min_distance: f32,
    pub max_distance: f32,
    pub attenuation: Attenuation,
    pub occlusion: f32,
}
impl Default for Spatial {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            min_distance: 1.,
            max_distance: 20.,
            attenuation: Default::default(),
            occlusion: 0.,
        }
    }
}
impl Spatial {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            finite3(self.position)
                && self.min_distance.is_finite()
                && self.max_distance.is_finite()
                && self.min_distance >= 0.
                && self.max_distance > self.min_distance
                && self.max_distance <= 1e6,
            "invalid spatial audio distances"
        );
        ensure!(unit(self.occlusion), "audio occlusion must be 0..1");
        if let Attenuation::Custom { points } = &self.attenuation {
            ensure!(
                (2..=64).contains(&points.len())
                    && points[0] == [0., 1.]
                    && points.last() == Some(&[1., 0.]),
                "custom attenuation needs normalized endpoints [0,1], [1,0]"
            );
            ensure!(
                points.iter().all(|p| unit(p[0]) && unit(p[1]))
                    && points
                        .windows(2)
                        .all(|p| p[0][0] < p[1][0] && p[0][1] >= p[1][1]),
                "custom attenuation must have increasing distance and nonincreasing gain"
            );
        }
        Ok(())
    }
    pub fn gain_at(&self, distance: f32) -> f32 {
        if distance <= self.min_distance {
            return 1.;
        }
        if distance >= self.max_distance {
            return 0.;
        }
        let t = (distance - self.min_distance) / (self.max_distance - self.min_distance);
        match &self.attenuation {
            Attenuation::Linear => 1. - t,
            Attenuation::Inverse => (1. - t) / (1. + 3. * t),
            Attenuation::InverseSquare => ((1. - t) / (1. + 3. * t)).powi(2),
            Attenuation::Custom { points } => points
                .windows(2)
                .find(|p| t <= p[1][0])
                .map(|p| {
                    let a = (t - p[0][0]) / (p[1][0] - p[0][0]);
                    p[0][1] + (p[1][1] - p[0][1]) * a
                })
                .unwrap_or(0.),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopPoints {
    pub start: u64,
    pub end: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceSpec {
    pub bus: Bus,
    pub gain: f32,
    pub rate: f32,
    pub pitch_semitones: f32,
    pub priority: i32,
    pub looping: bool,
    pub loop_points: Option<LoopPoints>,
    pub spatial: Option<Spatial>,
    pub fade_in_ms: f32,
    pub paused: bool,
}
impl Default for VoiceSpec {
    fn default() -> Self {
        Self {
            bus: Bus::Sfx,
            gain: 1.,
            rate: 1.,
            pitch_semitones: 0.,
            priority: 0,
            looping: false,
            loop_points: None,
            spatial: None,
            fade_in_ms: 0.,
            paused: false,
        }
    }
}
impl VoiceSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.bus != Bus::Master,
            "master is a control bus, not a voice bus"
        );
        ensure!(
            self.gain.is_finite() && (0.0..=4.).contains(&self.gain),
            "voice gain must be 0..4"
        );
        ensure!(
            self.rate.is_finite()
                && (0.125..=4.).contains(&self.rate)
                && self.pitch_semitones.is_finite()
                && (-24.0..=24.).contains(&self.pitch_semitones),
            "audio rate/pitch out of range"
        );
        duration(self.fade_in_ms)?;
        if let Some(s) = &self.spatial {
            s.validate()?
        }
        if let Some(p) = self.loop_points {
            ensure!(
                self.looping && p.start < p.end,
                "loop points require looping and start < end"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Variant {
    pub clip: Clip,
    pub weight: u32,
}
#[derive(Clone, Debug)]
pub struct RandomContainer {
    pub variants: Vec<Variant>,
    pub gain: [f32; 2],
    pub pitch_semitones: [f32; 2],
    pub avoid_repeat: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct RandomSelection {
    pub index: usize,
    pub gain: f32,
    pub pitch_semitones: f32,
}
impl RandomContainer {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.variants.is_empty()
                && self.variants.len() <= 256
                && self.variants.iter().all(|v| v.weight > 0),
            "invalid random audio variants"
        );
        ensure!(
            self.gain
                .iter()
                .all(|x| x.is_finite() && (0.0..=4.).contains(x))
                && self.gain[0] <= self.gain[1],
            "random gain range"
        );
        ensure!(
            self.pitch_semitones
                .iter()
                .all(|x| x.is_finite() && (-24.0..=24.).contains(x))
                && self.pitch_semitones[0] <= self.pitch_semitones[1],
            "random pitch range"
        );
        Ok(())
    }
}

struct Cursor {
    source: Box<dyn AudioSource>,
    buffer: Vec<StereoFrame>,
    start: u64,
    count: usize,
    next: u64,
}
impl Cursor {
    fn new(source: Box<dyn AudioSource>) -> Self {
        Self {
            source,
            buffer: vec![[0.; 2]; 1024],
            start: 0,
            count: 0,
            next: 0,
        }
    }
    fn sample(&mut self, index: u64) -> Result<Option<StereoFrame>> {
        if self
            .source
            .frame_count()
            .is_some_and(|total| index >= total)
        {
            return Ok(None);
        }
        if index >= self.start && index < self.start + self.count as u64 {
            return Ok(Some(self.buffer[(index - self.start) as usize]));
        }
        if index != self.next {
            self.source.seek(index)?
        }
        self.start = index;
        self.count = self.source.read(&mut self.buffer)?;
        self.next = index + self.count as u64;
        ensure!(
            self.count <= self.buffer.len(),
            "audio source returned invalid frame count"
        );
        Ok(if self.count > 0 {
            Some(self.buffer[0])
        } else {
            None
        })
    }
}
#[derive(Clone, Copy)]
struct Ramp {
    value: f32,
    target: f32,
    left: u64,
}
impl Ramp {
    fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            left: 0,
        }
    }
    fn to(&mut self, target: f32, frames: u64) {
        self.target = target;
        self.left = frames;
        if frames == 0 {
            self.value = target
        }
    }
    fn step(&mut self) -> f32 {
        let out = self.value;
        if self.left > 0 {
            self.value += (self.target - self.value) / self.left as f32;
            self.left -= 1;
        }
        out
    }
}
struct Voice {
    handle: VoiceHandle,
    spec: VoiceSpec,
    cursor: Cursor,
    position: f64,
    sequence: u64,
    envelope: Ramp,
    gain: Ramp,
    pan: Ramp,
    stop_after_fade: bool,
    pause_after_fade: bool,
    lowpass: StereoFrame,
    acoustic: Option<crysirror_acoustics::spectrum::BandFilter>,
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct MixStats {
    pub active: usize,
    pub stolen: u64,
    pub rejected: u64,
    pub rendered_frames: u64,
    pub peak: f32,
}
struct Client {
    voice_limit: Option<usize>,
    buses: BTreeMap<Bus, BusSettings>,
    ducking: Ducking,
    duck: Ramp,
    duck_output: f32,
    dialogue_voices: usize,
    random: u64,
    previous: BTreeMap<String, usize>,
}
impl Client {
    fn new(seed: u64) -> Self {
        Self {
            voice_limit: None,
            buses: BTreeMap::new(),
            ducking: Default::default(),
            duck: Ramp::new(1.),
            duck_output: 1.,
            dialogue_voices: 0,
            random: seed.max(1),
            previous: BTreeMap::new(),
        }
    }
    fn level(&self, bus: Bus) -> f32 {
        self.buses.get(&bus).copied().unwrap_or_default().level()
    }
}
pub struct Mixer {
    config: MixerConfig,
    voices: Vec<Option<Voice>>,
    generations: Vec<u64>,
    clients: BTreeMap<u32, Client>,
    listeners: BTreeMap<u32, Listener>,
    sequence: u64,
    stats: MixStats,
}
impl Default for Mixer {
    fn default() -> Self {
        Self::new(MixerConfig::default()).expect("valid audio defaults")
    }
}
impl Mixer {
    pub fn new(config: MixerConfig) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&config.sample_rate)
                && (1..=1024).contains(&config.max_voices),
            "audio mixer rate/voice limit"
        );
        let count = config.max_voices;
        Ok(Self {
            config,
            voices: (0..count).map(|_| None).collect(),
            generations: vec![0; count],
            clients: BTreeMap::new(),
            listeners: BTreeMap::new(),
            sequence: 0,
            stats: Default::default(),
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    pub fn stats(&self) -> MixStats {
        MixStats {
            active: self.voices.iter().filter(|v| v.is_some()).count(),
            ..self.stats
        }
    }
    fn client(&mut self, id: u32) -> Result<&mut Client> {
        ensure!(id > 0, "audio client must be nonzero");
        ensure!(
            self.clients.contains_key(&id) || self.clients.len() < 64,
            "audio mixer client limit"
        );
        let seed = self.config.seed ^ (id as u64).wrapping_mul(0x9e3779b97f4a7c15);
        Ok(self.clients.entry(id).or_insert_with(|| Client::new(seed)))
    }
    pub fn set_client_limit(&mut self, client: u32, limit: usize) -> Result<()> {
        ensure!(
            (1..=self.config.max_voices).contains(&limit),
            "client voice limit outside host pool"
        );
        ensure!(
            self.voices
                .iter()
                .flatten()
                .filter(|v| v.handle.client == client)
                .count()
                <= limit,
            "stop active voices before lowering their limit"
        );
        self.client(client)?.voice_limit = Some(limit);
        Ok(())
    }
    pub fn set_seed(&mut self, client: u32, seed: u64) -> Result<()> {
        let c = self.client(client)?;
        c.random = seed.max(1);
        c.previous.clear();
        Ok(())
    }
    pub fn set_bus(&mut self, client: u32, bus: Bus, settings: BusSettings) -> Result<()> {
        ensure!(
            settings.gain.is_finite() && (0.0..=4.).contains(&settings.gain),
            "bus gain must be 0..4"
        );
        self.client(client)?.buses.insert(bus, settings);
        Ok(())
    }
    pub fn bus(&self, client: u32, bus: Bus) -> BusSettings {
        self.clients
            .get(&client)
            .and_then(|c| c.buses.get(&bus))
            .copied()
            .unwrap_or_default()
    }
    pub fn set_ducking(&mut self, client: u32, config: Ducking) -> Result<()> {
        ensure!(unit(config.gain), "duck gain must be 0..1");
        duration(config.attack_ms)?;
        duration(config.release_ms)?;
        self.client(client)?.ducking = config;
        Ok(())
    }
    pub fn set_listener(&mut self, client: u32, listener: Listener) -> Result<()> {
        ensure!(
            finite3(listener.position)
                && finite3(listener.forward)
                && finite3(listener.up)
                && len(listener.forward) > 1e-5
                && len(cross(listener.forward, listener.up)) > 1e-5,
            "invalid audio listener basis"
        );
        self.client(client)?;
        self.listeners.insert(client, listener);
        Ok(())
    }
    pub fn play(
        &mut self,
        client: u32,
        source: Box<dyn AudioSource>,
        spec: VoiceSpec,
    ) -> Result<VoiceHandle> {
        spec.validate()?;
        ensure!(
            (8000..=192000).contains(&source.sample_rate()),
            "audio source sample rate"
        );
        if let Some(p) = spec.loop_points {
            if let Some(total) = source.frame_count() {
                ensure!(p.end <= total, "loop endpoint outside source")
            }
        }
        self.client(client)?;
        let client_full = self.clients[&client].voice_limit.is_some_and(|limit| {
            self.voices
                .iter()
                .flatten()
                .filter(|v| v.handle.client == client)
                .count()
                >= limit
        });
        let slot = if let Some(i) = self
            .voices
            .iter()
            .position(Option::is_none)
            .filter(|_| !client_full)
        {
            i
        } else {
            let (i, v) = self
                .voices
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.as_ref().map(|v| (i, v)))
                .filter(|(_, v)| !client_full || v.handle.client == client)
                .min_by_key(|(_, v)| (v.spec.priority, v.sequence))
                .context("audio pool")?;
            if spec.priority < v.spec.priority {
                self.stats.rejected += 1;
                bail!("audio voice pool full: priority rejected")
            }
            self.stats.stolen += 1;
            i
        };
        let generation = self.generations[slot]
            .checked_add(1)
            .context("audio handle generation exhausted")?;
        let sequence = self
            .sequence
            .checked_add(1)
            .context("audio sequence exhausted")?;
        let handle = VoiceHandle {
            client,
            slot: slot as u32,
            generation,
        };
        let mut envelope = Ramp::new(if spec.fade_in_ms > 0. { 0. } else { 1. });
        envelope.to(1., frames(spec.fade_in_ms, self.config.sample_rate));
        self.voices[slot] = Some(Voice {
            handle,
            gain: Ramp::new(spec.gain),
            pan: Ramp::new(0.5),
            spec,
            cursor: Cursor::new(source),
            position: 0.,
            sequence,
            envelope,
            stop_after_fade: false,
            pause_after_fade: false,
            lowpass: [0.; 2],
            acoustic: None,
        });
        self.generations[slot] = generation;
        self.sequence = sequence;
        Ok(handle)
    }
    fn voice(&mut self, client: u32, h: VoiceHandle) -> Result<&mut Voice> {
        ensure!(client == h.client, "audio handle belongs to another client");
        let v = self
            .voices
            .get_mut(h.slot as usize)
            .and_then(Option::as_mut)
            .context("audio voice ended or was stolen")?;
        ensure!(v.handle == h, "stale audio voice handle");
        Ok(v)
    }
    pub fn is_playing(&self, client: u32, h: VoiceHandle) -> bool {
        h.client == client
            && self
                .voices
                .get(h.slot as usize)
                .and_then(Option::as_ref)
                .is_some_and(|v| v.handle == h)
    }
    pub fn stop(&mut self, client: u32, h: VoiceHandle, fade_ms: f32) -> Result<()> {
        duration(fade_ms)?;
        let n = frames(fade_ms, self.config.sample_rate);
        let v = self.voice(client, h)?;
        if n == 0 {
            self.voices[h.slot as usize] = None
        } else {
            v.envelope.to(0., n);
            v.stop_after_fade = true
        }
        Ok(())
    }
    pub fn stop_client(&mut self, client: u32) {
        for v in &mut self.voices {
            if v.as_ref().is_some_and(|v| v.handle.client == client) {
                *v = None
            }
        }
    }
    pub fn clear(&mut self) {
        for v in &mut self.voices {
            *v = None
        }
    }
    pub fn set_gain(&mut self, client: u32, h: VoiceHandle, gain: f32) -> Result<()> {
        ensure!(
            gain.is_finite() && (0.0..=4.).contains(&gain),
            "voice gain must be 0..4"
        );
        let voice = self.voice(client, h)?;
        voice.spec.gain = gain;
        voice.gain.to(gain, 0);
        Ok(())
    }
    /// Linear gain ramp in output samples; native timed controls and the
    /// public Audio API use the same device-free implementation.
    pub fn fade_gain(
        &mut self,
        client: u32,
        h: VoiceHandle,
        gain: f32,
        duration_ms: f32,
    ) -> Result<()> {
        ensure!(
            gain.is_finite() && (0.0..=4.).contains(&gain),
            "voice gain must be 0..4"
        );
        duration(duration_ms)?;
        let count = frames(duration_ms, self.config.sample_rate);
        let voice = self.voice(client, h)?;
        voice.spec.gain = gain;
        voice.gain.to(gain, count);
        Ok(())
    }
    /// Stereo balance: 0 left, 0.5 unchanged, 1 right. The equal-power law
    /// preserves stereo channels rather than converting the source to mono.
    pub fn set_pan(
        &mut self,
        client: u32,
        h: VoiceHandle,
        pan: f32,
        duration_ms: f32,
    ) -> Result<()> {
        ensure!(unit(pan), "voice pan must be 0..1");
        duration(duration_ms)?;
        let count = frames(duration_ms, self.config.sample_rate);
        self.voice(client, h)?.pan.to(pan, count);
        Ok(())
    }
    /// Keep the decoder cursor, fractional source position and gain. None
    /// preserves loop points; Some updates their start without a seek.
    /// Disabling looping finishes the current source pass.
    pub fn set_looped(
        &mut self,
        client: u32,
        h: VoiceHandle,
        looping: bool,
        loop_start: Option<u64>,
    ) -> Result<()> {
        let voice = self.voice(client, h)?;
        ensure!(
            !looping || voice.cursor.source.frame_count() != Some(0),
            "cannot loop empty audio"
        );
        if let Some(start) = loop_start {
            let end = voice
                .spec
                .loop_points
                .map(|p| p.end)
                .or_else(|| voice.cursor.source.frame_count())
                .context("loop point update needs known source length")?;
            ensure!(start < end, "loop start outside source range");
            voice.spec.loop_points = Some(LoopPoints { start, end });
        }
        voice.spec.looping = looping;
        Ok(())
    }
    pub fn set_rate(&mut self, client: u32, h: VoiceHandle, rate: f32, pitch: f32) -> Result<()> {
        ensure!(
            rate.is_finite()
                && (0.125..=4.).contains(&rate)
                && pitch.is_finite()
                && (-24.0..=24.).contains(&pitch),
            "voice rate/pitch out of range"
        );
        let v = self.voice(client, h)?;
        v.spec.rate = rate;
        v.spec.pitch_semitones = pitch;
        Ok(())
    }
    pub fn set_spatial(
        &mut self,
        client: u32,
        h: VoiceHandle,
        spatial: Option<Spatial>,
    ) -> Result<()> {
        if let Some(s) = &spatial {
            s.validate()?
        }
        self.voice(client, h)?.spec.spatial = spatial;
        Ok(())
    }
    /// Apply prepared direct-path frequency gains. No geometry queries or I/O
    /// run in the mixer. Existing distance, pan, buses and envelopes still apply.
    pub fn set_acoustic_gains(
        &mut self,
        client: u32,
        h: VoiceHandle,
        gains: Option<[f32; 3]>,
    ) -> Result<()> {
        let rate = self.config.sample_rate;
        let voice = self.voice(client, h)?;
        if let Some(gains) = gains {
            ensure!(
                voice.spec.spatial.is_some(),
                "acoustic gains require a spatial voice"
            );
            if let Some(filter) = &mut voice.acoustic {
                filter.set_gains(gains)?;
            } else {
                voice.acoustic = Some(crysirror_acoustics::spectrum::BandFilter::with_gains(
                    rate, gains,
                )?);
            }
        } else {
            voice.acoustic = None;
        }
        Ok(())
    }
    pub fn pause(&mut self, client: u32, h: VoiceHandle, paused: bool) -> Result<()> {
        self.voice(client, h)?.spec.paused = paused;
        Ok(())
    }
    pub fn pause_fade(
        &mut self,
        client: u32,
        h: VoiceHandle,
        paused: bool,
        duration_ms: f32,
    ) -> Result<()> {
        duration(duration_ms)?;
        let count = frames(duration_ms, self.config.sample_rate);
        let voice = self.voice(client, h)?;
        if paused && count == 0 {
            voice.spec.paused = true;
            voice.pause_after_fade = false;
        } else if paused {
            voice.pause_after_fade = true;
            voice.envelope.to(0., count);
        } else {
            voice.spec.paused = false;
            voice.pause_after_fade = false;
            voice.envelope.to(1., count);
        }
        Ok(())
    }
    pub fn pause_client(&mut self, client: u32, paused: bool) {
        for v in self.voices.iter_mut().flatten() {
            if v.handle.client == client {
                v.spec.paused = paused
            }
        }
    }
    pub fn seek(&mut self, client: u32, h: VoiceHandle, source_frame: u64) -> Result<()> {
        let v = self.voice(client, h)?;
        v.cursor.source.seek(source_frame)?;
        v.cursor.count = 0;
        v.cursor.next = source_frame;
        v.position = source_frame as f64;
        v.lowpass = [0.; 2];
        Ok(())
    }
    pub fn position(&self, client: u32, h: VoiceHandle) -> Result<f64> {
        ensure!(client == h.client, "audio handle belongs to another client");
        let v = self
            .voices
            .get(h.slot as usize)
            .and_then(Option::as_ref)
            .context("audio voice ended")?;
        ensure!(v.handle == h, "stale audio voice handle");
        Ok(v.position / v.cursor.source.sample_rate() as f64)
    }
    pub fn spec(&self, client: u32, h: VoiceHandle) -> Result<VoiceSpec> {
        ensure!(client == h.client, "audio handle belongs to another client");
        let v = self
            .voices
            .get(h.slot as usize)
            .and_then(Option::as_ref)
            .context("audio voice ended")?;
        ensure!(v.handle == h, "stale audio voice handle");
        Ok(v.spec.clone())
    }
    pub fn play_random(
        &mut self,
        client: u32,
        name: &str,
        container: &RandomContainer,
        mut spec: VoiceSpec,
    ) -> Result<(VoiceHandle, RandomSelection)> {
        container.validate()?;
        ensure!(
            !name.is_empty() && name.len() <= 256,
            "random container name"
        );
        let c = self.client(client)?;
        ensure!(
            c.previous.contains_key(name) || c.previous.len() < 1024,
            "random audio container limit"
        );
        let previous = if container.avoid_repeat && container.variants.len() > 1 {
            c.previous.get(name).copied()
        } else {
            None
        };
        let total: u64 = container
            .variants
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != previous)
            .map(|(_, v)| v.weight as u64)
            .sum();
        let mut pick = random(&mut c.random) % total;
        let mut index = 0;
        for (i, v) in container.variants.iter().enumerate() {
            if Some(i) == previous {
                continue;
            }
            if pick < v.weight as u64 {
                index = i;
                break;
            }
            pick -= v.weight as u64;
        }
        let gain = between(&mut c.random, container.gain);
        let pitch = between(&mut c.random, container.pitch_semitones);
        c.previous.insert(name.into(), index);
        spec.gain *= gain;
        spec.pitch_semitones += pitch;
        spec.validate()?;
        let h = self.play(client, container.variants[index].clip.source(), spec)?;
        Ok((
            h,
            RandomSelection {
                index,
                gain,
                pitch_semitones: pitch,
            },
        ))
    }
    /// Overwrites the destination with unclipped linear stereo PCM. One output
    /// limiter/clamp belongs at the hardware/export boundary, not per voice.
    pub fn render(&mut self, out: &mut [StereoFrame]) -> Result<()> {
        ensure!(
            out.len() <= self.config.sample_rate as usize * 10,
            "audio render block exceeds 10 seconds"
        );
        out.fill([0.; 2]);
        let rate = self.config.sample_rate;
        for c in self.clients.values_mut() {
            c.dialogue_voices = 0;
        }
        for v in self.voices.iter().flatten() {
            let c = &mut self
                .clients
                .get_mut(&v.handle.client)
                .expect("registered voice client");
            if v.spec.bus == Bus::Voice
                && !v.spec.paused
                && v.spec.gain > 0.
                && c.level(Bus::Voice) > 0.
            {
                c.dialogue_voices += 1;
            }
        }
        let spatial: Vec<_> = self
            .voices
            .iter()
            .map(|v| {
                v.as_ref().and_then(|v| {
                    v.spec.spatial.as_ref().map(|s| {
                        spatial_mix(
                            s,
                            self.listeners
                                .get(&v.handle.client)
                                .copied()
                                .unwrap_or_default(),
                            rate,
                        )
                    })
                })
            })
            .collect();
        let rates: Vec<_> = self
            .voices
            .iter()
            .map(|v| {
                v.as_ref().map_or(0., |v| {
                    v.cursor.source.sample_rate() as f64 / rate as f64
                        * v.spec.rate as f64
                        * 2f64.powf(v.spec.pitch_semitones as f64 / 12.)
                })
            })
            .collect();
        for sample in out.iter_mut() {
            for c in self.clients.values_mut() {
                let dialogue = c.dialogue_voices > 0;
                let target = if dialogue { c.ducking.gain } else { 1. };
                if c.duck.target != target {
                    c.duck.to(
                        target,
                        frames(
                            if dialogue {
                                c.ducking.attack_ms
                            } else {
                                c.ducking.release_ms
                            },
                            rate,
                        ),
                    );
                }
                c.duck_output = c.duck.step();
            }
            for (slot, (effect, step)) in self.voices.iter_mut().zip(spatial.iter().zip(&rates)) {
                let Some(v) = slot else { continue };
                if v.spec.paused {
                    continue;
                }
                let loop_range = if v.spec.looping {
                    v.spec.loop_points.or_else(|| {
                        v.cursor
                            .source
                            .frame_count()
                            .map(|end| LoopPoints { start: 0, end })
                    })
                } else {
                    None
                };
                if let Some(p) = loop_range {
                    ensure!(p.start < p.end, "cannot loop empty audio");
                    if v.position >= p.end as f64 {
                        v.position =
                            p.start as f64 + (v.position - p.end as f64) % (p.end - p.start) as f64
                    }
                }
                let mut index = v.position.floor() as u64;
                let a = match v.cursor.sample(index)? {
                    Some(a) => a,
                    None if v.spec.looping => {
                        v.position = 0.;
                        index = 0;
                        if let Some(a) = v.cursor.sample(0)? {
                            a
                        } else {
                            retire_dialogue(v, &mut self.clients);
                            *slot = None;
                            continue;
                        }
                    }
                    None => {
                        retire_dialogue(v, &mut self.clients);
                        *slot = None;
                        continue;
                    }
                };
                let next = if let Some(p) = loop_range {
                    if index + 1 >= p.end {
                        p.start
                    } else {
                        index + 1
                    }
                } else {
                    index + 1
                };
                let b = if next == index {
                    a
                } else {
                    v.cursor.sample(next)?.unwrap_or(a)
                };
                let t = (v.position - index as f64) as f32;
                let mut s = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let c = &self.clients[&v.handle.client];
                let mut gain =
                    v.gain.step() * v.envelope.step() * c.level(Bus::Master) * c.level(v.spec.bus);
                let pan = v.pan.step();
                if pan != 0.5 {
                    s[0] *= (1. - pan).sqrt() * std::f32::consts::SQRT_2;
                    s[1] *= pan.sqrt() * std::f32::consts::SQRT_2;
                }
                if v.spec.bus == Bus::Music {
                    gain *= c.duck_output
                }
                if let Some(effect) = effect {
                    gain *= effect.gain;
                    let mono = (s[0] + s[1]) * 0.5;
                    let mono = v
                        .acoustic
                        .as_mut()
                        .map_or(mono, |filter| filter.process(mono));
                    s = [mono * effect.pan[0], mono * effect.pan[1]];
                    if let Some(alpha) = effect.alpha {
                        for (state, input) in v.lowpass.iter_mut().zip(s) {
                            *state += alpha * (input - *state);
                        }
                        s = v.lowpass;
                    } else {
                        v.lowpass = s
                    }
                }
                sample[0] += s[0] * gain;
                sample[1] += s[1] * gain;
                v.position += step;
                if v.pause_after_fade && v.envelope.left == 0 {
                    v.spec.paused = true;
                    v.pause_after_fade = false;
                }
                if (v.stop_after_fade && v.envelope.left == 0 && v.envelope.value == 0.)
                    || (!v.spec.looping
                        && v.cursor
                            .source
                            .frame_count()
                            .is_some_and(|n| v.position >= n as f64))
                {
                    retire_dialogue(v, &mut self.clients);
                    *slot = None
                }
            }
            ensure!(
                sample.iter().all(|x| x.is_finite()),
                "non-finite mixed audio"
            );
            self.stats.peak = self.stats.peak.max(sample[0].abs()).max(sample[1].abs());
        }
        self.stats.rendered_frames = self.stats.rendered_frames.saturating_add(out.len() as u64);
        Ok(())
    }
}
struct SpatialMix {
    gain: f32,
    pan: [f32; 2],
    alpha: Option<f32>,
}
fn spatial_mix(spatial: &Spatial, listener: Listener, rate: u32) -> SpatialMix {
    let offset = sub(spatial.position, listener.position);
    let distance = len(offset);
    let right = normalized(cross(listener.forward, listener.up));
    let pan = if distance > 1e-6 {
        dot(offset, right) / distance
    } else {
        0.
    }
    .clamp(-1., 1.);
    let alpha = if spatial.occlusion > 0. {
        let cutoff =
            (20000.0f32 * (800.0f32 / 20000.).powf(spatial.occlusion)).min(rate as f32 * 0.45);
        Some(1. - (-std::f32::consts::TAU * cutoff / rate as f32).exp())
    } else {
        None
    };
    SpatialMix {
        gain: spatial.gain_at(distance) * (1. - 0.7 * spatial.occlusion),
        pan: [((1. - pan) * 0.5).sqrt(), ((1. + pan) * 0.5).sqrt()],
        alpha,
    }
}
fn retire_dialogue(v: &Voice, clients: &mut BTreeMap<u32, Client>) {
    let c = clients
        .get_mut(&v.handle.client)
        .expect("registered voice client");
    if v.spec.bus == Bus::Voice && !v.spec.paused && v.spec.gain > 0. && c.level(Bus::Voice) > 0. {
        c.dialogue_voices = c.dialogue_voices.saturating_sub(1);
    }
}

fn random(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}
fn between(state: &mut u64, range: [f32; 2]) -> f32 {
    let f = (random(state) >> 40) as f32 / 16777216.;
    range[0] + (range[1] - range[0]) * f
}
fn duration(ms: f32) -> Result<()> {
    ensure!(
        ms.is_finite() && (0.0..=600000.).contains(&ms),
        "audio fade duration must be 0..600000 ms"
    );
    Ok(())
}
fn frames(ms: f32, rate: u32) -> u64 {
    (ms as f64 * rate as f64 / 1000.).round() as u64
}
fn unit(x: f32) -> bool {
    x.is_finite() && (0.0..=1.).contains(&x)
}
fn finite3(v: [f32; 3]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= 1e6)
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}
fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = len(v);
    [v[0] / l, v[1] / l, v[2] / l]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
