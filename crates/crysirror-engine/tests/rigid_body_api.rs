// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn app() -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"physics-api","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"one","game":"one","adapter":"crysirror-script-v1"},{"name":"two","game":"two","adapter":"crysirror-script-v1"}]});
    let path = root.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let runtime = Runtime::load(&path, &root.path().join("state")).unwrap();
    (root, App::new(runtime, "one").unwrap())
}
fn v(value: serde_json::Value) -> Value {
    Value::from_json(value)
}
#[test]
fn public_impulses_forces_contacts_and_joint_handles_use_shared_runtime() {
    let (_root, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let physics = cx.singleton("Physics").unwrap();
    cx.set(physics, "gravity", Value::vec3([0.; 3])).unwrap();
    let body=cx.call(world,"spawn",&[v(json!({"name":"body","position":[0,2,0],"collider":{"shape":{"kind":"box","half_extents":[0.5,0.5,0.5]},"body":"dynamic","mass":2}}))]).unwrap().as_object().unwrap();
    cx.call(body, "applyImpulse", &[Value::vec3([2., 0., 0.])])
        .unwrap();
    assert!((cx.get(body, "velocity").unwrap().as_vec3().unwrap()[0] - 1.).abs() < 1e-6);
    cx.call(body, "addForce", &[Value::vec3([2., 0., 0.])])
        .unwrap();
    cx.call(world,"spawn",&[v(json!({"name":"trigger","position":[0,2,0],"collider":{"shape":{"kind":"sphere","radius":2},"sensor":true}}))]).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let cx = app.context();
    assert!(cx.get(body, "velocity").unwrap().as_vec3().unwrap()[0] > 1.01);
    cx.call(body, "clearForces", &[]).unwrap();
    let velocity = cx.get(body, "velocity").unwrap();
    let events = cx.call(physics, "events", &[]).unwrap().to_json();
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "SensorEntered"),
        "{events}"
    );
    app.frame(16, InputFrame::default()).unwrap();
    let cx = app.context();
    assert_eq!(cx.get(body, "velocity").unwrap(), velocity);
    let anchor=cx.call(world,"spawn",&[v(json!({"name":"anchor","position":[10,2,0],"collider":{"shape":{"kind":"sphere","radius":0.2}}}))]).unwrap();
    let joint = cx
        .call(
            physics,
            "createJoint",
            &[
                anchor,
                Value::object(body, "Entity"),
                v(json!({"kind":"spherical","local_anchor1":[-10,0,0]})),
            ],
        )
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(cx.get(joint, "alive").unwrap(), Value::Bool(true));
    assert_eq!(
        cx.get(physics, "stats")
            .unwrap()
            .field("joints")
            .unwrap()
            .as_i64()
            .unwrap(),
        1
    );
    cx.call(
        joint,
        "setSpec",
        &[v(
            json!({"kind":"fixed","local_frame1":{"translation":[-10,0,0]}}),
        )],
    )
    .unwrap();
    assert_eq!(
        cx.get(physics, "stats")
            .unwrap()
            .field("joints")
            .unwrap()
            .as_i64()
            .unwrap(),
        1
    );
    cx.call(joint, "destroy", &[]).unwrap();
    assert_eq!(cx.get(joint, "alive").unwrap(), Value::Bool(false));
    assert!(cx.call(joint, "destroy", &[]).is_err());
    assert!(cx
        .call(
            physics,
            "raycast",
            &[
                Value::vec3([0., 5., 0.]),
                Value::vec3([0.; 3]),
                Value::Float(10.)
            ]
        )
        .is_err());
    cx.set(body, "angularVelocity", Value::vec3([0., 90., 0.]))
        .unwrap();
    assert_eq!(
        cx.get(body, "angularVelocity").unwrap(),
        Value::vec3([0., 90., 0.])
    );
    cx.call(world, "pause", &[Value::Bool(true)]).unwrap();
    let prior = cx.get(body, "velocity").unwrap().as_vec3().unwrap();
    cx.call(body, "applyImpulse", &[Value::vec3([2., 0., 0.])])
        .unwrap();
    let paused_velocity = cx.get(body, "velocity").unwrap();
    assert!(
        (paused_velocity.as_vec3().unwrap()[0] - prior[0] - 1.).abs() < 1e-5,
        "paused impulse must update saved velocity immediately"
    );
    cx.call(body, "applyTorqueImpulse", &[Value::vec3([0., 1., 0.])])
        .unwrap();
    let paused_angular = cx.get(body, "angularVelocity").unwrap();
    assert!(
        paused_angular.as_vec3().unwrap()[1] > 90.1,
        "paused torque impulse must update saved angular velocity"
    );
    let paused_position = cx.get(body, "position").unwrap();
    let save = app.runtime().unwrap().snapshot().unwrap();
    app.frame(32, InputFrame::default()).unwrap();
    assert_eq!(
        app.context().get(body, "position").unwrap(),
        paused_position,
        "paused bodies must not integrate"
    );
    app.runtime_mut().unwrap().restore(&save).unwrap();
    let cx = app.context();
    assert_eq!(cx.get(body, "velocity").unwrap(), paused_velocity);
    assert_eq!(cx.get(body, "angularVelocity").unwrap(), paused_angular);
    let rebuilt = cx.get(body, "physicsState").unwrap();
    for (a, b) in rebuilt
        .field("velocity")
        .unwrap()
        .as_vec3()
        .unwrap()
        .into_iter()
        .zip(paused_velocity.as_vec3().unwrap())
    {
        assert!((a - b).abs() < 1e-5);
    }
    for (a, b) in rebuilt
        .field("angularVelocity")
        .unwrap()
        .as_vec3()
        .unwrap()
        .into_iter()
        .zip(paused_angular.as_vec3().unwrap())
    {
        assert!((a - b).abs() < 1e-4);
    }
    cx.call(body, "destroy", &[]).unwrap();
    assert!(cx
        .call(body, "applyImpulse", &[Value::vec3([1., 0., 0.])])
        .is_err());
}

#[test]
fn entity_payload_and_query_ignore_cannot_alias_another_clients_same_id() {
    let (_root, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let physics = cx.singleton("Physics").unwrap();
    let spec = |name| v(json!({"name":name,"collider":{"shape":{"kind":"sphere","radius":1}}}));
    let one = cx
        .call(world, "spawn", &[spec("one")])
        .unwrap()
        .as_object()
        .unwrap();
    cx.state().unwrap().client = ClientId(2);
    let two = cx
        .call(world, "spawn", &[spec("two")])
        .unwrap()
        .as_object()
        .unwrap();
    assert_ne!(one, two);
    assert_eq!(cx.get(two, "name").unwrap(), Value::Str("two".into()));
    cx.state().unwrap().client = ClientId(1);
    assert!(cx.get(two, "name").is_err());
    assert_eq!(cx.get(one, "name").unwrap(), Value::Str("one".into()));
    let options = Value::Map(std::collections::BTreeMap::from([(
        "ignore".into(),
        Value::object(two, "Entity"),
    )]));
    assert!(cx
        .call(
            physics,
            "raycast",
            &[
                Value::vec3([0., 3., 0.]),
                Value::vec3([0., -1., 0.]),
                Value::Float(10.),
                options
            ]
        )
        .is_err());
    let overlaps = cx
        .call(
            physics,
            "overlapShape",
            &[v(json!({"kind":"sphere","radius":2})), v(json!({}))],
        )
        .unwrap();
    assert_eq!(overlaps.to_json().as_array().unwrap().len(), 1);
}
