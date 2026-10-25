// SPDX-License-Identifier: MPL-2.0
//! Systems working together: perception → navigation around a wall →
//! facing → ability activation with cost/cooldown, plus animation graphs.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"ai","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"ai"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(),
        "game",
    )
    .unwrap();
    (tmp, app)
}

#[test]
fn knight_perceives_navigates_around_wall_faces_and_attacks() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world, "defineGameplay", &[v(serde_json::json!({
        "effects": {
            "StaminaCost": {"duration": {"kind": "instant"}, "modifiers": [{"attribute": "Stamina", "op": "add", "magnitude": -10}]},
            "AttackCooldown": {"duration": {"kind": "duration", "seconds": 1.0}, "grant_tags": ["Cooldown.Attack"]}
        },
        "abilities": {"Attack": {"cost": "StaminaCost", "cooldown": "AttackCooldown", "active_tags": ["State.Attacking"], "duration": 0.5, "cue": "Cue.Swing"}}
    }))]).unwrap();
    cx.call(
        world,
        "defineBehaviorTree",
        &[
            Value::Str("Knight".into()),
            v(serde_json::json!({
            "kind": "selector", "children": [
                {"kind": "sequence", "children": [
                    {"kind": "perceive", "tag": "Team.Player", "radius": 30, "into": "target"},
                    {"kind": "selector", "children": [
                        {"kind": "sequence", "children": [
                            {"kind": "near", "target": "target", "less_than": 1.4},
                            {"kind": "face", "target": "target", "degrees_per_second": 720},
                            {"kind": "ability", "name": "Attack"}
                        ]},
                        {"kind": "move_to", "target": "target", "speed": 4, "acceptance": 1.0}
                    ]}
                ]},
                {"kind": "wait", "seconds": 0.5}
            ]})),
        ],
    )
    .unwrap();
    let mesh = serde_json::json!({"shape": {"kind": "mesh"}});
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "floor", "geometry": "cube", "scale": [30, 1, 30], "position": [0, -0.5, 0], "collider": mesh}))]).unwrap();
    // A wall between knight and player, open only at z > 4.
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "wall", "geometry": "cube", "scale": [0.5, 3, 16], "position": [0, 1.5, -4], "collider": mesh}))]).unwrap();
    let capsule = serde_json::json!({"shape": {"kind": "capsule", "radius": 0.3, "half_height": 0.5}, "body": "character"});
    let player = cx.call(world, "spawn", &[v(serde_json::json!({"name": "player", "position": [4, 0.8, -4], "collider": capsule}))]).unwrap().as_object().unwrap();
    cx.call(player, "addTag", &[Value::Str("Team.Player".into())])
        .unwrap();
    let knight = cx.call(world, "spawn", &[v(serde_json::json!({"name": "knight", "position": [-4, 0.8, -4], "collider": capsule}))]).unwrap().as_object().unwrap();
    cx.call(
        knight,
        "initAttributes",
        &[v(serde_json::json!({"Stamina": {"base": 100, "min": 0}}))],
    )
    .unwrap();
    cx.call(knight, "grantAbility", &[Value::Str("Attack".into())])
        .unwrap();
    let nav = cx
        .call(
            world,
            "buildNavigation",
            &[v(
                serde_json::json!({"min": [-12, -1, -12], "max": [12, 4, 12], "cell": 0.25}),
            )],
        )
        .unwrap();
    assert!(
        nav.field("walkable").unwrap().as_i64().unwrap() > 5000,
        "{nav:?}"
    );
    // The straight line is blocked: the path detours around the wall's end.
    let path = cx
        .call(
            world,
            "findPath",
            &[Value::vec3([-4., 0.8, -4.]), Value::vec3([4., 0.8, -4.])],
        )
        .unwrap();
    let Value::List(points) = &path else {
        panic!("no path")
    };
    assert!(
        points.iter().any(|p| p.as_vec3().unwrap()[2] > 3.5),
        "detour: {path:?}"
    );
    cx.call(knight, "runBehaviorTree", &[Value::Str("Knight".into())])
        .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let mut attacks = 0;
    for _ in 0..400 {
        app.frame(16, InputFrame::default()).unwrap();
        let cx = app.context();
        let ev = cx.call(knight, "drainEvents", &[]).unwrap().to_json();
        attacks += ev
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "ability_activated")
            .count();
    }
    let cx = app.context();
    let kp = cx.get(knight, "position").unwrap().as_vec3().unwrap();
    let pp = cx.get(player, "position").unwrap().as_vec3().unwrap();
    let dist = ((kp[0] - pp[0]).powi(2) + (kp[2] - pp[2]).powi(2)).sqrt();
    assert!(dist < 1.5, "knight reached the player: {kp:?} vs {pp:?}");
    assert!(kp[0] > 0.3, "went around to the player's side of the wall");
    // ~6.4 s total; travel then one attack per 1 s cooldown.
    assert!((2..=6).contains(&attacks), "attacks {attacks}");
    let stamina = cx
        .call(knight, "attribute", &[Value::Str("Stamina".into())])
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((stamina - (100. - 10. * attacks as f64)).abs() < 1e-3);
    let yaw = cx.get(knight, "rotation").unwrap().as_vec3().unwrap()[1];
    let want = ((pp[0] - kp[0]).atan2(pp[2] - kp[2])).to_degrees();
    assert!(
        ((yaw - want + 540.).rem_euclid(360.) - 180.).abs() < 5.,
        "faces target: {yaw} vs {want}"
    );
    let ai = cx.get(knight, "ai").unwrap().to_json();
    assert!(ai["blackboard"]["target"].is_number(), "{ai}");
}

#[test]
fn state_machine_and_montage_drive_layers_through_the_api() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world, "defineAnimation", &[v(serde_json::json!({
        "machines": {"Fox": {"entry": "Move", "parameters": {"speed": 0},
            "states": {"Move": {"kind": "blend1d", "param": "speed", "samples": [{"at": 0, "clip": "Survey"}, {"at": 1.6, "clip": "Walk"}, {"at": 5, "clip": "Run"}]}}}},
        "montages": {"Bite": {"clip": "Survey", "length": 0.6, "notifies": [{"time": 0.2, "name": "Hit"}]}}
    }))]).unwrap();
    let fox = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "fox"}))])
        .unwrap()
        .as_object()
        .unwrap();
    cx.call(fox, "setStateMachine", &[Value::Str("Fox".into())])
        .unwrap();
    cx.call(
        fox,
        "setAnimParam",
        &[Value::Str("speed".into()), Value::Float(5.)],
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(32, InputFrame::default()).unwrap();
    let cx = app.context();
    let a = cx.get(fox, "animation").unwrap().to_json();
    assert_eq!(a["layers"][0]["clip"], "Run", "{a}");
    cx.call(
        fox,
        "playMontage",
        &[Value::Str("Bite".into()), Value::Null],
    )
    .unwrap();
    for _ in 0..50 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    let ev = cx.call(fox, "drainAnimEvents", &[]).unwrap().to_json();
    let names: Vec<_> = ev
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        names.contains(&"Hit".into()) && names.contains(&"Bite".into()),
        "{names:?}"
    );
}

#[test]
fn navigation_steps_over_small_steep_bumps_but_not_tall_ones() {
    // Imported levels scatter decals and trim with steep faces over floors;
    // a bump lower than the step height must not punch holes into the grid.
    let (_tmp, mut app) = app();
    app.init().unwrap();
    app.start().unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let mesh = serde_json::json!({"shape": {"kind": "mesh"}});
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "floor", "geometry": "cube", "scale": [20, 1, 20], "position": [0, -0.5, 0], "collider": mesh}))]).unwrap();
    // A low pyramid (0.1 high, 60° faces) and a tall one (2 high).
    let pyramid = |h: f32| serde_json::json!({"kind": "triangles", "vertices": [[-0.1,0.,-0.1],[0.1,0.,-0.1],[0.1,0.,0.1],[-0.1,0.,0.1],[0.,h,0.]],
        "indices": [0,4,1, 1,4,2, 2,4,3, 3,4,0]});
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "low", "position": [-3, 0, 0], "collider": mesh, "geometry": pyramid(0.17)}))]).unwrap();
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "tall", "position": [3, 0, 0], "collider": mesh, "geometry": pyramid(2.0)}))]).unwrap();
    let mut walkable = |min_x: f64, max_x: f64| {
        let nav = cx.call(world, "buildNavigation", &[v(serde_json::json!({"min": [min_x, -1, -2], "max": [max_x, 3, 2], "cell": 0.1, "agent_radius": 0.1, "max_step": 0.35, "max_slope": 45}))]).unwrap();
        nav.field("walkable").unwrap().as_i64().unwrap()
    };
    let (low, tall, plain) = (walkable(-5., -1.), walkable(1., 5.), walkable(-9., -5.));
    assert_eq!(low, plain, "a 0.17 bump is stepped over");
    assert!(tall < plain, "a 2.0 spike blocks cells: {tall} vs {plain}");
}
