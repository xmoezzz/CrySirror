// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Narrow host services. Backends implement these; legacy adapters consume them.
use crate::{
    Result,
    path::VfsPath,
    state::{ClientId, Command, SceneState, TextRun},
};
use alloc::{string::String, sync::Arc, vec::Vec};
mod pattern;
pub use pattern::*;
mod media_probe;
pub use media_probe::*;
mod audio_loop;
pub use audio_loop::*;
mod audio_release;
pub use audio_release::*;
mod audio_gain_envelope;
pub use audio_gain_envelope::*;
mod audio_balance;
pub use audio_balance::*;
mod echo;
pub use echo::*;
mod audio_sequence;
pub use audio_sequence::*;
mod audio_replacement;
pub use audio_replacement::*;
mod audio_source_meter;
pub use audio_source_meter::*;
mod audio_pcm16_gain;
pub use audio_pcm16_gain::*;
mod audio_resident;
pub use audio_resident::*;
mod civil_time;
pub use civil_time::*;
mod http;
pub use http::*;
mod stepped_pointer;
mod stepped_pulse;
pub use stepped_pulse::{SteppedPulse,SteppedRamp};
pub use stepped_pointer::*;
mod view_transform;
pub use view_transform::*;
mod session_seed;
pub use session_seed::*;
mod diagnostics;
pub use diagnostics::*;
mod keyboard_locks;
pub use keyboard_locks::*;
mod font;
pub use font::*;
mod frame_history;
pub use frame_history::*;
mod input_observation;
pub use input_observation::*;
mod text_input;
pub use text_input::*;
mod platform;
pub use platform::*;
mod spectrum_graph;
pub use spectrum_graph::*;
mod mounted_media;
pub use mounted_media::*;
mod desktop;
pub use desktop::*;

pub const SIMPLE_TEXT_LAYOUT: &str = "simple-horizontal-v1";
/// Shared per-client count budget for retained drawing canvases and their
/// materialized images. Pixel bytes, dimensions and node graphs have separate
/// admission limits; reserving a logical surface does not allocate its pixels.
pub const MAX_CANVAS_SURFACES_PER_CLIENT: usize = 4096;
/// The same horizontal glyph metrics without automatic width wrapping.
pub const UNWRAPPED_TEXT_LAYOUT: &str = "simple-horizontal-unwrapped-v1";

/// Read-only capability for an asset such as a large native archive. Offsets
/// are explicit: callers can keep independent seek cursors without sharing a
/// filesystem path or the underlying stream position. Never serialized.
pub trait AssetReader: Send + Sync {
    fn len(&self) -> Result<u64>;
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize>;
    /// The bytes and length cannot change while this reader is alive.
    /// Read-only access alone does not imply an immutable backing source.
    fn is_immutable(&self) -> bool {
        false
    }
}

/// Bounded fallback for in-memory/provider assets; disk hosts can stream.
pub struct MemoryAssetReader(pub Arc<[u8]>);
impl AssetReader for MemoryAssetReader {
    fn is_immutable(&self) -> bool {
        true
    }
    fn len(&self) -> Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize> {
        if offset >= self.0.len() as u64 {
            return Ok(0);
        }
        let start = offset as usize;
        let count = output.len().min(self.0.len() - start);
        output[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

/// Host-owned exclusive namespace lease. Releasing this object releases the
/// lease; OS/process handles never enter VM or save data.
pub trait StorageLease: Send + Sync {}

/// Maximum resident stereo PCM frames (128 MiB of f32 samples).
pub const MAX_PCM_FRAMES: usize = 16 * 1024 * 1024;
/// Metadata for a validated resident PCM resource, without samples or leases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmAudioInfo {
    /// Sample frames per second, 8000 through 192000.
    pub sample_rate: u32,
    /// Number of stereo frames, independent of playback position.
    pub frames: usize,
    /// Resident sample bytes; excludes allocation and resource overhead.
    pub sample_bytes: usize,
}
/// Validated immutable PCM resource. Construction scans samples and allocates;
/// realtime callers prepare it on workers. Clone/publication never decodes.
/// Storage receipts and PCM are derived resources, never VM/project save data.
#[derive(Clone)]
pub struct PcmAudio {
    rate: u32,
    frames: Arc<[[f32; 2]]>,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl PcmAudio {
    pub fn new(rate: u32, frames: Vec<[f32; 2]>) -> Result<Self> {
        if !(8000..=192000).contains(&rate) {
            return Err(crate::Error::Invalid("PCM sample rate"));
        }
        if frames.is_empty() || frames.len() > MAX_PCM_FRAMES {
            return Err(crate::Error::Limit("PCM frame count"));
        }
        if frames
            .iter()
            .flatten()
            .any(|x| !x.is_finite() || *x < -16. || *x > 16.)
        {
            return Err(crate::Error::Invalid("nonfinite or excessive PCM"));
        }
        Ok(Self {
            rate,
            frames: frames.into(),
            storage: None,
        })
    }
    pub fn info(&self) -> PcmAudioInfo {
        PcmAudioInfo {
            sample_rate: self.rate,
            frames: self.frame_count(),
            sample_bytes: self.byte_len(),
        }
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn frames(&self) -> &[[f32; 2]] {
        &self.frames
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    pub fn byte_len(&self) -> usize {
        self.frames.len() * core::mem::size_of::<[f32; 2]>()
    }
    /// Attach before publication. Every live clone keeps the receipt until its
    /// samples are released, including after a producer resource is stopped.
    pub fn retain_storage(&mut self, storage: Arc<dyn Send + Sync>) {
        self.storage = Some(storage);
    }
}
impl core::fmt::Debug for PcmAudio {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PcmAudio")
            .field("rate", &self.rate)
            .field("frames", &self.frames.len())
            .finish()
    }
}
/// Logical encoded-audio resource; container policies are resolved by adapters.
#[derive(Clone)]
pub struct EncodedAudio {
    pub bytes: Arc<[u8]>,
    pub frame_limit: Option<u64>,
}
impl core::fmt::Debug for EncodedAudio {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncodedAudio")
            .field("bytes", &self.bytes.len())
            .field("frame_limit", &self.frame_limit)
            .finish()
    }
}
#[derive(Clone, Copy, Debug)]
pub enum PcmBus {
    Music,
    Sfx,
    Voice,
    Ambient,
    Ui,
}
/// A client-owned logical gain group. Parents must exist in the same mixer
/// namespace. This is a control graph, not a second decoder/effect engine.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioMixGroupSettings {
    /// Optional group parent; None routes directly to the voice's fixed bus.
    pub parent: Option<String>,
    /// Independent linear gain, 0..4. Multiplies the voice and ancestor gains.
    pub gain: f32,
    /// Fade toward silence while retaining the authored gain value.
    pub muted: bool,
}
impl Default for AudioMixGroupSettings {
    fn default() -> Self {
        Self {
            parent: None,
            gain: 1.,
            muted: false,
        }
    }
}
/// Owner-observed group controls; frame counters use this mixer's output clock.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioMixGroupInfo {
    /// Authored target settings, independent of the currently interpolated gain.
    pub settings: AudioMixGroupSettings,
    /// Own interpolated gain for the next output frame, including mute.
    pub gain: f32,
    /// Product of current own and ancestor gains for the next output frame.
    pub effective_gain: f32,
    /// Remaining gain fade frames, independent of voice pause/starvation.
    pub fade_frames: u64,
    /// Frames per second used by this group's fade counter.
    pub output_rate: u32,
}
#[derive(Clone, Debug)]
pub struct AudioParameters {
    /// Optional stepped attenuation installed atomically with the new source.
    /// Its clock starts at actual publication, not preparation-request time.
    pub gain_envelope: Option<AudioGainEnvelopeSpec>,
    /// Optional legacy quantized fader. Ordinary playback uses a float gain.
    pub pcm16_gain: Option<AudioPcm16Gain>,
    /// Mixer voice gain, default 1.
    pub gain: f32,
    /// Legacy amplitude applied by the shared decoded-source wrapper, default 1.
    pub decode_gain: f32,
    /// Source playback rate, default 1.
    pub rate: f32,
    /// Stereo balance 0..1, with 0.5 centered.
    pub pan: f32,
    /// Repeat source input, default false.
    pub looping: bool,
    /// Total remaining source-range passes, including the current pass.
    /// None preserves unbounded looping; Some must be positive and requires looping.
    pub loop_passes: Option<u32>,
    /// Exchange decoded left/right channels before pan/spatial processing.
    pub swap_channels: bool,
    /// Decoded-source frame to repeat from, default zero.
    pub loop_start: u64,
    /// Decoded-source frame at which a loop jumps back to `loop_start`;
    /// None loops at the end of the source.
    pub loop_end: Option<u64>,
    /// Startup position in source milliseconds unless start_frame is set.
    pub start_ms: u64,
    /// Exact decoded-source frame; when present this overrides start_ms.
    pub start_frame: Option<u64>,
    /// Clamp authored source/loop positions to the prepared source boundary.
    pub clamp_range: bool,
    /// Keep the canonical source, cursor and controls after natural EOF or an
    /// own-gain stop. Held voices consume pool capacity and cannot be stolen;
    /// admission never steals another voice for a retained instance. Stop and
    /// client replacement still release it. Default false.
    pub retain_on_finish: bool,
    /// Run input/per-voice DSP. Inactive transport retains source position while
    /// independent gain/pan controls continue; ordinary Pause still freezes
    /// those controls. Reactivating EOF requires a completed seek. Default true.
    pub transport_active: bool,
    /// Publish without consuming samples until an explicit resume.
    pub paused: bool,
    /// Fade the previous channel voice after successful replacement publication.
    pub replace_fade_ms: f32,
    /// Initial fade-in duration in milliseconds.
    pub fade_ms: f32,
    /// False keeps a currently active channel instead of reopening it.
    pub restart: bool,
    /// Optional existing client-scoped mix group, default None.
    pub mix_group: Option<String>,
    /// Shared output bus, default Sfx.
    pub bus: PcmBus,
}
impl Default for AudioParameters {
    fn default() -> Self {
        Self {
            mix_group: None,
            gain_envelope: None,
            pcm16_gain: None,
            gain: 1.,
            decode_gain: 1.,
            rate: 1.,
            pan: 0.5,
            looping: false,
            loop_passes: None,
            swap_channels: false,
            loop_start: 0,
            loop_end: None,
            start_ms: 0,
            start_frame: None,
            clamp_range: false,
            paused: false,
            retain_on_finish: false,
            transport_active: true,
            replace_fade_ms: 0.,
            fade_ms: 0.,
            restart: true,
            bus: PcmBus::Sfx,
        }
    }
}
/// Shared source transport inspection. Processing tails do not prolong source
/// completion. Frame positions are decoded-source units; fade/delay counts use
/// output_rate. No device or executable pointers cross this boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioPlaybackInfo {
    /// Peak absolute source PCM in the last completed mixer block, before
    /// gain, bus, pan and spatial processing. Paused/inactive sources report 0.
    pub source_peak: f32,
    /// Signed source samples at interleaved block indices 0, N/4, 3N/4
    /// and N-2, before gain, panning, channel exchange and per-voice effects.
    /// They support inexpensive presentation/lip-sync meters without exposing
    /// a decoder buffer or its lifetime. Inactive sources return zeroes.
    pub source_taps: [f32; 4],
    /// Mixer-owner observation of the independent external dB balance track.
    pub balance: AudioBalanceInfo,
    /// Source has remaining input, even when paused; excludes processing tails.
    pub source_playing: bool,
    /// At least one source/loop endpoint has been reached by mixer transport.
    /// Latched until this voice is replaced; decoder read-ahead, pause, gain
    /// and wall time alone cannot set it. A joined intro belongs to the first
    /// pass; later passes begin at the configured loop start. Seek preserves it.
    pub first_pass_complete: bool,
    /// Source clock currently frozen by pause or a freezing scheduled resume.
    pub paused: bool,
    /// Canonical source retention/protection policy of this allocated voice.
    pub retain_on_finish: bool,
    /// Whether input/per-voice DSP may run. Separate from full Pause and from
    /// source_playing, which reports remaining input even when inactive.
    pub transport_active: bool,
    /// Actual prepared source loop mode.
    pub looping: bool,
    /// Remaining range passes including the current pass; None is unbounded
    /// when looping, or no counted schedule when looping is disabled.
    pub loop_passes: Option<u32>,
    /// Actual decoded-channel exchange before pan/spatial processing.
    pub swap_channels: bool,
    /// Output frames since voice publication, including pauses, dependency
    /// holds, starvation and effect tails. Seeking does not reset this age.
    pub age_frames: u64,
    /// Output frames while input transport is running, including decoder/seek/
    /// preparation holds. Excludes fully paused/delayed input and effect tails.
    /// Pause fades count until frozen; seek, loop and playback rate do not reset it.
    /// Explicit retained-source activation starts a new playback interval.
    pub transport_frames: u64,
    /// A stop fade or scheduled retirement is pending on the allocated voice.
    pub stopping: bool,
    /// Input will pause when its current fade completes.
    pub pausing: bool,
    /// Current decoded-source frame, without worker read-ahead.
    pub position_frames: u64,
    /// Decoded-source frames per second.
    pub source_rate: u32,
    /// Prepared source boundary, when known.
    pub source_frames: Option<u64>,
    /// Mixer frames per second, used by fade/delay counters.
    pub output_rate: u32,
    /// Remaining stop deadline or pause-fade frames; gain fades are excluded.
    /// A stop deadline is independent of a later pause envelope.
    pub fade_out_frames: u64,
    /// Remaining scheduled-resume delay in mixer frames.
    pub resume_delay_frames: u64,
    /// Current own voice amplitude and target, excluding bus/groups/envelopes.
    pub gain: f32,
    pub gain_target: f32,
    /// Remaining unpaused output-control frames in the amplitude ramp. Parked
    /// transport still advances it; explicit full Pause freezes it.
    pub gain_fade_frames: u64,
    /// Stop after this amplitude ramp, independently of transport stop/pause.
    pub gain_stop_after_fade: bool,
}
/// How existing input transport behaves before a scheduled resume deadline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioResumePolicy {
    /// Freeze immediately, canceling any pending pause/stop fade.
    #[default]
    Freeze,
    /// Keep running or paused, including any current pause/stop fade, until
    /// the deadline. Source EOF or completed stop cancels the scheduled action.
    Preserve,
}
/// Scheduled input/gain transition, independent of asset preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioResumeSettings {
    /// Input behavior before the output-clock deadline.
    pub policy: AudioResumePolicy,
    /// At the deadline, cancel any unfinished gain ramp and fade from zero to
    /// the latest requested gain using fade_ms, with transport envelope open.
    /// False preserves the gain ramp and applies fade_ms to transport instead.
    pub reset_gain: bool,
}
/// Deferred, client-capability audio source. No bytes or device pointers in VM state.
#[derive(Clone, Debug)]
pub struct AudioAsset {
    /// Canonical read-only asset path under the submitting client.
    pub path: VfsPath,
    /// Optional native declared decoded-frame boundary.
    pub frame_limit: Option<u64>,
}
/// Worker-prepared numeric transform, separate from source transport/pitch.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioResourceTransform {
    /// Pinned mono PCM16 splice profile; 100..400 percent, 100 exact identity.
    SiglusJitan { percent: u16 },
    /// Ordered integer stereo filter chain applied to the decoded PCM16
    /// (at most 16 stages); see [`PcmFilter`].
    PcmFilters(alloc::sync::Arc<[PcmFilter]>),
    /// Ordered floating-point WaveFilter chain (at most 16 stages) applied to
    /// the decoded PCM at its own rate; see [`WaveFilter`].
    WaveFilters(alloc::sync::Arc<[WaveFilter]>),
    /// Independently gained stereo stems (1..=64) of one voice, mixed in
    /// lockstep; see [`MultiTrackStem`] and `AudioRequest::SetTrackGains`.
    MultiTrack(alloc::sync::Arc<[MultiTrackStem]>),
}
/// One stem of a multi-track resource: the resource's own asset or another
/// asset of the same client and rate, optionally a stereo pick of its decoded
/// channels (`None` entries are silent; up to 32 channels), at a start gain.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiTrackStem {
    pub asset: Option<crate::path::VfsPath>,
    pub pick: Option<[Option<u16>; 2]>,
    pub gain: f32,
}
impl MultiTrackStem {
    pub fn validate(&self) -> Result<()> {
        if !self.gain.is_finite()
            || !(0.0..=16.).contains(&self.gain)
            || self.pick.is_some_and(|p| p.iter().flatten().any(|c| *c >= 32))
        {
            return Err(crate::Error::Invalid("multi-track stem"));
        }
        Ok(())
    }
}
/// Gain change for some tracks of a multi-track voice (`None`: all tracks),
/// reached linearly over `fade_ms` milliseconds of source time.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrackGain {
    pub tracks: Option<alloc::vec::Vec<u32>>,
    pub gain: f32,
    pub fade_ms: u32,
}
impl AudioResourceTransform {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::SiglusJitan { percent } if (100..=400).contains(percent) => Ok(()),
            Self::PcmFilters(chain) if !chain.is_empty() && chain.len() <= 16 => {
                chain.iter().try_for_each(PcmFilter::validate)
            }
            Self::WaveFilters(chain) if !chain.is_empty() && chain.len() <= 16 => {
                chain.iter().try_for_each(WaveFilter::validate)
            }
            Self::MultiTrack(stems) if !stems.is_empty() && stems.len() <= 64 => {
                stems.iter().try_for_each(MultiTrackStem::validate)
            }
            _ => Err(crate::Error::Invalid("audio transform parameters")),
        }
    }
}
/// Wet routing of a delay/modulation stage (normal, stereo-inverted right,
/// left-only, right-only wet signal); dry input is kept on both sides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PcmFilterMix {
    #[default]
    Normal,
    Stereo,
    Left,
    Right,
}
/// How generated noise combines with the input before the amount crossfade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PcmNoiseBlend {
    Replace,
    Add,
    #[default]
    Multiply,
}
/// One integer stereo filter stage with native-range integer parameters
/// (percentages, milliseconds, Hz). Algorithms live in crysirror-audio; the
/// arithmetic follows the QLIE OggData TODFilter family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PcmFilter {
    Echo { decay: i32, delay_ms: i32, wet: i32, dry: i32, mix: PcmFilterMix },
    Reverb { decay: i32, delay_ms: i32, wet: i32, dry: i32, mix: PcmFilterMix, room_hz: i32 },
    Chorus { decay: i32, delay_ms: i32, wet: i32, dry: i32, mix: PcmFilterMix, rate: i32, depth: i32 },
    Volume { volume: i32, range: i32, pan: i32 },
    Distortion { volume: i32, range: i32, foldback: bool },
    Noise { amount: i32, volume: i32, blend: PcmNoiseBlend },
    ThreeBandEq { low: i32, mid: i32, high: i32, low_hz: i32, high_hz: i32 },
    Mono { amount: i32 },
    PitchShift { pitch: i32, wet: i32, dry: i32, mix: PcmFilterMix },
    Agc { gain: i32, rate: i32, quiet: i32, target: i32 },
}
impl PcmFilter {
    pub fn validate(&self) -> Result<()> {
        let within = |v: i32, lo: i32, hi: i32| (lo..=hi).contains(&v);
        let ok = match *self {
            Self::Echo { decay, delay_ms, wet, dry, .. } => {
                within(decay, 0, 100) && within(delay_ms, 5, 1000) && within(wet, 0, 1000) && within(dry, 0, 1000)
            }
            Self::Reverb { decay, delay_ms, wet, dry, room_hz, .. } => {
                within(decay, 0, 100) && within(delay_ms, 5, 1000) && within(wet, 0, 1000) && within(dry, 0, 1000)
                    && within(room_hz, 20, 40000)
            }
            Self::Chorus { decay, delay_ms, wet, dry, rate, depth, .. } => {
                within(decay, 0, 100) && within(delay_ms, 5, 1000) && within(wet, 0, 1000) && within(dry, 0, 1000)
                    && within(rate, 0, 2000) && within(depth, 0, 100)
            }
            Self::Volume { volume, range, pan } => within(volume, 0, 1000) && within(range, 10, 200) && within(pan, 0, 100),
            Self::Distortion { volume, range, .. } => within(volume, 0, 1000) && within(range, 0, 400),
            Self::Noise { amount, volume, .. } => within(amount, 0, 100) && within(volume, 0, 100),
            Self::ThreeBandEq { low, mid, high, low_hz, high_hz } => {
                [low, mid, high].iter().all(|v| within(*v, -1000, 1000)) && within(low_hz, 50, 24000) && within(high_hz, 50, 24000)
            }
            Self::Mono { amount } => within(amount, 0, 100),
            Self::PitchShift { pitch, wet, dry, .. } => within(pitch, 10, 400) && within(wet, 0, 200) && within(dry, 0, 200),
            Self::Agc { gain, rate, quiet, target } => {
                within(gain, 10, 400) && within(rate, 10, 1000) && within(quiet, 0, 32700) && within(target, 0, 32700)
            }
        };
        if ok { Ok(()) } else { Err(crate::Error::Invalid("PCM filter parameters")) }
    }
}
/// Response family of a [`WaveFilter::Iir`] design (DSPFilters families).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IirFamily {
    Rbj,
    Butterworth,
    ChebyshevI,
    ChebyshevII,
    Elliptic,
    Bessel,
    Legendre,
    Custom,
}
/// Response kind of a [`WaveFilter::Iir`] design. `BandPass` is the pole
/// families' band pass and RBJ's constant-skirt band pass; `BandPass2` is
/// RBJ's constant-peak band pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IirKind {
    LowPass,
    HighPass,
    BandPass,
    BandPass2,
    BandStop,
    LowShelf,
    HighShelf,
    BandShelf,
    AllPass,
    TwoPole,
    OnePole,
}
/// Biquad realisation of a [`WaveFilter::Iir`] cascade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IirStructure {
    DirectFormI,
    #[default]
    DirectFormII,
    TransposedDirectFormI,
    TransposedDirectFormII,
}
/// One DSPFilters design parameter (`ParamInfo`). `min`/`max` are the
/// upstream `toNativeValue(0)`/`toNativeValue(1)` of its control mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IirParamInfo {
    /// Short name (`szLabel`, e.g. `Fc`).
    pub label: &'static str,
    /// Long name (`szName`, e.g. `Cutoff Frequency`).
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}
const fn param(label: &'static str, name: &'static str, min: f64, max: f64, default: f64) -> IirParamInfo {
    IirParamInfo { label, name, default, min, max }
}
// Log-mapped frequencies: 1.5^(log(10)/log(1.5)) evaluates to 10.000000000000004.
const LOG_MIN: f64 = 10.000000000000004;
const P_FS: IirParamInfo = param("Fs", "Sample Rate", 11025., 192000., 44100.);
const P_FC: IirParamInfo = param("Fc", "Cutoff Frequency", LOG_MIN, 22040., 2000.);
const P_FCENT: IirParamInfo = param("Fc", "Center Frequency", LOG_MIN, 22040., 2000.);
const P_Q: IirParamInfo = param("Q", "Resonance", 0.0625, 16., 1.);
const P_BW: IirParamInfo = param("BW", "Bandwidth (Octaves)", 0.0625, 16., 1.);
const P_BWHZ: IirParamInfo = param("BW", "Bandwidth (Hz)", LOG_MIN, 22040., 1720.);
const P_GAIN: IirParamInfo = param("Gain", "Gain", -24., 24., -6.);
const P_SLOPE: IirParamInfo = param("Slope", "Slope", 0.25, 4., 1.);
const P_RIPPLE: IirParamInfo = param("Ripple", "Ripple dB", 0.001, 12., 0.01);
const P_STOP: IirParamInfo = param("Stop", "Stopband dB", 3., 60., 48.);
const P_ROLLOFF: IirParamInfo = param("W", "Transition Width", -16., 4., 0.);
const P_POLE_RHO: IirParamInfo = param("Pd", "Pole Distance", 0., 1., 0.5);
const P_POLE_THETA: IirParamInfo = param("Pa", "Pole Angle", 0., core::f64::consts::PI, core::f64::consts::FRAC_PI_2);
// Upstream reuses the pole labels for the zero parameters.
const P_ZERO_RHO: IirParamInfo = param("Pd", "Zero Distance", 0., 1., 0.5);
const P_ZERO_THETA: IirParamInfo = param("Pa", "Zero Angle", 0., core::f64::consts::PI, core::f64::consts::FRAC_PI_2);
const P_POLE_REAL: IirParamInfo = param("A1", "Pole Real", -1., 1., 0.25);
const P_ZERO_REAL: IirParamInfo = param("B1", "Zero Real", -1., 1., -0.25);
/// Highest order of a pole family as the shipped designs instantiate it.
pub fn iir_max_order(family: IirFamily) -> usize {
    match family {
        IirFamily::Bessel | IirFamily::Legendre => 25,
        _ => 50,
    }
}
/// A design's parameter list (element 0 is the sample rate), `None` for a
/// kind the family does not implement.
pub fn iir_params(kind: IirKind, family: IirFamily) -> Option<alloc::vec::Vec<IirParamInfo>> {
    use IirFamily as F;
    use IirKind as K;
    use alloc::vec;
    let order = param("Order", "Order", 1., iir_max_order(family) as f64, 2.);
    let extra: &[IirParamInfo] = match family {
        F::ChebyshevI => &[P_RIPPLE],
        F::ChebyshevII => &[P_STOP],
        F::Elliptic => &[P_RIPPLE, P_ROLLOFF],
        _ => &[],
    };
    let pole = |base: &[IirParamInfo]| {
        let mut v = vec![P_FS, order];
        v.extend_from_slice(base);
        v.extend_from_slice(extra);
        Some(v)
    };
    match (family, kind) {
        (F::Rbj, K::LowPass | K::HighPass | K::AllPass) => Some(vec![P_FS, P_FC, P_Q]),
        (F::Rbj, K::BandPass | K::BandPass2 | K::BandStop) => Some(vec![P_FS, P_FCENT, P_BW]),
        (F::Rbj, K::LowShelf | K::HighShelf) => Some(vec![P_FS, P_FC, P_GAIN, P_SLOPE]),
        (F::Rbj, K::BandShelf) => Some(vec![P_FS, P_FCENT, P_GAIN, P_BW]),
        (F::Custom, K::OnePole) => Some(vec![P_FS, P_GAIN, P_POLE_REAL, P_ZERO_REAL]),
        (F::Custom, K::TwoPole) => Some(vec![P_FS, P_GAIN, P_POLE_RHO, P_POLE_THETA, P_ZERO_RHO, P_ZERO_THETA]),
        (F::Rbj | F::Custom, _) => None,
        (F::Elliptic | F::Legendre, K::LowShelf | K::HighShelf | K::BandShelf) => None,
        (F::Bessel, K::HighShelf | K::BandShelf) => None,
        (_, K::LowPass | K::HighPass) => pole(&[P_FC]),
        (_, K::BandPass | K::BandStop) => pole(&[P_FCENT, P_BWHZ]),
        (_, K::LowShelf | K::HighShelf) => pole(&[P_FC, P_GAIN]),
        (_, K::BandShelf) => pole(&[P_FCENT, P_BWHZ, P_GAIN]),
        (_, K::BandPass2 | K::AllPass | K::TwoPole | K::OnePole) => None,
    }
}
/// `getDefaultParams()`: every parameter's default, unused slots zero.
pub fn iir_default_params(kind: IirKind, family: IirFamily) -> Option<[f64; 8]> {
    let list = iir_params(kind, family)?;
    let mut out = [0.; 8];
    for (o, p) in out.iter_mut().zip(&list) {
        *o = p.default;
    }
    Some(out)
}
/// One floating-point WaveFilter stage over decoded PCM. Algorithms live in
/// crysirror-audio (`wave_filters`). `extend_ms` stages keep running on
/// silence after the input ends, so their tail is part of the resource.
#[derive(Clone, Debug, PartialEq)]
pub enum WaveFilter {
    /// Per-channel feedback delay: `d = line[t - time]`,
    /// `line[t] = (x + d) * feedback`, `y = x + d * mix`.
    Delay { time_ms: u32, feedback: f32, mix: f32, extend_ms: u32 },
    /// Jezar FreeVerb in the STK formulation (stereo output).
    FreeVerb { effect_mix: f64, room_size: f64, damping: f64, width: f64, frozen: bool, extend_ms: u32 },
    /// Ten-band allpass-section graphic equalizer, linear band gains.
    GraphicEq { gains: [f32; 10] },
    /// DSPFilters-style cascade. `params[0]` is replaced by the stream rate;
    /// the rest follow the design's parameter order.
    Iir { kind: IirKind, family: IirFamily, structure: IirStructure, params: [f64; 8] },
    /// Linear gain, then by `mode`: 0 nothing, 1..=4 a sigmoid soft limit
    /// above `threshold` (x/(1+|x|), 2·atan(πx/2)/π, x/√(1+x²), tanh), any
    /// other value a hard clip to ±`threshold`.
    GainLimit { gain: f32, threshold: f32, mode: i8 },
    /// KiriKiri phase vocoder: `pitch` frequency scale, `time` duration
    /// scale, FFT `window` (power of two) and `overlap` (0 automatic).
    PhaseVocoder { pitch: f32, time: f32, window: u32, overlap: u32 },
    /// `|x|`.
    Abs,
    /// `x + value`.
    Add { value: f32 },
    /// `x · value`.
    Mul { value: f32 },
    /// Every channel multiplied by a mono carrier.
    RingMod { carrier: Oscillator },
    /// Channel vocoder (voclib): the mono-mixed input shapes the carrier.
    Vocoder { carrier: Oscillator, formant_shift: f32, reaction_time: f32, bands: u8, filters_per_band: u8 },
}
/// Mono carrier generator of [`WaveFilter::RingMod`] / [`WaveFilter::Vocoder`].
/// Frequencies are in Hz at the stream rate.
#[derive(Clone, Debug, PartialEq)]
pub enum Oscillator {
    Const { value: f32 },
    /// 2048-point table sine (STK SineWave).
    Sine { gain: f32, freq: f32 },
    /// xoshiro128+ white noise in ±gain; `seed` None uses a fixed seed.
    Noise { gain: f32, seed: Option<u64> },
    /// STK band-limited impulse train; `harmonics` 0 means up to Nyquist.
    Blit { gain: f32, freq: f32, harmonics: u32 },
    BlitSaw { gain: f32, freq: f32, harmonics: u32 },
    BlitSquare { gain: f32, freq: f32, harmonics: u32 },
    /// STK Modulate: vibrato sine plus low-passed random drift.
    Modulate { gain: f32, vibrato_rate: f32, vibrato_gain: f64, random_gain: f64 },
    /// Sum of generators.
    Mix { sources: alloc::vec::Vec<Oscillator> },
    /// A generator run through filter stages as a mono signal.
    Filter { source: alloc::boxed::Box<Oscillator>, filters: alloc::vec::Vec<WaveFilter> },
    /// Mono samples looped end to start, read at `pitch` with linear steps.
    Loop { samples: alloc::sync::Arc<[f32]>, pitch: f64 },
}
impl Oscillator {
    fn validate(&self, depth: u32) -> Result<()> {
        let fin = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let ok = depth < 8
            && match self {
                Self::Const { value } => fin(&[*value]),
                Self::Sine { gain, freq } => fin(&[*gain, *freq]),
                Self::Noise { gain, .. } => fin(&[*gain]),
                Self::Blit { gain, freq, .. } | Self::BlitSaw { gain, freq, .. } | Self::BlitSquare { gain, freq, .. } => {
                    fin(&[*gain, *freq]) && *freq > 0.
                }
                Self::Modulate { gain, vibrato_rate, vibrato_gain, random_gain } => {
                    fin(&[*gain, *vibrato_rate]) && vibrato_gain.is_finite() && random_gain.is_finite()
                }
                Self::Mix { sources } => sources.len() <= 32 && sources.iter().all(|s| s.validate(depth + 1).is_ok()),
                Self::Filter { source, filters } => {
                    filters.len() <= 16
                        && source.validate(depth + 1).is_ok()
                        && filters.iter().all(|f| f.validate_at(depth + 1).is_ok())
                }
                Self::Loop { samples, pitch } => !samples.is_empty() && pitch.is_finite() && *pitch > 0.,
            };
        if ok { Ok(()) } else { Err(crate::Error::Invalid("oscillator parameters")) }
    }
}
impl WaveFilter {
    pub fn validate(&self) -> Result<()> {
        self.validate_at(0)
    }
    fn validate_at(&self, depth: u32) -> Result<()> {
        let ok = match self {
            Self::Delay { time_ms, feedback, mix, extend_ms } => {
                (1..=10_000).contains(time_ms) && feedback.is_finite() && mix.is_finite() && *extend_ms <= 60_000
            }
            Self::FreeVerb { effect_mix, room_size, damping, width, extend_ms, .. } => {
                [effect_mix, room_size, damping, width].iter().all(|v| v.is_finite()) && *extend_ms <= 60_000
            }
            Self::GraphicEq { gains } => gains.iter().all(|g| g.is_finite()),
            Self::Iir { params, .. } => params.iter().all(|v| v.is_finite()),
            Self::GainLimit { gain, threshold, .. } => gain.is_finite() && threshold.is_finite(),
            Self::PhaseVocoder { pitch, time, window, overlap } => {
                pitch.is_finite() && *pitch > 0. && time.is_finite() && *time > 0.
                    && window.is_power_of_two() && (64..=32768).contains(window)
                    && matches!(overlap, 0 | 2 | 4 | 8 | 16 | 32)
            }
            Self::Abs => true,
            Self::Add { value } | Self::Mul { value } => value.is_finite(),
            Self::RingMod { carrier } => carrier.validate(depth).is_ok(),
            Self::Vocoder { carrier, formant_shift, reaction_time, bands, filters_per_band } => {
                carrier.validate(depth).is_ok()
                    && (0.25..=4.).contains(formant_shift)
                    && (0.002..=2.).contains(reaction_time)
                    && (4..=96).contains(bands)
                    && (1..=8).contains(filters_per_band)
            }
        };
        if ok { Ok(()) } else { Err(crate::Error::Invalid("wave filter parameters")) }
    }
}
/// Ephemeral client-scoped immutable audio resource. IDs are not reused during
/// a host process; handles never enter project or native/runtime save bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AudioResourceHandle {
    /// Owning client capability.
    pub client: u32,
    /// Host-minted resource identity, distinct from a playback voice.
    pub id: u64,
}
/// Encoded snapshot metadata, independent of playback cursors and devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioResourceInfo {
    /// Decoded-source frames per second.
    pub source_rate: u32,
    /// Bounded decoded-source length when known/measured.
    pub source_frames: Option<u64>,
    /// Immutable encoded snapshot bytes retained by the resource.
    pub encoded_bytes: u64,
    /// Replay-gain tags were found in the source metadata. Presence does not
    /// mean a gain policy was applied; adapters must handle or reject it.
    pub has_replay_gain: bool,
}
/// Resource preparation and publication, separate from creating a voice.
#[derive(Clone, Debug)]
pub struct AudioResourcePreparation {
    /// Pending, ready or failed resource work.
    pub phase: AudioPreparePhase,
    /// Published resource capability, present only when ready.
    pub resource: Option<AudioResourceHandle>,
    /// Published metadata, present only when ready.
    pub info: Option<AudioResourceInfo>,
    /// Retained preparation error.
    pub error: Option<String>,
}
/// Acknowledgement of preparation and owner publication, separate from EOF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioPreparePhase {
    Pending,
    Ready,
    Failed,
}
/// Readiness is distinct from currently playing: short audio can finish before
/// its caller consumes a successful preparation receipt.
#[derive(Clone, Debug)]
pub struct AudioPreparation {
    /// Client-local channel targeted by the request.
    pub channel: u32,
    /// Ready means published; it does not mean the voice is still active.
    pub phase: AudioPreparePhase,
    /// Retained preparation/publication error.
    pub error: Option<String>,
}
/// Client-scoped channel commands for the shared decoder/mixer. Encoded bytes
/// and decoder resources are not part of logical VM snapshots.
#[derive(Clone, Debug)]
pub enum AudioRequest {
    /// Bounded segmented source, decoded on the existing audio worker. Nested
    /// repeats do not expand encoded data or decoded PCM in memory.
    PlayPlaylist { channel: u32, source: AudioPlaylist, parameters: AudioParameters },
    /// Decode all bounded input frames before publication. Playback, loops and
    /// exact seeks subsequently use resident PCM without a decoder/I/O worker.
    PlayResident {channel:u32,source:EncodedAudio,intro:Option<EncodedAudio>,parameters:AudioParameters},
    /// Control the independent dB balance on this client's channel. Missing
    /// sources remain a no-op; newly published voices start centered.
    Balance { channel:u32, command:AudioBalanceCommand },
    /// Atomically exchange two logical channels in this client. The existing
    /// voice/source handles, fractional cursor, DSP history and gain/transport
    /// ramps remain intact. An empty side moves the other voice. Unpublished
    /// preparation intents, pending seeks/retirements or an active render lease
    /// reject before mutation. This is unrelated to decoded left/right exchange.
    ExchangeChannels {
        left: u32,
        right: u32,
    },
    /// Set only a group's gain, creating a root with default settings if absent.
    /// Existing parent and mute controls remain host-owned.
    SetMixGroupGain {
        name: String,
        gain: f32,
        fade_ms: f32,
    },
    /// Configure an owner-scoped gain group, with atomic parent/cycle/budget
    /// validation and an output-clock fade. Does not open an audio source.
    SetMixGroup {
        name: String,
        settings: AudioMixGroupSettings,
        fade_ms: f32,
    },
    /// Open an independent decoder over an existing immutable encoded resource
    /// on workers. Releasing its handle does not invalidate admitted/live work.
    PlayResource {
        channel: u32,
        resource: AudioResourceHandle,
        parameters: AudioParameters,
    },
    /// Publish a worker-prepared PCM resource without opening a codec.
    PlayPcm {
        channel: u32,
        source: PcmAudio,
        parameters: AudioParameters,
    },
    /// Worker preparation only; synchronous submission must reject this variant.
    PlayAsset {
        channel: u32,
        source: AudioAsset,
        intro: Option<AudioAsset>,
        parameters: AudioParameters,
    },
    Play {
        channel: u32,
        source: EncodedAudio,
        intro: Option<EncodedAudio>,
        parameters: AudioParameters,
    },
    Stop {
        channel: u32,
        fade_ms: f32,
    },
    /// Park/re-enable input and per-voice DSP without reopening a decoder,
    /// resetting gain or cancelling a seek. Independent gain/pan clocks keep
    /// running unless fully paused. Ended sources must seek before activation.
    SetTransportActive {
        channel: u32,
        active: bool,
    },
    Pause {
        channel: u32,
        paused: bool,
        fade_ms: f32,
    },
    /// Freeze the source now, then resume after an output-clock delay with a
    /// fresh fade-in. A later pause/stop/resume replaces this scheduled action.
    Resume {
        channel: u32,
        delay_ms: f32,
        fade_ms: f32,
    },
    /// Resume at an output-clock deadline with explicit pre-deadline transport.
    /// Later pause/stop/resume replaces this action; readiness is independent.
    ResumeWithOptions {
        channel: u32,
        delay_ms: f32,
        fade_ms: f32,
        settings: AudioResumeSettings,
    },
    /// Change the source playback multiplier without seeking, restarting or
    ///resetting resampler/DSP history. Pitch, looping and transport survive.
    SetRate {channel:u32,rate:f32},
    /// Replace a live voice's bounded stereo delay. None removes the effect.
    /// A missing voice is unchanged; no source is opened by this control.
    SetEcho {channel:u32,effect:Option<StereoEchoSpec>},
    /// Replace a live voice's portable chorus/reverb; None clears it.
    SetStereoEffect { channel: u32, effect: Option<StereoEffectSpec> },
    SetGain {
        channel: u32,
        gain: f32,
        fade_ms: f32,
    },
    /// Fade the own amplitude to a target, optionally stop when that ramp
    /// finishes. Unlike Stop this adds no independent envelope toward zero.
    FadeGain {
        channel: u32,
        gain: f32,
        fade_ms: f32,
        stop_after: bool,
    },
    /// Finish the current own-amplitude ramp at its retained target. A pending
    /// gain-stop retires the voice; independent transport deadlines stay intact.
    FinishGainFade {
        channel: u32,
    },
    SetPan {
        channel: u32,
        pan: f32,
        fade_ms: f32,
    },
    /// Independent post-processing left/right amplitude controls, each0..4.
    /// Unity preserves the current pan/spatial law. This is not equal-power pan.
    SetStereoGain {
        channel: u32,
        gains: [f32; 2],
        fade_ms: f32,
    },
    /// Replace the decoded-frame loop range and remaining-pass schedule while
    /// retaining the current fractional cursor. None passes repeats unbounded;
    /// Some must be positive, including the current pass. None end uses EOF.
    SetLoopSchedule {
        channel: u32,
        start: u64,
        end: Option<u64>,
        passes: Option<u32>,
    },
    /// Retarget track gains of a multi-track voice from its current source
    /// frame, or (with `positions`) at the first listed source frame reached;
    /// a new deferred request replaces an unapplied one.
    SetTrackGains {
        channel: u32,
        positions: Option<alloc::vec::Vec<u64>>,
        changes: alloc::vec::Vec<AudioTrackGain>,
    },
    /// Exchange decoded left/right channels without restarting transport.
    SetChannelSwap {
        channel: u32,
        swapped: bool,
    },
    /// Change loop mode at the current source position; never restart/seek.
    /// Enabling restores unbounded repeats, clearing a prior counted schedule.
    SetLooped {
        channel: u32,
        looping: bool,
        loop_start: Option<u64>,
    },
}

pub trait FileSystem {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> Result<Vec<u8>>;
    /// Either the complete new value is visible or the previous value remains.
    fn write_atomic(&mut self, client: ClientId, path: &VfsPath, data: &[u8]) -> Result<()>;
    /// Remove one client-owned writable file. A missing file is not an error;
    /// the asset mount is read-only. Hosts without deletion report Unsupported.
    fn remove(&mut self, _client: ClientId, _path: &VfsPath) -> Result<()> {
        Err(crate::Error::Unsupported("client file removal"))
    }
}

/// Frame selection for indexed movies using a native millisecond clock.
/// Half-rate modes select even indices only for 30 Hz source material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexedMoviePacing {
    NativeRate,
    HalfRate30,
    /// Drop an extra frame after a decode over 30 ms, then select even
    /// indices after 30 observations of lateness.
    Adaptive30,
}

/// Engine media resources, independent of a script VM or a window.
#[derive(Clone,Copy,Debug)]
pub struct MediaSourceOptions {
    pub looping:bool,
    pub audio:bool,
    pub gain:f32,
}

/// Engine media resources, independent of a script VM or a window.
#[derive(Clone, Debug)]
pub enum MediaRequest {
    Play {
        id: u32,
        asset: String,
        looping: bool,
        audio: bool,
        gain: f32,
    },
    /// Decode colour in the top half and a green-channel alpha plane in
    /// the bottom half of each picture. Geometry is half the encoded height.
    /// The ordinary Play contract and its default opaque frames are unchanged.
    PlayAlpha {
        id: u32,
        asset: String,
        looping: bool,
        audio: bool,
        gain: f32,
    },
    Stop {
        id: u32,
    },
    /// Change retained/live audio amplitude without restarting a decoder,
    /// changing presentation time or acquiring a new voice. Finite 0..4.
    SetGain {
        id: u32,
        gain: f32,
    },
    /// Stop workers/playback and release this scoped media resource and surface.
    /// Unlike Stop, later queries report the handle as absent.
    Remove {
        id: u32,
    },
    /// Select/sample an external monotonic video clock before first playback.
    /// The engine retains the presentation origin, pause/seek/loop offsets.
    SetClock {
        id: u32,
        now_ms: u64,
    },
    /// Freeze the media clock (audio stops; the current picture stays).
    Pause {
        id: u32,
    },
    /// Continue a paused clock from where it stopped.
    Resume {
        id: u32,
    },
    /// Restart decoding and present the picture at `position_ms` on the
    /// stream's own timeline; a paused resource stays paused there.
    Seek {
        id: u32,
        position_ms: u64,
    },
}
impl MediaRequest {
    pub fn id(&self) -> u32 {
        match self {
            Self::Play { id, .. }
            | Self::PlayAlpha { id, .. }
            | Self::SetGain { id, .. }
            | Self::Stop { id }
            | Self::Remove { id }
            | Self::SetClock { id, .. }
            | Self::Pause { id }
            | Self::Resume { id }
            | Self::Seek { id, .. } => *id,
        }
    }
}
/// A presenting host's display in physical pixel units: the whole screen and
/// the usable work area (`[x, y, w, h]`, excluding OS bars). An explicitly
/// virtual presenter may publish its own display model in the same units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub screen: [u32; 2],
    pub work: [i32; 4],
}
/// Cached facts about the current runtime host, independent of legacy VM
/// platform claims. No executable, driver or system-library probe is implied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInformation {
    pub os_name: String,
    pub architecture: String,
    /// Host-admitted parallelism; absent when the platform cannot report it.
    pub logical_cpu_count: Option<u32>,
    /// User interface locale as a BCP 47-style tag ("ja-JP"), when known.
    pub locale: Option<String>,
}
/// Geometry in physical pixel units reported by the presenting window owner,
/// including an explicitly virtual offscreen window model when installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    pub inner_size: [u32; 2],
    pub outer_size: [u32; 2],
    pub outer_position: Option<[i32; 2]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaPhase {
    Loading,
    Playing,
    Ended,
    Stopped,
    Failed,
}
/// Position of a movie's decoded PCM source, independent of the output device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAudioClock {
    NoTrack,
    Inactive,
    Playing(u64),
}

#[derive(Clone, Copy, Debug)]
pub struct MediaInfo {
    pub phase: MediaPhase,
    pub position_ms: u64,
    pub published_frames: u64,
    /// The clock is paused (`MediaRequest::Pause`); phase is unchanged.
    pub paused: bool,
    /// Picture duration the decoder reported (0 before it is known).
    pub frame_ms: u64,
    /// Validated display dimensions after decoder-header preparation.
    /// None means metadata is pending, not a zero-sized picture.
    pub size: Option<[u32; 2]>,
    /// Header-declared duration in milliseconds, when the container provides it.
    /// None remains unknown; decoded/published frame counts are not a duration.
    pub duration_ms: Option<u64>,
}
/// Immutable decoded media picture. Clones retain its engine storage lease;
/// no device pointer or native VM state is exposed.
#[derive(Clone)]
pub struct MediaFrame {
    size: [u32; 2],
    rgba: Arc<[u8]>,
    pub revision: u64,
    pub pts_ms: i64,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl MediaFrame {
    /// Retain this completed picture as a client-scoped immutable image without
    /// pixel copies or encoding. The image keeps the decoder storage lease and
    /// can be published for scene snapshots after playback is stopped.
    pub fn as_image(&self,client:ClientId)->Arc<dyn crate::image_processing::ImageResource> {
        Arc::new(MediaPictureImage{client,frame:self.clone()})
    }
    pub fn new(size: [u32; 2], rgba: Vec<u8>, revision: u64, pts_ms: i64) -> Result<Self> {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        if size.iter().any(|&n| n == 0 || n > 8192)
            || bytes > 64 * 1024 * 1024
            || bytes != rgba.len() as u64
        {
            return Err(crate::Error::Invalid("media picture dimensions/bytes"));
        }
        Ok(Self {
            size,
            rgba: rgba.into(),
            revision,
            pts_ms,
            storage: None,
        })
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
    pub fn shared_rgba(&self) -> Arc<[u8]> {
        self.rgba.clone()
    }
    pub fn retain_storage(&mut self, lease: Arc<dyn Send + Sync>) {
        self.storage = Some(lease);
    }
}
struct MediaPictureImage {client:ClientId,frame:MediaFrame}
impl core::fmt::Debug for MediaPictureImage {
    fn fmt(&self,f:&mut core::fmt::Formatter<'_>)->core::fmt::Result {
        f.debug_struct("MediaPictureImage").field("client",&self.client).field("size",&self.frame.size()).field("pts_ms",&self.frame.pts_ms).finish()
    }
}
impl crate::image_processing::ImageResource for MediaPictureImage {
    fn client(&self)->ClientId {self.client}
    fn size(&self)->[u32;2] {self.frame.size()}
    fn rgba(&self)->Arc<[u8]> {self.frame.shared_rgba()}
    // Conservative coverage metadata avoids scanning a completed movie frame
    // on the owner thread. Actual stored alpha is retained byte-for-byte.
    fn opaque(&self)->bool {false}
    fn pixel_at(&self,point:[u32;2])->Result<Option<[u8;4]>> {
        crate::image_processing::rgba_pixel_at(self.frame.size(),self.frame.rgba(),point)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub advance: f32,
}
#[derive(Clone, Copy, Debug)]
pub enum EngineQuery {
    /// Poll one engine-routed input lease. Repeated polls retain its receipt.
    InputRouteDelivery,
    InputRouteReceiver(u32),
    InputRouteStats,
    Layer(u32),
    LayerCrossfade(u32),
    /// Owned layer motion tracks and their engine-sampled values.
    LayerMotion(u32),
    /// Retained incremental queues, exact owner generation/revision and channels.
    LayerSequence(u32),
    /// Read-only next update for scheduling dependencies before committing an
    /// owner tick. Uses the same engine queue algorithm as actual advancement.
    LayerSequencePreview {id:u32,rate:f32,fast:bool},
    Canvas(u32),
    /// Exact stored RGBA sample of an owned canvas. Unsupported paint profiles
    /// fail explicitly; no device readback or default matte is implied.
    CanvasPixel {
        id: u32,
        point: [u32; 2],
    },
    UiWidget(u32),
    /// Owner-scoped retained button pointer/focus/press state.
    UiInteraction(u32),
    /// Retained last text-control action for one owned widget; does not drain UI events.
    UiTextAction(u32),
    /// Whether an owned interactive widget is enabled, visible and admitted
    /// by the current topmost modal subtree (independent of focus/hover).
    UiInputEligible(u32),
    /// Owned widget motion tracks; external elapsed values use caller units.
    UiMotion(u32),
    /// Freeze this client's current engine UI into reusable logical LayerSpec
    /// records. None selects all owned UI; a root selects its widget subtree.
    UiSnapshot {
        canvas: [u32; 2],
        root: Option<u32>,
    },
    /// Query completed worker state for this owner's VFX entity/revision.
    /// A declaration or admitted task is not a completion receipt. Results are
    /// pending until the requested tick and RNG exchange have completed.
    VfxSequence {
        entity: u32,
        revision: u64,
        completed_tick: u64,
        exchange_epoch: u64,
    },
    /// Consume only this client's retained events within an owned widget subtree.
    UiSubtreeEvents(u32),
    UiEvents,
    MenuEvents,
    /// Only `dialog_result` events (modal message dialog choices).
    DialogEvents,
}

/// Retained completion of an accepted worker-backed atomic write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageWriteState {
    Pending,
    Ready,
    Failed(String),
}
/// Owner-polled acknowledgement of an atomic capability write. Dropping it
/// requests cancellation; a commit already in progress cannot be revoked.
pub trait StorageWrite: Send {
    fn poll(&mut self) -> Result<StorageWriteState>;
}

/// A thread-safe capability handle to one client's namespaces, for engines
/// that read synchronously from their own threads or stream large packs
/// (ADR 0008). It carries no authority beyond that client's asset (read-only,
/// including provider mounts), save and cache roots.
/// Capability-scoped metadata. Missing timestamps mean unavailable, never epoch zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageMetadata {
    pub is_directory: bool,
    pub size: Option<u64>,
    pub created_ms: Option<i64>,
    pub modified_ms: Option<i64>,
    pub accessed_ms: Option<i64>,
}

pub trait ClientStorage: Send + Sync {
    /// Install a scoped, read-only asset-prefix rewrite. The first rule for a
    /// prefix stands until erased; member substitution uses literal strings.
    /// Open streams keep their original resource even after a rule changes.
    fn insert_asset_redirect(&self, _source: &str, _target: &str, _find: &str, _replace: &str) -> Result<()> {
        Err(crate::Error::Unsupported("asset redirects"))
    }
    fn erase_asset_redirect(&self, _source: &str) -> Result<()> {
        Err(crate::Error::Unsupported("asset redirects"))
    }
    fn clear_asset_redirects(&self) -> Result<()> {
        Err(crate::Error::Unsupported("asset redirects"))
    }
    /// Retain frozen asset bytes for subsequent scoped reads. No I/O occurs
    /// here; callers prepare on workers and publish after cancellation checks.
    /// Admission is atomic and may not evict other explicitly retained files.
    fn retain_asset_bytes(&self, _path: &VfsPath, _bytes: Arc<[u8]>, _max_resident: u64) -> Result<()> {
        Err(crate::Error::Unsupported("retained asset bytes"))
    }
    /// Remove one retained asset, or all when None. Existing readers keep their
    /// immutable leases; releasing a cache entry does not invalidate a consumer.
    fn release_asset_bytes(&self, _path: Option<&VfsPath>) -> Result<()> {
        Err(crate::Error::Unsupported("retained asset bytes"))
    }
    /// Read metadata for a canonical mount-relative name; empty names denote
    /// the mount root. Queries must not open ambient paths or decode resources.
    fn metadata(
        &self,
        _mount: crate::path::Mount,
        _relative: &str,
    ) -> Result<Option<StorageMetadata>> {
        Err(crate::Error::Unsupported("capability storage metadata"))
    }

    /// Physical read-only permission, independent of a capability's access mode.
    /// Empty canonical names refer to the selected capability root.
    fn read_only(&self, _mount: crate::path::Mount, _relative: &str) -> Result<bool> {
        Err(crate::Error::Unsupported(
            "storage physical read-only permission",
        ))
    }

    /// Copy a loose regular file and its permissions to an existing directory.
    /// Preserve its modification time when the filesystem permits. Providers
    /// unable to copy metadata reject rather than falling back to byte upload.
    /// Clearing an existing target's read-only permission is explicit policy.
    fn copy_file(
        &self,
        _source: &VfsPath,
        _target: &VfsPath,
        _replace_read_only: bool,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("storage file copy with metadata"))
    }

    /// Reserve worker capacity before moving immutable bytes. Ready means the
    /// atomic commit completed; unsupported providers reject before admission.
    fn begin_write_atomic(
        &self,
        _path: &VfsPath,
        _data: Arc<[u8]>,
    ) -> Result<alloc::boxed::Box<dyn StorageWrite>> {
        Err(crate::Error::Unsupported("worker atomic storage write"))
    }
    fn read(&self, path: &VfsPath) -> Result<Vec<u8>>;
    /// Open only the asset namespace. Default providers use bounded memory;
    /// filesystem hosts override this to stream without reading the full pack.
    fn open_asset(&self, path: &VfsPath) -> Result<Arc<dyn AssetReader>> {
        if path.mount() != crate::path::Mount::Asset {
            return Err(crate::Error::ReadOnly);
        }
        let bytes = self.read(path)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(crate::Error::Limit("in-memory asset stream"));
        }
        Ok(Arc::new(MemoryAssetReader(Arc::from(bytes))))
    }
    /// Read-only capability stream with no whole-file fallback. Providers must
    /// explicitly support opening before the consumer admits its byte budget.
    fn open_read(&self, _path: &VfsPath) -> Result<Arc<dyn AssetReader>> {
        Err(crate::Error::Unsupported("capability read stream"))
    }
    /// Find a previously mounted container within this client's asset namespace.
    /// This metadata query does not open files or grant additional authority.
    fn container_mount(&self, _source: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// True when `path` resolves to a mounted container provider (an archive
    /// whose contents cannot change while mounted), not to a plain file.
    fn served_by_container(&self, _path: &VfsPath) -> bool {
        false
    }
    /// The mounted container serving `path`, as `(container source, mount
    /// point)` in the client's asset namespace; `None` for plain files.
    fn serving_container(&self, _path: &VfsPath) -> Option<(String, String)> {
        None
    }
    /// Optional exclusive lease inside this client's cache namespace. Used by
    /// native save implementations that cannot merge concurrent writers.
    fn acquire_lease(&self, _name: &str) -> Result<alloc::boxed::Box<dyn StorageLease>> {
        Err(crate::Error::Unsupported("storage lease"))
    }
    fn write_atomic(&self, path: &VfsPath, data: &[u8]) -> Result<()>;
    fn remove(&self, path: &VfsPath) -> Result<()>;
    /// Create exactly one directory level in a writable namespace. Existing
    /// directories fail; implicit parent creation is an atomic-write policy.
    /// Move a file/directory atomically without replacing an existing target.
    /// Both names are capability paths; unsupported hosts reject admission.
    fn rename_no_replace(&self,_source:&VfsPath,_target:&VfsPath)->Result<()>{
        Err(crate::Error::Unsupported("atomic no-replace rename"))
    }
    /// Atomic replacement which requires the destination's parent to exist.
    fn write_atomic_existing_parent(&self, _path: &VfsPath, _data: &[u8]) -> Result<()> {
        Err(crate::Error::Unsupported(
            "atomic write requiring an existing parent",
        ))
    }
    fn create_directory(&self, _path: &VfsPath) -> Result<()> {
        Err(crate::Error::Unsupported("storage directory creation"))
    }
    /// Remove an empty directory only; never a recursive deletion.
    fn remove_directory(&self, _path: &VfsPath) -> Result<()> {
        Err(crate::Error::Unsupported("storage empty-directory removal"))
    }

    /// Names directly inside `dir` ("" is the mount root), sorted.
    fn list(&self, mount: crate::path::Mount, dir: &str) -> Result<Vec<String>>;
    fn exists(&self, path: &VfsPath) -> bool {
        self.read(path).is_ok()
    }
}

/// Thread-safe, long-lived handle to the shared image decoders for one
/// client's assets. Native VMs whose original API answers image metadata
/// synchronously (for example a layer's image size right after a load) hold
/// it between steps instead of borrowing the host or decoding privately.
/// Result of [`HostServices::upload_encoded_image`].
#[derive(Clone, Debug)]
pub struct UploadedImage {
    pub path: VfsPath,
    pub size: [u32; 2],
    /// True only when every decoded texel has alpha 255.
    pub opaque: bool,
}

pub trait ImageMetadata: Send + Sync {
    /// Decode an integer-label image on workers. Source palette indices are
    /// preserved independently of display colours/alpha. A minimum extent
    /// repeats smaller source axes; larger decoded axes are retained.
    fn begin_region_map(
        &self,
        _path: &VfsPath,
        _minimum_size: Option<[u32; 2]>,
    ) -> Result<alloc::boxed::Box<dyn crate::region_map::RegionMapQuery>> {
        Err(crate::Error::Unsupported("integer label-image resources"))
    }
    fn begin_packed_pixels(
        &self,
        _spec: crate::image_processing::PackedImageSpec,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported(
            "packed image resource preparation",
        ))
    }

    fn begin_bitmap_text(
        &self,
        _destination: crate::image_processing::ImageInput,
        _spec: crate::image_processing::BitmapTextSpec,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("integer bitmap text records"))
    }

    /// Decode bounded immutable encoded bytes on engine workers. Dimensions
    /// are learned by the shared registry, without an adapter-side decoder.
    /// Rasterise reusable vector groups/brushes/masks/filters against frozen
    /// client-owned image resources, without a compatibility VM or device.
    fn begin_vector_scene(&self,_spec:crate::vector_scene::VectorSceneSpec)->Result<crate::image_processing::ImageJob>{
        Err(crate::Error::Unsupported("vector scene raster job"))
    }
    /// Decode one encoded delta frame over a frozen client-owned image.
    fn begin_encoded_frame(
        &self,
        _spec: crate::frame_image::EncodedFrameSpec,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("encoded frame image job"))
    }
    fn begin_encoded(
        &self,
        _name: &str,
        _bytes: Arc<[u8]>,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported(
            "encoded image resource preparation",
        ))
    }
    /// Freeze an authorized font and prepare regular positioned coverage using
    /// the engine text rasterizer. Polling is nonblocking; dropping the job
    /// cancels preparation. The result is not implicitly published to a catalog.
    fn begin_glyph_image(
        &self,
        _spec: crate::image_processing::GlyphImageSpec,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("worker glyph image preparation"))
    }
    /// Freeze the same scoped face and prepare an explicit styled/coverage
    /// pipeline. This is not an implicit claim of a native platform rasterizer.
    fn begin_glyph_image_batch(
        &self,
        _spec: crate::image_processing::GlyphImageBatchSpec,
    ) -> Result<alloc::boxed::Box<dyn crate::image_processing::GlyphImageBatchQuery>> {
        Err(crate::Error::Unsupported("batched glyph image preparation"))
    }
    fn begin_styled_glyph_image(
        &self,
        _spec: crate::image_processing::StyledGlyphImageSpec,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported(
            "worker styled glyph image preparation",
        ))
    }
    /// Original encoded properties from the same shared decoder selection.
    /// Absence is Unsupported, never inferred from normalized pixel opacity.
    fn begin_encoding_info(
        &self,
        _asset: &str,
    ) -> Result<alloc::boxed::Box<dyn crate::image_encoding::ImageEncodingInfoQuery>> {
        Err(crate::Error::Unsupported("encoded image metadata"))
    }
    /// Read original image properties through an explicit client capability
    /// mount. Non-Asset mounts require a provider implementation.
    fn begin_scoped_encoding_info(
        &self,
        path: &VfsPath,
    ) -> Result<alloc::boxed::Box<dyn crate::image_encoding::ImageEncodingInfoQuery>> {
        if path.mount() == crate::path::Mount::Asset {
            self.begin_encoding_info(path.relative())
        } else {
            Err(crate::Error::Unsupported("scoped encoded image metadata"))
        }
    }
    /// Encoded content identity observed by the metadata decode. Native retained
    /// image processing rejects a changed file rather than transforming new bytes.
    fn fingerprint(&self, _asset: &str) -> Result<[u8; 32]> {
        Err(crate::Error::Unsupported("image content identity"))
    }

    /// Shared image operations encoded as a bounded ImageEffect array. A
    /// provider reserves worker/memory capacity before accepting the input.
    fn begin_process(
        &self,
        _source: crate::image_processing::ImageInput,
        _effects: Arc<[u8]>,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("worker image processing"))
    }

    /// Poll bounded preparation for a resumable client. Once ready, size,
    /// opacity and fingerprint can be read as one decoded metadata record.
    /// Synchronous-only providers retain their normal size lookup.
    fn prepare_metadata(&self, _asset: &str) -> Result<bool> { Ok(true) }
    fn size(&self, asset: &str) -> Result<[u32; 2]>;
    /// True only when every decoded texel has alpha 255.
    fn opaque(&self, _asset: &str) -> Result<bool> {
        Err(crate::Error::Unsupported("image alpha metadata"))
    }
}

/// Logical extents of the engine's simple horizontal text layout, before
/// wrapping or clipping. Advance includes kerning; height is one line pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextExtent {
    pub advance: f32,
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// One Unicode scalar at an unwrapped floating-point baseline. Synthetic bold
/// does not change this simple layout's advance; shaping has a separate API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimpleTextPlacement {
    pub character: char,
    pub baseline: [f32; 2],
}

/// Immutable CPU font-resource snapshot. Native script callbacks can measure
/// synchronously on any thread without borrowing the owner host or a renderer.
/// This is the simple-horizontal-v1 layout, not a native layout substitution.
pub trait TextMeasurement: Send + Sync {
    /// Font ink bounds for the shared simple layout; no I/O or pixel raster is
    /// performed. Decorations use the selected outline's design metrics.
    fn ink_bounds_simple(&self,_font:&str,_size:f32,_text:&str,_bold:bool,_underline:bool,_strikeout:bool)->Result<[i32;4]>{
        Err(crate::Error::Unsupported("simple text ink bounds"))
    }
    fn measure_simple(&self, font: &str, size: f32, text: &str) -> Result<TextExtent>;
    fn position_simple(
        &self,
        _font: &str,
        _size: u16,
        _text: &str,
    ) -> Result<Vec<SimpleTextPlacement>> {
        Err(crate::Error::Unsupported("simple text placement"))
    }
}

/// Mapping from logical canvas pixels to the host presentation surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationScale {
    #[default]
    Fit,
    Stretch,
    /// Native pixel size centered in the surface; fit if it would be cropped.
    Center,
}

pub trait HostServices: FileSystem {
    fn open_text_input_session(&mut self,_client:ClientId,_spec:TextInputSessionSpec)->Result<TextInputSessionHandle>{Err(crate::Error::Unsupported("text input presenter"))}
    fn configure_text_input_session(&mut self,_client:ClientId,_handle:TextInputSessionHandle,_spec:TextInputSessionSpec)->Result<()>{Err(crate::Error::Unsupported("text input presenter"))}
    fn text_input_session_info(&self,_client:ClientId,_handle:TextInputSessionHandle)->Result<TextInputSessionInfo>{Err(crate::Error::Unsupported("text input presenter"))}
    fn take_text_input_committed(&mut self,_client:ClientId,_handle:TextInputSessionHandle)->Result<Arc<String>>{Err(crate::Error::Unsupported("text input presenter"))}
    fn replace_text_input_committed(&mut self,_client:ClientId,_handle:TextInputSessionHandle,_text:String)->Result<()>{Err(crate::Error::Unsupported("text input presenter"))}
    fn release_text_input_session(&mut self,_client:ClientId,_handle:TextInputSessionHandle)->Result<()>{Err(crate::Error::Unsupported("text input presenter"))}
    fn host_information(&self) -> Result<HostInformation> {
        Err(crate::Error::Unsupported("host information"))
    }
    /// Prepare bounded seed words for a client's game random streams. Sampling
    /// is an explicit worker dependency; no ambient/simulation clock fallback.
    fn begin_session_seed(
        &mut self,
        _client: ClientId,
        _source: SessionSeedSource,
    ) -> Result<alloc::boxed::Box<dyn SessionSeedQuery>> {
        Err(crate::Error::Unsupported("session seed material"))
    }
    fn begin_media_probe(&mut self, _client: ClientId, _asset: &VfsPath) -> Result<MediaProbeJob> {
        Err(crate::Error::Unsupported("media decoder header probe"))
    }
    /// Freeze a client Canvas and asynchronously prepare an immutable engine
    /// image. Requires an installed presenter; polling never waits for a GPU.
    fn begin_canvas_image(
        &mut self,
        _client: ClientId,
        _canvas: u32,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("Canvas image capture presenter"))
    }
    /// Freeze a source-local [x,y,width,height] region of a retained Canvas.
    /// Region extents must lie inside the source; large logical surfaces need
    /// only admit the requested bounded pixel region to a presenter.
    fn begin_canvas_region_image(
        &mut self,
        _client: ClientId,
        _canvas: u32,
        _region: [u32; 4],
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported(
            "Canvas region image capture presenter",
        ))
    }
    /// Asynchronously capture the frame the presenter would show now, with every
    /// blend, movie and transition as displayed. Requires an installed presenter.
    /// Freeze only the owned Layer painter prefix below an exclusive order.
    /// The presenter rasterizes shared scene resources; no adapter pixels or
    /// later display payloads enter the result.
    fn begin_layer_prefix_image(
        &mut self,
        _client: ClientId,
        _size: [u32; 2],
        _order_exclusive: [i32; 2],
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("layer prefix image capture"))
    }
    /// Rasterize a detached candidate with the shared Layer presenter. Commands
    /// are a JSON array of declarative Layer put/remove operations, starting
    /// from an empty scoped scene; all referenced ancestors must be supplied.
    /// The exclusive order freezes glyph reveal and resource generations just
    /// like prefix capture. No live Layer/UI/input state is changed. Requires
    /// the same installed presenter, image memory and cancellation budget.
    fn begin_detached_layers_image(
        &mut self,
        _client: ClientId,
        _size: [u32; 2],
        _order_exclusive: [i32; 2],
        _commands: &[u8],
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("detached Layer image capture"))
    }
    fn begin_layer_prefix_image_over(
        &mut self,
        client: ClientId,
        size: [u32; 2],
        order_exclusive: [i32; 2],
        clear: [u8; 4],
    ) -> Result<crate::image_processing::ImageJob> {
        if clear == [0; 4] {
            self.begin_layer_prefix_image(client, size, order_exclusive)
        } else {
            Err(crate::Error::Unsupported(
                "layer prefix capture clear colour",
            ))
        }
    }

    fn begin_frame_capture(
        &mut self,
        _client: ClientId,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("frame capture presenter"))
    }
    /// Freeze an owned persistent Layer target at the next presenter fence.
    /// The job owns bounded staging/output admission; deletion or replacement
    /// cancels its generation. Returned RGBA contains bitmap pixels before the
    /// presentation layer's transform, opacity or surrounding scene.
    fn begin_layer_target_image(
        &mut self,
        _client: ClientId,
        _layer: u32,
    ) -> Result<crate::image_processing::ImageJob> {
        Err(crate::Error::Unsupported("persistent Layer target readback"))
    }
    fn begin_pattern(&mut self, _client: ClientId, _request: PatternRequest) -> Result<PatternJob> {
        Err(crate::Error::Unsupported("persistent pattern worker"))
    }

    /// Atomically upsert explicit decoded-frame metadata; duplicate batch keys fail.
    fn publish_audio_loop_metadata(
        &mut self,
        _client: ClientId,
        _entries: &[AudioLoopMetadata],
    ) -> Result<AudioLoopCatalogReceipt> {
        Err(crate::Error::Unsupported("audio loop catalog"))
    }
    fn audio_loop_metadata(
        &self,
        _client: ClientId,
        _asset: &str,
    ) -> Result<Option<AudioLoopMetadata>> {
        Err(crate::Error::Unsupported("audio loop catalog"))
    }
    fn clear_audio_loop_metadata(&mut self, _client: ClientId) -> Result<AudioLoopCatalogReceipt> {
        Err(crate::Error::Unsupported("audio loop catalog"))
    }

    /// Sample host local time on a worker; Ready carries its actual Unix instant
    /// and local offset. Does not substitute the deterministic engine clock.
    fn begin_civil_time(
        &mut self,
        _client: ClientId,
    ) -> Result<alloc::boxed::Box<dyn CivilTimeQuery>> {
        Err(crate::Error::Unsupported("host civil clock"))
    }

    /// Resolve midnight of an already normalized Gregorian date in the host
    /// local timezone on a worker. Ready includes its historical UTC offset
    /// and Unix instant. An ambiguous or nonexistent midnight fails explicitly;
    /// no current-offset guess or implicit DST normalization is applied.
    fn begin_civil_date(
        &mut self,
        _client: ClientId,
        _year: i32,
        _month: u8,
        _day: u8,
    ) -> Result<alloc::boxed::Box<dyn CivilTimeQuery>> {
        Err(crate::Error::Unsupported("host local civil date"))
    }

    /// Capture the host's published recurring timezone policy on a worker.
    /// This is distinct from historical instant/date resolution: callers may
    /// retain the profile for a process-style CRT timezone cache. Unsupported
    /// platform metadata or unrepresentable rules fail explicitly.
    fn begin_civil_timezone(
        &mut self,
        _client: ClientId,
    ) -> Result<alloc::boxed::Box<dyn CivilTimeZoneQuery>> {
        Err(crate::Error::Unsupported("host civil timezone policy"))
    }

    /// Observe an exact-origin grant installed by the host owner. VM/project
    /// data cannot install grants. The request still requires separate admission.
    fn http_origin_allowed(&self, _client: ClientId, _origin: &str) -> Result<bool> {
        Err(crate::Error::Unsupported("authorized HTTP transport"))
    }
    /// Begin bounded HTTP I/O on a shared worker. Redirects must remain on the
    /// original authorized origin; completion belongs exclusively to this job.
    fn begin_http(&mut self, _client: ClientId, _request: HttpRequest)
        -> Result<alloc::boxed::Box<dyn HttpRequestJob>> {
        Err(crate::Error::Unsupported("authorized HTTP transport"))
    }

    /// Append to the canonical engine log at its exclusive owner boundary.
    /// The returned receipt means committed, not just queued. No filesystem sink.
    fn append_log(&mut self, _client: ClientId, _message: &str) -> Result<LogReceipt> {
        Err(crate::Error::Unsupported("engine log"))
    }
    fn log_snapshot(&self, _client: ClientId) -> Result<LogSnapshot> {
        Err(crate::Error::Unsupported("engine log"))
    }
    /// Read client logical locks and explicit physical capability availability.
    fn keyboard_locks(&self, _client: ClientId) -> Result<KeyboardLockSnapshot> {
        Err(crate::Error::Unsupported("keyboard locks"))
    }
    /// Commit logical state only; must not toggle an ambient OS keyboard.
    fn set_keyboard_locks(
        &mut self,
        _client: ClientId,
        _state: KeyboardLockState,
    ) -> Result<KeyboardLockSnapshot> {
        Err(crate::Error::Unsupported("keyboard locks"))
    }

    /// Admit a client-owned live-voice retirement and external-clock release.
    fn begin_audio_release(
        &mut self,
        _client: ClientId,
        _spec: AudioReleaseSpec,
    ) -> Result<AudioReleaseHandle> {
        Err(crate::Error::Unsupported("audio voice release"))
    }
    /// Submit a monotonic caller update; completion is observed separately.
    fn advance_audio_release(
        &mut self,
        _client: ClientId,
        _handle: AudioReleaseHandle,
        _elapsed_updates: u64,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("audio voice release"))
    }
    /// Poll the actual owner boundary, never infer completion from admission.
    fn audio_release_info(
        &mut self,
        _client: ClientId,
        _handle: AudioReleaseHandle,
    ) -> Result<AudioReleaseInfo> {
        Err(crate::Error::Unsupported("audio voice release"))
    }
    /// Release a terminal receipt. Active producer state cannot be discarded.
    fn release_audio_release(
        &mut self,
        _client: ClientId,
        _handle: AudioReleaseHandle,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("audio voice release"))
    }

    /// Availability of a canonical codec in the host's actual decoder registry.
    /// This does not promise support for every channel mapping or metadata mode.
    fn audio_codec_supported(&self, _client: ClientId, _codec: &str) -> Result<bool> {
        Ok(false)
    }
    fn mesh_frame_history(&self, _client: ClientId) -> Result<Option<Arc<dyn MeshFrameHistory>>> {
        Ok(None)
    }
    fn capture_audio_sequence(
        &self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
    ) -> Result<AudioSequenceCursor> {
        Err(crate::Error::Unsupported("audio cue cursor capture"))
    }
    fn restore_audio_sequence(
        &mut self,
        _client: ClientId,
        _spec: AudioSequenceSpec,
        _cursor: AudioSequenceCursor,
    ) -> Result<AudioSequenceHandle> {
        Err(crate::Error::Unsupported("audio cue cursor restore"))
    }
    /// Create an owner-scoped cue scheduler. It opens no assets or voices.
    fn create_audio_sequence(
        &mut self,
        _client: ClientId,
        _spec: AudioSequenceSpec,
        _seed: u32,
    ) -> Result<AudioSequenceHandle> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    /// Exactly one owner update supplies both clocks. Pending dependency work
    /// freezes the sequence; polling alone never advances time or a frame budget.
    fn advance_audio_sequence(
        &mut self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
        _game_ms: u64,
        _real_ms: u64,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    /// Return the same pending emission until acknowledged. Optional seed is
    /// installed only when selecting a new due cue, for native shared RNGs.
    fn poll_audio_sequence(
        &mut self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
        _seed: Option<u32>,
    ) -> Result<Option<AudioSequenceEmission>> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    fn complete_audio_sequence(
        &mut self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
        _serial: u64,
        _duration_ms: u64,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    fn audio_sequence_info(
        &self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
    ) -> Result<AudioSequenceInfo> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    /// Cancel scheduling only. Previously emitted voices remain caller-owned.
    fn release_audio_sequence(
        &mut self,
        _client: ClientId,
        _handle: AudioSequenceHandle,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("audio cue sequences"))
    }
    /// Inspect a native/media mixer's client-owned logical group, if present.
    fn audio_mix_group(&self, _client: ClientId, _name: &str) -> Result<Option<AudioMixGroupInfo>> {
        Err(crate::Error::Unsupported("audio mix group inspection"))
    }

    /// Snapshot/probe a client asset on workers. Optional length measurement
    /// streams/discards decoded chunks when the container has no frame count.
    fn begin_audio_resource(
        &mut self,
        _client: ClientId,
        _asset: AudioAsset,
        _measure_frames: bool,
    ) -> Result<u64> {
        Err(crate::Error::Unsupported("prepared audio resources"))
    }
    /// Prepare a scoped immutable encoded resource from an archive member or
    /// VM memory block. Decoder work and metadata measurement stay on workers.
    fn begin_encoded_audio_resource(
        &mut self,
        _client: ClientId,
        _name: &str,
        _encoded: EncodedAudio,
        _measure_frames: bool,
    ) -> Result<u64> {
        Err(crate::Error::Unsupported(
            "encoded audio resource preparation",
        ))
    }
    /// Decode a scoped reader once into reusable PCM on an audio worker.
    /// An optional PCM16 byte limit truncates decoders without a frame count,
    /// measured in the decoder's mono/stereo layout before mixer conversion.
    fn begin_resident_audio(
        &mut self,
        _client: ClientId,
        _reader: Arc<dyn AssetReader>,
        _name: &str,
        _unknown_pcm16_bytes: Option<u64>,
    ) -> Result<ResidentAudioJob> {
        Err(crate::Error::Unsupported("resident audio preparation"))
    }
    /// Prepare a new immutable resource using a bounded CPU transform.
    fn begin_transformed_audio_resource(
        &mut self,
        _client: ClientId,
        _asset: AudioAsset,
        _transform: AudioResourceTransform,
    ) -> Result<u64> {
        Err(crate::Error::Unsupported("transformed audio resources"))
    }
    fn audio_resource_preparation(
        &mut self,
        _client: ClientId,
        _ticket: u64,
    ) -> Result<AudioResourcePreparation> {
        Err(crate::Error::Unsupported("prepared audio resource status"))
    }
    /// Cancel unpublished work or release its receipt; a published resource stays.
    fn release_audio_resource_preparation(
        &mut self,
        _client: ClientId,
        _ticket: u64,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("prepared audio resource receipt"))
    }
    /// Prepare a spectrum bitmap from an independent source decoder and an
    /// immutable destination. None is the native stopped-source silence case.
    fn begin_spectrum_graph(&mut self,_client:ClientId,_source:Option<AudioResourceHandle>,_position:u64,_destination:crate::image_processing::ImageInput,_spec:SpectrumGraphSpec,_state:SpectrumGraphState)->Result<SpectrumGraphJob>{
        Err(crate::Error::Unsupported("source audio spectrum visualization"))
    }
    /// Inspect original source-layout PCM on a worker, without seeking playback.
    fn begin_audio_samples(&mut self,_client:ClientId,_resource:AudioResourceHandle,_request:AudioSampleRequest)->Result<alloc::boxed::Box<dyn AudioSampleQuery>>{
        Err(crate::Error::Unsupported("source-layout audio samples"))
    }
    /// Read integer PCM at the play cursor for visualization. Encoded sources
    /// can be decoded at their native rate. Source-layout inspection remains
    /// available separately to callers that need the original PCM format.
    fn begin_audio_visual_samples(&mut self,client:ClientId,resource:AudioResourceHandle,request:AudioSampleRequest)->Result<alloc::boxed::Box<dyn AudioSampleQuery>>{
        self.begin_audio_samples(client,resource,request)
    }
    fn audio_resource_info(
        &self,
        _client: ClientId,
        _resource: AudioResourceHandle,
    ) -> Result<AudioResourceInfo> {
        Err(crate::Error::Unsupported(
            "prepared audio resource inspection",
        ))
    }
    /// Remove lookup authority. Already admitted/live playback keeps its lease.
    fn release_audio_resource(
        &mut self,
        _client: ClientId,
        _resource: AudioResourceHandle,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("prepared audio resource release"))
    }
    fn audio_playing(&self, _client: ClientId, _channel: u32) -> Result<bool> {
        Err(crate::Error::Unsupported("engine audio status"))
    }
    /// Current shared channel transport, or None for an ended/unallocated voice.
    fn audio_info(&self, _client: ClientId, _channel: u32) -> Result<Option<AudioPlaybackInfo>> {
        Err(crate::Error::Unsupported(
            "channel audio transport inspection",
        ))
    }
    /// Mark the currently allocated channel for retirement at the next
    /// nonempty mixer render, or cancel that mark. Returns whether its
    /// mark changed. It has no effect on a pending replacement generation.
    fn set_audio_channel_retirement(
        &mut self,
        _client: ClientId,
        _channel: u32,
        _retire: bool,
    ) -> Result<bool> {
        Err(crate::Error::Unsupported("audio channel retirement"))
    }
    /// Activate a parked or retiring retained source at its current cursor
    /// and start a new transport timing interval.
    /// An exhausted source reports active through the next nonempty mix and
    /// generates no more PCM. Returns whether an allocated slot was activated.
    fn resume_retained_audio_channel(
        &mut self,
        _client: ClientId,
        _channel: u32,
        _looping: bool,
    ) -> Result<bool> {
        Err(crate::Error::Unsupported("resume retained audio channel"))
    }
    /// Read the delayed stereo mean absolute PCM16 level at the most recent
    /// published mixer boundary. An inactive metered source reports zeroes.
    /// None means the channel lacks a live metered source.
    fn audio_source_level(&self, _client: ClientId, _channel: u32) -> Result<Option<[u32; 2]>> {
        Err(crate::Error::Unsupported("channel audio source meter"))
    }
    /// Replace this channel's gain track without moving or reopening its
    /// source. Output time advances it even through pause/decoder starvation.
    /// None cancels it at the last applied gain. An absent voice stays absent.
    fn set_audio_gain_envelope(&mut self, _client: ClientId, _channel: u32,
        _spec: Option<AudioGainEnvelopeSpec>) -> Result<()> {
        Err(crate::Error::Unsupported("stepped audio gain envelope"))
    }
    fn audio_gain_envelope_info(&self, _client: ClientId, _channel: u32)
        -> Result<Option<AudioGainEnvelopeInfo>> {
        Err(crate::Error::Unsupported("stepped audio gain envelope status"))
    }
    /// Seek an existing native/media channel without reopening its source or
    /// resetting gain, loop count, pause, fades or source identity. Completion
    /// remains pending until the real decoder cursor and owner publication.
    /// Prepare a complete client replacement from pure play recipes. An empty
    /// list means clear that client's voices at commit; this starts no playback.
    fn begin_audio_replacement(
        &mut self,
        _client: ClientId,
        _requests: Vec<AudioRequest>,
    ) -> Result<AudioReplacementJob> {
        Err(crate::Error::Unsupported(
            "detached client audio replacement",
        ))
    }
    fn begin_audio_seek(&mut self, _client: ClientId, _channel: u32, _frame: u64) -> Result<u64> {
        Err(crate::Error::Unsupported("channel audio seek"))
    }
    /// Hold a retained voice and its controls while rewinding its decoder to
    /// frame zero. The seek receipt publishes activation with the new loop
    /// mode on the same scoped voice. Cancellation releases the hold.
    fn begin_retained_audio_rewind(
        &mut self,
        _client: ClientId,
        _channel: u32,
        _looping: bool,
    ) -> Result<u64> {
        Err(crate::Error::Unsupported("retained audio rewind"))
    }
    /// Retained scoped seek receipt; Ready is distinct from later voice EOF.
    fn audio_seek_preparation(
        &mut self,
        _client: ClientId,
        _ticket: u64,
    ) -> Result<AudioPreparation> {
        Err(crate::Error::Unsupported("channel audio seek status"))
    }
    /// Cancel a pending seek/restore the previous logical position, or release
    /// a terminal receipt. A newer seek cannot be cancelled by an older ID.
    fn release_audio_seek(&mut self, _client: ClientId, _ticket: u64) -> Result<()> {
        Err(crate::Error::Unsupported("channel audio seek receipt"))
    }
    /// Capture selected logical Layer/Canvas/widget subtrees and their exact
    /// clocks. The capability pins generated image paths within this host.
    fn capture_scene_checkpoint(
        &mut self, _client:ClientId,
        _scope:&crate::scene_checkpoint::SceneCheckpointScope,
    )->Result<Arc<dyn crate::scene_checkpoint::SceneCheckpoint>> {
        Err(crate::Error::Unsupported("scoped scene checkpoint"))
    }
    /// Atomically replace only the selected owned scopes, preserving peers,
    /// host input observations and the host clock. External producers receive
    /// fresh generations and must query/rebind before publishing another sample.
    /// Removal roots/canvases must belong to the captured scope; runtime or
    /// original-save replacement invalidates this client's old capabilities.
    fn restore_scene_checkpoint(
        &mut self, _client:ClientId,
        _checkpoint:&dyn crate::scene_checkpoint::SceneCheckpoint,
        _remove:&crate::scene_checkpoint::SceneCheckpointScope,
    )->Result<()> {
        Err(crate::Error::Unsupported("scoped scene checkpoint restore"))
    }
    fn query_engine(&mut self, _client: ClientId, _query: EngineQuery) -> Result<Vec<u8>> {
        Err(crate::Error::Unsupported("engine state query"))
    }
    fn image_size(&mut self, _client: ClientId, _asset: &str) -> Result<[u32; 2]> {
        Err(crate::Error::Unsupported("engine image metadata"))
    }
    /// True only when every decoded texel has alpha 255. Uses the shared decoder.
    fn image_is_opaque(&mut self, _client: ClientId, _asset: &str) -> Result<bool> {
        Err(crate::Error::Unsupported("engine image alpha metadata"))
    }
    fn media_request(&mut self, _client: ClientId, _request: MediaRequest) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("engine media"))
    }
    /// Play an already-authorized immutable reader, including an embedded
    /// archive member, through the same decoder queues and clocks as files.
    /// Ownership stays with the media resource until Remove/replacement.
    fn media_reader(&mut self,_client:ClientId,_id:u32,_source:Arc<dyn AssetReader>,_options:MediaSourceOptions)->Result<VfsPath>{
        Err(crate::Error::Unsupported("reader-backed engine media"))
    }
    /// Start reader-backed media whose embedded audio uses a legacy PCM16
    /// fader. The fader follows the retained track through seek or restart.
    fn media_reader_with_pcm16_gain(
        &mut self,
        _client: ClientId,
        _id: u32,
        _source: Arc<dyn AssetReader>,
        _options: MediaSourceOptions,
        _gain: AudioPcm16Gain,
    ) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("reader media with PCM16 gain"))
    }
    /// Start an indexed CMV with an audio-master clock (elapsed time for a silent
    /// stream) and decode only pictures requested by the presentation owner.
    fn indexed_movie_reader(&mut self, _client: ClientId, _id: u32, _source: Arc<dyn AssetReader>,
        _options: MediaSourceOptions, _gain: AudioPcm16Gain, _pacing: IndexedMoviePacing) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("indexed movie playback"))
    }
    fn media_info(&self, _client: ClientId, _id: u32) -> Result<MediaInfo> {
        Err(crate::Error::Unsupported("engine media status"))
    }
    /// The display the presenting host shows the canvas on; `None` when the
    /// host has no display model.
    fn display_info(&self) -> Option<DisplayInfo> {
        None
    }
    /// Resize the logical render/input canvas. This is independent of physical
    /// window dimensions. Hosts publish the accepted size at their tick boundary.
    fn request_canvas_size(&mut self, _client: ClientId, _size: [u32; 2]) -> Result<()> {
        Err(crate::Error::Unsupported("logical canvas resizing"))
    }
    /// Publish decoded straight RGBA pixels for a presenting host's window icon.
    /// Virtual hosts accept the same bounded logical request.
    fn request_window_icon(
        &mut self, _client: ClientId, _size: [u32; 2], _rgba: Vec<u8>,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("native window icon"))
    }
    /// Current focus published by the physical or explicitly virtual window
    /// owner. None means no focus model, rather than an inactive window.
    fn window_focused(&self, _client: ClientId) -> Result<Option<bool>> {
        Ok(None)
    }
    fn window_geometry(&self, _client: ClientId) -> Result<Option<WindowGeometry>> {
        Ok(None)
    }
    /// Actual minimized/iconic state from the physical or virtual window owner. Unavailable is
    /// distinct from false; loss of keyboard focus is not minimization.
    fn window_minimized(&self, _client: ClientId) -> Result<Option<bool>> {
        Ok(None)
    }
    /// Maximized state of the client's presenting window, if the host models it.
    fn window_maximized(&self, _client: ClientId) -> Result<Option<bool>> { Ok(None) }
    /// Actual fullscreen state published by the physical or explicitly virtual window owner.
    /// An accepted UI request alone does not establish this observation.
    /// None means neither a physical nor virtual window observation is available.
    fn window_fullscreen(&self, _client: ClientId) -> Result<Option<bool>> {
        Ok(None)
    }
    /// Live backend pointer-motion token, independent of admitted input-frame
    /// revisions. Changes on actual movement/entry even if canvas coordinates
    /// round to the same pixel. None means the backend has no such observation.
    fn pointer_activity(&self, _client: ClientId) -> Result<Option<u64>> {
        Ok(None)
    }
    /// Move the host pointer to canvas coordinates (a native `SetCursorPos`
    /// or confined mouse area). Hosts without a pointer they may move report
    /// Unsupported; the request never fabricates an input event.
    fn warp_pointer(&mut self, _client: ClientId, _x: i32, _y: i32) -> Result<()> {
        Err(crate::Error::Unsupported("pointer warp"))
    }
    /// Offer one of the client's own read-only assets (a game manual) to the
    /// platform's document viewer. Never executes it and never accepts paths
    /// outside the client's asset mount.
    fn open_document(&mut self, _client: ClientId, _path: &crate::path::VfsPath) -> Result<()> {
        Err(crate::Error::Unsupported("open document"))
    }
    /// Last completed owner input admission. A query does not consume edges,
    /// advance input time, or turn unknown pointer/focus metadata into defaults.
    fn input_observation(&self, _client: ClientId) -> Result<InputObservation> {
        Err(crate::Error::Unsupported("input observation"))
    }
    /// Separate admitted keyboard/pointer and normalized per-device controller
    /// samples. Unavailable host observations remain Unsupported, not invented
    /// from an aggregate InputFrame. Shared modal UI masks source controls too.
    fn input_sources(&self,_client:ClientId)->Result<InputSourcesObservation>{
        Err(crate::Error::Unsupported("input source observation"))
    }

    /// Whether shared UI owns the last admitted physical input frame. This
    /// owner fact belongs to the same routing frame as `input_observation`;
    /// it does not infer a physical key-up from buttons masked by a modal UI.
    fn ui_input_owned(&self, _client: ClientId) -> Result<bool> {
        Err(crate::Error::Unsupported("UI input ownership observation"))
    }
    /// Availability of an explicitly installed platform backend. This is not
    /// an entitlement check and never substitutes an SDK achievement result.
    /// Admit one asynchronous desktop operation against the client's roots.
    /// Providers distinguish unavailable capability, failure and user cancel.
    fn begin_desktop_request(&mut self,_client:ClientId,_request:DesktopRequest)->Result<alloc::boxed::Box<dyn DesktopRequestJob>>{
        Err(crate::Error::Unsupported("desktop host provider"))
    }
    /// Queue a client-owned original-save load without recursively borrowing
    /// its executing VM. The runtime admits the detached preparation after
    /// `step` returns and atomically installs it when ready. A successful load
    /// replaces that VM; a failure leaves it available to observe the receipt.
    /// Request tickets belong to a separate domain from host-issued restores.
    fn request_original_save_restore(
        &mut self,
        _client: ClientId,
        _bytes: Arc<[u8]>,
    ) -> Result<crate::native_restore::OriginalRestoreTicket> {
        Err(crate::Error::Unsupported("queued original save restore"))
    }
    fn original_save_restore_request_status(
        &self,
        _client: ClientId,
        _ticket: crate::native_restore::OriginalRestoreTicket,
    ) -> Result<crate::native_restore::OriginalRestoreStatus> {
        Err(crate::Error::Unsupported("queued original save restore"))
    }
    fn cancel_original_save_restore_request(
        &mut self,
        _client: ClientId,
        _ticket: crate::native_restore::OriginalRestoreTicket,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("queued original save restore"))
    }
    fn release_original_save_restore_request(
        &mut self,
        _client: ClientId,
        _ticket: crate::native_restore::OriginalRestoreTicket,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("queued original save restore"))
    }
    fn platform_integration(
        &self,
        _client: ClientId,
        _integration: PlatformIntegration,
    ) -> Result<PlatformIntegrationInfo> {
        Err(crate::Error::Unsupported("platform integration status"))
    }
    /// Last owner-published picture; None while no frame has completed. A stale
    /// or foreign media identity must not expose another client's picture.
    /// Client-scoped zero-copy image lease over the last published picture.
    fn media_image(
        &self,
        _client: ClientId,
        _id: u32,
    ) -> Result<Option<Arc<dyn crate::image_processing::ImageResource>>> {
        Err(crate::Error::Unsupported("media frame image lease"))
    }
    fn media_audio_clock(&self, _client: ClientId, _id: u32) -> Result<MediaAudioClock> {
        Err(crate::Error::Unsupported("media PCM clock"))
    }
    fn media_frame(&self, _client: ClientId, _id: u32) -> Result<Option<MediaFrame>> {
        Err(crate::Error::Unsupported("engine media picture"))
    }
    fn font_metrics(&self, _font: &str, _size: f32, _character: char) -> Result<FontMetrics> {
        Err(crate::Error::Unsupported("engine font metrics"))
    }
    fn submit(&mut self, client: ClientId, commands: &[Command]) -> Result<()>;
    fn now_ms(&self) -> u64;
    /// Begin resource preparation. A successful return admits work; adapters
    /// must retain their native event until audio_preparation reports Ready.
    fn begin_audio(&mut self, _client: ClientId, _request: AudioRequest) -> Result<u64> {
        Err(crate::Error::Unsupported("channel audio preparation"))
    }
    /// Prepare playback with a source-level meter installed at publication.
    /// A preparation that retains existing playback also retains its meter.
    fn begin_metered_audio(
        &mut self,
        _client: ClientId,
        _request: AudioRequest,
        _meter: AudioSourceMeterSpec,
    ) -> Result<u64> {
        Err(crate::Error::Unsupported("metered channel audio preparation"))
    }
    fn audio_preparation(&mut self, _client: ClientId, _ticket: u64) -> Result<AudioPreparation> {
        Err(crate::Error::Unsupported(
            "channel audio preparation status",
        ))
    }
    /// Cancel an unpublished request or release a ready receipt. Ready playback
    /// remains alive; stale/foreign receipts never authorize another client.
    fn release_audio(&mut self, _client: ClientId, _ticket: u64) -> Result<()> {
        Err(crate::Error::Unsupported(
            "channel audio preparation release",
        ))
    }
    fn submit_audio(&mut self, _client: ClientId, _request: AudioRequest) -> Result<()> {
        Err(crate::Error::Unsupported("shared channel audio"))
    }
    /// Update the legacy fader on a live client channel. Missing channels stay
    /// empty. A live channel must have opted into this fader at publication.
    fn update_audio_pcm16_gain(
        &mut self,
        _client: ClientId,
        _channel: u32,
        _update: AudioPcm16GainUpdate,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("channel PCM16 gain"))
    }
    /// Set the client-scoped PCM16 device attenuation (0–256). It affects
    /// opt-in live and future voices between the secondary and stereo stages.
    fn set_audio_pcm16_device_gain(&mut self, _client: ClientId, _gain: u16) -> Result<()> {
        Err(crate::Error::Unsupported("client PCM16 device gain"))
    }
    /// Atomically replace the own gain of live channels and the retained gain
    /// of existing media slots in one client scope. Missing audio channels are
    /// ignored. Unknown media slots or invalid gains reject the whole update.
    /// Independent transport fades, source identity and media clocks survive.
    fn set_playback_gains(
        &mut self,
        _client: ClientId,
        _audio: &[(u32, f32)],
        _media: &[(u32, f32)],
    ) -> Result<()> {
        Err(crate::Error::Unsupported("atomic playback gain update"))
    }
    /// Atomically replace only the listed client channels. Prepared play
    /// recipes must address this scope. Channels with no recipe are cleared.
    /// None clears the entire supplied scope without preparing a source.
    fn replace_audio_channels(
        &mut self,
        _client: ClientId,
        _channels: &[u32],
        _prepared: Option<&Arc<dyn AudioReplacementResource>>,
    ) -> Result<()> {
        Err(crate::Error::Unsupported("scoped audio replacement"))
    }
    /// Submit a bounded block already mixed by a shared CPU audio service.
    /// Native adapters never create output devices. Current output rate: 48kHz.
    fn submit_pcm(
        &mut self,
        _client: ClientId,
        _sample_rate: u32,
        _frames: &[[f32; 2]],
    ) -> Result<()> {
        Err(crate::Error::Unsupported("PCM output"))
    }
    /// Versioned, bounded CPU engine operations. Implementations must validate
    /// the entire batch and scope every handle to the submitting client.
    fn submit_engine(&mut self, _client: ClientId, _schema: u32, _data: &[u8]) -> Result<()> {
        Err(crate::Error::Unsupported("engine services"))
    }
    /// Publish an already-completed immutable ImageResource on the owner.
    /// No pixel algorithm/encoding/copy; returned path is a new generation.
    fn publish_image_resource(
        &mut self,
        _client: ClientId,
        _id: u32,
        _image: Arc<dyn crate::image_processing::ImageResource>,
    ) -> Result<UploadedImage> {
        Err(crate::Error::Unsupported("image resource publication"))
    }
    /// Borrow a completed immutable image generation already published by this
    /// client. Paths never grant access to another client's resource catalog.
    /// None means this owner has no resident published generation at the path.
    fn published_image_resource(&mut self,_client:ClientId,_path:&str)->Result<Option<Arc<dyn crate::image_processing::ImageResource>>>{Ok(None)}
    /// Retire a live path. Frozen renderer frames may retain its resource lease.
    fn release_image_resource(&mut self, _client: ClientId, _path: &str) -> Result<()> {
        Err(crate::Error::Unsupported("image resource publication"))
    }
    /// Register decoded pixels under a host-minted client-scoped logical path.
    fn upload_rgba(
        &mut self,
        _client: ClientId,
        _id: u32,
        _width: u32,
        _height: u32,
        _rgba: &[u8],
    ) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("decoded image upload"))
    }
    /// Register an adapter-built glTF binary (GLB) model under a host-minted,
    /// immutable client-scoped asset path, for world `scene`/`model` geometry.
    /// Used when a native format's scene graph is assembled by the VM (for
    /// example Unity prefabs) rather than stored as one file.
    fn upload_model(&mut self, _client: ClientId, _id: u32, _glb: &[u8]) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("model upload"))
    }
    /// Register an adapter-built linear-RGB equirectangular environment
    /// (`width = 2 × height`, rows from +Y down) as a Radiance `.hdr` asset
    /// under a host-minted client-scoped path, for `Environment.hdr` (for
    /// example a time-of-day sky). Released with `release_model`.
    fn upload_hdr(&mut self, _client: ClientId, _id: u32, _width: u32, _height: u32, _rgb: &[[f32; 3]]) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("environment upload"))
    }
    /// End the caller's use of an `upload_model` (or `upload_hdr`) path. The host drops the
    /// model bytes once no world entity of the client references the asset
    /// (checked at the end of each tick, so a despawn in the same tick is
    /// enough); an identical later upload registers it again. Releasing a
    /// path still in use only defers the drop.
    fn release_model(&mut self, _client: ClientId, _path: &VfsPath) -> Result<()> {
        Err(crate::Error::Unsupported("model release"))
    }
    /// Decode provider-owned encoded image bytes (for example an image embedded
    /// in a native container the VFS does not mount) with the engine's shared
    /// decoder registry, then register the pixels exactly like `upload_rgba`.
    /// `name` is the logical name used for format probing.
    fn upload_encoded_image(
        &mut self,
        _client: ClientId,
        _id: u32,
        _name: &str,
        _bytes: &[u8],
    ) -> Result<UploadedImage> {
        Err(crate::Error::Unsupported("encoded image upload"))
    }
    /// Thread-safe image metadata handle for this client (ADR 0053).
    fn image_metadata(&mut self, _client: ClientId) -> Result<Arc<dyn ImageMetadata>> {
        Err(crate::Error::Unsupported("image metadata handle"))
    }
    /// Client-authorized asynchronous registration of engine-owned font faces.
    fn font_library(&mut self, _client: ClientId) -> Result<Arc<dyn FontLibrary>> {
        Err(crate::Error::Unsupported("font library handle"))
    }
    /// Client-authorized immutable font measurement capability.
    fn text_measurement(&mut self, _client: ClientId) -> Result<Arc<dyn TextMeasurement>> {
        Err(crate::Error::Unsupported("text measurement handle"))
    }
    /// Immutable host-published media aliases; querying performs no device I/O.
    fn mounted_media(&mut self, client: ClientId) -> Result<MountedMediaCatalog> {
        MountedMediaCatalog::new(client, Vec::new())
    }
    /// Thread-safe storage handle for this client (ADR 0008).
    fn storage(&mut self, _client: ClientId) -> Result<Arc<dyn ClientStorage>> {
        Err(crate::Error::Unsupported("client storage handle"))
    }
    /// Publish a validated JSON PuppetFrame on an existing client-owned layer.
    /// Only decoded textures and engine mesh data cross this device-free boundary.
    fn present_puppet_frame(
        &mut self,
        _client: ClientId,
        _layer: u32,
        _frame: &[u8],
    ) -> Result<()> {
        Err(crate::Error::Unsupported("dynamic puppet frame"))
    }
    /// Replace a client-scoped dynamic RGBA resource or engine render target.
    /// Do not use this to host another engine's composed screen. Unlike `upload_rgba`, the
    /// logical path is stable and its contents may change every frame;
    /// surfaces are render products and are not saved (ADR 0008).
    fn present_surface(
        &mut self,
        _client: ClientId,
        _slot: u32,
        _width: u32,
        _height: u32,
        _rgba: &[u8],
    ) -> Result<VfsPath> {
        Err(crate::Error::Unsupported("dynamic surface"))
    }
    /// Drop a `present_surface` slot after this tick; its path stops resolving.
    /// Releasing an absent slot is a no-op.
    fn release_surface(&mut self, _client: ClientId, _slot: u32) -> Result<()> {
        Err(crate::Error::Unsupported("dynamic surface release"))
    }
}

/// Logical button bits in [`InputFrame`], shared by every frontend.
pub mod buttons {
    pub const ENTER: u64 = 1;
    pub const SPACE: u64 = 1 << 1;
    pub const LEFT: u64 = 1 << 2;
    pub const RIGHT: u64 = 1 << 3;
    pub const UP: u64 = 1 << 4;
    pub const DOWN: u64 = 1 << 5;
    pub const TAB: u64 = 1 << 6;
    pub const KEY_W: u64 = 1 << 7;
    pub const KEY_A: u64 = 1 << 8;
    pub const KEY_S: u64 = 1 << 9;
    pub const KEY_D: u64 = 1 << 10;
    pub const SHIFT: u64 = 1 << 11;
    pub const CTRL: u64 = 1 << 12;
    pub const KEY_E: u64 = 1 << 13;
    pub const KEY_Q: u64 = 1 << 14;
    pub const KEY_F: u64 = 1 << 15;
    pub const MOUSE_LEFT: u64 = 1 << 16;
    pub const MOUSE_RIGHT: u64 = 1 << 17;
    pub const KEY_R: u64 = 1 << 18;
    pub const ESCAPE: u64 = 1 << 19;
    pub const BACKSPACE: u64 = 1 << 20;
    pub const HOME: u64 = 1 << 21;
    pub const END: u64 = 1 << 22;
    pub const PAGE_UP: u64 = 1 << 23;
    pub const PAGE_DOWN: u64 = 1 << 24;
    pub const KEY_0: u64 = 1 << 25;
    pub const KEY_1: u64 = 1 << 26;
    pub const KEY_2: u64 = 1 << 27;
    pub const KEY_3: u64 = 1 << 28;
    pub const KEY_4: u64 = 1 << 29;
    pub const KEY_5: u64 = 1 << 30;
    pub const KEY_6: u64 = 1 << 31;
    pub const KEY_7: u64 = 1 << 32;
    pub const KEY_8: u64 = 1 << 33;
    pub const KEY_9: u64 = 1 << 34;
    pub const F1: u64 = 1 << 35;
    pub const F2: u64 = 1 << 36;
    pub const F3: u64 = 1 << 37;
    pub const F4: u64 = 1 << 38;
    pub const F5: u64 = 1 << 39;
    pub const F6: u64 = 1 << 40;
    pub const F7: u64 = 1 << 41;
    pub const F8: u64 = 1 << 42;
    pub const F9: u64 = 1 << 43;
    pub const F10: u64 = 1 << 44;
    pub const F11: u64 = 1 << 45;
    pub const F12: u64 = 1 << 46;
    pub const WHEEL_UP: u64 = 1 << 47;
    pub const WHEEL_DOWN: u64 = 1 << 48;
    pub const KEY_C: u64 = 1 << 49;
    pub const KEY_L: u64 = 1 << 50;
    pub const MOUSE_MIDDLE: u64 = 1 << 51;
    pub const KEY_Z: u64 = 1 << 52;
    pub const INSERT:u64=1<<53;
    pub const ALT: u64 = 1 << 54;
    pub const DELETE: u64 = 1 << 55;
    pub const KEY_H: u64 = 1 << 56;
    pub const KEY_I: u64 = 1 << 57;
    pub const KEY_J: u64 = 1 << 58;
    pub const KEY_M: u64 = 1 << 59;
    pub const KEY_V: u64 = 1 << 60;
    /// Additional letters retain all existing InputFrame bit assignments.
    pub const KEY_T: u64 = 1 << 61;
    pub const KEY_B: u64 = 1 << 62;
    pub const KEY_N: u64 = 1 << 63;
    /// Logical names used by action maps, in bit order.
    pub const NAMES: &[(&str, u64)] = &[
        ("enter", ENTER),
        ("space", SPACE),
        ("left", LEFT),
        ("right", RIGHT),
        ("up", UP),
        ("down", DOWN),
        ("tab", TAB),
        ("w", KEY_W),
        ("a", KEY_A),
        ("s", KEY_S),
        ("d", KEY_D),
        ("shift", SHIFT),
        ("ctrl", CTRL),
        ("e", KEY_E),
        ("q", KEY_Q),
        ("f", KEY_F),
        ("mouse_left", MOUSE_LEFT),
        ("mouse_right", MOUSE_RIGHT),
        ("r", KEY_R),
        ("escape", ESCAPE),
        ("backspace", BACKSPACE),
        ("home", HOME),
        ("end", END),
        ("page_up", PAGE_UP),
        ("page_down", PAGE_DOWN),
        ("0", KEY_0),
        ("1", KEY_1),
        ("2", KEY_2),
        ("3", KEY_3),
        ("4", KEY_4),
        ("5", KEY_5),
        ("6", KEY_6),
        ("7", KEY_7),
        ("8", KEY_8),
        ("9", KEY_9),
        ("f1", F1),
        ("f2", F2),
        ("f3", F3),
        ("f4", F4),
        ("f5", F5),
        ("f6", F6),
        ("f7", F7),
        ("f8", F8),
        ("f9", F9),
        ("f10", F10),
        ("f11", F11),
        ("f12", F12),
        ("insert",INSERT),
        ("wheel_up", WHEEL_UP),
        ("wheel_down", WHEEL_DOWN),
        ("c", KEY_C),
        ("l", KEY_L),
        ("mouse_middle", MOUSE_MIDDLE),
        ("z", KEY_Z),
        ("alt", ALT),
        ("delete", DELETE),
        ("h", KEY_H),
        ("i", KEY_I),
        ("j", KEY_J),
        ("m", KEY_M),
        ("v", KEY_V),
        ("t", KEY_T),
        ("b", KEY_B),
        ("n", KEY_N),
    ];
    /// Checked legacy VK translation for this framed-button profile. None
    /// means unrepresented, not released. Distinct keypad/side-specific keys
    /// are not aliased to a different physical key by this conversion.
    pub fn windows_virtual_key(code: u32) -> Option<u64> {
        Some(match code {
            1 => MOUSE_LEFT,
            2 => MOUSE_RIGHT,
            4 => MOUSE_MIDDLE,
            8 => BACKSPACE,
            9 => TAB,
            13 => ENTER,
            16 => SHIFT,
            17 => CTRL,
            18 => ALT,
            27 => ESCAPE,
            32 => SPACE,
            33 => PAGE_UP,
            34 => PAGE_DOWN,
            35 => END,
            36 => HOME,
            37 => LEFT,
            38 => UP,
            39 => RIGHT,
            40 => DOWN,
            45 => INSERT,
            46 => DELETE,
            0x30..=0x39 => KEY_0 << (code - 0x30),
            0x41 => KEY_A,
            0x43 => KEY_C,
            0x44 => KEY_D,
            0x45 => KEY_E,
            0x46 => KEY_F,
            0x48 => KEY_H,
            0x49 => KEY_I,
            0x4a => KEY_J,
            0x4c => KEY_L,
            0x4d => KEY_M,
            0x4e => KEY_N,
            0x42 => KEY_B,
            0x54 => KEY_T,
            0x51 => KEY_Q,
            0x52 => KEY_R,
            0x53 => KEY_S,
            0x56 => KEY_V,
            0x57 => KEY_W,
            0x5a => KEY_Z,
            0x70..=0x7b => F1 << (code - 0x70),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputFrame {
    pub buttons_down: u64,
    pub buttons_pressed: u64,
    pub pointer_x: i32,
    pub pointer_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub glyph_id: u32,
    pub x_26_6: i32,
    pub y_26_6: i32,
    pub advance_26_6: i32,
}

/// Like a small text service API, not an implementation of Win32/GDI.
/// Ruby, kinsoku and encoding-specific quirks remain adapter layout policies.
pub trait TextService {
    fn layout(&mut self, request: &TextRun) -> Result<Vec<Glyph>>;
}

pub trait RenderBackend {
    /// Called only after the command batch is committed. This does not own state.
    fn present(&mut self, scene: &SceneState) -> Result<()>;
}

pub trait AudioBackend {
    fn sync(&mut self, scene: &SceneState) -> Result<()>;
}

mod audio_playlist;
pub use audio_playlist::{AudioPlaylist, AudioPlaylistNode};

#[path = "audio_samples.rs"]
pub mod audio_samples;
pub use audio_samples::{AudioSampleFormat,AudioSampleRequest,AudioSampleReceipt,AudioSampleState,AudioSampleQuery};
