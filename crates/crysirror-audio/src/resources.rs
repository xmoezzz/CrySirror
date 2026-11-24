// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Immutable encoded resources and independent decoder leases, without devices.
mod samples;
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
    /// Short, repeatedly played compressed sounds share immutable PCM.
    /// Every voice and visualization query still owns its own cursor.
    short_pcm: Option<crysirror_core::service::PcmAudio>,
    extension: Option<String>,
    frame_limit: Option<u64>,
    info: AudioResourceInfo,
    /// Multi-track stems; `open` then yields a [`crate::multitrack::TrackSet`].
    stems: Option<Vec<PreparedStem>>,
}
struct PreparedStem {
    snapshot: Arc<Snapshot>,
    extension: Option<String>,
    pick: Option<[Option<usize>; 2]>,
    gain: f32,
}
fn open_stem(stem: &PreparedStem) -> Result<Box<dyn AudioSource>> {
    match stem.pick {
        Some(pick) => decoder::open_asset_picked(stem.snapshot.clone(), stem.extension.as_deref(), pick),
        None => decoder::open_asset(stem.snapshot.clone(), stem.extension.as_deref()),
    }
}
fn read_snapshot(
    storage: &dyn ClientStorage,
    path: &crysirror_core::path::VfsPath,
    budget: &Arc<AtomicUsize>,
    cancel: &Cancellation,
) -> Result<Arc<Snapshot>> {
    ensure!(path.mount() == Mount::Asset, "prepared audio requires asset authority");
    let reader = storage.open_asset(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let len = usize::try_from(reader.len().map_err(|e| anyhow::anyhow!("{e}"))?)?;
    ensure!(len > 0 && len <= RESOURCE_BYTES, "prepared audio snapshot exceeds 128 MiB or is empty");
    let charge = Charge::new(budget.clone(), len)?;
    let mut bytes = vec![0; len];
    let mut at = 0;
    while at < len {
        cancel.check()?;
        let end = (at + 65536).min(len);
        let n = reader
            .read_at(at as u64, &mut bytes[at..end])
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(n > 0 && n <= end - at, "prepared audio snapshot truncated/invalid read");
        at += n;
    }
    Ok(Arc::new(Snapshot { bytes, _charge: charge }))
}
fn extension_of(path: &crysirror_core::path::VfsPath) -> Option<String> {
    path.relative().rsplit_once('.').map(|v| v.1.to_owned())
}
/// Cache only complete, short sounds; long recordings keep their streaming decoder.
fn prepare_short_pcm(
    source: &mut dyn AudioSource, frames: Option<u64>, snapshot: &Snapshot, cancel: &Cancellation,
) -> Result<Option<crysirror_core::service::PcmAudio>> {
    let Some(count @ 1..=16_384) = frames else { return Ok(None) };
    let Ok(mut charge) = Charge::new(snapshot._charge.budget.clone(), count as usize * 16) else {
        return Ok(None);
    };
    // Measurement may have reached EOF. When rewind is unavailable the
    // ordinary encoded lease can still open an independent decoder.
    if source.seek(0).is_err() { return Ok(None); }
    let mut pcm = vec![[0.; 2]; count as usize];
    let mut at = 0;
    while at < pcm.len() {
        cancel.check()?;
        let read = source.read(&mut pcm[at..])?;
        ensure!(read <= pcm.len() - at, "short sound decoder extent");
        if read == 0 { return Ok(None); }
        at += read;
    }
    let Ok(mut pcm) = crysirror_core::service::PcmAudio::new(source.sample_rate(), pcm) else {
        return Ok(None);
    };
    charge.shrink(count as usize * 8);
    pcm.retain_storage(Arc::new(charge));
    Ok(Some(pcm))
}

impl PreparedSound {
    pub fn info(&self) -> AudioResourceInfo {
        self.info
    }
    pub fn open(self: &Arc<Self>) -> Result<Box<dyn AudioSource>> {
        if let Some(pcm) = &self.short_pcm {
            return Ok(Box::new(Instance {
                inner: decoder::open_pcm(pcm.clone()),
                _resource: self.clone(),
            }));
        }
        if let Some(stems) = &self.stems {
            let tracks = stems
                .iter()
                .map(|stem| Ok((open_stem(stem)?, stem.gain)))
                .collect::<Result<Vec<_>>>()?;
            return Ok(Box::new(Instance {
                inner: Box::new(crate::multitrack::TrackSet::new(tracks)?),
                _resource: self.clone(),
            }));
        }
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
    fn track_count(&self) -> usize {
        self.inner.track_count()
    }
    fn take_tracks(&mut self) -> Option<Vec<(Box<dyn AudioSource>, f32)>> {
        let resource = self._resource.clone();
        self.inner.take_tracks().map(|tracks| {
            tracks
                .into_iter()
                .map(|(inner, g)| (Box::new(Instance { inner, _resource: resource.clone() }) as Box<dyn AudioSource>, g))
                .collect()
        })
    }
    fn has_replay_gain(&self) -> bool {
        self._resource.info.has_replay_gain
    }
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
        self.begin_transformed(
            client,
            asset,
            measure_frames,
            None,
            storage,
            pool,
            nonblocking,
        )
    }
    pub fn begin_transformed(
        &mut self,
        client: u32,
        asset: AudioAsset,
        measure_frames: bool,
        transform: Option<crysirror_core::service::AudioResourceTransform>,
        storage: Arc<dyn ClientStorage>,
        pool: Option<TaskPool>,
        nonblocking: bool,
    ) -> Result<u64> {
        if let Some(t) = &transform {
            t.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        enum Work {
            Jitan(u16),
            Filters(Arc<[crysirror_core::service::PcmFilter]>),
            Wave(Arc<[crysirror_core::service::WaveFilter]>),
        }
        let multitrack = match &transform {
            Some(crysirror_core::service::AudioResourceTransform::MultiTrack(stems)) => Some(stems.clone()),
            _ => None,
        };
        let compression = match transform {
            Some(crysirror_core::service::AudioResourceTransform::SiglusJitan { percent })
                if percent != 100 =>
            {
                Some(Work::Jitan(percent))
            }
            Some(crysirror_core::service::AudioResourceTransform::PcmFilters(chain)) => {
                Some(Work::Filters(chain))
            }
            Some(crysirror_core::service::AudioResourceTransform::WaveFilters(chain)) => {
                Some(Work::Wave(chain))
            }
            _ => None,
        };
        ensure!(
            asset.path.mount() == Mount::Asset,
            "prepared audio requires asset authority"
        );
        self.jobs.admit(client)?;
        ensure!(
            self.resources.len() + self.reserved.len() < 1024
                && self.resources.keys().filter(|h| h.client == client).count()
                    + self.reserved.values().filter(|v| **v == client).count()
                    < 256,
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
            let scratch = if compression.is_some() {
                128 * 1024 * 1024
            } else {
                SCRATCH_BYTES
            };
            let charge = Charge::new(budget.clone(), len + scratch)?;
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
            let mut extension = asset
                .path
                .relative()
                .rsplit_once('.')
                .map(|v| v.1.to_owned());
            if let Some(stems) = multitrack {
                if let Some(s) = Arc::get_mut(&mut snapshot) {
                    s._charge.shrink(len + 8192);
                }
                // Each distinct stem asset is snapshotted once; every stem is
                // probed for rate and length and opened again per voice.
                let mut snapshots: BTreeMap<String, Arc<Snapshot>> = BTreeMap::new();
                let mut prepared = Vec::with_capacity(stems.len());
                let mut rate = None;
                let mut longest = 0u64;
                let mut encoded = len;
                for stem in stems.iter() {
                    cancel.check()?;
                    let (snap, ext) = match &stem.asset {
                        None => (snapshot.clone(), extension.clone()),
                        Some(path) => {
                            let key = path.relative().to_owned();
                            let snap = match snapshots.get(&key) {
                                Some(s) => s.clone(),
                                None => {
                                    let s = read_snapshot(storage.as_ref(), path, &budget, &cancel)?;
                                    encoded += s.bytes.len();
                                    snapshots.insert(key, s.clone());
                                    s
                                }
                            };
                            (snap, extension_of(path))
                        }
                    };
                    let stem = PreparedStem {
                        snapshot: snap,
                        extension: ext,
                        pick: stem.pick.map(|p| p.map(|c| c.map(usize::from))),
                        gain: stem.gain,
                    };
                    let mut probe = open_stem(&stem)?;
                    let r = probe.sample_rate();
                    ensure!(rate.is_none_or(|x| x == r), "multi-track stems differ in sample rate");
                    rate = Some(r);
                    let frames = match probe.frame_count() {
                        Some(n) => n,
                        None => {
                            let mut count = 0u64;
                            let mut block = vec![[0.; 2]; 4096];
                            loop {
                                cancel.check()?;
                                let n = probe.read(&mut block)?;
                                if n == 0 {
                                    break;
                                }
                                count += n as u64;
                            }
                            count
                        }
                    };
                    longest = longest.max(frames);
                    prepared.push(stem);
                }
                let rate = rate.context("multi-track resource without stems")?;
                ensure!((8000..=192000).contains(&rate), "prepared audio sample rate");
                ensure!(longest > 0, "empty prepared audio source");
                return Ok(Product::Prepared(Arc::new(PreparedSound {
                    snapshot,
                    short_pcm: None,
                    extension,
                    frame_limit: None,
                    info: AudioResourceInfo {
                        source_rate: rate,
                        source_frames: Some(longest),
                        encoded_bytes: encoded as u64,
                        has_replay_gain: false,
                    },
                    stems: Some(prepared),
                })));
            }
            let mut source = decoder::open_asset(snapshot.clone(), extension.as_deref())?;
            let mut has_replay_gain = source.has_replay_gain();
            let rate = source.sample_rate();
            let mut frame_limit = asset.frame_limit;
            let mut stored_len = len;
            if let Some(work) = compression {
                let channels = source
                    .channel_count()
                    .context("audio transform requires an explicit mono/stereo layout")?;
                ensure!(
                    (1..=2).contains(&channels),
                    "unsupported transformed source channels"
                );
                let cap = crate::voice_compression::MAX_INPUT_FRAMES;
                ensure!(
                    asset.frame_limit != Some(0),
                    "empty transformed source range"
                );
                if let Some(n) = source.frame_count() {
                    ensure!(
                        n.min(asset.frame_limit.unwrap_or(u64::MAX)) <= cap as u64,
                        "audio transform decoded-frame budget"
                    );
                }
                let float = matches!(work, Work::Wave(_));
                let mut pcm = Vec::new();
                let mut wide = Vec::new();
                let mut block = vec![[0.; 2]; 4096];
                let mut total = 0usize;
                loop {
                    cancel.check()?;
                    let count = block.len().min(asset.frame_limit.map_or(usize::MAX, |n| {
                        n.saturating_sub(total as u64).min(usize::MAX as u64) as usize
                    }));
                    if count == 0 {
                        break;
                    }
                    let n = source.read(&mut block[..count])?;
                    ensure!(n <= count, "invalid transform decoder read");
                    if n == 0 {
                        break;
                    }
                    total = total
                        .checked_add(n)
                        .context("audio transform frame overflow")?;
                    ensure!(total <= cap, "audio transform decoded-frame budget");
                    for frame in &block[..n] {
                        for &sample in &frame[..channels as usize] {
                            ensure!(sample.is_finite(), "nonfinite transform input");
                            if float {
                                wide.push(sample);
                                continue;
                            }
                            pcm.push(
                                (sample.clamp(-1., 1.) * 32768.)
                                    .round()
                                    .clamp(-32768., 32767.) as i16,
                            );
                        }
                    }
                }
                drop(source);
                let converted = match work {
                    Work::Jitan(percent) => crate::voice_compression::siglus_jitan_cancellable(
                        &pcm, channels, rate, percent, &cancel,
                    )?,
                    Work::Filters(chain) => crate::voice_compression::Pcm16 {
                        rate,
                        channels: 2,
                        samples: crate::pcm_filters::apply(&chain, channels, &pcm, &cancel)?,
                    },
                    Work::Wave(chain) => {
                        let (channels, out) = crate::wave_filters::apply(
                            &chain,
                            rate,
                            channels,
                            std::mem::take(&mut wide),
                            &cancel,
                        )?;
                        crate::voice_compression::Pcm16 {
                            rate,
                            channels,
                            samples: out
                                .iter()
                                .map(|s| {
                                    (s.clamp(-1., 1.) * 32768.).round().clamp(-32768., 32767.) as i16
                                })
                                .collect(),
                        }
                    }
                };
                drop(pcm);
                let raw = converted
                    .samples
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect::<Vec<_>>();
                let encoded = crysirror_resource::WaveSpec {
                    sample_rate: rate,
                    channels: converted.channels,
                    ..Default::default()
                }
                .encode(&raw)?;
                drop(raw);
                drop(converted);
                stored_len = encoded.len();
                // Keep original+transform scratch charged until input and output
                // buffers no longer overlap; the final snapshot retains actual bytes.
                let mut output_charge = Charge::new(budget, stored_len + 8192)?;
                drop(snapshot);
                cancel.check()?;
                output_charge.shrink(stored_len + 8192);
                snapshot = Arc::new(Snapshot {
                    bytes: encoded,
                    _charge: output_charge,
                });
                extension = Some("wav".into());
                frame_limit = None;
                source = decoder::open_asset(snapshot.clone(), Some("wav"))?;
            }
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
            has_replay_gain |= source.has_replay_gain();
            let short_pcm = if frame_limit.is_none() && snapshot.bytes.starts_with(b"OggS") {
                prepare_short_pcm(&mut *source, frames, &snapshot, &cancel)?
            } else { None };
            drop(source);
            frames = match (frame_limit, frames) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(_), None) => None,
                (None, b) => b,
            };
            ensure!(frames != Some(0), "empty prepared audio source");
            cancel.check()?;
            Arc::get_mut(&mut snapshot)
                .context("decoder retained unexpected snapshot owner")?
                ._charge
                .shrink(stored_len + 8192);
            Ok(Product::Prepared(Arc::new(PreparedSound {
                snapshot,
                short_pcm,
                extension,
                frame_limit,
                info: AudioResourceInfo {
                    source_rate: rate,
                    source_frames: frames,
                    encoded_bytes: stored_len as u64,
                    has_replay_gain,
                },
                stems: None,
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
