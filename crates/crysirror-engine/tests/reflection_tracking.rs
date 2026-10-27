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
            decoder::open_pcm(PcmAudio::new(48000, vec![[1.; 2]; 480]).unwrap()),
            VoiceSpec {
                looping: true,
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
fn tracking(app: &mut App, h: VoiceHandle) -> Json {
    call(app, "reflectionsVoice", &[v(json!(h))]).to_json()["tracking"].clone()
}
fn track(app: &mut App, h: VoiceHandle) {
    call(
        app,
        "trackVoiceReflections",
        &[
            v(json!(h)),
            v(
                json!({"refresh_interval_ms":16,"crossfade_ms":1,"position_threshold_metres":0,"angle_threshold_degrees":0,"options":{"propagation":{"air_absorption":[0,0,0]}}}),
            ),
        ],
    );
}
fn pump(app: &mut App, h: VoiceHandle, mut ready: impl FnMut(&Json) -> bool) -> Json {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        output(app, 16);
        let s = tracking(app, h);
        if ready(&s) {
            return s;
        }
        assert!(Instant::now() < end, "{s}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn continuously_moving_source_and_listener_publish_completed_snapshots_without_starvation() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    track(&mut app, h);
    pump(&mut app, h, |s| {
        s["completed_updates"].as_u64().unwrap() > 0
    });
    for i in 1..=30 {
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .set_spatial(
                1,
                h,
                Some(Spatial {
                    position: [i as f32 * 0.1, 0., 2.],
                    min_distance: 10.,
                    ..Default::default()
                }),
            )
            .unwrap();
        output(&mut app, 16);
        std::thread::sleep(Duration::from_millis(2));
    }
    let state = pump(&mut app, h, |s| s["published"]["query"]["source"][0] == 3.);
    assert!(state["completed_updates"].as_u64().unwrap() > 5, "{state}");
    call(
        &mut app,
        "setListener",
        &[v(
            json!({"position":[0.5,0,0],"forward":[1,0,0],"up":[0,1,0]}),
        )],
    );
    let state = pump(&mut app, h, |s| {
        s["published"]["query"]["listener"][0] == 0.5
    });
    assert_eq!(
        state["published"]["query"]["listener_forward"],
        json!([1.0, 0.0, 0.0])
    );
    assert_eq!(state["muted"], false);
    assert!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .reflection_paths(1, h)
            .unwrap()
            .is_some()
    );
}

#[test]
fn instance_commit_invalidates_old_reflector_and_automatically_publishes_new_paths() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    track(&mut app, h);
    let before = pump(&mut app, h, |s| {
        s["completed_updates"].as_u64().unwrap_or(0) > 0
    });
    let rev = before["published"]["revision"].clone();
    let paths = call(&mut app, "reflectionsVoice", &[v(json!(h))]).to_json();
    assert!(
        !paths["paths"]["paths"].as_array().unwrap().is_empty(),
        "{paths}"
    );
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(
            json!([{"id":7,"transform":glam::Mat4::from_translation(glam::Vec3::X*30.).to_cols_array()}]),
        )],
    );
    let ready = wait(&mut app, "acousticStatus", &t);
    assert_eq!(ready["state"], "ready", "{ready}");
    call(&mut app, "cancelAcousticJob", &[t]);
    let after = pump(&mut app, h, |s| {
        s["published"]["revision"] != rev
            && s["completed_updates"].as_u64().unwrap_or(0) > 1
            && !s["pending"].as_bool().unwrap_or(true)
    });
    assert_eq!(after["muted"], false);
    let paths = call(&mut app, "reflectionsVoice", &[v(json!(h))]).to_json();
    assert!(
        paths["paths"]["paths"].as_array().unwrap().is_empty(),
        "{paths}"
    );
    let pcm = output(&mut app, 50);
    assert!(
        pcm[pcm.len() - 512..]
            .iter()
            .flatten()
            .all(|s| s.abs() < 1e-6)
    );
}

#[test]
fn instance_motion_keeps_completed_reflection_snapshot_while_new_revision_refreshes() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    track(&mut app, h);
    let initial = pump(&mut app, h, |s| s["completed_updates"] == 1);
    let old = initial["published"]["revision"].clone();
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    app.context()
        .state()
        .unwrap()
        .mixer_audio
        .mixer
        .set_spatial(
            1,
            h,
            Some(Spatial {
                position: [0.1, 0., 2.],
                min_distance: 10.,
                ..Default::default()
            }),
        )
        .unwrap();
    output(&mut app, 16);
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(
            json!([{"id":7,"transform":glam::Mat4::from_translation(glam::Vec3::X*0.1).to_cols_array()}]),
        )],
    );
    gate.finish();
    let ready = wait(&mut app, "acousticStatus", &t);
    assert_eq!(ready["state"], "ready", "{ready}");
    call(&mut app, "cancelAcousticJob", &[t]);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let s = tracking(&mut app, h);
        if s["completed_updates"] == 2 {
            assert_eq!(s["published"]["revision"], old);
            assert_eq!(s["geometry_stale"], true);
            assert_eq!(s["muted"], false);
            break;
        }
        assert!(Instant::now() < end, "{s}");
        std::thread::yield_now();
    }
    let final_state = pump(&mut app, h, |s| {
        s["published"]["revision"] == ready["result"]["revision"]
    });
    assert_eq!(final_state["geometry_stale"], false);
}
#[test]
fn blocked_refresh_keeps_old_history_and_clear_or_restore_prevents_late_publication() {
    let (_dir, mut app) = app();
    let h = source(&mut app);
    track(&mut app, h);
    pump(&mut app, h, |s| s["completed_updates"] == 1);
    output(&mut app, 32);
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    app.context()
        .state()
        .unwrap()
        .mixer_audio
        .mixer
        .set_spatial(
            1,
            h,
            Some(Spatial {
                position: [2., 0., 2.],
                min_distance: 10.,
                ..Default::default()
            }),
        )
        .unwrap();
    let out = output(&mut app, 32);
    assert!(out.iter().flatten().any(|s| s.abs() > 0.01));
    let state = tracking(&mut app, h);
    assert_eq!(state["published"]["query"]["source"][0], 0.0);
    assert_eq!(state["pending"], true);
    app.context().state().unwrap().client = ClientId(2);
    assert!(
        invoke(
            &mut app,
            "trackVoiceReflections",
            &[v(json!(h)), v(json!({}))]
        )
        .is_err()
    );
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "clearVoiceReflections", &[v(json!(h))]);
    gate.finish();
    output(&mut app, 32);
    assert!(tracking(&mut app, h).is_null());
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
    track(&mut app, h);
    let saved = app.runtime().unwrap().snapshot().unwrap();
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&saved)
        .unwrap();
    output(&mut app, 0);
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
fn replacing_geometry_mutes_old_paths_until_current_revision_and_clear_removes_tracking() {
    let (dir, mut app) = app();
    let h = source(&mut app);
    track(&mut app, h);
    let initial = pump(&mut app, h, |s| s["completed_updates"] == 1);
    let old = initial["published"]["revision"].as_u64().unwrap();
    let mut room: Json =
        serde_json::from_slice(&std::fs::read(dir.path().join("assets/a/room.json")).unwrap())
            .unwrap();
    for point in room["meshes"][0]["vertices"].as_array_mut().unwrap() {
        point[2] = json!(-3);
    }
    std::fs::write(
        dir.path().join("assets/a/room.json"),
        serde_json::to_vec(&room).unwrap(),
    )
    .unwrap();
    let ticket = call(
        &mut app,
        "beginAcousticScene",
        &[Value::Str("room.json".into())],
    );
    let new = wait(&mut app, "acousticStatus", &ticket)["result"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(old, new);
    let state = tracking(&mut app, h);
    assert_eq!(state["muted"], true, "{state}");
    let state = pump(&mut app, h, |s| s["published"]["revision"] == new);
    assert_eq!(state["muted"], false);
    let paths = call(&mut app, "reflectionsVoice", &[v(json!(h))]).to_json();
    assert!(
        (paths["paths"]["paths"][0]["length_metres"]
            .as_f64()
            .unwrap()
            - 8.)
            .abs()
            < 1e-5
    );
    call(&mut app, "clearAcousticScene", &[]);
    assert!(tracking(&mut app, h).is_null());
}
#[test]
fn round_robin_admission_reaches_all_voices_even_when_the_first_ones_keep_moving() {
    for count in [5, 12] {
        let (_dir, mut app) = app();
        let mut handles = vec![];
        for _ in 0..count {
            let h = source(&mut app);
            track(&mut app, h);
            handles.push(h);
        }
        let end = Instant::now() + Duration::from_secs(10);
        let mut step = 0;
        loop {
            step += 1;
            for h in handles.iter().take(4) {
                app.context()
                    .state()
                    .unwrap()
                    .mixer_audio
                    .mixer
                    .set_spatial(
                        1,
                        *h,
                        Some(Spatial {
                            position: [step as f32 * 0.01, 0., 2.],
                            min_distance: 10.,
                            ..Default::default()
                        }),
                    )
                    .unwrap();
            }
            output(&mut app, 16);
            std::thread::sleep(Duration::from_millis(1));
            if handles.iter().all(|h| {
                tracking(&mut app, *h)["completed_updates"]
                    .as_u64()
                    .unwrap()
                    > 0
            }) {
                break;
            }
            assert!(Instant::now() < end, "initial voices starved");
        }
    }
}

#[test]
fn declarative_short_sound_waits_for_hrtf_then_reflections_and_drains_real_echoes() {
    let (dir, mut app) = app();
    let mut wave = Vec::new();
    wave.extend(b"RIFF");
    wave.extend(38u32.to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(48000u32.to_le_bytes());
    wave.extend(96000u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend(2u32.to_le_bytes());
    wave.extend(16384i16.to_le_bytes());
    std::fs::write(dir.path().join("assets/a/pulse.wav"), wave).unwrap();
    std::fs::write(dir.path().join("assets/a/ears.json"),serde_json::to_vec(&json!({"sample_rate":48000,"measurements":[{"direction":[0,0,1],"left":[1],"right":[1]}]})).unwrap()).unwrap();
    let ticket = call(
        &mut app,
        "beginHrtf",
        &[
            Value::Str("ears.json".into()),
            v(json!({"max_delay_seconds":0})),
        ],
    );
    assert_eq!(wait(&mut app, "hrtfStatus", &ticket)["state"], "ready");
    let ticket = call(
        &mut app,
        "beginPreloadSfx",
        &[Value::Str("pulse.wav".into())],
    );
    assert_eq!(wait(&mut app, "playbackStatus", &ticket)["state"], "ready");
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let voice = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"pulse.wav","binaural":true,"spatial":{"position":[0,0,2],"min_distance":10},"reflections":{"options":{"propagation":{"air_absorption":[0,0,0]}}}}),
        )],
    );
    let h: VoiceHandle = serde_json::from_value(voice.to_json()).unwrap();
    app.context()
        .state()
        .unwrap()
        .mixer_audio
        .mixer
        .set_acoustic_gains(1, h, Some([0.; 3]))
        .unwrap();
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
    assert_eq!(
        call(&mut app, "voice", &[voice.clone()]).to_json()["position_seconds"],
        0.0
    );
    assert!(
        tracking(&mut app, h)["waiting_for"]
            .as_str()
            .unwrap()
            .contains("HRTF")
    );
    gate.finish();
    // Zero-duration control frames publish both dependent resources before any
    // source sample is consumed, so even a one-frame clip is retained.
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        output(&mut app, 0);
        if tracking(&mut app, h)["completed_updates"] == 1 {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = tracking(&mut app, h);
    assert_eq!(state["published"]["latency"], 64);
    let first = output(&mut app, 1);
    assert!(first.iter().flatten().all(|s| *s == 0.));
    assert_eq!(
        call(&mut app, "voice", &[voice.clone()]).to_json()["source_playing"],
        false
    );
    assert_eq!(
        call(&mut app, "voice", &[voice.clone()]).to_json()["playing"],
        true
    );
    let wet = output(&mut app, 12);
    assert!(wet.iter().flatten().any(|s| *s > 0.01));
    output(&mut app, 40);
    assert_eq!(
        call(&mut app, "voice", &[voice]).to_json()["playing"],
        false
    );
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let entity = cx
        .call(
            world,
            "spawn",
            &[v(json!({"name":"tracked emitter","position":[1,0,2]}))],
        )
        .unwrap();
    let voice = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"pulse.wav","looping":true,"follow":entity.to_json(),"reflections":{}}),
        )],
    );
    let h = serde_json::from_value(voice.to_json()).unwrap();
    pump(&mut app, h, |s| s["published"]["query"]["source"][0] == 1.0);
    app.context()
        .set(
            entity.as_object().unwrap(),
            "position",
            Value::vec3([3., 0., 2.]),
        )
        .unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(
        world,
        "setCamera",
        &[v(json!({"eye":[0.25,0,0],"target":[0.25,0,-1]}))],
    )
    .unwrap();
    pump(&mut app, h, |s| {
        s["published"]["query"]["source"][0] == 3.0
            && s["published"]["query"]["listener"][0] == 0.25
    });
    app.context()
        .call(entity.as_object().unwrap(), "destroy", &[])
        .unwrap();
    output(&mut app, 16);
    assert_eq!(
        call(&mut app, "voice", &[voice]).to_json()["playing"],
        false
    );
}
