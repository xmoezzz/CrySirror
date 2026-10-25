// SPDX-License-Identifier: MPL-2.0
//! Screen transitions through the script API: world-time progress, save
//! round-trip, owner checks, generated Siglus masks (ADR 0022).
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

fn project(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"wipes","content_revision":"1","canvas":[64,48],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"wipes"}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    dir.join("project.json")
}

#[test]
fn scripted_transition_advances_with_world_time_and_ends() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(Runtime::load(&project(tmp.path()), &tmp.path().join("s")).unwrap(), "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let cx = app.context();
    let layers = cx.singleton("Layers").unwrap();
    assert_eq!(cx.get(layers, "activeTransition").unwrap(), Value::Null);
    let spec = |v: serde_json::Value| Value::from_json(v);
    // Invalid specs fail visibly.
    for bad in [
        serde_json::json!({"effect":{"kind":"crossfade"},"duration_ms":0}),
        serde_json::json!({"effect":{"kind":"siglus","code":300},"duration_ms":100}),
        serde_json::json!({"effect":{"kind":"mask","asset":"m.png","fade":9},"duration_ms":100}),
        serde_json::json!({"effect":{"kind":"wobble"},"duration_ms":100}),
    ] {
        assert!(cx.call(layers, "transition", &[spec(bad.clone())]).is_err(), "{bad}");
    }
    cx.call(layers, "transition", &[spec(serde_json::json!({"effect":{"kind":"siglus","code":5,"options":[0,0,16]},"duration_ms":320,"easing":"smooth"}))])
        .unwrap();
    for _ in 0..10 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    let t = cx.get(layers, "activeTransition").unwrap().to_json();
    assert_eq!(t["code"], 5);
    assert_eq!(t["elapsed_ms"], 160);
    assert_eq!(t["mine"], true);
    // smoothstep(0.5) = 0.5
    assert!((t["progress"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    // The generated pattern mask lands in the owner's surface slot once.
    let frame = app.runtime_mut().unwrap().render_transition().unwrap().unwrap();
    assert_eq!(frame.mask.as_deref(), Some("__surface/999"));
    let client = app.runtime().unwrap().client_id("game").unwrap();
    let mask = app.runtime().unwrap().assets.surface(client, "__surface/999").unwrap();
    assert_eq!((mask.width, mask.height), (64, 48));
    let distinct: std::collections::BTreeSet<u8> = mask.rgba.chunks_exact(4).map(|p| p[0]).collect();
    assert!(distinct.len() > 2, "pattern mask has a gradient of thresholds");
    // Saved with the engine state.
    let saved = serde_json::to_value(&app.runtime().unwrap().host.engine.layers).unwrap();
    assert_eq!(saved["transition"]["elapsed_ms"], 160);
    for _ in 0..10 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    assert_eq!(app.context().get(layers, "activeTransition").unwrap(), Value::Null);
    // Skip ends immediately.
    let cx = app.context();
    cx.call(layers, "transition", &[spec(serde_json::json!({"effect":{"kind":"crossfade"},"duration_ms":1000}))]).unwrap();
    cx.call(layers, "endTransition", &[]).unwrap();
    assert_eq!(cx.get(layers, "activeTransition").unwrap(), Value::Null);
}
