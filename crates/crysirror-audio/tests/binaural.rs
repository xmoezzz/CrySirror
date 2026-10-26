// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_acoustics::binaural::*;
use crysirror_audio::{decoder::Clip, mixer::*};
fn stream() -> (FilterPublisher, BinauralVoice) {
    let bytes=serde_json::to_vec(&serde_json::json!({"sample_rate":48000,"measurements":[{"direction":[1,0,0],"left":[1,0.5],"right":[0.25,0.125],"delays_seconds":[0,0.000065]}]})).unwrap();
    HrtfDataset::from_bytes(
        &bytes,
        48000,
        HrtfOptions {
            max_delay_seconds: 0.00007,
            ..Default::default()
        },
    )
    .unwrap()
    .voice([1., 0., 0.])
    .unwrap()
}
fn setup(samples: &[f32]) -> (Mixer, VoiceHandle) {
    let mut mixer = Mixer::new(Default::default()).unwrap();
    let clip = Clip::new(48000, samples.iter().map(|x| [*x, *x]).collect()).unwrap();
    let h = mixer
        .play(
            1,
            clip.source(),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [1., 0., 0.],
                    min_distance: 10.,
                    max_distance: 20.,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    (mixer, h)
}
fn render(m: &mut Mixer, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    m.render(&mut out).unwrap();
    out
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn short_source_keeps_delayed_ears_and_fir_tail_and_bypasses_stereo_pan() {
    let (mut mixer, h) = setup(&[1., -0.5]);
    let (_pub, voice) = stream();
    mixer.set_binaural(1, h, Some(voice)).unwrap();
    let out = render(&mut mixer, 256);
    near(out[64][0], 1.);
    near(out[65][0], 0.);
    near(out[66][0], -0.25);
    near(out[67][1], 0.25);
    near(out[69][1], -0.0625);
    assert!(out[..64].iter().flatten().all(|x| *x == 0.));
    assert!(!mixer.is_playing(1, h));
}
#[test]
fn pending_preparation_freezes_short_voice_and_pause_seek_preserve_contracts() {
    let (mut mixer, h) = setup(&[1.]);
    mixer.pending_binaural(1, h).unwrap();
    assert!(render(&mut mixer, 4096).iter().flatten().all(|x| *x == 0.));
    assert_eq!(mixer.position(1, h).unwrap(), 0.);
    assert!(mixer.is_playing(1, h));
    let (_pub, voice) = stream();
    mixer.set_binaural(1, h, Some(voice)).unwrap();
    render(&mut mixer, 64);
    mixer.pause(1, h, true).unwrap();
    assert!(render(&mut mixer, 100).iter().flatten().all(|x| *x == 0.));
    mixer.pause(1, h, false).unwrap();
    near(render(&mut mixer, 1)[0][0], 1.);
    mixer.seek(1, h, 0).unwrap();
    assert!(render(&mut mixer, 64).iter().flatten().all(|x| *x == 0.));
    near(render(&mut mixer, 1)[0][0], 1.);
    assert!(mixer.set_binaural(2, h, None).is_err());
}
#[test]
fn acoustic_bands_and_buses_apply_to_the_same_binaural_pcm() {
    let (mut mixer, h) = setup(&[1.; 256]);
    let (_pub, voice) = stream();
    mixer.set_binaural(1, h, Some(voice)).unwrap();
    mixer.set_acoustic_gains(1, h, Some([0.; 3])).unwrap();
    assert!(render(&mut mixer, 96).iter().flatten().all(|x| *x == 0.));
    mixer.set_acoustic_gains(1, h, None).unwrap();
    mixer
        .set_bus(
            1,
            Bus::Sfx,
            BusSettings {
                gain: 0.5,
                muted: false,
            },
        )
        .unwrap();
    let out = render(&mut mixer, 128);
    near(out.last().unwrap()[0], 0.75);
    near(out.last().unwrap()[1], 0.1875);
    mixer.set_binaural(1, h, None).unwrap();
    near(render(&mut mixer, 1)[0][0], 0.);
}
