// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine-owned acoustic geometry -> bounded jobs -> real offline PCM output.
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
        let state = call(app, "acousticStatus", &[ticket.clone()]).to_json();
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
fn wall() -> serde_json::Value {
    json!({"materials":[{"transmission":[0,0,0]}],"meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]})
}
fn prepare(app: &mut App, name: &str) -> Value {
    let ticket = call(app, "beginAcousticScene", &[Value::Str(name.into())]);
    let result = wait(app, &ticket);
    assert_eq!(result["state"], "ready", "{result}");
    ticket
}
#[test]
fn build_and_probe_are_nonblocking_scoped_and_transactional() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    asset(&dir, "a", "bad.json", json!({"unsupported":true}));
    asset(&dir, "b", "room.json", json!({}));
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let ticket = call(
        &mut app,
        "beginAcousticScene",
        &[Value::Str("room.json".into())],
    );
    assert_eq!(
        call(&mut app, "acousticStatus", &[ticket.clone()]).to_json()["state"],
        "pending"
    );
    output(&mut app, 16); // owner frame progresses while every CPU worker is gated
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "acousticStatus", &[ticket.clone()]).is_err());
    assert!(invoke(&mut app, "cancelAcousticJob", &[ticket.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    gate.finish();
    assert_eq!(wait(&mut app, &ticket)["state"], "ready");
    call(&mut app, "cancelAcousticJob", &[ticket.clone()]);
    assert!(call(&mut app, "acousticStats", &[]).to_json()["scene"].is_object());
    let probe = call(
        &mut app,
        "beginAcousticProbe",
        &[v(
            json!({"source":[0,0,-4],"listener":[0,0,0],"settings":{"air_absorption":[0,0,0]}}),
        )],
    );
    let result = wait(&mut app, &probe);
    assert_eq!(result["result"]["gains"], json!([0., 0., 0.]));
    assert_eq!(result["result"]["revision"], ticket.to_json());
    let bad = call(
        &mut app,
        "beginAcousticScene",
        &[Value::Str("bad.json".into())],
    );
    assert_eq!(wait(&mut app, &bad)["state"], "failed");
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["scene"]["revision"],
        ticket.to_json()
    );
    app.context().state().unwrap().client = ClientId(2);
    let other = prepare(&mut app, "room.json");
    assert_ne!(other.to_json(), ticket.to_json());
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["scene"]["triangles"],
        0
    );
    assert!(
        invoke(
            &mut app,
            "beginAcousticScene",
            &[Value::Str("../room.json".into())]
        )
        .is_err()
    );
}
#[test]
fn canceled_jobs_cannot_publish_and_restore_invalidates_scene_and_tickets() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let canceled = call(
        &mut app,
        "beginAcousticScene",
        &[Value::Str("room.json".into())],
    );
    call(&mut app, "cancelAcousticJob", &[canceled.clone()]);
    gate.finish();
    assert!(invoke(&mut app, "acousticStatus", &[canceled]).is_err());
    assert!(call(&mut app, "acousticStats", &[]).to_json()["scene"].is_null());
    let ready = prepare(&mut app, "room.json");
    let save = app.runtime().unwrap().snapshot().unwrap();
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&save)
        .unwrap();
    assert!(invoke(&mut app, "acousticStatus", &[ready]).is_err());
    assert!(call(&mut app, "acousticStats", &[]).to_json()["scene"].is_null());
}
#[test]
fn automatic_propagation_changes_real_pcm_and_scene_clear_restores_direct_audio() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
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
            json!({"asset":"tone.wav","looping":true,"spatial":{"position":[0,0,-4],"min_distance":10,"max_distance":20},"acoustics":{"air_absorption":[0,0,0]}}),
        )],
    );
    // A blocked source is silent from its first sample, including before first completion.
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        output(&mut app, 0);
        let state = call(&mut app, "acousticVoice", &[h.clone()]).to_json();
        if state["result"]["path"].is_object() {
            assert_eq!(state["result"]["path"]["gains"], json!([0., 0., 0.]));
            break;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::yield_now();
    }
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "acousticVoice", &[h.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    call(&mut app, "clearAcousticScene", &[]);
    assert!(output(&mut app, 16).last().unwrap()[0] > 0.3);
    assert!(
        invoke(
            &mut app,
            "playSfx",
            &[v(json!({"asset":"tone.wav","spatial":{},"acoustics":{}}))]
        )
        .is_err()
    );
}
#[test]
fn admission_bounds_retained_results_and_releasing_returns_geometry_reservations() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    let scene = prepare(&mut app, "room.json");
    let mut probes = vec![];
    for _ in 0..15 {
        let ticket = call(
            &mut app,
            "beginAcousticProbe",
            &[v(json!({"source":[0,0,-4],"listener":[0,0,0]}))],
        );
        assert_eq!(wait(&mut app, &ticket)["state"], "ready");
        probes.push(ticket);
    }
    assert!(
        invoke(
            &mut app,
            "beginAcousticProbe",
            &[v(json!({"source":[0,0,-4],"listener":[0,0,0]}))]
        )
        .is_err()
    );
    for ticket in probes {
        call(&mut app, "cancelAcousticJob", &[ticket]);
    }
    call(&mut app, "cancelAcousticJob", &[scene]);
    call(&mut app, "clearAcousticScene", &[]);
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["host_geometry_reservation"],
        0
    );
}
#[test]
fn legacy_physics_occlusion_uses_voice_owner_instead_of_default_query_client() {
    let (_dir, mut app) = app();
    app.context().state().unwrap().client = ClientId(2);
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world,"spawn",&[v(json!({"name":"wall","position":[0,0,-2],"geometry":{"kind":"cube"},"collider":{"shape":{"kind":"box","half_extents":[2,2,0.1]}}}))]).unwrap();
    call(
        &mut app,
        "setListener",
        &[v(
            json!({"position":[0,0,0],"forward":[0,0,-1],"up":[0,1,0]}),
        )],
    );
    let voice = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"spatial":{"position":[0,0,-4]},"raycast_occlusion":true}),
        )],
    );
    output(&mut app, 16);
    let handle = serde_json::from_value(voice.to_json()).unwrap();
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .spec(2, handle)
            .unwrap()
            .spatial
            .unwrap()
            .occlusion,
        1.
    );
}

#[test]
fn explicit_probe_keeps_immutable_geometry_alive_after_scene_clear() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    let scene = prepare(&mut app, "room.json");
    call(&mut app, "cancelAcousticJob", &[scene.clone()]);
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let ticket = call(
        &mut app,
        "beginAcousticProbe",
        &[v(
            json!({"source":[0,0,-4],"listener":[0,0,0],"settings":{"air_absorption":[0,0,0]}}),
        )],
    );
    call(&mut app, "clearAcousticScene", &[]);
    let stats = call(&mut app, "acousticStats", &[]).to_json();
    assert!(stats["scene"].is_null());
    assert!(stats["host_geometry_reservation"].as_u64().unwrap() > 0);
    gate.finish();
    let path = wait(&mut app, &ticket);
    assert_eq!(path["result"]["gains"], json!([0., 0., 0.]));
    assert_eq!(path["result"]["revision"], scene.to_json());
    call(&mut app, "cancelAcousticJob", &[ticket]);
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["host_geometry_reservation"],
        0
    );
}

fn matrix(x: f32) -> serde_json::Value {
    json!(glam::Mat4::from_translation(glam::Vec3::X * x).to_cols_array())
}
fn probe(app: &mut App) -> serde_json::Value {
    let t = call(
        app,
        "beginAcousticProbe",
        &[v(
            json!({"source":[0,0,-4],"listener":[0,0,0],"settings":{"radius":0,"air_absorption":[0,0,0]}}),
        )],
    );
    let result = wait(app, &t);
    assert_eq!(result["state"], "ready", "{result}");
    call(app, "cancelAcousticJob", &[t]);
    result["result"].clone()
}
fn world(app: &mut App, commands: serde_json::Value) {
    let commands: Vec<crysirror_scene::world::WorldCommand> =
        serde_json::from_value(commands).unwrap();
    app.context()
        .state()
        .unwrap()
        .runtime
        .host
        .engine
        .world
        .apply_batch(ClientId(1), &commands)
        .unwrap();
}
fn door(app: &mut App) -> crysirror_scene::world::EntityHandle {
    world(
        app,
        json!([{"op":"spawn","entity":{"id":20,"name":"door"}}]),
    );
    app.context()
        .state()
        .unwrap()
        .runtime
        .host
        .engine
        .world
        .handle(ClientId(1), 20)
        .unwrap()
}
fn instance_pump(
    app: &mut App,
    mut done: impl FnMut(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        output(app, 16);
        let s = call(app, "acousticStats", &[]).to_json();
        if done(&s) {
            return s;
        }
        assert!(Instant::now() < end, "{s}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn explicit_instance_commits_are_atomic_scoped_and_leave_captured_probe_geometry_unchanged() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    let initial = prepare(&mut app, "room.json");
    call(&mut app, "cancelAcousticJob", &[initial]);
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let captured = call(
        &mut app,
        "beginAcousticProbe",
        &[v(
            json!({"source":[0,0,-4],"listener":[0,0,0],"settings":{"air_absorption":[0,0,0]}}),
        )],
    );
    let change = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(json!([{"id":7,"transform":matrix(30.)}]))],
    );
    assert_eq!(
        call(&mut app, "acousticStatus", &[change.clone()]).to_json()["state"],
        "pending"
    );
    assert!(
        invoke(
            &mut app,
            "beginUpdateAcousticScene",
            &[v(json!([{"id":7,"enabled":false}]))]
        )
        .is_err()
    );
    app.context().state().unwrap().client = ClientId(2);
    assert!(invoke(&mut app, "acousticStatus", &[change.clone()]).is_err());
    app.context().state().unwrap().client = ClientId(1);
    gate.finish();
    assert_eq!(
        wait(&mut app, &captured)["result"]["gains"],
        json!([0., 0., 0.])
    );
    let result = wait(&mut app, &change);
    assert_eq!(result["state"], "ready", "{result}");
    assert_eq!(result["result"]["geometry"]["reused_mesh_bvhs"], 1);
    call(&mut app, "cancelAcousticJob", &[captured]);
    call(&mut app, "cancelAcousticJob", &[change]);
    assert_eq!(probe(&mut app)["gains"], json!([1., 1., 1.]));
    let before = call(&mut app, "acousticStats", &[]).to_json()["scene"]["revision"].clone();
    for changes in [
        json!([{"id":99,"enabled":false}]),
        json!([{"id":7,"enabled":false},{"id":7,"enabled":true}]),
        json!([{"id":7,"transform":vec![0;16]}]),
    ] {
        assert!(invoke(&mut app, "beginUpdateAcousticScene", &[v(changes)]).is_err());
    }
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["scene"]["revision"],
        before
    );
}
#[test]
fn world_bound_mesh_coalesces_poses_follows_parent_visibility_and_disables_stale_entity() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    let initial = prepare(&mut app, "room.json");
    call(&mut app, "cancelAcousticJob", &[initial]);
    let entity = door(&mut app);
    call(
        &mut app,
        "bindAcousticMesh",
        &[v(
            json!({"mesh":7,"entity":entity,"follow_visibility":true}),
        )],
    );
    assert!(
        invoke(
            &mut app,
            "beginUpdateAcousticScene",
            &[v(json!([{"id":7,"enabled":false}]))]
        )
        .is_err()
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    for x in [20., 25., 30.] {
        world(
            &mut app,
            json!([{"op":"set_transform","id":20,"transform":{"translation":[x,0,0]}}]),
        );
        output(&mut app, 16);
    }
    let pending = call(&mut app, "acousticStats", &[]).to_json();
    assert_eq!(pending["instances"]["completed_updates"], 0);
    assert_eq!(pending["preparing"], true);
    gate.finish();
    let ready = instance_pump(&mut app, |s| {
        s["instances"]["completed_updates"].as_u64().unwrap_or(0) >= 2
            && !s["preparing"].as_bool().unwrap()
    });
    assert_eq!(
        ready["instances"]["bindings"][0]["desired"]["transform"],
        matrix(30.)
    );
    assert_eq!(probe(&mut app)["gains"], json!([1., 1., 1.]));
    world(
        &mut app,
        json!([{"op":"spawn","entity":{"id":21,"name":"parent","transform":{"translation":[-30,0,0]}}},{"op":"set_parent","id":20,"parent":21}]),
    );
    instance_pump(&mut app, |s| {
        s["instances"]["completed_updates"].as_u64().unwrap_or(0) >= 3
            && !s["preparing"].as_bool().unwrap()
    });
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    world(
        &mut app,
        json!([{"op":"set_visible","id":21,"visible":false}]),
    );
    instance_pump(&mut app, |s| s["scene"]["geometry"]["active_meshes"] == 0);
    assert_eq!(probe(&mut app)["gains"], json!([1., 1., 1.]));
    world(
        &mut app,
        json!([{"op":"set_visible","id":21,"visible":true}]),
    );
    instance_pump(&mut app, |s| s["scene"]["geometry"]["active_meshes"] == 1);
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    world(&mut app, json!([{"op":"despawn","id":20}]));
    let gone = instance_pump(&mut app, |s| s["scene"]["geometry"]["active_meshes"] == 0);
    assert!(
        gone["instances"]["bindings"][0]["error"]
            .as_str()
            .unwrap()
            .contains("unavailable")
    );
    call(&mut app, "unbindAcousticMesh", &[Value::Int(7)]);
    assert_eq!(
        call(&mut app, "acousticStats", &[]).to_json()["instances"]["bindings"],
        json!([])
    );
}
#[test]
fn canceled_updates_unbinding_and_restore_cannot_publish_obsolete_geometry() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(json!([{"id":7,"enabled":false}]))],
    );
    call(&mut app, "cancelAcousticJob", &[t.clone()]);
    gate.finish();
    assert!(invoke(&mut app, "acousticStatus", &[t]).is_err());
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    let entity = door(&mut app);
    let mut foreign = entity;
    foreign.client = 2;
    assert!(
        invoke(
            &mut app,
            "bindAcousticMesh",
            &[v(json!({"mesh":7,"entity":foreign}))]
        )
        .is_err()
    );
    call(
        &mut app,
        "bindAcousticMesh",
        &[v(json!({"mesh":7,"entity":entity}))],
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    world(
        &mut app,
        json!([{"op":"set_transform","id":20,"transform":{"translation":[30,0,0]}}]),
    );
    output(&mut app, 16);
    call(&mut app, "unbindAcousticMesh", &[Value::Int(7)]);
    gate.finish();
    output(&mut app, 16);
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    call(
        &mut app,
        "bindAcousticMesh",
        &[v(json!({"mesh":7,"entity":entity}))],
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    output(&mut app, 16);
    let save = app.runtime().unwrap().snapshot().unwrap();
    app.runtime_mut().unwrap().restore(&save).unwrap();
    gate.finish();
    output(&mut app, 0);
    let stats = call(&mut app, "acousticStats", &[]).to_json();
    assert!(stats["scene"].is_null());
    assert_eq!(stats["instances"]["bindings"], json!([]));
}

#[test]
fn moving_geometry_updates_real_source_filter_without_restarting_its_clock() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"spatial":{"position":[0,0,-4],"min_distance":10},"acoustics":{"radius":0,"air_absorption":[0,0,0]}}),
        )],
    );
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        output(&mut app, 0);
        if call(&mut app, "acousticVoice", &[h.clone()]).to_json()["result"]["path"].is_object() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    assert!(output(&mut app, 16).iter().flatten().all(|s| *s == 0.));
    let before = call(&mut app, "playbackInfo", &[h.clone()]).to_json()["age_frames"]
        .as_u64()
        .unwrap();
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(json!([{"id":7,"transform":matrix(30.)}]))],
    );
    let ready = wait(&mut app, &t);
    assert_eq!(ready["state"], "ready", "{ready}");
    let revision = ready["result"]["revision"].clone();
    call(&mut app, "cancelAcousticJob", &[t]);
    loop {
        output(&mut app, 0);
        if call(&mut app, "acousticVoice", &[h.clone()]).to_json()["result"]["revision"] == revision
        {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    let pcm = output(&mut app, 50);
    assert!(pcm.last().unwrap()[0] > 0.3);
    assert!(
        call(&mut app, "playbackInfo", &[h]).to_json()["age_frames"]
            .as_u64()
            .unwrap()
            > before
    );
}

#[test]
fn instance_motion_retains_completed_direct_snapshot_and_reports_geometry_lag() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
    let old = call(&mut app, "acousticStats", &[]).to_json()["scene"]["revision"].clone();
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"spatial":{"position":[0,0,-4]},"acoustics":{"air_absorption":[0,0,0]}}),
        )],
    );
    output(&mut app, 0);
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(json!([{"id":7,"transform":matrix(30.)}]))],
    );
    gate.finish();
    let ready = wait(&mut app, &t);
    assert_eq!(ready["state"], "ready", "{ready}");
    call(&mut app, "cancelAcousticJob", &[t]);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let info = call(&mut app, "acousticVoice", &[h.clone()]).to_json();
        if info["result"]["revision"] == old {
            assert_eq!(info["geometry_stale"], true);
            break;
        }
        assert!(Instant::now() < end, "{info}");
        std::thread::yield_now();
    }
    loop {
        output(&mut app, 0);
        let info = call(&mut app, "acousticVoice", &[h.clone()]).to_json();
        if info["result"]["revision"] == ready["result"]["revision"] {
            assert_eq!(info["geometry_stale"], false);
            break;
        }
        assert!(Instant::now() < end, "{info}");
        std::thread::yield_now();
    }
}

#[test]
fn foreign_entity_objects_cannot_be_reinterpreted_as_current_client_audio_entities() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
    let entity = {
        let cx = app.context();
        let w = cx.singleton("World").unwrap();
        cx.call(w, "spawn", &[v(json!({"name":"a"}))]).unwrap()
    };
    call(
        &mut app,
        "bindAcousticMesh",
        &[v(json!({"mesh":7,"entity":entity.to_json()}))],
    );
    app.context().state().unwrap().client = ClientId(2);
    {
        let cx = app.context();
        let w = cx.singleton("World").unwrap();
        cx.call(w, "spawn", &[v(json!({"name":"b"}))]).unwrap();
    }
    assert!(
        invoke(
            &mut app,
            "playSfx",
            &[v(json!({"asset":"tone.wav","follow":entity.to_json()}))]
        )
        .is_err()
    );
    assert!(
        invoke(
            &mut app,
            "bindAcousticMesh",
            &[v(json!({"mesh":7,"entity":entity.to_json()}))]
        )
        .is_err()
    );
}

#[test]
fn invalid_worker_geometry_retains_scene_without_retry_spin_and_recovers_on_changed_pose() {
    let (dir, mut app) = app();
    asset(&dir, "a", "room.json", wall());
    prepare(&mut app, "room.json");
    let initial = call(&mut app, "acousticStats", &[]).to_json();
    let entity = door(&mut app);
    call(
        &mut app,
        "bindAcousticMesh",
        &[v(json!({"mesh":7,"entity":entity}))],
    );
    world(
        &mut app,
        json!([{"op":"set_transform","id":20,"transform":{"translation":[999999,0,0]}}]),
    );
    let failed = instance_pump(&mut app, |s| {
        s["instances"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("world vertex"))
            && !s["preparing"].as_bool().unwrap()
    });
    assert_eq!(failed["scene"]["revision"], initial["scene"]["revision"]);
    assert_eq!(
        failed["host_geometry_reservation"],
        initial["host_geometry_reservation"]
    );
    let submitted = failed["instances"]["submitted_at_ms"].clone();
    for _ in 0..8 {
        output(&mut app, 16);
    }
    let same = call(&mut app, "acousticStats", &[]).to_json();
    assert_eq!(same["instances"]["submitted_at_ms"], submitted);
    assert_eq!(same["preparing"], false);
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    world(
        &mut app,
        json!([{"op":"set_transform","id":20,"transform":{"translation":[30,0,0]}}]),
    );
    let good = instance_pump(&mut app, |s| s["instances"]["completed_updates"] == 1);
    assert!(good["instances"]["error"].is_null());
    assert_eq!(probe(&mut app)["gains"], json!([1., 1., 1.]));
}

#[cfg(feature = "acoustics-obvhs")]
#[test]
fn optional_obvhs_scene_keeps_selected_backend_through_public_instance_commit() {
    let (dir, mut app) = app();
    let mut room = wall();
    room["backend"] = json!("obvhs");
    asset(&dir, "a", "room.json", room);
    prepare(&mut app, "room.json");
    assert_eq!(probe(&mut app)["gains"], json!([0., 0., 0.]));
    let t = call(
        &mut app,
        "beginUpdateAcousticScene",
        &[v(json!([{"id":7,"transform":matrix(30.)}]))],
    );
    let ready = wait(&mut app, &t);
    assert_eq!(ready["state"], "ready", "{ready}");
    assert_eq!(ready["result"]["backend"], "obvhs");
    assert_eq!(ready["result"]["geometry"]["reused_mesh_bvhs"], 1);
    call(&mut app, "cancelAcousticJob", &[t]);
    assert_eq!(probe(&mut app)["gains"], json!([1., 1., 1.]));
}
