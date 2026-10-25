// SPDX-License-Identifier: MPL-2.0
//! Desktop lifecycle glue. Voice scheduling/buses and devices belong to the engine.
use anyhow::Result;
use crysirror_runtime::Runtime;

pub struct Audio {
    muted: bool,
    #[cfg(feature = "audio")]
    output: crysirror_audio::AudioSystem<crysirror_audio::legacy_pcm::MixerDevice>,
    #[cfg(feature = "audio")]
    mixer_output: crysirror_audio::pcm_output::PcmOutput,
    #[cfg(feature = "audio")]
    mixer_restored: Option<u64>,
    #[cfg(feature = "audio")]
    last_ms: Option<u64>,
    #[cfg(feature = "audio")]
    remainder: u64,
}
impl Audio {
    pub fn new(muted: bool) -> Self {
        Self {
            muted,
            #[cfg(feature = "audio")]
            output: crysirror_audio::AudioSystem::new(Default::default()),
            #[cfg(feature = "audio")]
            mixer_output: Default::default(),
            #[cfg(feature = "audio")]
            mixer_restored: None,
            #[cfg(feature = "audio")]
            last_ms: None,
            #[cfg(feature = "audio")]
            remainder: 0,
        }
    }
    pub fn sync(&mut self, rt: &mut Runtime) -> Result<()> {
        // Drain even muted sessions: native producers must never accumulate an
        // unconsumed device queue while --no-audio is in use.
        let native = rt.host.take_pcm_output();
        if self.muted {
            return Ok(());
        }
        #[cfg(feature = "audio")]
        {
            self.output.sync(&rt.host.scene, rt.restored, |id, voice| {
                rt.assets.read_for(id.client, voice.asset.relative())
            })?;
            if self.mixer_restored != Some(rt.restored) {
                self.mixer_output.reset();
                self.mixer_restored = Some(rt.restored);
                self.last_ms = None;
                self.remainder = 0;
            }
            let now = rt.now_ms();
            let delta = now.saturating_sub(self.last_ms.replace(now).unwrap_or(now));
            let numerator = delta
                .checked_mul(48_000)
                .and_then(|n| n.checked_add(self.remainder))
                .ok_or_else(|| anyhow::anyhow!("desktop audio clock overflow"))?;
            anyhow::ensure!(
                numerator / 1000 <= 480_000,
                "desktop audio frame exceeds ten seconds"
            );
            let mut channels = vec![[0.; 2]; (numerator / 1000) as usize];
            self.remainder = numerator % 1000;
            self.output.render_pcm(&mut channels)?;
            self.mixer_output.submit(&mix_pcm(&channels, &native)?)
        }
        #[cfg(not(feature = "audio"))]
        {
            anyhow::ensure!(
                rt.host.scene.voices().is_empty()
                    && !native.iter().flatten().any(|sample| *sample != 0.),
                "audio feature disabled: rebuild with audio or explicitly pass --no-audio"
            );
            Ok(())
        }
    }
    pub fn pause(&mut self) {
        #[cfg(feature = "audio")]
        {
            self.output.pause();
            self.mixer_output.pause();
        }
    }
    pub fn resume(&mut self) {
        #[cfg(feature = "audio")]
        {
            self.output.resume();
            self.mixer_output.resume();
        }
    }
    #[cfg(feature = "scripting")]
    pub fn sync_mixed(
        &mut self,
        frames: &[[f32; 2]],
        legacy_voices_present: bool,
        restored: u64,
    ) -> Result<()> {
        if self.muted {
            return Ok(());
        }
        #[cfg(feature = "audio")]
        {
            let _ = legacy_voices_present;
            if self.mixer_restored != Some(restored) {
                self.mixer_output.reset();
                self.mixer_restored = Some(restored);
            }
            self.mixer_output.submit(frames)
        }
        #[cfg(not(feature = "audio"))]
        {
            let _ = restored;
            anyhow::ensure!(
                !legacy_voices_present && !frames.iter().flatten().any(|v| *v != 0.),
                "audio feature disabled: rebuild with audio or explicitly pass --no-audio"
            );
            Ok(())
        }
    }
}

/// Combine CPU producers before the one output device applies its final clamp.
#[cfg(any(feature = "audio", test))]
fn mix_pcm(first: &[[f32; 2]], second: &[[f32; 2]]) -> Result<Vec<[f32; 2]>> {
    anyhow::ensure!(
        first.len().max(second.len()) <= 480_000,
        "mixed audio frame exceeds ten seconds"
    );
    let mut frames = vec![[0.; 2]; first.len().max(second.len())];
    for source in [first, second] {
        for (target, frame) in frames.iter_mut().zip(source) {
            for channel in 0..2 {
                target[channel] += frame[channel];
                anyhow::ensure!(target[channel].is_finite(), "non-finite mixed audio sample");
            }
        }
    }
    Ok(frames)
}
#[cfg(test)]
mod tests {
    use super::mix_pcm;
    #[test]
    fn native_and_channel_pcm_add_in_stereo_before_final_clamp() {
        assert_eq!(
            mix_pcm(&[[0.75, -0.5], [0.2, 0.3]], &[[0.75, 0.25]]).unwrap(),
            [[1.5, -0.25], [0.2, 0.3]]
        );
        assert!(mix_pcm(&[[f32::NAN, 0.]], &[]).is_err());
    }
}
