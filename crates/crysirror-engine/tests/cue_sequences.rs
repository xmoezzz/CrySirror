// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use anyhow::Result;
use crysirror_core::{
    service::{HostServices, InputFrame},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
use std::time::{Duration, Instant};
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = dir.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"cue","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &dir.path().join("state")).unwrap(), "a").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    (dir, a)
}
fn call(a: &mut App, name: &str, args: Json) -> Result<Json> {
    let cx = a.context();
    let audio = cx.singleton("Audio")?;
    let args = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    Ok(cx.call(audio, name, &args)?.to_json())
}
fn create(a: &mut App) -> Json {
    call(a,"createCueSequence",json!([{"clock":"real","include_duration":true,"lines":[{"key":"tone","min_ms":10,"max_ms":10}]},7])).unwrap()
}
#[test]
fn public_clocks_are_explicit_receipts_are_idempotent_and_invalid_updates_are_atomic() {
    let (_d, mut a) = app();
    let h = create(&mut a);
    a.frame(100, InputFrame::default()).unwrap(); // App time is not the caller's program clock.
    let before = call(&mut a, "cueSequence", json!([h])).unwrap();
    assert_eq!(before["elapsed_ms"], 0);
    let e = call(&mut a, "pollCueSequence", json!([h, {}])).unwrap();
    call(&mut a, "advanceCueSequence", json!([h, 60000, 60000])).unwrap();
    assert_eq!(
        call(&mut a, "pollCueSequence", json!([h,{"seed":999}])).unwrap(),
        e
    );
    assert!(call(&mut a, "pollCueSequence", json!([h,{"unknown":1}])).is_err());
    assert!(call(&mut a, "advanceCueSequence", json!([h, -1, 0])).is_err());
    assert!(call(&mut a, "completeCueSequence", json!([h, 999, 0])).is_err());
    assert_eq!(
        call(&mut a, "cueSequence", json!([h])).unwrap()["rng_state"],
        7
    );
    call(&mut a, "completeCueSequence", json!([h, e["serial"], 3])).unwrap();
    call(&mut a, "advanceCueSequence", json!([h, 100, 12])).unwrap();
    assert_eq!(
        call(&mut a, "cueSequence", json!([h])).unwrap()["active"],
        true
    );
    call(&mut a, "advanceCueSequence", json!([h, 0, 1])).unwrap();
    assert_eq!(
        call(&mut a, "cueSequence", json!([h])).unwrap()["active"],
        false
    );
    assert!(
        call(
            &mut a,
            "createCueSequence",
            json!([{"lines":[{"key":"x","extra":1}]},1])
        )
        .is_err()
    );
    assert!(call(&mut a, "createCueSequence", json!([{"random":"unknown"},1])).is_err());
}
#[test]
fn host_and_script_facades_share_scoped_state_and_restore_invalidates_generations() {
    let (_d, mut a) = app();
    let h = create(&mut a);
    let native = crysirror_core::service::AudioSequenceHandle {
        client: 1,
        generation: h["generation"].as_u64().unwrap(),
    };
    assert!(
        a.runtime()
            .unwrap()
            .host
            .audio_sequence_info(ClientId(1), native)
            .unwrap()
            .active
    );
    assert!(
        a.runtime()
            .unwrap()
            .host
            .audio_sequence_info(ClientId(99), native)
            .is_err()
    );
    a.context().state().unwrap().client = ClientId(2);
    assert!(call(&mut a, "cueSequence", json!([h])).is_err());
    assert!(call(&mut a, "releaseCueSequence", json!([h])).is_err());
    a.context().state().unwrap().client = ClientId(1);
    let save = a.runtime().unwrap().snapshot().unwrap();
    a.runtime_mut().unwrap().restore(&save).unwrap();
    assert!(call(&mut a, "cueSequence", json!([h])).is_err());
    let new = create(&mut a);
    assert_ne!(h, new);
    call(&mut a, "releaseCueSequence", json!([new])).unwrap();
    assert!(call(&mut a, "cueSequence", json!([new])).is_err());
}
#[test]
fn pending_dsp_allows_only_frozen_observation_and_never_advances_the_cue_clock() {
    let (_d, mut a) = app();
    let h = create(&mut a);
    let e = call(&mut a, "pollCueSequence", json!([h, {}])).unwrap();
    call(&mut a, "completeCueSequence", json!([h, e["serial"], 10])).unwrap();
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    a.begin_frame(16, InputFrame::default()).unwrap();
    assert!(a.frame_pending());
    let before = call(&mut a, "cueSequence", json!([h])).unwrap();
    for _ in 0..3 {
        assert!(!a.poll_frame().unwrap());
        assert_eq!(call(&mut a, "cueSequence", json!([h])).unwrap(), before);
        for (method, args) in [
            ("advanceCueSequence", json!([h, 1, 1])),
            ("pollCueSequence", json!([h, {}])),
            ("releaseCueSequence", json!([h])),
        ] {
            assert!(call(&mut a, method, args).is_err());
        }
    }
    drop(permits);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !a.poll_frame().unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(call(&mut a, "cueSequence", json!([h])).unwrap(), before);
    call(&mut a, "advanceCueSequence", json!([h, 0, 20])).unwrap();
    assert_eq!(
        call(&mut a, "cueSequence", json!([h])).unwrap()["active"],
        false
    );
}

#[test]
fn portable_public_cursor_roundtrip_is_validated_before_publication_and_has_a_new_identity() {
    let (_d, mut a) = app();
    let spec = json!({"mode":"loop","lines":[{"key":"cue","min_ms":10,"max_ms":10}]});
    let h = call(&mut a, "createCueSequence", json!([spec, 17])).unwrap();
    let e = call(&mut a, "pollCueSequence", json!([h, {}])).unwrap();
    assert!(call(&mut a, "captureCueSequence", json!([h])).is_err());
    call(&mut a, "completeCueSequence", json!([h, e["serial"], 0])).unwrap();
    call(&mut a, "advanceCueSequence", json!([h, 7, 7])).unwrap();
    let cursor = call(&mut a, "captureCueSequence", json!([h])).unwrap();
    let before = call(&mut a, "cueSequence", json!([h])).unwrap();
    let mut invalid = cursor.clone();
    invalid["last_index"] = json!(999);
    assert!(call(&mut a, "restoreCueSequence", json!([spec, invalid])).is_err());
    let restored = call(&mut a, "restoreCueSequence", json!([spec, cursor])).unwrap();
    assert_ne!(restored, h);
    assert_eq!(
        call(&mut a, "cueSequence", json!([restored])).unwrap(),
        before
    );
    call(&mut a, "advanceCueSequence", json!([restored, 3, 3])).unwrap();
    let next = call(&mut a, "pollCueSequence", json!([restored, {}])).unwrap();
    assert_eq!(next["serial"], 2);
    assert!(
        call(
            &mut a,
            "completeCueSequence",
            json!([restored, e["serial"], 0])
        )
        .is_err()
    );
    assert_eq!(call(&mut a, "cueSequence", json!([h])).unwrap(), before);
}
