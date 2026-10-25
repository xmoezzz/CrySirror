// SPDX-License-Identifier: MPL-2.0
use anyhow::Result;
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, AppState, EngineContext, Lifecycle, Plugin, Value};
use crysirror_runtime::Runtime;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

fn project(dir: &Path, extra_clients: serde_json::Value) -> Runtime {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let mut clients =
        vec![serde_json::json!({"name":"game","adapter":"crysirror-script-v1","game":"demo"})];
    if let serde_json::Value::Array(a) = extra_clients {
        clients.extend(a);
    }
    let p = serde_json::json!({"schema":2,"name":"engine-test","content_revision":"1","canvas":[320,180],"assets":"assets","clients":clients});
    let path = dir.join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    Runtime::load(&path, &dir.join("state")).unwrap()
}

struct Recorder {
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
    fail_on_update: bool,
}
impl Recorder {
    fn push(&self, s: impl Into<String>) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.name, s.into()));
    }
}
impl Lifecycle for Recorder {
    fn name(&self) -> &str {
        self.name
    }
    fn on_init(&mut self, _: &mut EngineContext) -> Result<()> {
        self.push("init");
        Ok(())
    }
    fn on_start(&mut self, _: &mut EngineContext) -> Result<()> {
        self.push("start");
        Ok(())
    }
    fn on_fixed_update(&mut self, _: &mut EngineContext, step: u64) -> Result<()> {
        self.push(format!("fixed{step}"));
        Ok(())
    }
    fn on_update(&mut self, _: &mut EngineContext, dt: u64) -> Result<()> {
        self.push(format!("update{dt}"));
        anyhow::ensure!(!self.fail_on_update, "deliberate failure");
        Ok(())
    }
    fn on_late_update(&mut self, _: &mut EngineContext, _: u64) -> Result<()> {
        self.push("late");
        Ok(())
    }
    fn on_pause(&mut self, _: &mut EngineContext) -> Result<()> {
        self.push("pause");
        Ok(())
    }
    fn on_resume(&mut self, _: &mut EngineContext) -> Result<()> {
        self.push("resume");
        Ok(())
    }
    fn on_shutdown(&mut self, _: &mut EngineContext) -> Result<()> {
        self.push("shutdown");
        Ok(())
    }
}

#[test]
fn lifecycle_order_fixed_steps_pause_and_reverse_shutdown() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    for name in ["a", "b"] {
        app.add_behaviour(Box::new(Recorder {
            name,
            log: log.clone(),
            fail_on_update: false,
        }))
        .unwrap();
    }
    assert!(
        app.frame(16, InputFrame::default()).is_err(),
        "frame before start"
    );
    app.init().unwrap();
    app.start().unwrap();
    app.frame(40, InputFrame::default()).unwrap(); // 2 fixed steps, 8 ms carried
    app.frame(8, InputFrame::default()).unwrap(); // carry completes 1 step
    app.pause().unwrap();
    app.frame(1000, InputFrame::default()).unwrap(); // paused: nothing runs
    app.resume().unwrap();
    app.shutdown().unwrap();
    let log = log.lock().unwrap().join(" ");
    assert_eq!(
        log,
        "a:init b:init a:start b:start \
         a:fixed16 b:fixed16 a:fixed16 b:fixed16 a:update40 b:update40 a:late b:late \
         a:fixed16 b:fixed16 a:update8 b:update8 a:late b:late \
         a:pause b:pause a:resume b:resume b:shutdown a:shutdown"
    );
    assert_eq!(app.state(), AppState::Stopped);
    assert_eq!(app.engine().unwrap().frame, 2);
    assert_eq!(app.runtime().unwrap().now_ms(), 48);
}

#[test]
fn behaviour_error_faults_the_app_visibly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    app.add_behaviour(Box::new(Recorder {
        name: "bad",
        log,
        fail_on_update: true,
    }))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let err = app.frame(16, InputFrame::default()).unwrap_err();
    assert!(format!("{err:#}").contains("deliberate failure"));
    assert_eq!(app.state(), AppState::Faulted);
    assert!(app.fault().unwrap().contains("behaviour 'bad' on_update"));
    assert!(app.frame(16, InputFrame::default()).is_err());
}

#[test]
fn object_api_spawns_moves_and_invalidates_entities() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let spec = Value::from_json(
        serde_json::json!({"name":"box","geometry":"cube","position":[1,2,3],"color":[255,0,0,255]}),
    );
    let e = cx
        .call(world, "spawn", &[spec])
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        cx.get(e, "position").unwrap().as_vec3().unwrap(),
        [1., 2., 3.]
    );
    cx.set(e, "position", Value::vec3([4., 5., 6.])).unwrap();
    assert_eq!(
        cx.get(e, "position").unwrap().as_vec3().unwrap(),
        [4., 5., 6.]
    );
    let child_spec = Value::from_json(serde_json::json!({"name":"kid","parent":{"$object":e.0}}));
    let kid = cx
        .call(world, "spawn", &[child_spec])
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        cx.get(kid, "parent").unwrap().as_object().unwrap(),
        e,
        "interned handle"
    );
    // Read-only, unknown members and bad values fail without side effects.
    assert!(cx.set(e, "name", "x".into()).is_err());
    assert!(cx.call(e, "fly", &[]).is_err());
    assert!(cx.set(e, "scale", Value::vec3([0., 1., 1.])).is_err());
    assert!(cx
        .call(
            world,
            "spawn",
            &[Value::from_json(serde_json::json!({"bogus":1}))]
        )
        .is_err());
    assert_eq!(cx.get(world, "count").unwrap(), Value::Int(2));
    // Cascading destroy; both handles become stale.
    cx.call(e, "destroy", &[]).unwrap();
    assert_eq!(cx.get(e, "alive").unwrap(), Value::Bool(false));
    assert_eq!(cx.get(kid, "alive").unwrap(), Value::Bool(false));
    assert!(cx.get(kid, "position").is_err());
    assert_eq!(cx.get(world, "count").unwrap(), Value::Int(0));
    // A respawn with the reused id gets a new generation; the old object stays dead.
    let again = cx
        .call(
            world,
            "spawn",
            &[Value::from_json(serde_json::json!({"name":"box2"}))],
        )
        .unwrap()
        .as_object()
        .unwrap();
    assert_ne!(again, e);
    assert!(cx.get(e, "position").is_err());
}

struct Spin;
impl Lifecycle for Spin {
    fn name(&self) -> &str {
        "spin"
    }
    fn on_start(&mut self, cx: &mut EngineContext) -> Result<()> {
        let world = cx.singleton("World")?;
        cx.call(
            world,
            "spawn",
            &[Value::from_json(
                serde_json::json!({"name":"spinner","geometry":"cube"}),
            )],
        )?;
        Ok(())
    }
    fn on_fixed_update(&mut self, cx: &mut EngineContext, step: u64) -> Result<()> {
        let world = cx.singleton("World")?;
        let e = cx.call(world, "find", &["spinner".into()])?.as_object()?;
        let mut r = cx.get(e, "rotation")?.as_vec3()?;
        r[1] += step as f32 * 0.09; // 90 degrees per second
        cx.set(e, "rotation", Value::vec3(r))
    }
}
struct SpinPlugin;
impl Plugin for SpinPlugin {
    fn name(&self) -> &'static str {
        "spin"
    }
    fn build(&self, app: &mut App) -> Result<()> {
        app.add_behaviour(Box::new(Spin))?;
        Ok(())
    }
}

#[test]
fn plugins_drive_world_through_runtime_extraction() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    app.add_plugin(&SpinPlugin).unwrap();
    assert!(app.add_plugin(&SpinPlugin).is_err(), "duplicate plugin");
    app.init().unwrap();
    assert!(
        app.add_plugin(&SpinPlugin).is_err(),
        "configuration after init"
    );
    app.start().unwrap();
    for _ in 0..10 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let rt = app.runtime().unwrap();
    let scene = rt.host.engine.world.render_scene(None).unwrap().unwrap();
    assert_eq!(scene.meshes.len(), 1);
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let e = cx
        .call(world, "find", &["spinner".into()])
        .unwrap()
        .as_object()
        .unwrap();
    let r = cx.get(e, "rotation").unwrap().as_vec3().unwrap();
    assert!(
        (r[1] - 14.4).abs() < 1e-3,
        "10 fixed steps * 1.44 deg, got {r:?}"
    );
}

#[test]
fn filesystem_class_uses_client_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    std::fs::write(tmp.path().join("assets/hello.txt"), "hi").unwrap();
    let cx = app.context();
    let fs = cx.singleton("Fs").unwrap();
    assert_eq!(
        cx.call(fs, "readText", &["hello.txt".into()]).unwrap(),
        Value::Str("hi".into())
    );
    assert_eq!(
        cx.call(fs, "exists", &["../project.json".into()]).ok(),
        None
    );
    assert_eq!(
        cx.call(fs, "readSave", &["slot1.json".into()]).unwrap(),
        Value::Null
    );
    cx.call(fs, "writeSave", &["slot1.json".into(), "{\"v\":1}".into()])
        .unwrap();
    assert_eq!(
        cx.call(fs, "readSave", &["slot1.json".into()]).unwrap(),
        Value::Str("{\"v\":1}".into())
    );
}

#[test]
fn native_siglus_vm_is_a_peer_client_visible_through_the_object_api() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    std::fs::write(
        tmp.path().join("assets/scene.scn"),
        crysirror_siglus_fixture(),
    )
    .unwrap();
    let rt = project(
        tmp.path(),
        serde_json::json!([{"name":"siglus","adapter":"siglus","game":"fixture","config":{"scene":"scene.scn","format":"scn","string_codec":"plain"}}]),
    );
    let mut app = App::new(rt, "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(0, InputFrame::default()).unwrap();
    let cx = app.context();
    let engine = cx.singleton("Engine").unwrap();
    let Value::List(clients) = cx.call(engine, "clients", &[]).unwrap() else {
        panic!()
    };
    assert_eq!(clients.len(), 2);
    let siglus = clients[1].as_object().unwrap();
    assert_eq!(
        cx.get(siglus, "engine").unwrap(),
        Value::Str("siglus".into())
    );
    let diag = cx.get(siglus, "diagnostics").unwrap();
    assert_eq!(
        diag.field("int_stack").unwrap(),
        &Value::List(vec![Value::Int(42)])
    );
    // Siglus is in a native wait; App frames keep driving it to completion.
    for _ in 0..8 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    assert_eq!(
        cx.get(siglus, "state").unwrap(),
        Value::Str("Halted".into())
    );
}

/// Synthetic Siglus bytecode from crysirror-siglus-vm/src/fixture.rs.
fn crysirror_siglus_fixture() -> Vec<u8> {
    crysirror_siglus_vm::fixture::demo_scene()
}

#[test]
fn typescript_declarations_cover_registered_classes() {
    let tmp = tempfile::tempdir().unwrap();
    let app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    let dts = app.typescript_declarations().unwrap();
    assert!(dts.contains("declare class Entity {"));
    assert!(dts.contains("  position: Vec3;"));
    assert!(dts.contains("  readonly name: string;"));
    assert!(dts.contains(
        "  importModel(path: string, options: Record<string, any> | null): Record<string, any>;"
    ));
    assert!(dts.contains("declare const world: World;"));
}

#[test]
fn engine_material_api_validates_rolls_back_and_survives_save_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(project(tmp.path(), serde_json::json!([])), "game").unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let e=cx.call(world,"spawn",&[Value::from_json(serde_json::json!({"name":"glass","geometry":"cube","material":{"unlit":true,"alpha_mode":"blend","base_color_factor":[0.3,0.4,0.5,0.25],"depth_fade":2,"effect":{"kind":"custom","asset":"effects/soft.wgsl","parameters":[[1,2,3,4],[0,0,0,0],[0,0,0,0],[0,0,0,0]]}}}))]).unwrap().as_object().unwrap();
    assert!(
        cx.state_ref()
            .unwrap()
            .runtime
            .render_pbr()
            .unwrap()
            .is_some(),
        "material data selects the PBR path even without an explicit environment"
    );
    let before = cx.get(e, "material").unwrap().to_json();
    for invalid in [
        serde_json::json!({"alpha_mode":"guess"}),
        serde_json::json!({"base_color_factor":[1,1,1,2]}),
        serde_json::json!({"opacity_texture":"../private.png"}),
        serde_json::json!({"uv_scale":[1,2,3]}),
        serde_json::json!({"depth_fade":-1}),
    ] {
        assert!(cx
            .call(e, "setMaterial", &[Value::from_json(invalid)])
            .is_err());
        assert_eq!(cx.get(e, "material").unwrap().to_json(), before);
    }
    let save = cx.state_ref().unwrap().runtime.snapshot().unwrap();
    cx.call(e, "setMaterial", &[Value::Null]).unwrap();
    assert!(matches!(cx.get(e, "material").unwrap(), Value::Null));
    cx.state().unwrap().runtime.restore(&save).unwrap();
    assert_eq!(cx.get(e, "material").unwrap().to_json(), before);
}
