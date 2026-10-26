// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::{
    service::{AudioParameters, AudioRequest, InputFrame, PcmAudio},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"room","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","program":[]},{"name":"b","adapter":"crysirror-script-v1","game":"b","program":[]}]});
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
fn invoke(app: &mut App, name: &str, args: &[Value]) -> anyhow::Result<Value> {
    let cx = app.context();
    let audio = cx.singleton("Audio")?;
    cx.call(audio, name, args)
}
fn call(app: &mut App, name: &str, args: &[Value]) -> Value {
    invoke(app, name, args).unwrap()
}
fn begin(app: &mut App) -> Value {
    call(
        app,
        "beginReverb",
        &[Value::from_json(
            json!({"room":{"decay_seconds":0.3,"damping":0},"sends":[{"bus":"sfx","gain":1}]}),
        )],
    )
}
fn wait(app: &mut App, ticket: &Value) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let status = call(app, "reverbStatus", &[ticket.clone()]).to_json();
        assert_ne!(status["state"], "failed", "{status}");
        if status["state"] == "ready" {
            assert_eq!(status["result"]["kind"], "parametric_late_reverb");
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn output(app: &mut App, ms: u64) -> Vec<[f32; 2]> {
    app.frame(ms, InputFrame::default()).unwrap();
    app.context().state().unwrap().mixer_audio.take_output()
}
struct Gate(Arc<(Mutex<bool>, Condvar)>, Vec<crysirror_tasks::Task<()>>);
impl Gate {
    fn block(pool: crysirror_tasks::TaskPool) -> Self {
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
                    let (ready, _) = wake
                        .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(10), |v| !*v)
                        .unwrap();
                    anyhow::ensure!(*ready, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..jobs.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self(shared, jobs)
    }
    fn release(&self) {
        let (lock, wake) = &*self.0;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
    fn finish(mut self) {
        self.release();
        for job in self.1.drain(..) {
            job.wait().unwrap();
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
#[test]
fn prepared_room_publishes_both_mixers_and_native_pcm_tail_reaches_app_output() {
    let (_dir, mut app) = app();
    let ticket = begin(&mut app);
    wait(&mut app, &ticket);
    call(&mut app, "cancelReverbJob", &[ticket]);
    {
        let state = app.context();
        let state = state.state().unwrap();
        assert!(state.mixer_audio.mixer.reverb(1).is_some());
        assert!(state.runtime.host.channel_audio.mixer.reverb(1).is_some());
        state
            .runtime
            .host
            .channel_audio
            .submit(
                1,
                AudioRequest::PlayPcm {
                    channel: 9,
                    source: PcmAudio::new(48000, vec![[1.; 2]]).unwrap(),
                    parameters: AudioParameters::default(),
                },
            )
            .unwrap();
    }
    let out = output(&mut app, 200);
    assert_eq!(out[0], [1.; 2]);
    assert!(out[2400..].iter().flatten().any(|v| v.abs() > 1e-5));
    app.context().state().unwrap().client = ClientId(2);
    assert!(call(&mut app, "reverbSettings", &[]).to_json().is_null());
    call(&mut app, "clearReverb", &[]);
    app.context().state().unwrap().client = ClientId(1);
    assert!(!call(&mut app, "reverbSettings", &[]).to_json().is_null());
    call(&mut app, "clearReverb", &[]);
    assert!(output(&mut app, 200).iter().flatten().all(|v| *v == 0.));
}
#[test]
fn queued_clear_and_supersession_cannot_publish_stale_rooms_and_tickets_are_scoped() {
    let (_dir, mut app) = app();
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let old = begin(&mut app);
    let latest = begin(&mut app);
    assert!(invoke(&mut app, "reverbStatus", &[old]).is_err());
    assert_eq!(
        call(&mut app, "reverbStatus", &[latest.clone()]).to_json()["state"],
        "pending"
    );
    output(&mut app, 0);
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "reverbStatus", &[latest.clone()]).is_err());
    assert!(invoke(&mut app, "cancelReverbJob", &[latest.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "clearReverb", &[]);
    gate.finish();
    assert!(invoke(&mut app, "reverbStatus", &[latest]).is_err());
    output(&mut app, 16);
    assert!(call(&mut app, "reverbSettings", &[]).to_json().is_null());
    let fresh = begin(&mut app);
    wait(&mut app, &fresh);
}
#[test]
fn invalid_replacement_preserves_room_and_restore_drops_derived_histories_and_tickets() {
    let (_dir, mut app) = app();
    let ticket = begin(&mut app);
    wait(&mut app, &ticket);
    let prior = call(&mut app, "reverbSettings", &[]).to_json();
    assert!(
        invoke(
            &mut app,
            "beginReverb",
            &[Value::from_json(json!({"room":{"decay_seconds":0}}))]
        )
        .is_err()
    );
    assert_eq!(call(&mut app, "reverbSettings", &[]).to_json(), prior);
    let save = app.runtime().unwrap().snapshot().unwrap();
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let queued = begin(&mut app);
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&save)
        .unwrap();
    assert!(invoke(&mut app, "reverbStatus", &[ticket]).is_err());
    assert!(invoke(&mut app, "reverbStatus", &[queued]).is_err());
    assert!(call(&mut app, "reverbSettings", &[]).to_json().is_null());
    gate.finish();
    let state = app.context();
    let state = state.state().unwrap();
    assert!(state.runtime.host.channel_audio.mixer.reverb(1).is_none());
}

#[test]
fn second_mixer_publication_failure_restores_the_first_mixers_old_room() {
    let (_dir, mut app) = app();
    {
        let cx = app.context();
        let state = cx.state().unwrap();
        let effect = crysirror_audio::reverb::PreparedReverb::prepare(
            48000,
            crysirror_audio::reverb::ReverbSettings {
                wet_gain: 0.37,
                ..Default::default()
            },
        )
        .unwrap();
        state.mixer_audio.mixer.set_reverb(1, Some(effect)).unwrap();
        // Fill the native mixer's independent client table without creating client 1.
        for client in 100..164 {
            state
                .runtime
                .host
                .channel_audio
                .mixer
                .set_bus(
                    client,
                    crysirror_audio::mixer::Bus::Master,
                    Default::default(),
                )
                .unwrap();
        }
    }
    let ticket = begin(&mut app);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let status = call(&mut app, "reverbStatus", &[ticket.clone()]).to_json();
        if status["state"] != "pending" {
            assert_eq!(status["state"], "failed", "{status}");
            assert!(status["error"].as_str().unwrap().contains("client limit"));
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        call(&mut app, "reverbSettings", &[]).to_json()["wet_gain"],
        0.37f32
    );
    assert!(
        app.context()
            .state()
            .unwrap()
            .runtime
            .host
            .channel_audio
            .mixer
            .reverb(1)
            .is_none()
    );
}
