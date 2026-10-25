// SPDX-License-Identifier: MPL-2.0
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, AppState, ScriptHost};
use crysirror_runtime::Runtime;
use crysirror_script_js::JsHost;
use std::path::Path;

fn app(dir: &Path) -> App {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"js-test","content_revision":"1","canvas":[320,180],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"demo"}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    App::new(
        Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap(),
        "game",
    )
    .unwrap()
}

const GAME_TS: &str = r#"
interface Spin { speed: number }
let cube: Entity | null = null;
const spin: Spin = { speed: 0.09 };
function onStart(): void {
  cube = world.spawn({ name: "cube", geometry: "cube", position: [0, 1, 0], color: [40, 160, 255, 255] });
  engine.log(`spawned ${cube.name} #${cube.id}`);
}
function onFixedUpdate(step: number): void {
  const r = cube!.rotation;
  cube!.rotation = [r[0], r[1] + step * spin.speed, r[2]];
}
function onLateUpdate(dt: number): void {
  if (engine.frame === 2) { engine.log(`t=${engine.timeMs} count=${world.count}`); }
}
"#;

#[test]
fn typescript_behaviour_drives_world_through_object_api() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    app.add_script_host(Box::new(
        JsHost::with_source("game", "game.ts", GAME_TS).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    for _ in 0..4 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let log: Vec<_> = app.engine().unwrap().log.iter().cloned().collect();
    assert_eq!(log, vec!["spawned cube #1", "t=48 count=1"]);
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let cube = cx
        .call(world, "find", &["cube".into()])
        .unwrap()
        .as_object()
        .unwrap();
    let r = cx.get(cube, "rotation").unwrap().as_vec3().unwrap();
    assert!(
        (r[1] - 5.76).abs() < 1e-3,
        "4 steps * 16 ms * 0.09, got {r:?}"
    );
}

#[test]
fn engine_errors_become_catchable_js_exceptions_and_uncaught_ones_fault() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let src = r#"
      function onStart() {
        const e = world.spawn({ name: "a" });
        e.destroy();
        try { e.position = [1, 2, 3]; } catch (err) { engine.log("caught: " + err.message); }
        try { e.name = "x"; } catch (err) { engine.log("readonly: " + err.message); }
      }
      function onUpdate() { world.spawn({ nope: true }); }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("game", "game.js", src).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let log: Vec<_> = app.engine().unwrap().log.iter().cloned().collect();
    assert!(
        log[0].starts_with("caught: ") && log[0].contains("world entity missing"),
        "{log:?}"
    );
    // Sloppy-mode scripts still see read-only writes rejected by the engine.
    assert!(log[1].contains("Entity.name is read-only"), "{log:?}");
    let err = app.frame(16, InputFrame::default()).unwrap_err();
    assert!(
        format!("{err:#}").contains("unknown spawn field 'nope'"),
        "{err:#}"
    );
    assert_eq!(app.state(), AppState::Faulted);
}

#[test]
fn runaway_loops_are_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let mut host =
        JsHost::with_source("game", "loop.js", "function onUpdate() { for (;;) {} }").unwrap();
    host.loop_limit = 10_000;
    app.add_script_host(Box::new(host)).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    assert!(app.frame(16, InputFrame::default()).is_err());
    assert_eq!(app.state(), AppState::Faulted);
}

#[test]
fn script_imports_blender_gltf_model() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let glb =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/engine/assets/lantern.glb");
    std::fs::copy(glb, tmp.path().join("assets/lantern.glb")).unwrap();
    let src = r#"
      function onStart(): void {
        const anchor = world.spawn({ name: "anchor", position: [2, 0, 0] });
        const result = world.importModel("lantern.glb", { parent: anchor });
        engine.log(JSON.stringify(result.report));
        const base = result.roots[0] as Entity;
        engine.log(base.name + ">" + base.children().map((c: Entity) => c.name).join(","));
        engine.log(String(world.count));
      }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("game", "import.ts", src).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let log: Vec<_> = app.engine().unwrap().log.iter().cloned().collect();
    let report: serde_json::Value = serde_json::from_str(&log[0]).unwrap();
    assert_eq!(report["nodes"], 3);
    assert_eq!(report["primitives"], 3);
    assert!(report["triangles"].as_u64().unwrap() > 12);
    assert_eq!(report["dropped"], serde_json::json!([]));
    assert_eq!(log[1], "lantern:base>lantern:body");
    assert_eq!(log[2], "4");
    let scene = app
        .runtime()
        .unwrap()
        .host
        .engine
        .world
        .render_scene(None)
        .unwrap()
        .unwrap();
    assert_eq!(scene.meshes.len(), 3);
    // Blender's +Z-up roof apex at z=1.6 becomes glTF +Y-up; the anchor adds +2 X.
    let top = scene
        .meshes
        .iter()
        .find(|m| m.name.ends_with("lantern:roof"))
        .unwrap();
    let max_y = serde_json::to_value(&top.geometry).unwrap()["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v[1].as_f64().unwrap())
        .fold(f64::MIN, f64::max);
    assert!((max_y - 1.6).abs() < 1e-3, "roof apex y {max_y}");
}

#[test]
fn sources_must_be_added_before_init() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let mut host = JsHost::new("game");
    assert!(
        host.add_source("bad.ts", "let x: = ;").is_err(),
        "TS parse errors surface"
    );
    host.add_source("ok.js", "var ready = true;").unwrap();
    assert_eq!(host.language(), "javascript");
    app.add_script_host(Box::new(host)).unwrap();
    app.init().unwrap();
}

#[test]
fn scripts_reach_layers_ui_camera_environment_and_save_load() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let src = r#"
      let box: Entity;
      function onStart(): void {
        layers.put({ id: 1, rect: [10, 10, 100, 40], paint: { kind: "solid", color: [255, 0, 0, 255] } });
        layers.setOpacity(1, 128);
        ui.put({ id: 7, rect: [0, 0, 80, 20], kind: "panel" });
        ui.setVisible(7, false);
        try { ui.setText(7, "Start", null); } catch (e) { engine.log("ui: " + e.message); }
        world.setCamera({ eye: [0, 2, 5], target: [0, 0, 0], fov: 60 });
        world.setEnvironment({ exposure: 1.0, tonemap: "aces" });
        box = world.spawn({ name: "box", geometry: "cube", position: [1, 0, 0] });
        engine.log(`layers=${layers.count} cam=${JSON.stringify(world.camera.eye)} env=${world.environment.tonemap}`);
        engine.log(`saved=${save.save("slot1.bin") > 0}`);
        box.position = [9, 9, 9];
        save.load("slot1.bin");
        engine.log(`restored=${JSON.stringify(box.position)} exists=${save.exists("slot1.bin")}`);
      }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("game", "svc.ts", src).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let log: Vec<_> = app.engine().unwrap().log.iter().cloned().collect();
    assert!(log[0].starts_with("ui: "), "{log:?}");
    assert_eq!(log[1], "layers=1 cam=[0,2,5] env=aces");
    assert_eq!(log[2], "saved=true");
    assert_eq!(log[3], "restored=[1,0,0] exists=true");
}

#[test]
fn script_controls_a_character_with_input_actions_gravity_and_raycasts() {
    use crysirror_core::service::buttons;
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let src = r#"
      let hero: Entity; let vy = 0; let jumps = 0;
      function onStart(): void {
        input.bind("forward", ["w"]); input.bind("jump", ["space"]);
        world.spawn({ name: "floor", geometry: "cube", scale: [40, 1, 40], position: [0, -0.5, 0], collider: { shape: { kind: "mesh" } } });
        hero = world.spawn({ name: "hero", position: [0, 2, 0], collider: { shape: { kind: "capsule", radius: 0.3, half_height: 0.5 }, body: "character" } });
      }
      function onFixedUpdate(ms: number): void {
        const dt = ms / 1000;
        vy -= 20 * dt;
        const m = hero.moveCharacter([0, vy * dt, input.axis("none", "forward") * 3 * dt], dt, null);
        if (m.grounded) { if (input.pressed("jump")) { vy = 6; jumps++; } else { vy = -1; } }
      }
      function onLateUpdate(): void {
        const hit = world.raycast(hero.position, [0, -1, 0], 10, hero);
        engine.log(`${hero.position[1].toFixed(2)} ${hero.position[2].toFixed(2)} ${hit ? hit.distance.toFixed(2) : "none"} ${jumps}`);
      }
    "#;
    let mut host = JsHost::with_source("game", "char.ts", src).unwrap();
    host.add_source("none.js", "input.bind('none', ['s']);")
        .unwrap();
    app.add_script_host(Box::new(host)).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    for _ in 0..30 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let idle: Vec<f32> = app
        .engine()
        .unwrap()
        .log
        .back()
        .unwrap()
        .split(' ')
        .map(|x| x.parse().unwrap_or(-1.))
        .collect();
    // Feet on the floor within Rapier's 2 cm controller skin offset.
    assert!(
        (idle[0] - 0.8).abs() < 0.03
            && idle[1] == 0.
            && (idle[2] - idle[0]).abs() < 1e-3
            && idle[3] == 0.,
        "landed: {idle:?}"
    );
    for i in 0..50 {
        let f = InputFrame {
            buttons_down: buttons::KEY_W,
            buttons_pressed: if i == 0 { buttons::KEY_W } else { 0 },
            ..Default::default()
        };
        app.frame(16, f).unwrap();
    }
    let walked: Vec<f32> = app
        .engine()
        .unwrap()
        .log
        .back()
        .unwrap()
        .split(' ')
        .map(|x| x.parse().unwrap_or(-1.))
        .collect();
    assert!((walked[1] - 2.4).abs() < 0.1, "walked {walked:?}");
    app.frame(
        16,
        InputFrame {
            buttons_down: buttons::SPACE,
            buttons_pressed: buttons::SPACE,
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..5 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let air: Vec<f32> = app
        .engine()
        .unwrap()
        .log
        .back()
        .unwrap()
        .split(' ')
        .map(|x| x.parse().unwrap_or(-1.))
        .collect();
    assert!(air[0] > 1.1 && air[3] == 1., "jumped {air:?}");
}

#[test]
fn typescript_materials_use_the_shared_engine_contract_and_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let source = r#"
 function onStart():void{
  const pane=world.spawn({name:"pane",geometry:"cube",material:{alpha_mode:"blend",unlit:true,base_color_factor:[0.2,0.4,0.8,0.5]}});
  pane.setMaterial({alpha_mode:"additive",unlit:true,base_color_factor:[0.1,0.2,0.4,0.3],effect:{kind:"fog",density:2}});
  save.save("material.bin");pane.setMaterial(null);save.load("material.bin");
  engine.log(pane.material.alpha_mode);
  engine.log(pane.material.effect.kind);
  try{pane.setMaterial({opacity_texture:"../escape.png"});}catch(e){engine.log("rejected");}
  engine.log(String(Math.round(pane.material.base_color_factor[3]*100)));
 }
 "#;
    app.add_script_host(Box::new(
        JsHost::with_source("material", "material.ts", source).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let log: Vec<_> = app.engine().unwrap().log.iter().cloned().collect();
    assert_eq!(log, vec!["additive", "fog", "rejected", "30"]);
}
