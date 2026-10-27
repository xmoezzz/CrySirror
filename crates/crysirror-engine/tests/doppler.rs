// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Public engine pose capture and actual shared mixer transport, no devices/VM.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
use std::time::{Duration, Instant};
fn v(x: Json) -> Value {
    Value::from_json(x)
}
fn app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("assets/a")).unwrap();
    let size = 32000u32;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for _ in 0..16000 {
        b.extend(16384i16.to_le_bytes());
    }
    std::fs::write(d.path().join("assets/a/tone.wav"), b).unwrap();
    let p = d.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"Doppler","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &d.path().join("state")).unwrap(), "a").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    camera(&mut a, 0.);
    (d, a)
}
fn call(a: &mut App, name: &str, args: &[Value]) -> Value {
    let cx = a.context();
    let audio = cx.singleton("Audio").unwrap();
    cx.call(audio, name, args).unwrap()
}
fn camera(a: &mut App, z: f32) {
    let cx = a.context();
    let w = cx.singleton("World").unwrap();
    cx.call(
        w,
        "setCamera",
        &[v(json!({"eye":[0,0,z],"target":[0,0,z-1.]}))],
    )
    .unwrap();
}
fn spawn(a: &mut App, name: &str, z: f32) -> Value {
    let cx = a.context();
    let w = cx.singleton("World").unwrap();
    cx.call(w, "spawn", &[v(json!({"name":name,"position":[0,0,z]}))])
        .unwrap()
}
fn position(a: &mut App, e: &Value, z: f32) {
    a.context()
        .set(e.as_object().unwrap(), "position", Value::vec3([0., 0., z]))
        .unwrap();
}
fn play(a: &mut App, follow: Option<Value>) -> Value {
    let mut spec = v(
        json!({"asset":"tone.wav","looping":true,"spatial":{"min_distance":100,"max_distance":1000,"doppler":{"speed_of_sound":200,"smoothing_ms":0}}}),
    );
    if let Value::Map(ref mut m) = spec {
        if let Some(e) = follow {
            m.insert("follow".into(), e);
        }
    }
    call(a, "playSfx", &[spec])
}
fn frame(a: &mut App, ms: u64) {
    a.frame(ms, InputFrame::default()).unwrap();
    a.context().state().unwrap().mixer_audio.take_output();
}
fn info(a: &mut App, h: &Value) -> Json {
    call(a, "dopplerVoice", &[h.clone()]).to_json()
}
fn near(x: &Json, want: f64) {
    let actual = x.as_f64().unwrap();
    assert!((actual - want).abs() < 1e-4, "{actual} != {want}");
}
#[test]
fn entity_parent_and_camera_motion_drive_shared_doppler_and_report_teleport_resets() {
    let (_d, mut a) = app();
    let parent = spawn(&mut a, "parent", 0.);
    let emitter = spawn(&mut a, "source", -10.);
    a.context()
        .call(emitter.as_object().unwrap(), "setParent", &[parent.clone()])
        .unwrap();
    let h = play(&mut a, Some(emitter));
    frame(&mut a, 16);
    near(&info(&mut a, &h)["ratio"], 1.);
    position(&mut a, &parent, 1.6);
    frame(&mut a, 16);
    let i = info(&mut a, &h);
    near(&i["ratio"], 2.);
    near(&i["source_velocity"][2], 100.);
    assert_eq!(i["motion"]["source"]["status"], "inferred");
    near(
        &call(&mut a, "playbackInfo", &[h.clone()]).to_json()["position_frames"],
        384.,
    );
    position(&mut a, &parent, 3.2);
    camera(&mut a, 1.6);
    frame(&mut a, 16);
    near(&info(&mut a, &h)["ratio"], 1.);
    position(&mut a, &parent, 100.);
    frame(&mut a, 16);
    let i = info(&mut a, &h);
    near(&i["ratio"], 1.);
    assert_eq!(i["motion"]["source"]["status"], "discontinuity");
}
#[test]
fn explicit_velocities_override_inference_and_motion_policy_resets_without_replaying() {
    let (_d, mut a) = app();
    let h = play(&mut a, None);
    call(
        &mut a,
        "setVoice",
        &[
            h.clone(),
            v(
                json!({"spatial":{"position":[0,0,-10],"velocity":[0,0,50],"doppler":{"speed_of_sound":100,"smoothing_ms":0},"min_distance":100,"max_distance":1000}}),
            ),
        ],
    );
    call(
        &mut a,
        "setListener",
        &[v(
            json!({"position":[0,0,0],"forward":[0,0,-1],"up":[0,1,0],"velocity":[0,0,50]}),
        )],
    );
    frame(&mut a, 16);
    let i = info(&mut a, &h);
    near(&i["ratio"], 1.);
    assert_eq!(i["motion"]["source"]["status"], "explicit");
    call(
        &mut a,
        "setMotionSettings",
        &[v(json!({"teleport_distance":10,"max_interval_ms":100}))],
    );
    call(&mut a, "resetMotion", &[]);
    frame(&mut a, 16);
    assert_eq!(
        call(&mut a, "motionSettings", &[]).to_json()["teleport_distance"],
        10.
    );
    near(&info(&mut a, &h)["ratio"], 1.);
    let before = call(&mut a, "playbackInfo", &[h.clone()]).to_json()["age_frames"]
        .as_u64()
        .unwrap();
    call(&mut a, "setListener", &[Value::Null]);
    frame(&mut a, 16);
    near(&info(&mut a, &h)["ratio"], 2.);
    assert!(
        call(&mut a, "playbackInfo", &[h.clone()]).to_json()["age_frames"]
            .as_u64()
            .unwrap()
            > before
    );
    let snapshot = a.runtime().unwrap().snapshot().unwrap();
    a.runtime_mut().unwrap().restore(&snapshot).unwrap();
    frame(&mut a, 0);
    assert_eq!(
        call(&mut a, "motionSettings", &[]).to_json()["teleport_distance"],
        10.
    );
    let cx = a.context();
    let audio = cx.singleton("Audio").unwrap();
    assert!(cx.call(audio, "dopplerVoice", &[h]).is_err());
}
#[test]
fn worker_frame_freezes_observation_and_does_not_capture_motion_twice_under_backpressure() {
    let (_d, mut a) = app();
    let emitter = spawn(&mut a, "source", -10.);
    let h = play(&mut a, Some(emitter.clone()));
    frame(&mut a, 16);
    position(&mut a, &emitter, -8.4);
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits: Vec<_> = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect();
    a.begin_frame(16, InputFrame::default()).unwrap();
    assert!(a.frame_pending());
    let expected = info(&mut a, &h);
    near(&expected["target"]["ratio"], 2.);
    near(&expected["ratio"], 1.);
    for _ in 0..3 {
        assert!(!a.poll_frame().unwrap());
        assert_eq!(info(&mut a, &h), expected);
    }
    let cx = a.context();
    let audio = cx.singleton("Audio").unwrap();
    assert!(cx.call(audio, "resetMotion", &[]).is_err());
    drop(permits);
    let end = Instant::now() + Duration::from_secs(5);
    while !a.poll_frame().unwrap() {
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    near(&info(&mut a, &h)["ratio"], 2.);
    near(
        &call(&mut a, "playbackInfo", &[h]).to_json()["age_frames"],
        1536.,
    );
}
#[test]
fn invalid_settings_foreign_handles_and_gap_or_zero_time_pose_changes_are_visible() {
    let (_d, mut a) = app();
    let emitter = spawn(&mut a, "source", -10.);
    let h = play(&mut a, Some(emitter.clone()));
    frame(&mut a, 16);
    for settings in [
        json!({"max_interval_ms":0}),
        json!({"teleport_distance":0}),
        json!({"unknown":true}),
    ] {
        let cx = a.context();
        let audio = cx.singleton("Audio").unwrap();
        assert!(cx.call(audio, "setMotionSettings", &[v(settings)]).is_err());
    }
    assert_eq!(
        call(&mut a, "motionSettings", &[]).to_json()["teleport_distance"],
        50.
    );
    position(&mut a, &emitter, -8.);
    frame(&mut a, 0);
    assert_eq!(
        info(&mut a, &h)["motion"]["source"]["status"],
        "discontinuity"
    );
    position(&mut a, &emitter, -7.);
    frame(&mut a, 500);
    assert_eq!(info(&mut a, &h)["motion"]["source"]["status"], "gap");
    near(&info(&mut a, &h)["target"]["ratio"], 1.);
    let mut foreign = h.to_json();
    foreign["client"] = json!(2);
    let cx = a.context();
    let audio = cx.singleton("Audio").unwrap();
    assert!(cx.call(audio, "dopplerVoice", &[v(foreign)]).is_err());
}
