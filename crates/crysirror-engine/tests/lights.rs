// SPDX-License-Identifier: MPL-2.0
//! Local lights through the script API, world commands and PBR extraction.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use crysirror_scene::components::LightShape;

fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"lights","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"lights"}]});
    std::fs::write(tmp.path().join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let app = App::new(Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(), "game").unwrap();
    (tmp, app)
}

#[test]
fn script_lights_follow_transforms_units_and_toggles() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world, "setEnvironment", &[v(serde_json::json!({}))]).unwrap();
    let lamp = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "lamp", "position": [1, 2, 3],
            "light": {"color": [1, 0.5, 0.25], "intensity": 4.0 * std::f64::consts::PI, "units": "lumens", "range": 5}}))])
        .unwrap()
        .as_object()
        .unwrap();
    // A spot parented under a rotated pivot inherits the world transform.
    let pivot = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "pivot", "position": [0, 4, 0], "rotation": [-90, 0, 0]}))])
        .unwrap();
    let spot = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "spot",
            "light": {"shape": {"kind": "spot", "inner_degrees": 10, "outer_degrees": 30}, "intensity": 50}}))])
        .unwrap()
        .as_object()
        .unwrap();
    cx.call(spot, "setParent", &[pivot]).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let scene = app.runtime().unwrap().render_pbr().unwrap().unwrap();
    assert_eq!(scene.lights.len(), 2);
    let point = scene.lights.iter().find(|l| matches!(l.shape, LightShape::Point)).unwrap();
    assert_eq!(point.position, [1., 2., 3.]);
    // 4π·~1 lumens → ~1 candela per channel × color.
    assert!((point.intensity[0] - 1.).abs() < 1e-3 && (point.intensity[1] - 0.5).abs() < 1e-3, "{:?}", point.intensity);
    let s = scene.lights.iter().find(|l| matches!(l.shape, LightShape::Spot { .. })).unwrap();
    assert!((s.position[1] - 4.).abs() < 1e-4);
    assert!(s.direction[1] < -0.999, "spot axis follows parent rotation: {:?}", s.direction);

    // Script reads, disables, then removes.
    let cx = app.context();
    let got = cx.get(lamp, "light").unwrap().to_json();
    assert_eq!(got["units"], "lumens");
    cx.call(lamp, "setLight", &[v(serde_json::json!({"intensity": 3, "enabled": false}))]).unwrap();
    cx.call(spot, "setLight", &[Value::Null]).unwrap();
    // Invalid lights fail visibly instead of being clamped.
    assert!(cx
        .call(lamp, "setLight", &[v(serde_json::json!({"shape": {"kind": "spot", "inner_degrees": 40, "outer_degrees": 20}}))])
        .is_err());
    assert!(cx.call(lamp, "setLight", &[v(serde_json::json!({"intensity": -1}))]).is_err());
    assert!(cx.call(lamp, "setLight", &[v(serde_json::json!({"radius": 2}))]).is_err());
    app.frame(16, InputFrame::default()).unwrap();
    assert!(app.runtime().unwrap().render_pbr().unwrap().unwrap().lights.is_empty());
    let dts = app.typescript_declarations().unwrap();
    assert!(dts.contains("  setLight(") && dts.contains("  readonly light:"));
}

#[test]
fn script_reflection_probes_extract_world_positions_and_validate() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world, "setEnvironment", &[v(serde_json::json!({}))]).unwrap();
    let pivot = cx.call(world, "spawn", &[v(serde_json::json!({"name": "room", "position": [10, 0, 0]}))]).unwrap();
    let probe = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "capture", "position": [0, 2, 0],
            "reflectionProbe": {"extent": [4, 3, 5], "intensity": 0.5}}))])
        .unwrap()
        .as_object()
        .unwrap();
    cx.call(probe, "setParent", &[pivot]).unwrap();
    assert!(cx.call(world, "spawn", &[v(serde_json::json!({"name": "bad", "reflectionProbe": {"extent": [0, 1, 1]}}))]).is_err());
    assert!(cx.call(probe, "setReflectionProbe", &[v(serde_json::json!({"extent": [1, 1, 1], "radius": 3}))]).is_err());
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let scene = app.runtime().unwrap().render_pbr().unwrap().unwrap();
    assert_eq!(scene.probes.len(), 1);
    let p = &scene.probes[0];
    assert_eq!((p.position, p.extent, p.intensity, p.revision), ([10., 2., 0.], [4., 3., 5.], 0.5, 0));
    // Revision bump (recapture request) and removal.
    let cx = app.context();
    cx.call(probe, "setReflectionProbe", &[v(serde_json::json!({"extent": [4, 3, 5], "revision": 1}))]).unwrap();
    assert_eq!(cx.get(probe, "reflectionProbe").unwrap().to_json()["revision"], 1);
    cx.call(probe, "setReflectionProbe", &[Value::Null]).unwrap();
    assert!(app.runtime().unwrap().render_pbr().unwrap().unwrap().probes.is_empty());
}
