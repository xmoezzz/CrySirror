// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::{
    decoder::Clip,
    doppler::*,
    mixer::{Listener, Mixer, MixerConfig, Spatial, VoiceSpec},
    motion::*,
};
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
#[test]
fn radial_model_covers_source_listener_equal_motion_transverse_and_extremes() {
    let settings = DopplerSettings {
        speed_of_sound: 100.,
        smoothing_ms: 0.,
        ..Default::default()
    };
    let calc = |s, l| calculate(settings, [10., 0., 0.], s, [0.; 3], l).unwrap();
    near(calc([-10., 0., 0.], [0.; 3]).ratio.into(), 100. / 90.);
    near(calc([10., 0., 0.], [0.; 3]).ratio.into(), 100. / 110.);
    near(calc([0.; 3], [10., 0., 0.]).ratio.into(), 1.1);
    near(calc([0.; 3], [-10., 0., 0.]).ratio.into(), 0.9);
    near(calc([10., 0., 0.], [10., 0., 0.]).ratio.into(), 1.);
    near(calc([0., 20., 0.], [0., -20., 0.]).ratio.into(), 1.);
    let extreme = calc([-1000., 0., 0.], [1000., 0., 0.]);
    assert!(extreme.velocity_clamped && extreme.ratio_clamped);
    assert_eq!(extreme.ratio, 4.);
    assert_eq!(
        calculate(settings, [0.; 3], [1.; 3], [0.; 3], [-1.; 3])
            .unwrap()
            .ratio,
        1.
    );
    assert_eq!(
        calculate(
            DopplerSettings {
                factor: 0.,
                ..settings
            },
            [10., 0., 0.],
            [-1000.; 3],
            [0.; 3],
            [1000.; 3]
        )
        .unwrap()
        .ratio,
        1.
    );
    for s in [
        DopplerSettings {
            factor: f32::NAN,
            ..settings
        },
        DopplerSettings {
            max_ratio: 0.,
            ..settings
        },
        DopplerSettings {
            speed_of_sound: 0.,
            ..settings
        },
    ] {
        assert!(s.validate().is_err());
    }
    assert!(calculate(settings, [f32::NAN, 0., 0.], [0.; 3], [0.; 3], [0.; 3]).is_err());
}
fn setup(smooth: f32) -> (Mixer, crysirror_audio::mixer::VoiceHandle) {
    let mut m = Mixer::new(MixerConfig {
        sample_rate: 8000,
        ..Default::default()
    })
    .unwrap();
    let h = m
        .play(
            1,
            Clip::new(8000, (0..4096).map(|i| [i as f32 / 4096.; 2]).collect())
                .unwrap()
                .source(),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [0., 0., -10.],
                    velocity: Some([0., 0., 50.]),
                    doppler: Some(DopplerSettings {
                        speed_of_sound: 100.,
                        smoothing_ms: smooth,
                        ..Default::default()
                    }),
                    min_distance: 20.,
                    max_distance: 100.,
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
#[test]
fn samples_and_source_clock_follow_doppler_without_changing_user_rate_or_pitch() {
    let (mut m, h) = setup(0.);
    let pcm = render(&mut m, 8);
    for (i, s) in pcm.iter().enumerate() {
        near(
            s[0].into(),
            (2 * i) as f64 / 4096. * std::f64::consts::FRAC_1_SQRT_2,
        );
    }
    let i = m.playback_info(1, h).unwrap();
    assert_eq!(i.position_frames, 16);
    assert_eq!(i.age_frames, 8);
    assert_eq!(i.transport_frames, 8);
    near(m.doppler_info(1, h).unwrap().ratio.into(), 2.);
    assert_eq!(m.spec(1, h).unwrap().rate, 1.);
    m.set_rate(1, h, 2., 12.).unwrap();
    render(&mut m, 1);
    assert_eq!(m.playback_info(1, h).unwrap().position_frames, 24);
    near(m.doppler_info(1, h).unwrap().effective_rate.into(), 8.);
    assert!(m.doppler_info(2, h).is_err());
    let mut spatial = m.spec(1, h).unwrap().spatial.unwrap();
    spatial.doppler = None;
    m.set_spatial(1, h, Some(spatial)).unwrap();
    render(&mut m, 1);
    near(m.doppler_info(1, h).unwrap().ratio.into(), 1.);
}
#[test]
fn interpolation_is_chunk_invariant_and_tracks_output_time_while_input_is_paused() {
    let (mut a, h) = setup(1.);
    let (mut b, k) = setup(1.);
    let all = render(&mut a, 16);
    let mut split = render(&mut b, 3);
    split.extend(render(&mut b, 13));
    assert_eq!(all, split);
    near(a.position(1, h).unwrap(), 27.5 / 8000.);
    assert_eq!(a.doppler_info(1, h).unwrap().smoothing_frames, 0);
    let mut s = a.spec(1, h).unwrap().spatial.unwrap();
    s.velocity = Some([0., 0., -50.]);
    a.set_spatial(1, h, Some(s)).unwrap();
    a.pause(1, h, true).unwrap();
    let before = a.position(1, h).unwrap();
    render(&mut a, 8);
    near(a.position(1, h).unwrap(), before);
    near(a.doppler_info(1, h).unwrap().ratio.into(), 2. / 3.);
    assert_eq!(b.playback_info(1, k).unwrap().position_frames, 27);
}
#[test]
fn listener_velocity_is_scoped_and_invalid_updates_are_atomic() {
    let (mut a, h) = setup(0.);
    let before = a.listener(1);
    assert!(
        a.set_listener(
            1,
            Listener {
                velocity: Some([f32::NAN, 0., 0.]),
                ..before
            }
        )
        .is_err()
    );
    assert_eq!(a.listener(1).velocity, None);
    a.set_listener(
        2,
        Listener {
            velocity: Some([0., 0., 50.]),
            ..before
        },
    )
    .unwrap();
    render(&mut a, 1);
    near(a.doppler_info(1, h).unwrap().ratio.into(), 2.);
    a.set_listener(
        1,
        Listener {
            velocity: Some([0., 0., 50.]),
            ..before
        },
    )
    .unwrap();
    render(&mut a, 1);
    near(a.doppler_info(1, h).unwrap().ratio.into(), 1.);
}
#[test]
fn pose_inference_reports_resets_zero_time_and_explicit_override() {
    let mut t = MotionTracker::default();
    let settings = MotionSettings::default();
    assert_eq!(
        t.observe([0.; 3], 16, None, settings).unwrap().status,
        MotionStatus::Initial
    );
    let x = t.observe([1., 0., 0.], 100, None, settings).unwrap();
    assert_eq!(x.velocity, [10., 0., 0.]);
    let x = t.observe([1., 0., 0.], 0, None, settings).unwrap();
    assert_eq!(x.velocity, [10., 0., 0.]);
    assert_eq!(x.status, MotionStatus::ZeroInterval);
    assert_eq!(
        t.observe([2., 0., 0.], 0, None, settings).unwrap().status,
        MotionStatus::Discontinuity
    );
    assert_eq!(
        t.observe([2., 0., 0.], 251, None, settings).unwrap().status,
        MotionStatus::Gap
    );
    assert_eq!(
        t.observe([100., 0., 0.], 100, None, settings)
            .unwrap()
            .status,
        MotionStatus::Discontinuity
    );
    let x = t.observe([100.; 3], 0, Some([2.; 3]), settings).unwrap();
    assert_eq!(x.status, MotionStatus::Explicit);
    assert_eq!(x.velocity, [2.; 3]);
    t.reset();
    assert_eq!(
        t.observe([0.; 3], 16, None, settings).unwrap().velocity,
        [0.; 3]
    );
    assert!(
        t.observe([f32::INFINITY, 0., 0.], 16, None, settings)
            .is_err()
    );
}
