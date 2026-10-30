// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Worker-side PCM resource preparation, reusable by media and ordinary clients.
use anyhow::{Result, ensure};
use crysirror_core::service::{MAX_PCM_FRAMES, PcmAudio};
/// Incremental interleaved i16 conversion. Matches the shared decoder's current
/// mono duplication / first-two-channel projection; not a surround downmixer.
pub struct PcmBuilder {
    rate: u32,
    channels: usize,
    limit: usize,
    frames: Vec<[f32; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn float_append_preserves_samples_and_failed_blocks_are_atomic() {
        let mut builder=PcmBuilder::new(44100,1,3).unwrap();
        builder.append_f32(&[0.123456,-0.4]).unwrap();
        assert!(builder.append_f32(&[f32::NAN]).is_err());
        assert!(builder.append_f32(&[0.1,0.2]).is_err());
        builder.append_f32(&[0.5]).unwrap();
        let pcm=builder.finish().unwrap();
        assert_eq!(pcm.frames(),&[[0.123456,0.123456],[-0.4,-0.4],[0.5,0.5]]);
    }
}
impl PcmBuilder {
    pub fn new(rate: u32, channels: u16, max_frames: usize) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&rate) && (1..=8).contains(&channels),
            "PCM format"
        );
        ensure!(
            max_frames > 0 && max_frames <= MAX_PCM_FRAMES,
            "PCM builder frame limit"
        );
        Ok(Self {
            rate,
            channels: channels as usize,
            limit: max_frames,
            frames: vec![],
        })
    }
    pub fn append_i16(&mut self, samples: &[i16]) -> Result<()> {
        ensure!(
            samples.len() % self.channels == 0,
            "PCM partial interleaved frame"
        );
        ensure!(
            samples.len() / self.channels <= self.limit - self.frames.len(),
            "PCM builder frame budget"
        );
        self.frames
            .extend(samples.chunks_exact(self.channels).map(|r| {
                [
                    r[0] as f32 / 32768.,
                    r[usize::from(self.channels > 1)] as f32 / 32768.,
                ]
            }));
        Ok(())
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    /// Append decoded float PCM without an intermediate integer quantization.
    pub fn append_f32(&mut self, samples: &[f32]) -> Result<()> {
        ensure!(
            samples.len() % self.channels == 0,
            "PCM partial interleaved frame"
        );
        ensure!(
            samples.len() / self.channels <= self.limit - self.frames.len(),
            "PCM builder frame budget"
        );
        ensure!(
            samples
                .iter()
                .all(|x| x.is_finite() && (-16.0..=16.0).contains(x)),
            "nonfinite or excessive PCM"
        );
        self.frames.extend(
            samples
                .chunks_exact(self.channels)
                .map(|r| [r[0], r[usize::from(self.channels > 1)]]),
        );
        Ok(())
    }
    pub fn byte_len(&self) -> usize {
        self.frames.len() * 8
    }
    /// Validation and Arc conversion belong on a worker, not a device callback.
    pub fn finish(self) -> Result<PcmAudio> {
        PcmAudio::new(self.rate, self.frames).map_err(|e| anyhow::anyhow!("{e}"))
    }
}
