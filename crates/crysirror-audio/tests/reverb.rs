// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Independent impulse oracles over shared buses; no VM or output device.
use crysirror_acoustics::reverb::ReverbSpec;
use crysirror_audio::{
    decoder,
    mixer::{Bus, BusSettings, Mixer, MixerConfig, VoiceSpec},
    reverb::{PreparedReverb, ReverbSend, ReverbSettings},
};
use crysirror_core::service::{AudioParameters, AudioRequest, PcmAudio};
fn settings() -> ReverbSettings {
    ReverbSettings {
        room: ReverbSpec {
            room_size_metres: 10.,
            decay_seconds: 0.3,
            damping: 0.,
        },
        sends: vec![ReverbSend {
            bus: Bus::Sfx,
            gain: 1.,
        }],
        ..Default::default()
    }
}
fn mixer(master: f32, source_owner: u32, mute_source: bool, mute_return: bool) -> Mixer {
    let mut m = Mixer::new(MixerConfig {
        sample_rate: 8000,
        ..Default::default()
    })
    .unwrap();
    m.set_reverb(1, Some(PreparedReverb::prepare(8000, settings()).unwrap()))
        .unwrap();
    m.set_bus(
        1,
        Bus::Master,
        BusSettings {
            gain: master,
            muted: false,
        },
    )
    .unwrap();
    m.set_bus(
        1,
        Bus::Sfx,
        BusSettings {
            gain: 1.,
            muted: mute_source,
        },
    )
    .unwrap();
    m.set_bus(
        1,
        Bus::Ambient,
        BusSettings {
            gain: 1.,
            muted: mute_return,
        },
    )
    .unwrap();
    m.play(
        source_owner,
        decoder::open_pcm(PcmAudio::new(8000, vec![[1.; 2]]).unwrap()),
        VoiceSpec::default(),
    )
    .unwrap();
    m
}
fn render(m: &mut Mixer, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    m.render(&mut out).unwrap();
    out
}
fn energy(v: &[[f32; 2]]) -> f64 {
    v.iter().flatten().map(|x| (*x as f64).powi(2)).sum()
}
#[test]
fn impulse_tail_survives_source_eof_and_decays_without_dry_feedback() {
    let mut m = mixer(1., 1, false, false);
    let out = render(&mut m, 16000);
    assert_eq!(out[0], [1.; 2]);
    assert_eq!(m.stats().active, 0);
    assert!(
        out[1..250].iter().flatten().all(|x| *x == 0.),
        "FDN output arrived before its shortest delay"
    );
    let early = energy(&out[800..2400]);
    let late = energy(&out[12000..13600]);
    assert!(
        early > 1e-5 && late < early * 0.01,
        "reverb did not decay: {early}/{late}"
    );
    assert!(out.iter().flatten().all(|x| x.is_finite()));
}
#[test]
fn master_scales_once_source_mute_blocks_sends_return_mute_keeps_dry_and_clients_are_isolated() {
    let base = render(&mut mixer(1., 1, false, false), 8000);
    let half = render(&mut mixer(0.5, 1, false, false), 8000);
    for (a, b) in base.iter().flatten().zip(half.iter().flatten()) {
        assert!((a * 0.5 - b).abs() < 1e-7);
    }
    let muted = render(&mut mixer(1., 1, true, false), 8000);
    assert!(muted.iter().flatten().all(|x| *x == 0.));
    for mut m in [mixer(1., 1, false, true), mixer(1., 2, false, false)] {
        let out = render(&mut m, 8000);
        assert_eq!(out[0], [1.; 2]);
        assert!(out[1..].iter().flatten().all(|x| *x == 0.));
    }
}
#[test]
fn arbitrary_blocks_preserve_the_same_tail_and_clear_releases_actual_storage() {
    let whole = render(&mut mixer(1., 1, false, false), 8000);
    let mut m = mixer(1., 1, false, false);
    let mut split = vec![];
    for size in [1, 3, 61, 127, 7, 1024, 4096, 2681] {
        split.extend(render(&mut m, size));
    }
    assert_eq!(whole, split);
    let lease = std::sync::Arc::new(());
    let weak = std::sync::Arc::downgrade(&lease);
    let mut effect = PreparedReverb::prepare(8000, settings()).unwrap();
    effect.retain_storage(lease);
    m.set_reverb(1, Some(effect)).unwrap();
    assert!(weak.upgrade().is_some());
    assert!(
        m.set_reverb(1, Some(PreparedReverb::prepare(48000, settings()).unwrap()))
            .is_err()
    );
    assert!(m.reverb(1).is_some());
    assert!(weak.upgrade().is_some());
    m.clear();
    assert!(m.reverb(1).is_none());
    assert!(weak.upgrade().is_none());
    assert!(render(&mut m, 100).iter().flatten().all(|x| *x == 0.));
}
#[test]
fn native_pcm_channels_share_the_bus_effect_and_keep_eof_tail() {
    let mut audio = crysirror_audio::channels::ChannelAudio::default();
    audio
        .mixer
        .set_reverb(1, Some(PreparedReverb::prepare(48000, settings()).unwrap()))
        .unwrap();
    audio
        .submit(
            1,
            AudioRequest::PlayPcm {
                channel: 2,
                source: PcmAudio::new(48000, vec![[1.; 2]]).unwrap(),
                parameters: AudioParameters::default(),
            },
        )
        .unwrap();
    audio.render(500).unwrap();
    let out = audio.take_output();
    assert_eq!(out[0], [1.; 2]);
    assert!(!audio.is_playing(1, 2));
    assert!(energy(&out[2400..]) > 1e-5);
}
#[test]
fn invalid_or_duplicate_routes_and_nonfinite_room_parameters_are_rejected() {
    for spec in [
        ReverbSpec {
            room_size_metres: f64::NAN,
            ..Default::default()
        },
        ReverbSpec {
            decay_seconds: 0.,
            ..Default::default()
        },
        ReverbSpec {
            damping: 1.1,
            ..Default::default()
        },
    ] {
        assert!(
            PreparedReverb::prepare(
                48000,
                ReverbSettings {
                    room: spec,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let mut s = settings();
    s.sends.push(s.sends[0].clone());
    assert!(s.validate().is_err());
    s = settings();
    s.return_bus = Bus::Master;
    assert!(s.validate().is_err());
    s = settings();
    s.sends[0].bus = Bus::Master;
    assert!(s.validate().is_err());
    s = settings();
    s.wet_gain = f32::NAN;
    assert!(s.validate().is_err());
}
