// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::voice_compression::*;
#[test]
fn pcm_profile_matches_pinned_reference_golden_vectors() {
    let input = (0..8000usize)
        .map(|i| (((i * 7919 + 12345) % 60001) as i32 - 30000) as i16)
        .collect::<Vec<_>>();
    for (percent, n, hash) in [
        (125, 6526, 0x32ed7b7901ceab1au64),
        (150, 5754, 0xcb55646dcdd69270),
        (200, 4130, 0x7422c527b9b4de3d),
        (300, 2599, 0x88033d2b78a75beb),
    ] {
        let out = siglus_jitan(&input, 1, 8000, percent).unwrap();
        let actual = out
            .samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
        assert_eq!(out.samples.len(), n, "percent={percent}");
        assert_eq!(actual, hash, "percent={percent}");
    }
}

#[test]
fn sampled_profile_matches_784_pinned_reference_cases_and_cancellation_is_visible() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/jitan_reference.json")).unwrap();
    for case in golden["cases"].as_array().unwrap() {
        let c = case
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect::<Vec<_>>();
        let (rate, ms, channels, pattern, percent) =
            (c[0] as u32, c[1] as usize, c[2] as u16, c[3], c[4] as u16);
        let input = (0..rate as usize * ms / 1000 * channels as usize)
            .map(|i| match pattern {
                0 => 32767,
                1 => -32768,
                2 => {
                    if i % 80 < 40 {
                        32767
                    } else {
                        -32768
                    }
                }
                3 => (((i * 7919 + 12345) % 65536) as i32 - 32768) as i16,
                4 => (((i * 431) % 60001) as i32 - 30000) as i16,
                5 => {
                    if i % 99 == 0 {
                        32767
                    } else {
                        0
                    }
                }
                _ => ((i as f64 * 0.11).sin() * 30000.) as i16,
            })
            .collect::<Vec<_>>();
        let output = siglus_jitan(&input, channels, rate, percent).unwrap();
        let hash = output
            .samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
        assert_eq!(
            (output.samples.len() as u64, hash),
            (c[5], c[6]),
            "case={case}"
        );
        assert_eq!(output.rate, rate);
        assert_eq!(output.channels, if percent == 100 { channels } else { 1 });
    }
    let cancel = crysirror_tasks::Cancellation::default();
    cancel.cancel();
    assert!(siglus_jitan_cancellable(&[0; 8000], 1, 8000, 200, &cancel).is_err());
    assert!(siglus_jitan(&[0; 8000], 1, 8000, 99).is_err());
    assert!(siglus_jitan(&[0; 3], 2, 8000, 200).is_err());
}
