// SPDX-License-Identifier: MPL-2.0
//! Adapt canonical channel voices to the shared CPU mixer so native PCM and
//! modern mixer voices can reach one host output device.
use crate::{
    decoder::{StereoFrame, Stream},
    mixer::{Mixer, MixerConfig, VoiceHandle, VoiceSpec},
    AudioDevice, AudioSystem, VoiceStart,
};
use anyhow::Result;
use crysirror_core::state::ObjectId;
use std::collections::BTreeMap;

pub struct MixerDevice {
    mixer: Mixer,
    voices: BTreeMap<ObjectId, VoiceHandle>,
}
impl Default for MixerDevice {
    fn default() -> Self {
        Self {
            mixer: Mixer::new(MixerConfig {
                max_voices: crysirror_core::state::Limits::default().voice_count,
                ..Default::default()
            })
            .expect("valid canonical voice limits"),
            voices: BTreeMap::new(),
        }
    }
}
impl AudioDevice for MixerDevice {
    fn start(&mut self, id: ObjectId, request: VoiceStart) -> Result<()> {
        let extension = request
            .voice
            .asset
            .relative()
            .rsplit_once('.')
            .map(|(_, ext)| ext);
        let source = Stream::from_bytes(request.bytes, extension)?;
        let handle = self.mixer.play(
            id.client.0,
            Box::new(source),
            VoiceSpec {
                gain: f32::from(request.gain_milli) / 1000.,
                looping: request.voice.looping,
                paused: request.paused,
                ..Default::default()
            },
        )?;
        self.voices.insert(id, handle);
        Ok(())
    }
    fn stop(&mut self, id: ObjectId) {
        if let Some(handle) = self.voices.remove(&id) {
            if self.mixer.is_playing(id.client.0, handle) {
                let _ = self.mixer.stop(id.client.0, handle, 0.);
            }
        }
    }
    fn set_gain(&mut self, id: ObjectId, gain: u16) {
        if let Some(&handle) = self.voices.get(&id) {
            if self.mixer.is_playing(id.client.0, handle) {
                let _ = self
                    .mixer
                    .set_gain(id.client.0, handle, f32::from(gain) / 1000.);
            }
        }
    }
    fn pause(&mut self, id: ObjectId) {
        if let Some(&handle) = self.voices.get(&id) {
            if self.mixer.is_playing(id.client.0, handle) {
                let _ = self.mixer.pause(id.client.0, handle, true);
            }
        }
    }
    fn resume(&mut self, id: ObjectId) {
        if let Some(&handle) = self.voices.get(&id) {
            if self.mixer.is_playing(id.client.0, handle) {
                let _ = self.mixer.pause(id.client.0, handle, false);
            }
        }
    }
}
impl AudioSystem<MixerDevice> {
    pub fn render_pcm(&mut self, output: &mut [StereoFrame]) -> Result<()> {
        self.device.mixer.render(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        path::{Mount, VfsPath},
        state::{AudioVoice, ClientId, Command, Limits, SceneState},
    };
    use std::sync::Arc;
    fn wav() -> Arc<[u8]> {
        let mut data = Vec::new();
        data.extend(b"RIFF");
        data.extend(68u32.to_le_bytes());
        data.extend(b"WAVEfmt ");
        data.extend(16u32.to_le_bytes());
        data.extend(1u16.to_le_bytes());
        data.extend(2u16.to_le_bytes());
        data.extend(48000u32.to_le_bytes());
        data.extend(192000u32.to_le_bytes());
        data.extend(4u16.to_le_bytes());
        data.extend(16u16.to_le_bytes());
        data.extend(b"data");
        data.extend(32u32.to_le_bytes());
        for _ in 0..16 {
            data.extend(16384i16.to_le_bytes());
        }
        data.into()
    }
    #[test]
    fn canonical_channel_gain_pause_stop_and_replay_mix_without_device() {
        let client = ClientId(1);
        let mut scene = SceneState::default();
        let play = Command::PlayAudio {
            channel: 7,
            voice: AudioVoice {
                asset: VfsPath::new(Mount::Asset, "tone.wav").unwrap(),
                gain_milli: 500,
                looping: true,
            },
        };
        scene
            .apply_batch(client, &[play.clone()], Limits::default())
            .unwrap();
        let mut audio = AudioSystem::new(MixerDevice::default());
        audio.sync(&scene, 0, |_, _| Ok(wav())).unwrap();
        let mut output = [[0.; 2]; 4];
        audio.render_pcm(&mut output).unwrap();
        assert!(output
            .iter()
            .flatten()
            .all(|sample| (*sample - 0.25).abs() < 1e-6));
        scene
            .apply_batch(
                client,
                &[Command::SetAudioGain {
                    channel: 7,
                    gain_milli: 250,
                }],
                Limits::default(),
            )
            .unwrap();
        audio
            .sync(&scene, 0, |_, _| {
                panic!("gain change must not restart the channel")
            })
            .unwrap();
        audio.render_pcm(&mut output).unwrap();
        assert_eq!(output, [[0.125; 2]; 4]);
        audio.pause();
        audio.render_pcm(&mut output).unwrap();
        assert_eq!(output, [[0.; 2]; 4]);
        audio.resume();
        audio.render_pcm(&mut output).unwrap();
        assert_eq!(output, [[0.125; 2]; 4]);
        scene
            .apply_batch(
                client,
                &[Command::StopAudio { channel: 7 }],
                Limits::default(),
            )
            .unwrap();
        audio
            .sync(&scene, 0, |_, _| panic!("stopped channel must not load"))
            .unwrap();
        audio.render_pcm(&mut output).unwrap();
        assert_eq!(output, [[0.; 2]; 4]);
        scene
            .apply_batch(client, &[play], Limits::default())
            .unwrap();
        audio.sync(&scene, 0, |_, _| Ok(wav())).unwrap();
        audio.render_pcm(&mut output).unwrap();
        assert_eq!(output, [[0.25; 2]; 4]);
    }
}
