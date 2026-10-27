// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared capabilities used by native scripts, exercised without a native VM.
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
fn app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"shared native prerequisites","content_revision":"1","assets":"assets","canvas":[640,480],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"test"}]})).unwrap()).unwrap();
    let a = App::new(Runtime::load(&path, &d.path().join("state")).unwrap(), "a").unwrap();
    (d, a)
}
fn call(a: &mut App, class: &'static str, method: &str, args: Json) -> anyhow::Result<Json> {
    let cx = a.context();
    let h = cx.singleton(class)?;
    Ok(cx
        .call(
            h,
            method,
            &args
                .as_array()
                .unwrap()
                .iter()
                .cloned()
                .map(Value::from_json)
                .collect::<Vec<_>>(),
        )?
        .to_json())
}
#[test]
fn desktop_window_policy_is_engine_owned_validated_and_saved_without_any_vm() {
    let (_d, mut a) = app();
    call(&mut a, "Ui", "setWindowPolicy", json!([false, true])).unwrap();
    let p = a.runtime().unwrap().host.engine.ui.window_policy.unwrap();
    assert_eq!(p.0, 1);
    assert!(!p.1.decorated && p.1.resizable);
    call(&mut a, "Ui", "setFullscreen", json!([true])).unwrap();
    assert_eq!(
        a.runtime().unwrap().host.engine.ui.fullscreen,
        Some((1, true))
    );
    let saved = a.runtime().unwrap().snapshot().unwrap();
    call(&mut a, "Ui", "setWindowPolicy", json!([true, false])).unwrap();
    a.runtime_mut().unwrap().restore(&saved).unwrap();
    assert_eq!(a.runtime().unwrap().host.engine.ui.window_policy, Some(p));
    assert_eq!(
        a.runtime().unwrap().host.engine.ui.fullscreen,
        Some((1, true))
    );
    assert!(call(&mut a, "Ui", "setWindowPolicy", json!([1, false])).is_err());
    assert_eq!(a.runtime().unwrap().host.engine.ui.window_policy, Some(p));
}
#[test]
#[ignore = "requires explicitly supplied system font at runtime"]
fn public_positions_share_font_metrics_and_feed_retained_bold_glyph_paint() {
    let (_d, mut a) = app();
    let bytes =
        std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").expect("system font path")).unwrap();
    a.runtime_mut()
        .unwrap()
        .assets
        .text
        .add_font("default", bytes)
        .unwrap();
    let text = "AV 日本";
    let p = call(
        &mut a,
        "Text",
        "positionSimple",
        json!(["default", 24, text]),
    )
    .unwrap();
    assert_eq!(p.as_array().unwrap().len(), text.chars().count());
    let m = call(
        &mut a,
        "Text",
        "measureSimple",
        json!(["default", 24, text]),
    )
    .unwrap();
    assert_eq!(p[0]["baseline"][1], m["ascent"]);
    assert!(
        p.as_array()
            .unwrap()
            .windows(2)
            .all(|w| w[1]["baseline"][0].as_f64().unwrap() > w[0]["baseline"][0].as_f64().unwrap())
    );
    let state = a.context().state().unwrap();
    state.runtime.host.text = state.runtime.assets.text.clone();
    call(&mut a,"Canvases","apply",json!([[{"op":"create","id":3,"width":300,"height":80},{"op":"draw","id":3,"layer":{"id":0,"rect":[0,0,300,80],"paint":{"kind":"glyph_run","font":"default","size":24,"bold":true,"glyphs":p}}},{"op":"present","id":3,"layer":7,"order":0}]])).unwrap();
    assert!(
        a.runtime()
            .unwrap()
            .host
            .engine
            .layers
            .get(crysirror_core::state::ClientId(1), 7)
            .is_some()
    );
    assert!(
        call(
            &mut a,
            "Text",
            "positionSimple",
            json!(["default", 24.5, "bad"])
        )
        .is_err()
    );
    assert!(
        call(
            &mut a,
            "Text",
            "positionSimple",
            json!(["default", 24, "bad\nline"])
        )
        .is_err()
    );
}
