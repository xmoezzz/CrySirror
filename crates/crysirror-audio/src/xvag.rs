// SPDX-License-Identifier: MPL-2.0
//! Sony `XVAG` (PlayStation 3) streaming playback and WAVE export. The container is a header
//! of tagged chunks (`fmat` gives channels, codec, frames and rate) followed
//! by the payload at the header size. Codec 6 is PS-ADPCM: 16-byte frames of
//! 28 samples, channels interleaved frame by frame. Other codecs (MPEG,
//! ATRAC9) are rejected.
use crate::decoder::{AudioSource, MediaSource, StereoFrame};
use anyhow::{Context, Result, bail, ensure};
use std::io::{BufReader, Read, Seek, SeekFrom};

pub const CODEC_PS_ADPCM: u32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub channels: u16,
    pub codec: u32,
    pub rate: u32,
    /// Frames per channel.
    pub frames: u64,
    /// Byte range of the payload.
    pub data: std::ops::Range<usize>,
}

fn u32be(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(o..o + 4).context("XVAG truncated")?.try_into()?,
    ))
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Self> {
        Self::from_prefix(b, b.len() as u64)
    }

    fn from_prefix(b: &[u8], file_len: u64) -> Result<Self> {
        ensure!(b.get(..4) == Some(b"XVAG".as_slice()), "not an XVAG stream");
        // Retail streams with flags 0x01 and 0x03 both carry big-endian
        // chunk sizes and PS-ADPCM data.
        ensure!(matches!(b.get(8), Some(1 | 3)), "XVAG byte order or flags");
        let start = u32be(b, 4)? as usize;
        ensure!(start >= 0x20 && start <= b.len(), "XVAG header size");
        let mut at = 0x20;
        while at + 8 <= start {
            let size = u32be(b, at + 4)? as usize;
            let end = (at + 8)
                .checked_add(size)
                .filter(|&end| end <= start)
                .context("XVAG chunk bounds")?;
            if &b[at..at + 4] == b"fmat" {
                ensure!(size >= 0x1c, "XVAG fmat size");
                let f = at + 8;
                let channels = u32be(b, f)?;
                let codec = u32be(b, f + 4)?;
                let frames = u32be(b, f + 8)?;
                let layers = u32be(b, f + 16)?;
                let rate = u32be(b, f + 20)?;
                let size = u32be(b, f + 24)? as usize;
                ensure!(
                    (1..=16).contains(&channels) && (1..=192_000).contains(&rate),
                    "XVAG format"
                );
                ensure!(layers == 1, "XVAG layered streams");
                let end = start
                    .checked_add(size)
                    .filter(|&e| e as u64 <= file_len)
                    .context("XVAG data size")?;
                return Ok(Self {
                    channels: channels as u16,
                    codec,
                    rate,
                    frames: frames.into(),
                    data: start..end,
                });
            }
            at = end;
        }
        bail!("XVAG without fmat chunk")
    }
}

// PS3 extended VAG predictors, expressed in 1/256 units to preserve the
// quarter coefficients. Predictors 0–4 are exactly the original 1/64 table.
// BCAS20097 uses 5, 6 and 9 even in the short drake-impact-foot bank tone.
// Format reference: vgmstream/src/coding/psx_decoder.c, ps_adpcm_coefs_f.
const COEF: [(i32, i32); 16] = [
    (0, 0), (240, 0), (460, -208), (392, -220), (488, -240),
    (120, 0), (230, -104), (196, -110), (244, -120), (60, 0),
    (115, -52), (98, -55), (122, -60), (128, -240), (60, -240), (28, -240),
];

/// Decode one channel's run of 16-byte PS-ADPCM frames, keeping the
/// predictor history across calls.
pub fn ps_adpcm_frame(frame: &[u8; 16], hist: &mut (i32, i32), out: &mut Vec<i16>) -> Result<()> {
    let mut samples = [0; 28];
    decode_frame(frame, hist, &mut samples)?;
    out.extend_from_slice(&samples);
    Ok(())
}

fn decode_frame(frame: &[u8; 16], hist: &mut (i32, i32), out: &mut [i16; 28]) -> Result<()> {
    // Flag 7 is the silent stop/loop pad. End flag 1 belongs to a real
    // final frame; the predictor in its other header byte still applies.
    if frame[1] == 7 {
        out.fill(0);
        *hist = (0, 0);
        return Ok(());
    }
    let shift = (frame[0] & 0x0f) as i32;
    let filter = (frame[0] >> 4) as usize;
    let (c1, c2) = COEF[filter];
    // Shifts above 12 are treated as 9 by the hardware decoder.
    let shift = if shift > 12 { 9 } else { shift };
    for (pair, byte) in out.chunks_exact_mut(2).zip(&frame[2..]) {
        for (sample, nibble) in pair.iter_mut().zip([byte & 0x0f, byte >> 4]) {
            let n = ((nibble as i32) << 28) >> 28;
            let s = ((n << 12) >> shift) + ((hist.0 * c1 + hist.1 * c2 + 128) >> 8);
            let s = s.clamp(-32768, 32767);
            hist.1 = hist.0;
            hist.0 = s;
            *sample = s as i16;
        }
    }
    Ok(())
}

// cpan identifies the speaker of each encoded plane. Films and logos use
// different orders even when both are six-channel.
fn stereo_weights(header: &[u8], channels: usize) -> Result<Vec<StereoFrame>> {
    if channels == 1 {
        return Ok(vec![[1., 1.]]);
    }
    let mut at = 0x20;
    while at + 8 <= header.len() {
        let size = u32be(header, at + 4)? as usize;
        let end = (at + 8)
            .checked_add(size)
            .filter(|&e| e <= header.len())
            .context("XVAG chunk bounds")?;
        if &header[at..at + 4] == b"cpan" {
            let data = &header[at + 8..end];
            ensure!(
                size >= 16 + channels * 16 && u32be(data, 0)? as usize == channels,
                "XVAG cpan layout"
            );
            let side = std::f32::consts::FRAC_1_SQRT_2;
            return (0..channels)
                .map(|i| {
                    Ok(match u32be(data, 16 + i * 16)? {
                        0x80000001 => [1., 0.],
                        0x80000002 => [0., 1.],
                        0x80000004 => [side, side],
                        0x80000008 => [0., 0.], // omit the dedicated LFE in stereo
                        0x80000010 | 0x80000200 => [side, 0.],
                        0x80000020 | 0x80000400 => [0., side],
                        role => bail!("XVAG speaker assignment {role:#x}"),
                    })
                })
                .collect();
        }
        at = end;
    }
    ensure!(channels == 2, "multichannel XVAG without cpan");
    Ok(vec![[1., 0.], [0., 1.]])
}

struct Stream {
    reader: BufReader<Box<dyn MediaSource>>,
    header: Header,
    weights: Vec<StereoFrame>,
    histories: Vec<(i32, i32)>,
    row: [StereoFrame; 28],
    position: u64,
}

pub(crate) fn open(mut reader: Box<dyn MediaSource>) -> Result<Box<dyn AudioSource>> {
    let mut first = [0; 8];
    reader.read_exact(&mut first)?;
    let start = u32be(&first, 4)? as usize;
    ensure!((0x20..=65536).contains(&start), "XVAG header size");
    let mut prefix = vec![0; start];
    prefix[..8].copy_from_slice(&first);
    reader.read_exact(&mut prefix[8..])?;
    let header = Header::from_prefix(&prefix, reader.byte_len().context("XVAG source length")?)?;
    ensure!(
        header.codec == CODEC_PS_ADPCM,
        "XVAG codec {}",
        header.codec
    );
    let channels = usize::from(header.channels);
    ensure!(
        header.frames.div_ceil(28) <= (header.data.len() / (16 * channels)) as u64,
        "XVAG frame count"
    );
    Ok(Box::new(Stream {
        reader: BufReader::with_capacity(64 << 10, reader),
        weights: stereo_weights(&prefix, channels)?,
        histories: vec![(0, 0); channels],
        header,
        row: [[0.; 2]; 28],
        position: 0,
    }))
}

impl AudioSource for Stream {
    fn frame_count(&self) -> Option<u64> {
        Some(self.header.frames)
    }
    fn sample_rate(&self) -> u32 {
        self.header.rate
    }
    fn channel_count(&self) -> Option<u16> {
        Some(self.header.channels)
    }

    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let wanted = out.len().min((self.header.frames - self.position) as usize);
        let mut done = 0;
        while done < wanted {
            let at = (self.position % 28) as usize;
            if at == 0 {
                self.row.fill([0.; 2]);
                for (hist, weight) in self.histories.iter_mut().zip(&self.weights) {
                    let mut frame = [0; 16];
                    self.reader.read_exact(&mut frame)?;
                    let mut samples = [0; 28];
                    decode_frame(&frame, hist, &mut samples)?;
                    for (dst, s) in self.row.iter_mut().zip(samples) {
                        let value = f32::from(s) / 32768.;
                        dst[0] += value * weight[0];
                        dst[1] += value * weight[1];
                    }
                }
            }
            let n = (28 - at).min(wanted - done);
            out[done..done + n].copy_from_slice(&self.row[at..at + n]);
            self.position += n as u64;
            done += n;
        }
        Ok(done)
    }

    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(frame <= self.header.frames, "XVAG seek outside source");
        if frame < self.position {
            self.reader
                .seek(SeekFrom::Start(self.header.data.start as u64))?;
            self.histories.fill((0, 0));
            self.position = 0;
        }
        let mut discard = [[0.; 2]; 512];
        while self.position < frame {
            let n = (frame - self.position).min(discard.len() as u64) as usize;
            self.read(&mut discard[..n])?;
        }
        Ok(())
    }
}
/// Decode to interleaved PCM16.
pub fn decode(b: &[u8], h: &Header) -> Result<Vec<i16>> {
    ensure!(h.codec == CODEC_PS_ADPCM, "XVAG codec {}", h.codec);
    let channels = h.channels as usize;
    let data = &b[h.data.clone()];
    let rows = data.len() / (16 * channels);
    ensure!(h.frames <= rows as u64 * 28, "XVAG frame count");
    let mut planes = vec![Vec::with_capacity(rows * 28); channels];
    let mut hist = vec![(0, 0); channels];
    for row in data.chunks_exact(16 * channels) {
        for (c, frame) in row.chunks_exact(16).enumerate() {
            ps_adpcm_frame(frame.try_into()?, &mut hist[c], &mut planes[c])?;
        }
    }
    let frames = h.frames as usize;
    let mut out = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        for plane in &planes {
            out.push(plane[i]);
        }
    }
    Ok(out)
}

/// A complete WAVE (PCM16) stream for the shared decoders.
pub fn to_wave(b: &[u8]) -> Result<Vec<u8>> {
    let h = Header::parse(b)?;
    let pcm = decode(b, &h)?;
    let channels = u32::from(h.channels);
    let bytes = u32::try_from(pcm.len() * 2).context("XVAG WAVE size")?;
    let mut w = Vec::with_capacity(44 + bytes as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + bytes).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&(channels as u16).to_le_bytes());
    w.extend_from_slice(&h.rate.to_le_bytes());
    w.extend_from_slice(&(h.rate * channels * 2).to_le_bytes());
    w.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&bytes.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(channels: u32, frames: u32, data: &[u8]) -> Vec<u8> {
        let mut b = b"XVAG".to_vec();
        b.extend_from_slice(&0x44u32.to_be_bytes());
        b.push(1);
        b.resize(0x20, 0);
        b.extend_from_slice(b"fmat");
        b.extend_from_slice(&0x1cu32.to_be_bytes());
        for v in [channels, 6, frames, frames, 1, 48000, data.len() as u32] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn frames_interleave_per_channel() {
        // Left: shift 12, filter 0, nibble 1 → 1. Right: nibble -1 → -1.
        let mut data = vec![0u8; 32];
        data[0] = 0x0c;
        data[2] = 0x01;
        data[16] = 0x0c;
        data[18] = 0x0f;
        let b = stream(2, 2, &data);
        let h = Header::parse(&b).unwrap();
        assert_eq!((h.channels, h.rate, h.frames), (2, 48000, 2));
        assert_eq!(decode(&b, &h).unwrap(), [1, -1, 0, 0]);
        assert_eq!(to_wave(&b).unwrap().len(), 44 + 8);
    }

    #[test]
    fn streamed_film_respects_speaker_order_and_exact_seek() {
        // A centre impulse carries dialogue. The following row uses predictor
        // history. The second speaker carries only LFE and must stay inaudible.
        let mut data = vec![0u8; 2 * 16 * 6];
        for row in data.chunks_exact_mut(96) {
            row[16..32].fill(0x11);
            row[16] = 0x10;
            row[80..96].fill(0x33);
            row[80] = 0;
        }
        let mut bytes = stream(6, 53, &data);
        let roles: [u32; 6] = [1, 4, 2, 16, 32, 8];
        let mut cpan = b"cpan".to_vec();
        cpan.extend_from_slice(&112u32.to_be_bytes());
        cpan.extend_from_slice(&6u32.to_be_bytes());
        cpan.resize(24, 0);
        for role in roles {
            cpan.extend_from_slice(&(0x80000000 | role).to_be_bytes());
            cpan.resize(cpan.len() + 12, 0);
        }
        let start = 0x44 + cpan.len();
        bytes[8] = 3;
        bytes.splice(0x44..0x44, cpan);
        bytes[4..8].copy_from_slice(&(start as u32).to_be_bytes());
        let header = Header::parse(&bytes).unwrap();
        let raw = decode(&bytes, &header).unwrap();
        let mut player = crate::decoder::open_memory(bytes.into(), None).unwrap();
        let mut result = [[0.; 2]; 60];
        assert_eq!(player.read(&mut result[..19]).unwrap(), 19);
        assert_eq!(player.read(&mut result[19..]).unwrap(), 34);
        assert!(result[12][0] > 0.4);
        for (frame, planes) in result.iter().zip(raw.chunks_exact(6)) {
            let centre = f32::from(planes[1]) / 32768. * std::f32::consts::FRAC_1_SQRT_2;
            assert_eq!(*frame, [centre; 2]);
        }
        player.seek(16).unwrap();
        let mut replay = [[0.; 2]; 21];
        assert_eq!(player.read(&mut replay).unwrap(), 21);
        assert_eq!(replay, result[16..37]);
        assert!(player.seek(54).is_err());
    }

    #[test]
    fn legacy_and_extended_filter_history() {
        let mut out = Vec::new();
        let mut hist = (1000, 0);
        let mut f = [0u8; 16];
        f[0] = 0x1c;
        ps_adpcm_frame(&f, &mut hist, &mut out).unwrap();
        // 60/64 decay of the previous sample.
        assert_eq!(out[0], ((1000 * 60 + 32) >> 6) as i16);
        // All extended predictors, including the fractional pairs, with
        // an impulse in each history slot and zero residual.
        for (filter, expected) in [120, 282, 251, 304, 60, 141, 126, 152, 248, 180, 148].into_iter().enumerate() {
            f[0] = ((5 + filter as u8) << 4) | 12;
            hist = (256, -128);
            out.clear();
            ps_adpcm_frame(&f, &mut hist, &mut out).unwrap();
            assert_eq!(out[0], expected, "filter {}", 5 + filter);
        }
        let b = stream(1, 29, &[0; 16]);
        assert!(decode(&b, &Header::parse(&b).unwrap()).is_err());
        assert!(Header::parse(&b[..0x30]).is_err());
        f[0] = 0xfc;
        f[1] = 7;
        f[2..].fill(0x77);
        out.clear();
        ps_adpcm_frame(&f, &mut hist, &mut out).unwrap();
        assert_eq!(out, [0; 28]);
        assert_eq!(hist, (0, 0));

    }
}
