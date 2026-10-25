// SPDX-License-Identifier: MPL-2.0
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

fn app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"phys","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"phys"}]});
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
fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}

#[test]
fn character_falls_lands_slides_on_walls_and_climbs_steps() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let mesh = serde_json::json!({"shape":{"kind":"mesh"}});
    for spec in [
        serde_json::json!({"name":"floor","geometry":"cube","scale":[20,1,20],"position":[0,-0.5,0],"collider":mesh}),
        serde_json::json!({"name":"wall","geometry":"cube","scale":[1,4,20],"position":[3,2,0],"collider":mesh}),
        serde_json::json!({"name":"step","geometry":"cube","scale":[2,0.2,20],"position":[-3,0.1,0],"collider":mesh}),
    ] {
        cx.call(world, "spawn", &[v(spec)]).unwrap();
    }
    let hero = cx
        .call(world, "spawn", &[v(serde_json::json!({"name":"hero","position":[0,3,0],
            "collider":{"shape":{"kind":"capsule","radius":0.3,"half_height":0.5},"body":"character"}}))])
        .unwrap()
        .as_object()
        .unwrap();
    // Fall under gravity until grounded; feet rest on the floor (center at 0.8).
    let mut vy = 0f32;
    let mut grounded = false;
    for _ in 0..200 {
        vy -= 9.81 * 0.016;
        let r = cx
            .call(
                hero,
                "moveCharacter",
                &[
                    Value::vec3([0., vy * 0.016, 0.]),
                    Value::Float(0.016),
                    Value::Null,
                ],
            )
            .unwrap();
        grounded = r.field("grounded").unwrap().as_bool().unwrap();
        if grounded {
            vy = 0.;
        }
    }
    assert!(grounded);
    let y = cx.get(hero, "position").unwrap().as_vec3().unwrap()[1];
    assert!((y - 0.8).abs() < 0.05, "standing height {y}");
    // Walk into the wall: stopped at the wall face minus radius.
    for _ in 0..200 {
        cx.call(
            hero,
            "moveCharacter",
            &[
                Value::vec3([0.05, -0.01, 0.]),
                Value::Float(0.016),
                Value::Null,
            ],
        )
        .unwrap();
    }
    let x = cx.get(hero, "position").unwrap().as_vec3().unwrap()[0];
    assert!(x < 2.5 - 0.29 && x > 2.0, "blocked by wall at {x}");
    // Walk the other way onto the 0.2 high step: autostep raises the feet.
    for _ in 0..400 {
        if cx.get(hero, "position").unwrap().as_vec3().unwrap()[0] < -3.0 {
            break;
        }
        cx.call(
            hero,
            "moveCharacter",
            &[
                Value::vec3([-0.05, -0.01, 0.]),
                Value::Float(0.016),
                Value::Null,
            ],
        )
        .unwrap();
    }
    for _ in 0..10 {
        cx.call(
            hero,
            "moveCharacter",
            &[
                Value::vec3([0., -0.01, 0.]),
                Value::Float(0.016),
                Value::Null,
            ],
        )
        .unwrap();
    }
    let p = cx.get(hero, "position").unwrap().as_vec3().unwrap();
    assert!(
        p[0] < -2.9 && p[0] > -3.2 && (p[1] - 1.0).abs() < 0.06,
        "on the step: {p:?}"
    );
    // Raycast down from above the floor hits it at y = 0.
    let hit = cx
        .call(
            world,
            "raycast",
            &[
                Value::vec3([1., 5., 1.]),
                Value::vec3([0., -1., 0.]),
                Value::Float(100.),
                Value::Object(crysirror_engine::value::ObjectRef {
                    id: hero.0,
                    class: None,
                }),
            ],
        )
        .unwrap();
    let point = hit.field("point").unwrap().as_vec3().unwrap();
    assert!(point[1].abs() < 1e-3, "{point:?}");
    assert_eq!(
        cx.get(hit.field("entity").unwrap().as_object().unwrap(), "name")
            .unwrap(),
        Value::Str("floor".into())
    );
    // Character integration is owned by physics: world velocity does not move it.
    app.init().unwrap();
    app.start().unwrap();
    app.frame(100, InputFrame::default()).unwrap();
}

#[test]
fn animation_crossfade_is_world_state() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let e = cx
        .call(world, "spawn", &[v(serde_json::json!({"name":"fox"}))])
        .unwrap()
        .as_object()
        .unwrap();
    cx.call(e, "play", &[Value::Str("Walk".into()), Value::Null])
        .unwrap();
    cx.call(
        e,
        "play",
        &[
            Value::Str("Run".into()),
            v(serde_json::json!({"speed":1.5,"fade":0.32})),
        ],
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(160, InputFrame::default()).unwrap(); // 10 fixed steps = 0.16 s
    let cx = app.context();
    let a = cx.get(e, "animation").unwrap().to_json();
    let layers = a["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert!(
        (layers[1]["weight"].as_f64().unwrap() - 0.5).abs() < 0.02,
        "{a}"
    );
    assert!(
        (layers[1]["time"].as_f64().unwrap() - 0.24).abs() < 0.01,
        "{a}"
    );
}

#[test]
fn sphere_casts_stop_a_radius_early_and_ground_pose_follows_slopes_and_steps() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let mesh = serde_json::json!({"shape":{"kind":"mesh"}});
    // Floor, a wall at x = 5 (face at 4.5) and a 20° ramp starting at z = 10.
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"floor","geometry":"cube","scale":[40,1,40],"position":[0,-0.5,0],"collider":mesh}))]).unwrap();
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"wall","geometry":"cube","scale":[1,4,4],"position":[5,2,0],"collider":mesh}))]).unwrap();
    let rise = 20f32.to_radians().tan() * 10.;
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"ramp","collider":mesh,"geometry":{"kind":"triangles",
        "vertices":[[-3.,0.,10.],[3.,0.,10.],[3.,rise,20.],[-3.,rise,20.]],"indices":[0,2,1,0,3,2]}}))]).unwrap();
    // A 0.5 step: a box from z = -20 to -10.
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"step","geometry":"cube","scale":[6,0.5,10],"position":[0,0.25,-15],"collider":mesh}))]).unwrap();
    let cx = app.context();
    let num = |m: &Value, k: &str| m.field(k).unwrap().as_f64().unwrap();
    let ray = cx.call(world, "raycast", &[Value::vec3([0., 1., 0.]), Value::vec3([1., 0., 0.]), Value::Float(20.)]).unwrap();
    let ball = cx.call(world, "sphereCast", &[Value::vec3([0., 1., 0.]), Value::vec3([1., 0., 0.]), Value::Float(0.5), Value::Float(20.)]).unwrap();
    assert!((num(&ray, "distance") - 4.5).abs() < 1e-3);
    assert!((num(&ball, "distance") - 4.0).abs() < 0.02, "sphere centre stops one radius before the wall: {ball:?}");
    let n = ball.field("normal").unwrap().as_vec3().unwrap();
    assert!(n[0] < -0.99, "normal faces back toward the caster: {n:?}");
    assert_eq!(cx.call(world, "sphereCast", &[Value::vec3([0., 1., 0.]), Value::vec3([-1., 0., 0.]), Value::Float(0.5), Value::Float(3.)]).unwrap(), Value::Null);
    // Ground pose: flat, on the ramp (20° nose up when facing +Z), across a step.
    let mut pose = |p: [f32; 3], yaw: f32| cx.call(world, "groundPose", &[Value::vec3(p), Value::Float(yaw as f64), Value::Float(1.), Value::Float(3.)]).unwrap();
    let flat = pose([0., 0.5, 0.], 0.);
    assert!(num(&flat, "height").abs() < 1e-3 && num(&flat, "pitch").abs() < 1e-3);
    let up = pose([0., rise * 0.5 + 0.5, 15.], 0.);
    assert!((num(&up, "pitch") - 20.).abs() < 0.1, "{up:?}");
    let down = pose([0., rise * 0.5 + 0.5, 15.], 180.);
    assert!((num(&down, "pitch") + 20.).abs() < 0.1, "{down:?}");
    // Straddling the step edge at z = -10 facing -Z: front on the step, back on the floor.
    let step = pose([0., 0.8, -10.], 180.);
    assert!((num(&step, "height") - 0.25).abs() < 1e-3 && num(&step, "pitch") > 10., "{step:?}");
    assert_eq!(pose([100., 0.5, 100.], 0.), Value::Null, "no ground");
}

#[test]
fn model_info_reports_rest_bounds() {
    let (tmp, mut app) = app();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/engine/assets/lantern.glb"),
        tmp.path().join("assets/lantern.glb"),
    )
    .unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let info = cx.call(world, "modelInfo", &[Value::Str("lantern.glb".into())]).unwrap();
    let b = info.field("bounds").unwrap();
    let (lo, hi) = (b.field("min").unwrap().as_vec3().unwrap(), b.field("max").unwrap().as_vec3().unwrap());
    assert!((0..3).all(|i| hi[i] > lo[i]), "{lo:?} {hi:?}");
    let _ = &mut app;
}

#[test]
fn arcade_vehicle_accelerates_steers_drifts_climbs_and_stops_at_walls() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let mesh = serde_json::json!({"shape":{"kind":"mesh"}});
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"floor","geometry":"cube","scale":[600,1,600],"position":[0,-0.5,0],"collider":mesh}))]).unwrap();
    // A wall across +X far away, and a 10° ramp along -Z.
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"wall","geometry":"cube","scale":[2,6,60],"position":[150,3,0],"collider":mesh}))]).unwrap();
    let rise = 10f32.to_radians().tan() * 40.;
    cx.call(world, "spawn", &[v(serde_json::json!({"name":"ramp","collider":mesh,"geometry":{"kind":"triangles",
        "vertices":[[-200.,0.,-100.],[-180.,0.,-100.],[-180.,rise,-140.],[-200.,rise,-140.]],"indices":[0,1,2,0,2,3]}}))]).unwrap();
    let body = serde_json::json!({"shape":{"kind":"capsule","radius":0.5,"half_height":0.05},"body":"character"});
    let spawn = |cx: &mut crysirror_engine::EngineContext, p: [f32; 3], yaw: f32| {
        let car = cx.call(world, "spawn", &[v(serde_json::json!({"name":"car","position":p,"rotation":[0,yaw,0],"collider":body}))]).unwrap().as_object().unwrap();
        cx.call(car, "setVehicle", &[v(serde_json::json!({}))]).unwrap();
        car
    };
    let drive = |cx: &mut crysirror_engine::EngineContext, car, input: serde_json::Value, secs: f32| {
        let mut last = Value::Null;
        for _ in 0..(secs * 60.) as usize {
            last = cx.call(car, "drive", &[v(input.clone()), Value::Float(1. / 60.)]).unwrap();
        }
        last
    };
    let num = |m: &Value, k: &str| m.field(k).unwrap().as_f64().unwrap();
    // Straight line along +X (yaw 90): accelerates, stays grounded at ride height.
    let cx = app.context();
    let car = spawn(cx, [0., 0.6, 0.], 90.);
    let r = drive(cx, car, serde_json::json!({"throttle":1}), 3.);
    assert!(num(&r, "speed") > 20. && r.field("grounded").unwrap() == &Value::Bool(true), "{r:?}");
    let p = cx.get(car, "position").unwrap().as_vec3().unwrap();
    assert!((p[1] - 0.6).abs() < 0.05 && p[0] > 30. && p[2].abs() < 0.5, "drove straight along +X: {p:?}");
    // Into the wall at x = 149: blocked, speed collapses.
    let r = drive(cx, car, serde_json::json!({"throttle":1}), 6.);
    let p = cx.get(car, "position").unwrap().as_vec3().unwrap();
    assert!(p[0] < 149. && num(&r, "speed").abs() < 5., "the wall stops the car: {p:?} {r:?}");
    // Steering: positive steer turns the heading toward +X of the car (yaw up).
    let car2 = spawn(cx, [0., 0.6, 100.], 0.);
    drive(cx, car2, serde_json::json!({"throttle":1}), 2.);
    let r = drive(cx, car2, serde_json::json!({"throttle":0.5,"steer":1}), 1.);
    assert!(num(&r, "yaw") > 20., "{r:?}");
    // Handbrake turn slides much more than a gripping turn.
    let lateral = |cx: &mut crysirror_engine::EngineContext, z: f32, hb: bool| {
        let c = spawn(cx, [0., 0.6, z], 0.);
        drive(cx, c, serde_json::json!({"throttle":1}), 3.);
        let r = drive(cx, c, serde_json::json!({"throttle":1,"steer":0.6,"handbrake":hb}), 0.5);
        num(&r, "lateral").abs()
    };
    let (grip, slide) = (lateral(cx, 200., false), lateral(cx, -250., true));
    assert!(slide > grip * 3. && slide > 3., "drift: handbrake {slide} vs grip {grip}");
    // Up the 10° ramp facing -Z: the body pitches nose-up by the ramp angle.
    let car3 = spawn(cx, [-190., 0.6, -80.], 180.);
    // 0.6 throttle reaches the ramp foot (z = -100) after ~2.5 s.
    drive(cx, car3, serde_json::json!({"throttle":0.6}), 4.2);
    let rot = cx.get(car3, "rotation").unwrap().as_vec3().unwrap();
    let p = cx.get(car3, "position").unwrap().as_vec3().unwrap();
    assert!(p[2] < -105. && p[2] > -135., "on the ramp: {p:?}");
    let pitch = state_pitch(&rot);
    assert!((pitch - 10.).abs() < 1.5, "pitch {pitch} from rotation {rot:?}");
}

/// Nose-up pitch of a vehicle from its XYZ Euler rotation.
fn state_pitch(rot: &[f32; 3]) -> f32 {
    let q = glam::Quat::from_euler(glam::EulerRot::XYZ, rot[0].to_radians(), rot[1].to_radians(), rot[2].to_radians());
    let fwd = q * glam::Vec3::Z;
    fwd.y.asin().to_degrees()
}
