// SPDX-License-Identifier: MPL-2.0
//! Shared channel decoding, sequencing and mixing; no output device.
use crysirror_audio::{
    channels::ChannelAudio,
    mixer::{Bus, BusSettings, Ducking},
};
use crysirror_core::service::{AudioParameters, AudioRequest, EncodedAudio, PcmBus};
use std::sync::Arc;

fn wav(samples: &[[i16; 2]], rate: u32) -> EncodedAudio {
    let size = (samples.len() * 4) as u32;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((size + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 4).to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for frame in samples {
        for sample in frame {
            bytes.extend(sample.to_le_bytes());
        }
    }
    EncodedAudio {
        bytes: Arc::from(bytes),
        frame_limit: None,
    }
}
fn play(channel: u32, source: EncodedAudio, parameters: AudioParameters) -> AudioRequest {
    AudioRequest::Play {
        channel,
        source,
        intro: None,
        parameters,
    }
}
fn constant(channel: u32, value: i16, bus: PcmBus) -> AudioRequest {
    play(
        channel,
        wav(&[[value; 2]; 16], 48_000),
        AudioParameters {
            looping: true,
            bus,
            ..Default::default()
        },
    )
}
fn render(audio: &mut ChannelAudio) -> Vec<[f32; 2]> {
    audio.render(1).unwrap();
    audio.take_output()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn intro_plays_once_then_only_tail_loops_and_declared_length_is_respected() {
    let mut audio = ChannelAudio::default();
    audio
        .submit(
            1,
            AudioRequest::Play {
                channel: 7,
                source: wav(&[[8192; 2], [16384; 2]], 48_000),
                intro: Some(wav(&[[4096; 2]; 2], 48_000)),
                parameters: AudioParameters {
                    looping: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let output = render(&mut audio);
    for (frame, expected) in output
        .iter()
        .zip([0.125, 0.125, 0.25, 0.5, 0.25, 0.5, 0.25, 0.5])
    {
        near(frame[0], expected);
        near(frame[1], expected);
    }
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 7,
                fade_ms: 0.,
            },
        )
        .unwrap();
    let mut limited = wav(&[[8192; 2]; 100], 48_000);
    limited.frame_limit = Some(3);
    audio
        .submit(1, play(7, limited, AudioParameters::default()))
        .unwrap();
    let output = render(&mut audio);
    assert_eq!(&output[..3], &[[0.25; 2]; 3]);
    assert!(output[3..].iter().flatten().all(|sample| *sample == 0.));
}

#[test]
fn pan_gain_and_pause_fades_obey_sample_boundaries() {
    let mut audio = ChannelAudio::default();
    audio
        .submit(
            1,
            play(
                3,
                wav(&[[16384; 2]; 100], 48_000),
                AudioParameters {
                    looping: true,
                    gain: 0.5,
                    decode_gain: 2.,
                    pan: 0.,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    // The original Kira Frame::panned law keeps center unity and boosts a
    // hard-panned channel by sqrt(2), rather than using a linear balance.
    for frame in render(&mut audio) {
        near(frame[0], 0.5 * std::f32::consts::SQRT_2);
        near(frame[1], 0.);
    }
    audio
        .submit(
            1,
            AudioRequest::SetGain {
                channel: 3,
                gain: 0.,
                fade_ms: 1.,
            },
        )
        .unwrap();
    let fade = render(&mut audio);
    assert!(fade[0][0] > fade[47][0]);
    // A 1 ms ramp at 48 kHz samples t=0..47 here; t=48 is the next block.
    near(fade[0][0], 0.5 * std::f32::consts::SQRT_2);
    near(fade[47][0], 0.5 * std::f32::consts::SQRT_2 / 48.);
    assert!(fade.windows(2).all(|p| p[0][0] >= p[1][0]));
    assert_eq!(render(&mut audio), vec![[0.; 2]; 48]);
    audio
        .submit(
            1,
            AudioRequest::SetGain {
                channel: 3,
                gain: 1.,
                fade_ms: 0.,
            },
        )
        .unwrap();
    audio
        .submit(
            1,
            AudioRequest::SetPan {
                channel: 3,
                pan: 1.,
                fade_ms: 0.,
            },
        )
        .unwrap();
    for frame in render(&mut audio) {
        near(frame[0], 0.);
        near(frame[1], std::f32::consts::SQRT_2);
    }
    audio
        .submit(
            1,
            AudioRequest::Pause {
                channel: 3,
                paused: true,
                fade_ms: 1.,
            },
        )
        .unwrap();
    let fade = render(&mut audio);
    assert!(fade[0][1] > fade[47][1]);
    near(fade[47][1], std::f32::consts::SQRT_2 / 48.);
    assert_eq!(render(&mut audio), vec![[0.; 2]; 48]);
    audio
        .submit(
            1,
            AudioRequest::Pause {
                channel: 3,
                paused: false,
                fade_ms: 0.,
            },
        )
        .unwrap();
    for frame in render(&mut audio) {
        near(frame[0], 0.);
        near(frame[1], std::f32::consts::SQRT_2);
    }
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 3,
                fade_ms: 1.,
            },
        )
        .unwrap();
    near(render(&mut audio)[47][1], std::f32::consts::SQRT_2 / 48.);
    assert_eq!(render(&mut audio), vec![[0.; 2]; 48]);
}

#[test]
fn public_buses_and_ducking_control_only_the_owning_clients_channels() {
    let mut audio = ChannelAudio::default();
    audio
        .set_ducking(
            1,
            Ducking {
                gain: 0.,
                attack_ms: 0.,
                release_ms: 0.,
            },
        )
        .unwrap();
    audio.submit(1, constant(1, 8192, PcmBus::Music)).unwrap();
    audio.submit(2, constant(1, 4096, PcmBus::Music)).unwrap();
    audio.submit(1, constant(2, 16384, PcmBus::Voice)).unwrap();
    near(render(&mut audio)[0][0], 0.625);
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 2,
                fade_ms: 0.,
            },
        )
        .unwrap();
    near(render(&mut audio)[0][0], 0.375);
    audio
        .mixer
        .set_bus(
            1,
            Bus::Music,
            BusSettings {
                gain: 1.,
                muted: true,
            },
        )
        .unwrap();
    near(render(&mut audio)[0][0], 0.125);
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 1,
                fade_ms: 0.,
            },
        )
        .unwrap();
    near(render(&mut audio)[0][0], 0.125);
}

#[test]
fn malformed_replacement_preserves_playing_channel_and_restart_false_preserves_cursor() {
    let mut audio = ChannelAudio::default();
    audio.submit(1, constant(1, 8192, PcmBus::Sfx)).unwrap();
    let malformed = EncodedAudio {
        bytes: Arc::from(b"not audio".as_slice()),
        frame_limit: None,
    };
    assert!(audio
        .submit(1, play(1, malformed.clone(), AudioParameters::default()))
        .is_err());
    near(render(&mut audio)[0][0], 0.25);
    assert!(audio
        .submit(
            1,
            play(
                1,
                wav(&[[8192; 2]; 16], 48_000),
                AudioParameters {
                    rate: f32::NAN,
                    ..Default::default()
                }
            )
        )
        .is_err());
    near(render(&mut audio)[0][0], 0.25);
    audio
        .submit(
            1,
            play(
                1,
                malformed.clone(),
                AudioParameters {
                    restart: false,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    near(render(&mut audio)[0][0], 0.25);
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 1,
                fade_ms: 0.,
            },
        )
        .unwrap();
    let mut sequence = vec![[4096; 2]; 48];
    sequence.extend([[8192; 2]; 48]);
    audio
        .submit(
            1,
            play(2, wav(&sequence, 48_000), AudioParameters::default()),
        )
        .unwrap();
    near(render(&mut audio)[0][0], 0.125);
    audio
        .submit(
            1,
            play(
                2,
                malformed,
                AudioParameters {
                    restart: false,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    // The existing source advances into block two instead of restarting.
    near(render(&mut audio)[0][0], 0.25);
    assert!(audio.render(2001).is_err());
    assert!(audio.take_output().is_empty());
}
