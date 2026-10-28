//! Native Rust ISO-BMFF video. The caller owns I/O, scheduling, cancellation,
//! queues and presentation. No filesystem, process, device or VM is opened here.
//!
//! AVC, HEVC and AV1 use the audited, version-pinned OxideAV implementations.
//! This is a software decoder, not a claim of every profile's conformance.
use crate::{Container, VideoInfo, mpeg::VideoFrame};
use anyhow::{Context, Result, bail, ensure};
use oxideav_core::{
    CodecParameters, Decoder, DecoderLimits, Demuxer, Error, Frame as DecodedFrame, MediaType,
    PixelFormat, RuntimeContext, TimeBase,
};
use oxideav_pixfmt::{ConvertOptions, FrameInfo};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 64 * 1024 * 1024;

/// Signature probe. The full parser validates sizes, brands and sample tables.
pub fn is_mp4(head: &[u8]) -> bool {
    head.len() >= 8 && matches!(&head[4..8], b"ftyp" | b"moov" | b"free" | b"wide")
}

/// Check top-level box extents and bound metadata before the demuxer allocates.
fn preflight(reader: &mut (impl Read + Seek)) -> Result<()> {
    let len = reader.seek(SeekFrom::End(0))?;
    ensure!(
        len > 0 && len <= crate::MAX_VIDEO_BYTES as u64,
        "MP4 encoded input budget"
    );
    let mut pos = 0u64;
    let mut metadata = 0u64;
    while pos < len {
        ensure!(len - pos >= 8, "truncated MP4 box header");
        reader.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 8];
        reader.read_exact(&mut header)?;
        let short = u32::from_be_bytes(header[..4].try_into().unwrap());
        let (size, minimum) = if short == 1 {
            let mut large = [0u8; 8];
            reader.read_exact(&mut large)?;
            (u64::from_be_bytes(large), 16)
        } else if short == 0 {
            (len - pos, 8)
        } else {
            (u64::from(short), 8)
        };
        ensure!(
            size >= minimum && size <= len - pos,
            "invalid MP4 box extent"
        );
        if !matches!(&header[4..], b"mdat" | b"free" | b"skip" | b"wide") {
            metadata = metadata
                .checked_add(size)
                .context("MP4 metadata overflow")?;
            ensure!(metadata <= MAX_METADATA_BYTES, "MP4 metadata budget");
        }
        pos += size;
    }
    reader.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// A CPU decoder with one caller-owned input and no unbounded output queue.
/// Move it to an engine worker; poll cancellation between packets and frames.
pub struct Mp4Stream {
    demux: oxideav_mp4::demux::Mp4Demuxer,
    decoder: Box<dyn Decoder>,
    headers: HeaderGuard,
    params: CodecParameters,
    info: VideoInfo,
    stream: u32,
    time_base: TimeBase,
    audio: bool,
    eof: bool,
    failed: bool,
    output_count: u64,
    discard: BTreeMap<i64, usize>,
    seek_target_ms: Option<i64>,
}

impl Mp4Stream {
    pub fn new<R: Read + Seek + Send + 'static>(mut reader: R) -> Result<Self> {
        preflight(&mut reader)?;
        let mut ctx = RuntimeContext::new();
        oxideav_h264::register(&mut ctx);
        oxideav_h265::register(&mut ctx);
        oxideav_av1::register(&mut ctx);
        let demux =
            oxideav_mp4::demux::open_typed(Box::new(reader), &ctx.codecs).context("open MP4")?;
        let selected = demux
            .streams()
            .iter()
            .find(|s| s.params.media_type == MediaType::Video)
            .context("MP4 has no video track")?;
        let stream = selected.index;
        let time_base = selected.time_base;
        ensure!(
            time_base.num() > 0 && time_base.den() > 0,
            "invalid MP4 video time base"
        );
        let mut params = selected.params.clone();
        ensure!(
            params.options.get("protection_scheme").is_none(),
            "encrypted MP4 requires a caller-provided decryption service"
        );
        ensure!(
            matches!(params.codec_id.as_str(), "h264" | "h265" | "hevc" | "av1"),
            "unsupported MP4 video codec {}",
            params.codec_id
        );
        video_info(&params)?;
        let headers = HeaderGuard::new(&params)?;
        params.limits = DecoderLimits::default()
            .with_max_pixels_per_frame(MAX_FRAME_BYTES / 4)
            .with_max_alloc_bytes_per_frame(MAX_FRAME_BYTES);
        let mut decoder = ctx
            .codecs
            .first_decoder(&params)
            .context("create native MP4 decoder")?;
        // The Media service already budgets the outer worker. No hidden fan-out.
        decoder.set_execution_context(&oxideav_core::ExecutionContext::serial());
        if let Some(output) = decoder.output_params() {
            params.merge_output_params(&output);
        }
        let info = video_info(&params)?;
        let audio = demux
            .streams()
            .iter()
            .any(|s| s.params.media_type == MediaType::Audio);
        Ok(Self {
            demux,
            decoder,
            headers,
            params,
            info,
            stream,
            time_base,
            audio,
            eof: false,
            failed: false,
            output_count: 0,
            discard: BTreeMap::new(),
            seek_target_ms: None,
        })
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }
    pub fn codec(&self) -> &str {
        self.params.codec_id.as_str()
    }
    pub fn has_audio(&self) -> bool {
        self.audio
    }

    /// Seek by presentation time. Decode from the preceding sync sample and
    /// discard pre-roll; the next published frame is at or after the target.
    pub fn seek(&mut self, pts_ms: i64) -> Result<()> {
        ensure!(!self.failed, "MP4 decoder failed; reopen its input");
        ensure!(pts_ms >= 0, "negative MP4 seek target");
        let ticks = TimeBase::new(1, 1000)
            .rescale_checked(pts_ms, self.time_base)
            .context("MP4 seek timestamp overflow")?;
        self.demux
            .seek_to(self.stream, ticks)
            .context("MP4 sync-sample seek")?;
        self.decoder.reset().context("reset MP4 decoder")?;
        self.eof = false;
        self.discard.clear();
        self.seek_target_ms = Some(pts_ms);
        Ok(())
    }

    /// `None` means EOF or caller cancellation; cancellation does not poison the
    /// stream. Other failures are terminal, so partial failures cannot become EOF.
    pub fn next_frame(
        &mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<VideoFrame>> {
        ensure!(!self.failed, "MP4 decoder failed; reopen its input");
        let result = self.next_inner(&mut cancelled);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn next_inner(&mut self, cancelled: &mut impl FnMut() -> bool) -> Result<Option<VideoFrame>> {
        loop {
            if cancelled() {
                return Ok(None);
            }
            match self.decoder.receive_frame() {
                Ok(DecodedFrame::Video(frame)) => {
                    let pts = frame
                        .pts
                        .context("MP4 decoded frame has no presentation timestamp")?;
                    if let Some(count) = self.discard.get_mut(&pts) {
                        *count -= 1;
                        if *count == 0 {
                            self.discard.remove(&pts);
                        }
                        continue;
                    }
                    let pts_ms = self
                        .time_base
                        .rescale_checked(pts, TimeBase::new(1, 1000))
                        .context("MP4 presentation timestamp overflow")?;
                    if self.seek_target_ms.is_some_and(|target| pts_ms < target) {
                        continue;
                    }
                    self.seek_target_ms = None;
                    if let Some(output) = self.decoder.output_params() {
                        self.params.merge_output_params(&output);
                    }
                    self.info = video_info(&self.params)?;
                    let format = self
                        .params
                        .pixel_format
                        .context("MP4 decoder did not declare pixel format")?;
                    let rgba = oxideav_pixfmt::convert(
                        &frame,
                        FrameInfo::new(format, self.info.width, self.info.height),
                        PixelFormat::Rgba,
                        &ConvertOptions::default(),
                    )
                    .context("MP4 RGBA conversion")?;
                    let plane = rgba
                        .image_planes()
                        .first()
                        .context("MP4 missing RGBA plane")?;
                    let row = self.info.width as usize * 4;
                    let height = self.info.height as usize;
                    ensure!(
                        plane.stride >= row
                            && plane.data.len()
                                >= plane
                                    .stride
                                    .checked_mul(height)
                                    .context("MP4 stride overflow")?,
                        "MP4 RGBA plane extent"
                    );
                    let mut bytes = Vec::with_capacity(row * height);
                    for y in 0..height {
                        bytes.extend_from_slice(
                            &plane.data[y * plane.stride..y * plane.stride + row],
                        );
                    }
                    self.output_count += 1;
                    return Ok(Some(VideoFrame {
                        pts_ms,
                        width: self.info.width,
                        height: self.info.height,
                        rgba: bytes,
                    }));
                }
                Ok(_) => bail!("non-video frame from MP4 video decoder"),
                Err(Error::Eof) => {
                    ensure!(self.eof, "MP4 decoder ended before container EOF");
                    ensure!(
                        self.output_count > 0 || self.seek_target_ms.is_some(),
                        "MP4 track decoded no video frames"
                    );
                    return Ok(None);
                }
                Err(Error::NeedMore) if !self.eof => match self.demux.next_packet() {
                    Ok(packet) if packet.stream_index == self.stream => {
                        ensure!(
                            self.demux.sample_description_index_of_last_packet() == Some(1),
                            "MP4 sample-description switching is not supported"
                        );
                        ensure!(
                            packet.data.len() <= MAX_PACKET_BYTES,
                            "MP4 compressed packet budget"
                        );
                        if packet.flags.discard {
                            let pts = packet.pts.context("discarded MP4 packet has no PTS")?;
                            *self.discard.entry(pts).or_default() += 1;
                        }
                        self.headers.check_packet(&packet.data)?;
                        self.decoder
                            .send_packet(&packet)
                            .context("decode MP4 video packet")?;
                    }
                    Ok(_) => {}
                    Err(Error::Eof) => {
                        self.eof = true;
                        self.decoder.flush().context("flush MP4 video decoder")?;
                    }
                    Err(e) => return Err(e).context("read MP4 packet"),
                },
                Err(Error::NeedMore) => bail!("MP4 decoder requested input after flush"),
                Err(e) => return Err(e).context("receive MP4 video frame"),
            }
        }
    }
}

fn video_info(p: &CodecParameters) -> Result<VideoInfo> {
    let width = p.width.context("MP4 width missing")?;
    let height = p.height.context("MP4 height missing")?;
    ensure!(
        width > 0
            && height > 0
            && width <= 8192
            && height <= 8192
            && u64::from(width) * u64::from(height) * 4 <= MAX_FRAME_BYTES,
        "MP4 frame dimensions/budget"
    );
    Ok(VideoInfo {
        container: Container::Mp4,
        width,
        height,
        fps: p
            .frame_rate
            .map(|r| r.as_f64())
            .filter(|fps| fps.is_finite() && *fps > 0.),
    })
}

impl crate::VideoStream for Mp4Stream {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn next_frame(&mut self) -> Result<Option<crate::Frame>> {
        Ok(
            Mp4Stream::next_frame(self, || false)?.map(|f| crate::Frame {
                pts_ms: f.pts_ms.max(0) as u64,
                width: f.width,
                height: f.height,
                rgba: f.rgba,
            }),
        )
    }
    fn rewind(&mut self) -> Result<()> {
        self.seek(0)
    }
}

// These decoder revisions do not yet enforce CodecParameters::limits. Validate
// coded sequence dimensions ourselves *before* sending configuration or packets
// to them; output-only size checks would come after the decoder's allocations.
// Reject multilayer extensions explicitly, rather than treating layers as frames.
enum HeaderGuard {
    Avc(usize),
    Hevc(usize),
    Av1,
}
impl HeaderGuard {
    fn new(params: &CodecParameters) -> Result<Self> {
        match params.codec_id.as_str() {
            "h264" => {
                let config =
                    oxideav_h264::avcc::AvcDecoderConfigurationRecord::parse(&params.extradata)
                        .map_err(|e| anyhow::anyhow!("MP4 avcC: {e:?}"))?;
                let guard = Self::Avc(usize::from(config.length_size));
                for nal in config.sps.iter().chain(config.sps_ext.iter()) {
                    guard.check_nal(nal)?;
                }
                Ok(guard)
            }
            "h265" | "hevc" => {
                let (config, consumed) = oxideav_h265::hvcc::parse_hvcc_with_len(&params.extradata)
                    .map_err(|e| anyhow::anyhow!("MP4 hvcC: {e:?}"))?;
                ensure!(
                    consumed == params.extradata.len(),
                    "layered HEVC configuration is unsupported"
                );
                for nal in &config.nal_units {
                    ensure!(nal.header.nuh_layer_id == 0, "layered HEVC is unsupported");
                    if nal.header.nal_unit_type == 33 {
                        check_hevc_sps(&nal.rbsp)?;
                    }
                }
                Ok(Self::Hevc(config.length_size))
            }
            "av1" => {
                let config = oxideav_av1::codec_config::Av1CodecConfig::parse(&params.extradata)
                    .context("MP4 av1C configuration")?;
                let guard = Self::Av1;
                guard.check_packet(&config.config_obus)?;
                Ok(guard)
            }
            _ => bail!("unsupported MP4 codec"),
        }
    }
    fn check_packet(&self, data: &[u8]) -> Result<()> {
        if let Self::Av1 = self {
            for obu in oxideav_av1::obu::ObuIter::new(data) {
                let obu = obu.map_err(|e| anyhow::anyhow!("MP4 AV1 OBU: {e:?}"))?;
                ensure!(obu.spatial_id == 0, "layered AV1 is unsupported");
                if obu.obu_type == oxideav_av1::obu::ObuType::SequenceHeader {
                    let seq = oxideav_av1::sequence_header::parse_sequence_header(obu.payload)
                        .map_err(|e| anyhow::anyhow!("MP4 AV1 sequence: {e:?}"))?;
                    coded_dimensions(
                        u64::from(seq.max_frame_width_minus_1) + 1,
                        u64::from(seq.max_frame_height_minus_1) + 1,
                    )?;
                }
            }
            return Ok(());
        }
        let length_size = match self {
            Self::Avc(n) | Self::Hevc(n) => *n,
            Self::Av1 => unreachable!(),
        };
        ensure!((1..=4).contains(&length_size), "MP4 NAL length size");
        let mut data = data;
        while !data.is_empty() {
            ensure!(data.len() >= length_size, "truncated MP4 NAL length");
            let size = data[..length_size]
                .iter()
                .fold(0usize, |n, b| (n << 8) | usize::from(*b));
            data = &data[length_size..];
            ensure!(size > 0 && size <= data.len(), "invalid MP4 NAL extent");
            self.check_nal(&data[..size])?;
            data = &data[size..];
        }
        Ok(())
    }
    fn check_nal(&self, nal: &[u8]) -> Result<()> {
        ensure!(!nal.is_empty(), "empty MP4 NAL");
        match self {
            Self::Avc(_) => {
                let kind = nal[0] & 31;
                ensure!(
                    !matches!(kind, 13 | 14 | 15 | 20 | 21),
                    "layered AVC is unsupported"
                );
                if kind == 7 {
                    let unit = oxideav_h264::nal::parse_nal_unit(nal)
                        .map_err(|e| anyhow::anyhow!("MP4 AVC NAL: {e:?}"))?;
                    let sps = oxideav_h264::sps::Sps::parse(&unit.rbsp)
                        .map_err(|e| anyhow::anyhow!("MP4 AVC SPS: {e:?}"))?;
                    coded_dimensions(
                        (u64::from(sps.pic_width_in_mbs_minus1) + 1) * 16,
                        (u64::from(sps.pic_height_in_map_units_minus1) + 1)
                            * 16
                            * if sps.frame_mbs_only_flag { 1 } else { 2 },
                    )?;
                }
            }
            Self::Hevc(_) => {
                ensure!(nal.len() >= 2, "truncated HEVC NAL");
                let layer = ((nal[0] & 1) << 5) | (nal[1] >> 3);
                ensure!(layer == 0, "layered HEVC is unsupported");
                if (nal[0] >> 1) & 63 == 33 {
                    let unit = oxideav_h265::hvcc::nal_unit_from_coded(nal)
                        .map_err(|e| anyhow::anyhow!("MP4 HEVC NAL: {e:?}"))?;
                    check_hevc_sps(&unit.rbsp)?;
                }
            }
            Self::Av1 => unreachable!(),
        }
        Ok(())
    }
}
fn check_hevc_sps(rbsp: &[u8]) -> Result<()> {
    let sps = oxideav_h265::sps::SeqParameterSet::parse(rbsp)
        .map_err(|e| anyhow::anyhow!("MP4 HEVC SPS: {e:?}"))?;
    coded_dimensions(
        u64::from(sps.pic_width_in_luma_samples),
        u64::from(sps.pic_height_in_luma_samples),
    )
}
fn coded_dimensions(width: u64, height: u64) -> Result<()> {
    ensure!(
        width > 0
            && height > 0
            && width <= 8192
            && height <= 8192
            && width * height <= MAX_FRAME_BYTES / 8,
        "MP4 coded sequence dimensions exceed decoder budget"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn coded_dimensions_are_bounded_before_decoder_allocation() {
        assert!(super::coded_dimensions(3840, 2160).is_ok());
        for (w, h) in [(0, 100), (8192, 8192), (u64::MAX, 4), (64, 0)] {
            assert!(super::coded_dimensions(w, h).is_err());
        }
    }
}
