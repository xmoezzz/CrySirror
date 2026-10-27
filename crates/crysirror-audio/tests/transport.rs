// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::{
    channels::ChannelAudio,
    decoder,
    mixer::{Mixer, MixerConfig, VoiceSpec},
};
use crysirror_core::service::{AudioParameters, AudioRequest, PcmAudio};
fn render(m: &mut Mixer, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    m.render(&mut out).unwrap();
    out
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn exact_start_paused_publication_ranges_and_replacement_crossfade_share_channel_resources() {
    let mut a = ChannelAudio::default();
    let pcm = PcmAudio::new(48000, (0..1000).map(|i| [i as f32 / 1000.; 2]).collect()).unwrap();
    a.submit(
        1,
        AudioRequest::PlayPcm {
            channel: 3,
            source: pcm,
            parameters: AudioParameters {
                start_ms: 100,
                start_frame: Some(7),
                paused: true,
                ..Default::default()
            },
        },
    )
    .unwrap();
    let info = a.playback_info(1, 3).unwrap().unwrap();
    assert_eq!(info.position_frames, 7);
    assert!(info.paused);
    assert_eq!(info.source_rate, 48000);
    assert!(a.playback_info(2, 3).unwrap().is_none());
    a.render(2).unwrap();
    assert!(a.take_output().iter().flatten().all(|x| *x == 0.));
    a.submit(
        1,
        AudioRequest::Resume {
            channel: 3,
            delay_ms: 1.,
            fade_ms: 0.,
        },
    )
    .unwrap();
    a.render(1).unwrap();
    assert!(a.take_output().iter().flatten().all(|x| *x == 0.));
    assert!(!a.playback_info(1, 3).unwrap().unwrap().paused);
    a.render(1).unwrap();
    near(a.take_output()[0][0], 0.007);
    let second = PcmAudio::new(48000, vec![[0.25; 2]; 1000]).unwrap();
    a.submit(
        1,
        AudioRequest::PlayPcm {
            channel: 3,
            source: second,
            parameters: AudioParameters {
                replace_fade_ms: 2.,
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(a.mixer.stats().active, 2);
    a.render(2).unwrap();
    assert_eq!(a.mixer.stats().active, 1);
    assert!(a.take_output()[0][0] > 0.3);
    a.submit(
        1,
        AudioRequest::Stop {
            channel: 3,
            fade_ms: 0.,
        },
    )
    .unwrap();
    a.submit(
        1,
        AudioRequest::SetGain {
            channel: 3,
            gain: 0.2,
            fade_ms: 0.,
        },
    )
    .unwrap();
    assert!(a.playback_info(1, 3).unwrap().is_none());
    a.submit(
        1,
        AudioRequest::PlayPcm {
            channel: 3,
            source: PcmAudio::new(48000, vec![[0.5; 2]; 100]).unwrap(),
            parameters: AudioParameters {
                clamp_range: true,
                start_frame: Some(500),
                loop_start: 40,
                looping: true,
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(a.playback_info(1, 3).unwrap().unwrap().position_frames, 40);
}
#[test]
fn scheduled_delay_fade_pause_stop_and_replacement_have_sample_oracles() {
    let mut m = Mixer::new(MixerConfig {
        sample_rate: 8000,
        ..Default::default()
    })
    .unwrap();
    let h = m
        .play(
            1,
            decoder::open_pcm(PcmAudio::new(8000, vec![[1.; 2]; 128]).unwrap()),
            VoiceSpec::default(),
        )
        .unwrap();
    m.schedule_resume(1, h, 1., 1.).unwrap();
    let out = render(&mut m, 8);
    assert!(out.iter().flatten().all(|x| *x == 0.));
    assert_eq!(m.playback_info(1, h).unwrap().position_frames, 0);
    assert!(!m.playback_info(1, h).unwrap().paused);
    let out = render(&mut m, 8);
    for (i, s) in out.iter().enumerate() {
        near(s[0], i as f32 / 8.);
    }
    m.schedule_resume(1, h, 2., 0.).unwrap();
    m.pause(1, h, true).unwrap();
    render(&mut m, 32);
    assert_eq!(m.playback_info(1, h).unwrap().position_frames, 8);
    m.stop(1, h, 1.).unwrap();
    render(&mut m, 8);
    assert!(!m.is_playing(1, h));
    let h = m
        .play(
            1,
            decoder::open_pcm(PcmAudio::new(8000, vec![[0.5; 2]; 128]).unwrap()),
            VoiceSpec::default(),
        )
        .unwrap();
    let bad = VoiceSpec {
        rate: f32::NAN,
        ..Default::default()
    };
    assert!(
        m.replace(
            1,
            h,
            decoder::open_pcm(PcmAudio::new(8000, vec![[1.; 2]; 128]).unwrap()),
            bad
        )
        .is_err()
    );
    near(render(&mut m, 1)[0][0], 0.5);
    assert!(m.playback_info(2, h).is_err());
}

#[test]
fn a_full_pool_rejects_crossfade_without_stealing_and_replaces_the_exact_channel() {
    let mut a = ChannelAudio::default();
    for channel in 0..256 {
        a.submit(
            1,
            AudioRequest::PlayPcm {
                channel,
                source: PcmAudio::new(48000, vec![[if channel == 0 { 0.25 } else { 0. }; 2]; 128])
                    .unwrap(),
                parameters: AudioParameters {
                    looping: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    }
    let request = |fade| AudioRequest::PlayPcm {
        channel: 0,
        source: PcmAudio::new(48000, vec![[0.5; 2]; 128]).unwrap(),
        parameters: AudioParameters {
            looping: true,
            replace_fade_ms: fade,
            ..Default::default()
        },
    };
    assert!(a.submit(1, request(2.)).is_err());
    assert_eq!(a.mixer.stats().active, 256);
    assert_eq!(a.mixer.stats().stolen, 0);
    a.render(1).unwrap();
    near(a.take_output()[0][0], 0.25);
    a.submit(1, request(0.)).unwrap();
    assert_eq!(a.mixer.stats().active, 256);
    assert_eq!(a.mixer.stats().stolen, 0);
    a.render(1).unwrap();
    near(a.take_output()[0][0], 0.5);
}
#[test]
fn stop_and_pause_deadlines_advance_on_starvation_while_source_and_initial_fade_wait() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Source(Arc<AtomicBool>);
    impl decoder::AudioSource for Source {
        fn sample_rate(&self) -> u32 {
            8000
        }
        fn frame_count(&self) -> Option<u64> {
            Some(128)
        }
        fn read(&mut self, _: &mut [[f32; 2]]) -> anyhow::Result<usize> {
            panic!("blocking decoder in mixer")
        }
        fn seek(&mut self, _: u64) -> anyhow::Result<()> {
            panic!("blocking seek in mixer")
        }
        fn poll_seek(&mut self, _: u64) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn poll_read(&mut self, out: &mut [[f32; 2]]) -> anyhow::Result<decoder::SourceRead> {
            if !self.0.load(Ordering::Acquire) {
                return Ok(decoder::SourceRead::Pending);
            }
            let n = out.len().min(128);
            out[..n].fill([1.; 2]);
            Ok(decoder::SourceRead::Frames(n))
        }
        fn is_nonblocking(&self) -> bool {
            true
        }
    }
    for mode in 0..4 {
        let ready = Arc::new(AtomicBool::new(false));
        let mut m = Mixer::new(MixerConfig {
            sample_rate: 8000,
            ..Default::default()
        })
        .unwrap();
        let h = m
            .play(
                1,
                Box::new(Source(ready.clone())),
                VoiceSpec {
                    fade_in_ms: 1.,
                    ..Default::default()
                },
            )
            .unwrap();
        if mode == 1 {
            m.stop(1, h, 1.).unwrap();
        }
        if mode == 2 {
            m.pause_fade(1, h, true, 1.).unwrap();
        }
        if mode == 3 {
            m.pending_reflections(1, h).unwrap();
            m.stop(1, h, 1.).unwrap();
        }
        assert!(render(&mut m, 8).iter().flatten().all(|s| *s == 0.));
        if mode == 1 || mode == 3 {
            assert!(!m.is_playing(1, h));
            continue;
        }
        let info = m.playback_info(1, h).unwrap();
        assert_eq!(info.age_frames, 8);
        assert_eq!(info.position_frames, 0);
        if mode == 2 {
            assert!(info.paused);
            assert_eq!(info.fade_out_frames, 0);
            m.schedule_resume(1, h, 0., 0.).unwrap();
        }
        ready.store(true, Ordering::Release);
        let out = render(&mut m, 8);
        if mode == 0 {
            for (i, s) in out.iter().enumerate() {
                near(s[0], i as f32 / 8.);
            }
        } else {
            near(out[0][0], 1.);
        }
    }
}
