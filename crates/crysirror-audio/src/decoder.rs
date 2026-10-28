// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded packet decoding over caller-owned seekable media sources. No device
//! or ambient filesystem access. File authority belongs to the host VFS.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::Arc,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

use crysirror_core::service::AssetReader;
pub use symphonia::core::io::MediaSource;

pub type StereoFrame = [f32; 2];
pub const MAX_PACKET_FRAMES: usize = 65536;
pub const MAX_CLIP_FRAMES: usize = 4_194_304;

/// A nonblocking source distinguishes temporary starvation from actual EOF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRead {
    Frames(usize),
    Pending,
    End,
}
/// Bounded source-buffer state. Decoder work never enters project/save JSON.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceBufferStatus {
    /// Ready decoded frames, excluding the mixer's small resampling cursor.
    pub ready_frames: usize,
    /// One read/seek job owns the decoder while true.
    pub worker_pending: bool,
    /// An uncached seek awaits exact decoded-frame completion.
    pub seek_pending: bool,
    /// Source timeline generation; stale reads cannot enter a new timeline.
    pub generation: u64,
    /// Successfully decoded frames, including warmup and read-ahead.
    pub decoded_frames: u64,
    /// A visible decoder failure, retained until the source is replaced.
    pub error: Option<String>,
}
pub trait AudioSource: Send {
    /// Encoded channel layout before the stereo projection, when known.
    fn channel_count(&self) -> Option<u16> {
        None
    }
    fn sample_rate(&self) -> u32;
    fn frame_count(&self) -> Option<u64>;
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize>;
    /// Exact decoded-frame position; packet seeks decode/discard the prefix.
    fn seek(&mut self, frame: u64) -> Result<()>;
    /// Poll ready PCM. Synchronous decoders keep their original contract;
    /// worker-buffered sources override this to report Pending without I/O.
    fn poll_read(&mut self, out: &mut [StereoFrame]) -> Result<SourceRead> {
        Ok(match self.read(out)? {
            0 => SourceRead::End,
            n => SourceRead::Frames(n),
        })
    }
    /// True only when the exact frame is ready. Repeated polling is idempotent.
    fn poll_seek(&mut self, frame: u64) -> Result<bool> {
        self.seek(frame)?;
        Ok(true)
    }
    /// Owner/control phase: collect completed work and submit bounded prefetch.
    fn service(&mut self) -> Result<()> {
        Ok(())
    }
    /// Position of the next poll_read result, when the backend exposes it.
    fn read_position(&self) -> Option<u64> {
        None
    }
    /// True only when poll_read/poll_seek/service execute no decoder I/O.
    fn is_nonblocking(&self) -> bool {
        false
    }
    fn buffer_status(&self) -> Option<SourceBufferStatus> {
        None
    }
}

/// Unified resident-resource decoder used by channels, data-VM playback and
/// engine Audio APIs. Native codecs share the same AudioSource/mixer contract.
pub fn open_memory(bytes: Arc<[u8]>, extension: Option<&str>) -> Result<Box<dyn AudioSource>> {
    if let Some(source)=crate::synth::open(bytes.clone(),extension)?{return Ok(source)}
    if let Some(source) = crate::native::open_if_native(bytes.clone())? {
        return Ok(source);
    }
    Ok(Box::new(Stream::from_bytes(bytes, extension)?))
}

/// Unified seekable-provider decoder. Standard streams still decode packets;
/// NWA reads only a bounded header/table and its current block. Non-seekable
/// standard sources keep the existing Symphonia path unchanged.
pub fn open_reader(
    mut reader: Box<dyn MediaSource>,
    extension: Option<&str>,
) -> Result<Box<dyn AudioSource>> {
    use std::io::Read;
    if crate::synth::is_extension(extension){
        let mut bytes=Vec::new();reader.take(8*1024*1024+1).read_to_end(&mut bytes)?;
        return crate::synth::open(Arc::from(bytes),extension)?.context("synth decoder absent");
    }
    if reader.is_seekable() {
        let start = reader.stream_position()?;
        let mut prefix = [0u8; 4];
        let mut count = 0;
        while count < prefix.len() {
            let n = reader.read(&mut prefix[count..])?;
            if n == 0 {
                break;
            }
            count += n;
        }
        reader.seek(SeekFrom::Start(start))?;
        if crate::native::may_be_nwa(&prefix[..count]) {
            return crate::native::open_reader(reader);
        }
    }
    Ok(Box::new(Stream::open(reader, extension)?))
}

#[derive(Clone)]
pub struct Clip {
    rate: u32,
    samples: Arc<[StereoFrame]>,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl Clip {
    pub fn new(rate: u32, samples: Vec<StereoFrame>) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&rate),
            "audio sample rate must be 8000..192000"
        );
        ensure!(
            !samples.is_empty() && samples.len() <= MAX_CLIP_FRAMES,
            "audio clip size"
        );
        ensure!(
            samples
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 16.),
            "non-finite or excessive PCM sample"
        );
        Ok(Self {
            rate,
            samples: samples.into(),
            storage: None,
        })
    }
    /// The receipt follows every cached/live clone until the last PCM owner drops.
    pub fn retain_storage(&mut self, storage: Arc<dyn Send + Sync>) {
        self.storage = Some(storage);
    }
    pub fn source(&self) -> Box<dyn AudioSource> {
        Box::new(ClipSource {
            clip: self.clone(),
            at: 0,
        })
    }
    pub fn frames(&self) -> usize {
        self.samples.len()
    }
}
impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clip")
            .field("rate", &self.rate)
            .field("frames", &self.frames())
            .finish()
    }
}
/// Constant-time publication of validated PCM, independent of Clip's shorter
/// SFX size limit. Reads and exact seeks perform no codec/filesystem work.
pub fn open_pcm(pcm: crysirror_core::service::PcmAudio) -> Box<dyn AudioSource> {
    Box::new(PcmSource { pcm, at: 0 })
}
struct PcmSource {
    pcm: crysirror_core::service::PcmAudio,
    at: usize,
}
impl AudioSource for PcmSource {
    fn channel_count(&self) -> Option<u16> {
        Some(2)
    }
    fn sample_rate(&self) -> u32 {
        self.pcm.sample_rate()
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.pcm.frame_count() as u64)
    }
    fn is_nonblocking(&self) -> bool {
        true
    }
    fn read_position(&self) -> Option<u64> {
        Some(self.at as u64)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let n = out
            .len()
            .min(self.pcm.frame_count().saturating_sub(self.at));
        out[..n].copy_from_slice(&self.pcm.frames()[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(
            frame <= self.pcm.frame_count() as u64,
            "PCM seek outside source"
        );
        self.at = frame as usize;
        Ok(())
    }
}
struct ClipSource {
    clip: Clip,
    at: usize,
}
impl AudioSource for ClipSource {
    fn channel_count(&self) -> Option<u16> {
        Some(2)
    }
    fn is_nonblocking(&self) -> bool {
        true
    }
    fn sample_rate(&self) -> u32 {
        self.clip.rate
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.clip.samples.len() as u64)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let n = out
            .len()
            .min(self.clip.samples.len().saturating_sub(self.at));
        out[..n].copy_from_slice(&self.clip.samples[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(
            frame <= self.clip.samples.len() as u64,
            "clip seek outside source"
        );
        self.at = frame as usize;
        Ok(())
    }
}

/// Decoder buffers one packet, never an entire song. The encoded media source
/// can be a capability-opened File or a provider's already resident bytes.
pub struct Stream {
    format: Option<Box<dyn FormatReader>>,
    decoder: Box<dyn Decoder>,
    track: u32,
    rate: u32,
    channels: usize,
    frames: Option<u64>,
    packet: Vec<StereoFrame>,
    at: usize,
    position: u64,
    eof: bool,
    startup_delay: u64,
    trim_start: u64,
}
impl Stream {
    pub fn open(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }
        let mss = MediaSourceStream::new(source, Default::default());
        let format = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .context("probe audio stream")?
            .format;
        let track = format
            .default_track()
            .context("audio stream has no default track")?;
        let rate = track
            .codec_params
            .sample_rate
            .context("audio stream has no sample rate")?;
        let channels = track
            .codec_params
            .channels
            .context("audio stream has no channels")?
            .count();
        ensure!(
            (8000..=192000).contains(&rate) && (1..=2).contains(&channels),
            "only mono/stereo audio at 8000..192000 Hz is supported"
        );
        let decoder = crate::opus::codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("audio codec")?;
        let (id, frames) = (track.id, track.codec_params.n_frames);
        let startup_delay =
            if [symphonia::core::codecs::CODEC_TYPE_VORBIS, symphonia::core::codecs::CODEC_TYPE_OPUS]
                .contains(&track.codec_params.codec)
            {
                track.codec_params.delay.unwrap_or(0) as u64
            } else {
                0
            };
        Ok(Self {
            format: Some(format),
            decoder,
            track: id,
            rate,
            channels,
            frames,
            packet: Vec::new(),
            at: 0,
            position: 0,
            eof: false,
            startup_delay,
            trim_start: startup_delay,
        })
    }
    pub fn from_bytes(bytes: Arc<[u8]>, extension: Option<&str>) -> Result<Self> {
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "resident encoded audio exceeds 64 MiB"
        );
        Self::open(Box::new(Cursor::new(bytes)), extension)
    }
    fn packet(&mut self) -> Result<bool> {
        if self.eof {
            return Ok(false);
        }
        loop {
            let packet = match self
                .format
                .as_mut()
                .context("audio reader unavailable")?
                .next_packet()
            {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(false);
                }
                Err(e) => return Err(anyhow::anyhow!("audio packet: {e}")),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = self
                .decoder
                .decode(&packet)
                .map_err(|e| anyhow::anyhow!("audio decode: {e}"))?;
            ensure!(
                decoded.spec().rate == self.rate
                    && decoded.spec().channels.count() == self.channels,
                "audio format changed mid-stream"
            );
            ensure!(
                decoded.capacity() <= MAX_PACKET_FRAMES,
                "audio packet exceeds decoded-frame limit"
            );
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            samples.copy_interleaved_ref(decoded);
            // After a Vorbis seek the first MDCT packet primes the overlap
            // window and yields no samples. Its timestamp duration still passes;
            // otherwise every exact seek is shifted by one decoded block.
            if samples.samples().is_empty() && packet.dur() > 0 {
                let tb = self
                    .format
                    .as_ref()
                    .context("audio reader unavailable")?
                    .tracks()
                    .iter()
                    .find(|t| t.id == self.track)
                    .and_then(|t| t.codec_params.time_base)
                    .context("empty audio packet needs time base")?;
                let time = tb.calc_time(packet.dur());
                let skip = (time.seconds as f64 * self.rate as f64 + time.frac * self.rate as f64)
                    .round() as u64;
                self.position = self
                    .position
                    .checked_add(skip)
                    .context("audio timestamp overflow")?;
            }
            self.packet.clear();
            self.at = 0;
            // Ogg/Vorbis may normalize packet timestamps while leaving the
            // declared encoder delay unapplied to PCM. Account for explicit
            // packet trims first, then remove any remaining initial delay.
            self.trim_start = self.trim_start.saturating_sub(packet.trim_start() as u64);
            let skip =
                self.trim_start
                    .min((samples.samples().len() / self.channels) as u64) as usize;
            self.trim_start -= skip as u64;
            for row in samples.samples().chunks_exact(self.channels).skip(skip) {
                let frame = [row[0], if self.channels == 1 { row[0] } else { row[1] }];
                ensure!(
                    frame.iter().all(|x| x.is_finite() && x.abs() <= 16.),
                    "invalid decoded PCM"
                );
                self.packet.push(frame);
            }
            if !self.packet.is_empty() {
                return Ok(true);
            }
        }
    }
    pub fn buffered_frames(&self) -> usize {
        self.packet.len()
    }
}
impl AudioSource for Stream {
    fn channel_count(&self) -> Option<u16> {
        Some(self.channels as u16)
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn frame_count(&self) -> Option<u64> {
        self.frames
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let mut done = 0;
        while done < out.len() {
            if self.at == self.packet.len() && !self.packet()? {
                break;
            }
            let n = (out.len() - done).min(self.packet.len() - self.at);
            out[done..done + n].copy_from_slice(&self.packet[self.at..self.at + n]);
            self.at += n;
            done += n;
            self.position += n as u64;
        }
        Ok(done)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        if frame == self.position {
            return Ok(());
        }
        if let Some(total) = self.frames {
            ensure!(frame <= total, "audio seek outside source")
        }
        // Re-probe from the same capability-owned stream before seeking. This
        // clears demuxer packet-parser state too (FLAC retains buffered packets
        // across FormatReader::seek). Decode preroll for MDCT/bit reservoirs.
        let mut mss = self
            .format
            .take()
            .context("audio reader unavailable")?
            .into_inner();
        mss.seek(SeekFrom::Start(0))?;
        let format = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )?
            .format;
        let track = format.default_track().context("audio track disappeared")?;
        self.decoder = crate::opus::codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;
        self.track = track.id;
        self.format = Some(format);
        // Seek timestamps are in the track time base; Symphonia audio sample
        // timestamps need not equal sample indices, so ask in wall-clock Time.
        let seek_frame = frame.saturating_sub(self.rate as u64 / 4);
        self.trim_start = if seek_frame == 0 {
            self.startup_delay
        } else {
            0
        };
        if seek_frame == 0 {
            self.packet.clear();
            self.at = 0;
            self.position = 0;
            self.eof = false;
            let mut discard = [[0.; 2]; 1024];
            while self.position < frame {
                let n = (frame - self.position).min(discard.len() as u64) as usize;
                if self.read(&mut discard[..n])? == 0 {
                    bail!("audio ended during seek")
                }
            }
            return Ok(());
        }
        let time = symphonia::core::units::Time::from(seek_frame as f64 / self.rate as f64);
        let seek = self
            .format
            .as_mut()
            .context("audio reader unavailable")?
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track),
                },
            )
            .map_err(|e| anyhow::anyhow!("audio seek: {e}"))?;
        let track = self
            .format
            .as_ref()
            .context("audio reader unavailable")?
            .tracks()
            .iter()
            .find(|t| t.id == self.track)
            .context("audio track disappeared")?;
        let tb = track
            .codec_params
            .time_base
            .context("audio seek requires time base")?;
        let actual = tb.calc_time(seek.actual_ts);
        let at = (actual.seconds as f64 * self.rate as f64 + actual.frac * self.rate as f64).round()
            as u64;
        ensure!(at <= frame, "accurate audio seek passed requested frame");
        self.decoder.reset();
        self.packet.clear();
        self.at = 0;
        self.position = at;
        self.eof = false;
        let mut discard = [[0.; 2]; 1024];
        while self.position < frame {
            let n = (frame - self.position).min(discard.len() as u64) as usize;
            if self.read(&mut discard[..n])? == 0 {
                bail!("audio ended during seek")
            }
        }
        Ok(())
    }
}

pub fn decode_clip(mut source: Box<dyn AudioSource>) -> Result<Clip> {
    if let Some(n) = source.frame_count() {
        ensure!(
            n <= MAX_CLIP_FRAMES as u64,
            "audio clip too long; use streaming music"
        )
    }
    let rate = source.sample_rate();
    let mut samples = Vec::new();
    let mut block = [[0.; 2]; 1024];
    loop {
        let n = source.read(&mut block)?;
        if n == 0 {
            break;
        }
        ensure!(
            samples.len() + n <= MAX_CLIP_FRAMES,
            "audio clip too long; use streaming music"
        );
        samples.extend_from_slice(&block[..n]);
    }
    Clip::new(rate, samples)
}

impl MediaSource for AssetMedia {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.len())
    }
}
/// Shared capability reader used by modern and native audio. Open/probe on a
/// worker in realtime hosts; callers retain responsibility for admission.
pub fn open_asset(
    reader: Arc<dyn AssetReader>,
    extension: Option<&str>,
) -> Result<Box<dyn AudioSource>> {
    open_reader(
        Box::new(AssetMedia(crysirror_io::AssetIo::new(reader)?)),
        extension,
    )
}

struct AssetMedia(crysirror_io::AssetIo);
impl Read for AssetMedia {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(out)
    }
}
impl Seek for AssetMedia {
    fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(at)
    }
}
