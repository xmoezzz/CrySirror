//! Public receiver APIs operate on the shared router without a native adapter.
use crysirror_core::{service::buttons, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use crysirror_scene::input_routes::{InputRouteEvent, InputRoutePhase, InputRouteSource};
use serde_json::json;

#[test]
fn public_receivers_share_priority_delivery_acknowledgement_and_restore_epochs() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&json!({
            "schema":2,"name":"routes","content_revision":"1","assets":"assets","canvas":[16,16],
            "clients":[{"name":"app","adapter":"crysirror-script-v1","game":"routes"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let runtime = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let mut app = App::new(runtime, "app").unwrap();
    let routes = app.context().singleton("InputRoutes").unwrap();
    for (id, priority) in [(1, -5), (2, 10)] {
        app.context()
            .call(
                routes,
                "put",
                &[Value::from_json(json!({
                    "id":id,"priority":priority,"sources":1,"rect":null,"hover_enabled":false
                }))],
            )
            .unwrap();
    }
    let event = || InputRouteEvent {
        source: InputRouteSource::Keyboard,
        code: buttons::CTRL as u32,
        phase: InputRoutePhase::Press,
        point: [0, 0],
        origin: None,
    };
    app.runtime_mut()
        .unwrap()
        .host
        .engine
        .input_routes
        .dispatch(ClientId(1), event())
        .unwrap();
    let delivery = app
        .context()
        .call(routes, "delivery", &[])
        .unwrap()
        .to_json();
    assert_eq!(delivery["receiver"], 2);
    for name in ["epoch", "event", "delivery"] {
        assert!(delivery["receipt"][name].is_string());
    }
    assert_eq!(
        app.context()
            .call(routes, "delivery", &[])
            .unwrap()
            .to_json(),
        delivery
    );
    let mut foreign = delivery["receipt"].clone();
    foreign["client"] = json!(99);
    assert!(app
        .context()
        .call(
            routes,
            "acknowledge",
            &[Value::from_json(foreign), Value::Bool(true)]
        )
        .is_err());
    app.context()
        .call(
            routes,
            "acknowledge",
            &[
                Value::from_json(delivery["receipt"].clone()),
                Value::Bool(false),
            ],
        )
        .unwrap();
    let next = app
        .context()
        .call(routes, "delivery", &[])
        .unwrap()
        .to_json();
    assert_eq!(next["receiver"], 1);
    assert_ne!(next["receipt"], delivery["receipt"]);
    app.context()
        .call(
            routes,
            "acknowledge",
            &[Value::from_json(next["receipt"].clone()), Value::Bool(true)],
        )
        .unwrap();
    assert_eq!(
        app.context()
            .call(routes, "delivery", &[])
            .unwrap()
            .to_json(),
        json!(null)
    );
    assert!(app
        .context()
        .call(
            routes,
            "acknowledge",
            &[Value::from_json(next["receipt"].clone()), Value::Bool(true)]
        )
        .is_err());
    app.runtime_mut()
        .unwrap()
        .host
        .engine
        .input_routes
        .dispatch(ClientId(1), event())
        .unwrap();
    let stale = app
        .context()
        .call(routes, "delivery", &[])
        .unwrap()
        .to_json();
    let snapshot = app.runtime().unwrap().snapshot().unwrap();
    app.runtime_mut().unwrap().restore(&snapshot).unwrap();
    assert_eq!(
        app.context()
            .call(routes, "delivery", &[])
            .unwrap()
            .to_json(),
        json!(null)
    );
    assert!(app
        .context()
        .call(
            routes,
            "acknowledge",
            &[
                Value::from_json(stale["receipt"].clone()),
                Value::Bool(true)
            ]
        )
        .is_err());
    assert_eq!(
        app.context()
            .call(routes, "receiver", &[Value::Int(2)])
            .unwrap()
            .to_json()["priority"],
        10
    );
    app.context().call(routes, "clear", &[]).unwrap();
    assert_eq!(
        app.context().call(routes, "stats", &[]).unwrap().to_json()["receivers"],
        0
    );
}
