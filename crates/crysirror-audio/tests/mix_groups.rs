// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Independent gain/control and ownership oracles without a VM/device.
use crysirror_audio::{
    channels::ChannelAudio,
    decoder,
    mixer::{Bus, BusSettings, Ducking, Mixer, MixerConfig, VoiceSpec},
    reverb::{PreparedReverb, ReverbSend, ReverbSettings},
};
use crysirror_core::service::{
    AudioMixGroupSettings as Settings, AudioParameters, AudioRequest, PcmAudio,
};
fn mixer() -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 8000,
        ..Default::default()
    })
    .unwrap()
}
fn settings(parent: Option<&str>, gain: f32, muted: bool) -> Settings {
    Settings {
        parent: parent.map(str::to_owned),
        gain,
        muted,
    }
}
fn source() -> Box<dyn decoder::AudioSource> {
    decoder::open_pcm(PcmAudio::new(8000, vec![[1.; 2]; 512]).unwrap())
}
fn render(m: &mut Mixer, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    m.render(&mut out).unwrap();
    out
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
#[test]
fn hierarchy_scopes_live_routing_and_bus_master_gain_multiply_once() {
    let mut m = mixer();
    m.set_mix_group(1, "actors", settings(None, 0.5, false), 0.)
        .unwrap();
    m.set_mix_group(1, "hero", settings(Some("actors"), 0.25, false), 0.)
        .unwrap();
    m.set_mix_group(2, "hero", settings(None, 0.75, false), 0.)
        .unwrap();
    for bus in [Bus::Sfx, Bus::Master] {
        m.set_bus(
            1,
            bus,
            BusSettings {
                gain: 0.5,
                muted: false,
            },
        )
        .unwrap();
    }
    let a = m
        .play(
            1,
            source(),
            VoiceSpec {
                mix_group: Some("hero".into()),
                gain: 0.8,
                ..Default::default()
            },
        )
        .unwrap();
    m.play(
        2,
        source(),
        VoiceSpec {
            mix_group: Some("hero".into()),
            ..Default::default()
        },
    )
    .unwrap();
    near(render(&mut m, 1)[0][0], 0.775);
    assert!(m.set_voice_mix_group(2, a, None).is_err());
    assert!(m.remove_mix_group(1, "hero").is_err());
    assert!(m.remove_mix_group(1, "actors").is_err());
    let at = m.playback_info(1, a).unwrap().position_frames;
    m.set_voice_mix_group(1, a, None).unwrap();
    assert_eq!(m.playback_info(1, a).unwrap().position_frames, at);
    near(render(&mut m, 1)[0][0], 0.95);
    m.remove_mix_group(1, "hero").unwrap();
    m.remove_mix_group(1, "actors").unwrap();
    assert!(m.mix_group(1, "hero").unwrap().is_none());
    assert!(m.mix_group(2, "hero").unwrap().is_some());
    let before = m.stats();
    assert!(
        m.play(
            1,
            source(),
            VoiceSpec {
                mix_group: Some("missing".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(m.stats().active, before.active);
    assert_eq!(m.stats().stolen, before.stolen);
}
#[test]
fn graph_validation_is_atomic_for_cycles_descendants_orphans_bad_values_and_budgets() {
    let mut m = mixer();
    m.set_mix_group(1, "root", Settings::default(), 0.).unwrap();
    m.set_mix_group(1, "leaf", settings(Some("root"), 0.25, false), 0.)
        .unwrap();
    let before = m.mix_group(1, "root").unwrap();
    for s in [
        settings(Some("leaf"), 0.5, false),
        settings(Some("missing"), 0.5, false),
        settings(None, f32::NAN, false),
    ] {
        assert!(m.set_mix_group(1, "root", s, 0.).is_err());
        assert_eq!(m.mix_group(1, "root").unwrap(), before);
    }
    assert!(
        m.set_mix_group(1, "root", Settings::default(), f32::NAN)
            .is_err()
    );
    assert_eq!(m.mix_group(1, "root").unwrap(), before);
    for i in 0..7 {
        let p = (i > 0).then(|| format!("chain{}", i - 1));
        m.set_mix_group(
            1,
            &format!("chain{i}"),
            Settings {
                parent: p,
                ..Default::default()
            },
            0.,
        )
        .unwrap();
    }
    assert!(
        m.set_mix_group(1, "root", settings(Some("chain6"), 1., false), 0.)
            .is_err()
    );
    assert_eq!(m.mix_group(1, "root").unwrap(), before);
    let mut m = mixer();
    for client in 1..=4 {
        for i in 0..64 {
            m.set_mix_group(client, &format!("g{i}"), Settings::default(), 0.)
                .unwrap();
        }
    }
    assert!(
        m.set_mix_group(1, "overflow", Settings::default(), 0.)
            .is_err()
    );
    assert!(
        m.set_mix_group(5, "overflow", Settings::default(), 0.)
            .is_err()
    );
    m.remove_mix_group(1, "g0").unwrap();
    m.set_mix_group(5, "reused", Settings::default(), 0.)
        .unwrap();
}
#[test]
fn group_fades_follow_output_clock_during_silence_and_paused_source_clocks() {
    let mut m = mixer();
    m.set_mix_group(1, "group", Settings::default(), 0.)
        .unwrap();
    m.set_mix_group_gain(1, "group", 0., 1.).unwrap();
    render(&mut m, 8);
    let i = m.mix_group(1, "group").unwrap().unwrap();
    assert_eq!(i.fade_frames, 0);
    near(i.gain, 0.);
    let h = m
        .play(
            1,
            source(),
            VoiceSpec {
                mix_group: Some("group".into()),
                paused: true,
                ..Default::default()
            },
        )
        .unwrap();
    m.set_mix_group_gain(1, "group", 1., 1.).unwrap();
    assert!(render(&mut m, 8).iter().flatten().all(|x| *x == 0.));
    assert_eq!(m.playback_info(1, h).unwrap().position_frames, 0);
    assert_eq!(m.playback_info(1, h).unwrap().age_frames, 8);
    m.pause(1, h, false).unwrap();
    near(render(&mut m, 1)[0][0], 1.);
    m.set_mix_group_gain(1, "group", 0., 1.).unwrap();
    let out = render(&mut m, 8);
    for (i, s) in out.iter().enumerate() {
        near(s[0], 1. - i as f32 / 8.);
    }
    assert_eq!(m.mix_group(1, "group").unwrap().unwrap().fade_frames, 0);
}
fn dialogue() -> Mixer {
    let mut m = mixer();
    m.set_ducking(
        1,
        Ducking {
            gain: 0.25,
            attack_ms: 0.,
            release_ms: 0.,
        },
    )
    .unwrap();
    m.set_mix_group(1, "dialogue", Settings::default(), 0.)
        .unwrap();
    m.play(
        1,
        source(),
        VoiceSpec {
            bus: Bus::Music,
            ..Default::default()
        },
    )
    .unwrap();
    m.play(
        1,
        source(),
        VoiceSpec {
            bus: Bus::Voice,
            mix_group: Some("dialogue".into()),
            ..Default::default()
        },
    )
    .unwrap();
    m.set_mix_group(1, "dialogue", settings(None, 1., true), 1.)
        .unwrap();
    m
}
#[test]
fn grouped_dialogue_mute_releases_ducking_at_the_sample_boundary_in_any_block_partition() {
    let whole = render(&mut dialogue(), 16);
    for (i, s) in whole.iter().enumerate() {
        near(s[0], if i < 8 { 1.25 - i as f32 / 8. } else { 1. });
    }
    let mut m = dialogue();
    let mut split = vec![];
    for n in [3, 5, 1, 7] {
        split.extend(render(&mut m, n));
    }
    assert_eq!(whole, split);
}
#[test]
fn native_group_gain_preserves_parent_and_host_mute_while_pcm_clocks_continue() {
    let mut a = ChannelAudio::default();
    a.submit(
        1,
        AudioRequest::SetMixGroup {
            name: "root".into(),
            settings: settings(None, 0.5, false),
            fade_ms: 0.,
        },
    )
    .unwrap();
    a.submit(
        1,
        AudioRequest::SetMixGroup {
            name: "native".into(),
            settings: settings(Some("root"), 0.75, true),
            fade_ms: 0.,
        },
    )
    .unwrap();
    a.submit(
        1,
        AudioRequest::SetMixGroupGain {
            name: "native".into(),
            gain: 0.25,
            fade_ms: 0.,
        },
    )
    .unwrap();
    let i = a.mixer.mix_group(1, "native").unwrap().unwrap();
    assert_eq!(i.settings.parent.as_deref(), Some("root"));
    assert!(i.settings.muted);
    near(i.settings.gain, 0.25);
    a.submit(
        1,
        AudioRequest::PlayPcm {
            channel: 7,
            source: PcmAudio::new(48000, vec![[1.; 2]; 1000]).unwrap(),
            parameters: AudioParameters {
                mix_group: Some("native".into()),
                ..Default::default()
            },
        },
    )
    .unwrap();
    a.render(1).unwrap();
    assert!(a.take_output().iter().flatten().all(|x| *x == 0.));
    assert_eq!(a.playback_info(1, 7).unwrap().unwrap().position_frames, 48);
}
fn room(group: bool) -> Mixer {
    let mut m = mixer();
    let mut r = ReverbSettings::default();
    r.room.decay_seconds = 0.3;
    r.sends = vec![ReverbSend {
        bus: Bus::Sfx,
        gain: 1.,
    }];
    m.set_reverb(1, Some(PreparedReverb::prepare(8000, r).unwrap()))
        .unwrap();
    if group {
        m.set_mix_group(1, "g", settings(None, 0.5, false), 0.)
            .unwrap();
    }
    m.set_bus(
        1,
        Bus::Master,
        BusSettings {
            gain: 0.5,
            muted: false,
        },
    )
    .unwrap();
    m.play(
        1,
        decoder::open_pcm(PcmAudio::new(8000, vec![[1.; 2]]).unwrap()),
        VoiceSpec {
            gain: if group { 1. } else { 0.5 },
            mix_group: group.then(|| "g".into()),
            ..Default::default()
        },
    )
    .unwrap();
    m
}
#[test]
fn grouped_impulse_scales_direct_and_late_reverb_send_once_and_retains_committed_tail() {
    let mut grouped = room(true);
    let first = render(&mut grouped, 1);
    near(first[0][0], 0.25);
    grouped
        .set_mix_group(1, "g", settings(None, 0.5, true), 0.)
        .unwrap();
    grouped.remove_mix_group(1, "g").unwrap();
    let tail = render(&mut grouped, 7999);
    assert!(tail.iter().flatten().any(|x| x.abs() > 1e-5));
    let mut actual = first;
    actual.extend(tail);
    let reference = render(&mut room(false), 8000);
    for (a, b) in actual.iter().flatten().zip(reference.iter().flatten()) {
        near(*a, *b);
    }
}
