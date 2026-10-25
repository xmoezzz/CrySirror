// SPDX-License-Identifier: MPL-2.0
//! Incremental MPEG CPU services. Callers own readers, cancellation, queues,
//! playback clocks and native policy. No VM, filesystem path or device is held.
use anyhow::{Context, Result, bail, ensure};
use na_mpeg2_decoder::{Ac3AudioDecoder, Demuxer, MpaAudioDecoder, StreamType};
use std::io::{Read, Seek, SeekFrom};
pub const CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    pub pts_ms: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioChunk {
    pub pts_ms: i64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DecodeStats {
    pub encoded_bytes: u64,
    pub outputs: u64,
    pub cancelled: bool,
}
/// Sequence metadata read through the caller's seekable capability. Cursor
/// position is restored; absence of a header is distinct from a read failure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequenceInfo {
    pub width: u32,
    pub height: u32,
    pub frame_ms: f64,
}
pub fn sequence_info(reader: &mut (impl Read + Seek)) -> Result<Option<SequenceInfo>> {
    let start = reader.stream_position()?;
    let mut head = Vec::new();
    let read = reader.by_ref().take(64 * 1024).read_to_end(&mut head);
    let restored = reader.seek(SeekFrom::Start(start));
    read.context("MPEG sequence read")?;
    restored.context("restore MPEG sequence cursor")?;
    Ok(
        siglus_assets::mpeg2::find_sequence_header(&head).map(|h| SequenceInfo {
            width: u32::from(h.width),
            height: u32::from(h.height),
            frame_ms: siglus_assets::mpeg2::fps_from_frame_rate_code(h.frame_rate_code)
                .map_or(1000. / 29.97, |fps| 1000. / f64::from(fps)),
        }),
    )
}
fn video_frame(
    f: &na_mpeg2_decoder::Frame,
    crop: Option<(u32, u32)>,
    index: u64,
    per_frame: f64,
) -> Result<VideoFrame> {
    ensure!(
        f.width > 0
            && f.height > 0
            && f.width <= 8192
            && f.height <= 8192
            && f.width * f.height * 4 <= MAX_FRAME_BYTES,
        "MPEG decoded frame budget"
    );
    let (w, h) = (f.width as u32, f.height as u32);
    let mut rgba = vec![0; f.width * f.height * 4];
    na_mpeg2_decoder::frame_to_rgba_bt601_limited(f, &mut rgba);
    let (width, height) = crop.filter(|&(cw, ch)| cw < w || ch < h).unwrap_or((w, h));
    ensure!(
        width > 0 && height > 0 && width <= w && height <= h,
        "MPEG sequence crop exceeds decoded frame"
    );
    if width != w || height != h {
        let mut cropped = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height as usize {
            cropped.extend_from_slice(&rgba[y * f.width * 4..y * f.width * 4 + width as usize * 4]);
        }
        rgba = cropped;
    }
    Ok(VideoFrame {
        pts_ms: f
            .pts_90k
            .map_or((index as f64 * per_frame) as i64, |pts| pts / 90),
        width,
        height,
        rgba,
    })
}
/// Callback true continues, false stops intentionally. Read/decode/callback
/// errors are errors, never an EOF. Sequence crop, BT601 and PTS truncation
/// match the pinned RealLive movie helper; delayed frames flush at true EOF.
pub fn decode_video<R: Read + Seek>(
    mut reader: R,
    cancel: impl Fn() -> bool,
    mut output: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    let mut stats = DecodeStats::default();
    if cancel() {
        stats.cancelled = true;
        return Ok(stats);
    }
    let header = sequence_info(&mut reader)?;
    let per_frame = header.map_or(1000. / 29.97, |h| h.frame_ms);
    let crop = header.map(|h| (h.width, h.height));
    let mut pipe = na_mpeg2_decoder::MpegVideoPipeline::new();
    let mut bytes = vec![0; CHUNK_BYTES];
    let mut alive = true;
    let mut callback_error = None;
    loop {
        if cancel() || !alive {
            stats.cancelled = true;
            break;
        }
        let n = reader.read(&mut bytes).context("MPEG video read")?;
        if n == 0 {
            let result = pipe.flush_with(|f| {
                if alive && !cancel() {
                    match video_frame(&f, crop, stats.outputs, per_frame).and_then(&mut output) {
                        Ok(keep) => {
                            stats.outputs += 1;
                            alive = keep
                        }
                        Err(e) => {
                            callback_error = Some(e);
                            alive = false
                        }
                    }
                }
            });
            if let Some(e) = callback_error {
                return Err(e);
            }
            result.map_err(|e| anyhow::anyhow!("MPEG video flush: {e}"))?;
            stats.cancelled = !alive || cancel();
            break;
        }
        stats.encoded_bytes = stats
            .encoded_bytes
            .checked_add(n as u64)
            .context("MPEG input counter overflow")?;
        let result = pipe.push_with(&bytes[..n], None, |f| {
            if alive && !cancel() {
                match video_frame(&f, crop, stats.outputs, per_frame).and_then(&mut output) {
                    Ok(keep) => {
                        stats.outputs += 1;
                        alive = keep
                    }
                    Err(e) => {
                        callback_error = Some(e);
                        alive = false
                    }
                }
            }
        });
        if let Some(e) = callback_error {
            return Err(e);
        }
        result.map_err(|e| anyhow::anyhow!("MPEG video decode: {e}"))?;
    }
    ensure!(
        stats.cancelled || stats.outputs > 0,
        "MPEG stream produced no video frames"
    );
    Ok(stats)
}
fn chunk(pts_ms: i64, rate: u32, channels: u16, samples: &[f32]) -> Result<AudioChunk> {
    ensure!(
        (8000..=192000).contains(&rate)
            && (1..=8).contains(&channels)
            && samples.len() <= 4 * 1024 * 1024
            && samples.len() % channels as usize == 0,
        "MPEG audio chunk format/budget"
    );
    ensure!(
        samples.iter().all(|v| v.is_finite()),
        "MPEG audio nonfinite sample"
    );
    Ok(AudioChunk {
        pts_ms,
        sample_rate: rate,
        channels,
        samples: samples
            .iter()
            .map(|&s| (s.clamp(-1., 1.) * 32767.) as i16)
            .collect(),
    })
}
fn lpcm(data: &[u8], pts: i64) -> Result<AudioChunk> {
    ensure!(data.len() >= 8, "truncated DVD LPCM packet");
    let l = &data[4..];
    let format = l[1];
    let channels = u16::from((format & 7) + 1);
    let rate = match (format >> 4) & 3 {
        0 => 48000,
        1 => 96000,
        2 => 44100,
        _ => 32000,
    };
    let samples = match format >> 6 {
        0 => l[3..]
            .chunks_exact(2)
            .map(|b| i16::from_be_bytes([b[0], b[1]]) as f32 / 32768.)
            .collect::<Vec<_>>(),
        2 => l[3..]
            .chunks_exact(3)
            .map(|b| {
                let raw = ((b[0] as i32) << 24) | ((b[1] as i32) << 16) | ((b[2] as i32) << 8);
                raw as f32 / 2147483648.
            })
            .collect(),
        _ => bail!("unsupported DVD LPCM bit depth"),
    };
    ensure!(!samples.is_empty(), "empty DVD LPCM packet");
    chunk(pts, rate, channels, &samples)
}
/// Strict incremental audio demux: unlike the original convenience pipeline,
/// decoder Result errors are not discarded. Codec-internal resynchronization
/// remains codec behavior; no claim of detecting every damaged bit is made.
pub fn decode_audio<R: Read>(
    mut reader: R,
    cancel: impl Fn() -> bool,
    mut output: impl FnMut(AudioChunk) -> Result<bool>,
) -> Result<DecodeStats> {
    let mut stats = DecodeStats::default();
    let mut demux = Demuxer::new_auto();
    let mut mpa = MpaAudioDecoder::new();
    let mut ac3 = Ac3AudioDecoder::new();
    let mut packets = Vec::new();
    let mut bytes = vec![0; CHUNK_BYTES];
    let mut alive = true;
    loop {
        if cancel() || !alive {
            stats.cancelled = true;
            break;
        }
        let n = reader.read(&mut bytes).context("MPEG audio read")?;
        if n == 0 {
            break;
        }
        stats.encoded_bytes = stats
            .encoded_bytes
            .checked_add(n as u64)
            .context("MPEG audio input counter")?;
        packets.clear();
        demux.push_into(&bytes[..n], None, &mut packets);
        for packet in packets.drain(..) {
            if cancel() || !alive {
                stats.cancelled = true;
                break;
            }
            let pts = packet.pts_90k.map(|p| p / 90);
            let mut callback_error = None;
            let mut emit = |pts, rate, channels, samples: &[f32]| {
                if alive && !cancel() {
                    match chunk(pts, rate, channels, samples).and_then(&mut output) {
                        Ok(keep) => {
                            stats.outputs += 1;
                            alive = keep
                        }
                        Err(e) => {
                            callback_error = Some(e);
                            alive = false
                        }
                    }
                }
            };
            match packet.stream_type {
                StreamType::MpegAudio => mpa
                    .push_with(&packet.data, pts, |c| {
                        emit(c.pts_ms, c.sample_rate, c.channels, &c.samples)
                    })
                    .map_err(|e| anyhow::anyhow!("MPEG audio decode: {e}"))?,
                StreamType::DvdLpcmAudio => {
                    let (&id, rest) = packet
                        .data
                        .split_first()
                        .context("empty DVD private audio packet")?;
                    match id {
                        0x80..=0x87 => {
                            let payload = if rest.len() >= 3 { &rest[3..] } else { rest };
                            ac3.push_with(payload, Some(pts.unwrap_or(0)), |c| {
                                emit(c.pts_ms, c.sample_rate, c.channels, &c.samples)
                            })
                            .map_err(|e| anyhow::anyhow!("AC3 decode: {e}"))?;
                        }
                        0xa0..=0xaf => {
                            let c = lpcm(&packet.data, pts.unwrap_or(0))?;
                            alive = output(c)?;
                            stats.outputs += 1;
                        }
                        _ => bail!("unsupported DVD private audio substream {id:#x}"),
                    }
                }
                _ => {}
            }
            if let Some(e) = callback_error {
                return Err(e);
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_metadata_preserves_the_capability_cursor() {
        let bytes = include_bytes!("../tests/fixtures/red_blue.mpg");
        let mut reader = std::io::Cursor::new(bytes);
        let info = sequence_info(&mut reader).unwrap().unwrap();
        assert!(info.width > 0 && info.height > 0 && info.frame_ms > 0.);
        assert_eq!(reader.position(), 0);
        struct PartialFailure {
            position: u64,
        }
        impl Read for PartialFailure {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.position > 0 {
                    return Err(std::io::Error::other("partial source fault"));
                }
                out[0] = 0;
                self.position += 1;
                Ok(1)
            }
        }
        impl Seek for PartialFailure {
            fn seek(&mut self, target: SeekFrom) -> std::io::Result<u64> {
                match target {
                    SeekFrom::Current(0) => Ok(self.position),
                    SeekFrom::Start(p) => {
                        self.position = p;
                        Ok(p)
                    }
                    _ => unreachable!(),
                }
            }
        }
        let mut reader = PartialFailure { position: 0 };
        assert!(sequence_info(&mut reader).is_err());
        assert_eq!(reader.position, 0);
    }
    #[test]
    fn native_crop_and_pts_keep_top_left_pixels_and_signed_time() {
        let mut frame =
            na_mpeg2_decoder::Frame::new(16, 16, na_mpeg2_decoder::PixelFormat::Yuv420p);
        frame.data_y.fill(235);
        frame.pts_90k = Some(-9001);
        let actual = video_frame(&frame, Some((10, 9)), 8, 40.).unwrap();
        assert_eq!((actual.width, actual.height, actual.pts_ms), (10, 9, -100));
        assert_eq!(actual.rgba, vec![255; 10 * 9 * 4]);
        assert!(video_frame(&frame, Some((20, 9)), 0, 40.).is_err());
    }
    #[test]
    fn dvd_lpcm_uses_original_integer_conversion_and_rejects_unsupported_depth() {
        let mut bytes = vec![0xa0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&[0x80, 0, 0, 0, 0x7f, 0xff]);
        let result = lpcm(&bytes, 123).unwrap();
        assert_eq!(
            (result.pts_ms, result.sample_rate, result.channels),
            (123, 48000, 1)
        );
        assert_eq!(result.samples, vec![-32767, 0, 32766]);
        bytes[5] = 1 << 6;
        assert!(lpcm(&bytes, 0).is_err());
    }
}
