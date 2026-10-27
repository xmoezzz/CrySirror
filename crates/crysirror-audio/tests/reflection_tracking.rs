// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_acoustics::{
    reflection_dsp::{EarlyReflections, ReflectionPlan},
    reflections::{ReflectionOptions, ReflectionPath, ReflectionPaths, ReflectionQuery},
};
use std::sync::Arc;
fn paths(delay: f32, gain: f32) -> ReflectionPaths {
    ReflectionPaths {
        query: ReflectionQuery {
            source: [0., 0., 2.],
            listener: [0.; 3],
            listener_forward: [0., 0., -1.],
            listener_up: [0., 1., 0.],
            options: ReflectionOptions {
                max_delay_seconds: 0.1,
                ..Default::default()
            },
        },
        tested_triangles: 1,
        paths: vec![ReflectionPath {
            mesh: 1,
            triangle: 0,
            material: 0,
            bounce: [0., 0., -2.],
            length_metres: 6.,
            delay_seconds: 6. / 343.,
            excess_delay_seconds: delay / 8000.,
            gains: [gain; 3],
            leg_transmission: [1.; 3],
            crossings: 0,
            direction: [0., 0., -1.],
        }],
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
#[test]
fn changing_delay_reuses_past_samples_including_an_initially_empty_plan() {
    for empty in [false, true] {
        let mut initial = paths(1., 1.);
        if empty {
            initial.paths.clear();
        }
        let mut fx = EarlyReflections::prepare_tracked(8000, initial, 0).unwrap();
        let capacity = fx.history_capacity_frames();
        for i in 0..20 {
            fx.process_frame(if i == 0 { 1. } else { 0. }).unwrap();
        }
        fx.publish_plan(ReflectionPlan::prepare(8000, paths(40., 1.), 0).unwrap(), 1)
            .unwrap();
        let out = (20..50)
            .map(|_| fx.process_frame(0.).unwrap())
            .collect::<Vec<_>>();
        near(out[20][0], std::f32::consts::FRAC_1_SQRT_2);
        assert_eq!(capacity, fx.history_capacity_frames());
    }
}
#[test]
fn crossfade_is_sample_bounded_and_retired_plans_live_until_owner_collection() {
    let mut fx = EarlyReflections::prepare_tracked(8000, paths(0., 1.), 0).unwrap();
    for _ in 0..16 {
        fx.process_frame(1.).unwrap();
    }
    let receipt = Arc::new(());
    let weak = Arc::downgrade(&receipt);
    let mut plan = ReflectionPlan::prepare(8000, paths(0., 0.5), 0).unwrap();
    plan.retain_storage(receipt);
    fx.publish_plan(plan, 1).unwrap();
    assert!(!fx.can_publish_plan());
    assert!(
        fx.publish_plan(ReflectionPlan::prepare(8000, paths(0., 0.), 0).unwrap(), 1)
            .is_err()
    );
    for i in 0..8 {
        near(
            fx.process_frame(1.).unwrap()[0],
            std::f32::consts::FRAC_1_SQRT_2 * (1. - 0.5 * i as f32 / 8.),
        );
    }
    fx.publish_plan(ReflectionPlan::prepare(8000, paths(0., 0.), 0).unwrap(), 1)
        .unwrap();
    for _ in 0..8 {
        fx.process_frame(1.).unwrap();
    }
    assert!(weak.upgrade().is_some(), "sample loop reclaimed a plan");
    let retired = fx.retire_plan().unwrap();
    assert!(weak.upgrade().is_some());
    drop(retired);
    assert!(weak.upgrade().is_none());
}
#[test]
fn invalidated_geometry_fades_from_silence_and_seek_never_reveals_old_history() {
    let mut fx = EarlyReflections::prepare_tracked(8000, paths(0., 1.), 0).unwrap();
    for _ in 0..16 {
        fx.process_frame(1.).unwrap();
    }
    fx.set_audible(false);
    assert_eq!(fx.process_frame(1.).unwrap(), [0.; 2]);
    fx.publish_plan(ReflectionPlan::prepare(8000, paths(0., 0.5), 0).unwrap(), 1)
        .unwrap();
    assert_eq!(fx.process_frame(1.).unwrap(), [0.; 2]);
    for i in 1..8 {
        near(
            fx.process_frame(1.).unwrap()[0],
            std::f32::consts::FRAC_1_SQRT_2 * 0.5 * i as f32 / 8.,
        );
    }
    fx.reset();
    for _ in 0..20 {
        assert_eq!(fx.process_frame(0.).unwrap(), [0.; 2]);
    }
    let mut too_long = paths(0., 1.);
    too_long.query.options.max_delay_seconds = 2.;
    too_long.paths[0].excess_delay_seconds = 1.;
    assert!(
        fx.publish_plan(ReflectionPlan::prepare(8000, too_long, 0).unwrap(), 1)
            .is_err()
    );
}

#[test]
fn native_channels_publish_moving_plans_and_keep_source_completion_separate() {
    use crysirror_audio::channels::ChannelAudio;
    use crysirror_core::service::{AudioParameters, AudioRequest, PcmAudio};
    let mut channels = ChannelAudio::default();
    channels
        .submit(
            1,
            AudioRequest::PlayPcm {
                channel: 7,
                source: PcmAudio::new(48000, vec![[1.; 2]; 480]).unwrap(),
                parameters: AudioParameters::default(),
            },
        )
        .unwrap();
    channels
        .set_reflections(
            1,
            7,
            Some(EarlyReflections::prepare_tracked(48000, paths(40., 1.), 0).unwrap()),
        )
        .unwrap();
    channels.render(1).unwrap();
    channels.take_output();
    let mut next = paths(200., 0.5);
    next.query.source = [1., 0., 2.];
    channels
        .publish_reflection_plan(1, 7, ReflectionPlan::prepare(48000, next, 0).unwrap(), 1)
        .unwrap();
    assert!(!channels.reflection_plan_ready(1, 7).unwrap());
    assert!(
        channels
            .publish_reflection_plan(
                2,
                7,
                ReflectionPlan::prepare(48000, paths(200., 0.5), 0).unwrap(),
                1
            )
            .is_err()
    );
    channels.render(1).unwrap();
    assert!(channels.reflection_plan_ready(1, 7).unwrap());
    channels.render(10).unwrap();
    assert!(!channels.is_playing(1, 7));
    channels.render(20).unwrap();
    assert!(channels.take_output().iter().flatten().any(|s| *s > 0.3));
}
