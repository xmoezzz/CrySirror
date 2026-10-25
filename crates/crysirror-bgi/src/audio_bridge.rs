// SPDX-License-Identifier: MPL-2.0
//! Native container/channel policy -> shared host decoder/mixer commands.
use anyhow::{ensure, Result};
use crysirror_core::{
    service::{AudioParameters, AudioRequest, EncodedAudio, HostServices, PcmBus},
    state::ClientId,
};
use ethornell_app::host_runtime::{AudioAsset, AudioCommand};
use std::sync::Arc;

fn encoded(bytes: &[u8]) -> EncodedAudio {
    let limit = ethornell_audio::declared_frame_count(bytes).map(|n| n as u64);
    let payload = ethornell_audio::unwrap_buriko_wave_ogg(bytes).unwrap_or(bytes);
    EncodedAudio {
        bytes: Arc::from(payload),
        frame_limit: limit,
    }
}
fn finite(values: &[f64]) -> Result<()> {
    ensure!(
        values.iter().all(|v| v.is_finite()),
        "non-finite native audio parameters"
    );
    Ok(())
}
fn bus(asset: &AudioAsset) -> PcmBus {
    match asset
        .archive
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "bgm.arc" => PcmBus::Music,
        "voice.arc" => PcmBus::Voice,
        _ => PcmBus::Sfx,
    }
}
pub fn submit(client: ClientId, command: AudioCommand, host: &mut dyn HostServices) -> Result<()> {
    let request = match command {
        AudioCommand::Play {
            asset,
            loop_asset,
            channel,
            looped,
            volume,
            decode_gain,
            playback_rate,
            panning,
            fade_ms,
            restart,
        } => {
            finite(&[volume, decode_gain, playback_rate, panning])?;
            let (source, intro, loop_start) = if let Some(tail) = &loop_asset {
                (encoded(&tail.bytes), Some(encoded(&asset.bytes)), 0)
            } else {
                (
                    encoded(&asset.bytes),
                    None,
                    ethornell_audio::buriko_wave_loop_start(&asset.bytes),
                )
            };
            AudioRequest::Play {
                channel: channel as u32,
                source,
                intro,
                parameters: AudioParameters {
                    gain: volume.clamp(0., 1.) as f32,
                    decode_gain: decode_gain as f32,
                    rate: playback_rate as f32,
                    pan: panning.clamp(0., 1.) as f32,
                    looping: looped,
                    loop_start,
                    start_ms: 0,
                    fade_ms: fade_ms as f32,
                    restart,
                    bus: bus(&asset),
                },
            }
        }
        AudioCommand::PlayStream {
            channel,
            bytes,
            volume,
            start_ms,
        } => {
            finite(&[volume])?;
            AudioRequest::Play {
                channel: channel as u32,
                source: encoded(&bytes.0),
                intro: None,
                parameters: AudioParameters {
                    gain: volume.clamp(0., 1.) as f32,
                    start_ms,
                    ..Default::default()
                },
            }
        }
        AudioCommand::Pause {
            channel,
            paused,
            fade_ms,
        } => AudioRequest::Pause {
            channel: channel as u32,
            paused,
            fade_ms: fade_ms as f32,
        },
        AudioCommand::Stop { channel, fade_ms } => AudioRequest::Stop {
            channel: channel as u32,
            fade_ms: fade_ms as f32,
        },
        AudioCommand::SetVolume {
            channel,
            volume,
            fade_ms,
        } => {
            finite(&[volume])?;
            AudioRequest::SetGain {
                channel: channel as u32,
                gain: volume.clamp(0., 1.) as f32,
                fade_ms: fade_ms as f32,
            }
        }
        AudioCommand::SetPanning {
            channel,
            panning,
            fade_ms,
        } => {
            finite(&[panning])?;
            AudioRequest::SetPan {
                channel: channel as u32,
                pan: panning.clamp(0., 1.) as f32,
                fade_ms: fade_ms as f32,
            }
        }
    };
    host.submit_audio(client, request)
        .map_err(|e| anyhow::anyhow!("BGI shared audio: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_audio::mixer::{Bus, BusSettings};
    use crysirror_core::service::InputFrame;
    use crysirror_runtime::{AdapterRegistry, Project, Runtime};
    fn wav(samples: &[[i16; 2]]) -> Vec<u8> {
        let size = (samples.len() * 4) as u32;
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend((size + 36).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(48000u32.to_le_bytes());
        out.extend(192000u32.to_le_bytes());
        out.extend(4u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(size.to_le_bytes());
        for frame in samples {
            for sample in frame {
                out.extend(sample.to_le_bytes());
            }
        }
        out
    }
    fn runtime(root: &std::path::Path) -> Runtime {
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let project:Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"BGI audio bridge","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[
            {"name":"one","game":"one","adapter":"crysirror-script-v1","program":[{"op":"halt"}]},
            {"name":"two","game":"two","adapter":"crysirror-script-v1","program":[{"op":"halt"}]}]})).unwrap();
        std::fs::write(
            root.join("project.json"),
            serde_json::to_vec(&project).unwrap(),
        )
        .unwrap();
        Runtime::with_registry(
            project,
            &root.join("project.json"),
            &root.join("state"),
            AdapterRegistry::default(),
        )
        .unwrap()
    }
    fn play(archive: &str) -> AudioCommand {
        AudioCommand::Play {
            asset: AudioAsset {
                archive: archive.into(),
                file: "synthetic.wav".into(),
                bytes: wav(&[[16384; 2]; 96]),
            },
            loop_asset: None,
            channel: 7,
            looped: true,
            volume: 0.5,
            decode_gain: 1.,
            playback_rate: 1.,
            panning: 0.5,
            fade_ms: 0,
            restart: true,
        }
    }
    fn render(runtime: &mut Runtime) -> Vec<[f32; 2]> {
        runtime.tick(1, InputFrame::default()).unwrap();
        let out = runtime.host.take_pcm_output();
        assert_eq!(out.len(), 48);
        out
    }
    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }
    #[test]
    fn native_commands_decode_and_mix_in_host_with_public_bus_and_client_scope() {
        let root = tempfile::tempdir().unwrap();
        let mut rt = runtime(root.path());
        submit(ClientId(1), play("BGM.ARC"), &mut rt.host).unwrap();
        near(render(&mut rt)[0][0], 0.25);
        rt.host
            .channel_audio
            .mixer
            .set_bus(
                1,
                Bus::Music,
                BusSettings {
                    gain: 0.4,
                    muted: false,
                },
            )
            .unwrap();
        near(render(&mut rt)[0][0], 0.1);
        submit(ClientId(2), play("se.arc"), &mut rt.host).unwrap();
        near(render(&mut rt)[0][0], 0.35);
        assert!(submit(ClientId(99), play("se.arc"), &mut rt.host).is_err());
        submit(
            ClientId(1),
            AudioCommand::Stop {
                channel: 7,
                fade_ms: 0,
            },
            &mut rt.host,
        )
        .unwrap();
        near(render(&mut rt)[0][0], 0.25);
        assert!(submit(
            ClientId(2),
            AudioCommand::SetVolume {
                channel: 7,
                volume: f64::NAN,
                fade_ms: 0
            },
            &mut rt.host
        )
        .is_err());
        near(render(&mut rt)[0][0], 0.25);
        submit(
            ClientId(2),
            AudioCommand::SetVolume {
                channel: 7,
                volume: 0.25,
                fade_ms: 0,
            },
            &mut rt.host,
        )
        .unwrap();
        submit(
            ClientId(2),
            AudioCommand::SetPanning {
                channel: 7,
                panning: 1.,
                fade_ms: 0,
            },
            &mut rt.host,
        )
        .unwrap();
        for frame in render(&mut rt) {
            near(frame[0], 0.);
            near(frame[1], 0.125 * std::f32::consts::SQRT_2);
        }
        submit(
            ClientId(2),
            AudioCommand::Pause {
                channel: 7,
                paused: true,
                fade_ms: 0,
            },
            &mut rt.host,
        )
        .unwrap();
        assert_eq!(render(&mut rt), vec![[0.; 2]; 48]);
        submit(
            ClientId(2),
            AudioCommand::Pause {
                channel: 7,
                paused: false,
                fade_ms: 0,
            },
            &mut rt.host,
        )
        .unwrap();
        for frame in render(&mut rt) {
            near(frame[0], 0.);
            near(frame[1], 0.125 * std::f32::consts::SQRT_2);
        }
    }
    #[test]
    fn native_movie_stream_start_offset_reaches_shared_decoder() {
        let root = tempfile::tempdir().unwrap();
        let mut rt = runtime(root.path());
        let mut samples = vec![[8192; 2]; 48];
        samples.extend([[16384; 2]; 48]);
        let bytes = ethornell_app::host_runtime::StreamBytes(Arc::from(wav(&samples)));
        submit(
            ClientId(1),
            AudioCommand::PlayStream {
                channel: 3,
                bytes,
                volume: 1.,
                start_ms: 1,
            },
            &mut rt.host,
        )
        .unwrap();
        assert_eq!(render(&mut rt), vec![[0.5; 2]; 48]);
        assert_eq!(render(&mut rt), vec![[0.; 2]; 48]);
    }
}
