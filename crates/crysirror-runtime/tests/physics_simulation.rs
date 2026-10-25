// SPDX-License-Identifier: MPL-2.0
//! Real shared simulation from data-only projects. No App/script/device setup.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::world::{EntityHandle, WorldCommand};
use glam::Vec3;
use serde_json::{json, Value};

fn runtime(clients: Vec<Value>) -> (tempfile::TempDir, Runtime) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"data-physics","content_revision":"1","assets":"assets","canvas":[64,64],"clients":clients});
    let path = root.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let rt = Runtime::load(&path, &root.path().join("state")).unwrap();
    (root, rt)
}
fn client(name: &str, entities: Vec<Value>, physics: Value) -> Value {
    json!({"name":name,"game":name,"adapter":"crysirror-script-v1","systems":{"entities":entities,"physics":physics}})
}
fn cube(id: u32, y: f32, body: &str) -> Value {
    json!({"id":id,"name":format!("body{id}"),"geometry":{"kind":"cube"},"transform":{"translation":[0,y,0]},"collider":{"shape":{"kind":"box","half_extents":[0.5,0.5,0.5]},"body":body,"mass":2}})
}
fn handle(rt: &Runtime, c: u32, id: u32) -> EntityHandle {
    rt.host.engine.world.handle(ClientId(c), id).unwrap()
}
fn entity(rt: &Runtime, c: u32, id: u32) -> &crysirror_scene::world::EntitySpec {
    rt.host.engine.world.entity(handle(rt, c, id)).unwrap()
}
fn step(rt: &mut Runtime, count: u32) {
    for _ in 0..count {
        rt.tick(16, InputFrame::default()).unwrap();
    }
}
fn floor() -> Value {
    json!({"id":10,"name":"floor","transform":{"translation":[0,-0.5,0]},"collider":{"shape":{"kind":"box","half_extents":[10,0.5,10]}}})
}

#[test]
fn json_rigid_bodies_fall_stack_and_do_not_rebuild_on_unrelated_revisions() {
    let (_root, mut rt) = runtime(vec![client(
        "game",
        vec![
            floor(),
            cube(1, 1., "dynamic"),
            cube(2, 2.1, "dynamic"),
            cube(3, 3.2, "dynamic"),
        ],
        json!({}),
    )]);
    rt.tick(16, InputFrame::default()).unwrap();
    assert!(
        entity(&rt, 1, 3).transform.translation[1] > 3.1968,
        "World must not integrate dynamic velocity a second time"
    );
    assert!(entity(&rt, 1, 3).velocity[1] < -0.15);
    let before = rt.physics.stats(1);
    for i in 0..240 {
        rt.host
            .engine
            .world
            .apply_batch(
                ClientId(1),
                &[WorldCommand::SetVisible {
                    id: 1,
                    visible: i % 2 == 0,
                }],
            )
            .unwrap();
        rt.tick(16, InputFrame::default()).unwrap();
    }
    for (id, height) in [(1, 0.5), (2, 1.5), (3, 2.5)] {
        let e = entity(&rt, 1, id);
        assert!(
            (e.transform.translation[1] - height).abs() < 0.12,
            "body {id}: {:?}",
            e.transform.translation
        );
        assert!(e.velocity[1].abs() < 0.3);
    }
    let after = rt.physics.stats(1);
    assert_eq!(after.bodies_created, before.bodies_created);
    assert_eq!(after.colliders_created, before.colliders_created);
    assert_eq!(after.shape_builds, before.shape_builds);
    assert!(after.simulation_steps >= 241);
}

#[test]
fn one_32ms_tick_matches_two_16ms_ticks_with_moving_kinematic_contact() {
    let mut platform = floor();
    platform["collider"]["body"] = "kinematic".into();
    platform["velocity"] = json!([0, 0.1, 0]);
    let clients = vec![client(
        "game",
        vec![platform, cube(1, 0.7, "dynamic")],
        json!({}),
    )];
    let (_a, mut first) = runtime(clients.clone());
    let (_b, mut second) = runtime(clients);
    for _ in 0..60 {
        first.tick(32, InputFrame::default()).unwrap();
        second.tick(16, InputFrame::default()).unwrap();
        second.tick(16, InputFrame::default()).unwrap();
    }
    for id in [1, 10] {
        let a = entity(&first, 1, id);
        let b = entity(&second, 1, id);
        for (x, y) in a.transform.translation.iter().zip(b.transform.translation) {
            assert!((x - y).abs() < 1e-5, "{id}: {x} {y}");
        }
        for (x, y) in a.velocity.iter().zip(b.velocity) {
            assert!((x - y).abs() < 1e-5);
        }
    }
    assert_eq!(first.physics.stats(1).simulation_steps, 120);
    assert_eq!(second.physics.stats(1).simulation_steps, 120);
}

#[test]
fn restore_rebuilds_saved_velocity_forces_joint_and_pending_target() {
    let mut moving = cube(1, 0., "dynamic");
    moving["velocity"] = json!([1, 0, 0]);
    let target = cube(2, 3., "kinematic_position");
    let physics = json!({"gravity":[0,0,0],"forces":{"1":{"generation":1,"force":[2,0,0]}},"kinematic_targets":{"2":{"generation":1,"pose":{"translation":[4,3,0]}}},"next_joint":1,"joints":{"1":{"generation":1,"a":{"client":1,"id":3,"generation":1},"b":{"client":1,"id":4,"generation":1},"joint":{"kind":"fixed"}}}});
    let (_root, mut rt) = runtime(vec![client(
        "game",
        vec![
            moving,
            target,
            cube(3, 10., "static"),
            cube(4, 10., "dynamic"),
        ],
        physics,
    )]);
    let pending = rt.snapshot().unwrap();
    rt.tick(16, InputFrame::default()).unwrap();
    assert!((entity(&rt, 1, 2).transform.translation[0] - 4.).abs() < 1e-5);
    assert!(rt.host.engine.world.physics.clients[&1]
        .kinematic_targets
        .is_empty());
    assert_eq!(rt.physics.stats(1).joints, 1);
    let advanced = entity(&rt, 1, 1).transform.translation;
    rt.restore(&pending).unwrap();
    assert_eq!(rt.physics.stats(1).bodies, 0);
    rt.tick(16, InputFrame::default()).unwrap();
    assert!((entity(&rt, 1, 2).transform.translation[0] - 4.).abs() < 1e-5);
    assert_eq!(entity(&rt, 1, 1).transform.translation, advanced);
    assert_eq!(rt.physics.stats(1).joints, 1);
    let saved = rt.snapshot().unwrap();
    let velocity = entity(&rt, 1, 1).velocity;
    step(&mut rt, 4);
    rt.restore(&saved).unwrap();
    assert_eq!(entity(&rt, 1, 1).velocity, velocity);
    rt.tick(16, InputFrame::default()).unwrap();
    assert!(
        entity(&rt, 1, 1).velocity[0] > velocity[0],
        "saved persistent force must resume"
    );
}

#[test]
fn clients_collisions_queries_ignores_and_generations_are_isolated() {
    let (_root, mut rt) = runtime(vec![
        client("one", vec![floor(), cube(1, 3., "dynamic")], json!({})),
        client("two", vec![cube(1, 3., "dynamic")], json!({})),
    ]);
    step(&mut rt, 100);
    assert!(entity(&rt, 1, 1).transform.translation[1] > 0.4);
    assert!(
        entity(&rt, 2, 1).transform.translation[1] < -5.,
        "client two cannot collide with client one's floor"
    );
    let other = handle(&rt, 2, 1);
    assert!(rt
        .physics
        .query_options(1, Some(other), None, false, false)
        .is_err());
    let hit = rt
        .physics
        .query_raycast(
            1,
            Vec3::new(5., 10., 0.),
            Vec3::NEG_Y,
            100.,
            Default::default(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(hit.key.id, 10);
    assert!((hit.distance - 10.).abs() < 1e-4);
    assert_eq!(hit.point.x, 5.);
    assert!(rt
        .physics
        .query_raycast(
            2,
            Vec3::new(5., 10., 0.),
            Vec3::NEG_Y,
            100.,
            Default::default()
        )
        .unwrap()
        .is_none());
    assert!(rt
        .physics
        .query_raycast(1, Vec3::ZERO, Vec3::ZERO, 10., Default::default())
        .is_err());
    assert!(rt
        .physics
        .query_raycast(
            1,
            Vec3::ZERO,
            Vec3::splat(f32::NAN),
            10.,
            Default::default()
        )
        .is_err());
    let old = handle(&rt, 1, 1);
    let spec: crysirror_scene::world::EntitySpec =
        serde_json::from_value(cube(1, 5., "dynamic")).unwrap();
    rt.host
        .engine
        .world
        .apply_batch(
            ClientId(1),
            &[
                WorldCommand::Despawn { id: 1 },
                WorldCommand::Spawn { entity: spec },
            ],
        )
        .unwrap();
    rt.physics
        .sync(&rt.host.engine.world, &mut rt.host.fs)
        .unwrap();
    assert!(rt.physics.body_state(old).is_err());
    assert_ne!(handle(&rt, 1, 1).generation, old.generation);
}

#[test]
fn failed_staged_sync_discards_derived_world_before_retry_or_query() {
    let mut bad = cube(1, 3., "dynamic");
    bad["collider"]["shape"] = json!({"kind":"sphere","radius":0.5});
    bad["transform"]["scale"] = json!([1, 2, 1]);
    let (_root, mut rt) = runtime(vec![
        client("one", vec![cube(1, 3., "dynamic")], json!({})),
        client("two", vec![bad], json!({})),
    ]);
    assert!(rt.tick(16, InputFrame::default()).is_err());
    assert_eq!(rt.now_ms(), 0);
    assert_eq!(rt.host.engine.world.elapsed_ms, 0);
    assert_eq!(entity(&rt, 1, 1).transform.translation, [0., 3., 0.]);
    assert_eq!(rt.physics.stats(1).bodies, 0);
    let mut transform = entity(&rt, 2, 1).transform.clone();
    transform.scale = [1.; 3];
    rt.host
        .engine
        .world
        .apply_batch(
            ClientId(2),
            &[WorldCommand::SetTransform { id: 1, transform }],
        )
        .unwrap();
    rt.physics
        .sync(&rt.host.engine.world, &mut rt.host.fs)
        .unwrap();
    assert_eq!(
        rt.physics
            .body_state(handle(&rt, 1, 1))
            .unwrap()
            .pose
            .translation
            .y,
        3.
    );
    rt.tick(16, InputFrame::default()).unwrap();
    assert!(entity(&rt, 1, 1).transform.translation[1] < 3.);
}

#[test]
fn axis_heightfield_queries_keep_origin_and_distance_within_seam_tolerance() {
    let terrain = json!({"id":1,"name":"flat terrain","geometry":{"kind":"terrain","resolution":5,"size":[10,10],"height":0},"collider":{"shape":{"kind":"mesh"}}});
    let (_root, mut rt) = runtime(vec![client("game", vec![terrain], json!({}))]);
    rt.physics
        .sync(&rt.host.engine.world, &mut rt.host.fs)
        .unwrap();
    for x in [0., -2.5, 2.5] {
        let hit = rt
            .physics
            .query_raycast(
                1,
                Vec3::new(x, 5., 0.),
                Vec3::NEG_Y,
                20.,
                Default::default(),
            )
            .unwrap()
            .unwrap();
        assert!((hit.distance - 5.).abs() < 1e-3);
        assert!(
            (hit.point.x - x).abs() < 1e-3 && hit.point.z.abs() < 1e-3 && hit.point.y.abs() < 1e-3
        );
        assert!(hit.normal.y > 0.999);
    }
}
