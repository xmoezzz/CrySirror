// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Narrow host services. Backends implement these; legacy adapters consume them.
use crate::{
    Result,
    path::VfsPath,
    state::{ClientId, Command, SceneState, TextRun},
};
use alloc::{string::String, sync::Arc, vec::Vec};

pub const SIMPLE_TEXT_LAYOUT: &str = "simple-horizontal-v1";
/// The same horizontal glyph metrics without automatic width wrapping.
pub const UNWRAPPED_TEXT_LAYOUT: &str = "simple-horizontal-unwrapped-v1";

/// Read-only capability for an asset such as a large native archive. Offsets
/// are explicit: callers can keep independent seek cursors without sharing a
/// filesystem path or the underlying stream position. Never serialized.
pub trait AssetReader: Send + Sync {
    fn len(&self) -> Result<u64>;
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize>;
}

/// Bounded fallback for in-memory/provider assets; disk hosts can stream.
pub struct MemoryAssetReader(pub Arc<[u8]>);
impl AssetReader for MemoryAssetReader {
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
#[derive(Clone, Debug)]
pub struct AudioParameters {
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
    /// Decoded-source frame to repeat from, default zero.
    pub loop_start: u64,
    /// Startup position in source milliseconds unless start_frame is set.
    pub start_ms: u64,
    /// Exact decoded-source frame; when present this overrides start_ms.
    pub start_frame: Option<u64>,
    /// Clamp authored source/loop positions to the prepared source boundary.
    pub clamp_range: bool,
    /// Publish without consuming samples until an explicit resume.
    pub paused: bool,
    /// Fade the previous channel voice after successful replacement publication.
    pub replace_fade_ms: f32,
    /// Initial fade-in duration in milliseconds.
    pub fade_ms: f32,
    /// False keeps a currently active channel instead of reopening it.
    pub restart: bool,
    /// Shared output bus, default Sfx.
    pub bus: PcmBus,
}
impl Default for AudioParameters {
    fn default() -> Self {
        Self {
            gain: 1.,
            decode_gain: 1.,
            rate: 1.,
            pan: 0.5,
            looping: false,
            loop_start: 0,
            start_ms: 0,
            start_frame: None,
            clamp_range: false,
            paused: false,
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
    /// Source has remaining input, even when paused; excludes processing tails.
    pub source_playing: bool,
    /// Source clock currently frozen by pause or scheduled resume.
    pub paused: bool,
    /// Actual prepared source loop mode.
    pub looping: bool,
    /// Current decoded-source frame, without worker read-ahead.
    pub position_frames: u64,
    /// Decoded-source frames per second.
    pub source_rate: u32,
    /// Prepared source boundary, when known.
    pub source_frames: Option<u64>,
    /// Mixer frames per second, used by fade/delay counters.
    pub output_rate: u32,
    /// Remaining stop/pause fade frames; ordinary gain fades are excluded.
    pub fade_out_frames: u64,
    /// Remaining scheduled-resume delay in mixer frames.
    pub resume_delay_frames: u64,
}
/// Deferred, client-capability audio source. No bytes or device pointers in VM state.
#[derive(Clone, Debug)]
pub struct AudioAsset {
    /// Canonical read-only asset path under the submitting client.
    pub path: VfsPath,
    /// Optional native declared decoded-frame boundary.
    pub frame_limit: Option<u64>,
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
    SetGain {
        channel: u32,
        gain: f32,
        fade_ms: f32,
    },
    SetPan {
        channel: u32,
        pan: f32,
        fade_ms: f32,
    },
    /// Change loop mode at the current source position; never restart/seek.
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
    Stop {
        id: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaPhase {
    Loading,
    Playing,
    Ended,
    Stopped,
    Failed,
}
#[derive(Clone, Copy, Debug)]
pub struct MediaInfo {
    pub phase: MediaPhase,
    pub position_ms: u64,
    pub published_frames: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub advance: f32,
}
#[derive(Clone, Copy, Debug)]
pub enum EngineQuery {
    Layer(u32),
    Canvas(u32),
    UiWidget(u32),
    UiEvents,
    MenuEvents,
}

/// A thread-safe capability handle to one client's namespaces, for engines
/// that read synchronously from their own threads or stream large packs
/// (ADR 0008). It carries no authority beyond that client's asset (read-only,
/// including provider mounts), save and cache roots.
pub trait ClientStorage: Send + Sync {
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
    /// Optional exclusive lease inside this client's cache namespace. Used by
    /// native save implementations that cannot merge concurrent writers.
    fn acquire_lease(&self, _name: &str) -> Result<alloc::boxed::Box<dyn StorageLease>> {
        Err(crate::Error::Unsupported("storage lease"))
    }
    fn write_atomic(&self, path: &VfsPath, data: &[u8]) -> Result<()>;
    fn remove(&self, path: &VfsPath) -> Result<()>;
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
pub trait ImageMetadata: Send + Sync {
    fn size(&self, asset: &str) -> Result<[u32; 2]>;
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

/// Immutable CPU font-resource snapshot. Native script callbacks can measure
/// synchronously on any thread without borrowing the owner host or a renderer.
/// This is the simple-horizontal-v1 layout, not a native layout substitution.
pub trait TextMeasurement: Send + Sync {
    fn measure_simple(&self, font: &str, size: f32, text: &str) -> Result<TextExtent>;
}

pub trait HostServices: FileSystem {
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
    fn media_info(&self, _client: ClientId, _id: u32) -> Result<MediaInfo> {
        Err(crate::Error::Unsupported("engine media status"))
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
    /// Thread-safe image metadata handle for this client (ADR 0053).
    fn image_metadata(&mut self, _client: ClientId) -> Result<Arc<dyn ImageMetadata>> {
        Err(crate::Error::Unsupported("image metadata handle"))
    }
    /// Client-authorized immutable font measurement capability.
    fn text_measurement(&mut self, _client: ClientId) -> Result<Arc<dyn TextMeasurement>> {
        Err(crate::Error::Unsupported("text measurement handle"))
    }
    /// Thread-safe storage handle for this client (ADR 0008).
    fn storage(&mut self, _client: ClientId) -> Result<Arc<dyn ClientStorage>> {
        Err(crate::Error::Unsupported("client storage handle"))
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
        ("wheel_up", WHEEL_UP),
        ("wheel_down", WHEEL_DOWN),
    ];
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
