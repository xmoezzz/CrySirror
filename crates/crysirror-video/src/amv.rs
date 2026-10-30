// SPDX-License-Identifier: MPL-2.0
//! Incremental AlphaMovie (AJPM, KiriKiri `AlphaMovie.dll`) decoding through
//! the adopted amv_decoder: packets are read from the caller's seekable
//! capability and decoded in file order onto the previous picture (delta
//! frames). Frames are straight RGBA with the movie's own alpha.
use crate::mpeg::{DecodeStats, VideoFrame};
use anyhow::{Context, Result, ensure};
use std::io::{Read, Seek, SeekFrom};

/// Header facts a player needs before the first frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmvInfo {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Frame duration in milliseconds. The header's `fps_num / fps_den` is
    /// the frame duration in seconds (xp3-brute writes 1/30 for 30 fps).
    pub frame_ms: f64,
    /// `fps_den` and `fps_num`, the rate/scale pair AlphaMovie reports.
    pub rate: u32,
    pub scale: u32,
}

pub fn is_input(reader: &mut (impl Read + Seek)) -> Result<bool> {
    let start = reader.stream_position()?;
    let mut magic = [0u8; 4];
    let read = reader.read(&mut magic);
    reader.seek(SeekFrom::Start(start))?;
    Ok(read? == 4 && &magic == b"AJPM")
}

/// Read the header; the cursor position is restored.
pub fn info(reader: &mut (impl Read + Seek)) -> Result<AmvInfo> {
    let start = reader.stream_position()?;
    let header = amv_decoder::AmvFile::from_reader(&mut *reader)
        .map(|f| f.header)
        .context("AlphaMovie header");
    reader.seek(SeekFrom::Start(start))?;
    let h = header?;
    ensure!(
        (1..=8192).contains(&h.width) && (1..=8192).contains(&h.height),
        "AlphaMovie dimensions"
    );
    let frame_ms = 1000. * f64::from(h.fps_num) / f64::from(h.fps_den);
    ensure!(frame_ms.is_finite() && frame_ms > 0. && frame_ms <= 10000., "AlphaMovie frame clock");
    Ok(AmvInfo {
        width: u32::from(h.width),
        height: u32::from(h.height),
        frames: h.frame_count,
        frame_ms,
        rate: h.fps_den,
        scale: h.fps_num,
    })
}

pub fn decode_video<R: Read + Seek>(
    reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut publish: impl FnMut(VideoFrame) -> Result<bool>,
) -> Result<DecodeStats> {
    let mut stats = DecodeStats::default();
    let mut file = amv_decoder::AmvFile::from_reader(reader).context("AlphaMovie header")?;
    let header = file.header.clone();
    let [width, height] = [u32::from(header.width), u32::from(header.height)];
    crysirror_image::rgba_len(width, height)?;
    let frame_ms = 1000. * f64::from(header.fps_num) / f64::from(header.fps_den);
    let mut decoder = amv_decoder::AmvDecoder::new(header.clone(), file.mode, file.qtables.clone())?;
    for index in 0..u64::from(header.frame_count) {
        if cancelled() {
            stats.cancelled = true;
            break;
        }
        let Some(packet) = file.iter_frames().read_next()? else {
            break;
        };
        stats.encoded_bytes += packet.raw_packet.len() as u64;
        let frame = decoder.decode_packet(&packet)?;
        ensure!(frame.rgba.len() == (width * height * 4) as usize, "AlphaMovie frame size");
        stats.outputs += 1;
        let pts_ms = (index as f64 * frame_ms).round() as i64;
        if !publish(VideoFrame { pts_ms, width, height, rgba: frame.rgba })? {
            break;
        }
    }
    Ok(stats)
}
