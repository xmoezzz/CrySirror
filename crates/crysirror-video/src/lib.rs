// SPDX-License-Identifier: MPL-2.0
//! Engine video service (ADR 0020): one frame API over the decoders vendored
//! with siglus_rs: Siglus OMV and plain Ogg/Theora, MPEG-1/2 program and
//! elementary streams, and ASF/WMV (WMV2, WMV3, VC-1). Frames are RGBA8 with
//! presentation times. The crate never opens a window, GPU or audio device;
//! renderers upload frames as textures and audio is handed to the mixer.
#![forbid(unsafe_code)]
pub mod amv;
pub mod isobmff;
pub mod mpeg;
pub mod mpeg_probe;
pub mod omv;
pub mod asf;
pub mod theora;
use anyhow::{Context, Result, bail, ensure};
use std::io::Cursor;

/// Upper bound on accepted encoded input (whole-file decoding).
pub const MAX_VIDEO_BYTES: usize = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    /// Siglus OMV: header + embedded Ogg/Theora (optional alpha rows).
    Omv,
    Ogg,
    Mpeg,
    Asf,
    Mp4,
    /// KiriKiri AlphaMovie (AJPM): JPEG-like intra/delta frames with alpha.
    Amv,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub container: Container,
    pub width: u32,
    pub height: u32,
    /// Frames per second when the stream declares it.
    pub fps: Option<f64>,
}

/// Validated stream-header facts published before the first decoded picture.
/// No playback clock, VM state or device resource is held here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMetadata {
    pub size: [u32; 2],
    /// Unknown duration stays None; packet/page hints are never frame counts.
    pub duration_ms: Option<u64>,
    pub frame_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub pts_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub trait VideoStream: Send {
    fn info(&self) -> &VideoInfo;
    /// Next frame in presentation order, `None` at the end.
    fn next_frame(&mut self) -> Result<Option<Frame>>;
    /// Restart from the first frame.
    fn rewind(&mut self) -> Result<()>;
}

/// Identify the container from its leading bytes; unknown data fails.
pub fn sniff(bytes: &[u8]) -> Result<Container> {
    const ASF: [u8; 16] = [
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ];
    if isobmff::is_mp4(bytes) {
        return Ok(Container::Mp4);
    }
    if bytes.starts_with(b"OggS") {
        return Ok(Container::Ogg);
    }
    if bytes.starts_with(b"AJPM") {
        return Ok(Container::Amv);
    }
    if bytes.starts_with(&ASF) {
        return Ok(Container::Asf);
    }
    if bytes.len() >= 4 && bytes[..3] == [0, 0, 1] && matches!(bytes[3], 0xBA | 0xB3) {
        return Ok(Container::Mpeg);
    }
    // OMV: little-endian header size ≥ 0x58 followed later by an Ogg page.
    if bytes.len() >= 0x58 {
        let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        if (0x58..=0x10000).contains(&size) && bytes.get(size..size + 4) == Some(b"OggS") {
            return Ok(Container::Omv);
        }
    }
    bail!("unrecognized video container")
}

pub fn open(bytes: Vec<u8>) -> Result<Box<dyn VideoStream>> {
    ensure!(bytes.len() <= MAX_VIDEO_BYTES, "video exceeds 1 GiB");
    Ok(match sniff(&bytes)? {
        Container::Omv => Box::new(TheoraStream::omv(bytes)?),
        Container::Ogg => Box::new(TheoraStream::ogg(bytes)?),
        Container::Mpeg => Box::new(MpegStream::new(bytes)?),
        Container::Asf => Box::new(AsfStream::new(bytes)?),
        Container::Mp4 => Box::new(isobmff::Mp4Stream::new(Cursor::new(bytes))?),
        Container::Amv => Box::new(AmvStream::new(bytes)?),
    })
}

/// AlphaMovie (AJPM) through the adopted amv_decoder: packets are decoded in
/// file order onto the previous frame (delta frames), timed by index at the
/// header's frame duration. Frames carry the movie's own alpha.
struct AmvStream {
    bytes: std::sync::Arc<[u8]>,
    file: amv_decoder::AmvFile<Cursor<std::sync::Arc<[u8]>>>,
    decoder: amv_decoder::AmvDecoder,
    info: VideoInfo,
    index: u64,
    frame_ms: f64,
}
impl AmvStream {
    fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes: std::sync::Arc<[u8]> = bytes.into();
        let (file, decoder) = Self::open(&bytes)?;
        let h = &file.header;
        ensure!(
            (1..=8192).contains(&h.width) && (1..=8192).contains(&h.height),
            "AlphaMovie dimensions"
        );
        // The header's `fps_num / fps_den` is the frame *duration* in seconds
        // (xp3-brute's AMV encoder writes 1/30 for 30 fps).
        let fps = (h.fps_num > 0 && h.fps_den > 0).then(|| f64::from(h.fps_den) / f64::from(h.fps_num));
        Ok(Self {
            info: VideoInfo {
                container: Container::Amv,
                width: u32::from(h.width),
                height: u32::from(h.height),
                fps,
            },
            frame_ms: 1000. / fps.unwrap_or(30.),
            bytes,
            file,
            decoder,
            index: 0,
        })
    }
    fn open(
        bytes: &std::sync::Arc<[u8]>,
    ) -> Result<(amv_decoder::AmvFile<Cursor<std::sync::Arc<[u8]>>>, amv_decoder::AmvDecoder)> {
        let file = amv_decoder::AmvFile::from_reader(Cursor::new(bytes.clone())).context("AlphaMovie header")?;
        let decoder = amv_decoder::AmvDecoder::new(file.header.clone(), file.mode, file.qtables.clone())?;
        Ok((file, decoder))
    }
}
impl VideoStream for AmvStream {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        if self.index >= u64::from(self.file.header.frame_count) {
            return Ok(None);
        }
        let Some(packet) = self.file.iter_frames().read_next()? else {
            return Ok(None);
        };
        let frame = self.decoder.decode_packet(&packet)?;
        ensure!(
            frame.rgba.len() == self.info.width as usize * self.info.height as usize * 4,
            "AlphaMovie frame size"
        );
        let pts_ms = (self.index as f64 * self.frame_ms).round() as u64;
        self.index += 1;
        Ok(Some(Frame {
            pts_ms,
            width: self.info.width,
            height: self.info.height,
            rgba: frame.rgba,
        }))
    }
    fn rewind(&mut self) -> Result<()> {
        (self.file, self.decoder) = Self::open(&self.bytes)?;
        self.index = 0;
        Ok(())
    }
}

struct TheoraStream {
    file: siglus_omv_decoder::TheoraFile,
    info: VideoInfo,
    theora_type: u32,
    vinfo: siglus_omv_decoder::VideoInfo,
    packed: Vec<u8>,
    index: u64,
    frame_ms: f64,
}
impl TheoraStream {
    fn omv(bytes: Vec<u8>) -> Result<Self> {
        use crate::omv;
        let header = omv::header(&bytes)?;
        let ogg = omv::embedded_ogg(&bytes)?;
        let fps = (header.frame_time_us != 0).then(|| 1_000_000.0 / header.frame_time_us as f64);
        Self::open(
            ogg,
            Container::Omv,
            Some((header.display_width, header.display_height)),
            header.theora_type,
            fps,
        )
    }
    fn ogg(bytes: Vec<u8>) -> Result<Self> {
        // Plain Theora is the YUV variant (2) at Theora's picture size.
        Self::open(bytes, Container::Ogg, None, 2, None)
    }
    fn open(
        ogg: Vec<u8>,
        container: Container,
        display: Option<(u32, u32)>,
        theora_type: u32,
        fps: Option<f64>,
    ) -> Result<Self> {
        let file = siglus_omv_decoder::TheoraFile::open_from_memory(ogg).context("open Theora")?;
        let vinfo = file.info();
        let (width, height) =
            display.unwrap_or((vinfo.width.max(1) as u32, vinfo.height.max(1) as u32));
        ensure!(
            (1..=8192).contains(&width) && (1..=8192).contains(&height),
            "video dimensions"
        );
        let fps = fps.or((vinfo.fps > 0.).then_some(vinfo.fps));
        let packed =
            vec![0; crate::omv::packed_len(vinfo.frame_width, vinfo.frame_height, vinfo.fmt)?];
        Ok(Self {
            file,
            info: VideoInfo {
                container,
                width,
                height,
                fps,
            },
            theora_type,
            vinfo,
            packed,
            index: 0,
            frame_ms: 1000. / fps.unwrap_or(30.),
        })
    }
}
impl VideoStream for TheoraStream {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        if !self.file.read_video_frame(&mut self.packed)? {
            return Ok(None);
        }
        let v = &self.vinfo;
        let rgba = crate::omv::to_rgba(
            &self.packed,
            v.frame_width,
            v.frame_height,
            v.fmt,
            self.info.width as i32,
            self.info.height as i32,
            self.theora_type,
        )?;
        let pts_ms = (self.index as f64 * self.frame_ms).round() as u64;
        self.index += 1;
        Ok(Some(Frame {
            pts_ms,
            width: self.info.width,
            height: self.info.height,
            rgba,
        }))
    }
    fn rewind(&mut self) -> Result<()> {
        self.file.reset();
        self.index = 0;
        Ok(())
    }
}

/// MPEG-1/2: decoded exactly as siglus_scene_vm does (whole stream pushed,
/// then flushed); frames are timed by index at the sequence header's rate.
struct MpegStream {
    bytes: Vec<u8>,
    frames: Vec<Frame>,
    cursor: usize,
    info: VideoInfo,
}
impl MpegStream {
    fn new(bytes: Vec<u8>) -> Result<Self> {
        let header = crate::mpeg_probe::find_sequence_header(&bytes[..bytes.len().min(1 << 20)]);
        let fps = header
            .and_then(|h| crate::mpeg_probe::fps_from_frame_rate_code(h.frame_rate_code))
            .map(f64::from);
        let mut pipe = na_mpeg2_decoder::av::MpegAvPipeline::new();
        let mut raw = Vec::new();
        let mut collect = |ev| {
            if let na_mpeg2_decoder::av::MpegAvEvent::Video(f) = ev {
                raw.push(f);
            }
        };
        pipe.push_with(&bytes, None, &mut collect)
            .map_err(|e| anyhow::anyhow!("MPEG decode: {e}"))?;
        pipe.flush_with(&mut collect)
            .map_err(|e| anyhow::anyhow!("MPEG flush: {e}"))?;
        ensure!(!raw.is_empty(), "MPEG stream has no video frames");
        let frame_ms = 1000. / fps.unwrap_or(30.);
        let frames: Vec<Frame> = raw
            .into_iter()
            .enumerate()
            .map(|(i, f)| Frame {
                pts_ms: (i as f64 * frame_ms).round() as u64,
                width: f.width,
                height: f.height,
                rgba: f.rgba,
            })
            .collect();
        let info = VideoInfo {
            container: Container::Mpeg,
            width: frames[0].width,
            height: frames[0].height,
            fps,
        };
        ensure!(
            (1..=8192).contains(&info.width) && (1..=8192).contains(&info.height),
            "video dimensions"
        );
        Ok(Self {
            bytes,
            frames,
            cursor: 0,
            info,
        })
    }
}
impl VideoStream for MpegStream {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        let f = self.frames.get(self.cursor).cloned();
        self.cursor += f.is_some() as usize;
        Ok(f)
    }
    fn rewind(&mut self) -> Result<()> {
        let _ = &self.bytes;
        self.cursor = 0;
        Ok(())
    }
}

struct AsfStream {
    bytes: std::sync::Arc<Vec<u8>>,
    dec: wmv_decoder::AsfWmv2Decoder<Cursor<SharedBytes>>,
    info: VideoInfo,
}
#[derive(Clone)]
struct SharedBytes(std::sync::Arc<Vec<u8>>);
impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl AsfStream {
    fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = std::sync::Arc::new(bytes);
        let dec = wmv_decoder::AsfWmv2Decoder::open(Cursor::new(SharedBytes(bytes.clone())))
            .map_err(|e| anyhow::anyhow!("open ASF: {e}"))?;
        let v = dec.video_stream_info();
        let info = VideoInfo {
            container: Container::Asf,
            width: v.width,
            height: v.height,
            fps: None,
        };
        ensure!(
            (1..=8192).contains(&info.width) && (1..=8192).contains(&info.height),
            "video dimensions"
        );
        Ok(Self { bytes, dec, info })
    }
}
impl VideoStream for AsfStream {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        let Some(f) = self
            .dec
            .next_frame()
            .map_err(|e| anyhow::anyhow!("WMV decode: {e}"))?
        else {
            return Ok(None);
        };
        let (w, h) = (f.frame.width, f.frame.height);
        Ok(Some(Frame {
            pts_ms: f.pts_ms as u64,
            width: w,
            height: h,
            rgba: wmv_decoder::yuv420p_to_rgba_scaled(&f.frame, w, h),
        }))
    }
    fn rewind(&mut self) -> Result<()> {
        *self = Self::new(self.bytes.as_ref().clone())?;
        Ok(())
    }
}

/// Time-driven playback: `advance` decodes up to the frame due at the current
/// time and keeps it as the presented frame.
pub struct Player {
    stream: Box<dyn VideoStream>,
    pub looping: bool,
    time_ms: f64,
    current: Option<Frame>,
    next: Option<Frame>,
    ended: bool,
    /// Bumped whenever the presented frame changes (texture upload key).
    pub revision: u64,
}
impl Player {
    pub fn new(stream: Box<dyn VideoStream>, looping: bool) -> Result<Self> {
        let mut p = Self {
            stream,
            looping,
            time_ms: 0.,
            current: None,
            next: None,
            ended: false,
            revision: 0,
        };
        p.next = p.stream.next_frame()?;
        ensure!(p.next.is_some(), "video has no frames");
        p.advance(0.)?;
        Ok(p)
    }
    pub fn info(&self) -> &VideoInfo {
        self.stream.info()
    }
    pub fn time_ms(&self) -> f64 {
        self.time_ms
    }
    pub fn ended(&self) -> bool {
        self.ended
    }
    pub fn frame(&self) -> Option<&Frame> {
        self.current.as_ref()
    }
    pub fn advance(&mut self, dt_ms: f64) -> Result<()> {
        ensure!(dt_ms.is_finite() && dt_ms >= 0., "invalid video time step");
        if self.ended {
            return Ok(());
        }
        self.time_ms += dt_ms;
        loop {
            match self.next.take() {
                Some(f) if (f.pts_ms as f64) <= self.time_ms => {
                    self.current = Some(f);
                    self.revision += 1;
                    self.next = self.stream.next_frame()?;
                }
                Some(f) => {
                    self.next = Some(f);
                    return Ok(());
                }
                None if self.looping => {
                    let duration = self.current.as_ref().map_or(0., |c| c.pts_ms as f64) + 1.;
                    self.stream.rewind()?;
                    self.time_ms = (self.time_ms - duration).max(0.);
                    self.next = self.stream.next_frame()?;
                    if self.next.is_none() {
                        self.ended = true;
                        return Ok(());
                    }
                }
                None => {
                    self.ended = true;
                    return Ok(());
                }
            }
        }
    }
}
