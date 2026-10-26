// SPDX-License-Identifier: MPL-2.0
//! Shared RIFF/WAVE serialization for native resources. Container layout is
//! usable by archive/file plugins and tools, independently of sound output.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveLoop {
    /// Inclusive first sample frame.
    pub start: u32,
    /// Native final sample frame.
    pub end: u32,
    /// Zero means repeat indefinitely.
    pub play_count: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WaveSpec {
    /// Native WAVE format tag; PCM is 1.
    pub format: u16,
    /// Interleaved channel count, 1..=32.
    pub channels: u16,
    /// Sample frames per second.
    pub sample_rate: u32,
    /// Native sample depth, retained without conversion.
    pub bits_per_sample: u16,
    /// Native WAVEFORMATEX extension bytes.
    pub extra: Vec<u8>,
    /// Sample loop metadata serialized in a smpl chunk.
    pub loops: Vec<WaveLoop>,
    /// Zero derives channel_count * bits / 8. Explicit compressed formats may
    /// supply their native packet alignment and byte rate.
    pub block_align: u16,
    /// Zero derives sample_rate * block_align.
    pub byte_rate: u32,
}
impl Default for WaveSpec {
    fn default() -> Self {
        Self {
            format: 1,
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            extra: vec![],
            loops: vec![],
            block_align: 0,
            byte_rate: 0,
        }
    }
}
impl WaveSpec {
    pub fn encoded_len(&self, sample_bytes: usize) -> Result<usize> {
        ensure!(
            (1..=32).contains(&self.channels)
                && self.sample_rate > 0
                && self.bits_per_sample <= 64
                && self.bits_per_sample > 0,
            "WAVE stream parameters"
        );
        ensure!(
            self.extra.len() <= u16::MAX as usize && self.loops.len() <= 4096,
            "WAVE extra/loop limits"
        );
        let size = 46usize
            .checked_add(self.extra.len())
            .and_then(|n| n.checked_add(sample_bytes))
            .and_then(|n| {
                n.checked_add(if self.loops.is_empty() {
                    0
                } else {
                    44 + self.loops.len() * 24
                })
            })
            .context("WAVE size overflow")?;
        ensure!(
            size - 8 <= u32::MAX as usize && sample_bytes <= u32::MAX as usize,
            "WAVE RIFF size limit"
        );
        Ok(size)
    }
    /// Retains native 18-byte WAVEFORMATEX headers and loop units. No implicit
    /// resampling/channel conversion or codec rewrite is performed here.
    pub fn encode(&self, samples: &[u8]) -> Result<Vec<u8>> {
        let size = self.encoded_len(samples.len())?;
        let mut out = Vec::with_capacity(size);
        let alignment = if self.block_align != 0 {
            self.block_align
        } else {
            self.channels
                .checked_mul(self.bits_per_sample)
                .context("WAVE block alignment overflow")?
                / 8
        };
        let byte_rate = if self.byte_rate != 0 {
            self.byte_rate
        } else {
            self.sample_rate
                .checked_mul(u32::from(alignment))
                .context("WAVE byte rate overflow")?
        };
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((size - 8) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&(18u32 + self.extra.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.format.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&alignment.to_le_bytes());
        out.extend_from_slice(&self.bits_per_sample.to_le_bytes());
        out.extend_from_slice(&(self.extra.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.extra);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        out.extend_from_slice(samples);
        if !self.loops.is_empty() {
            out.extend_from_slice(b"smpl");
            out.extend_from_slice(&(36u32 + self.loops.len() as u32 * 24).to_le_bytes());
            for n in [0u32, 0, 0, 0, 0, 0, 0, self.loops.len() as u32, 0] {
                out.extend_from_slice(&n.to_le_bytes());
            }
            for (i, l) in self.loops.iter().enumerate() {
                for n in [i as u32, 0, l.start, l.end, 0, l.play_count] {
                    out.extend_from_slice(&n.to_le_bytes());
                }
            }
        }
        ensure!(out.len() == size, "WAVE serializer size invariant");
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wave_retains_format_and_loops_without_output_device() {
        let spec = WaveSpec {
            channels: 2,
            sample_rate: 22050,
            loops: vec![WaveLoop {
                start: 3,
                end: 8,
                play_count: 2,
            }],
            ..Default::default()
        };
        let bytes = spec.encode(&[1, 2, 3, 4]).unwrap();
        assert_eq!(bytes.len(), spec.encoded_len(4).unwrap());
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 18);
        assert_eq!(&bytes[50..54], b"smpl");
        assert_eq!(u32::from_le_bytes(bytes[102..106].try_into().unwrap()), 3);
    }
}
