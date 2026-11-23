//! Scoped PCM windows for source inspection and playback visualization.
use crate::{Error, Result, service::AudioResourceHandle, state::ClientId};
use alloc::{sync::Arc, vec::Vec};
/// Layout of the returned samples, unsigned 8-bit or signed little-endian
/// 16-bit. Source inspection preserves the input layout. A visualization
/// query can return decoded stereo without changing the source format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSampleFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}
/// Read at a source frame offset. Zero bytes performs format inspection only.
#[derive(Clone, Copy, Debug)]
pub struct AudioSampleRequest {
    pub start_frame: u64,
    pub max_bytes: u32,
}
/// Clones share sample storage and its engine residency charge.
#[derive(Clone)]
pub struct AudioSampleReceipt {
    pub client: ClientId,
    pub resource: AudioResourceHandle,
    pub format: AudioSampleFormat,
    pub start_frame: u64,
    pub frames: u32,
    pub bytes: Arc<[u8]>,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl AudioSampleReceipt {
    pub fn new(
        client: ClientId,
        resource: AudioResourceHandle,
        format: AudioSampleFormat,
        start_frame: u64,
        frames: u32,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        if resource.client != client.0
            || !matches!(format.channels, 1 | 2)
            || !matches!(format.bits_per_sample, 8 | 16)
            || (8000..=192000).contains(&format.sample_rate) == false
        {
            return Err(Error::Invalid("source PCM sample receipt"));
        }
        let length =
            u64::from(frames) * u64::from(format.channels) * u64::from(format.bits_per_sample / 8);
        if bytes.len() > 1024 * 1024 || length != bytes.len() as u64 {
            return Err(Error::Limit("source PCM sample receipt bytes"));
        }
        Ok(Self {
            client,
            resource,
            format,
            start_frame,
            frames,
            bytes: bytes.into(),
            storage: None,
        })
    }
    pub fn retain_storage(&mut self, storage: Arc<dyn Send + Sync>) {
        self.storage = Some(storage);
    }
}
pub enum AudioSampleState {
    Pending,
    Ready(AudioSampleReceipt),
}
/// Dropping a query cancels unconsumed worker work and releases retained leases.
/// A failed query continues reporting its original failure on subsequent polls.
pub trait AudioSampleQuery: Send {
    fn poll(&mut self) -> Result<AudioSampleState>;
}
