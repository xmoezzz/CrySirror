//! Incremental ASF/WMV services over caller-owned readers. No VM, path or device.
use crate::mpeg::{DecodeStats, VideoFrame};
use anyhow::{Context, Result, ensure};
use std::io::{Read, Seek, SeekFrom};

pub fn is_asf(reader: &mut (impl Read + Seek)) -> Result<bool> {
    let at = reader.stream_position()?;
    let mut bytes = [0u8; 16];
    let result = reader.read_exact(&mut bytes);
    reader.seek(SeekFrom::Start(at))?;
    match result {
        Ok(()) => Ok(bytes == wmv_decoder::asf::GUID_ASF_HEADER.0),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn metadata(reader: &mut (impl Read + Seek)) -> Result<wmv_decoder::asf::AsfFile> {
    let at = reader.stream_position()?;
    ensure!(at == 0, "ASF capability cursor must start at zero");
    let result = wmv_decoder::asf::AsfFile::open(reader);
    reader.seek(SeekFrom::Start(at))?;
    let info = result.context("ASF metadata")?;
    for v in &info.video_streams {
        crysirror_image::rgba_len(v.width, v.height)?;
        ensure!(
            v.width % 2 == 0 && v.height % 2 == 0,
            "WMV 4:2:0 coded dimensions"
        );
    }
    for a in &info.audio_streams {
        ensure!(
            (8000..=192000).contains(&a.sample_rate) && (1..=8).contains(&a.channels),
            "ASF audio format"
        );
    }
    Ok(info)
}

pub fn decode_video<R: Read + Seek>(
    reader: R,
    cancelled: impl FnMut() -> bool,
    publish: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    decode_video_with_metadata(reader, cancelled, |_| Ok(()), publish)
}
/// Publish the selected decoder stream's header before decoding any picture.
pub fn decode_video_with_metadata<R: Read + Seek>(
    mut reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut publish_metadata: impl FnMut(crate::VideoMetadata) -> Result<()>,
    mut publish: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    if cancelled() {
        return Ok(DecodeStats {
            cancelled: true,
            ..Default::default()
        });
    }
    let info = metadata(&mut reader)?;
    let mut decoder =
        wmv_decoder::AsfWmv2Decoder::open(reader).context("open engine WMV decoder")?;
    let selected = decoder.video_stream_info();
    publish_metadata(crate::VideoMetadata {
        size: [selected.width, selected.height],
        duration_ms: info.play_duration_ms,
        frame_ms: None,
    })?;
    let mut stats = DecodeStats::default();
    loop {
        if cancelled() {
            stats.cancelled = true;
            break;
        }
        let Some(f) = decoder.next_frame().context("decode engine WMV frame")? else {
            break;
        };
        let (width, height) = (f.frame.width, f.frame.height);
        crysirror_image::rgba_len(width, height)?;
        let frame = VideoFrame {
            pts_ms: i64::from(f.pts_ms),
            width,
            height,
            rgba: wmv_decoder::yuv420p_to_rgba_scaled(&f.frame, width, height),
        };
        stats.outputs += 1;
        if !publish(frame)? {
            stats.cancelled = true;
            break;
        }
    }
    Ok(stats)
}

pub struct AudioChunk {
    pub pts_ms: i64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}
pub fn decode_audio<R: Read + Seek>(
    mut reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut publish: impl FnMut(AudioChunk) -> Result<bool>,
) -> Result<DecodeStats> {
    if cancelled() {
        return Ok(DecodeStats {
            cancelled: true,
            ..Default::default()
        });
    }
    let info = metadata(&mut reader)?;
    if info.audio_streams.is_empty() {
        return Ok(DecodeStats::default());
    }
    ensure!(
        info.audio_streams
            .iter()
            .any(|a| matches!(a.format_tag, 0x0160..=0x0162)),
        "ASF audio codec unsupported"
    );
    let mut decoder =
        wmv_decoder::AsfWmaDecoder::open(reader).context("open engine WMA decoder")?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    let mut stats = DecodeStats::default();
    loop {
        if cancelled() {
            stats.cancelled = true;
            break;
        }
        let Some(f) = decoder.next_frame().context("decode engine WMA frame")? else {
            break;
        };
        ensure!(
            f.frame.samples.len() <= 1024 * 1024
                && f.frame.samples.len() % usize::from(channels) == 0,
            "WMA decoded block budget/alignment"
        );
        ensure!(
            f.frame
                .samples
                .iter()
                .all(|x| x.is_finite() && (-16.0..=16.0).contains(x)),
            "WMA invalid PCM"
        );
        stats.outputs += 1;
        if !publish(AudioChunk {
            pts_ms: i64::from(f.pts_ms),
            sample_rate: rate,
            channels,
            samples: f.frame.samples,
        })? {
            stats.cancelled = true;
            break;
        }
    }
    Ok(stats)
}
