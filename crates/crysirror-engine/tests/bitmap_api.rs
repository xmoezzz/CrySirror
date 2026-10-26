// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{service::InputFrame, vfs::MemoryProvider};
use crysirror_engine::{App, Value};
use crysirror_image::{ImageSurface, encode_png};
use crysirror_runtime::Runtime;
use serde_json::json;
use std::sync::Arc;

fn app(root: &std::path::Path) -> App {
    std::fs::create_dir(root.join("assets")).unwrap();
    let project = json!({"schema":2,"name":"bitmaps","content_revision":"1","canvas":[8,8],"assets":"assets",
        "clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]});
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
fn call(app: &mut App, name: &str, args: serde_json::Value) -> anyhow::Result<Value> {
    let cx = app.context();
    let bitmap = cx.singleton("Bitmap")?;
    let values: Vec<_> = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect();
    cx.call(bitmap, name, &values)
}
fn pixels(app: &App, slot: u32) -> Vec<u8> {
    let runtime = app.runtime().unwrap();
    runtime
        .assets
        .surface(
            runtime.client_id("a").unwrap(),
            &format!("__surface/{slot}"),
        )
        .unwrap()
        .rgba
        .to_vec()
}

#[test]
fn associated_composition_filters_and_failures_use_client_scoped_engine_surfaces() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    call(&mut app, "create", json!([1, 1, 1, [64, 32, 16, 255]])).unwrap();
    call(&mut app, "create", json!([2, 1, 1, [200, 100, 50, 71]])).unwrap();
    call(&mut app, "create", json!([3, 1, 1, [0, 0, 0, 128]])).unwrap();
    call(&mut app, "composite", json!([2,"__surface/1",{"source_rect":{"x":0,"y":0,"width":1,"height":1},"destination":[0,0],"weight":128,"coverage_write":"source"},"__surface/3"])).unwrap();
    assert_eq!(pixels(&app, 2), [182, 91, 45, 128]);
    call(
        &mut app,
        "filterRgb",
        json!([2,{"x":0,"y":0,"width":1,"height":1},{"kind":"fade","rgb":[0,255,0],"weight":128}]),
    )
    .unwrap();
    assert_eq!(pixels(&app, 2), [91, 173, 22, 128]);
    let before = pixels(&app, 2);
    let info = call(&mut app, "info", json!([2])).unwrap();
    assert!(
        call(
            &mut app,
            "filterRgb",
            json!([2,{"x":0,"y":0,"width":1,"height":1},{"kind":"multiply","factors":[256,257,0]}])
        )
        .is_err()
    );
    assert_eq!(pixels(&app, 2), before);
    assert_eq!(call(&mut app, "info", json!([2])).unwrap(), info);
    let b = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = b;
    call(&mut app, "create", json!([9, 1, 1, [0, 0, 0, 0]])).unwrap();
    assert!(call(&mut app,"composite",json!([9,"__surface/1",{"source_rect":{"x":0,"y":0,"width":1,"height":1},"destination":[0,0]}])).is_err());
}

#[test]
fn provider_decode_self_copy_and_failed_mutation_are_real_client_surface_operations() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let src = ImageSurface {
        width: 3,
        height: 1,
        rgba: vec![10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255],
    };
    let mut provider = MemoryProvider::new();
    provider
        .insert("source.png", encode_png(&src).unwrap())
        .unwrap();
    // A disk file of the same name is deliberately invalid: provider must win.
    std::fs::write(tmp.path().join("assets/source.png"), b"invalid disk PNG").unwrap();
    let a = app.runtime().unwrap().client_id("a").unwrap();
    app.runtime_mut()
        .unwrap()
        .host
        .fs
        .mount_provider(a, "", Arc::new(provider))
        .unwrap();
    assert_eq!(
        call(&mut app, "load", json!([7, "source.png"]))
            .unwrap()
            .as_str()
            .unwrap(),
        "__surface/7"
    );
    assert_eq!(pixels(&app, 7), src.rgba);
    app.runtime_mut()
        .unwrap()
        .assets
        .register_generated(a, "__crysirror/native.png", encode_png(&src).unwrap())
        .unwrap();
    call(&mut app, "load", json!([8, "__crysirror/native.png"])).unwrap();
    assert_eq!(pixels(&app, 8), src.rgba);
    call(&mut app,"blit",json!([7,"__surface/7",{"source_rect":{"x":0,"y":0,"width":2,"height":1},"destination":[1,0]}])).unwrap();
    assert_eq!(
        pixels(&app, 7),
        [10, 0, 0, 255, 10, 0, 0, 255, 20, 0, 0, 255]
    );
    let before = pixels(&app, 7);
    let info = call(&mut app, "info", json!([7])).unwrap();
    assert!(call(&mut app,"blit",json!([7,"__surface/7",{"source_rect":{"x":0,"y":0,"width":3,"height":1},"destination":[0,0],"opacity":100,"mode":"premultiplied_mask256"}])).is_err());
    assert!(call(&mut app, "load", json!([7, "../outside.png"])).is_err());
    assert_eq!(pixels(&app, 7), before);
    assert_eq!(call(&mut app, "info", json!([7])).unwrap(), info);
    call(
        &mut app,
        "fill",
        json!([7,{"x":-1,"y":0,"width":2,"height":1},[99,88,77,66]]),
    )
    .unwrap();
    assert_eq!(&pixels(&app, 7)[..4], &[99, 88, 77, 66]);

    let b = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = b;
    assert!(call(&mut app, "info", json!([7])).is_err());
    assert!(call(&mut app, "load", json!([1, "__surface/7"])).is_err());
    assert!(call(&mut app, "load", json!([1, "__crysirror/native.png"])).is_err());
    assert_eq!(
        call(&mut app, "remove", json!([7])).unwrap(),
        Value::Bool(false)
    );
    call(&mut app, "create", json!([7, 1, 1, [1, 2, 3, 4]])).unwrap();
    app.context().state().unwrap().client = a;
    assert_eq!(&pixels(&app, 7)[..4], &[99, 88, 77, 66]);
}

#[test]
fn scaling_output_feeds_existing_layer_rotation_and_filter_without_a_native_renderer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    call(&mut app, "create", json!([1, 2, 1, [255, 0, 0, 255]])).unwrap();
    call(
        &mut app,
        "fill",
        json!([1,{"x":1,"y":0,"width":1,"height":1},[0,0,255,128]]),
    )
    .unwrap();
    call(&mut app, "create", json!([2, 4, 1])).unwrap();
    call(&mut app,"stretch",json!([2,"__surface/1",{"source_rect":{"x":0,"y":0,"width":2,"height":1},"destination":{"x":0,"y":0,"width":4,"height":1},"sampling":"nearest"}])).unwrap();
    assert_eq!(
        pixels(&app, 2),
        [
            255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 128, 0, 0, 255, 128
        ]
    );
    let cx = app.context();
    let layers = cx.singleton("Layers").unwrap();
    cx.call(
        layers,
        "put",
        &[Value::from_json(
            json!({"id":90,"rect":[0,0,4,1],"paint":{"kind":"image","asset":"__surface/2"}}),
        )],
    )
    .unwrap();
    cx.call(
        layers,
        "setTransform",
        &[
            90_i64.into(),
            Value::from_json(json!({"rotate":1.5707964,"pivot_x":2,"pivot_y":0.5})),
        ],
    )
    .unwrap();
    cx.call(
        layers,
        "setFilter",
        &[90_i64.into(), Value::from_json(json!({"mono":1.0}))],
    )
    .unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let runtime = app.runtime().unwrap();
    let frame = runtime.host.engine.layers.frame([8, 8]).unwrap();
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].filter.as_ref().unwrap().mono, 1.0);
    assert!(
        matches!(&frame[0].paint,crysirror_scene::layer::Paint::Image {asset,..} if asset == "__surface/2")
    );
    assert!(frame[0].quad.iter().flatten().all(|v| v.is_finite()));
    let declarations = app.typescript_declarations().unwrap();
    assert!(declarations.contains("Bitmap"));
    assert!(declarations.contains("declare const bitmap"));
}
