// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_acoustics::{
    reflection_dsp::EarlyReflections,
    reflections::{ReflectionOptions, ReflectionPath, ReflectionPaths, ReflectionQuery},
};
use crysirror_audio::{
    decoder,
    mixer::{Mixer, MixerConfig, Spatial, VoiceHandle, VoiceSpec},
};
use crysirror_core::service::{AudioParameters, AudioRequest, PcmAudio};
fn paths(rate: u32, delay: f32, direction: [f32; 3]) -> ReflectionPaths {
    ReflectionPaths {
        query: ReflectionQuery {
            source: [0., 0., 2.],
            listener: [0.; 3],
            listener_forward: [0., 0., -1.],
            listener_up: [0., 1., 0.],
            options: ReflectionOptions::default(),
        },
        tested_triangles: 1,
        paths: vec![ReflectionPath {
            mesh: 1,
            triangle: 0,
            material: 0,
            bounce: [0., 0., -2.],
            length_metres: 6.,
            delay_seconds: 6. / 343.,
            excess_delay_seconds: delay / rate as f32,
            gains: [1.; 3],
            leg_transmission: [1.; 3],
            crossings: 0,
            direction,
        }],
    }
}
fn setup(rate: u32) -> (Mixer, VoiceHandle) {
    let mut m = Mixer::new(MixerConfig {
        sample_rate: rate,
        ..Default::default()
    })
    .unwrap();
    let h = m
        .play(
            1,
            decoder::open_pcm(PcmAudio::new(rate, vec![[1.; 2]]).unwrap()),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [0., 0., 2.],
                    min_distance: 10.,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    (m, h)
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
fn fractional_delay_oracle_pan_reset_and_resource_lifetime_do_not_need_a_mixer_or_vm() {
    let mut effect = EarlyReflections::prepare(8000, paths(8000, 1.5, [1., 0., 0.]), 0).unwrap();
    let lease = std::sync::Arc::new(());
    let weak = std::sync::Arc::downgrade(&lease);
    effect.retain_storage(lease);
    let output = (0..8)
        .map(|i| effect.process_frame(if i == 0 { 1. } else { 0. }).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(output[0], [0.; 2]);
    near(output[1][1], 0.5);
    near(output[2][1], 0.5);
    assert!(output.iter().all(|v| v[0] == 0.));
    effect.reset();
    for _ in 0..8 {
        assert_eq!(effect.process_frame(0.).unwrap(), [0.; 2]);
    }
    assert!(weak.upgrade().is_some());
    drop(effect);
    assert!(weak.upgrade().is_none());
}
#[test]
fn echo_bypasses_direct_occlusion_drains_after_source_eof_and_native_completion_excludes_it() {
    let (mut m, h) = setup(8000);
    m.set_acoustic_gains(1, h, Some([0.; 3])).unwrap();
    m.set_reflections(
        1,
        h,
        Some(EarlyReflections::prepare(8000, paths(8000, 3., [0., 0., -1.]), 0).unwrap()),
    )
    .unwrap();
    let first = render(&mut m, 1);
    assert_eq!(first, vec![[0.; 2]]);
    assert!(!m.source_is_playing(1, h));
    assert!(m.is_playing(1, h));
    let rest = render(&mut m, 100);
    near(rest[2][0], std::f32::consts::FRAC_1_SQRT_2);
    near(rest[2][1], std::f32::consts::FRAC_1_SQRT_2);
    assert!(!m.is_playing(1, h));
    let mut channel = crysirror_audio::channels::ChannelAudio::default();
    channel
        .submit(
            1,
            AudioRequest::PlayPcm {
                channel: 5,
                source: PcmAudio::new(48000, vec![[1.; 2]]).unwrap(),
                parameters: AudioParameters::default(),
            },
        )
        .unwrap();
    channel
        .set_reflections(
            1,
            5,
            Some(EarlyReflections::prepare(48000, paths(48000, 100., [0., 0., -1.]), 0).unwrap()),
        )
        .unwrap();
    channel.render(1).unwrap();
    assert!(!channel.is_playing(1, 5));
    channel.render(2).unwrap();
    assert!(
        channel
            .take_output()
            .iter()
            .flatten()
            .any(|x| x.abs() > 0.6)
    );
}
#[test]
fn initial_worker_hold_cancel_pose_invalidation_and_seek_preserve_source_boundaries() {
    let (mut m, h) = setup(8000);
    m.pending_reflections(1, h).unwrap();
    assert!(render(&mut m, 16).iter().flatten().all(|x| *x == 0.));
    near(m.position(1, h).unwrap() as f32, 0.);
    m.set_reflections(
        1,
        h,
        Some(EarlyReflections::prepare(8000, paths(8000, 3., [0., 0., -1.]), 0).unwrap()),
    )
    .unwrap();
    render(&mut m, 1);
    m.seek(1, h, 0).unwrap();
    let out = render(&mut m, 4);
    near(out[0][0], std::f32::consts::FRAC_1_SQRT_2);
    near(out[3][0], std::f32::consts::FRAC_1_SQRT_2);
    m.set_listener(
        1,
        crysirror_audio::mixer::Listener {
            position: [1., 0., 0.],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(m.reflection_paths(1, h).unwrap().is_none());
    render(&mut m, 1);
    assert!(!m.is_playing(1, h));
    let (mut m, h) = setup(8000);
    m.pending_reflections(1, h).unwrap();
    m.cancel_reflection_preparation(1, h).unwrap();
    near(render(&mut m, 1)[0][0], std::f32::consts::FRAC_1_SQRT_2);
}
#[test]
fn hrtf_bridge_reference_latency_is_applied_once_and_mismatched_effects_are_rejected() {
    use crysirror_acoustics::binaural::{HrtfDataset, HrtfOptions};
    let (mut m, h) = setup(48000);
    let dataset = HrtfDataset::from_bytes(
        br#"{"sample_rate":48000,"measurements":[{"direction":[0,0,1],"left":[1],"right":[1]}]}"#,
        48000,
        HrtfOptions {
            partition_frames: 32,
            max_delay_seconds: 0.,
            ..Default::default()
        },
    )
    .unwrap();
    let (_publisher, binaural) = dataset.voice([0., 0., 1.]).unwrap();
    m.set_binaural(1, h, Some(binaural)).unwrap();
    assert!(
        m.set_reflections(
            1,
            h,
            Some(EarlyReflections::prepare(48000, paths(48000, 3., [0., 0., -1.]), 0).unwrap())
        )
        .is_err()
    );
    m.set_reflections(
        1,
        h,
        Some(EarlyReflections::prepare(48000, paths(48000, 3., [0., 0., -1.]), 32).unwrap()),
    )
    .unwrap();
    let mut out = render(&mut m, 33);
    assert!(!m.source_is_playing(1, h));
    assert!(m.is_playing(1, h));
    out.extend(render(&mut m, 991));
    assert!(out[..32].iter().flatten().all(|x| *x == 0.));
    near(out[32][0], 1.);
    near(out[35][0], std::f32::consts::FRAC_1_SQRT_2);
    assert!(!m.is_playing(1, h));
}
#[test]
fn invalid_authored_taps_are_rejected_before_buffer_allocation() {
    let mut p = paths(8000, 1., [0., 0., -1.]);
    p.paths[0].gains[0] = f32::NAN;
    assert!(EarlyReflections::prepare(8000, p, 0).is_err());
    let mut p = paths(8000, 1., [0., 0., -1.]);
    p.paths[0].excess_delay_seconds = 3.;
    assert!(EarlyReflections::prepare(8000, p, 0).is_err());
    assert!(EarlyReflections::prepare(8000, paths(8000, 1., [0.; 3]), 0).is_err());
}
