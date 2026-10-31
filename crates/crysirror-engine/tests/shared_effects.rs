// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Public engine APIs without a native VM or graphics/audio device.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{json, Value as Json};
use std::time::{Duration, Instant};
fn app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    let p = d.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"effects","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &d.path().join("state")).unwrap(), "a").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    (d, a)
}
fn call(a: &mut App, class: &'static str, name: &str, args: Json) -> anyhow::Result<Json> {
    let cx = a.context();
    let h = cx.singleton(class)?;
    Ok(cx
        .call(
            h,
            name,
            &args
                .as_array()
                .unwrap()
                .iter()
                .cloned()
                .map(Value::from_json)
                .collect::<Vec<_>>(),
        )?
        .to_json())
}
fn wait(a: &mut App, class: &'static str, method: &str, ticket: Json) -> Json {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let s = call(a, class, method, json!([ticket])).unwrap();
        if s["state"] != "pending" {
            return s;
        }
        assert!(Instant::now() < end, "{s}");
        std::thread::yield_now();
    }
}
fn pixels(a: &App, slot: u32) -> Vec<u8> {
    a.runtime()
        .unwrap()
        .assets
        .surface(ClientId(1), &format!("__surface/{slot}"))
        .unwrap()
        .rgba
        .to_vec()
}
#[test]
fn alpha_clip_pair_is_public_scoped_snapshot_based_and_revision_checked(){
    let (_d,mut a)=app();
    call(&mut a,"Bitmap","create",json!([1,3,1,[18,52,86,128]])).unwrap();
    call(&mut a,"Bitmap","create",json!([2,1,1,[200,50,20,64]])).unwrap();
    let options=json!({"destination":[1,0],"source":[0,0,1,1],"clip":[0,0,3,1],"clear":0});
    let ticket=call(&mut a,"Bitmap","beginAlphaClip",json!([1,2,options])).unwrap();
    a.context().state().unwrap().client=ClientId(2);
    assert!(call(&mut a,"Bitmap","effectStatus",json!([ticket])).is_err());
    a.context().state().unwrap().client=ClientId(1);
    call(&mut a,"Bitmap","fill",json!([2,{"x":0,"y":0,"width":1,"height":1},[0,0,0,255]])).unwrap();
    assert_eq!(wait(&mut a,"Bitmap","effectStatus",ticket)["state"],"ready");
    assert_eq!(pixels(&a,1),[18,52,86,0,18,52,86,32,18,52,86,0],"worker must use admitted source generation");
    let mut invalid=options.clone();invalid["self_source"]=json!(true);
    assert!(call(&mut a,"Bitmap","beginAlphaClip",json!([1,2,invalid])).is_err());
    let ticket=call(&mut a,"Bitmap","beginAlphaClip",json!([1,2,options])).unwrap();
    call(&mut a,"Bitmap","fill",json!([1,{"x":0,"y":0,"width":3,"height":1},[9,10,11,255]])).unwrap();
    let result=wait(&mut a,"Bitmap","effectStatus",ticket);
    assert_eq!(result["state"],"failed");assert!(result["error"].as_str().unwrap().contains("stale"));
    assert_eq!(pixels(&a,1),[9,10,11,255,9,10,11,255,9,10,11,255]);
    assert!(a.typescript_declarations().unwrap().contains("beginAlphaClip("));
}
#[test]
fn shared_image_worker_publication_is_scoped_revision_checked_cancellable_and_restore_safe() {
    let (_d, mut a) = app();
    call(
        &mut a,
        "Bitmap",
        "create",
        json!([1, 2, 1, [128, 0, 0, 255]]),
    )
    .unwrap();
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    assert!(call(
        &mut a,
        "Bitmap",
        "beginEffect",
        json!([1,[{"kind":"invert"}]])
    )
    .is_err());
    assert_eq!(pixels(&a, 1), [128, 0, 0, 255, 128, 0, 0, 255]);
    drop(permits);
    let ticket = call(
        &mut a,
        "Bitmap",
        "beginEffect",
        json!([1,[{"kind":"gamma","gamma":[2,2,2],"floor":[0,0,0],"ceiling":[255,255,255]}]]),
    )
    .unwrap();
    a.context().state().unwrap().client = ClientId(2);
    assert!(call(&mut a, "Bitmap", "effectStatus", json!([ticket])).is_err());
    a.context().state().unwrap().client = ClientId(1);
    let s = wait(&mut a, "Bitmap", "effectStatus", ticket.clone());
    assert_eq!(s["state"], "ready", "{s}");
    assert_eq!(pixels(&a, 1), [181, 0, 0, 255, 181, 0, 0, 255]);
    call(&mut a, "Bitmap", "cancelEffect", json!([ticket])).unwrap();
    let t = call(
        &mut a,
        "Bitmap",
        "beginEffect",
        json!([1,[{"kind":"invert"}]]),
    )
    .unwrap();
    call(
        &mut a,
        "Bitmap",
        "fill",
        json!([1,{"x":0,"y":0,"width":2,"height":1},[1,2,3,255]]),
    )
    .unwrap();
    let s = wait(&mut a, "Bitmap", "effectStatus", t.clone());
    assert_eq!(s["state"], "failed");
    assert!(s["error"].as_str().unwrap().contains("changed"));
    assert_eq!(pixels(&a, 1), [1, 2, 3, 255, 1, 2, 3, 255]);
    call(&mut a, "Bitmap", "cancelEffect", json!([t])).unwrap();
    let t = call(
        &mut a,
        "Bitmap",
        "beginEffect",
        json!([1,[{"kind":"invert"}]]),
    )
    .unwrap();
    call(&mut a, "Bitmap", "cancelEffect", json!([t])).unwrap();
    assert!(call(&mut a, "Bitmap", "effectStatus", json!([t])).is_err());
    let t = call(
        &mut a,
        "Bitmap",
        "beginEffect",
        json!([1,[{"kind":"invert"}]]),
    )
    .unwrap();
    let snapshot = a.runtime().unwrap().snapshot().unwrap();
    a.runtime_mut().unwrap().restore(&snapshot).unwrap();
    assert!(call(&mut a, "Bitmap", "effectStatus", json!([t])).is_err());
}
#[test]
fn two_source_image_transition_uses_snapshot_inputs_and_shared_bitmap_pixels() {
    let (_d, mut a) = app();
    call(
        &mut a,
        "Bitmap",
        "create",
        json!([1, 2, 1, [255, 0, 0, 255]]),
    )
    .unwrap();
    call(
        &mut a,
        "Bitmap",
        "create",
        json!([2, 2, 1, [0, 0, 255, 255]]),
    )
    .unwrap();
    let t = call(
        &mut a,
        "Bitmap",
        "beginTransition",
        json!([1,2,{"progress":0.5,"softness":0,"blur":0}]),
    )
    .unwrap();
    call(
        &mut a,
        "Bitmap",
        "fill",
        json!([2,{"x":0,"y":0,"width":2,"height":1},[0,255,0,255]]),
    )
    .unwrap();
    assert_eq!(wait(&mut a, "Bitmap", "effectStatus", t)["state"], "ready");
    assert_eq!(pixels(&a, 1), [128, 0, 128, 255, 128, 0, 128, 255]);
}
#[test]
fn synthetic_audio_assets_prepare_and_play_through_public_worker_mixer_without_uk2() {
    let (d, mut a) = app();
    let score = json!({"schema":1,"output_rate":48000,"frames":4800,"writes":[{"frame":0,"port":0,"register":0,"value":71},{"frame":0,"port":0,"register":1,"value":0},{"frame":0,"port":0,"register":7,"value":62},{"frame":0,"port":0,"register":8,"value":15}]});
    std::fs::write(
        d.path().join("assets/tone.opna"),
        serde_json::to_vec(&score).unwrap(),
    )
    .unwrap();
    let t = call(
        &mut a,
        "Audio",
        "beginPrepareSource",
        json!(["tone.opna", {}]),
    )
    .unwrap();
    let s = wait(&mut a, "Audio", "sourcePreparation", t.clone());
    assert_eq!(s["state"], "ready", "{s}");
    assert_eq!(s["result"]["info"]["source_frames"], 4800);
    let resource = s["result"]["resource"].clone();
    call(&mut a, "Audio", "cancelSourcePreparation", json!([t])).unwrap();
    a.context().state().unwrap().client = ClientId(2);
    assert!(call(&mut a, "Audio", "sourceInfo", json!([resource])).is_err());
    a.context().state().unwrap().client = ClientId(1);
    let t = call(
        &mut a,
        "Audio",
        "beginPlaySource",
        json!([resource,{"bus":"music"}]),
    )
    .unwrap();
    let s = wait(&mut a, "Audio", "playbackStatus", t.clone());
    assert_eq!(s["state"], "ready", "{s}");
    call(&mut a, "Audio", "cancelPlaybackJob", json!([t])).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        a.frame(1, InputFrame::default()).unwrap();
        let pcm = a.context().state().unwrap().mixer_audio.take_output();
        if pcm.iter().any(|v| v[0].abs() > 0.01) {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    call(&mut a, "Audio", "releaseSource", json!([resource])).unwrap();
}
