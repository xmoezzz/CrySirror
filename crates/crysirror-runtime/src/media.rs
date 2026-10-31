// SPDX-License-Identifier: MPL-2.0
//! Engine-owned incremental media playback. A media resource owns a bounded
//! decoder queue and its clock; a VM only issues resource commands and queries.
//! Decoded video frames are individual texture resources, never engine screens.
use anyhow::{Context, Result, anyhow, ensure};
#[path="media_probe.rs"]
pub mod probe;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AssetReader, AudioParameters, AudioRequest, ClientStorage, PcmAudio, PcmBus},
    state::ClientId,
};
use crysirror_video::{
    amv, asf, isobmff, theora,
    mpeg::{self, VideoFrame},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

pub const MAX_MEDIA_RESOURCES: usize = 32;
pub const MAX_MEDIA_PER_CLIENT: usize = 16;
pub const VIDEO_QUEUE_FRAMES: usize = 4;
pub const MAX_ENCODED_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_QUEUED_VIDEO_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_AUDIO_SAMPLES: usize = 16 * 1024 * 1024;
pub const MAX_RETAINED_AUDIO_BYTES: usize = 128 * 1024 * 1024;

/// Presentation time source. External samples select a caller-owned monotonic
/// time domain; the engine still owns playback and pause/seek/loop state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum MediaClock {
    #[default]
    Engine,
    External { now_ms: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaOptions {
    pub looping: bool,
    pub audio: bool,
    pub gain: f32,
    /// The host reserves this channel within the same client's audio namespace.
    pub audio_channel: u32,
    /// External clocks currently require no decoded audio track. Audio-master
    /// clock mapping must be implemented before accelerated game time can be
    /// used with PCM; mismatched rates never silently desynchronize.
    pub clock: MediaClock,
}
impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            looping: false,
            audio: true,
            gain: 1.,
            audio_channel: 0x7000_0000,
            clock: MediaClock::Engine,
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
    /// The presentation clock is frozen by `pause`.
    #[serde(default)]
    pub paused: bool,
    /// Picture duration reported by the decoder; 0 until it is known.
    #[serde(default)]
    pub frame_ms: u64,
    /// Display geometry from a validated decoder header, pending until ready.
    pub size: Option<[u32; 2]>,
    /// Declared container duration; absent metadata is never inferred from queues.
    pub duration_ms: Option<u64>,
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

/// Opening and length queries are worker work. Audio/video share one opened
/// capability while keeping independent cursors, including across replay loops.
struct MediaInput {
    storage: Arc<dyn ClientStorage>,
    path: VfsPath,
    reader: OnceLock<core::result::Result<Arc<dyn AssetReader>, String>>,
}
impl MediaInput {
    fn reader(&self) -> Result<Arc<dyn AssetReader>> {
        self.reader
            .get_or_init(|| {
                self.storage
                    .open_asset(&self.path)
                    .map_err(|e| format!("open scoped media asset: {e}"))
            })
            .as_ref()
            .map(Arc::clone)
            .map_err(|e| anyhow!("{e}"))
    }
}
struct AssetCursor {
    inner: crysirror_resource::AssetIo,
}
impl AssetCursor {
    fn new(source: Arc<dyn AssetReader>) -> Result<Self> {
        let inner = crysirror_resource::AssetIo::new(source)?;
        ensure!(
            inner.len() > 0 && inner.len() <= MAX_ENCODED_BYTES,
            "media encoded stream budget (1 GiB)"
        );
        Ok(Self { inner })
    }
}
impl Read for AssetCursor {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.inner.read(out)
    }
}
impl Seek for AssetCursor {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(v) => i128::from(v),
            SeekFrom::Current(v) => i128::from(self.inner.stream_position()?) + i128::from(v),
            SeekFrom::End(v) => i128::from(self.inner.len()) + i128::from(v),
        };
        if !(0..=i128::from(self.inner.len())).contains(&target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "media seek outside capability stream",
            ));
        }
        self.inner.seek(SeekFrom::Start(target as u64))
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
    fn acquire(&self, bytes: usize) -> Result<()> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|sum| *sum <= self.limit)
            })
            .map_err(|_| anyhow!("media retained decoder memory budget exceeded"))?;
        Ok(())
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation> {
        self.acquire(bytes)?;
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
impl Reservation {
    fn grow(&mut self, bytes: usize) -> Result<()> {
        let next = self
            .bytes
            .checked_add(bytes)
            .context("media reservation overflow")?;
        self.budget.acquire(bytes)?;
        self.bytes = next;
        Ok(())
    }
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
    pcm: PcmAudio,
    first_pts_ms: i64,
}
enum DecodedSamples {I16(Vec<i16>), F32(Vec<f32>)}
impl DecodedSamples {
    fn len(&self)->usize {match self {Self::I16(v)=>v.len(),Self::F32(v)=>v.len()}}
    fn append(&self, builder:&mut crysirror_audio::pcm::PcmBuilder)->Result<()> {
        match self {Self::I16(v)=>builder.append_i16(v),Self::F32(v)=>builder.append_f32(v)}
    }
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
/// Probe without consuming the capability-backed reader's cursor.
fn mp4_input(reader: &mut (impl Read + Seek)) -> Result<bool> {
    let start = reader.stream_position()?;
    let mut head = [0u8; 8];
    let result = reader.read_exact(&mut head);
    reader.seek(SeekFrom::Start(start))?;
    result.context("media container header")?;
    Ok(isobmff::is_mp4(&head))
}

struct Workers {
    video: Receiver<QueuedFrame>,
    audio: Option<Receiver<Option<AudioTrack>>>,
    cancel: Arc<AtomicBool>,
    journal: Arc<Mutex<Journal>>,
    decoded: Arc<AtomicUsize>,
    frame_ms: Arc<AtomicU64>,
    metadata: Arc<Mutex<Option<crysirror_video::VideoMetadata>>>,
}
impl Drop for Workers {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Workers {
    fn new(
        source: Arc<MediaInput>,
        audio: bool,
        video_budget: Arc<ByteBudget>,
        audio_budget: Arc<ByteBudget>,
        worker_budget: Arc<ByteBudget>,
    ) -> Result<Self> {
        let video_input = source.clone();
        let video_lease = worker_budget
            .reserve(1)
            .context("media decoder worker capacity exceeded")?;
        let audio_lease = if audio {
            Some(
                worker_budget
                    .reserve(1)
                    .context("media decoder worker capacity exceeded")?,
            )
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
        let metadata = Arc::new(Mutex::new(None));
        let stream_metadata = metadata.clone();
        std::thread::Builder::new()
            .name("crysirror-media-video".into())
            .spawn(move || {
                let _worker_lease = video_lease;
                // Retain sender until the error journal has been committed. A
                // disconnected receiver can then never mistake an error for EOS.
                let _keep_sender = video_tx.clone();
                protected(&errors, || {
                    if stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let mut video_reader = AssetCursor::new(video_input.reader()?)?;
                    let mp4 = mp4_input(&mut video_reader)?;
                    let publish_metadata = |info: crysirror_video::VideoMetadata| -> Result<()> {
                        crysirror_image::rgba_len(info.size[0], info.size[1])?;
                        if let Some(ms) = info.frame_ms {
                            ensure!(ms.is_finite() && ms > 0. && ms <= 10000., "media header frame duration");
                            frame_duration.store(ms.round().max(1.) as u64, Ordering::Release);
                        }
                        *stream_metadata.lock().unwrap_or_else(|e| e.into_inner()) = Some(info);
                        Ok(())
                    };
                    let publish = |frame: VideoFrame| -> Result<bool> {
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
                    };
                    if mp4 {
                        let mut decoder = isobmff::Mp4Stream::new(video_reader)?;
                        publish_metadata(crysirror_video::VideoMetadata {
                            size: [decoder.info().width, decoder.info().height],
                            duration_ms: None,
                            frame_ms: decoder.info().fps.map(|fps| 1000. / fps),
                        })?;
                        while let Some(frame) =
                            decoder.next_frame(|| stop.load(Ordering::Acquire))?
                        {
                            if !publish(frame)? {
                                break;
                            }
                        }
                    } else if asf::is_asf(&mut video_reader)? {
                        asf::decode_video_with_metadata(video_reader, || stop.load(Ordering::Acquire), publish_metadata, publish)?;
                    } else if theora::is_input(&mut video_reader)? {
                        theora::decode_video_with_metadata(video_reader, || stop.load(Ordering::Acquire), publish_metadata, publish)?;
                    } else if amv::is_input(&mut video_reader)? {
                        let info = amv::info(&mut video_reader)?;
                        let numerator = u128::from(info.frames) * u128::from(info.scale) * 1000;
                        let denominator = u128::from(info.rate);
                        publish_metadata(crysirror_video::VideoMetadata {
                            size: [info.width, info.height],
                            duration_ms: Some(u64::try_from((numerator + denominator / 2) / denominator)?),
                            frame_ms: Some(info.frame_ms),
                        })?;
                        amv::decode_video(video_reader, || stop.load(Ordering::Acquire), publish)?;
                    } else {
                        if let Some(info) = mpeg::sequence_info(&mut video_reader)? {
                            publish_metadata(crysirror_video::VideoMetadata {
                                size: [info.width, info.height],
                                duration_ms: None,
                                frame_ms: Some(info.frame_ms),
                            })?;
                        }
                        mpeg::decode_video(video_reader, || stop.load(Ordering::Acquire), publish)?;
                    }
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
            metadata,
        };
        if let Some(audio_lease) = audio_lease {
            let (tx, rx) = mpsc::sync_channel(1);
            let stop = workers.cancel.clone();
            let errors = workers.journal.clone();
            std::thread::Builder::new()
                .name("crysirror-media-audio".into())
                .spawn(move || {
                    let _keep_sender = tx.clone();
                    let _worker_lease = audio_lease;
                    protected(&errors, || {
                        if stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let mut reader = AssetCursor::new(source.reader()?)?;
                        if mp4_input(&mut reader)? {
                            let decoder = isobmff::Mp4Stream::new(reader)?;
                            ensure!(!decoder.has_audio(),
                                "MP4 audio tracks are not supported by Media yet; explicitly disable audio for video-only playback");
                            let _ = tx.send(None);
                            return Ok(());
                        }
                        if amv::is_input(&mut reader)? {
                            // AlphaMovie carries no audio stream.
                            let _ = tx.send(None);
                            return Ok(());
                        }
                        let mut builder = None;
                        let mut input_samples = 0usize;
                        let mut reservation = audio_budget.reserve(0)?;
                        let mut format = None;
                        let mut first_pts_ms = 0;
                        let mut accept = |pts_ms, sample_rate, channels, samples: DecodedSamples| {
                            if stop.load(Ordering::Acquire) {
                                return Ok(false);
                            }
                            if let Some(current) = format {
                                ensure!(current == (sample_rate, channels),
                                    "media audio format changed within a track");
                            } else {
                                format = Some((sample_rate, channels));
                                first_pts_ms = pts_ms;
                                builder = Some(crysirror_audio::pcm::PcmBuilder::new(
                                    sample_rate, channels,
                                    crysirror_core::service::MAX_PCM_FRAMES)?);
                            }
                            input_samples = input_samples.checked_add(samples.len())
                                .context("media audio sample counter")?;
                            ensure!(input_samples <= MAX_AUDIO_SAMPLES,
                                "media full-track audio sample budget exceeded; streaming audio required for longer tracks");
                            ensure!(samples.len() % channels as usize == 0,
                                "media partial audio frame");
                            reservation.grow(samples.len() / channels as usize * 8)?;
                            samples.append(builder.as_mut().unwrap())?;
                            Ok(true)
                        };
                        if asf::is_asf(&mut reader)? {
                            asf::decode_audio(reader, || stop.load(Ordering::Acquire), |c| accept(c.pts_ms,c.sample_rate,c.channels,DecodedSamples::F32(c.samples)))?;
                        } else if theora::is_input(&mut reader)? {
                            theora::decode_audio(reader, || stop.load(Ordering::Acquire), |c| accept(c.pts_ms,c.sample_rate,c.channels,DecodedSamples::F32(c.samples)))?;
                        } else {
                            mpeg::decode_audio(reader, || stop.load(Ordering::Acquire), |c| accept(c.pts_ms,c.sample_rate,c.channels,DecodedSamples::I16(c.samples)))?;
                        }
                        if stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let track = if let Some(builder) = builder {
                            // Arc conversion may coexist with the staging allocation.
                            // The final lease follows mixer/update clones, not the slot.
                            let final_reservation = audio_budget.reserve(builder.byte_len())?;
                            let mut pcm = builder.finish()?;
                            pcm.retain_storage(Arc::new(final_reservation));
                            drop(reservation);
                            Some(AudioTrack { pcm, first_pts_ms })
                        } else {
                            None
                        };
                        let _ = tx.send(track);
                        Ok(())
                    });
                })
                .context("spawn engine media audio decoder")?;
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
struct Slot {
    source: Arc<MediaInput>,
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
    current: Option<crysirror_core::service::MediaFrame>,
    /// Engine clock time at which `pause` froze the presentation clock.
    paused_at: Option<u64>,
    /// Stream position the next decode run starts presenting at (`seek`).
    seek_ms: u64,
    /// After a seek, pictures already behind the clock are decoded but not
    /// published, so the target appears without a fast-forward flash.
    catching_up: bool,
}
impl Slot {
    fn clock_ms(&self, engine_ms: u64) -> u64 {
        match self.options.clock {
            MediaClock::Engine => engine_ms,
            MediaClock::External { now_ms } => now_ms,
        }
    }
    fn refresh_metadata(&mut self) {
        if let Some(info) = self.workers.as_ref().and_then(|w| *w.metadata.lock().unwrap_or_else(|e| e.into_inner())) {
            self.status.size = Some(info.size);
            self.status.duration_ms = info.duration_ms;
        }
    }
    fn new(source: Arc<MediaInput>, options: MediaOptions, workers: Workers) -> Self {
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
                paused: false,
                frame_ms: 0,
                size: None,
                duration_ms: None,
            },
            pending: None,
            track: None,
            audio_ready,
            audio_sent: false,
            start_ms: None,
            first_pts_ms: None,
            last_duration_ms: 1,
            eof: false,
            current: None,
            paused_at: None,
            seek_ms: 0,
            catching_up: false,
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
    worker_budget: Arc<ByteBudget>,
    pending_audio: Vec<MediaAudioUpdate>,
}
impl Default for MediaManager {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
            video_budget: ByteBudget::new(MAX_QUEUED_VIDEO_BYTES),
            audio_budget: ByteBudget::new(MAX_RETAINED_AUDIO_BYTES),
            worker_budget: ByteBudget::new(MAX_MEDIA_RESOURCES * 2),
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
        let reader = Arc::new(MediaInput {
            storage,
            path: path.clone(),
            reader: OnceLock::new(),
        });
        let workers = Workers::new(
            reader.clone(),
            options.audio,
            self.video_budget.clone(),
            self.audio_budget.clone(),
            self.worker_budget.clone(),
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
        slot.refresh_metadata();
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
        slot.current = None;
        slot.track = None;
        slot.audio_sent = false;
        slot.status.state = MediaState::Stopped;
        slot.status.audio_started = false;
        Ok(())
    }
    /// Freeze the presentation clock. Audio stops and is restarted at the
    /// matching offset on `resume`; decoded pictures stay queued.
    pub fn pause(&mut self, client: ClientId, id: u32, now_ms: u64) -> Result<()> {
        let slot = self
            .slots
            .get_mut(&(client, id))
            .context("media handle not found in client scope")?;
        if slot.paused_at.is_some() {
            return Ok(());
        }
        slot.paused_at = Some(slot.clock_ms(now_ms));
        slot.status.paused = true;
        if slot.audio_sent {
            self.pending_audio.push(MediaAudioUpdate {
                client,
                request: AudioRequest::Stop {
                    channel: slot.options.audio_channel,
                    fade_ms: 0.,
                },
            });
            slot.audio_sent = false;
            slot.status.audio_started = false;
        }
        Ok(())
    }
    pub fn resume(&mut self, client: ClientId, id: u32, now_ms: u64) -> Result<()> {
        let slot = self
            .slots
            .get_mut(&(client, id))
            .context("media handle not found in client scope")?;
        let now_ms = slot.clock_ms(now_ms);
        if let Some(paused) = slot.paused_at.take() {
            if let Some(start) = &mut slot.start_ms {
                *start = start.saturating_add(now_ms.saturating_sub(paused));
            }
        }
        slot.status.paused = false;
        Ok(())
    }
    /// Restart decoding from the beginning and present from `position_ms`;
    /// earlier pictures are decoded and dropped. Stopped and ended media
    /// restart; a paused resource stays paused at the new position.
    pub fn seek(&mut self, client: ClientId, id: u32, position_ms: u64, now_ms: u64) -> Result<()> {
        let slot = self
            .slots
            .get_mut(&(client, id))
            .context("media handle not found in client scope")?;
        let now_ms = slot.clock_ms(now_ms);
        ensure!(position_ms <= 24 * 3600 * 1000, "media seek position");
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
        slot.workers = Some(Workers::new(
            slot.source.clone(),
            slot.options.audio,
            self.video_budget.clone(),
            self.audio_budget.clone(),
            self.worker_budget.clone(),
        )?);
        slot.status.state = MediaState::Loading;
        slot.status.audio_started = false;
        slot.status.error = None;
        slot.pending = None;
        slot.track = None;
        slot.audio_ready = !slot.options.audio;
        slot.audio_sent = false;
        slot.start_ms = None;
        slot.first_pts_ms = None;
        slot.status.frame_pts_ms = None;
        slot.eof = false;
        slot.seek_ms = position_ms;
        slot.catching_up = position_ms > 0;
        if slot.paused_at.is_some() {
            slot.paused_at = Some(now_ms);
        }
        Ok(())
    }
    pub fn remove(&mut self, client: ClientId, id: u32) -> Result<()> {
        self.stop(client, id)?;
        self.slots.remove(&(client, id));
        Ok(())
    }
    /// Select external time before anchoring playback, then admit monotonic
    /// samples without touching position or manufacturing a decoder frame.
    pub fn set_clock(&mut self, client: ClientId, id: u32, now_ms: u64) -> Result<()> {
        let slot = self.slots.get_mut(&(client, id)).context("media handle not found in client scope")?;
        ensure!(slot.track.is_none(), "external media clock with decoded audio requires audio-master clock mapping");
        match slot.options.clock {
            MediaClock::Engine => {
                ensure!(slot.start_ms.is_none(), "select external media clock before first playback");
                if slot.paused_at.is_some() { slot.paused_at = Some(now_ms); }
            }
            MediaClock::External { now_ms: previous } => {
                ensure!(now_ms >= previous, "external media clock moved backwards");
            }
        }
        slot.options.clock = MediaClock::External { now_ms };
        Ok(())
    }
    pub fn status(&self, client: ClientId, id: u32) -> Option<MediaStatus> {
        self.slots.get(&(client, id)).map(|s| {
            let mut status = s.status.clone();
            if let Some(info) = s.workers.as_ref().and_then(|w| *w.metadata.lock().unwrap_or_else(|e| e.into_inner())) {
                status.size = Some(info.size);
                status.duration_ms = info.duration_ms;
            }
            if let (MediaClock::External { now_ms }, Some(start), MediaState::Playing) =
                (s.options.clock, s.start_ms, status.state)
            {
                // Clock queries observe an admitted sample immediately. Frame
                // publication remains owner-tick work and has its own PTS.
                status.position_ms = s.paused_at.unwrap_or(now_ms)
                    .saturating_sub(start).saturating_add(s.seek_ms);
            }
            status
        })
    }
    pub fn frame(&self, client: ClientId, id: u32) -> Option<crysirror_core::service::MediaFrame> {
        self.slots.get(&(client,id)).and_then(|s|s.current.clone())
    }
    pub fn take_audio(&mut self) -> Vec<MediaAudioUpdate> {
        std::mem::take(&mut self.pending_audio)
    }
    pub fn inspect(&self) -> serde_json::Value {
        serde_json::json!({"resources":self.slots.iter().map(|((client,id),s)|serde_json::json!({"client":client.0,"id":id,"status":s.status})).collect::<Vec<_>>(),
            "queued_video_bytes":self.video_budget.used.load(Ordering::Acquire),"retained_audio_bytes":self.audio_budget.used.load(Ordering::Acquire),"decoder_worker_reservations":self.worker_budget.used.load(Ordering::Acquire)})
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
            let now_ms = slot.clock_ms(now_ms);
            let Some(workers) = &mut slot.workers else {
                continue;
            };
            if let Some(info) = *workers.metadata.lock().unwrap_or_else(|e| e.into_inner()) {
                slot.status.size = Some(info.size);
                slot.status.duration_ms = info.duration_ms;
            }
            slot.status.decoded_frames = workers.decoded.load(Ordering::Relaxed) as u64;
            if slot.start_ms.is_some() {
                slot.status.frame_ms = slot.last_duration_ms;
            }
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
            if matches!(slot.options.clock, MediaClock::External { .. }) && slot.track.is_some() {
                return Err(slot.fail("external media clock with decoded audio requires audio-master clock mapping".into()));
            }
            if slot.start_ms.is_none() && slot.audio_ready && slot.pending.is_some() {
                // A paused (seeking) resource anchors its clock at the pause
                // instant; `seek_ms` offsets the stream position below.
                slot.start_ms = Some(slot.paused_at.unwrap_or(now_ms));
                slot.last_duration_ms = workers.frame_ms.load(Ordering::Acquire);
                slot.first_pts_ms = slot.pending.as_ref().map(|f| f.frame.pts_ms);
                slot.status.state = MediaState::Playing;
            }
            let Some(start) = slot.start_ms else { continue };
            let elapsed = slot
                .paused_at
                .unwrap_or(now_ms)
                .saturating_sub(start)
                .saturating_add(slot.seek_ms);
            slot.status.position_ms = elapsed;
            if !slot.audio_sent && slot.paused_at.is_none() {
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
                            request: AudioRequest::PlayPcm {
                                channel: slot.options.audio_channel,
                                source: track.pcm.clone(),
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
            let presented = presented.filter(|frame| {
                if !slot.catching_up {
                    return true;
                }
                let relative = frame
                    .frame
                    .pts_ms
                    .saturating_sub(slot.first_pts_ms.unwrap_or(frame.frame.pts_ms))
                    .max(0) as u64;
                let behind = !slot.eof
                    && slot.pending.is_none()
                    && relative.saturating_add(slot.last_duration_ms) <= elapsed;
                slot.catching_up = behind;
                !behind
            });
            if let Some(frame) = presented {
                slot.status.published_frames += 1;
                let lease=self.video_budget.reserve(frame.frame.rgba.len())?;
                let mut retained=crysirror_core::service::MediaFrame::new(
                    [frame.frame.width,frame.frame.height],frame.frame.rgba.clone(),
                    slot.status.published_frames,frame.frame.pts_ms).map_err(|e|anyhow!("{e}"))?;
                retained.retain_storage(Arc::new(lease));
                slot.current=Some(retained);
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
                    slot.workers = Some(
                        match Workers::new(
                            slot.source.clone(),
                            slot.options.audio,
                            self.video_budget.clone(),
                            self.audio_budget.clone(),
                            self.worker_budget.clone(),
                        ) {
                            Ok(workers) => workers,
                            Err(error) => {
                                // Preserve emitted audio controls when this tick fails,
                                // especially the Stop for the completed loop's voice.
                                self.pending_audio.append(&mut updates.audio);
                                return Err(slot.fail(format!("restart media loop: {error:#}")));
                            }
                        },
                    );
                    slot.status.loops_completed += 1;
                    slot.status.state = MediaState::Loading;
                    slot.pending = None;
                    slot.track = None;
                    slot.audio_ready = !slot.options.audio;
                    slot.start_ms = None;
                    slot.first_pts_ms = None;
                    slot.seek_ms = 0;
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
    fn validated_header_metadata_is_visible_before_first_tick_and_retained_on_stop() {
        let mut manager = MediaManager::default();
        manager.play(
            ClientId(1), 31,
            Arc::new(Storage(include_bytes!("../../crysirror-video/tests/fixtures/wmv_wma.asf").to_vec())),
            &path(), options(), 0,
        ).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        // Deliberately no manager.tick: metadata preparation must not require
        // a presented frame or moving the presentation clock.
        let status = loop {
            let status = manager.status(ClientId(1), 31).unwrap();
            if status.size.is_some() { break status; }
            assert!(Instant::now() < deadline, "header worker did not publish metadata");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(status.size, Some([64, 48]));
        assert!(status.duration_ms.is_some_and(|ms| ms > 0));
        assert_eq!(status.state, MediaState::Loading);
        assert_eq!((status.position_ms, status.published_frames), (0, 0));
        assert!(manager.frame(ClientId(1), 31).is_none());
        assert!(manager.status(ClientId(2), 31).is_none());
        manager.stop(ClientId(1), 31).unwrap();
        let stopped = manager.status(ClientId(1), 31).unwrap();
        assert_eq!(stopped.state, MediaState::Stopped);
        assert_eq!(stopped.size, status.size);
        assert_eq!(stopped.duration_ms, status.duration_ms);
    }
    #[test]
    fn native_mp4_codecs_publish_through_shared_media_workers() {
        for bytes in [
            include_bytes!("../../crysirror-video/tests/fixtures/h264_main.mp4").as_slice(),
            include_bytes!("../../crysirror-video/tests/fixtures/h265_10.mp4").as_slice(),
            include_bytes!("../../crysirror-video/tests/fixtures/av1_10.mp4").as_slice(),
        ] {
            let mut manager = MediaManager::default();
            let storage = Arc::new(Storage(bytes.to_vec()));
            // The storage name remains movie.mpg deliberately: dispatch is by
            // bytes and shares the same client capability and worker API.
            manager
                .play(ClientId(4), 9, storage, &path(), MediaOptions::default(), 0)
                .unwrap();
            wait(&mut manager, |m| {
                m.status(ClientId(4), 9).unwrap().state == MediaState::Playing
            });
            let mut now = 0;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut published = 0;
            while manager.status(ClientId(4), 9).unwrap().state != MediaState::Ended {
                now += 100;
                for update in manager.tick(now).unwrap().frames {
                    assert_eq!((update.client, update.id), (ClientId(4), 9));
                    assert_eq!((update.frame.width, update.frame.height), (64, 64));
                    published += 1;
                }
                assert!(
                    Instant::now() < deadline,
                    "MP4 worker did not reach EOF: {:?}",
                    manager.status(ClientId(4), 9)
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(published > 0);
            assert_eq!(manager.status(ClientId(4), 9).unwrap().decoded_frames, 5);
            assert!(manager.take_audio().is_empty());
        }
    }

    #[test]
    fn mp4_audio_is_explicitly_unsupported_instead_of_silently_discarded() {
        let mut manager = MediaManager::default();
        let storage = Arc::new(Storage(
            include_bytes!("../../crysirror-video/tests/fixtures/h264_aac.mp4").to_vec(),
        ));
        manager
            .play(ClientId(4), 9, storage, &path(), MediaOptions::default(), 0)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Err(error) = manager.tick(0) {
                assert!(error.to_string().contains("MP4 audio tracks"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "MP4 audio error was not delivered"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            manager
                .status(ClientId(4), 9)
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("MP4 audio tracks")
        );
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
    fn retained_movie_capacity_is_client_scoped_global_bounded_and_reusable() {
        let mut manager = MediaManager::default();
        for client in [ClientId(1), ClientId(2)] {
            for id in 0..MAX_MEDIA_PER_CLIENT as u32 {
                manager.play(client, id, fixture(), &path(), options(), 0).unwrap();
            }
            assert!(manager.play(client, MAX_MEDIA_PER_CLIENT as u32, fixture(), &path(), options(), 0).is_err());
        }
        assert_eq!(manager.slots.len(), MAX_MEDIA_RESOURCES);
        assert!(manager.play(ClientId(3), 0, fixture(), &path(), options(), 0).is_err());
        assert!(manager.remove(ClientId(3), 0).is_err());
        manager.remove(ClientId(1), 0).unwrap();
        manager.play(ClientId(3), 0, fixture(), &path(), options(), 0).unwrap();
        assert_eq!(manager.slots.len(), MAX_MEDIA_RESOURCES);
        assert!(manager.video_budget.used.load(Ordering::Acquire) <= MAX_QUEUED_VIDEO_BYTES);
    }
    #[test]
    fn immutable_media_picture_is_scoped_and_retains_its_budget_past_stop() {
        let mut manager=MediaManager::default();
        manager.play(ClientId(1),3,fixture(),&path(),options(),0).unwrap();
        wait(&mut manager,|m|m.status(ClientId(1),3).unwrap().published_frames>0);
        assert!(manager.frame(ClientId(2),3).is_none());
        let frame=manager.frame(ClientId(1),3).unwrap();
        assert_eq!(frame.rgba().len(),frame.size()[0] as usize*frame.size()[1] as usize*4);
        let retained=frame.rgba().len();
        manager.stop(ClientId(1),3).unwrap();
        assert!(manager.frame(ClientId(1),3).is_none());
        let until=Instant::now()+Duration::from_secs(2);
        while manager.video_budget.used.load(Ordering::Acquire)!=retained {
            assert!(Instant::now()<until,"frame/queue lease accounting");
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(frame);
        assert_eq!(manager.video_budget.used.load(Ordering::Acquire),0);
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
    fn pause_freezes_the_clock_and_a_paused_seek_presents_only_the_target() {
        // MP4 carries per-frame timestamps (the MPEG fixture repeats one PTS).
        let storage: Arc<dyn ClientStorage> = Arc::new(Storage(
            include_bytes!("../../crysirror-video/tests/fixtures/h264_main.mp4").to_vec(),
        ));
        let mut manager = MediaManager::default();
        let (c, id) = (ClientId(1), 1);
        manager.play(c, id, storage, &path(), options(), 0).unwrap();
        wait(&mut manager, |m| m.status(c, id).unwrap().frame_pts_ms.is_some());
        let first = manager.status(c, id).unwrap().frame_pts_ms.unwrap();
        manager.pause(c, id, 0).unwrap();
        let published = manager.status(c, id).unwrap().published_frames;
        for now in [500, 1000] {
            manager.tick(now).unwrap();
        }
        let status = manager.status(c, id).unwrap();
        assert!(status.paused);
        assert_eq!((status.position_ms, status.published_frames), (0, published));
        manager.resume(c, id, 1000).unwrap();
        manager.tick(1040).unwrap();
        assert_eq!(manager.status(c, id).unwrap().position_ms, 40);
        // Seek while paused: decoding restarts, the clock holds at 200 ms and
        // exactly one picture (the one due at 200 ms) is published.
        manager.pause(c, id, 1040).unwrap();
        manager.seek(c, id, 200, 1040).unwrap();
        let before = manager.status(c, id).unwrap().published_frames;
        let until = Instant::now() + Duration::from_secs(5);
        let mut pts = None;
        while pts.is_none() {
            let updates = manager.tick(1040).unwrap();
            assert!(updates.frames.len() <= 1);
            pts = updates.frames.first().map(|f| f.frame.pts_ms);
            assert!(Instant::now() < until, "seek never presented");
            std::thread::sleep(Duration::from_millis(1));
        }
        let status = manager.status(c, id).unwrap();
        assert_eq!(status.position_ms, 200);
        assert_eq!(status.published_frames, before + 1, "intermediate pictures were published");
        let pts = pts.unwrap() - first;
        assert!((100..=200).contains(&pts), "picture at {pts} ms for a 200 ms seek");
        for _ in 0..20 {
            assert!(manager.tick(1040).unwrap().frames.is_empty());
        }
    }
    #[test]
    fn external_clock_owns_progress_and_pause_seek_compensation() {
        let mut manager = MediaManager::default();
        let c = ClientId(1);
        manager.play(c, 21, Arc::new(Storage(include_bytes!("../../crysirror-video/tests/fixtures/h264_main.mp4").to_vec())),
            &path(), options(), 0).unwrap();
        manager.set_clock(c, 21, 500).unwrap();
        wait(&mut manager, |m| m.status(c, 21).unwrap().published_frames > 0);
        manager.set_clock(c, 21, 540).unwrap();
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 40);
        manager.tick(999_999).unwrap();
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 40);
        manager.pause(c, 21, 1_000_000).unwrap();
        manager.set_clock(c, 21, 2000).unwrap();
        manager.tick(2_000_000).unwrap();
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 40);
        manager.resume(c, 21, 9_000_000).unwrap();
        manager.set_clock(c, 21, 2040).unwrap();
        manager.tick(3).unwrap();
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 80);
        assert!(manager.set_clock(c, 21, 2039).is_err());
        assert!(manager.set_clock(ClientId(2), 21, 2040).is_err());
        manager.pause(c, 21, 4).unwrap();
        manager.seek(c, 21, 200, 5).unwrap();
        manager.set_clock(c, 21, 5000).unwrap();
        wait(&mut manager, |m| m.status(c, 21).unwrap().state == MediaState::Playing);
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 200);
        manager.resume(c, 21, 6).unwrap();
        manager.set_clock(c, 21, 5016).unwrap();
        manager.tick(7).unwrap();
        assert_eq!(manager.status(c, 21).unwrap().position_ms, 216);
    }
    #[test]
    fn external_clock_rejects_decoded_audio_before_pcm_or_picture_publication() {
        let mut manager = MediaManager::default();
        manager.play(ClientId(1), 22, Arc::new(Storage(include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg").to_vec())),
            &path(), MediaOptions { clock: MediaClock::External { now_ms: 0 }, ..Default::default() }, 0).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match manager.tick(0) {
                Ok(updates) => { assert!(updates.frames.is_empty() && updates.audio.is_empty()); }
                Err(error) => { assert!(error.to_string().contains("audio-master clock mapping")); break; }
            }
            assert!(Instant::now() < deadline, "audio clock mismatch was not delivered");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(manager.status(ClientId(1), 22).unwrap().published_frames, 0);
        assert_eq!(manager.status(ClientId(1), 22).unwrap().state, MediaState::Failed);
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
                if matches!(&update.request, AudioRequest::PlayPcm { .. }) {
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
    fn missing_media_open_is_a_visible_async_failure_without_pcm_publication() {
        let mut manager = MediaManager::default();
        let missing = VfsPath::new(Mount::Asset, "missing.mpg").unwrap();
        manager
            .play(ClientId(1), 1, fixture(), &missing, options(), 0)
            .unwrap();
        assert_eq!(
            manager.status(ClientId(1), 1).unwrap().state,
            MediaState::Loading
        );
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Err(error) = manager.tick(0) {
                assert!(error.to_string().contains("open scoped media asset"));
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            manager.status(ClientId(1), 1).unwrap().state,
            MediaState::Failed
        );
        assert!(
            !manager
                .take_audio()
                .iter()
                .any(|u| matches!(u.request, AudioRequest::PlayPcm { .. }))
        );
    }
    #[test]
    fn prepared_pcm_preserves_pts_lead_delay_and_late_owner_sample_skips() {
        // Audio starts 20 ms before video, on time, or 20 ms after video.
        // In the last case the owner next visits at 35 ms and must skip 15 ms.
        for (audio_pts, visit, expected_frame) in [(980, 0, 960), (1000, 0, 0), (1020, 35, 720)] {
            let mut manager = MediaManager::default();
            let (tx, rx) = mpsc::sync_channel(1);
            tx.send(QueuedFrame {
                frame: VideoFrame {
                    pts_ms: 1000,
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 255],
                },
                _reservation: manager.video_budget.reserve(4).unwrap(),
            })
            .unwrap();
            let workers = Workers {
                video: rx,
                audio: None,
                cancel: Arc::new(AtomicBool::new(false)),
                journal: Arc::new(Mutex::new(Journal::default())),
                decoded: Arc::new(AtomicUsize::new(1)),
                frame_ms: Arc::new(AtomicU64::new(34)),
                metadata: Arc::new(Mutex::new(None)),
            };
            let input = Arc::new(MediaInput {
                storage: fixture(),
                path: path(),
                reader: OnceLock::new(),
            });
            let mut slot = Slot::new(input, MediaOptions::default(), workers);
            slot.track = Some(AudioTrack {
                pcm: PcmAudio::new(48000, (0..4096).map(|n| [n as f32 / 10000.; 2]).collect())
                    .unwrap(),
                first_pts_ms: audio_pts,
            });
            slot.audio_ready = true;
            manager.slots.insert((ClientId(1), 1), slot);
            let initial = manager.tick(0).unwrap();
            let updates = if visit > 0 {
                assert!(initial.audio.is_empty());
                manager.tick(visit).unwrap()
            } else {
                initial
            };
            assert_eq!(updates.audio.len(), 1);
            let mut mixer = crysirror_audio::channels::ChannelAudio::default();
            for audio in updates.audio {
                mixer.submit(audio.client.0, audio.request).unwrap();
            }
            mixer.render(1).unwrap();
            let output = mixer.take_output();
            assert!((output[0][0] - expected_frame as f32 / 10000.).abs() < 1e-6);
            drop(tx);
        }
    }
    #[test]
    fn loop_admission_failure_is_failed_and_retains_the_completed_voices_stop() {
        let mut manager = MediaManager::default();
        manager
            .play(
                ClientId(1),
                1,
                Arc::new(Storage(
                    include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg").to_vec(),
                )),
                &path(),
                MediaOptions {
                    looping: true,
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        wait(&mut manager, |m| {
            m.status(ClientId(1), 1).unwrap().audio_started
        });
        manager.worker_budget = ByteBudget::new(0);
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Err(error) = manager.tick(10000) {
                assert!(error.to_string().contains("restart media loop"));
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            manager.status(ClientId(1), 1).unwrap().state,
            MediaState::Failed
        );
        assert!(
            manager
                .take_audio()
                .iter()
                .any(|u| matches!(u.request, AudioRequest::Stop { .. }))
        );
    }
    struct OpenGate {
        owner: std::thread::ThreadId,
        opens: AtomicUsize,
        released: Mutex<bool>,
        wake: std::sync::Condvar,
    }
    impl OpenGate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                owner: std::thread::current().id(),
                opens: AtomicUsize::new(0),
                released: Mutex::new(false),
                wake: std::sync::Condvar::new(),
            })
        }
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.wake.notify_all();
        }
        fn wait_for_opens(&self, count: usize) {
            let until = Instant::now() + Duration::from_secs(3);
            while self.opens.load(Ordering::Acquire) < count {
                assert!(Instant::now() < until, "worker did not open source");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    // Release blocked workers even if a test assertion unwinds.
    struct ReleaseGate(Arc<OpenGate>);
    impl Drop for ReleaseGate {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    struct ProbedStorage(Arc<OpenGate>);
    struct ProbedReader(Arc<OpenGate>, MemoryAssetReader);
    impl AssetReader for ProbedReader {
        fn len(&self) -> crysirror_core::Result<u64> {
            assert_ne!(
                self.0.owner,
                std::thread::current().id(),
                "length queried on owner"
            );
            self.1.len()
        }
        fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            assert_ne!(self.0.owner, std::thread::current().id(), "read on owner");
            self.1.read_at(at, out)
        }
    }
    impl ClientStorage for ProbedStorage {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            panic!("must open capability")
        }
        fn open_asset(&self, _: &VfsPath) -> crysirror_core::Result<Arc<dyn AssetReader>> {
            assert_ne!(self.0.owner, std::thread::current().id(), "open on owner");
            self.0.opens.fetch_add(1, Ordering::AcqRel);
            let mut released = self.0.released.lock().unwrap();
            while !*released {
                released = self.0.wake.wait(released).unwrap();
            }
            Ok(Arc::new(ProbedReader(
                self.0.clone(),
                MemoryAssetReader(Arc::from(
                    &include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg")[..],
                )),
            )))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(vec![])
        }
    }
    #[test]
    fn media_open_length_and_reads_stay_on_workers_and_share_one_capability() {
        let gate = OpenGate::new();
        let _release = ReleaseGate(gate.clone());
        let mut manager = MediaManager::default();
        manager
            .play(
                ClientId(1),
                1,
                Arc::new(ProbedStorage(gate.clone())),
                &path(),
                MediaOptions::default(),
                0,
            )
            .unwrap();
        gate.wait_for_opens(1);
        for _ in 0..8 {
            let update = manager.tick(0).unwrap();
            assert!(update.frames.is_empty() && update.audio.is_empty());
            assert_eq!(
                manager.status(ClientId(1), 1).unwrap().state,
                MediaState::Loading
            );
        }
        gate.release();
        wait(&mut manager, |m| {
            m.status(ClientId(1), 1).unwrap().audio_started
        });
        assert_eq!(
            gate.opens.load(Ordering::Acquire),
            1,
            "audio/video opened separate capabilities"
        );
        manager.stop(ClientId(1), 1).unwrap();
    }
    #[test]
    fn canceled_blocked_workers_keep_admission_leases_and_failed_replacement_is_atomic() {
        let gate = OpenGate::new();
        let _release = ReleaseGate(gate.clone());
        let mut manager = MediaManager::default();
        manager.worker_budget = ByteBudget::new(2);
        let budget = manager.worker_budget.clone();
        let storage = Arc::new(ProbedStorage(gate.clone()));
        for count in 1..=2 {
            manager
                .play(ClientId(1), 1, storage.clone(), &path(), options(), 0)
                .unwrap();
            gate.wait_for_opens(count);
        }
        assert_eq!(budget.used.load(Ordering::Acquire), 2);
        let old = manager.slots[&(ClientId(1), 1)].source.clone();
        let error = manager
            .play(ClientId(1), 1, storage, &path(), options(), 0)
            .unwrap_err();
        assert!(error.to_string().contains("worker capacity"));
        assert!(Arc::ptr_eq(&old, &manager.slots[&(ClientId(1), 1)].source));
        manager.stop(ClientId(1), 1).unwrap();
        assert_eq!(
            budget.used.load(Ordering::Acquire),
            2,
            "blocked workers lost their leases"
        );
        gate.release();
        let until = Instant::now() + Duration::from_secs(3);
        while budget.used.load(Ordering::Acquire) > 0 {
            assert!(
                Instant::now() < until,
                "canceled workers did not release admission"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            !manager
                .take_audio()
                .iter()
                .any(|u| matches!(u.request, AudioRequest::PlayPcm { .. }))
        );
    }
    #[test]
    fn media_pcm_reservation_follows_published_voice_past_resource_stop() {
        let mut manager = MediaManager::default();
        manager
            .play(
                ClientId(1),
                1,
                Arc::new(Storage(
                    include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg").to_vec(),
                )),
                &path(),
                MediaOptions::default(),
                0,
            )
            .unwrap();
        let budget = manager.audio_budget.clone();
        let until = Instant::now() + Duration::from_secs(5);
        let plays = loop {
            let update = manager.tick(0).unwrap();
            if update
                .audio
                .iter()
                .any(|u| matches!(u.request, AudioRequest::PlayPcm { .. }))
            {
                break update.audio;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        };
        manager.stop(ClientId(1), 1).unwrap();
        assert!(
            budget.used.load(Ordering::Acquire) > 0,
            "queued PCM lost its receipt"
        );
        let mut audio = crysirror_audio::channels::ChannelAudio::default();
        for play in plays {
            audio.submit(play.client.0, play.request).unwrap();
        }
        assert!(
            budget.used.load(Ordering::Acquire) > 0,
            "live mixer lost its receipt"
        );
        for stop in manager.take_audio() {
            audio.submit(stop.client.0, stop.request).unwrap();
        }
        drop(manager);
        audio.render(1).unwrap();
        assert!(audio.take_output().iter().flatten().all(|s| *s == 0.));
        let until = Instant::now() + Duration::from_secs(3);
        while budget.used.load(Ordering::Acquire) > 0 {
            assert!(
                Instant::now() < until,
                "PCM lease retained after all consumers stopped"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
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
        let mut audio_info = None;
        loop {
            let updates = manager.tick(now).unwrap();
            for frame in updates.frames {
                assert!(frame.frame.width > 0 && frame.frame.height > 0);
                frames += 1;
            }
            for audio in updates.audio {
                if let AudioRequest::PlayPcm { source, .. } = &audio.request {
                    audio_plays += 1;
                    let info = source.info();
                    audio_info = Some(
                        serde_json::json!({"sample_rate": info.sample_rate, "frames": info.frames, "sample_bytes": info.sample_bytes}),
                    );
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
            serde_json::json!({"test":"shared-media-without-vm","status":status,"published_updates":frames,"audio_plays":audio_plays,"pcm":audio_info,"mixed_peak":peak,"real_seconds":started.elapsed().as_secs_f64(),"virtual_ms":now})
        );
    }
}
