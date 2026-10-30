//! Worker-safe incremental OMV/Ogg Theora and Vorbis over capability readers.
use crate::{
    mpeg::{DecodeStats, VideoFrame},
    omv,
};
use anyhow::{Context, Result, ensure};
use std::io::{Read, Seek, SeekFrom};

pub fn is_input(reader: &mut (impl Read + Seek)) -> Result<bool> {
    let start = reader.stream_position()?;
    let mut head = Vec::new();
    let result = reader.by_ref().take(0x58).read_to_end(&mut head);
    reader.seek(SeekFrom::Start(start))?;
    result?;
    if head.starts_with(b"OggS") {
        return Ok(true);
    }
    Ok(head.len() == 0x58
        && (0x58..=64 * 1024 * 1024).contains(&u32::from_le_bytes(head[..4].try_into()?))
        && u32::from_le_bytes(head[0x28..0x2c].try_into()?) <= 2)
}
struct Source<R> {
    reader: R,
    base: u64,
    len: u64,
    pos: u64,
}
impl<R: Read + Seek> Read for Source<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(self.len.saturating_sub(self.pos) as usize);
        let n = self.reader.read(&mut out[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl<R: Read + Seek> Seek for Source<R> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let pos = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.pos) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.len) + i128::from(n),
        };
        if !(0..=i128::from(self.len)).contains(&pos) {
            return Err(std::io::Error::other("Ogg source seek range"));
        }
        self.reader.seek(SeekFrom::Start(self.base + pos as u64))?;
        self.pos = pos as u64;
        Ok(self.pos)
    }
}
fn source<R: Read + Seek>(mut reader: R) -> Result<(Source<R>, Option<omv::OmvHeader>)> {
    reader.seek(SeekFrom::Start(0))?;
    let mut head = Vec::new();
    reader.by_ref().take(0x58).read_to_end(&mut head)?;
    let header = if head.starts_with(b"OggS") {
        None
    } else {
        Some(omv::header(&head)?)
    };
    let end = reader.seek(SeekFrom::End(0))?;
    ensure!(end <= 1024 * 1024 * 1024, "Ogg encoded byte budget");
    reader.seek(SeekFrom::Start(0))?;
    let mut prefix = Vec::new();
    let mut read = 0u64;
    let base = loop {
        let mut buf = [0u8; 4096];
        let n = reader.read(&mut buf)?;
        ensure!(
            n > 0 && read + n as u64 <= 64 * 1024 * 1024,
            "Ogg payload/header budget"
        );
        prefix.extend_from_slice(&buf[..n]);
        if let Some(i) = prefix.windows(4).position(|b| b == b"OggS") {
            break read.saturating_sub((prefix.len() - n) as u64) + i as u64;
        }
        read += n as u64;
        prefix = prefix[prefix.len().saturating_sub(3)..].to_vec();
    };
    reader.seek(SeekFrom::Start(base))?;
    Ok((
        Source {
            reader,
            base,
            len: end - base,
            pos: 0,
        },
        header,
    ))
}
pub fn decode_video<R: Read + Seek>(
    reader: R,
    cancelled: impl FnMut() -> bool,
    publish: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    decode_video_with_metadata(reader, cancelled, |_| Ok(()), publish)
}
/// Decode over the same input, publishing validated headers before pictures.
pub fn decode_video_with_metadata<R: Read + Seek>(
    reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut metadata: impl FnMut(crate::VideoMetadata) -> Result<()>,
    mut publish: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    if cancelled() {
        return Ok(DecodeStats {
            cancelled: true,
            ..Default::default()
        });
    }
    let (reader, header) = source(reader)?;
    let mut decoder = siglus_omv_decoder::TheoraVideoStream::open(reader)?;
    let info = decoder.info();
    if header.is_none() {
        ensure!(
            info.pic_x == 0 && info.pic_y == 0,
            "Ogg picture offsets unsupported"
        );
    }
    let size = header
        .as_ref()
        .map(|h| [h.display_width, h.display_height])
        .unwrap_or([u32::try_from(info.width)?, u32::try_from(info.height)?]);
    crysirror_image::rgba_len(size[0], size[1])?;
    let kind = header
        .as_ref()
        .map_or(omv::OMV_THEORA_TYPE_YUV, |h| h.theora_type);
    let frame_ms = header
        .as_ref()
        .filter(|h| h.frame_time_us > 0)
        .map_or(1000.0 / info.fps, |h| f64::from(h.frame_time_us) / 1000.0);
    ensure!(
        frame_ms.is_finite() && frame_ms > 0. && frame_ms <= 10000.,
        "Theora frame clock"
    );
    metadata(crate::VideoMetadata {
        size,
        // OMV page/packet hints do not specify a duration. Plain Theora's
        // identification header likewise contains no total-frame count.
        duration_ms: None,
        frame_ms: Some(frame_ms),
    })?;
    let mut stats = DecodeStats::default();
    loop {
        if cancelled() {
            stats.cancelled = true;
            break;
        }
        let Some(planes) = decoder.read_video_frame()? else {
            break;
        };
        let rgba = omv::to_rgba(
            &planes,
            info.frame_width,
            info.frame_height,
            info.fmt,
            size[0] as i32,
            size[1] as i32,
            kind,
        )?;
        let frame = VideoFrame {
            pts_ms: (stats.outputs as f64 * frame_ms).round() as i64,
            width: size[0],
            height: size[1],
            rgba,
        };
        stats.outputs += 1;
        if !publish(frame)? {
            stats.cancelled = true;
            break;
        }
    }
    Ok(stats)
}
pub fn decode_audio<R: Read + Seek>(
    reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut publish: impl FnMut(crate::asf::AudioChunk) -> Result<bool>,
) -> Result<DecodeStats> {
    if cancelled() {
        return Ok(DecodeStats {
            cancelled: true,
            ..Default::default()
        });
    }
    let (reader, _) = source(reader)?;
    let Some(mut decoder) = siglus_omv_decoder::VorbisStream::open(reader)? else {
        return Ok(DecodeStats::default());
    };
    let (rate, channels) = decoder.info();
    let mut offset = 0u64;
    let mut stats = DecodeStats::default();
    let mut out = vec![0f32; 4096 * usize::from(channels)];
    loop {
        if cancelled() {
            stats.cancelled = true;
            break;
        }
        let n = decoder
            .read_samples(&mut out)
            .context("decode engine Vorbis block")?;
        if n == 0 {
            break;
        }
        ensure!(n % usize::from(channels) == 0, "Vorbis partial frame");
        let pts_ms = (offset * 1000 / u64::from(rate)) as i64;
        offset += n as u64 / u64::from(channels);
        stats.outputs += 1;
        if !publish(crate::asf::AudioChunk {
            pts_ms,
            sample_rate: rate,
            channels,
            samples: out[..n].to_vec(),
        })? {
            stats.cancelled = true;
            break;
        }
    }
    Ok(stats)
}
