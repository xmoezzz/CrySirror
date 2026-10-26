// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
use crysirror_acoustics::binaural::*;
fn data() -> HrirAsset {
    HrirAsset {
        sample_rate: 48000,
        measurements: vec![
            HrirMeasurement {
                direction: [1., 0., 0.],
                left: vec![1., 0.5, 0.25],
                right: vec![0.25, 0.125, 0.],
                delays_seconds: [0., 0.],
            },
            HrirMeasurement {
                direction: [-1., 0., 0.],
                left: vec![0.25, 0.125, 0.],
                right: vec![1., 0.5, 0.25],
                delays_seconds: [0., 0.],
            },
        ],
    }
}
fn dataset(asset: &HrirAsset) -> HrtfDataset {
    HrtfDataset::from_bytes(
        &serde_json::to_vec(asset).unwrap(),
        48000,
        HrtfOptions {
            max_delay_seconds: 0.,
            ..Default::default()
        },
    )
    .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn convolution_matches_direct_fir_oracle_and_has_explicit_partition_latency() {
    let ds = dataset(&data());
    let (_pub, mut stream) = ds.voice([1., 0., 0.]).unwrap();
    assert_eq!(stream.latency_frames(), 64);
    let input = (0..257).map(|i| (i as f32 * 0.3).sin()).collect::<Vec<_>>();
    let mut output = vec![];
    for x in input
        .iter()
        .copied()
        .chain(std::iter::repeat_n(0., stream.tail_frames()))
    {
        output.push(stream.process(x).unwrap());
    }
    for (i, y) in output.iter().enumerate() {
        for (ear, taps) in [
            data().measurements[0].left.clone(),
            data().measurements[0].right.clone(),
        ]
        .iter()
        .enumerate()
        {
            let expected = if i < 64 {
                0.
            } else {
                taps.iter()
                    .enumerate()
                    .filter_map(|(j, h)| {
                        (i - 64)
                            .checked_sub(j)
                            .and_then(|at| input.get(at))
                            .map(|x| h * x)
                    })
                    .sum()
            };
            near(y[ear], expected);
        }
    }
}
#[test]
fn left_right_delays_and_crossfade_updates_reach_real_samples() {
    let mut asset = data();
    asset.measurements[0].left = vec![1.; 1];
    asset.measurements[0].right = vec![0.];
    asset.measurements[1].left = vec![0.];
    asset.measurements[1].right = vec![1.];
    let ds = dataset(&asset);
    let (mut publisher, mut voice) = ds.voice([1., 0., 0.]).unwrap();
    for _ in 0..128 {
        voice.process(1.).unwrap();
    }
    near(voice.process(1.).unwrap()[0], 1.);
    publisher
        .publish(ds.prepare_filter([-1., 0., 0.]).unwrap())
        .unwrap();
    let mut middle = false;
    for _ in 0..2048 {
        let y = voice.process(1.).unwrap();
        near(y[0] + y[1], 1.);
        middle |= y[0] > 0.01 && y[0] < 0.99;
    }
    assert!(middle);
    let y = voice.process(1.).unwrap();
    near(y[0], 0.);
    near(y[1], 1.);
    publisher.reclaim();
    let mut asset = data();
    for m in &mut asset.measurements {
        m.delays_seconds = [2.1 / 48000., 5.1 / 48000.];
    }
    let ds = HrtfDataset::from_bytes(
        &serde_json::to_vec(&asset).unwrap(),
        48000,
        Default::default(),
    )
    .unwrap();
    let (_pub, mut voice) = ds.voice([1., 0., 0.]).unwrap();
    let out = (0..128)
        .map(|i| voice.process(if i == 0 { 1. } else { 0. }).unwrap())
        .collect::<Vec<_>>();
    near(out[66][0], 1.);
    near(out[69][1], 0.25);
    voice.reset();
    for _ in 0..128 {
        assert_eq!(voice.process(0.).unwrap(), [0.; 2]);
    }
}
#[test]
fn listener_rotation_preserves_handedness_and_handles_coincident_sources() {
    let f = [0., 0., -1.];
    let u = [0., 1., 0.];
    assert_eq!(
        listener_direction([1., 0., 0.], [0.; 3], f, u).unwrap(),
        [1., 0., 0.]
    );
    let r = listener_direction([1., 0., 0.], [0.; 3], [1., 0., 0.], u).unwrap();
    near(r[2], -1.);
    assert_eq!(listener_direction([0.; 3], [0.; 3], f, u).unwrap(), f);
    assert!(listener_direction([0.; 3], [0.; 3], f, f).is_err());
}
#[test]
fn invalid_profiles_fail_before_rendering() {
    let mut a = data();
    a.measurements[0].left.clear();
    assert!(
        HrtfDataset::from_bytes(&serde_json::to_vec(&a).unwrap(), 48000, Default::default())
            .is_err()
    );
    let mut a = data();
    a.measurements[1].direction = a.measurements[0].direction;
    assert!(
        HrtfDataset::from_bytes(&serde_json::to_vec(&a).unwrap(), 48000, Default::default())
            .is_err()
    );
    assert!(
        HrtfDataset::from_bytes(
            &serde_json::to_vec(&data()).unwrap(),
            44100,
            Default::default()
        )
        .is_err()
    );
    assert!(HrtfDataset::from_bytes(b"bad", 48000, Default::default()).is_err());
    assert!(
        HrtfOptions {
            partition_frames: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
#[ignore = "requires an explicit runtime SOFA dataset; never bundles measured HRIR data"]
fn sofa_dataset_resamples_and_renders_an_external_measurement() {
    let path = std::env::var("CRYSIRROR_HRTF_SOFA").expect("SOFA path");
    let bytes = std::fs::read(path).unwrap();
    let ds = HrtfDataset::from_bytes(&bytes, 48000, Default::default()).unwrap();
    let (mut publisher, mut stream) = ds.voice([1., 0., 0.]).unwrap();
    let mut energy = [0.; 2];
    for i in 0..stream.tail_frames() + 4096 {
        let y = stream.process(if i == 0 { 1. } else { 0. }).unwrap();
        for ear in 0..2 {
            energy[ear] += y[ear] * y[ear];
        }
    }
    assert!(energy.iter().all(|v| v.is_finite() && *v > 0.));
    publisher
        .publish(ds.prepare_filter([-1., 0., 0.]).unwrap())
        .unwrap();
    println!("metadata={} energy={energy:?}", ds.metadata());
}

#[test]
fn sofa_global_ear_delays_and_receiver_order_match_an_impulse_oracle() {
    for bytes in [
        include_bytes!("fixtures/impulse.sofa").as_slice(),
        include_bytes!("fixtures/impulse-v3.sofa").as_slice(),
        include_bytes!("fixtures/swapped-ears.sofa").as_slice(),
    ] {
        let ds = HrtfDataset::from_bytes(
            bytes,
            48000,
            HrtfOptions {
                ear_order: HrtfEarOrder::ReceiverPositions,
                max_delay_seconds: 0.001,
                ..Default::default()
            },
        )
        .unwrap();
        for direction in [[0., 0., -1.], [1., 0., 0.]] {
            let (_publisher, mut voice) = ds.voice(direction).unwrap();
            let output = (0..128)
                .map(|i| voice.process(if i == 0 { 1. } else { 0. }).unwrap())
                .collect::<Vec<_>>();
            for (i, frame) in output.iter().enumerate() {
                near(frame[0], if i == 67 { 1. } else { 0. });
                near(frame[1], if i == 69 { 0.5 } else { 0. });
            }
        }
    }
    assert!(
        HrtfDataset::from_bytes(
            include_bytes!("fixtures/unsupported-units.sofa"),
            48000,
            Default::default()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("units")
    );
}

#[test]
fn sofa_missing_source_coordinate_type_fails_instead_of_guessing_cartesian() {
    assert!(
        HrtfDataset::from_bytes(
            include_bytes!("fixtures/missing-type.sofa"),
            48000,
            Default::default()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("Type/Units")
    );
}

#[test]
fn explicit_sofa_ear_order_preserves_or_swaps_authored_channels_and_delays() {
    for (order, expected) in [
        (HrtfEarOrder::LeftRight, [(67, 1.), (69, 0.5)]),
        (HrtfEarOrder::RightLeft, [(69, 0.5), (67, 1.)]),
    ] {
        let ds = HrtfDataset::from_bytes(
            include_bytes!("fixtures/impulse.sofa"),
            48000,
            HrtfOptions {
                ear_order: order,
                ..Default::default()
            },
        )
        .unwrap();
        let (_publisher, mut stream) = ds.voice([0., 0., -1.]).unwrap();
        let out = (0..128)
            .map(|i| stream.process(if i == 0 { 1. } else { 0. }).unwrap())
            .collect::<Vec<_>>();
        for ear in 0..2 {
            for (i, frame) in out.iter().enumerate() {
                near(
                    frame[ear],
                    if i == expected[ear].0 {
                        expected[ear].1
                    } else {
                        0.
                    },
                );
            }
        }
    }
}
