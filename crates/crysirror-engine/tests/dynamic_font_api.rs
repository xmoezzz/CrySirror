//! The public Text object reaches shared asynchronous fonts without a native VM.
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
use std::time::{Duration, Instant};
fn call(app: &mut App, name: &str, args: Json) -> anyhow::Result<Json> {
    let cx = app.context();
    let text = cx.singleton("Text")?;
    Ok(cx
        .call(
            text,
            name,
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
#[ignore = "requires licensed runtime system font; font bytes are never committed"]
fn text_object_registers_shared_fonts_and_checks_owner_before_measurement() {
    let d = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(d.path().join("assets").join(name)).unwrap();
    }
    std::fs::copy(
        std::env::var("CRYSIRROR_TEST_FONT").unwrap(),
        d.path().join("assets/a/runtime.ttf"),
    )
    .unwrap();
    std::fs::write(d.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"dynamic font API","content_revision":"1","canvas":[256,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]})).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&d.path().join("project.json"), &d.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let ticket = call(&mut app, "beginFont", json!(["game", "runtime.ttf"])).unwrap();
    let start = Instant::now();
    let font = loop {
        let status = call(&mut app, "fontStatus", json!([ticket])).unwrap();
        if status["state"] == "ready" {
            break status["font"].clone();
        }
        assert_ne!(status["state"], "failed", "{status}");
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::yield_now();
    };
    let alias = font["alias"].as_str().unwrap();
    let metrics = call(&mut app, "measureSimple", json!([alias, 24, "Hello"])).unwrap();
    assert!(metrics["advance"].as_f64().unwrap() > 0.);
    assert_eq!(
        call(&mut app, "resolveFont", json!(["game"])).unwrap(),
        font
    );
    let second = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = second;
    assert!(call(&mut app, "fontStatus", json!([ticket])).is_err());
    assert!(call(&mut app, "measureSimple", json!([alias, 24, "Hello"])).is_err());
    assert!(
        call(&mut app, "resolveFont", json!([alias]))
            .unwrap()
            .is_null()
    );
    let first = app.runtime().unwrap().client_id("a").unwrap();
    app.context().state().unwrap().client = first;
    call(&mut app, "releaseFontJob", json!([ticket])).unwrap();
    call(&mut app, "releaseFont", json!([font["handle"]])).unwrap();
    assert!(call(&mut app, "measureSimple", json!([alias, 24, "Hello"])).is_err());
}
