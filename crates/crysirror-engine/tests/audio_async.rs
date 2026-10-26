// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Worker playback, scoped publication and exact seeks over shared mixer APIs.
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
    for i in 0..16000 {
        b.extend((if i >= 8192 { value / 2 } else { value }).to_le_bytes())
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
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let status = call(app, "playbackStatus", &[ticket.clone()]).to_json();
        if status["state"] != "pending" {
            return status;
        }
        assert!(Instant::now() < end, "{status}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn sfx(app: &mut App) -> Value {
    let ticket = call(
        app,
        "beginPlaySfx",
        &[v(json!({"asset":"tone.wav","looping":true}))],
    );
    let status = wait(app, &ticket);
    assert_eq!(status["state"], "ready", "{status}");
    call(app, "cancelPlaybackJob", &[ticket]);
    v(status["result"]["handle"].clone())
}
fn music(app: &mut App) -> Value {
    let t = call(
        app,
        "beginPlayMusic",
        &[
            Value::Str("bgm".into()),
            v(json!({"asset":"tone.wav"})),
            Value::Float(0.),
        ],
    );
    let j = wait(app, &t);
    assert_eq!(j["state"], "ready", "{j}");
    call(app, "cancelPlaybackJob", &[t]);
    v(j["result"]["handle"].clone())
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}
#[test]
fn worker_admission_cancel_and_ready_unobserved_cancel_never_start_playback() {
    let (_dir, mut app) = app();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let t = call(&mut app, "beginPlaySfx", &[v(json!({"asset":"tone.wav"}))]);
    assert_eq!(
        call(&mut app, "playbackStatus", &[t.clone()]).to_json()["state"],
        "pending"
    );
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "playbackStatus", &[t.clone()]).is_err());
    assert!(invoke(&mut app, "cancelPlaybackJob", &[t.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "cancelPlaybackJob", &[t]);
    gate.finish();
    let t = call(&mut app, "beginPlaySfx", &[v(json!({"asset":"tone.wav"}))]);
    let end = Instant::now() + Duration::from_secs(5);
    while app.runtime().unwrap().assets.io_tasks().stats().outstanding > 0 {
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    // The worker completed, but the owner has not published this result.
    call(&mut app, "cancelPlaybackJob", &[t]);
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
    assert!(invoke(&mut app, "playSfx", &[v(json!({"asset":"tone.wav"}))]).is_err());
}
#[test]
fn preload_cache_and_same_named_assets_remain_client_scoped() {
    let (_dir, mut app) = app();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let t = call(
        &mut app,
        "beginPreloadSfx",
        &[Value::Str("tone.wav".into())],
    );
    let j = wait(&mut app, &t);
    assert_eq!(j["result"]["frames"], 16000);
    call(&mut app, "cancelPlaybackJob", &[t]);
    let h = call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","looping":true}))],
    );
    near(output(&mut app, 16)[0][0], 0.5);
    call(&mut app, "clearSfxCache", &[]);
    near(output(&mut app, 16)[0][0], 0.5);
    assert!(invoke(&mut app, "playSfx", &[v(json!({"asset":"tone.wav"}))]).is_err());
    call(&mut app, "stopVoice", &[h, Value::Float(0.)]);
    app.context().state().unwrap().client = ClientId(2);
    let b = sfx(&mut app);
    near(output(&mut app, 16)[0][0], 0.25);
    app.context().state().unwrap().client = ClientId(1);
    assert!(invoke(&mut app, "sourceStatus", &[b.clone()]).is_err());
    let a = sfx(&mut app);
    near(output(&mut app, 16)[0][0], 0.75);
    call(&mut app, "stopVoice", &[a, Value::Float(0.)]);
}
#[test]
fn streamed_music_failed_replacement_and_exact_uncached_seek() {
    let (_dir, mut app) = app();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let h = music(&mut app);
    near(output(&mut app, 16)[0][0], 0.5);
    let bad = call(
        &mut app,
        "beginPlayMusic",
        &[
            Value::Str("bgm".into()),
            v(json!({"asset":"missing.wav"})),
            Value::Float(10.),
        ],
    );
    assert_eq!(wait(&mut app, &bad)["state"], "failed");
    near(output(&mut app, 16)[0][0], 0.5);
    call(&mut app, "cancelPlaybackJob", &[bad]);
    assert!(invoke(&mut app, "seekVoice", &[h.clone(), Value::Int(9000)]).is_err());
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let before = call(&mut app, "voice", &[h.clone()]).to_json();
    let seek = call(&mut app, "beginSeekVoice", &[h.clone(), Value::Int(9000)]);
    assert_eq!(
        call(&mut app, "playbackStatus", &[seek.clone()]).to_json()["state"],
        "pending"
    );
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
    assert_eq!(call(&mut app, "voice", &[h.clone()]).to_json(), before);
    gate.finish();
    assert_eq!(wait(&mut app, &seek)["result"]["source_frame"], 9000);
    let position = call(&mut app, "voice", &[h.clone()]).to_json()["position_seconds"]
        .as_f64()
        .unwrap();
    assert_eq!(position, 1.125);
    near(output(&mut app, 16)[0][0], 0.25);
    let status = call(&mut app, "sourceStatus", &[h]).to_json();
    assert!(status["ready_frames"].as_u64().unwrap() <= 12288);
}
#[test]
fn superseded_seek_and_foreign_seek_tickets_fail_and_cancel_resumes_old_position() {
    let (_dir, mut app) = app();
    let h = music(&mut app);
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let old = call(&mut app, "beginSeekVoice", &[h.clone(), Value::Int(9000)]);
    let newer = call(&mut app, "beginSeekVoice", &[h.clone(), Value::Int(12000)]);
    assert_eq!(wait(&mut app, &old)["state"], "failed");
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "cancelPlaybackJob", &[newer.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "cancelPlaybackJob", &[newer]);
    gate.finish();
    near(output(&mut app, 16)[0][0], 0.5);
    assert!(
        call(&mut app, "voice", &[h]).to_json()["position_seconds"]
            .as_f64()
            .unwrap()
            < 0.1
    );
}
#[test]
fn restore_invalidates_pending_preparation_and_stream_voices() {
    let (_dir, mut app) = app();
    let h = music(&mut app);
    let save = app.runtime().unwrap().snapshot().unwrap();
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let t = call(&mut app, "beginPlaySfx", &[v(json!({"asset":"tone.wav"}))]);
    app.runtime_mut().unwrap().restore(&save).unwrap();
    output(&mut app, 0);
    assert!(invoke(&mut app, "playbackStatus", &[t]).is_err());
    assert_eq!(call(&mut app, "voice", &[h]).to_json()["playing"], false);
    gate.finish();
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
}
#[test]
fn retained_result_capacity_bounds_pending_and_published_jobs() {
    let (_dir, mut app) = app();
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let mut tickets = vec![];
    for _ in 0..16 {
        tickets.push(call(
            &mut app,
            "beginPreloadSfx",
            &[Value::Str("tone.wav".into())],
        ));
    }
    assert!(
        invoke(
            &mut app,
            "beginPreloadSfx",
            &[Value::Str("tone.wav".into())]
        )
        .is_err()
    );
    for t in tickets {
        call(&mut app, "cancelPlaybackJob", &[t]);
    }
    gate.finish();
    let t = call(
        &mut app,
        "beginPreloadSfx",
        &[Value::Str("tone.wav".into())],
    );
    assert_eq!(wait(&mut app, &t)["state"], "ready");
}

#[test]
fn capability_stream_decodes_native_nwa_and_loops_through_shared_audio_api() {
    let (dir, mut app) = app();
    let mut bytes = vec![0; 44];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&16u16.to_le_bytes());
    for (at, n) in [
        (4, 48000u32),
        (8, u32::MAX),
        (20, 8192),
        (24, 8236),
        (28, 4096),
    ] {
        bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    for _ in 0..4096 {
        bytes.extend(16384i16.to_le_bytes());
    }
    std::fs::write(dir.path().join("assets/a/tone.nwa"), bytes).unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let t = call(
        &mut app,
        "beginPlayMusic",
        &[
            Value::Str("native".into()),
            v(json!({"asset":"tone.nwa"})),
            Value::Int(0),
        ],
    );
    let j = wait(&mut app, &t);
    assert_eq!(j["state"], "ready", "{j}");
    call(&mut app, "cancelPlaybackJob", &[t]);
    for _ in 0..12 {
        let pcm = output(&mut app, 16);
        assert!(pcm.iter().flatten().all(|x| (*x - 0.5).abs() < 1e-6));
    }
}

#[test]
fn stopping_a_voice_during_pending_seek_still_allows_ticket_release() {
    let (_dir, mut app) = app();
    let h = music(&mut app);
    let gate = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let t = call(&mut app, "beginSeekVoice", &[h.clone(), Value::Int(9000)]);
    call(&mut app, "stopVoice", &[h, Value::Int(0)]);
    call(&mut app, "cancelPlaybackJob", &[t.clone()]);
    assert!(invoke(&mut app, "playbackStatus", &[t]).is_err());
    gate.finish();
}
