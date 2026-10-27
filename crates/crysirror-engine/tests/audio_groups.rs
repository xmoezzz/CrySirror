// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Public classes and native/media services share the same group capabilities.
use crysirror_core::{
    service::{
        AudioMixGroupSettings, AudioParameters, AudioRequest, HostServices, InputFrame, PcmAudio,
    },
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let mut b = b"RIFF".to_vec();
    b.extend(16036u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(16000u32.to_le_bytes());
    for _ in 0..8000 {
        b.extend(16384i16.to_le_bytes());
    }
    std::fs::write(dir.path().join("assets/tone.wav"), b).unwrap();
    std::fs::write(dir.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"groups","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&dir.path().join("project.json"), &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn invoke(app: &mut App, name: &str, args: &[Value]) -> anyhow::Result<Value> {
    let cx = app.context();
    let audio = cx.singleton("Audio")?;
    cx.call(audio, name, args)
}
fn call(app: &mut App, name: &str, args: &[Value]) -> Value {
    invoke(app, name, args).unwrap()
}
#[test]
fn groups_route_both_mixers_preserve_partial_parents_and_reject_invalid_changes_atomically() {
    let (_dir, mut app) = app();
    call(
        &mut app,
        "setMixGroup",
        &[v(json!("game")), v(json!({"gain":0.5}))],
    );
    call(
        &mut app,
        "setMixGroup",
        &[v(json!("hero")), v(json!({"parent":"game","gain":0.5}))],
    );
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","mix_group":"hero","looping":true}),
        )],
    );
    app.runtime_mut()
        .unwrap()
        .host
        .submit_audio(
            ClientId(1),
            AudioRequest::PlayPcm {
                channel: 9,
                source: PcmAudio::new(48000, vec![[0.5; 2]; 1000]).unwrap(),
                parameters: AudioParameters {
                    mix_group: Some("hero".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    app.frame(1, InputFrame::default()).unwrap();
    let pcm = app.context().state().unwrap().mixer_audio.take_output();
    assert!((pcm[0][0] - 0.25).abs() < 1e-6);
    let before = call(&mut app, "mixGroup", &[v(json!("game"))]).to_json();
    for settings in [
        v(json!({"parent":"hero"})),
        v(json!({"gain":null})),
        v(json!({"muted":null})),
        Value::Map(std::collections::BTreeMap::from([(
            "gain".into(),
            Value::Float(f64::NAN),
        )])),
    ] {
        assert!(invoke(&mut app, "setMixGroup", &[v(json!("game")), settings]).is_err());
        assert_eq!(
            call(&mut app, "mixGroup", &[v(json!("game"))]).to_json(),
            before
        );
        assert_eq!(
            call(&mut app, "nativeMixGroup", &[v(json!("game"))]).to_json(),
            before
        );
    }
    call(
        &mut app,
        "setMixGroup",
        &[v(json!("hero")), v(json!({"gain":0.25})), Value::Null],
    );
    assert_eq!(
        call(&mut app, "mixGroup", &[v(json!("hero"))]).to_json()["settings"]["parent"],
        "game"
    );
    call(
        &mut app,
        "setMixGroup",
        &[v(json!("hero")), v(json!({"parent":null}))],
    );
    assert_eq!(
        call(&mut app, "mixGroup", &[v(json!("hero"))]).to_json()["settings"]["parent"],
        json!(null)
    );
    call(
        &mut app,
        "setMixGroup",
        &[v(json!("hero")), v(json!({"parent":"game"}))],
    );
    assert!(invoke(&mut app, "removeMixGroup", &[v(json!("hero"))]).is_err());
    app.context().state().unwrap().client = ClientId(2);
    assert_eq!(
        call(&mut app, "mixGroup", &[v(json!("hero"))]).to_json(),
        json!(null)
    );
    assert!(invoke(&mut app, "removeMixGroup", &[v(json!("hero"))]).is_err());
    assert!(invoke(&mut app, "setVoiceMixGroup", &[h.clone(), Value::Null]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    let at = call(&mut app, "playbackInfo", &[h.clone()]).to_json();
    call(&mut app, "setVoiceMixGroup", &[h]);
    assert_eq!(
        call(&mut app, "nativeChannelInfo", &[Value::Int(9)]).to_json()["age_frames"],
        48
    );
    assert_eq!(at["age_frames"], 48);
    app.runtime_mut()
        .unwrap()
        .host
        .submit_audio(
            ClientId(1),
            AudioRequest::Stop {
                channel: 9,
                fade_ms: 0.,
            },
        )
        .unwrap();
    call(&mut app, "removeMixGroup", &[v(json!("hero"))]);
    call(&mut app, "removeMixGroup", &[v(json!("game"))]);
}
#[test]
fn second_mixer_capacity_failure_cannot_publish_the_first_and_native_only_groups_can_be_removed() {
    let (_dir, mut app) = app();
    for i in 0..64 {
        app.runtime_mut()
            .unwrap()
            .host
            .channel_audio
            .mixer
            .set_mix_group(1, &format!("g{i}"), AudioMixGroupSettings::default(), 0.)
            .unwrap();
    }
    assert!(
        invoke(
            &mut app,
            "setMixGroup",
            &[v(json!("overflow")), v(json!({}))]
        )
        .is_err()
    );
    assert_eq!(
        call(&mut app, "mixGroup", &[v(json!("overflow"))]).to_json(),
        json!(null)
    );
    assert_eq!(
        call(&mut app, "nativeMixGroup", &[v(json!("overflow"))]).to_json(),
        json!(null)
    );
    call(&mut app, "removeMixGroup", &[v(json!("g0"))]);
    assert_eq!(
        call(&mut app, "nativeMixGroup", &[v(json!("g0"))]).to_json(),
        json!(null)
    );
    call(&mut app, "setMixGroup", &[v(json!("shared")), v(json!({}))]);
    assert_eq!(
        call(&mut app, "mixGroup", &[v(json!("shared"))]).to_json()["gain"],
        1.
    );
}
