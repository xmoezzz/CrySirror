// SPDX-License-Identifier: MPL-2.0
//! All samples are authored here. No audio device, bundled music or fonts.
use crysirror_audio::{
    decoder::{AudioSource, Clip, Stream},
    mixer::*,
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
fn clip(samples: &[f32]) -> Clip {
    Clip::new(8000, samples.iter().map(|s| [*s, *s]).collect()).unwrap()
}
fn constant(value: f32) -> Clip {
    clip(&vec![value; 1024])
}
fn mixer(limit: usize) -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 8000,
        max_voices: limit,
        seed: 123,
    })
    .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}")
}
fn render(m: &mut Mixer, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    m.render(&mut out).unwrap();
    out
}
fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let size = (samples.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for s in samples {
        b.extend(s.to_le_bytes())
    }
    b
}

#[test]
fn pool_steals_oldest_low_priority_and_rejects_stale_cross_client_handles() {
    let mut m = mixer(2);
    let h1 = m
        .play(
            1,
            constant(0.1).source(),
            VoiceSpec {
                priority: 10,
                ..Default::default()
            },
        )
        .unwrap();
    let h2 = m
        .play(
            2,
            constant(0.2).source(),
            VoiceSpec {
                priority: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(m
        .play(
            1,
            constant(0.3).source(),
            VoiceSpec {
                priority: 0,
                ..Default::default()
            }
        )
        .is_err());
    let h3 = m
        .play(
            2,
            constant(0.3).source(),
            VoiceSpec {
                priority: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!m.is_playing(2, h2));
    assert!(m.stop(2, h2, 0.).is_err());
    assert!(m.stop(2, h1, 0.).is_err());
    assert_ne!(h2.generation, h3.generation);
    near(render(&mut m, 1)[0][0], 0.4);
    assert_eq!(m.stats().stolen, 1);
    assert_eq!(m.stats().rejected, 1);
    m.stop_client(2);
    assert!(m.is_playing(1, h1));
    near(render(&mut m, 1)[0][0], 0.1);
    let mut scoped = mixer(4);
    scoped.set_client_limit(1, 1).unwrap();
    let a = scoped
        .play(
            1,
            constant(0.1).source(),
            VoiceSpec {
                priority: 10,
                ..Default::default()
            },
        )
        .unwrap();
    let b = scoped
        .play(
            2,
            constant(0.2).source(),
            VoiceSpec {
                priority: 0,
                ..Default::default()
            },
        )
        .unwrap();
    scoped
        .play(
            1,
            constant(0.3).source(),
            VoiceSpec {
                priority: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!scoped.is_playing(1, a));
    assert!(scoped.is_playing(2, b));
    near(render(&mut scoped, 1)[0][0], 0.5);
}
#[test]
fn buses_and_dialogue_ducking_are_client_scoped_and_restore_music() {
    let mut m = mixer(8);
    m.set_ducking(
        1,
        Ducking {
            gain: 0.25,
            attack_ms: 0.,
            release_ms: 0.,
        },
    )
    .unwrap();
    m.play(
        1,
        constant(0.4).source(),
        VoiceSpec {
            bus: Bus::Music,
            ..Default::default()
        },
    )
    .unwrap();
    m.play(
        2,
        constant(0.2).source(),
        VoiceSpec {
            bus: Bus::Music,
            ..Default::default()
        },
    )
    .unwrap();
    let v = m
        .play(
            1,
            constant(0.1).source(),
            VoiceSpec {
                bus: Bus::Voice,
                ..Default::default()
            },
        )
        .unwrap();
    near(render(&mut m, 1)[0][0], 0.4); // 0.4*0.25 + 0.2 + 0.1
    m.set_bus(
        1,
        Bus::Master,
        BusSettings {
            gain: 0.5,
            muted: false,
        },
    )
    .unwrap();
    near(render(&mut m, 1)[0][0], 0.3);
    m.set_bus(
        1,
        Bus::Voice,
        BusSettings {
            gain: 1.,
            muted: true,
        },
    )
    .unwrap();
    near(render(&mut m, 1)[0][0], 0.4); // no audible dialogue, no duck
    m.stop(1, v, 0.).unwrap();
    m.set_bus(
        1,
        Bus::Music,
        BusSettings {
            gain: 1.,
            muted: true,
        },
    )
    .unwrap();
    near(render(&mut m, 1)[0][0], 0.2);
}
#[test]
fn exact_fades_crossfade_loop_points_and_pause() {
    let mut m = mixer(4);
    let old = m
        .play(
            1,
            constant(0.8).source(),
            VoiceSpec {
                bus: Bus::Music,
                ..Default::default()
            },
        )
        .unwrap();
    let new = m
        .play(
            1,
            constant(0.4).source(),
            VoiceSpec {
                bus: Bus::Music,
                fade_in_ms: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
    m.stop(1, old, 0.5).unwrap();
    let a = render(&mut m, 5);
    for (i, want) in [0.8, 0.7, 0.6, 0.5, 0.4].iter().enumerate() {
        near(a[i][0], *want)
    }
    assert!(!m.is_playing(1, old));
    m.pause(1, new, true).unwrap();
    let p = m.position(1, new).unwrap();
    near(render(&mut m, 3)[0][0], 0.);
    assert_eq!(p, m.position(1, new).unwrap());
    m.clear();
    m.play(
        1,
        clip(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]).source(),
        VoiceSpec {
            looping: true,
            loop_points: Some(LoopPoints { start: 2, end: 5 }),
            ..Default::default()
        },
    )
    .unwrap();
    let a = render(&mut m, 9);
    for (i, want) in [0.1, 0.2, 0.3, 0.4, 0.5, 0.3, 0.4, 0.5, 0.3]
        .iter()
        .enumerate()
    {
        near(a[i][0], *want)
    }
}
#[test]
fn rate_pitch_and_backward_seek_change_actual_samples() {
    let mut m = mixer(2);
    let h = m
        .play(
            1,
            clip(&[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]).source(),
            VoiceSpec {
                rate: 2.,
                ..Default::default()
            },
        )
        .unwrap();
    let a = render(&mut m, 3);
    near(a[0][0], 0.);
    near(a[1][0], 0.2);
    near(a[2][0], 0.4);
    m.seek(1, h, 0).unwrap();
    m.set_rate(1, h, 1., 12.).unwrap();
    let a = render(&mut m, 3);
    near(a[1][0], 0.2);
    near(a[2][0], 0.4);
    m.seek(1, h, 0).unwrap();
    m.set_rate(1, h, 0.5, 0.).unwrap();
    let a = render(&mut m, 3);
    near(a[1][0], 0.05);
    near(a[2][0], 0.1);
    assert!(m.set_rate(1, h, 0., 0.).is_err());
}
#[test]
fn spatial_pan_distance_listener_orientation_and_occlusion_lowpass_reach_samples() {
    let mut m = mixer(4);
    let h = m
        .play(
            1,
            constant(1.).source(),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [1., 0., 0.],
                    min_distance: 1.,
                    max_distance: 11.,
                    attenuation: Attenuation::Linear,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let a = render(&mut m, 1);
    near(a[0][0], 0.);
    near(a[0][1], 1.);
    m.set_listener(
        1,
        Listener {
            forward: [0., 0., 1.],
            ..Default::default()
        },
    )
    .unwrap();
    let a = render(&mut m, 1);
    near(a[0][0], 1.);
    near(a[0][1], 0.);
    m.set_listener(1, Listener::default()).unwrap();
    m.set_spatial(
        1,
        h,
        Some(Spatial {
            position: [6., 0., 0.],
            min_distance: 1.,
            max_distance: 11.,
            attenuation: Attenuation::Linear,
            ..Default::default()
        }),
    )
    .unwrap();
    near(render(&mut m, 1)[0][1], 0.5);
    m.set_spatial(
        1,
        h,
        Some(Spatial {
            position: [11., 0., 0.],
            min_distance: 1.,
            max_distance: 11.,
            ..Default::default()
        }),
    )
    .unwrap();
    near(render(&mut m, 1)[0][1], 0.);
    m.clear();
    let wave = clip(
        &(0..128)
            .map(|i| if i % 2 == 0 { 1. } else { -1. })
            .collect::<Vec<_>>(),
    );
    m.play(
        1,
        wave.source(),
        VoiceSpec {
            spatial: Some(Spatial {
                position: [1., 0., 0.],
                occlusion: 1.,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let a = render(&mut m, 64);
    let energy = a[16..].iter().map(|s| s[1] * s[1]).sum::<f32>() / 48.;
    assert!(
        energy < 0.015,
        "high frequencies should be low-passed: {energy}"
    );
}
#[test]
fn random_container_is_reproducible_varied_and_avoids_immediate_repeats() {
    let container = RandomContainer {
        variants: vec![
            Variant {
                clip: constant(0.2),
                weight: 1,
            },
            Variant {
                clip: constant(0.6),
                weight: 3,
            },
            Variant {
                clip: constant(0.8),
                weight: 1,
            },
        ],
        gain: [0.8, 1.],
        pitch_semitones: [-2., 2.],
        avoid_repeat: true,
    };
    let mut a = mixer(1);
    let mut b = mixer(1);
    let (mut previous, mut indices) = (None, Vec::new());
    for _ in 0..32 {
        let (_, sa) = a
            .play_random(1, "steps", &container, VoiceSpec::default())
            .unwrap();
        let (_, sb) = b
            .play_random(1, "steps", &container, VoiceSpec::default())
            .unwrap();
        assert_eq!(sa.index, sb.index);
        assert_eq!(sa.gain, sb.gain);
        assert_eq!(sa.pitch_semitones, sb.pitch_semitones);
        assert_ne!(previous, Some(sa.index));
        assert!((0.8..=1.).contains(&sa.gain) && (-2.0..=2.).contains(&sa.pitch_semitones));
        indices.push(sa.index);
        previous = Some(sa.index);
        assert_eq!(render(&mut a, 1), render(&mut b, 1));
    }
    assert!(
        indices.contains(&0)
            && indices.contains(&1)
            && indices.contains(&2)
    );
}
#[test]
fn streaming_decoder_reads_bounded_prefix_and_seeks_exact_pcm_frames() {
    struct Counted {
        data: Cursor<Vec<u8>>,
        read: Arc<AtomicUsize>,
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = self.data.read(b)?;
            self.read.fetch_add(n, Ordering::Relaxed);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.data.seek(p)
        }
    }
    impl symphonia::core::io::MediaSource for Counted {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.data.get_ref().len() as u64)
        }
    }
    let data = wav(
        &(0..800000).map(|i| (i % 16000) as i16).collect::<Vec<_>>(),
        8000,
    );
    let total = data.len();
    let count = Arc::new(AtomicUsize::new(0));
    let mut stream = Stream::open(
        Box::new(Counted {
            data: Cursor::new(data),
            read: count.clone(),
        }),
        Some("wav"),
    )
    .unwrap();
    let mut out = [[0.; 2]; 32];
    assert_eq!(stream.read(&mut out).unwrap(), 32);
    near(out[7][0], 7. / 32768.);
    assert!(count.load(Ordering::Relaxed) < total / 4);
    assert!(stream.buffered_frames() <= 65536);
    stream.seek(12345).unwrap();
    stream.read(&mut out).unwrap();
    near(out[0][0], 12345. / 32768.);
    stream.seek(2).unwrap();
    stream.read(&mut out).unwrap();
    near(out[0][0], 2. / 32768.);
    let mut m = mixer(2);
    let h = m
        .play(
            1,
            Box::new(stream),
            VoiceSpec {
                looping: true,
                loop_points: Some(LoopPoints { start: 4, end: 7 }),
                ..Default::default()
            },
        )
        .unwrap();
    m.seek(1, h, 5).unwrap();
    let a = render(&mut m, 5);
    for (i, n) in [5, 6, 4, 5, 6].iter().enumerate() {
        near(a[i][0], *n as f32 / 32768.)
    }
}
#[test]
fn corrupt_unknown_and_invalid_controls_fail_without_devices() {
    assert!(Stream::from_bytes(Arc::from(b"RIFF-placeholder".as_slice()), Some("wav")).is_err());
    let mut m = mixer(1);
    assert!(m
        .set_bus(
            1,
            Bus::Sfx,
            BusSettings {
                gain: f32::NAN,
                muted: false
            }
        )
        .is_err());
    assert!(m
        .play(
            1,
            constant(0.1).source(),
            VoiceSpec {
                looping: true,
                loop_points: Some(LoopPoints {
                    start: 0,
                    end: 2000
                }),
                ..Default::default()
            }
        )
        .is_err());
    assert!(m
        .play(
            1,
            constant(0.1).source(),
            VoiceSpec {
                spatial: Some(Spatial {
                    attenuation: Attenuation::Custom {
                        points: vec![[0., 1.], [0.5, 0.2], [1., 0.4]]
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(m.stats().active, 0);
}

#[test]
fn compressed_music_streams_decode_and_seek_without_whole_song_pcm() {
    for (extension, bytes) in [
        ("flac", include_bytes!("fixtures/tone.flac").as_slice()),
        ("mp3", include_bytes!("fixtures/tone.mp3").as_slice()),
        ("ogg", include_bytes!("fixtures/tone.ogg").as_slice()),
    ] {
        let mut stream = Stream::from_bytes(Arc::from(bytes), Some(extension)).unwrap();
        assert_eq!(stream.sample_rate(), 48000);
        let mut block = vec![[0.; 2]; 2048];
        let n = stream.read(&mut block).unwrap();
        assert_eq!(n, 2048);
        let rms = (block[256..].iter().map(|p| p[0] * p[0]).sum::<f32>() / (n - 256) as f32).sqrt();
        assert!((0.03..0.12).contains(&rms), "{extension} RMS {rms}");
        assert!(stream.buffered_frames() <= 65536);
        stream.seek(0).unwrap();
        let mut repeated = vec![[0.; 2]; 2048];
        stream.read(&mut repeated).unwrap();
        let delta = block
            .iter()
            .zip(&repeated)
            .map(|(a, b)| (a[0] - b[0]).abs())
            .fold(0., f32::max);
        assert!(delta < 0.005, "{extension} rewind mismatch {delta}");
        stream.seek(0).unwrap();
        let mut reference = vec![[0.; 2]; 25000];
        assert_eq!(stream.read(&mut reference).unwrap(), 25000);
        stream.seek(24000).unwrap();
        let n = stream.read(&mut block[..100]).unwrap();
        assert_eq!(n, 100);
        let error = block[..100]
            .iter()
            .zip(&reference[24000..24100])
            .map(|(a, b)| (a[0] - b[0]).abs())
            .fold(0., f32::max);
        assert!(error < 0.005, "{extension} seek alignment {error}");
        let expected = stream.frame_count().unwrap();
        stream.seek(0).unwrap();
        let mut total = 0;
        loop {
            let n = stream.read(&mut block).unwrap();
            if n == 0 {
                break;
            }
            total += n as u64;
        }
        assert_eq!(
            total, expected,
            "{extension} duration/encoder delay must agree"
        );
    }
}
