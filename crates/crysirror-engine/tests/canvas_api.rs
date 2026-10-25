// SPDX-License-Identifier: MPL-2.0
//! Public engine capabilities exercised by a data-VM project with no native VM.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;

fn app(root: &std::path::Path) -> App {
    std::fs::create_dir_all(root.join("assets/a")).unwrap();
    std::fs::create_dir_all(root.join("assets/b")).unwrap();
    let project = json!({"schema":2,"name":"public-canvases","content_revision":"1",
        "canvas":[64,64],"assets":"assets","clients":[
            {"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},
            {"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]});
    std::fs::write(
        root.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&root.join("project.json"), &root.join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app
}
fn call(
    app: &mut App,
    class: &'static str,
    method: &str,
    args: serde_json::Value,
) -> anyhow::Result<Value> {
    let object = app.context().singleton(class)?;
    let args = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    app.context().call(object, method, &args)
}

#[test]
fn data_vm_client_uses_the_same_retained_canvas_state_and_transactional_commands() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let declarations = app.typescript_declarations().unwrap();
    assert!(declarations.contains("canvases: Canvases"));
    assert!(declarations.contains("media: Media"));
    call(&mut app,"Canvases","apply",json!([[
        {"op":"create","id":1,"width":16,"height":16},
        {"op":"clear","id":1,"color":[255,0,0,255]},
        {"op":"draw","id":1,"layer":{"id":101,"rect":[4,4,8,8],"paint":{"kind":"solid","color":[0,255,0,255]}}},
        {"op":"create","id":2,"width":16,"height":16},
        {"op":"copy","source":1,"destination":2,"rect":[0,0,16,16],"position":[0,0],"opacity":255,"replace":true},
        {"op":"present","id":2,"layer":42,"order":3}
    ]])).unwrap();
    let before = call(&mut app, "Canvases", "get", json!([2])).unwrap();
    assert_eq!(before.to_json()["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        before.to_json()["nodes"][0]["layer"]["paint"]["color"],
        json!([255, 0, 0, 255])
    );
    let frame = app
        .runtime()
        .unwrap()
        .host
        .engine
        .layers
        .frame([64, 64])
        .unwrap();
    assert!(
        frame.len() >= 3,
        "canvas must expose separate retained nodes"
    );
    assert!(frame.iter().any(|p| matches!(
        p.paint,
        crysirror_scene::layer::Paint::Solid {
            color: [0, 255, 0, 255]
        }
    )));
    call(
        &mut app,
        "Canvases",
        "apply",
        json!([[{"op":"clear","id":1,"color":[0,0,255,255]}]]),
    )
    .unwrap();
    assert_eq!(
        call(&mut app, "Canvases", "get", json!([2])).unwrap(),
        before,
        "copied draw state must be independent"
    );
    let stable = serde_json::to_value(&app.runtime().unwrap().host.engine).unwrap();
    assert!(call(&mut app,"Canvases","apply",json!([[
        {"op":"clear","id":2,"color":[0,0,255,255]},
        {"op":"copy","source":999,"destination":2,"rect":[0,0,1,1],"position":[0,0],"opacity":255}
    ]])).is_err());
    assert_eq!(
        serde_json::to_value(&app.runtime().unwrap().host.engine).unwrap(),
        stable
    );
    let a = app.runtime().unwrap().client_id("a").unwrap();
    let b = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = b;
    assert_eq!(
        call(&mut app, "Canvases", "get", json!([2])).unwrap(),
        Value::Null
    );
    call(
        &mut app,
        "Canvases",
        "apply",
        json!([[{"op":"create","id":2,"width":1,"height":1}]]),
    )
    .unwrap();
    assert_eq!(
        call(&mut app, "Canvases", "get", json!([2]))
            .unwrap()
            .to_json()["width"],
        1
    );
    app.context().state().unwrap().client = a;
    assert_eq!(
        call(&mut app, "Canvases", "get", json!([2])).unwrap(),
        before
    );
    assert!(call(&mut app, "Canvases", "get", json!([-1])).is_err());
}

#[test]
fn positioned_reveal_uses_public_layers_api_and_runtime_clock_for_data_vm_clients() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let layer = json!({"id":77,"rect":[0,0,64,64],"paint":{"kind":"glyph_run",
        "font":"deferred-font","size":24,"glyphs":[{"character":"A","baseline":[0,24]},
            {"character":"B","baseline":[20,24]}],
        "reveal":{"initial_visible":1,"interval_ms":20}}});
    call(&mut app, "Layers", "put", json!([layer])).unwrap();
    assert_eq!(
        call(&mut app, "Layers", "revealStatus", json!([77]))
            .unwrap()
            .to_json(),
        json!({"visible":1,"total":2,"complete":false})
    );
    app.frame(19, InputFrame::default()).unwrap();
    assert_eq!(
        call(&mut app, "Layers", "revealStatus", json!([77]))
            .unwrap()
            .to_json()["visible"],
        1
    );
    app.frame(1, InputFrame::default()).unwrap();
    assert_eq!(
        call(&mut app, "Layers", "revealStatus", json!([77]))
            .unwrap()
            .to_json()["complete"],
        true
    );
    call(&mut app, "Layers", "put", json!([layer])).unwrap();
    call(&mut app, "Layers", "completeReveal", json!([77])).unwrap();
    assert_eq!(
        call(&mut app, "Layers", "revealStatus", json!([77]))
            .unwrap()
            .to_json()["visible"],
        2
    );
    let b = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = b;
    assert_eq!(
        call(&mut app, "Layers", "revealStatus", json!([77])).unwrap(),
        Value::Null
    );
    assert!(call(&mut app, "Layers", "completeReveal", json!([77])).is_err());
}

#[test]
fn public_layer_transition_accepts_ordered_dither_without_a_native_vm() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    call(
        &mut app,
        "Layers",
        "transition",
        json!([{"duration_ms":2000,
        "effect":{"kind":"ordered_dither","cell":[1,1],"steps":1}}]),
    )
    .unwrap();
    let frame = app
        .runtime()
        .unwrap()
        .host
        .engine
        .layers
        .transition_frame()
        .unwrap();
    assert_eq!(
        frame.code,
        crysirror_scene::layer::ORDERED_DITHER_TRANSITION_CODE
    );
    assert_eq!(&frame.options[..4], &[1, 1, 0, 1]);
}

#[test]
fn public_media_resource_feeds_canvas_and_reports_playback_without_any_native_vm_or_device() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    std::fs::write(
        root.path().join("assets/a/movie.mpg"),
        include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg"),
    )
    .unwrap();
    assert_eq!(
        call(&mut app, "Media", "status", json!([3])).unwrap(),
        Value::Null
    );
    assert!(
        call(
            &mut app,
            "Media",
            "play",
            json!([3,"../movie.mpg",{"audio":false}])
        )
        .is_err()
    );
    assert!(
        call(
            &mut app,
            "Media",
            "play",
            json!([3,"movie.mpg",{"unexpected":true}])
        )
        .is_err()
    );
    assert!(call(&mut app, "Media", "play", json!([3,"movie.mpg",{"gain":9}])).is_err());
    let surface = call(
        &mut app,
        "Media",
        "play",
        json!([3,"movie.mpg",{"audio":false}]),
    )
    .unwrap();
    let surface = surface.as_str().unwrap().to_owned();
    assert!(surface.starts_with("__surface/"));
    call(&mut app,"Canvases","apply",json!([[
        {"op":"create","id":1,"width":64,"height":64},
        {"op":"draw","id":1,"layer":{"id":100,"rect":[0,0,64,64],"paint":{"kind":"image","asset":surface}}},
        {"op":"present","id":1,"layer":20}
    ]])).unwrap();
    let mut ended = false;
    for _ in 0..2000 {
        app.frame(16, InputFrame::default()).unwrap();
        let status = call(&mut app, "Media", "status", json!([3]))
            .unwrap()
            .to_json();
        if status["state"] == "ended" {
            assert!(status["published_frames"].as_u64().unwrap() > 0);
            assert!(status["decoded_frames"].as_u64().unwrap() > 0);
            assert_eq!(status["error"], serde_json::Value::Null);
            ended = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        ended,
        "shared media must finish even after the data VM halts"
    );
    let a = app.runtime().unwrap().client_id("a").unwrap();
    let b = app.runtime().unwrap().client_id("b").unwrap();
    let resource = app.runtime().unwrap().assets.surface(a, &surface).unwrap();
    assert!(
        resource.width > 1 && resource.height > 1,
        "decoded media must replace loading placeholder"
    );
    app.context().state().unwrap().client = b;
    assert_eq!(
        call(&mut app, "Media", "status", json!([3])).unwrap(),
        Value::Null
    );
    assert!(call(&mut app, "Media", "stop", json!([3])).is_err());
    app.context().state().unwrap().client = a;
    call(&mut app, "Media", "stop", json!([3])).unwrap();
    assert_eq!(
        call(&mut app, "Media", "status", json!([3]))
            .unwrap()
            .to_json()["state"],
        "stopped"
    );
}
