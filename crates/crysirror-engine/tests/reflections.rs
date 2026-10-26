// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::{
    decoder,
    mixer::{Spatial, VoiceHandle, VoiceSpec},
};
use crysirror_core::{
    service::{InputFrame, PcmAudio},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
fn v(j: Json) -> Value {
    Value::from_json(j)
}
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(format!("assets/{name}"))).unwrap();
    }
    std::fs::write(dir.path().join("assets/a/room.json"),serde_json::to_vec(&json!({"materials":[{"absorption":[0,0,0],"transmission":[0,0,0],"scattering":0}],"meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]})).unwrap()).unwrap();
    let project = json!({"schema":2,"name":"echo","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a","program":[]},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b","program":[]}]});
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&path, &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let ticket = call(
        &mut app,
        "beginAcousticScene",
        &[Value::Str("room.json".into())],
    );
    assert_eq!(wait(&mut app, "acousticStatus", &ticket)["state"], "ready");
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
fn wait(app: &mut App, name: &str, ticket: &Value) -> Json {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let status = call(app, name, &[ticket.clone()]).to_json();
        if status["state"] != "pending" {
            return status;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn source(app: &mut App) -> VoiceHandle {
    let cx = app.context();
    let s = cx.state().unwrap();
    let h = s
        .mixer_audio
        .mixer
        .play(
            1,
            decoder::open_pcm(PcmAudio::new(48000, vec![[1.; 2]]).unwrap()),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [0., 0., 2.],
                    min_distance: 10.,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    s.mixer_audio
        .mixer
        .set_acoustic_gains(1, h, Some([0.; 3]))
        .unwrap();
    h
}
fn begin(app: &mut App, h: VoiceHandle, options: Json) -> Value {
    call(app, "beginVoiceReflections", &[v(json!(h)), v(options)])
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
fn independent_probe_and_prepared_voice_publish_real_echo_samples_without_direct_sound() {
    let (_dir, mut app) = app();
    let probe = call(
        &mut app,
        "beginReflectionProbe",
        &[v(
            json!({"source":[0,0,2],"listener":[0,0,0],"options":{"propagation":{"air_absorption":[0,0,0]}}}),
        )],
    );
    let status = wait(&mut app, "reflectionStatus", &probe);
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(
        status["result"]["paths"]["paths"].as_array().unwrap().len(),
        1
    );
    let p = &status["result"]["paths"]["paths"][0];
    assert!((p["length_metres"].as_f64().unwrap() - 6.).abs() < 1e-5);
    call(&mut app, "cancelReflectionJob", &[probe]);
    let h = source(&mut app);
    let ticket = begin(
        &mut app,
        h,
        json!({"propagation":{"air_absorption":[0,0,0]}}),
    );
    assert_eq!(
        wait(&mut app, "reflectionStatus", &ticket)["state"],
        "ready"
    );
    call(&mut app, "cancelReflectionJob", &[ticket]);
    let out = output(&mut app, 20);
    assert!(out[..559].iter().flatten().all(|s| *s == 0.));
    assert!(out[559..562].iter().flatten().any(|s| s.abs() > 0.05));
    assert!(
        !app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .source_is_playing(1, h)
    );
    assert!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .is_playing(1, h)
    );
    output(&mut app, 20);
    assert!(
        !app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .is_playing(1, h)
    );
}
#[test]
fn blocked_initial_preparation_holds_only_its_source_and_cancel_cannot_publish_later() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let old = begin(&mut app, h, json!({}));
    let ticket = begin(&mut app, h, json!({}));
    assert!(invoke(&mut app, "reflectionStatus", &[old]).is_err());
    {
        let cx = app.context();
        cx.state()
            .unwrap()
            .mixer_audio
            .mixer
            .play(
                2,
                decoder::open_pcm(PcmAudio::new(48000, vec![[0.25; 2]; 1024]).unwrap()),
                VoiceSpec::default(),
            )
            .unwrap();
    }
    assert!(
        output(&mut app, 1)
            .iter()
            .all(|s| (s[0] - 0.25).abs() < 1e-6)
    );
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .position(1, h)
            .unwrap(),
        0.
    );
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "reflectionStatus", &[ticket.clone()]).is_err());
    assert!(invoke(&mut app, "clearVoiceReflections", &[v(json!(h))]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "cancelReflectionJob", &[ticket]);
    gate.finish();
    output(&mut app, 0);
    assert!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .reflection_paths(1, h)
            .unwrap()
            .is_none()
    );
}
#[test]
fn pose_and_scene_invalidation_leave_failed_receipts_and_release_the_clock_hold() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let ticket = begin(&mut app, h, json!({}));
    app.context()
        .state()
        .unwrap()
        .mixer_audio
        .mixer
        .set_spatial(
            1,
            h,
            Some(Spatial {
                position: [1., 0., 2.],
                ..Default::default()
            }),
        )
        .unwrap();
    let status = call(&mut app, "reflectionStatus", &[ticket.clone()]).to_json();
    assert_eq!(status["state"], "failed");
    assert!(status["error"].as_str().unwrap().contains("captured pose"));
    gate.finish();
    let next = begin(&mut app, h, json!({}));
    assert_eq!(wait(&mut app, "reflectionStatus", &next)["state"], "ready");
    call(&mut app, "clearAcousticScene", &[]);
    assert!(call(&mut app, "reflectionsVoice", &[v(json!(h))]).to_json()["paths"].is_null());
    assert!(output(&mut app, 1).iter().flatten().all(|s| *s == 0.));
    assert!(
        invoke(
            &mut app,
            "beginReflectionProbe",
            &[v(json!({"source":[0,0,2],"listener":[0,0,0]}))]
        )
        .is_err()
    );
}
#[test]
fn failed_replacement_preserves_old_paths_and_restore_removes_derived_effects_and_tickets() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    let ready = begin(&mut app, h, json!({}));
    assert_eq!(wait(&mut app, "reflectionStatus", &ready)["state"], "ready");
    let bad = begin(&mut app, h, json!({"triangle_budget":1}));
    let result = wait(&mut app, "reflectionStatus", &bad);
    assert_eq!(result["state"], "failed");
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("triangle budget")
    );
    assert!(call(&mut app, "reflectionsVoice", &[v(json!(h))]).to_json()["paths"].is_object());
    let saved = app.runtime().unwrap().snapshot().unwrap();
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&saved)
        .unwrap();
    assert!(invoke(&mut app, "reflectionStatus", &[ready]).is_err());
    assert!(
        !app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .is_playing(1, h)
    );
}

#[test]
fn full_voice_pool_keeps_replacement_headroom_after_worker_reservations_shrink() {
    let (_dir, mut app) = app();
    let mut first = None;
    for _ in 0..64 {
        let h = source(&mut app);
        first.get_or_insert(h);
        let ticket = begin(&mut app, h, json!({}));
        let result = wait(&mut app, "reflectionStatus", &ticket);
        assert_eq!(result["state"], "ready", "{result}");
        call(&mut app, "cancelReflectionJob", &[ticket]);
    }
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .stats()
            .active,
        64
    );
    let ticket = begin(&mut app, first.unwrap(), json!({"wet_gain":0.5}));
    let result = wait(&mut app, "reflectionStatus", &ticket);
    assert_eq!(result["state"], "ready", "{result}");
    assert_eq!(
        result["result"]["paths"]["query"]["options"]["wet_gain"],
        0.5
    );
}
