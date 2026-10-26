// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine HRTF resource/task ownership and actual binaural PCM.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn wav(value: i16) -> Vec<u8> {
    let size = 32000u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
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
        b.extend(value.to_le_bytes())
    }
    b
}
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    for (name, amplitude) in [("a", 16384), ("b", 8192)] {
        std::fs::create_dir_all(dir.path().join(format!("assets/{name}"))).unwrap();
        std::fs::write(
            dir.path().join(format!("assets/{name}/tone.wav")),
            wav(amplitude),
        )
        .unwrap();
    }
    let project = json!({"schema":2,"name":"audio","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]});
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&path, &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn call(app: &mut App, name: &str, args: &[Value]) -> Value {
    let cx = app.context();
    let audio = cx.singleton("Audio").unwrap();
    cx.call(audio, name, args).unwrap()
}
fn output(app: &mut App, ms: u64) -> Vec<[f32; 2]> {
    app.frame(ms, InputFrame::default()).unwrap();
    app.context().state().unwrap().mixer_audio.take_output()
}
use crysirror_tasks::{Task, TaskPool};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
struct Gate {
    shared: Arc<(Mutex<bool>, Condvar)>,
    jobs: Vec<Task<()>>,
}
impl Gate {
    fn block(pool: TaskPool) -> Self {
        let shared = Arc::new((Mutex::new(false), Condvar::new()));
        let mut jobs = vec![];
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..pool.stats().workers {
            let gate = shared.clone();
            let tx = tx.clone();
            jobs.push(
                pool.spawn(move |_| {
                    tx.send(())?;
                    let (lock, wake) = &*gate;
                    let released = lock.lock().unwrap();
                    let (released, _) = wake
                        .wait_timeout_while(released, Duration::from_secs(10), |ready| !*ready)
                        .unwrap();
                    anyhow::ensure!(*released, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..jobs.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self { shared, jobs }
    }
    fn release(&self) {
        let (lock, wake) = &*self.shared;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
    fn finish(mut self) {
        self.release();
        for job in self.jobs.drain(..) {
            job.wait().unwrap();
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
fn invoke(app: &mut App, name: &str, args: &[Value]) -> anyhow::Result<Value> {
    let cx = app.context();
    let audio = cx.singleton("Audio")?;
    cx.call(audio, name, args)
}
fn wait(app: &mut App, ticket: &Value) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = call(app, "hrtfStatus", &[ticket.clone()]).to_json();
        if state["state"] != "pending" {
            return state;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::yield_now();
    }
}
fn asset(dir: &tempfile::TempDir, client: &str, name: &str, data: serde_json::Value) {
    std::fs::write(
        dir.path().join(format!("assets/{client}/{name}")),
        serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
}
fn profile() -> serde_json::Value {
    json!({"sample_rate":48000,"measurements":[{"direction":[1,0,0],"left":[0.25],"right":[1]},{"direction":[-1,0,0],"left":[1],"right":[0.25]}]})
}
fn prepare(app: &mut App, name: &str) -> Value {
    let ticket = call(
        app,
        "beginHrtf",
        &[Value::Str(name.into()), v(json!({"max_delay_seconds":0}))],
    );
    let status = wait(app, &ticket);
    assert_eq!(status["state"], "ready", "{status}");
    ticket
}
fn wait_voice(app: &mut App, h: &Value, direction: serde_json::Value) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        output(app, 0);
        let status = call(app, "hrtfVoice", &[h.clone()]).to_json();
        assert!(status["result"]["error"].is_null(), "{status}");
        if status["result"]["ready"] == true && status["result"]["published_direction"] == direction
        {
            return;
        }
        assert!(Instant::now() < end, "{status}");
        std::thread::yield_now();
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}
#[test]
fn profile_preparation_is_scoped_and_cancelable_and_bad_replacements_keep_old_data() {
    let (dir, mut app) = app();
    asset(&dir, "a", "ears.json", profile());
    asset(&dir, "a", "bad.json", json!({"bad":true}));
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let ticket = call(
        &mut app,
        "beginHrtf",
        &[Value::Str("ears.json".into()), v(json!({}))],
    );
    assert_eq!(
        call(&mut app, "hrtfStatus", &[ticket.clone()]).to_json()["state"],
        "pending"
    );
    output(&mut app, 16);
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "hrtfStatus", &[ticket.clone()]).is_err());
    assert!(invoke(&mut app, "cancelHrtfJob", &[ticket.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    gate.finish();
    assert_eq!(wait(&mut app, &ticket)["state"], "ready");
    call(&mut app, "cancelHrtfJob", &[ticket.clone()]);
    assert_eq!(
        call(&mut app, "hrtfStats", &[]).to_json()["dataset"]["revision"],
        ticket.to_json()
    );
    let bad = call(
        &mut app,
        "beginHrtf",
        &[Value::Str("bad.json".into()), v(json!({}))],
    );
    assert_eq!(wait(&mut app, &bad)["state"], "failed");
    assert_eq!(
        call(&mut app, "hrtfStats", &[]).to_json()["dataset"]["revision"],
        ticket.to_json()
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let canceled = call(
        &mut app,
        "beginHrtf",
        &[Value::Str("ears.json".into()), v(json!({}))],
    );
    call(&mut app, "cancelHrtfJob", &[canceled.clone()]);
    gate.finish();
    assert!(invoke(&mut app, "hrtfStatus", &[canceled]).is_err());
    assert!(
        invoke(
            &mut app,
            "beginHrtf",
            &[Value::Str("../ears.json".into()), v(json!({}))]
        )
        .is_err()
    );
}
#[test]
fn automatic_hrtf_tracks_listener_orientation_and_clear_restores_engine_pan() {
    let (dir, mut app) = app();
    asset(&dir, "a", "ears.json", profile());
    let profile = prepare(&mut app, "ears.json");
    call(
        &mut app,
        "setListener",
        &[v(
            json!({"position":[0,0,0],"forward":[0,0,-1],"up":[0,1,0]}),
        )],
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"spatial":{"position":[1,0,0],"min_distance":10,"max_distance":20},"binaural":true}),
        )],
    );
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    let handle = serde_json::from_value(h.to_json()).unwrap();
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .position(1, handle)
            .unwrap(),
        0.
    );
    gate.finish();
    wait_voice(&mut app, &h, json!([1., 0., 0.]));
    let out = output(&mut app, 16);
    near(out.last().unwrap()[0], 0.125);
    near(out.last().unwrap()[1], 0.5);
    call(&mut app, "setVoiceHrtf", &[h.clone(), Value::Bool(true)]);
    near(output(&mut app, 16).last().unwrap()[0], 0.125); // repeated enable cannot freeze playback
    call(
        &mut app,
        "setListener",
        &[v(
            json!({"position":[0,0,0],"forward":[0,0,1],"up":[0,1,0]}),
        )],
    );
    wait_voice(&mut app, &h, json!([-1., 0., 0.]));
    let out = output(&mut app, 48);
    near(out.last().unwrap()[0], 0.5);
    near(out.last().unwrap()[1], 0.125);
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "hrtfVoice", &[h.clone()]).is_err());
    assert!(invoke(&mut app, "setVoiceHrtf", &[h.clone(), Value::Bool(false)]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "clearHrtf", &[]);
    let out = output(&mut app, 16);
    near(out.last().unwrap()[1], 0.);
    assert!(out.last().unwrap()[0] > 0.45);
    call(&mut app, "cancelHrtfJob", &[profile]);
    assert_eq!(
        call(&mut app, "hrtfStats", &[]).to_json()["host_reserved_bytes"],
        0
    );
}
#[test]
fn restore_invalidates_derived_profiles_voice_jobs_and_old_tickets() {
    let (dir, mut app) = app();
    asset(&dir, "a", "ears.json", profile());
    let ticket = prepare(&mut app, "ears.json");
    let save = app.runtime().unwrap().snapshot().unwrap();
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let h = call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","spatial":{},"binaural":true}))],
    );
    output(&mut app, 0);
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&save)
        .unwrap();
    assert!(invoke(&mut app, "hrtfStatus", &[ticket]).is_err());
    assert!(call(&mut app, "hrtfStats", &[]).to_json()["dataset"].is_null());
    gate.finish();
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    assert!(
        !call(&mut app, "voice", &[h])
            .field("playing")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    assert!(
        invoke(
            &mut app,
            "playSfx",
            &[v(json!({"asset":"tone.wav","spatial":{},"binaural":true}))]
        )
        .is_err()
    );
}
#[test]
fn profile_result_capacity_and_voice_admission_are_bounded() {
    let (dir, mut app) = app();
    asset(&dir, "a", "ears.json", profile());
    let mut jobs = vec![];
    for _ in 0..8 {
        jobs.push(prepare(&mut app, "ears.json"));
    }
    assert!(
        invoke(
            &mut app,
            "beginHrtf",
            &[Value::Str("ears.json".into()), v(json!({}))]
        )
        .is_err()
    );
    for ticket in jobs {
        call(&mut app, "cancelHrtfJob", &[ticket]);
    }
    let mut voices = vec![];
    for _ in 0..32 {
        voices.push(call(
            &mut app,
            "playSfx",
            &[v(
                json!({"asset":"tone.wav","looping":true,"spatial":{},"binaural":true}),
            )],
        ));
    }
    assert!(
        invoke(
            &mut app,
            "playSfx",
            &[v(json!({"asset":"tone.wav","spatial":{},"binaural":true}))]
        )
        .is_err()
    );
    for h in voices {
        call(&mut app, "stopVoice", &[h]);
    }
    // Admission polls ended voices, rather than requiring a frame to reclaim capacity.
    call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","spatial":{},"binaural":true}))],
    );
    call(&mut app, "clearHrtf", &[]);
    assert_eq!(call(&mut app, "hrtfStats", &[]).to_json()["voices"], 0);
}

#[test]
fn clients_load_distinct_same_named_profiles_and_clear_does_not_touch_other_audio() {
    let (dir, mut app) = app();
    asset(&dir, "a", "ears.json", profile());
    prepare(&mut app, "ears.json");
    let other = json!({"sample_rate":48000,"measurements":[{"direction":[1,0,0],"left":[0.75],"right":[0.75]}]});
    asset(&dir, "b", "ears.json", other);
    app.context().state().unwrap().client = ClientId(2);
    let second = prepare(&mut app, "ears.json");
    call(
        &mut app,
        "setListener",
        &[v(
            json!({"position":[0,0,0],"forward":[0,0,-1],"up":[0,1,0]}),
        )],
    );
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"gain":1,"spatial":{"position":[1,0,0],"min_distance":10,"max_distance":20},"binaural":true}),
        )],
    );
    wait_voice(&mut app, &h, json!([1., 0., 0.]));
    // Client b's fixture is 0.25 PCM, so its own 0.75 ears produce 0.1875.
    let out = output(&mut app, 16);
    near(out.last().unwrap()[0], 0.1875);
    near(out.last().unwrap()[1], 0.1875);
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "clearHrtf", &[]);
    assert!(call(&mut app, "hrtfStats", &[]).to_json()["dataset"].is_null());
    assert!(invoke(&mut app, "setVoiceHrtf", &[h.clone(), Value::Bool(false)]).is_err());
    let out = output(&mut app, 16);
    near(out.last().unwrap()[0], 0.1875);
    near(out.last().unwrap()[1], 0.1875);
    app.context().state().unwrap().client = ClientId(2);
    assert_eq!(
        call(&mut app, "hrtfStats", &[]).to_json()["dataset"]["revision"],
        second.to_json()
    );
    assert_eq!(call(&mut app, "hrtfVoice", &[h]).to_json()["enabled"], true);
}
