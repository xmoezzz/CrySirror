// SPDX-License-Identifier: MPL-2.0
//! Level sequences through the engine API (ADR 0040): camera path, entity
//! transform track, flickering light, events, pause, save and validation.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

fn project(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"seq","content_revision":"1","canvas":[64,48],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"seq","systems":{"entities":[
            {"id":1,"name":"gear","geometry":{"kind":"cube"}},
            {"id":2,"name":"torch","light":{"intensity":10,"range":5}}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    dir.join("project.json")
}

fn world(app: &App) -> serde_json::Value {
    serde_json::to_value(&app.runtime().unwrap().host.engine.world).unwrap()
}

#[test]
fn sequences_move_camera_entities_and_lights_and_fire_events() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(Runtime::load(&project(tmp.path()), &tmp.path().join("s")).unwrap(), "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let cx = app.context();
    let w = cx.singleton("World").unwrap();
    let j = |v: serde_json::Value| Value::from_json(v);
    let s = |v: &str| Value::Str(v.into());
    let seq = serde_json::json!({
        "duration_ms": 1600, "looping": true,
        "camera": {"keys":[{"time_ms":0,"eye":[0,2,10],"target":[0,0,0]},{"time_ms":1600,"eye":[10,2,0],"target":[0,0,0],"fov_y_degrees":30}],
                   "interpolation":"linear"},
        "tracks": [
            {"entity":1,"property":"rotation","keys":[{"time_ms":0,"value":[0,0,0]},{"time_ms":1600,"value":[0,360,0]}],"interpolation":"linear"},
            {"entity":2,"property":"light_intensity","keys":[{"time_ms":0,"value":[10]}],"noise":{"amplitude":4,"frequency_hz":8,"seed":1}}
        ],
        "events": [{"time_ms":800,"name":"clank"}]
    });
    // Camera tracks need a camera; unknown entities and light tracks on
    // non-lights fail at the call.
    assert!(cx.call(w, "playSequence", &[s("hall"), j(seq.clone())]).is_err());
    cx.call(w, "setCamera", &[j(serde_json::json!({"eye":[0,2,10],"target":[0,0,0]}))]).unwrap();
    let mut bad = seq.clone();
    bad["tracks"][1]["entity"] = 1.into();
    assert!(cx.call(w, "playSequence", &[s("hall"), j(bad)]).is_err());
    let mut bad = seq.clone();
    bad["tracks"][0]["entity"] = 9.into();
    assert!(cx.call(w, "playSequence", &[s("hall"), j(bad)]).is_err());
    cx.call(w, "playSequence", &[s("hall"), j(seq)]).unwrap();
    // 50 fixed steps = 800 ms: half way.
    let mut intensities = Vec::new();
    for _ in 0..50 {
        app.frame(16, InputFrame::default()).unwrap();
        intensities.push(world(&app)["clients"]["1"]["entities"]["2"]["spec"]["light"]["intensity"].as_f64().unwrap());
    }
    let state = world(&app);
    let rot = state["clients"]["1"]["entities"]["1"]["spec"]["transform"]["rotation_degrees"][1].as_f64().unwrap();
    assert!((rot - 180.).abs() < 1e-3, "gear half way: {rot}");
    let eye = &state["view"]["camera"]["eye"];
    assert!((eye[0].as_f64().unwrap() - 5.).abs() < 1e-3 && (eye[2].as_f64().unwrap() - 5.).abs() < 1e-3, "camera half way: {eye}");
    let (lo, hi) = intensities.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(hi - lo > 1. && lo >= 6. - 1e-6 && hi <= 14. + 1e-6, "torch flickers within ±4: {lo}..{hi}");
    let cx = app.context();
    assert_eq!(cx.call(w, "sequenceEvents", &[s("hall")]).unwrap(), Value::List(vec![s("clank")]));
    assert_eq!(cx.call(w, "sequenceEvents", &[s("hall")]).unwrap(), Value::List(vec![]), "events are consumed");
    // Saved with the world; a restored state passes validation.
    let saved = world(&app);
    assert_eq!(saved["clients"]["1"]["sequences"]["hall"]["time_ms"], 800.0);
    let restored: crysirror_scene::world::World = serde_json::from_value(saved).unwrap();
    restored.validate().unwrap();
    // Pausing the client holds the sequence.
    let cx = app.context();
    cx.call(w, "pause", &[Value::Bool(true)]).unwrap();
    app.frame(160, InputFrame::default()).unwrap();
    let cx = app.context();
    assert_eq!(cx.call(w, "sequenceStatus", &[s("hall")]).unwrap().to_json()["time_ms"], 800.0);
    cx.call(w, "pause", &[Value::Bool(false)]).unwrap();
    // Looping wraps; stopping removes it.
    for _ in 0..60 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    let st = cx.call(w, "sequenceStatus", &[s("hall")]).unwrap().to_json();
    assert_eq!(st["loops"], 1);
    assert_eq!(st["playing"], true);
    cx.call(w, "stopSequence", &[s("hall")]).unwrap();
    assert_eq!(cx.call(w, "sequenceStatus", &[s("hall")]).unwrap(), Value::Null);
}
