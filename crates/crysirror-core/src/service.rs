// SPDX-License-Identifier: MPL-2.0
//! Narrow host services. Backends implement these; legacy adapters consume them.
use crate::{
    path::VfsPath,
    state::{ClientId, Command, SceneState, TextRun},
    Result,
};
use alloc::{string::String, sync::Arc, vec::Vec};

pub const SIMPLE_TEXT_LAYOUT: &str = "simple-horizontal-v1";

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
    pub gain: f32,
    pub decode_gain: f32,
    pub rate: f32,
    pub pan: f32,
    pub looping: bool,
    pub loop_start: u64,
    pub start_ms: u64,
    pub fade_ms: f32,
    pub restart: bool,
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
            fade_ms: 0.,
            restart: true,
            bus: PcmBus::Sfx,
        }
    }
}
/// Client-scoped channel commands for the shared decoder/mixer. Encoded bytes
/// and decoder resources are not part of logical VM snapshots.
#[derive(Clone, Debug)]
pub enum AudioRequest {
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
}

pub trait FileSystem {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> Result<Vec<u8>>;
    /// Either the complete new value is visible or the previous value remains.
    fn write_atomic(&mut self, client: ClientId, path: &VfsPath, data: &[u8]) -> Result<()>;
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

pub trait HostServices: FileSystem {
    fn submit(&mut self, client: ClientId, commands: &[Command]) -> Result<()>;
    fn now_ms(&self) -> u64;
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
    /// Thread-safe storage handle for this client (ADR 0008).
    fn storage(&mut self, _client: ClientId) -> Result<Arc<dyn ClientStorage>> {
        Err(crate::Error::Unsupported("client storage handle"))
    }
    /// Replace a client-scoped dynamic RGBA surface (e.g. an original
    /// engine's own software-rendered frame). Unlike `upload_rgba`, the
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
