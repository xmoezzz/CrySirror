use super::error::RfvpResult;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::subsystem::resources::vfs::ReadSeek;
// `std` is the crate's own shim in `no_std` builds.
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

/// An encoded sound read from its file as it plays instead of held in memory: the music of some
/// games is tens of megabytes, more than a console with little memory has.
#[derive(Clone)]
pub struct AudioByteStream(Arc<Mutex<Box<dyn ReadSeek + Send + Sync>>>);

impl AudioByteStream {
    pub fn new(stream: Box<dyn ReadSeek + Send + Sync>) -> Self {
        Self(Arc::new(Mutex::new(stream)))
    }

    /// Read up to `buf.len()` bytes at `offset`; fewer only at the end.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> RfvpResult<usize> {
        let mut stream = self
            .0
            .lock()
            .map_err(|_| super::error::RfvpError::Backend)?;
        stream
            .seek(SeekFrom::Start(offset))
            .map_err(|_| super::error::RfvpError::Io)?;
        let mut filled = 0;
        while filled < buf.len() {
            match stream.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(_) => return Err(super::error::RfvpError::Io),
            }
        }
        Ok(filled)
    }

    /// The whole sound, for a host that cannot stream it.
    pub fn read_all(&self) -> RfvpResult<Vec<u8>> {
        let mut out = Vec::new();
        let mut chunk = alloc::vec![0u8; 64 * 1024];
        loop {
            let n = self.read_at(out.len() as u64, &mut chunk)?;
            if out
                .len()
                .checked_add(n)
                .map_or(true, |size| size > 64 * 1024 * 1024)
            {
                return Err(super::error::RfvpError::CapacityExceeded);
            }
            out.extend_from_slice(&chunk[..n]);
            if n < chunk.len() {
                return Ok(out);
            }
        }
    }
}

impl fmt::Debug for AudioByteStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AudioByteStream")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioStreamId(pub u32);

pub const BGM_LOGICAL_SLOT_COUNT: usize = 256;
pub const SE_LOGICAL_SLOT_COUNT: usize = 256;
pub const SE_STREAM_ID_BASE: u32 = 0x1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSlotKind {
    Bgm,
    Se,
}

impl AudioStreamId {
    pub const fn bgm(slot: usize) -> Self {
        Self(slot as u32)
    }

    pub const fn se(slot: usize) -> Self {
        Self(SE_STREAM_ID_BASE + slot as u32)
    }

    /// The soundtrack of the movie playing, if any.
    pub const MOVIE: Self = Self(0x2000);

    pub const fn slot_kind(self) -> Option<AudioSlotKind> {
        if self.0 < BGM_LOGICAL_SLOT_COUNT as u32 {
            Some(AudioSlotKind::Bgm)
        } else if self.0 >= SE_STREAM_ID_BASE
            && self.0 < SE_STREAM_ID_BASE + SE_LOGICAL_SLOT_COUNT as u32
        {
            Some(AudioSlotKind::Se)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleFormat {
    I16,
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedAudioKind {
    Unknown,
    Wav,
    Ogg,
    Mp3,
    Flac,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioStreamDesc {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: AudioSampleFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioParams {
    /// Linear gain: 0 is silent, 1 is the sound as recorded.
    pub volume: f32,
    /// -1 is the left speaker only, 0 the center, 1 the right speaker only.
    pub pan: f32,
    pub repeat: bool,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            repeat: false,
        }
    }
}

pub trait RfvpAudio {
    fn load_encoded(
        &mut self,
        id: AudioStreamId,
        kind: EncodedAudioKind,
        bytes: &[u8],
    ) -> RfvpResult<()>;

    /// Load a sound that is read as it plays. Hosts that cannot stream leave this out, and the
    /// core reads the sound whole and calls `load_encoded` instead.
    fn load_stream(
        &mut self,
        _id: AudioStreamId,
        _kind: EncodedAudioKind,
        _stream: AudioByteStream,
    ) -> RfvpResult<()> {
        Err(super::error::RfvpError::Unsupported)
    }

    fn create_stream(&mut self, id: AudioStreamId, desc: AudioStreamDesc) -> RfvpResult<()>;

    fn submit_i16(&mut self, id: AudioStreamId, samples: &[i16]) -> RfvpResult<()>;

    fn submit_f32(&mut self, id: AudioStreamId, samples: &[f32]) -> RfvpResult<()>;

    fn play(&mut self, id: AudioStreamId, params: AudioParams, fade_in_ms: u32) -> RfvpResult<()>;

    fn stop(&mut self, id: AudioStreamId, fade_ms: u32) -> RfvpResult<()>;

    fn pause(&mut self, id: AudioStreamId) -> RfvpResult<()>;

    fn resume(&mut self, id: AudioStreamId) -> RfvpResult<()>;

    fn set_params(&mut self, id: AudioStreamId, params: AudioParams) -> RfvpResult<()>;

    fn set_master_volume(&mut self, volume: f32) -> RfvpResult<()>;

    fn destroy_stream(&mut self, id: AudioStreamId);

    fn tick(&mut self, delta_us: u64) -> RfvpResult<()>;
}
