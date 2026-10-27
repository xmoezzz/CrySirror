// SPDX-License-Identifier: MPL-2.0
//! Desktop lifecycle glue. Voice scheduling/buses and devices belong to the engine.
use anyhow::Result;
use crysirror_runtime::Runtime;

pub struct Audio {
    muted: bool,
    scene_audio: crysirror_audio::scene::SceneAudio,
    mixer: crysirror_audio::mixer::Mixer,
    #[cfg(feature = "audio")]
    mixer_output: crysirror_audio::pcm_output::PcmOutput,
    mixer_restored: Option<u64>,
    remainder: u64,
    frame: Option<(u64, crysirror_audio::render_job::MixFrame)>,
    pause_pending: Option<bool>,
}
impl Audio {
    pub fn new(muted: bool) -> Self {
        Self {
            muted,
            scene_audio: Default::default(),
            mixer: crysirror_audio::mixer::Mixer::new(crysirror_audio::mixer::MixerConfig {
                max_voices: crysirror_core::state::Limits::default().voice_count,
                ..Default::default()
            })
            .expect("canonical mixer limits"),
            #[cfg(feature = "audio")]
            mixer_output: Default::default(),
            mixer_restored: None,
            remainder: 0,
            frame: None,
            pause_pending: None,
        }
    }
    #[cfg(all(test, feature = "audio"))]
    pub(crate) fn with_output(output: crysirror_audio::pcm_output::PcmOutput) -> Self {
        let mut audio = Self::new(false);
        audio.mixer_output = output;
        audio
    }
    pub fn device_opening(&self) -> bool {
        #[cfg(feature = "audio")]
        {
            !self.muted
                && self.mixer_output.status().phase
                    == crysirror_audio::pcm_output::OutputPhase::Opening
        }
        #[cfg(not(feature = "audio"))]
        {
            false
        }
    }
    pub fn poll_device(&self) -> Result<()> {
        #[cfg(feature = "audio")]
        if !self.muted {
            self.mixer_output.poll()?;
        }
        Ok(())
    }
    pub fn close_device(&mut self) {
        #[cfg(feature = "audio")]
        {
            self.mixer_output.begin_close();
        }
    }
    pub fn frame_pending(&self) -> bool {
        self.frame.is_some()
    }
    /// Called after runtime control/VM work. The two CPU mixers share one
    /// completion boundary; capacity retries never repeat simulation or input.
    pub fn begin_frame(&mut self, rt: &mut Runtime, delta_ms: u64) -> Result<()> {
        anyhow::ensure!(self.frame.is_none(), "desktop audio frame pending");
        anyhow::ensure!(
            delta_ms <= 2000,
            "desktop worker audio frame exceeds two seconds"
        );
        let pool = rt.assets.io_tasks();
        let live = rt.audio_is_nonblocking();
        self.scene_audio.sync(
            &mut self.mixer,
            &rt.host.scene,
            rt.restored,
            &pool,
            live,
            |client| {
                rt.host
                    .fs
                    .storage(client)
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },
        )?;
        if self.mixer_restored != Some(rt.restored) {
            #[cfg(feature = "audio")]
            self.mixer_output.reset();
            self.mixer_restored = Some(rt.restored);
            self.remainder = 0;
        }
        let numerator = delta_ms
            .checked_mul(48_000)
            .and_then(|n| n.checked_add(self.remainder))
            .ok_or_else(|| anyhow::anyhow!("desktop audio clock overflow"))?;
        let count = (numerator / 1000) as usize;
        self.remainder = numerator % 1000;
        let native = rt.host.channel_audio.prepare_render(delta_ms)?;
        let extra = rt.host.take_pcm_output();
        self.frame = Some((
            rt.restored,
            crysirror_audio::render_job::MixFrame::new(
                rt.assets.cpu_tasks(),
                vec![count, native],
                extra,
            )?,
        ));
        Ok(())
    }
    pub fn poll_frame(&mut self, rt: &mut Runtime) -> Result<bool> {
        let Some((epoch, mut frame)) = self.frame.take() else {
            return Ok(true);
        };
        if epoch != rt.restored {
            frame.cancel(&mut [&mut self.mixer, &mut rt.host.channel_audio.mixer]);
            self.scene_audio.reset(&mut self.mixer);
            return Ok(true);
        }
        match frame.poll(&mut [&mut self.mixer, &mut rt.host.channel_audio.mixer]) {
            Ok(None) => {
                self.frame = Some((epoch, frame));
                Ok(false)
            }
            Ok(Some(pcm)) => {
                if let Some(paused) = self.pause_pending.take() {
                    self.scene_audio.set_paused(&mut self.mixer, paused)?;
                }
                self.sync_mixed(&pcm, !rt.host.scene.voices().is_empty(), epoch)?;
                Ok(true)
            }
            Err(e) => {
                frame.cancel(&mut [&mut self.mixer, &mut rt.host.channel_audio.mixer]);
                Err(e)
            }
        }
    }
    pub fn cancel_frame(&mut self, rt: &mut Runtime) {
        if let Some((_, mut frame)) = self.frame.take() {
            frame.cancel(&mut [&mut self.mixer, &mut rt.host.channel_audio.mixer]);
        }
        self.pause_pending = None;
    }
    pub fn pause(&mut self) -> Result<()> {
        if self.frame_pending() {
            self.pause_pending = Some(true);
        } else {
            self.scene_audio.set_paused(&mut self.mixer, true)?;
        }
        #[cfg(feature = "audio")]
        self.mixer_output.pause();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        if self.frame_pending() {
            self.pause_pending = Some(false);
        } else {
            self.scene_audio.set_paused(&mut self.mixer, false)?;
        }
        #[cfg(feature = "audio")]
        self.mixer_output.resume();
        Ok(())
    }
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
#[cfg(test)]
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
    #[cfg(feature = "audio")]
    #[test]
    fn async_device_failure_is_visible_without_a_subsequent_pcm_submission() {
        use crysirror_audio::pcm_output::{OutputConsumer, OutputSignals, PcmOutput};
        let mut audio = super::Audio::new(false);
        audio.mixer_output = PcmOutput::with_backend(|_: OutputConsumer, _: OutputSignals| {
            anyhow::bail!("authored desktop output failure")
        });
        audio.sync_mixed(&[[0.25; 2]], false, 0).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !audio.mixer_output.watch().is_closed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            audio
                .poll_device()
                .unwrap_err()
                .to_string()
                .contains("authored desktop output failure")
        );
        audio.close_device();
    }
    #[test]
    fn native_and_channel_pcm_add_in_stereo_before_final_clamp() {
        assert_eq!(
            mix_pcm(&[[0.75, -0.5], [0.2, 0.3]], &[[0.75, 0.25]]).unwrap(),
            [[1.5, -0.25], [0.2, 0.3]]
        );
        assert!(mix_pcm(&[[f32::NAN, 0.]], &[]).is_err());
    }
}
