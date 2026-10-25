// SPDX-License-Identifier: MPL-2.0
//! Engine-owned incremental media playback. A media resource owns a bounded
//! decoder queue and its clock; a VM only issues resource commands and queries.
//! Decoded video frames are individual texture resources, never engine screens.
use anyhow::{Context, Result, anyhow, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AssetReader, AudioParameters, AudioRequest, ClientStorage, EncodedAudio, PcmBus},
    state::ClientId,
};
use crysirror_video::mpeg::{self, VideoFrame};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

pub const MAX_MEDIA_RESOURCES: usize = 8;
pub const MAX_MEDIA_PER_CLIENT: usize = 4;
pub const VIDEO_QUEUE_FRAMES: usize = 4;
pub const MAX_ENCODED_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_QUEUED_VIDEO_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_AUDIO_SAMPLES: usize = 16 * 1024 * 1024;
pub const MAX_RETAINED_AUDIO_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaOptions {
    pub looping: bool,
    pub audio: bool,
    pub gain: f32,
    /// The host reserves this channel within the same client's audio namespace.
    pub audio_channel: u32,
}
impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            looping: false,
            audio: true,
            gain: 1.,
            audio_channel: 0x7000_0000,
        }
    }
}
impl MediaOptions {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.gain.is_finite() && (0. ..=4.).contains(&self.gain),
            "media gain must be finite and 0..4"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaState {
    Loading,
    Playing,
    Ended,
    Stopped,
    Failed,
}
#[derive(Clone, Debug, Serialize)]
pub struct MediaStatus {
    pub state: MediaState,
    pub position_ms: u64,
    pub frame_pts_ms: Option<i64>,
    pub published_frames: u64,
    pub decoded_frames: u64,
    pub audio_started: bool,
    pub loops_completed: u64,
    pub error: Option<String>,
}
pub struct MediaFrameUpdate {
    pub client: ClientId,
    pub id: u32,
    pub frame: VideoFrame,
}
pub struct MediaAudioUpdate {
    pub client: ClientId,
    pub request: AudioRequest,
}
#[derive(Default)]
pub struct MediaUpdates {
    pub frames: Vec<MediaFrameUpdate>,
    pub audio: Vec<MediaAudioUpdate>,
}

/// A seek cursor over a client capability. It never obtains an OS path and
/// independent audio/video cursors do not share seek position.
struct AssetCursor {
    source: Arc<dyn AssetReader>,
    len: u64,
    position: u64,
}
impl AssetCursor {
    fn new(source: Arc<dyn AssetReader>) -> Result<Self> {
        let len = source
            .len()
            .map_err(|e| anyhow!("media stream length: {e}"))?;
        ensure!(
            len > 0 && len <= MAX_ENCODED_BYTES,
            "media encoded stream budget (1 GiB)"
        );
        Ok(Self {
            source,
            len,
            position: 0,
        })
    }
}
impl Read for AssetCursor {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self
            .source
            .read_at(self.position, out)
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.position = self
            .position
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("media cursor overflow"))?;
        Ok(n)
    }
}
impl Seek for AssetCursor {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(v) => i128::from(v),
            SeekFrom::Current(v) => i128::from(self.position) + i128::from(v),
            SeekFrom::End(v) => i128::from(self.len) + i128::from(v),
        };
        if !(0..=i128::from(self.len)).contains(&target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "media seek outside capability stream",
            ));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

struct ByteBudget {
    used: AtomicUsize,
    limit: usize,
}
impl ByteBudget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        })
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|sum| *sum <= self.limit)
            })
            .map_err(|_| anyhow!("media retained decoder memory budget exceeded"))?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
struct Reservation {
    budget: Arc<ByteBudget>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct QueuedFrame {
    frame: VideoFrame,
    _reservation: Reservation,
}
struct AudioTrack {
    bytes: Arc<[u8]>,
    first_pts_ms: i64,
    _reservation: Reservation,
}
#[derive(Default)]
struct Journal {
    errors: Vec<String>,
}
impl Journal {
    fn record(&mut self, error: String) {
        if self.errors.len() < 8 {
            self.errors.push(error.chars().take(4096).collect());
        }
    }
}
fn record(journal: &Mutex<Journal>, error: String) {
    journal
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(error);
}
fn protected(journal: &Mutex<Journal>, work: impl FnOnce() -> Result<()>) {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => record(journal, format!("{error:#}")),
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("decoder invariant failed");
            record(journal, format!("media decoder worker panic: {message}"));
        }
    }
}
struct Workers {
    video: Receiver<QueuedFrame>,
    audio: Option<Receiver<Option<AudioTrack>>>,
    cancel: Arc<AtomicBool>,
    journal: Arc<Mutex<Journal>>,
    decoded: Arc<AtomicUsize>,
    frame_ms: Arc<AtomicU64>,
}
impl Drop for Workers {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Workers {
    fn new(
        source: Arc<dyn AssetReader>,
        audio: bool,
        video_budget: Arc<ByteBudget>,
        audio_budget: Arc<ByteBudget>,
    ) -> Result<Self> {
        let mut video_reader = AssetCursor::new(source.clone())?;
        let audio_reader = if audio {
            Some(AssetCursor::new(source)?)
        } else {
            None
        };
        let (video_tx, video_rx) = mpsc::sync_channel(VIDEO_QUEUE_FRAMES);
        let cancel = Arc::new(AtomicBool::new(false));
        let journal = Arc::new(Mutex::new(Journal::default()));
        let decoded = Arc::new(AtomicUsize::new(0));
        let stop = cancel.clone();
        let errors = journal.clone();
        let counter = decoded.clone();
        let frame_ms = Arc::new(AtomicU64::new(34));
        let frame_duration = frame_ms.clone();
        std::thread::Builder::new()
            .name("crysirror-media-video".into())
            .spawn(move || {
                // Retain sender until the error journal has been committed. A
                // disconnected receiver can then never mistake an error for EOS.
                let _keep_sender = video_tx.clone();
                protected(&errors, || {
                    if let Some(info) = mpeg::sequence_info(&mut video_reader)? {
                        frame_duration
                            .store(info.frame_ms.round().max(1.) as u64, Ordering::Release);
                    }
                    mpeg::decode_video(
                        video_reader,
                        || stop.load(Ordering::Acquire),
                        |frame| {
                            if stop.load(Ordering::Acquire) {
                                return Ok(false);
                            }
                            let reservation = video_budget.reserve(frame.rgba.len())?;
                            counter.fetch_add(1, Ordering::Relaxed);
                            Ok(video_tx
                                .send(QueuedFrame {
                                    frame,
                                    _reservation: reservation,
                                })
                                .is_ok())
                        },
                    )?;
                    Ok(())
                });
            })
            .context("spawn engine video decoder")?;
        let mut workers = Self {
            video: video_rx,
            audio: None,
            cancel,
            journal,
            decoded,
            frame_ms,
        };
        if let Some(reader) = audio_reader {
            let (tx, rx) = mpsc::sync_channel(1);
            let stop = workers.cancel.clone();
            let errors = workers.journal.clone();
            std::thread::Builder::new().name("crysirror-media-audio".into()).spawn(move || {
                let _keep_sender = tx.clone();
                protected(&errors, || {
                    let mut samples = Vec::<i16>::new();
                    let mut reservations = Vec::new();
                    let mut format = None;
                    let mut first_pts_ms = 0;
                    mpeg::decode_audio(reader, || stop.load(Ordering::Acquire), |chunk| {
                        if stop.load(Ordering::Acquire) { return Ok(false); }
                        if let Some((rate, channels)) = format {
                            ensure!((rate,channels) == (chunk.sample_rate,chunk.channels), "media audio format changed within a track");
                        } else {
                            format = Some((chunk.sample_rate, chunk.channels));
                            first_pts_ms = chunk.pts_ms;
                        }
                        let count = samples.len().checked_add(chunk.samples.len()).context("media audio sample counter")?;
                        ensure!(count <= MAX_AUDIO_SAMPLES, "media full-track audio sample budget exceeded; streaming audio required for longer tracks");
                        reservations.push(audio_budget.reserve(chunk.samples.len() * 2)?);
                        samples.extend_from_slice(&chunk.samples);
                        Ok(true)
                    })?;
                    if stop.load(Ordering::Acquire) { return Ok(()); }
                    let track = if let Some((rate, channels)) = format {
                        let bytes = wave(rate, channels, &samples)?;
                        let reservation = audio_budget.reserve(bytes.len())?;
                        drop(samples);
                        drop(reservations);
                        Some(AudioTrack { bytes: Arc::from(bytes), first_pts_ms, _reservation: reservation })
                    } else { None };
                    let _ = tx.send(track);
                    Ok(())
                });
            }).context("spawn engine media audio decoder")?;
            workers.audio = Some(rx);
        }
        Ok(workers)
    }
    fn error(&self) -> Option<String> {
        self.journal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .errors
            .first()
            .cloned()
    }
}
fn wave(rate: u32, channels: u16, samples: &[i16]) -> Result<Vec<u8>> {
    let len = u32::try_from(samples.len().checked_mul(2).context("media WAV size")?)?;
    ensure!(
        rate > 0 && channels > 0 && samples.len() % channels as usize == 0,
        "media WAV format"
    );
    let mut out = Vec::with_capacity(len as usize + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(len + 36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(out)
}
struct Slot {
    source: Arc<dyn AssetReader>,
    workers: Option<Workers>,
    options: MediaOptions,
    status: MediaStatus,
    pending: Option<QueuedFrame>,
    track: Option<AudioTrack>,
    audio_ready: bool,
    audio_sent: bool,
    start_ms: Option<u64>,
    first_pts_ms: Option<i64>,
    last_duration_ms: u64,
    eof: bool,
}
impl Slot {
    fn new(source: Arc<dyn AssetReader>, options: MediaOptions, workers: Workers) -> Self {
        let audio_ready = !options.audio;
        Self {
            source,
            workers: Some(workers),
            options,
            status: MediaStatus {
                state: MediaState::Loading,
                position_ms: 0,
                frame_pts_ms: None,
                published_frames: 0,
                decoded_frames: 0,
                audio_started: false,
                loops_completed: 0,
                error: None,
            },
            pending: None,
            track: None,
            audio_ready,
            audio_sent: false,
            start_ms: None,
            first_pts_ms: None,
            last_duration_ms: 1,
            eof: false,
        }
    }
    fn fail(&mut self, error: String) -> anyhow::Error {
        self.status.state = MediaState::Failed;
        self.status.error = Some(error.clone());
        self.workers = None;
        anyhow!(error)
    }
}

pub struct MediaManager {
    slots: BTreeMap<(ClientId, u32), Slot>,
    video_budget: Arc<ByteBudget>,
    audio_budget: Arc<ByteBudget>,
    pending_audio: Vec<MediaAudioUpdate>,
}
impl Default for MediaManager {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
            video_budget: ByteBudget::new(MAX_QUEUED_VIDEO_BYTES),
            audio_budget: ByteBudget::new(MAX_RETAINED_AUDIO_BYTES),
            pending_audio: Vec::new(),
        }
    }
}
impl MediaManager {
    /// Decoder/clock state is an engine service resource, not serializable VM
    /// bytes. Hosts must reject snapshots or rebuild it from a supported spec.
    pub fn has_resources(&self) -> bool {
        !self.slots.is_empty()
    }
    pub fn has_active(&self) -> bool {
        self.slots
            .values()
            .any(|slot| matches!(slot.status.state, MediaState::Loading | MediaState::Playing))
    }
    pub fn play(
        &mut self,
        client: ClientId,
        id: u32,
        storage: Arc<dyn ClientStorage>,
        path: &VfsPath,
        options: MediaOptions,
        _now_ms: u64,
    ) -> Result<()> {
        ensure!(
            path.mount() == Mount::Asset,
            "media source must use asset capability"
        );
        options.validate()?;
        if options.audio {
            ensure!(
                !self
                    .slots
                    .iter()
                    .any(|((owner, other), slot)| *owner == client
                        && *other != id
                        && slot.options.audio
                        && slot.options.audio_channel == options.audio_channel
                        && matches!(slot.status.state, MediaState::Loading | MediaState::Playing)),
                "media audio channel is already reserved by another resource in this client"
            );
        }
        if !self.slots.contains_key(&(client, id)) {
            ensure!(
                self.slots.len() < MAX_MEDIA_RESOURCES,
                "media resource count budget"
            );
            ensure!(
                self.slots.keys().filter(|(c, _)| *c == client).count() < MAX_MEDIA_PER_CLIENT,
                "client media resource count budget"
            );
        }
        let reader = storage
            .open_asset(path)
            .map_err(|e| anyhow!("open scoped media asset: {e}"))?;
        let workers = Workers::new(
            reader.clone(),
            options.audio,
            self.video_budget.clone(),
            self.audio_budget.clone(),
        )?;
        let next = Slot::new(reader, options, workers);
        if let Some(previous) = self.slots.insert((client, id), next) {
            if previous.audio_sent {
                self.pending_audio.push(MediaAudioUpdate {
                    client,
                    request: AudioRequest::Stop {
                        channel: previous.options.audio_channel,
                        fade_ms: 0.,
                    },
                });
            }
        }
        Ok(())
    }
    pub fn stop(&mut self, client: ClientId, id: u32) -> Result<()> {
        let slot = self
            .slots
            .get_mut(&(client, id))
            .context("media handle not found in client scope")?;
        if slot.audio_sent {
            self.pending_audio.push(MediaAudioUpdate {
                client,
                request: AudioRequest::Stop {
                    channel: slot.options.audio_channel,
                    fade_ms: 0.,
                },
            });
        }
        slot.workers = None;
        slot.pending = None;
        slot.track = None;
        slot.audio_sent = false;
        slot.status.state = MediaState::Stopped;
        slot.status.audio_started = false;
        Ok(())
    }
    pub fn remove(&mut self, client: ClientId, id: u32) -> Result<()> {
        self.stop(client, id)?;
        self.slots.remove(&(client, id));
        Ok(())
    }
    pub fn status(&self, client: ClientId, id: u32) -> Option<MediaStatus> {
        self.slots.get(&(client, id)).map(|s| s.status.clone())
    }
    pub fn take_audio(&mut self) -> Vec<MediaAudioUpdate> {
        std::mem::take(&mut self.pending_audio)
    }
    pub fn inspect(&self) -> serde_json::Value {
        serde_json::json!({"resources":self.slots.iter().map(|((client,id),s)|serde_json::json!({"client":client.0,"id":id,"status":s.status})).collect::<Vec<_>>(),
            "queued_video_bytes":self.video_budget.used.load(Ordering::Acquire),"retained_audio_bytes":self.audio_budget.used.load(Ordering::Acquire)})
    }
    pub fn tick(&mut self, now_ms: u64) -> Result<MediaUpdates> {
        // Surface worker errors before consuming any other resource's outputs.
        for ((client, id), slot) in &mut self.slots {
            if let Some(error) = slot.workers.as_ref().and_then(Workers::error) {
                return Err(slot.fail(format!("media client {} resource {id}: {error}", client.0)));
            }
        }
        let mut updates = MediaUpdates {
            frames: Vec::new(),
            audio: self.take_audio(),
        };
        for ((client, id), slot) in &mut self.slots {
            let Some(workers) = &mut slot.workers else {
                continue;
            };
            slot.status.decoded_frames = workers.decoded.load(Ordering::Relaxed) as u64;
            if !slot.audio_ready {
                if let Some(rx) = &workers.audio {
                    match rx.try_recv() {
                        Ok(track) => {
                            slot.track = track;
                            slot.audio_ready = true;
                            workers.audio = None;
                        }
                        Err(TryRecvError::Empty) => {}
                        Err(TryRecvError::Disconnected) => {
                            let error = workers.error().unwrap_or_else(|| {
                                "media audio worker ended without an explicit track result".into()
                            });
                            return Err(slot.fail(error));
                        }
                    }
                }
            }
            if slot.pending.is_none() && !slot.eof {
                match workers.video.try_recv() {
                    Ok(frame) => slot.pending = Some(frame),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        if let Some(error) = workers.error() {
                            return Err(slot.fail(error));
                        }
                        slot.eof = true;
                    }
                }
            }
            if slot.start_ms.is_none() && slot.audio_ready && slot.pending.is_some() {
                slot.start_ms = Some(now_ms);
                slot.last_duration_ms = workers.frame_ms.load(Ordering::Acquire);
                slot.first_pts_ms = slot.pending.as_ref().map(|f| f.frame.pts_ms);
                slot.status.state = MediaState::Playing;
            }
            let Some(start) = slot.start_ms else { continue };
            let elapsed = now_ms.saturating_sub(start);
            slot.status.position_ms = elapsed;
            if !slot.audio_sent {
                if let Some(track) = &slot.track {
                    let relative = track
                        .first_pts_ms
                        .saturating_sub(slot.first_pts_ms.unwrap_or(track.first_pts_ms));
                    if relative <= 0 || elapsed >= relative as u64 {
                        // Catch up to the same logical presentation clock if a
                        // tick crosses the audio track's first PTS.
                        let skip = if relative >= 0 {
                            elapsed.saturating_sub(relative as u64)
                        } else {
                            elapsed.saturating_add(relative.unsigned_abs())
                        };
                        updates.audio.push(MediaAudioUpdate {
                            client: *client,
                            request: AudioRequest::Play {
                                channel: slot.options.audio_channel,
                                source: EncodedAudio {
                                    bytes: track.bytes.clone(),
                                    frame_limit: None,
                                },
                                intro: None,
                                parameters: AudioParameters {
                                    gain: slot.options.gain,
                                    start_ms: skip,
                                    bus: PcmBus::Music,
                                    ..Default::default()
                                },
                            },
                        });
                        slot.audio_sent = true;
                        slot.status.audio_started = true;
                    }
                }
            }
            let mut presented = None;
            loop {
                if slot.pending.is_none() && !slot.eof {
                    match workers.video.try_recv() {
                        Ok(frame) => slot.pending = Some(frame),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            if let Some(error) = workers.error() {
                                return Err(slot.fail(error));
                            }
                            slot.eof = true;
                        }
                    }
                }
                let Some(frame) = slot.pending.as_ref() else {
                    break;
                };
                let relative = frame
                    .frame
                    .pts_ms
                    .saturating_sub(slot.first_pts_ms.unwrap_or(frame.frame.pts_ms))
                    .max(0) as u64;
                if relative > elapsed {
                    break;
                }
                let frame = slot.pending.take().expect("pending checked");
                if let Some(previous) = slot.status.frame_pts_ms {
                    let delta = frame.frame.pts_ms.saturating_sub(previous);
                    if delta > 0 {
                        slot.last_duration_ms = delta as u64;
                    }
                }
                slot.status.frame_pts_ms = Some(frame.frame.pts_ms);
                presented = Some(frame);
            }
            if let Some(frame) = presented {
                slot.status.published_frames += 1;
                updates.frames.push(MediaFrameUpdate {
                    client: *client,
                    id: *id,
                    frame: frame.frame,
                });
            }
            let last = slot
                .status
                .frame_pts_ms
                .zip(slot.first_pts_ms)
                .map(|(last, first)| last.saturating_sub(first).max(0) as u64);
            if slot.eof
                && slot.pending.is_none()
                && last.is_some_and(|pts| elapsed >= pts.saturating_add(slot.last_duration_ms))
            {
                if slot.audio_sent {
                    updates.audio.push(MediaAudioUpdate {
                        client: *client,
                        request: AudioRequest::Stop {
                            channel: slot.options.audio_channel,
                            fade_ms: 0.,
                        },
                    });
                }
                slot.audio_sent = false;
                slot.status.audio_started = false;
                if slot.options.looping {
                    slot.workers = None;
                    slot.workers = Some(Workers::new(
                        slot.source.clone(),
                        slot.options.audio,
                        self.video_budget.clone(),
                        self.audio_budget.clone(),
                    )?);
                    slot.status.loops_completed += 1;
                    slot.status.state = MediaState::Loading;
                    slot.pending = None;
                    slot.track = None;
                    slot.audio_ready = !slot.options.audio;
                    slot.start_ms = None;
                    slot.first_pts_ms = None;
                    slot.eof = false;
                } else {
                    slot.workers = None;
                    slot.track = None;
                    slot.status.state = MediaState::Ended;
                }
            }
        }
        Ok(updates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{Error, service::MemoryAssetReader};
    use std::time::{Duration, Instant};
    struct Storage(Vec<u8>);
    impl ClientStorage for Storage {
        fn read(&self, p: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            if p.mount() == Mount::Asset && p.relative() == "movie.mpg" {
                Ok(self.0.clone())
            } else {
                Err(Error::NotFound)
            }
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }
    fn fixture() -> Arc<dyn ClientStorage> {
        Arc::new(Storage(
            include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg").to_vec(),
        ))
    }
    fn path() -> VfsPath {
        VfsPath::new(Mount::Asset, "movie.mpg").unwrap()
    }
    fn options() -> MediaOptions {
        MediaOptions {
            audio: false,
            ..Default::default()
        }
    }
    fn wait(manager: &mut MediaManager, mut ready: impl FnMut(&MediaManager) -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            manager.tick(0).unwrap();
            if ready(manager) {
                break;
            }
            assert!(Instant::now() < until, "decoder did not reach checkpoint");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn media_is_a_client_scoped_resource_with_bounded_queue_and_cancellable_stop() {
        let mut manager = MediaManager::default();
        manager
            .play(ClientId(1), 7, fixture(), &path(), options(), 0)
            .unwrap();
        assert!(manager.status(ClientId(2), 7).is_none());
        assert!(manager.stop(ClientId(2), 7).is_err());
        std::thread::sleep(Duration::from_millis(30));
        let workers = manager
            .slots
            .get(&(ClientId(1), 7))
            .unwrap()
            .workers
            .as_ref()
            .unwrap();
        assert!(workers.decoded.load(Ordering::Acquire) <= VIDEO_QUEUE_FRAMES + 1);
        wait(&mut manager, |m| {
            m.status(ClientId(1), 7).unwrap().state == MediaState::Playing
        });
        assert_eq!(manager.status(ClientId(1), 7).unwrap().position_ms, 0);
        manager.stop(ClientId(1), 7).unwrap();
        assert_eq!(
            manager.status(ClientId(1), 7).unwrap().state,
            MediaState::Stopped
        );
        let until = Instant::now() + Duration::from_secs(2);
        while manager.video_budget.used.load(Ordering::Acquire) > 0 {
            assert!(
                Instant::now() < until,
                "cancellation did not release queued buffers"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn generic_media_clock_publishes_resources_then_eos_and_loop_are_explicit() {
        let mut manager = MediaManager::default();
        manager
            .play(ClientId(1), 1, fixture(), &path(), options(), 0)
            .unwrap();
        wait(&mut manager, |m| {
            m.status(ClientId(1), 1).unwrap().state == MediaState::Playing
        });
        let until = Instant::now() + Duration::from_secs(5);
        let mut now = 0;
        let mut frames = 0;
        while manager.status(ClientId(1), 1).unwrap().state != MediaState::Ended {
            now += 50;
            let updates = manager.tick(now).unwrap();
            for update in updates.frames {
                assert_eq!((update.client, update.id), (ClientId(1), 1));
                assert!(update.frame.width > 0 && update.frame.height > 0);
                frames += 1;
            }
            assert!(Instant::now() < until, "media never reported EOS");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(frames > 0);
        assert_eq!(manager.status(ClientId(1), 1).unwrap().decoded_frames, 20);
        let mut looped = options();
        looped.looping = true;
        manager
            .play(ClientId(1), 1, fixture(), &path(), looped, now)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while manager.status(ClientId(1), 1).unwrap().loops_completed == 0 {
            now += 100;
            manager.tick(now).unwrap();
            assert!(Instant::now() < until, "loop did not restart");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_ne!(
            manager.status(ClientId(1), 1).unwrap().state,
            MediaState::Ended
        );
    }
    #[test]
    fn media_audio_enters_shared_channel_mixer_without_a_vm_or_output_device() {
        let mut manager = MediaManager::default();
        let storage = Arc::new(Storage(
            include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg").to_vec(),
        ));
        manager
            .play(
                ClientId(3),
                8,
                storage,
                &path(),
                MediaOptions {
                    audio_channel: 0x7000_0008,
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        let mut mixer = crysirror_audio::channels::ChannelAudio::default();
        let mut now = 0;
        let mut peak = 0f32;
        let mut plays = 0;
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let updates = manager.tick(now).unwrap();
            for update in updates.audio {
                assert_eq!(update.client, ClientId(3));
                if matches!(&update.request, AudioRequest::Play { .. }) {
                    plays += 1;
                }
                mixer.submit(update.client.0, update.request).unwrap();
            }
            mixer.render(16).unwrap();
            peak = mixer
                .take_output()
                .iter()
                .flatten()
                .fold(peak, |peak, sample| peak.max(sample.abs()));
            if manager.status(ClientId(3), 8).unwrap().state == MediaState::Ended {
                break;
            }
            assert!(
                Instant::now() < until,
                "media audio pipeline did not finish"
            );
            now += 16;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(plays, 1);
        assert!(
            peak > 0.01,
            "decoded media audio did not reach shared mixer output"
        );
    }
    #[test]
    fn invalid_capabilities_and_background_decode_errors_do_not_report_successful_eos() {
        let mut manager = MediaManager::default();
        let saved = VfsPath::new(Mount::Save, "movie.mpg").unwrap();
        assert!(
            manager
                .play(ClientId(1), 1, fixture(), &saved, options(), 0)
                .is_err()
        );
        assert!(manager.status(ClientId(1), 1).is_none());
        manager
            .play(
                ClientId(1),
                1,
                Arc::new(Storage(b"not a movie".to_vec())),
                &path(),
                options(),
                0,
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Err(error) = manager.tick(0) {
                assert!(error.to_string().contains("no video frames"));
                break;
            }
            assert!(Instant::now() < until, "worker error was swallowed");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            manager.status(ClientId(1), 1).unwrap().state,
            MediaState::Failed
        );
    }
    struct BrokenReader;
    impl AssetReader for BrokenReader {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(64)
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> crysirror_core::Result<usize> {
            Err(Error::Io("synthetic media read fault".into()))
        }
    }
    struct BrokenStorage;
    impl ClientStorage for BrokenStorage {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            panic!("must use capability stream")
        }
        fn open_asset(&self, _: &VfsPath) -> crysirror_core::Result<Arc<dyn AssetReader>> {
            Ok(Arc::new(BrokenReader))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn reader_failure_reaches_inspection_and_decoder_reservations_are_bounded() {
        let budget = ByteBudget::new(4);
        let reservation = budget.reserve(4).unwrap();
        assert!(budget.reserve(1).is_err());
        drop(reservation);
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
        let mut manager = MediaManager::default();
        manager
            .play(
                ClientId(1),
                1,
                Arc::new(BrokenStorage),
                &path(),
                options(),
                0,
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Err(error) = manager.tick(0) {
                assert!(error.to_string().contains("synthetic media read fault"));
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            manager.inspect()["resources"][0]["status"]["state"],
            "failed"
        );
        let mut cursor =
            AssetCursor::new(Arc::new(MemoryAssetReader(Arc::from(&b"abcd"[..])))).unwrap();
        assert_eq!(cursor.seek(SeekFrom::End(-1)).unwrap(), 3);
        let mut b = [0; 2];
        assert_eq!(cursor.read(&mut b).unwrap(), 1);
        assert_eq!(b[0], b'd');
        assert!(cursor.seek(SeekFrom::Current(1)).is_err());
    }
    #[test]
    #[ignore = "requires CRYSIRROR_MEDIA_FIXTURE pointing to an owner-supplied local MPEG; no game data is committed"]
    fn local_complete_movie_uses_only_engine_media_and_shared_audio_apis() {
        let path = std::path::PathBuf::from(
            std::env::var_os("CRYSIRROR_MEDIA_FIXTURE")
                .expect("set explicit owner-supplied MPEG path"),
        );
        let before = std::fs::metadata(&path).unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut fs = crate::vfs::DiskVfs::new(path.parent().unwrap(), state.path()).unwrap();
        fs.register(ClientId(1), "media-test", "local").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let asset = VfsPath::new(Mount::Asset, name).unwrap();
        let mut manager = MediaManager::default();
        manager
            .play(
                ClientId(1),
                1,
                fs.storage(ClientId(1)).unwrap(),
                &asset,
                MediaOptions::default(),
                0,
            )
            .unwrap();
        let mut mixer = crysirror_audio::channels::ChannelAudio::default();
        let started = Instant::now();
        let mut now = 0;
        let mut peak = 0f32;
        let mut frames = 0;
        let mut audio_plays = 0;
        loop {
            let updates = manager.tick(now).unwrap();
            for frame in updates.frames {
                assert!(frame.frame.width > 0 && frame.frame.height > 0);
                frames += 1;
            }
            for audio in updates.audio {
                if matches!(&audio.request, AudioRequest::Play { .. }) {
                    audio_plays += 1;
                }
                mixer.submit(audio.client.0, audio.request).unwrap();
            }
            mixer.render(16).unwrap();
            peak = mixer
                .take_output()
                .iter()
                .flatten()
                .fold(peak, |p, s| p.max(s.abs()));
            if manager.status(ClientId(1), 1).unwrap().state == MediaState::Ended {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(180),
                "local movie timed out"
            );
            now += 16;
            std::thread::sleep(Duration::from_micros(500));
        }
        let status = manager.status(ClientId(1), 1).unwrap();
        assert!(status.decoded_frames > 100 && frames > 100);
        assert_eq!(audio_plays, 1);
        assert!(peak > 0.01);
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        eprintln!(
            "{}",
            serde_json::json!({"test":"shared-media-without-vm","status":status,"published_updates":frames,"audio_plays":audio_plays,"mixed_peak":peak,"real_seconds":started.elapsed().as_secs_f64(),"virtual_ms":now})
        );
    }
}
