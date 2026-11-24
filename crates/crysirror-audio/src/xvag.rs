// SPDX-License-Identifier: MPL-2.0
//! Sony `XVAG` streams (PlayStation 3) to WAVE. The container is a header
//! of tagged chunks (`fmat` gives channels, codec, frames and rate) followed
//! by the payload at the header size. Codec 6 is PS-ADPCM: 16-byte frames of
//! 28 samples, channels interleaved frame by frame. Other codecs (MPEG,
//! ATRAC9) are rejected.
use anyhow::{bail, ensure, Context, Result};

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
    Ok(u32::from_be_bytes(b.get(o..o + 4).context("XVAG truncated")?.try_into()?))
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Self> {
        ensure!(b.get(..4) == Some(b"XVAG".as_slice()), "not an XVAG stream");
        // Byte 8 is 1 for the big-endian PlayStation 3 layout.
        ensure!(b.get(8) == Some(&1), "XVAG byte order");
        let start = u32be(b, 4)? as usize;
        ensure!(start >= 0x20 && start <= b.len(), "XVAG header size");
        let mut at = 0x20;
        while at + 8 <= start {
            let size = u32be(b, at + 4)? as usize;
            if &b[at..at + 4] == b"fmat" {
                ensure!(size >= 0x1c && at + 8 + size <= start, "XVAG fmat size");
                let f = at + 8;
                let channels = u32be(b, f)?;
                let codec = u32be(b, f + 4)?;
                let frames = u32be(b, f + 8)?;
                let layers = u32be(b, f + 16)?;
                let rate = u32be(b, f + 20)?;
                let size = u32be(b, f + 24)? as usize;
                ensure!((1..=16).contains(&channels) && (1..=192_000).contains(&rate), "XVAG format");
                ensure!(layers == 1, "XVAG layered streams");
                let end = start.checked_add(size).filter(|&e| e <= b.len()).context("XVAG data size")?;
                return Ok(Self { channels: channels as u16, codec, rate, frames: frames.into(), data: start..end });
            }
            at = at.checked_add(8 + size).context("XVAG chunk size")?;
        }
        bail!("XVAG without fmat chunk")
    }
}

const COEF: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// Decode one channel's run of 16-byte PS-ADPCM frames, keeping the
/// predictor history across calls.
pub fn ps_adpcm_frame(frame: &[u8; 16], hist: &mut (i32, i32), out: &mut Vec<i16>) -> Result<()> {
    let shift = (frame[0] & 0x0f) as i32;
    let filter = (frame[0] >> 4) as usize;
    let (c1, c2) = *COEF.get(filter).context("PS-ADPCM filter")?;
    // Shifts above 12 are treated as 9 by the hardware decoder.
    let shift = if shift > 12 { 9 } else { shift };
    for byte in &frame[2..] {
        for nibble in [byte & 0x0f, byte >> 4] {
            let n = ((nibble as i32) << 28) >> 28;
            let s = ((n << 12) >> shift) + ((hist.0 * c1 + hist.1 * c2 + 32) >> 6);
            let s = s.clamp(-32768, 32767);
            hist.1 = hist.0;
            hist.0 = s;
            out.push(s as i16);
        }
    }
    Ok(())
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
    fn filter_history_and_bounds() {
        let mut out = Vec::new();
        let mut hist = (1000, 0);
        let mut f = [0u8; 16];
        f[0] = 0x1c;
        ps_adpcm_frame(&f, &mut hist, &mut out).unwrap();
        // 60/64 decay of the previous sample.
        assert_eq!(out[0], ((1000 * 60 + 32) >> 6) as i16);
        f[0] = 0x5c;
        assert!(ps_adpcm_frame(&f, &mut hist, &mut out).is_err());
        let b = stream(1, 29, &[0; 16]);
        assert!(decode(&b, &Header::parse(&b).unwrap()).is_err());
        assert!(Header::parse(&b[..0x30]).is_err());
    }
}
