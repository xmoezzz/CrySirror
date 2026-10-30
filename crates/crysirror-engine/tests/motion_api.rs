//! Shared motion is available to engine users without a native VM.
use crysirror_core::{
    service::{EngineQuery, HostServices, InputFrame},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;

#[test]
fn public_motion_uses_engine_tracks_and_survives_restore() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("assets")).unwrap();
    let project = temp.path().join("project.json");
    std::fs::write(
        &project,
        serde_json::to_vec(&json!({
            "schema":2,"name":"motion","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"motion"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&project, &temp.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let v = Value::from_json;
    let layers = app.context().singleton("Layers").unwrap();
    let ui = app.context().singleton("Ui").unwrap();
    app.context()
        .call(
            layers,
            "put",
            &[v(
                json!({"id":7,"rect":[0,0,20,20],"paint":{"kind":"solid","color":[255,0,0,255]}}),
            )],
        )
        .unwrap();
    app.context()
        .call(
            ui,
            "put",
            &[v(
                json!({"id":8,"rect":[0,40,20,20],"kind":"button","text":""}),
            )],
        )
        .unwrap();
    app.context()
        .call(
            layers,
            "beginMotion",
            &[
                7i64.into(),
                v(json!({"property":"x","to":100,"duration":20,"external":true})),
            ],
        )
        .unwrap();
    app.context()
        .call(
            ui,
            "beginMotion",
            &[
                8i64.into(),
                v(json!({"property":"x","to":40,"duration":20})),
            ],
        )
        .unwrap();
    app.frame(10, InputFrame::default()).unwrap();
    let status = app
        .context()
        .call(layers, "motionStatus", &[7i64.into()])
        .unwrap()
        .to_json();
    assert_eq!(status["tracks"][0]["current"], 0.0);
    app.context()
        .call(
            layers,
            "setMotionElapsed",
            &[7i64.into(), "x".into(), 10i64.into()],
        )
        .unwrap();
    let status = app
        .context()
        .call(layers, "motionStatus", &[7i64.into()])
        .unwrap()
        .to_json();
    assert_eq!(status["tracks"][0]["current"], 50.0);
    let mut coverage = vec![255; 20 * 20];
    coverage[5 * 20 + 5] = 0;
    app.context()
        .call(
            ui,
            "setHitMask",
            &[
                8i64.into(),
                v(json!({"size":[20,20],"coverage":coverage,"threshold":1})),
            ],
        )
        .unwrap();
    let snapshot = app.runtime().unwrap().snapshot().unwrap();
    app.context()
        .call(
            layers,
            "endMotion",
            &[7i64.into(), Value::Null, true.into()],
        )
        .unwrap();
    app.runtime_mut().unwrap().restore(&snapshot).unwrap();
    let status = app
        .context()
        .call(layers, "motionStatus", &[7i64.into()])
        .unwrap()
        .to_json();
    assert_eq!(status["tracks"][0]["elapsed"], 10);
    let widget = app.runtime().unwrap().host.engine.ui.get(1, 8).unwrap();
    assert_eq!(widget.rect[0], 20.0);
    let runtime = app.runtime_mut().unwrap();
    assert!(runtime
        .host
        .query_engine(ClientId(1), EngineQuery::LayerMotion(7))
        .is_ok());
    assert!(runtime
        .host
        .query_engine(ClientId(999), EngineQuery::LayerMotion(7))
        .is_err());
    assert!(runtime
        .host
        .query_engine(ClientId(1), EngineQuery::UiMotion(999))
        .is_err());
    for (x, expected) in [(25, false), (26, true)] {
        app.frame(
            0,
            InputFrame {
                pointer_x: x,
                pointer_y: 45,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            app.runtime()
                .unwrap()
                .host
                .engine
                .ui
                .interaction(1, 8)
                .unwrap()
                .hovered,
            expected
        );
    }
    app.context()
        .call(ui, "setHitMask", &[8i64.into(), Value::Null])
        .unwrap();
    app.frame(
        0,
        InputFrame {
            pointer_x: 25,
            pointer_y: 45,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        app.runtime()
            .unwrap()
            .host
            .engine
            .ui
            .interaction(1, 8)
            .unwrap()
            .hovered
    );
}
