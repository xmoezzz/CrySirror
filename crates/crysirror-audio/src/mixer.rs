// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Deterministic, device-free stereo mixer. All controls are client scoped;
//! decoded PCM and decoder handles are derived playback resources, never saves.
use crate::decoder::{AudioSource, Clip, SourceBufferStatus, SourceRead, StereoFrame};
use crate::doppler::{DopplerInfo, DopplerSettings};
use crate::envelope::Ramp;
use crate::mix_groups::MixGroups;
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::{
    AudioMixGroupInfo, AudioMixGroupSettings, AudioResumePolicy, AudioResumeSettings,
};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
        if self.muted { 0. } else { self.gain }
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
/// World-space listener pose and optional medium-relative velocity.
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    /// Optional explicit world metres/second; standalone Mixer treats None as zero.
    pub velocity: Option<[f32; 3]>,
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}
impl Default for Listener {
    fn default() -> Self {
        Self {
            velocity: None,
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
/// Mono point-source position, distance gain and opt-in direct Doppler.
/// App can infer missing velocities from world poses; standalone Mixer cannot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spatial {
    /// Optional explicit world m/s. App infers absent velocities from poses;
    /// standalone Mixer uses zero.
    pub velocity: Option<[f32; 3]>,
    /// Opt-in direct-path Doppler; None retains existing source speed.
    pub doppler: Option<DopplerSettings>,
    /// World position in metres; an attachment uses this as a world-axis offset.
    pub position: [f32; 3],
    /// Distance below which gain is one, default 1 metre.
    pub min_distance: f32,
    /// Zero-gain distance, greater than min_distance, default 20 metres.
    pub max_distance: f32,
    /// Normalized distance-to-gain curve.
    pub attenuation: Attenuation,
    /// Manual obstruction amount 0..1, independent of geometric acoustics.
    pub occlusion: f32,
}
impl Default for Spatial {
    fn default() -> Self {
        Self {
            velocity: None,
            doppler: None,
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
            self.velocity.is_none_or(finite3),
            "invalid spatial audio velocity"
        );
        if let Some(d) = self.doppler {
            d.validate()?;
        }
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
/// Shared mixer playback parameters. Public Audio calls additionally accept an
/// asset/resource and optional entity/effect declarations outside this record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceSpec {
    /// An existing client-scoped logical gain group; None bypasses groups.
    pub mix_group: Option<String>,
    /// Source bus; Master is a control bus and is rejected.
    pub bus: Bus,
    /// Per-voice gain 0..4, default 1.
    pub gain: f32,
    /// Authored playback speed 0.125..4, default 1; separate from Doppler.
    pub rate: f32,
    /// Authored semitone shift -24..24, default 0.
    pub pitch_semitones: f32,
    /// Pool admission/stealing priority; default 0.
    pub priority: i32,
    /// Repeat source input; default false.
    pub looping: bool,
    /// Optional decoded-source frame range for looping.
    pub loop_points: Option<LoopPoints>,
    /// Optional point-source spatial parameters; absent retains stereo input.
    pub spatial: Option<Spatial>,
    /// Initial transport fade in output milliseconds; default 0.
    pub fade_in_ms: f32,
    /// Publish with frozen source input; default false.
    pub paused: bool,
}
impl Default for VoiceSpec {
    fn default() -> Self {
        Self {
            mix_group: None,
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
        if let Some(name) = &self.mix_group {
            crate::mix_groups::name(name)?;
        }
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

enum SampleRead {
    Frame(StereoFrame),
    Pending,
    End,
}
struct Cursor {
    source: Box<dyn AudioSource>,
    buffer: Vec<StereoFrame>,
    start: u64,
    count: usize,
    next: u64,
    edge: Option<(u64, StereoFrame)>,
}
impl Cursor {
    fn new(source: Box<dyn AudioSource>) -> Self {
        Self {
            source,
            buffer: vec![[0.; 2]; 1024],
            start: 0,
            count: 0,
            next: 0,
            edge: None,
        }
    }
    fn sample(&mut self, index: u64) -> Result<SampleRead> {
        if self.source.frame_count().is_some_and(|n| index >= n) {
            return Ok(SampleRead::End);
        }
        if index >= self.start && index < self.start + self.count as u64 {
            return Ok(SampleRead::Frame(
                self.buffer[(index - self.start) as usize],
            ));
        }
        // Linear interpolation can request the last frame of the preceding
        // chunk again after fetching its successor. Retain that single frame
        // rather than seeking a worker decoder backwards at every boundary.
        if let Some((at, sample)) = self.edge {
            if at == index {
                return Ok(SampleRead::Frame(sample));
            }
        }
        let edge = (self.count > 0).then(|| {
            (
                self.start + self.count as u64 - 1,
                self.buffer[self.count - 1],
            )
        });
        if (index != self.next || self.source.read_position().is_some_and(|n| n != index))
            && !self.source.poll_seek(index)?
        {
            return Ok(SampleRead::Pending);
        }
        match self.source.poll_read(&mut self.buffer)? {
            SourceRead::Pending => Ok(SampleRead::Pending),
            SourceRead::End => {
                self.start = index;
                self.count = 0;
                self.next = index;
                Ok(SampleRead::End)
            }
            SourceRead::Frames(n) => {
                ensure!(
                    n > 0 && n <= self.buffer.len(),
                    "audio source returned invalid frame count"
                );
                self.edge = edge;
                self.start = index;
                self.count = n;
                self.next = index + n as u64;
                Ok(SampleRead::Frame(self.buffer[0]))
            }
        }
    }
}

struct Voice {
    doppler: Ramp,
    doppler_settings: Option<DopplerSettings>,
    age_frames: u64,
    transport_frames: u64,
    mix_group: Option<usize>,
    handle: VoiceHandle,
    spec: VoiceSpec,
    cursor: Cursor,
    position: f64,
    seek: Option<SeekProgress>,
    seek_sequence: u64,
    sequence: u64,
    envelope: Ramp,
    gain: Ramp,
    pan: Ramp,
    stop_after_fade: bool,
    stop_remaining: u64,
    pause_after_fade: bool,
    resume_after: Option<ScheduledResume>,
    lowpass: StereoFrame,
    acoustic: Option<crysirror_acoustics::spectrum::BandFilter>,
    binaural: Option<crysirror_acoustics::binaural::BinauralVoice>,
    binaural_pending: bool,
    reflections: Option<crysirror_acoustics::reflection_dsp::EarlyReflections>,
    reflection_pending: bool,
    source_ended: bool,
    tail_left: usize,
    tail_elapsed: usize,
}
struct ScheduledResume {
    left: u64,
    fade: u64,
    settings: AudioResumeSettings,
}
fn resume_now(v: &mut Voice, fade: u64, settings: AudioResumeSettings) {
    v.stop_after_fade = false;
    v.stop_remaining = 0;
    v.pause_after_fade = false;
    v.spec.paused = false;
    v.resume_after = None;
    if settings.reset_gain {
        v.gain = Ramp::new(if fade == 0 { v.spec.gain } else { 0. });
        v.gain.to(v.spec.gain, fade);
        v.envelope = Ramp::new(1.);
    } else {
        v.envelope = Ramp::new(if fade == 0 { 1. } else { 0. });
        v.envelope.to(1., fade);
    }
}
/// A seek publishes only once the exact decoded frame is available.
#[derive(Clone, Debug, Serialize)]
pub struct SeekProgress {
    /// Monotonic voice-local request sequence; zero denotes cancellation recovery.
    pub sequence: u64,
    /// Exact decoded source frame requested.
    pub frame: u64,
    /// The voice clock stays frozen until ready PCM is published.
    pub pending: bool,
    /// Retained worker/seek failure, when present.
    pub error: Option<String>,
}
// Transport deadlines use the output clock even when source PCM/preparation is
// unavailable. Ordinary fade-in and source position remain frozen on starvation.
fn silent_transport_tick(v: &mut Voice) {
    if !v.stop_after_fade && !v.pause_after_fade {
        return;
    }
    v.envelope.step();
    if v.pause_after_fade && v.envelope.left == 0 {
        v.spec.paused = true;
        v.pause_after_fade = false;
    }
}
fn constrain_tail(v: &mut Voice) {
    if v.source_ended {
        let total = v
            .binaural
            .as_ref()
            .map_or(0, |b| b.tail_frames())
            .max(v.reflections.as_ref().map_or(0, |r| r.tail_frames()));
        v.tail_left = v.tail_left.min(total.saturating_sub(v.tail_elapsed));
    }
}
fn publish_seek(v: &mut Voice, frame: u64) {
    v.cursor.count = 0;
    v.cursor.edge = None;
    v.cursor.next = frame;
    v.position = frame as f64;
    v.lowpass = [0.; 2];
    if let Some(filter) = &mut v.acoustic {
        filter.reset_history();
    }
    if let Some(effect) = &mut v.reflections {
        effect.reset();
    }
    v.source_ended = false;
    v.tail_left = 0;
    v.tail_elapsed = 0;
    if let Some(voice) = &mut v.binaural {
        voice.reset();
    }
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct MixStats {
    pub active: usize,
    pub stolen: u64,
    pub rejected: u64,
    pub rendered_frames: u64,
    /// Output frames a voice could not consume because source PCM was pending.
    pub underrun_frames: u64,
    pub peak: f32,
}
struct Client {
    reverb: Option<crate::reverb::PreparedReverb>,
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
            reverb: None,
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
struct VoiceObservation {
    doppler: DopplerInfo,
    spec: VoiceSpec,
    info: crysirror_core::service::AudioPlaybackInfo,
    source: Option<SourceBufferStatus>,
    position: f64,
}
struct PublishedMix {
    voices: BTreeMap<VoiceHandle, VoiceObservation>,
    reverbs: BTreeMap<u32, crate::reverb::ReverbSettings>,
    stats: MixStats,
}
pub struct Mixer {
    lease: Option<u64>,
    published: Option<PublishedMix>,
    deferred_stop: std::collections::BTreeSet<u32>,
    deferred_pause: BTreeMap<u32, bool>,
    groups: MixGroups,
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
            lease: None,
            published: None,
            deferred_stop: Default::default(),
            deferred_pause: Default::default(),
            groups: MixGroups::default(),
            config,
            voices: (0..count).map(|_| None).collect(),
            generations: vec![0; count],
            clients: BTreeMap::new(),
            listeners: BTreeMap::new(),
            sequence: 0,
            stats: Default::default(),
        })
    }
    /// DSP state has one worker owner until its frame is collected. Read-only
    /// observations remain at the last owner-published boundary.
    pub fn is_rendering(&self) -> bool {
        self.lease.is_some()
    }
    pub(crate) fn idle(&self) -> Result<()> {
        ensure!(
            !self.is_rendering(),
            "audio render pending; collect its frame before control mutation"
        );
        Ok(())
    }
    pub(crate) fn lease_id(&self) -> Option<u64> {
        self.lease
    }
    pub(crate) fn lease_render(&mut self, id: u64) -> Result<Mixer> {
        self.idle()?;
        let mut observations = BTreeMap::new();
        for v in self.voices.iter().flatten() {
            let h = v.handle;
            observations.insert(
                h,
                VoiceObservation {
                    doppler: self.doppler_info(h.client, h)?,
                    spec: v.spec.clone(),
                    info: self.playback_info(h.client, h)?,
                    source: v.cursor.source.buffer_status(),
                    position: self.position(h.client, h)?,
                },
            );
        }
        let mut placeholder = Mixer::new(self.config.clone())?;
        placeholder.generations = self.generations.clone();
        placeholder.sequence = self.sequence;
        placeholder.groups = self.groups.clone();
        placeholder.listeners = self.listeners.clone();
        let mut reverbs = BTreeMap::new();
        for (&owner, c) in &self.clients {
            let mut clone = Client::new(c.random);
            clone.voice_limit = c.voice_limit;
            clone.buses = c.buses.clone();
            clone.ducking = c.ducking;
            clone.duck = c.duck;
            clone.duck_output = c.duck_output;
            clone.previous = c.previous.clone();
            if let Some(r) = &c.reverb {
                reverbs.insert(owner, r.settings.clone());
            }
            placeholder.clients.insert(owner, clone);
        }
        placeholder.stats = self.stats;
        placeholder.lease = Some(id);
        placeholder.published = Some(PublishedMix {
            voices: observations,
            reverbs,
            stats: self.stats(),
        });
        Ok(std::mem::replace(self, placeholder))
    }
    pub(crate) fn finish_render(&mut self, id: u64, mut mixer: Mixer) -> bool {
        if self.lease != Some(id) {
            return false;
        }
        for &client in &self.deferred_stop {
            mixer.stop_client(client);
        }
        for (&client, &paused) in &self.deferred_pause {
            mixer.pause_client(client, paused);
        }
        *self = mixer;
        true
    }
    pub(crate) fn abort_render(&mut self, id: u64) {
        if self.lease == Some(id) {
            self.clear();
        }
    }
    pub(crate) fn requires_render(&self) -> bool {
        self.voices.iter().any(Option::is_some)
            || self.groups.is_fading()
            || self.clients.values().any(|c| c.reverb.is_some())
    }
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    pub fn stats(&self) -> MixStats {
        if let Some(p) = &self.published {
            return p.stats;
        }
        MixStats {
            active: self.voices.iter().filter(|v| v.is_some()).count(),
            ..self.stats
        }
    }
    fn client(&mut self, id: u32) -> Result<&mut Client> {
        self.idle()?;
        ensure!(id > 0, "audio client must be nonzero");
        ensure!(
            self.clients.contains_key(&id) || self.clients.len() < 64,
            "audio mixer client limit"
        );
        let seed = self.config.seed ^ (id as u64).wrapping_mul(0x9e3779b97f4a7c15);
        Ok(self.clients.entry(id).or_insert_with(|| Client::new(seed)))
    }
    /// Install a worker-prepared room. Old history is returned for caller-owned
    /// retirement; installation never allocates delay lines or resets DSP.
    pub fn set_reverb(
        &mut self,
        client: u32,
        effect: Option<crate::reverb::PreparedReverb>,
    ) -> Result<Option<crate::reverb::PreparedReverb>> {
        ensure!(client > 0, "audio client must be nonzero");
        if effect.is_none() && !self.clients.contains_key(&client) {
            return Ok(None);
        }
        if let Some(effect) = &effect {
            ensure!(
                effect.sample_rate() == self.sample_rate(),
                "reverb/mixer sample rates differ"
            );
        }
        Ok(std::mem::replace(&mut self.client(client)?.reverb, effect))
    }
    pub fn reverb(&self, client: u32) -> Option<&crate::reverb::ReverbSettings> {
        if let Some(p) = &self.published {
            return p.reverbs.get(&client);
        }
        self.clients
            .get(&client)?
            .reverb
            .as_ref()
            .map(|r| r.settings())
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
    /// Validate without changing state; hosts use this before publishing to
    /// multiple mixers whose output rates may differ.
    pub fn validate_mix_group(
        &self,
        client: u32,
        name: &str,
        settings: &AudioMixGroupSettings,
        fade_ms: f32,
    ) -> Result<()> {
        self.idle()?;
        duration(fade_ms)?;
        self.groups.validate(client, name, settings).map(|_| ())
    }
    pub fn set_mix_group(
        &mut self,
        client: u32,
        name: &str,
        settings: AudioMixGroupSettings,
        fade_ms: f32,
    ) -> Result<()> {
        self.idle()?;
        duration(fade_ms)?;
        self.groups.set(
            client,
            name,
            settings,
            frames(fade_ms, self.config.sample_rate),
        )
    }
    pub fn set_mix_group_gain(
        &mut self,
        client: u32,
        name: &str,
        gain: f32,
        fade_ms: f32,
    ) -> Result<()> {
        let mut settings = self
            .mix_group(client, name)?
            .map(|i| i.settings)
            .unwrap_or_default();
        settings.gain = gain;
        self.set_mix_group(client, name, settings, fade_ms)
    }
    pub fn mix_group(&self, client: u32, name: &str) -> Result<Option<AudioMixGroupInfo>> {
        self.groups.info(client, name, self.config.sample_rate)
    }
    pub fn validate_remove_mix_group(&self, client: u32, name: &str) -> Result<()> {
        self.idle()?;
        let id = self.groups.resolve(client, Some(name))?.unwrap();
        ensure!(
            !self
                .voices
                .iter()
                .flatten()
                .any(|v| v.mix_group == Some(id)),
            "mix group still has live voice/effect instances"
        );
        self.groups.validate_remove(client, name)
    }
    pub fn remove_mix_group(&mut self, client: u32, name: &str) -> Result<()> {
        self.idle()?;
        self.validate_remove_mix_group(client, name)?;
        self.groups.remove(client, name)
    }
    pub fn set_voice_mix_group(
        &mut self,
        client: u32,
        h: VoiceHandle,
        name: Option<String>,
    ) -> Result<()> {
        let group = self.groups.resolve(client, name.as_deref())?;
        let v = self.voice(client, h)?;
        v.mix_group = group;
        v.spec.mix_group = name;
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
            listener.velocity.is_none_or(finite3)
                && finite3(listener.position)
                && finite3(listener.forward)
                && finite3(listener.up)
                && len(listener.forward) > 1e-5
                && len(cross(listener.forward, listener.up)) > 1e-5,
            "invalid audio listener basis"
        );
        self.client(client)?;
        for v in self
            .voices
            .iter_mut()
            .flatten()
            .filter(|v| v.handle.client == client)
        {
            if v.reflections.as_ref().is_some_and(|r| {
                let q = &r.paths().query;
                !r.is_tracked()
                    && (q.listener != listener.position
                        || q.listener_forward != listener.forward
                        || q.listener_up != listener.up)
            }) {
                v.reflections = None;
                constrain_tail(v);
            }
        }
        self.listeners.insert(client, listener);
        Ok(())
    }
    pub fn listener(&self, client: u32) -> Listener {
        self.listeners.get(&client).copied().unwrap_or_default()
    }
    pub fn play(
        &mut self,
        client: u32,
        source: Box<dyn AudioSource>,
        spec: VoiceSpec,
    ) -> Result<VoiceHandle> {
        self.idle()?;
        spec.validate()?;
        let mix_group = self.groups.resolve(client, spec.mix_group.as_deref())?;
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
            doppler: Ramp::new(1.),
            doppler_settings: None,
            age_frames: 0,
            transport_frames: 0,
            mix_group,
            handle,
            gain: Ramp::new(spec.gain),
            pan: Ramp::new(0.5),
            spec,
            cursor: Cursor::new(source),
            position: 0.,
            seek: None,
            seek_sequence: 0,
            sequence,
            envelope,
            stop_after_fade: false,
            stop_remaining: 0,
            pause_after_fade: false,
            resume_after: None,
            lowpass: [0.; 2],
            acoustic: None,
            binaural: None,
            binaural_pending: false,
            reflections: None,
            reflection_pending: false,
            source_ended: false,
            tail_left: 0,
            tail_elapsed: 0,
        });
        self.generations[slot] = generation;
        self.sequence = sequence;
        Ok(handle)
    }
    /// Replace this exact voice without stealing another channel. Validation
    /// failure restores the previous source/effect state.
    pub fn replace(
        &mut self,
        client: u32,
        previous: VoiceHandle,
        source: Box<dyn AudioSource>,
        spec: VoiceSpec,
    ) -> Result<VoiceHandle> {
        self.idle()?;
        self.spec(client, previous)?;
        let old = self.voices[previous.slot as usize].take();
        match self.play(client, source, spec) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                self.voices[previous.slot as usize] = old;
                Err(error)
            }
        }
    }
    pub(crate) fn has_spare_voice(&self, client: u32) -> bool {
        if self.is_rendering() {
            return false;
        }
        self.voices.iter().any(Option::is_none)
            && self.clients.get(&client).is_none_or(|c| {
                c.voice_limit.is_none_or(|limit| {
                    self.voices
                        .iter()
                        .flatten()
                        .filter(|v| v.handle.client == client)
                        .count()
                        < limit
                })
            })
    }
    /// The channel worker already sought/prewarmed this source. Publishing its
    /// logical cursor must not launch a second seek or consume a sample.
    pub(crate) fn prepared_position(
        &mut self,
        client: u32,
        h: VoiceHandle,
        frame: u64,
    ) -> Result<()> {
        let voice = self.voice(client, h)?;
        voice.position = frame as f64;
        voice.cursor.start = frame;
        voice.cursor.next = frame;
        Ok(())
    }
    fn voice(&mut self, client: u32, h: VoiceHandle) -> Result<&mut Voice> {
        self.idle()?;
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
        if let Some(p) = &self.published {
            return h.client == client && p.voices.contains_key(&h);
        }
        h.client == client
            && self
                .voices
                .get(h.slot as usize)
                .and_then(Option::as_ref)
                .is_some_and(|v| v.handle == h)
    }
    /// Source completion excludes derived HRTF/echo tails. Native waits consume
    /// this state while modern is_playing retains an effect's natural tail.
    pub fn source_is_playing(&self, client: u32, h: VoiceHandle) -> bool {
        if let Some(p) = &self.published {
            return h.client == client && p.voices.get(&h).is_some_and(|v| v.info.source_playing);
        }
        h.client == client
            && self
                .voices
                .get(h.slot as usize)
                .and_then(Option::as_ref)
                .is_some_and(|v| {
                    v.handle == h
                        && !v.source_ended
                        && (v.spec.looping
                            || v.cursor
                                .source
                                .frame_count()
                                .is_none_or(|n| v.position < n as f64))
                })
    }
    pub fn stop(&mut self, client: u32, h: VoiceHandle, fade_ms: f32) -> Result<()> {
        duration(fade_ms)?;
        let n = frames(fade_ms, self.config.sample_rate);
        let v = self.voice(client, h)?;
        v.resume_after = None;
        if n == 0 {
            self.voices[h.slot as usize] = None
        } else {
            v.envelope.to(0., n);
            v.stop_after_fade = true;
            v.stop_remaining = n;
            v.pause_after_fade = false;
        }
        Ok(())
    }
    pub fn stop_client(&mut self, client: u32) {
        if self.is_rendering() {
            if self.clients.contains_key(&client) {
                self.deferred_stop.insert(client);
            }
            return;
        }
        for v in &mut self.voices {
            if v.as_ref().is_some_and(|v| v.handle.client == client) {
                *v = None
            }
        }
    }
    pub fn clear(&mut self) {
        self.lease = None;
        self.published = None;
        self.deferred_pause.clear();
        self.deferred_stop.clear();
        for c in self.clients.values_mut() {
            c.reverb = None;
        }
        for v in &mut self.voices {
            *v = None
        }
    }
    /// Route an existing voice without replacing its source or timeline.
    pub fn set_voice_bus(&mut self, client: u32, h: VoiceHandle, bus: Bus) -> Result<()> {
        ensure!(bus != Bus::Master, "master is not a voice bus");
        self.voice(client, h)?.spec.bus = bus;
        Ok(())
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
        let v = self.voice(client, h)?;
        if v.spec.spatial.as_ref().map(|s| s.position) != spatial.as_ref().map(|s| s.position)
            && v.reflections.as_ref().is_none_or(|r| !r.is_tracked())
        {
            v.reflections = None;
            constrain_tail(v);
        }
        v.spec.spatial = spatial;
        Ok(())
    }
    /// Hold an initial one-shot clock during worker preparation. Replacement of
    /// an installed effect leaves its old source clock/effect running.
    pub fn pending_reflections(&mut self, client: u32, h: VoiceHandle) -> Result<()> {
        ensure!(
            self.source_is_playing(client, h),
            "reflection source has ended"
        );
        let v = self.voice(client, h)?;
        ensure!(
            !v.binaural_pending,
            "wait for HRTF preparation before reflections"
        );
        v.reflection_pending = v.reflections.is_none();
        Ok(())
    }
    pub fn cancel_reflection_preparation(&mut self, client: u32, h: VoiceHandle) -> Result<()> {
        self.voice(client, h)?.reflection_pending = false;
        Ok(())
    }
    pub fn direct_latency_frames(&self, client: u32, h: VoiceHandle) -> Result<usize> {
        self.idle()?;
        self.spec(client, h)?;
        let v = self.voices[h.slot as usize].as_ref().unwrap();
        ensure!(
            !v.binaural_pending,
            "wait for HRTF preparation before reflections"
        );
        Ok(v.binaural.as_ref().map_or(0, |b| b.latency_frames()))
    }
    pub fn set_reflections(
        &mut self,
        client: u32,
        h: VoiceHandle,
        effect: Option<crysirror_acoustics::reflection_dsp::EarlyReflections>,
    ) -> Result<Option<crysirror_acoustics::reflection_dsp::EarlyReflections>> {
        let rate = self.sample_rate();
        let listener = self.listener(client);
        let v = self.voice(client, h)?;
        if let Some(effect) = &effect {
            let q = &effect.paths().query;
            ensure!(
                effect.sample_rate() == rate
                    && !v.binaural_pending
                    && effect.direct_latency_frames()
                        == v.binaural.as_ref().map_or(0, |b| b.latency_frames()),
                "reflection sample rate/reference latency mismatch"
            );
            ensure!(
                effect.is_tracked()
                    || (v
                        .spec
                        .spatial
                        .as_ref()
                        .is_none_or(|s| s.position == q.source)
                        && listener.position == q.listener
                        && listener.forward == q.listener_forward
                        && listener.up == q.listener_up),
                "reflection captured pose changed"
            );
        }
        v.reflection_pending = false;
        let old = std::mem::replace(&mut v.reflections, effect);
        constrain_tail(v);
        Ok(old)
    }
    /// Initial tracked preparation can wait behind HRTF preparation without
    /// consuming a one-shot's source. Both preparations remain independent jobs.
    pub fn pending_tracked_reflections(&mut self, client: u32, h: VoiceHandle) -> Result<()> {
        ensure!(
            self.source_is_playing(client, h),
            "reflection source has ended"
        );
        let v = self.voice(client, h)?;
        v.reflection_pending = v.reflections.is_none();
        Ok(())
    }
    pub fn reflection_plan_ready(&self, client: u32, h: VoiceHandle) -> Result<bool> {
        self.idle()?;
        self.spec(client, h)?;
        Ok(self.voices[h.slot as usize]
            .as_ref()
            .unwrap()
            .reflections
            .as_ref()
            .is_some_and(|r| r.is_tracked() && r.can_publish_plan()))
    }
    pub fn publish_reflection_plan(
        &mut self,
        client: u32,
        h: VoiceHandle,
        plan: crysirror_acoustics::reflection_dsp::ReflectionPlan,
        crossfade_ms: u32,
    ) -> Result<Option<crysirror_acoustics::reflection_dsp::ReflectionPlan>> {
        ensure!(
            self.direct_latency_frames(client, h)? == plan.direct_latency_frames(),
            "reflection reference latency changed"
        );
        self.voice(client, h)?
            .reflections
            .as_mut()
            .context("tracked reflection history is missing")?
            .publish_plan(plan, crossfade_ms)
    }
    pub fn suppress_tracked_reflections(&mut self, client: u32, h: VoiceHandle) -> Result<()> {
        if let Some(effect) = &mut self.voice(client, h)?.reflections {
            if effect.is_tracked() {
                effect.set_audible(false);
            }
        }
        Ok(())
    }
    pub fn retire_reflection_plan(
        &mut self,
        client: u32,
        h: VoiceHandle,
    ) -> Result<Option<crysirror_acoustics::reflection_dsp::ReflectionPlan>> {
        let v = self.voice(client, h)?;
        let old = v.reflections.as_mut().and_then(|r| r.retire_plan());
        constrain_tail(v);
        Ok(old)
    }
    pub fn reflection_paths(
        &self,
        client: u32,
        h: VoiceHandle,
    ) -> Result<Option<&crysirror_acoustics::reflections::ReflectionPaths>> {
        self.idle()?;
        self.spec(client, h)?;
        Ok(self.voices[h.slot as usize]
            .as_ref()
            .unwrap()
            .reflections
            .as_ref()
            .map(|r| r.paths()))
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
    /// Request asynchronous binaural preparation. Source/envelope clocks wait
    /// for installation, so a short one-shot cannot disappear while workers run.
    pub fn pending_binaural(&mut self, client: u32, h: VoiceHandle) -> Result<()> {
        let v = self.voice(client, h)?;
        ensure!(
            v.spec.spatial.is_some(),
            "binaural audio requires spatial voice"
        );
        v.binaural_pending = true;
        Ok(())
    }
    /// Owner publication of worker-allocated convolution state. None restores
    /// stereo panning. Existing ready state survives a rejected publication.
    pub fn set_binaural(
        &mut self,
        client: u32,
        h: VoiceHandle,
        voice: Option<crysirror_acoustics::binaural::BinauralVoice>,
    ) -> Result<()> {
        let rate = self.config.sample_rate;
        let v = self.voice(client, h)?;
        if let Some(voice) = &voice {
            ensure!(
                v.spec.spatial.is_some() && voice.sample_rate() == rate,
                "binaural voice sample rate/spatial mismatch"
            );
        }
        let latency = voice.as_ref().map_or(0, |b| b.latency_frames());
        if v.reflections
            .as_ref()
            .is_some_and(|r| r.direct_latency_frames() != latency)
        {
            if v.reflections.as_ref().is_some_and(|r| r.is_tracked()) {
                v.reflections.as_mut().unwrap().set_audible(false);
            } else {
                v.reflections = None;
            }
        }
        v.binaural = voice;
        v.binaural_pending = false;
        constrain_tail(v);
        Ok(())
    }
    pub fn pause(&mut self, client: u32, h: VoiceHandle, paused: bool) -> Result<()> {
        let voice = self.voice(client, h)?;
        voice.resume_after = None;
        voice.spec.paused = paused;
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
        voice.resume_after = None;
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
    /// Freeze input for an output-clock delay, then run it with a fresh fade-in.
    pub fn schedule_resume(
        &mut self,
        client: u32,
        h: VoiceHandle,
        delay_ms: f32,
        fade_ms: f32,
    ) -> Result<()> {
        self.schedule_resume_with_policy(client, h, delay_ms, fade_ms, AudioResumePolicy::Freeze)
    }
    /// Schedule a resume while optionally preserving the existing transport and
    /// its pause/stop transition. Deadlines progress on the output sample clock.
    pub fn schedule_resume_with_policy(
        &mut self,
        client: u32,
        h: VoiceHandle,
        delay_ms: f32,
        fade_ms: f32,
        policy: AudioResumePolicy,
    ) -> Result<()> {
        self.schedule_resume_with_settings(
            client,
            h,
            delay_ms,
            fade_ms,
            AudioResumeSettings {
                policy,
                reset_gain: false,
            },
        )
    }
    /// Schedule transport with optional fresh gain-ramp ownership at the
    /// deadline. The latest requested gain is read when the action executes.
    pub fn schedule_resume_with_settings(
        &mut self,
        client: u32,
        h: VoiceHandle,
        delay_ms: f32,
        fade_ms: f32,
        settings: AudioResumeSettings,
    ) -> Result<()> {
        duration(delay_ms)?;
        duration(fade_ms)?;
        let delay = frames(delay_ms, self.config.sample_rate);
        let fade = frames(fade_ms, self.config.sample_rate);
        let voice = self.voice(client, h)?;
        if voice.source_ended
            || (!voice.spec.looping
                && voice
                    .cursor
                    .source
                    .frame_count()
                    .is_some_and(|n| voice.position >= n as f64))
        {
            // A source action cannot restart a retained effect tail.
            voice.resume_after = None;
            return Ok(());
        }
        if delay == 0 {
            resume_now(voice, fade, settings);
        } else {
            if settings.policy == AudioResumePolicy::Freeze {
                voice.stop_after_fade = false;
                voice.stop_remaining = 0;
                voice.pause_after_fade = false;
                voice.spec.paused = true;
            }
            voice.resume_after = Some(ScheduledResume {
                left: delay,
                fade,
                settings,
            });
        }
        Ok(())
    }
    /// Current kinematic target and interpolated rate, frozen during worker DSP.
    pub fn doppler_info(&self, client: u32, h: VoiceHandle) -> Result<DopplerInfo> {
        if let Some(p) = &self.published {
            ensure!(client == h.client, "foreign audio handle");
            return p
                .voices
                .get(&h)
                .map(|v| v.doppler)
                .context("audio voice ended or stale");
        }
        let spec = self.spec(client, h)?;
        let v = self.voices[h.slot as usize].as_ref().unwrap();
        let listener = self.listener(client);
        let source = spec.spatial.as_ref();
        let velocity = source.and_then(|s| s.velocity).unwrap_or([0.; 3]);
        let settings = source.and_then(|s| s.doppler);
        let enabled = settings.is_some();
        let target = if let Some(settings) = settings {
            crate::doppler::calculate(
                settings,
                source.map_or([0.; 3], |s| s.position),
                velocity,
                listener.position,
                listener.velocity.unwrap_or([0.; 3]),
            )?
        } else {
            crate::doppler::DopplerCalculation {
                ratio: 1.,
                source_radial: 0.,
                listener_radial: 0.,
                velocity_clamped: false,
                ratio_clamped: false,
            }
        };
        Ok(DopplerInfo {
            enabled,
            ratio: v.doppler.value,
            target,
            target_pending: v.doppler.target != target.ratio || v.doppler_settings != settings,
            effective_rate: spec.rate * 2f32.powf(spec.pitch_semitones / 12.) * v.doppler.value,
            source_velocity: velocity,
            listener_velocity: listener.velocity.unwrap_or([0.; 3]),
            smoothing_frames: v.doppler.left,
        })
    }
    pub fn playback_info(
        &self,
        client: u32,
        h: VoiceHandle,
    ) -> Result<crysirror_core::service::AudioPlaybackInfo> {
        if let Some(p) = &self.published {
            ensure!(client == h.client, "foreign audio handle");
            return p
                .voices
                .get(&h)
                .map(|v| v.info)
                .context("audio voice ended or stale");
        }
        self.spec(client, h)?;
        let voice = self.voices[h.slot as usize].as_ref().unwrap();
        Ok(crysirror_core::service::AudioPlaybackInfo {
            age_frames: voice.age_frames,
            transport_frames: voice.transport_frames,
            stopping: voice.stop_after_fade,
            pausing: voice.pause_after_fade,
            source_playing: self.source_is_playing(client, h),
            paused: voice.spec.paused,
            looping: voice.spec.looping,
            position_frames: voice.position.max(0.).floor() as u64,
            source_rate: voice.cursor.source.sample_rate(),
            source_frames: voice.cursor.source.frame_count(),
            output_rate: self.config.sample_rate,
            fade_out_frames: if voice.stop_after_fade {
                voice.stop_remaining
            } else if voice.pause_after_fade {
                voice.envelope.left
            } else {
                0
            },
            resume_delay_frames: voice.resume_after.as_ref().map_or(0, |s| s.left),
        })
    }
    pub fn pause_client(&mut self, client: u32, paused: bool) {
        if self.is_rendering() {
            if self.clients.contains_key(&client) {
                self.deferred_pause.insert(client, paused);
            }
            return;
        }
        for v in self.voices.iter_mut().flatten() {
            if v.handle.client == client {
                v.spec.paused = paused
            }
        }
    }
    pub fn seek(&mut self, client: u32, h: VoiceHandle, source_frame: u64) -> Result<()> {
        let v = self.voice(client, h)?;
        v.cursor.source.seek(source_frame)?;
        v.seek = None;
        publish_seek(v, source_frame);
        Ok(())
    }
    /// Nonblocking sources only. A new sequence supersedes an older seek.
    pub fn begin_seek(&mut self, client: u32, h: VoiceHandle, frame: u64) -> Result<u64> {
        let v = self.voice(client, h)?;
        ensure!(
            v.cursor.source.is_nonblocking(),
            "source requires synchronous seeking"
        );
        ensure!(
            v.cursor.source.frame_count().is_none_or(|n| frame <= n),
            "seek outside source"
        );
        let sequence = v
            .seek_sequence
            .checked_add(1)
            .context("seek sequence exhausted")?;
        let ready = v.cursor.source.poll_seek(frame)?;
        v.seek_sequence = sequence;
        v.seek = Some(SeekProgress {
            sequence,
            frame,
            pending: !ready,
            error: None,
        });
        if ready {
            publish_seek(v, frame);
        }
        Ok(sequence)
    }
    pub fn seek_progress(
        &mut self,
        client: u32,
        h: VoiceHandle,
        sequence: u64,
    ) -> Result<SeekProgress> {
        let v = self.voice(client, h)?;
        let seek = v.seek.as_ref().context("seek released")?;
        ensure!(seek.sequence == sequence, "seek superseded");
        Ok(seek.clone())
    }
    /// Restore the prior logical position. The decoder realigns asynchronously;
    /// a canceled ticket cannot publish the discarded destination into playback.
    pub fn cancel_seek(&mut self, client: u32, h: VoiceHandle, sequence: u64) -> Result<()> {
        let v = self.voice(client, h)?;
        if v.seek
            .as_ref()
            .is_none_or(|s| s.sequence != sequence || !s.pending)
        {
            return Ok(());
        }
        let frame = v.position.floor() as u64;
        let ready = v.cursor.source.poll_seek(frame)?;
        v.seek = Some(SeekProgress {
            sequence: 0,
            frame,
            pending: !ready,
            error: None,
        });
        // Retain fractional source position on cancellation; cursor refill will
        // observe read_position and realign without rewinding the audible clock.
        Ok(())
    }
    pub fn position(&self, client: u32, h: VoiceHandle) -> Result<f64> {
        if let Some(p) = &self.published {
            ensure!(client == h.client, "foreign audio handle");
            return p
                .voices
                .get(&h)
                .map(|v| v.position)
                .context("audio voice ended or stale");
        }
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
        if let Some(p) = &self.published {
            ensure!(client == h.client, "foreign audio handle");
            return p
                .voices
                .get(&h)
                .map(|v| v.spec.clone())
                .context("audio voice ended or stale");
        }
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
    /// Owner/control phase. It polls completion and submits prefetch without
    /// waiting. Synchronous source backends preserve their existing behavior.
    pub fn service_sources(&mut self) -> Result<()> {
        self.idle()?;
        for v in self.voices.iter_mut().flatten() {
            v.cursor.source.service()?;
            if let Some(seek) = v.seek.as_mut().filter(|s| s.pending) {
                match v.cursor.source.poll_seek(seek.frame) {
                    Ok(false) => {}
                    Ok(true) => {
                        let frame = seek.frame;
                        let canceled = seek.sequence == 0;
                        seek.pending = false;
                        if !canceled {
                            publish_seek(v, frame);
                        }
                    }
                    Err(error) => {
                        seek.pending = false;
                        seek.error = Some(format!("{error:#}"));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn source_status(&self, client: u32, h: VoiceHandle) -> Result<Option<SourceBufferStatus>> {
        if let Some(p) = &self.published {
            ensure!(client == h.client, "foreign audio handle");
            return p
                .voices
                .get(&h)
                .map(|v| v.source.clone())
                .context("audio voice ended or stale");
        }
        ensure!(client == h.client, "foreign audio handle");
        let v = self
            .voices
            .get(h.slot as usize)
            .and_then(Option::as_ref)
            .context("audio voice ended")?;
        ensure!(v.handle == h, "stale audio handle");
        Ok(v.cursor.source.buffer_status())
    }
    /// Overwrites the destination with unclipped linear stereo PCM. One output
    /// limiter/clamp belongs at the hardware/export boundary, not per voice.
    pub fn render(&mut self, out: &mut [StereoFrame]) -> Result<()> {
        self.idle()?;
        self.service_sources()?;
        ensure!(
            out.len() <= self.config.sample_rate as usize * 10,
            "audio render block exceeds 10 seconds"
        );
        out.fill([0.; 2]);
        let rate = self.config.sample_rate;
        for v in self.voices.iter_mut().flatten() {
            let settings = v.spec.spatial.as_ref().and_then(|s| s.doppler);
            let target = if let (Some(s), Some(settings)) = (&v.spec.spatial, settings) {
                let l = self
                    .listeners
                    .get(&v.handle.client)
                    .copied()
                    .unwrap_or_default();
                crate::doppler::calculate(
                    settings,
                    s.position,
                    s.velocity.unwrap_or([0.; 3]),
                    l.position,
                    l.velocity.unwrap_or([0.; 3]),
                )?
                .ratio
            } else {
                1.
            };
            if v.doppler.target != target || v.doppler_settings != settings {
                v.doppler
                    .to(target, settings.map_or(0, |s| frames(s.smoothing_ms, rate)));
                v.doppler_settings = settings;
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
            self.groups.step();
            for c in self.clients.values_mut() {
                c.dialogue_voices = 0;
            }
            for v in self.voices.iter().flatten() {
                let c = self
                    .clients
                    .get_mut(&v.handle.client)
                    .expect("registered voice client");
                if v.spec.bus == Bus::Voice
                    && !v.spec.paused
                    && !v.binaural_pending
                    && !v.reflection_pending
                    && !v.source_ended
                    && v.resume_after
                        .as_ref()
                        .is_none_or(|r| r.settings.policy == AudioResumePolicy::Preserve)
                    && v.spec.gain > 0.
                    && c.level(Bus::Voice) > 0.
                    && self.groups.output(v.mix_group) > 0.
                {
                    c.dialogue_voices += 1;
                }
            }
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
                v.age_frames = v.age_frames.saturating_add(1);
                let doppler = v.doppler.step();
                if v.resume_after
                    .as_ref()
                    .is_some_and(|r| r.settings.policy == AudioResumePolicy::Freeze)
                {
                    continue;
                }
                if !v.spec.paused
                    && !v.source_ended
                    && (v.spec.looping
                        || v.cursor
                            .source
                            .frame_count()
                            .is_none_or(|n| v.position < n as f64))
                {
                    v.transport_frames = v.transport_frames.saturating_add(1);
                }
                if v.spec.paused
                    || v.binaural_pending
                    || v.reflection_pending
                    || v.seek
                        .as_ref()
                        .is_some_and(|s| s.pending || s.error.is_some())
                {
                    silent_transport_tick(v);
                    continue;
                }
                if v.source_ended && v.tail_left == 0 && !v.stop_after_fade {
                    *slot = None;
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
                let a = if v.source_ended {
                    [0.; 2]
                } else {
                    match v.cursor.sample(index)? {
                        SampleRead::Frame(a) => a,
                        SampleRead::Pending => {
                            self.stats.underrun_frames =
                                self.stats.underrun_frames.saturating_add(1);
                            silent_transport_tick(v);
                            continue;
                        }
                        SampleRead::End if v.spec.looping => {
                            v.position = 0.;
                            index = 0;
                            match v.cursor.sample(0)? {
                                SampleRead::Frame(a) => a,
                                SampleRead::Pending => {
                                    self.stats.underrun_frames =
                                        self.stats.underrun_frames.saturating_add(1);
                                    silent_transport_tick(v);
                                    continue;
                                }
                                SampleRead::End => {
                                    *slot = None;
                                    continue;
                                }
                            }
                        }
                        SampleRead::End => {
                            v.resume_after = None;
                            let tail = v
                                .binaural
                                .as_ref()
                                .map_or(0, |b| b.tail_frames())
                                .max(v.reflections.as_ref().map_or(0, |r| r.tail_frames()));
                            if tail > 0 || v.stop_after_fade {
                                v.source_ended = true;
                                v.tail_left = tail;
                                [0.; 2]
                            } else {
                                *slot = None;
                                continue;
                            }
                        }
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
                let t = (v.position - index as f64) as f32;
                let b = if v.source_ended || next == index || t == 0. {
                    a
                } else {
                    match v.cursor.sample(next)? {
                        SampleRead::Frame(b) => b,
                        SampleRead::End => a,
                        SampleRead::Pending => {
                            self.stats.underrun_frames =
                                self.stats.underrun_frames.saturating_add(1);
                            silent_transport_tick(v);
                            continue;
                        }
                    }
                };
                let mut s = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let echo = if let Some(effect) = &mut v.reflections {
                    effect.process_frame((s[0] + s[1]) * 0.5)?
                } else {
                    [0.; 2]
                };
                let c = &self.clients[&v.handle.client];
                let master_gain = c.level(Bus::Master);
                let mut gain = v.gain.step()
                    * v.envelope.step()
                    * c.level(v.spec.bus)
                    * self.groups.output(v.mix_group);
                let pan = v.pan.step();
                if pan != 0.5 && v.binaural.is_none() {
                    s[0] *= (1. - pan).sqrt() * std::f32::consts::SQRT_2;
                    s[1] *= pan.sqrt() * std::f32::consts::SQRT_2;
                }
                if v.spec.bus == Bus::Music {
                    gain *= c.duck_output
                }
                let source_gain = gain;
                if let Some(effect) = effect {
                    gain *= effect.gain;
                    let mono = (s[0] + s[1]) * 0.5;
                    let mono = v
                        .acoustic
                        .as_mut()
                        .map_or(mono, |filter| filter.process(mono));
                    s = if let Some(binaural) = &mut v.binaural {
                        binaural.process(mono)?
                    } else {
                        [mono * effect.pan[0], mono * effect.pan[1]]
                    };
                    if let Some(alpha) = effect.alpha {
                        for (state, input) in v.lowpass.iter_mut().zip(s) {
                            *state += alpha * (input - *state);
                        }
                        s = v.lowpass;
                    } else {
                        v.lowpass = s
                    }
                }
                if pan != 0.5 && v.binaural.is_some() {
                    s[0] *= (1. - pan).sqrt() * std::f32::consts::SQRT_2;
                    s[1] *= pan.sqrt() * std::f32::consts::SQRT_2;
                }
                let mixed = [
                    s[0] * gain + echo[0] * source_gain,
                    s[1] * gain + echo[1] * source_gain,
                ];
                sample[0] += mixed[0] * master_gain;
                sample[1] += mixed[1] * master_gain;
                // Post source-bus fader/duck/spatial, before Master. Master is
                // applied once on the return, not squared with the input send.
                let c = self.clients.get_mut(&v.handle.client).unwrap();
                if let Some(reverb) = &mut c.reverb {
                    let send = reverb.send(v.spec.bus);
                    reverb.input[0] += mixed[0] * send;
                    reverb.input[1] += mixed[1] * send;
                }
                if v.source_ended {
                    v.tail_left = v.tail_left.saturating_sub(1);
                    v.tail_elapsed = v.tail_elapsed.saturating_add(1);
                } else {
                    v.position += step * f64::from(doppler);
                }
                if !v.spec.looping
                    && v.cursor
                        .source
                        .frame_count()
                        .is_some_and(|n| v.position >= n as f64)
                {
                    v.resume_after = None;
                }
                if v.pause_after_fade && v.envelope.left == 0 {
                    v.spec.paused = true;
                    v.pause_after_fade = false;
                }
                if !v.stop_after_fade
                    && v.binaural.is_none()
                    && v.reflections.as_ref().is_none_or(|r| r.tail_frames() == 0)
                    && !v.spec.looping
                    && v.cursor
                        .source
                        .frame_count()
                        .is_some_and(|n| v.position >= n as f64)
                {
                    *slot = None
                }
            }
            // Apply deadlines after this output frame. Preserve mode consumes
            // the last waiting frame using the old fade/paused state; both modes
            // start resumed PCM on the next frame, including at chunk boundaries.
            for slot in &mut self.voices {
                let Some(v) = slot else { continue };
                if v.stop_after_fade {
                    v.stop_remaining = v.stop_remaining.saturating_sub(1);
                    if v.stop_remaining == 0 {
                        *slot = None;
                        continue;
                    }
                }
                let done = if let Some(r) = &mut v.resume_after {
                    r.left = r.left.saturating_sub(1);
                    (r.left == 0).then_some((r.fade, r.settings))
                } else {
                    None
                };
                if let Some((fade, settings)) = done {
                    resume_now(v, fade, settings);
                }
            }
            for c in self.clients.values_mut() {
                let Some(reverb) = &mut c.reverb else {
                    continue;
                };
                let input = std::mem::replace(&mut reverb.input, [0.; 2]);
                let wet = reverb.dsp.process_frame(input)?;
                let bus = reverb.settings.return_bus;
                let wet_gain = reverb.settings.wet_gain;
                let gain = wet_gain
                    * c.level(Bus::Master)
                    * c.level(bus)
                    * if bus == Bus::Music { c.duck_output } else { 1. };
                sample[0] += wet[0] * gain;
                sample[1] += wet[1] * gain;
            }
            ensure!(
                sample.iter().all(|x| x.is_finite()),
                "non-finite mixed audio"
            );
            self.stats.peak = self.stats.peak.max(sample[0].abs()).max(sample[1].abs());
        }
        self.stats.rendered_frames = self.stats.rendered_frames.saturating_add(out.len() as u64);
        self.service_sources()?;
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
