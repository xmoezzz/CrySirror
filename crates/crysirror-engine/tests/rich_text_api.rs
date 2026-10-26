// SPDX-License-Identifier: MPL-2.0
//! Runtime font loading only. This suite never ships/copies a font asset.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;

#[test]
#[ignore = "requires an explicitly supplied system font; no fonts are distributed"]
fn shaped_surface_typewriter_localization_and_client_isolation() {
    let font = std::env::var("CRYSIRROR_TEST_FONT")
        .expect("set CRYSIRROR_TEST_FONT to a licensed system font");
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"rich text","content_revision":"1","assets":"assets","canvas":[320,96],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.runtime_mut()
        .unwrap()
        .assets
        .text
        .add_font("latin", std::fs::read(font).unwrap())
        .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let cx = app.context();
    let text = cx.singleton("Text").unwrap();
    let loc = cx.singleton("Localization").unwrap();
    let metrics = cx.call(text, "measureSimple", &[
        Value::Str("latin".into()), Value::Float(24.), Value::Str("AVATAR".into())
    ]).unwrap().to_json();
    assert!(metrics["advance"].as_f64().unwrap() > 0.);
    assert!(metrics["line_height"].as_f64().unwrap() > 0.);
    cx.call(
        loc,
        "define",
        &[
            Value::Str("en".into()),
            Value::from_json(json!({"strings":{},"fonts":["latin"]})),
        ],
    )
    .unwrap();
    let options = Value::from_json(
        json!({"layout":{"canvas":[320,96]},"raster":{"outline":{"width":1,"color":[0,0,0,255]},"shadow":{"offset":[2,2],"blur_radius":1,"color":[0,0,0,128]}},"graphemes_per_second":10}),
    );
    let path = cx
        .call(
            text,
            "draw",
            &[
                Value::Str("line".into()),
                Value::Str("[ruby=hello]Hi[/ruby] [b]office[/b][event=done]".into()),
                options,
            ],
        )
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let client = app.runtime().unwrap().client_id("a").unwrap();
    assert!(app
        .runtime()
        .unwrap()
        .assets
        .surface(client, &path)
        .unwrap()
        .rgba
        .chunks_exact(4)
        .all(|p| p[3] == 0));
    app.frame(100, InputFrame::default()).unwrap();
    let cx = app.context();
    let saved = cx
        .call(text, "state", &[Value::Str("line".into())])
        .unwrap();
    assert_eq!(saved.to_json()["reveal"]["revealed"], 1);
    cx.call(
        text,
        "pause",
        &[Value::Str("line".into()), Value::Bool(true)],
    )
    .unwrap();
    app.frame(100, InputFrame::default()).unwrap();
    let cx = app.context();
    assert_eq!(
        cx.call(text, "state", &[Value::Str("line".into())])
            .unwrap()
            .to_json()["reveal"]["revealed"],
        1
    );
    cx.call(text, "finish", &[Value::Str("line".into())])
        .unwrap();
    let events = cx
        .call(text, "drainEvents", &[Value::Str("line".into())])
        .unwrap()
        .to_json();
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "marker" && e["name"] == "done"));
    assert!(app
        .runtime()
        .unwrap()
        .assets
        .surface(client, &path)
        .unwrap()
        .rgba
        .chunks_exact(4)
        .any(|p| p[3] > 0));
    let cx = app.context();
    cx.call(text, "restore", &[Value::Str("line".into()), saved])
        .unwrap();
    assert_eq!(
        cx.call(text, "state", &[Value::Str("line".into())])
            .unwrap()
            .to_json()["reveal"]["revealed"],
        1
    );
    assert!(cx
        .call(
            text,
            "draw",
            &[
                Value::Str("bad".into()),
                Value::Str("[icon=../private.png,8,8]".into())
            ]
        )
        .is_err());
    let b = cx.state().unwrap().runtime.client_id("b").unwrap();
    cx.state().unwrap().client = b;
    assert!(cx
        .call(text, "state", &[Value::Str("line".into())])
        .is_err());
    assert!(cx
        .state()
        .unwrap()
        .runtime
        .assets
        .surface(b, &path)
        .is_none());
    cx.state().unwrap().client = client;
    cx.call(text, "remove", &[Value::Str("line".into())])
        .unwrap();
    assert!(cx
        .state()
        .unwrap()
        .runtime
        .assets
        .surface(client, &path)
        .is_none());
    cx.call(
        text,
        "draw",
        &[
            Value::Str("overflow".into()),
            Value::Str("X".repeat(5000)),
            Value::from_json(json!({"graphemes_per_second":10000})),
        ],
    )
    .unwrap();
    let before = cx
        .call(text, "state", &[Value::Str("overflow".into())])
        .unwrap()
        .to_json();
    assert!(app.frame(500, InputFrame::default()).is_err());
    let cx = app.context();
    let after = cx
        .call(text, "state", &[Value::Str("overflow".into())])
        .unwrap()
        .to_json();
    assert_eq!(before["reveal"], after["reveal"]);
    assert_eq!(
        cx.call(text, "drainEvents", &[Value::Str("overflow".into())])
            .unwrap()
            .to_json(),
        json!([])
    );
}
