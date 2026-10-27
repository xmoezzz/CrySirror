// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Immutable encoded resources and independent decoder leases, without devices.
use crate::decoder::{self, AudioSource, SourceBufferStatus, SourceRead, StereoFrame};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::Mount,
    service::{
        AssetReader, AudioAsset, AudioPreparePhase, AudioResourceHandle, AudioResourceInfo,
        AudioResourcePreparation, ClientStorage,
    },
};
use crysirror_tasks::{
    Cancellation, TaskPool,
    table::{OperationState, OperationTable},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
const RESOURCE_BYTES: usize = 128 * 1024 * 1024;
const HOST_BYTES: usize = 512 * 1024 * 1024;
const SCRATCH_BYTES: usize = 4 * 1024 * 1024;
struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(budget: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n
                    .checked_add(bytes)
                    .filter(|n| *n <= HOST_BYTES))
                .is_ok(),
            "prepared audio residency budget exceeded"
        );
        Ok(Self { budget, bytes })
    }
    fn shrink(&mut self, bytes: usize) {
        self.budget.fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        self.bytes = bytes;
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Snapshot {
    bytes: Vec<u8>,
    _charge: Charge,
}
impl AssetReader for Snapshot {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.bytes.len() as u64)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
        let Some(input) = usize::try_from(offset)
            .ok()
            .and_then(|at| self.bytes.get(at..))
        else {
            return Ok(0);
        };
        let n = output.len().min(input.len());
        output[..n].copy_from_slice(&input[..n]);
        Ok(n)
    }
}
/// An immutable source lease; opening a decoder belongs on a worker. Instances
/// share encoded storage while retaining independent decoder/cursor state.
pub struct PreparedSound {
    snapshot: Arc<Snapshot>,
    extension: Option<String>,
    frame_limit: Option<u64>,
    info: AudioResourceInfo,
}
impl PreparedSound {
    pub fn info(&self) -> AudioResourceInfo {
        self.info
    }
    pub fn open(self: &Arc<Self>) -> Result<Box<dyn AudioSource>> {
        let inner = decoder::open_asset(self.snapshot.clone(), self.extension.as_deref())?;
        let inner =
            crate::channels::wrap_source(inner, self.info.source_frames.or(self.frame_limit), 1.)?;
        Ok(Box::new(Instance {
            inner,
            _resource: self.clone(),
        }))
    }
}
struct Instance {
    inner: Box<dyn AudioSource>,
    _resource: Arc<PreparedSound>,
}
impl AudioSource for Instance {
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn frame_count(&self) -> Option<u64> {
        self.inner.frame_count()
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        self.inner.read(out)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.inner.seek(frame)
    }
    fn poll_read(&mut self, out: &mut [StereoFrame]) -> Result<SourceRead> {
        self.inner.poll_read(out)
    }
    fn poll_seek(&mut self, frame: u64) -> Result<bool> {
        self.inner.poll_seek(frame)
    }
    fn service(&mut self) -> Result<()> {
        self.inner.service()
    }
    fn read_position(&self) -> Option<u64> {
        self.inner.read_position()
    }
    fn is_nonblocking(&self) -> bool {
        self.inner.is_nonblocking()
    }
    fn buffer_status(&self) -> Option<SourceBufferStatus> {
        self.inner.buffer_status()
    }
}
enum Product {
    Prepared(Arc<PreparedSound>),
    Published(AudioResourceHandle, AudioResourceInfo),
}
/// Single-owner resource namespace. Clear invalidates handles/jobs; existing
/// decoder leases keep storage charged until their actual last owner drops.
pub struct AudioResources {
    jobs: OperationTable<Product>,
    reserved: BTreeMap<u64, u32>,
    resources: BTreeMap<AudioResourceHandle, Arc<PreparedSound>>,
    budget: Arc<AtomicUsize>,
}
impl Default for AudioResources {
    fn default() -> Self {
        Self {
            jobs: OperationTable::new(16, 64).unwrap(),
            reserved: BTreeMap::new(),
            resources: BTreeMap::new(),
            budget: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl AudioResources {
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.reserved.clear();
        self.resources.clear();
    }
    pub fn resident_bytes(&self) -> usize {
        self.budget.load(Ordering::Acquire)
    }
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }
    pub fn capture(&self, client: u32, handle: AudioResourceHandle) -> Result<Arc<PreparedSound>> {
        ensure!(handle.client == client, "foreign prepared audio resource");
        self.resources
            .get(&handle)
            .cloned()
            .context("prepared audio resource released or obsolete")
    }
    pub fn release(&mut self, client: u32, handle: AudioResourceHandle) -> Result<()> {
        self.capture(client, handle)?;
        self.resources.remove(&handle);
        Ok(())
    }
    pub fn begin(
        &mut self,
        client: u32,
        asset: AudioAsset,
        measure_frames: bool,
        storage: Arc<dyn ClientStorage>,
        pool: Option<TaskPool>,
        nonblocking: bool,
    ) -> Result<u64> {
        ensure!(
            asset.path.mount() == Mount::Asset,
            "prepared audio requires asset authority"
        );
        self.jobs.admit(client)?;
        ensure!(
            self.resources.len() + self.reserved.len() < 128
                && self.resources.keys().filter(|h| h.client == client).count()
                    + self.reserved.values().filter(|v| **v == client).count()
                    < 32,
            "prepared audio handle capacity exceeded"
        );
        let budget = self.budget.clone();
        let work = move |cancel: Cancellation| -> Result<Product> {
            cancel.check()?;
            let reader = storage
                .open_asset(&asset.path)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let len = usize::try_from(reader.len().map_err(|e| anyhow::anyhow!("{e}"))?)?;
            ensure!(
                len > 0 && len <= RESOURCE_BYTES,
                "prepared audio snapshot exceeds 128 MiB or is empty"
            );
            let charge = Charge::new(budget, len + SCRATCH_BYTES)?;
            let mut bytes = vec![0; len];
            let mut at = 0;
            while at < len {
                cancel.check()?;
                let end = (at + 65536).min(len);
                let n = reader
                    .read_at(at as u64, &mut bytes[at..end])
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    n > 0 && n <= end - at,
                    "prepared audio snapshot truncated/invalid read"
                );
                at += n;
            }
            ensure!(
                reader.len().map_err(|e| anyhow::anyhow!("{e}"))? == len as u64,
                "audio size changed during preparation"
            );
            drop(reader);
            cancel.check()?;
            let mut snapshot = Arc::new(Snapshot {
                bytes,
                _charge: charge,
            });
            let extension = asset
                .path
                .relative()
                .rsplit_once('.')
                .map(|v| v.1.to_owned());
            let mut source = decoder::open_asset(snapshot.clone(), extension.as_deref())?;
            let rate = source.sample_rate();
            let mut frames = source.frame_count();
            ensure!(
                (8000..=192000).contains(&rate),
                "prepared audio sample rate"
            );
            if frames.is_none() && measure_frames {
                let mut count = 0u64;
                let mut block = vec![[0.; 2]; 4096];
                loop {
                    cancel.check()?;
                    let n = source.read(&mut block)?;
                    ensure!(n <= block.len(), "invalid prepared audio read");
                    if n == 0 {
                        break;
                    }
                    count = count
                        .checked_add(n as u64)
                        .context("audio frame count overflow")?;
                }
                frames = Some(count);
            }
            drop(source);
            frames = match (asset.frame_limit, frames) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(_), None) => None,
                (None, b) => b,
            };
            ensure!(frames != Some(0), "empty prepared audio source");
            cancel.check()?;
            Arc::get_mut(&mut snapshot)
                .context("decoder retained unexpected snapshot owner")?
                ._charge
                .shrink(len + 8192);
            Ok(Product::Prepared(Arc::new(PreparedSound {
                snapshot,
                extension,
                frame_limit: asset.frame_limit,
                info: AudioResourceInfo {
                    source_rate: rate,
                    source_frames: frames,
                    encoded_bytes: len as u64,
                },
            })))
        };
        let ticket = if nonblocking {
            self.jobs.submit_deferred(
                client,
                0,
                "audio_resource",
                pool.as_ref()
                    .context("configure workers before prepared audio loading")?,
                work,
            )?
        } else {
            let product = work(Cancellation::default())?;
            self.jobs
                .insert_ready(client, 0, "audio_resource", product)?
        };
        self.reserved.insert(ticket, client);
        Ok(ticket)
    }
    /// Publication is scoped observation, never a side effect of cancellation.
    pub fn preparation(&mut self, client: u32, ticket: u64) -> Result<AudioResourcePreparation> {
        let status = self.jobs.status(client, 0, ticket)?;
        if status.state == OperationState::Failed {
            self.reserved.remove(&ticket);
        }
        if status.state == OperationState::Ready {
            let entries = &mut self.resources;
            let reserved = &mut self.reserved;
            self.jobs
                .publish(client, 0, ticket, |product| match product {
                    Product::Prepared(sound) => {
                        let handle = AudioResourceHandle { client, id: ticket };
                        let info = sound.info();
                        entries.insert(handle, sound);
                        reserved.remove(&ticket);
                        Ok(Product::Published(handle, info))
                    }
                    done => Ok(done),
                })?;
        }
        let status = self.jobs.status(client, 0, ticket)?;
        let result = if status.state == OperationState::Ready {
            self.jobs.poll(client, 0, ticket)?
        } else {
            None
        };
        let (resource, info) = match result {
            Some(Product::Published(h, i)) => (Some(*h), Some(*i)),
            _ => (None, None),
        };
        Ok(AudioResourcePreparation {
            phase: match status.state {
                OperationState::Pending => AudioPreparePhase::Pending,
                OperationState::Ready => AudioPreparePhase::Ready,
                OperationState::Failed => AudioPreparePhase::Failed,
            },
            resource,
            info,
            error: status.error,
        })
    }
    pub fn release_preparation(&mut self, client: u32, ticket: u64) -> Result<()> {
        self.jobs.cancel(client, ticket)?;
        self.reserved.remove(&ticket);
        Ok(())
    }
}
