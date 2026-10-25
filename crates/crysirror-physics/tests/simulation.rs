use crysirror_physics::*;
use glam::{Mat4, Quat, Vec3};
use std::sync::Arc;
fn key(id: u32) -> BodyKey {
    BodyKey { id, generation: 0 }
}
fn world() -> PhysicsWorld {
    PhysicsWorld::new(7, WorldConfig::default()).unwrap()
}
fn cube(kind: BodyKind, p: Vec3) -> BodyDesc {
    BodyDesc {
        kind,
        pose: Pose::at(p),
        ..Default::default()
    }
}
fn put(w: &mut PhysicsWorld, id: u32, desc: BodyDesc) -> BodyHandle {
    w.upsert(key(id), &desc, &[ColliderDesc::default()])
        .unwrap()
}
fn floor(w: &mut PhysicsWorld) {
    w.upsert(
        key(0),
        &cube(BodyKind::Fixed, Vec3::new(0., -0.5, 0.)),
        &[ColliderDesc {
            shape: Shape::Cuboid {
                half_extents: Vec3::new(50., 0.5, 50.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
}
fn steps(w: &mut PhysicsWorld, n: usize) {
    for _ in 0..n {
        w.step_fixed(1. / 60.).unwrap();
    }
}
#[test]
fn gravity_collision_stack_and_natural_sleep() {
    let mut w = world();
    floor(&mut w);
    let mut handles = Vec::new();
    for id in 1..=5 {
        handles.push(put(
            &mut w,
            id,
            cube(BodyKind::Dynamic, Vec3::new(0., id as f32 * 1.02 + 1., 0.)),
        ));
    }
    steps(&mut w, 720);
    for (i, h) in handles.iter().enumerate() {
        let s = w.state(*h).unwrap();
        assert!(
            (s.pose.translation.y - (i as f32 + 0.5)).abs() < 0.12,
            "level{i}: {:?}",
            s
        );
        assert!(s.pose.translation.x.abs() < 0.2);
    }
    assert!(w.stats().sleeping_bodies >= 4, "{:?}", w.stats());
    assert_eq!(w.stats().bodies_created, 6);
    assert!(w
        .drain_events()
        .iter()
        .any(|e| e.kind == EventKind::ContactStarted));
}
#[test]
fn total_mass_impulse_force_and_damping_have_physical_effects() {
    let mut w = PhysicsWorld::new(
        1,
        WorldConfig {
            gravity: Vec3::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let h = put(
        &mut w,
        1,
        BodyDesc {
            kind: BodyKind::Dynamic,
            mass: Some(2.),
            ..Default::default()
        },
    );
    w.impulse(h, Vec3::X * 2., None).unwrap();
    assert!((w.state(h).unwrap().linear_velocity.x - 1.).abs() < 1e-5);
    w.force(h, Vec3::X * 4., None).unwrap();
    steps(&mut w, 60);
    assert!((w.state(h).unwrap().linear_velocity.x - 3.).abs() < 0.03);
    w.clear_forces(h).unwrap();
    steps(&mut w, 60);
    assert!((w.state(h).unwrap().linear_velocity.x - 3.).abs() < 0.03);
    let pose = w.state(h).unwrap().pose;
    w.upsert(
        key(1),
        &BodyDesc {
            kind: BodyKind::Dynamic,
            mass: Some(2.),
            pose,
            linear_velocity: Vec3::X * 3.,
            linear_damping: 3.,
            ..Default::default()
        },
        &[ColliderDesc::default()],
    )
    .unwrap();
    steps(&mut w, 60);
    assert!(w.state(h).unwrap().linear_velocity.x < 0.2);
}
#[test]
fn contact_sensor_force_and_removed_exit_events_are_real() {
    let mut w = world();
    floor(&mut w);
    w.upsert(
        key(2),
        &cube(BodyKind::Fixed, Vec3::new(0., 2., 0.)),
        &[ColliderDesc {
            sensor: true,
            shape: Shape::Cuboid {
                half_extents: Vec3::new(2., 0.2, 2.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
    let h = w
        .upsert(
            key(1),
            &cube(BodyKind::Dynamic, Vec3::new(0., 4., 0.)),
            &[ColliderDesc {
                contact_force_threshold: Some(0.1),
                ..Default::default()
            }],
        )
        .unwrap();
    steps(&mut w, 240);
    let events = w.drain_events();
    for kind in [
        EventKind::SensorEntered,
        EventKind::SensorExited,
        EventKind::ContactStarted,
        EventKind::ContactForce,
    ] {
        assert!(events.iter().any(|e| e.kind == kind), "missing {kind:?}");
    }
    w.remove(key(1)).unwrap();
    assert!(
        w.drain_events()
            .iter()
            .any(|e| e.kind == EventKind::ContactStopped
                && [Some(e.body), e.other].contains(&Some(h)))
    );
    assert!(w.state(h).is_err());
}
#[test]
fn sensors_observe_kinematic_characters_without_solid_response() {
    let mut w = world();
    w.upsert(
        key(0),
        &BodyDesc::default(),
        &[ColliderDesc {
            sensor: true,
            ..Default::default()
        }],
    )
    .unwrap();
    let h = put(
        &mut w,
        1,
        BodyDesc {
            kind: BodyKind::KinematicPosition,
            pose: Pose::at(Vec3::X * 3.),
            character: true,
            ..Default::default()
        },
    );
    w.set_kinematic_target(h, Pose::default()).unwrap();
    steps(&mut w, 1);
    assert!(w
        .drain_events()
        .iter()
        .any(|e| e.kind == EventKind::SensorEntered));
    assert!(w.state(h).unwrap().pose.translation.length() < 0.01);
}
fn projectile(ccd: bool) -> f32 {
    let mut w = PhysicsWorld::new(
        1,
        WorldConfig {
            gravity: Vec3::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    w.upsert(
        key(0),
        &BodyDesc::default(),
        &[ColliderDesc {
            shape: Shape::Cuboid {
                half_extents: Vec3::new(0.025, 5., 5.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
    let h = w
        .upsert(
            key(1),
            &BodyDesc {
                kind: BodyKind::Dynamic,
                pose: Pose::at(Vec3::X * -5.),
                linear_velocity: Vec3::X * 1000.,
                ccd,
                ..Default::default()
            },
            &[ColliderDesc {
                shape: Shape::Ball { radius: 0.1 },
                ..Default::default()
            }],
        )
        .unwrap();
    steps(&mut w, 1);
    w.state(h).unwrap().pose.translation.x
}
#[test]
fn ccd_stops_high_speed_projectile_that_discrete_collision_misses() {
    let discrete = projectile(false);
    let ccd = projectile(true);
    assert!(discrete > 1., "discrete {discrete}");
    assert!(ccd < 0.01 && ccd > -0.5, "CCD {ccd}");
}
#[test]
fn sleep_wake_and_handle_generation_are_enforced() {
    let mut w = world();
    floor(&mut w);
    let h = put(&mut w, 1, cube(BodyKind::Dynamic, Vec3::new(0., 0.6, 0.)));
    steps(&mut w, 360);
    assert!(w.state(h).unwrap().sleeping);
    w.drain_events();
    w.impulse(h, Vec3::X, None).unwrap();
    steps(&mut w, 1);
    assert!(!w.state(h).unwrap().sleeping);
    assert!(w
        .drain_events()
        .iter()
        .any(|e| e.kind == EventKind::Awakened));
    w.remove(key(1)).unwrap();
    let newer = w
        .upsert(
            BodyKey {
                id: 1,
                generation: 1,
            },
            &BodyDesc::default(),
            &[ColliderDesc::default()],
        )
        .unwrap();
    assert_ne!(newer, h);
    assert!(w.state(h).is_err());
    assert!(w.set_pose(h, Pose::default(), true).is_err());
}
#[test]
fn unchanged_and_material_updates_keep_rigid_body_and_collider_handles() {
    let mut w = world();
    let desc = BodyDesc::default();
    let mut collider = ColliderDesc::default();
    let h = w.upsert(key(0), &desc, &[collider.clone()]).unwrap();
    for _ in 0..1000 {
        assert_eq!(w.upsert(key(0), &desc, &[collider.clone()]).unwrap(), h);
    }
    collider.friction = 0.9;
    assert_eq!(w.upsert(key(0), &desc, &[collider]).unwrap(), h);
    assert_eq!(w.stats().bodies_created, 1);
    assert_eq!(w.stats().colliders_created, 1);
    assert_eq!(w.stats().collider_updates, 1);
    for id in 1..10 {
        put(&mut w, id, desc.clone());
    }
    assert_eq!(w.stats().shape_builds, 1);
    assert!(w.stats().shape_cache_hits >= 9);
    w.update_queries();
    let count = w.stats().query_updates;
    w.update_queries();
    assert_eq!(count, w.stats().query_updates);
}
#[test]
fn body_sync_does_not_reapply_gravity_or_teleport_an_advancing_dynamic_body() {
    let mut w = world();
    let desc = cube(BodyKind::Dynamic, Vec3::Y * 10.);
    let h = put(&mut w, 1, desc.clone());
    for _ in 0..60 {
        w.upsert(key(1), &desc, &[ColliderDesc::default()]).unwrap();
        w.step_fixed(1. / 60.).unwrap();
    }
    let s = w.state(h).unwrap();
    assert!((s.linear_velocity.y + 9.81).abs() < 0.03);
    assert!(s.pose.translation.y < 5.5 && s.pose.translation.y > 4.5);
}
#[test]
fn per_client_and_per_world_handles_never_alias() {
    let mut a = world();
    let mut b = PhysicsWorld::new(9, WorldConfig::default()).unwrap();
    let ha = put(&mut a, 1, BodyDesc::default());
    let hb = put(&mut b, 1, BodyDesc::default());
    assert!(a.state(hb).is_err());
    assert!(b.impulse(ha, Vec3::X, None).is_err());
    assert!(a
        .create_joint(
            ha,
            hb,
            &JointDesc::Spherical {
                local_anchor1: Vec3::ZERO,
                local_anchor2: Vec3::ZERO,
                contacts_enabled: false
            }
        )
        .is_err());
    assert!(a
        .raycast(
            Vec3::Y * 3.,
            -Vec3::Y,
            10.,
            QueryOptions {
                ignore: Some(hb),
                ..Default::default()
            }
        )
        .is_err());
    let mut same_client = world();
    put(&mut same_client, 1, BodyDesc::default());
    assert!(same_client.state(ha).is_err());
}
#[test]
fn query_groups_sensor_filters_and_incremental_pose_updates() {
    let mut w = world();
    let h = put(&mut w, 1, BodyDesc::default());
    let hit = w
        .raycast(Vec3::X * -3., Vec3::X, 10., QueryOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(hit.body, h);
    assert!((hit.distance - 2.5).abs() < 1e-5);
    assert!(
        w.sphere_cast(Vec3::X * -3., Vec3::X, 0.5, 10., QueryOptions::default())
            .unwrap()
            .unwrap()
            .distance
            < 2.01
    );
    assert_eq!(
        w.overlaps(
            &Shape::Ball { radius: 0.25 },
            Pose::default(),
            QueryOptions::default()
        )
        .unwrap()
        .len(),
        1
    );
    w.set_pose(h, Pose::at(Vec3::Y * 10.), false).unwrap();
    assert!(w
        .raycast(Vec3::X * -3., Vec3::X, 10., QueryOptions::default())
        .unwrap()
        .is_none());
    w.upsert(
        key(2),
        &BodyDesc::default(),
        &[ColliderDesc {
            sensor: true,
            groups: CollisionGroups {
                memberships: 2,
                filter: 2,
            },
            ..Default::default()
        }],
    )
    .unwrap();
    assert!(w
        .raycast(Vec3::X * -3., Vec3::X, 10., QueryOptions::default())
        .unwrap()
        .is_none());
    let opts = QueryOptions {
        include_sensors: true,
        groups: Some(CollisionGroups {
            memberships: 2,
            filter: 2,
        }),
        ..Default::default()
    };
    assert!(w
        .raycast(Vec3::X * -3., Vec3::X, 10., opts)
        .unwrap()
        .is_some());
    assert!(w
        .raycast(
            Vec3::X * -3.,
            Vec3::X,
            10.,
            QueryOptions {
                groups: Some(CollisionGroups {
                    memberships: 1,
                    filter: 1
                }),
                ..opts
            }
        )
        .unwrap()
        .is_none());
}
#[test]
fn incompatible_collision_groups_really_disable_response() {
    let mut w = world();
    w.upsert(
        key(0),
        &BodyDesc::default(),
        &[ColliderDesc {
            groups: CollisionGroups {
                memberships: 1,
                filter: 1,
            },
            shape: Shape::Cuboid {
                half_extents: Vec3::new(5., 0.1, 5.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
    let h = w
        .upsert(
            key(1),
            &cube(BodyKind::Dynamic, Vec3::Y * 2.),
            &[ColliderDesc {
                groups: CollisionGroups {
                    memberships: 2,
                    filter: 2,
                },
                ..Default::default()
            }],
        )
        .unwrap();
    steps(&mut w, 90);
    assert!(w.state(h).unwrap().pose.translation.y < -1.);
}
#[test]
fn exact_scaled_rotated_shapes_and_shear_rejection() {
    let mut w = world();
    let shape = Shape::Cuboid {
        half_extents: Vec3::splat(0.5),
    }
    .scaled(Vec3::new(2., 3., 4.))
    .unwrap();
    w.upsert(
        key(0),
        &BodyDesc::default(),
        &[ColliderDesc {
            shape,
            ..Default::default()
        }],
    )
    .unwrap();
    assert!(
        (w.raycast(Vec3::X * 5., -Vec3::X, 10., QueryOptions::default())
            .unwrap()
            .unwrap()
            .distance
            - 4.)
            .abs()
            < 1e-5
    );
    let rotation = Quat::from_rotation_y(0.6);
    let matrix = Mat4::from_scale_rotation_translation(Vec3::new(2., 3., 4.), rotation, Vec3::ONE);
    let (p, s) = decompose_transform(matrix).unwrap();
    assert!(p.rotation.dot(rotation).abs() > 0.9999);
    assert!((s - Vec3::new(2., 3., 4.)).length() < 1e-5);
    assert!(decompose_transform(
        Mat4::from_scale(Vec3::new(2., 1., 1.)) * Mat4::from_rotation_y(0.6)
    )
    .is_err());
    assert!(Shape::Ball { radius: 1. }
        .scaled(Vec3::new(1., 2., 1.))
        .is_err());
    assert!(Shape::Capsule {
        radius: 0.5,
        half_height: 1.
    }
    .scaled(Vec3::new(1., 2., 1.))
    .is_err());
}
#[test]
fn terrain_and_triangle_mesh_queries_use_the_supplied_geometry() {
    let mut w = world();
    let mesh = Shape::TriMesh {
        vertices: Arc::new(vec![
            Vec3::new(-2., 0., -2.),
            Vec3::new(-2., 0., 2.),
            Vec3::new(2., 0., 2.),
            Vec3::new(2., 0., -2.),
        ]),
        indices: Arc::new(vec![[0, 1, 2], [0, 2, 3]]),
    };
    w.upsert(
        key(0),
        &cube(BodyKind::Fixed, Vec3::Y),
        &[ColliderDesc {
            shape: mesh,
            ..Default::default()
        }],
    )
    .unwrap();
    let hit = w
        .raycast(
            Vec3::new(0.1, 4., 0.2),
            -Vec3::Y,
            10.,
            QueryOptions::default(),
        )
        .unwrap()
        .unwrap();
    assert!((hit.point.y - 1.).abs() < 1e-5);
    w.upsert(
        key(1),
        &cube(BodyKind::Fixed, Vec3::new(8., 0., 0.)),
        &[ColliderDesc {
            shape: Shape::HeightField {
                heights: Arc::new(vec![0.; 9]),
                rows: 3,
                cols: 3,
                scale: Vec3::new(4., 1., 4.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
    assert!(w
        .raycast(
            Vec3::new(8.1, 3., 0.2),
            -Vec3::Y,
            10.,
            QueryOptions::default()
        )
        .unwrap()
        .is_some());
}
#[test]
fn kinematic_velocity_and_character_queries_do_not_double_integrate() {
    let mut w = world();
    floor(&mut w);
    let h = put(
        &mut w,
        1,
        BodyDesc {
            kind: BodyKind::KinematicVelocity,
            pose: Pose::at(Vec3::Y * 3.),
            linear_velocity: Vec3::X,
            ..Default::default()
        },
    );
    steps(&mut w, 60);
    assert!((w.state(h).unwrap().pose.translation.x - 1.).abs() < 0.001);
    let ch = w
        .upsert(
            key(2),
            &BodyDesc {
                kind: BodyKind::KinematicPosition,
                pose: Pose::at(Vec3::Y),
                character: true,
                ..Default::default()
            },
            &[ColliderDesc {
                shape: Shape::Capsule {
                    radius: 0.5,
                    half_height: 0.5,
                },
                ..Default::default()
            }],
        )
        .unwrap();
    w.upsert(
        key(3),
        &cube(BodyKind::Fixed, Vec3::new(2., 1., 0.)),
        &[ColliderDesc {
            shape: Shape::Cuboid {
                half_extents: Vec3::new(0.1, 3., 3.),
            },
            ..Default::default()
        }],
    )
    .unwrap();
    let movement = w
        .move_character(ch, Vec3::new(5., -0.1, 0.), CharacterConfig::default())
        .unwrap();
    assert!(movement.translation.x < 1.5, "{:?}", movement);
    assert_eq!(w.state(ch).unwrap().pose.translation, Vec3::Y);
}
#[test]
fn fixed_step_chunking_and_time_drop_are_reported() {
    let cfg = WorldConfig::default();
    let dt = cfg.fixed_dt as f64;
    let mut a = world();
    let mut b = world();
    let ha = put(&mut a, 1, cube(BodyKind::Dynamic, Vec3::Y * 20.));
    let hb = put(&mut b, 1, cube(BodyKind::Dynamic, Vec3::Y * 20.));
    for _ in 0..120 {
        a.advance(dt / 2.).unwrap();
    }
    for _ in 0..60 {
        b.advance(dt).unwrap();
    }
    assert!(
        (a.state(ha).unwrap().pose.translation - b.state(hb).unwrap().pose.translation).length()
            < 1e-5
    );
    let r = a.advance(1.).unwrap();
    assert_eq!(r.steps, 8);
    assert!(r.dropped_seconds > 0.8 && r.interpolation_alpha < 1.);
}
#[test]
fn idle_skip_never_hides_pending_sensor_edits_after_a_query_flush() {
    let mut w = world();
    w.upsert(
        key(0),
        &BodyDesc::default(),
        &[ColliderDesc {
            sensor: true,
            ..Default::default()
        }],
    )
    .unwrap();
    let body = put(&mut w, 1, BodyDesc::default());
    w.raycast(Vec3::Y * 3., -Vec3::Y, 10., QueryOptions::default())
        .unwrap();
    steps(&mut w, 1);
    assert!(w
        .drain_events()
        .iter()
        .any(|e| e.kind == EventKind::SensorEntered));
    let pipeline = w.stats().pipeline_steps;
    steps(&mut w, 60);
    assert_eq!(w.stats().pipeline_steps, pipeline);
    assert!(w.stats().idle_steps >= 60);
    w.set_pose(body, Pose::at(Vec3::Y * 5.), false).unwrap();
    w.raycast(Vec3::Y * 10., -Vec3::Y, 20., QueryOptions::default())
        .unwrap();
    steps(&mut w, 1);
    assert!(w
        .drain_events()
        .iter()
        .any(|e| e.kind == EventKind::SensorExited));
    assert_eq!(w.stats().pipeline_steps, pipeline + 1);
    w.configure(Vec3::ZERO, 1. / 60.).unwrap();
    steps(&mut w, 1);
    assert_eq!(w.stats().pipeline_steps, pipeline + 2);
}
#[test]
fn disabled_bodies_and_colliders_leave_queries_immediately_and_return_on_enable() {
    let mut w = world();
    let mut d = BodyDesc::default();
    let mut c = ColliderDesc::default();
    w.upsert(key(0), &d, &[c.clone()]).unwrap();
    assert!(w
        .raycast(Vec3::Y * 3., -Vec3::Y, 10., QueryOptions::default())
        .unwrap()
        .is_some());
    d.enabled = false;
    w.upsert(key(0), &d, &[c.clone()]).unwrap();
    assert!(w
        .raycast(Vec3::Y * 3., -Vec3::Y, 10., QueryOptions::default())
        .unwrap()
        .is_none());
    steps(&mut w, 1);
    d.enabled = true;
    w.upsert(key(0), &d, &[c.clone()]).unwrap();
    assert!(w
        .raycast(Vec3::Y * 3., -Vec3::Y, 10., QueryOptions::default())
        .unwrap()
        .is_some());
    c.enabled = false;
    w.upsert(key(0), &d, &[c]).unwrap();
    assert!(w
        .raycast(Vec3::Y * 3., -Vec3::Y, 10., QueryOptions::default())
        .unwrap()
        .is_none());
}
#[test]
fn mass_override_is_total_across_multiple_colliders_and_rotation_locks_hold() {
    let mut w = PhysicsWorld::new(
        1,
        WorldConfig {
            gravity: Vec3::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let h = w
        .upsert(
            key(0),
            &BodyDesc {
                kind: BodyKind::Dynamic,
                mass: Some(6.),
                locked_rotation: [true; 3],
                ..Default::default()
            },
            &[
                ColliderDesc {
                    local_pose: Pose::at(-Vec3::X),
                    ..Default::default()
                },
                ColliderDesc {
                    local_pose: Pose::at(Vec3::X),
                    shape: Shape::Ball { radius: 0.7 },
                    density: 3.,
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    w.impulse(h, Vec3::X * 6., None).unwrap();
    assert!((w.state(h).unwrap().linear_velocity.x - 1.).abs() < 1e-5);
    w.torque(h, Vec3::Z * 100., true).unwrap();
    steps(&mut w, 120);
    assert!(
        w.state(h)
            .unwrap()
            .pose
            .rotation
            .angle_between(Quat::IDENTITY)
            < 1e-5
    );
}
#[test]
fn convex_hull_uses_native_collision_and_invalid_shapes_fail_before_replacing_a_body() {
    let mut w = world();
    floor(&mut w);
    let points = Arc::new(
        [-0.5, 0.5]
            .into_iter()
            .flat_map(|x| {
                [-0.5, 0.5]
                    .into_iter()
                    .flat_map(move |y| [-0.5, 0.5].into_iter().map(move |z| Vec3::new(x, y, z)))
            })
            .collect(),
    );
    let h = w
        .upsert(
            key(1),
            &cube(BodyKind::Dynamic, Vec3::Y * 3.),
            &[ColliderDesc {
                shape: Shape::ConvexHull { points },
                ..Default::default()
            }],
        )
        .unwrap();
    steps(&mut w, 240);
    assert!((w.state(h).unwrap().pose.translation.y - 0.5).abs() < 0.05);
    let bad = ColliderDesc {
        shape: Shape::Ball { radius: -1. },
        ..Default::default()
    };
    assert!(w.upsert(key(1), &BodyDesc::default(), &[bad]).is_err());
    assert!(w.state(h).is_ok());
    assert_eq!(w.stats().bodies_created, 2);
}
#[test]
fn total_mass_survives_one_of_multiple_collider_shapes_changing() {
    let mut w = PhysicsWorld::new(
        1,
        WorldConfig {
            gravity: Vec3::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let desc = BodyDesc {
        kind: BodyKind::Dynamic,
        mass: Some(4.),
        ..Default::default()
    };
    let mut colliders = vec![
        ColliderDesc {
            local_pose: Pose::at(-Vec3::X),
            ..Default::default()
        },
        ColliderDesc {
            local_pose: Pose::at(Vec3::X),
            ..Default::default()
        },
    ];
    let h = w.upsert(key(0), &desc, &colliders).unwrap();
    colliders[0].shape = Shape::Ball { radius: 1. };
    assert_eq!(h, w.upsert(key(0), &desc, &colliders).unwrap());
    w.impulse(h, Vec3::X * 4., None).unwrap();
    assert!((w.state(h).unwrap().linear_velocity.x - 1.).abs() < 1e-5);
}
#[test]
fn large_translation_does_not_hide_shear_or_perspective() {
    let shear = Mat4::from_translation(Vec3::splat(1_000_000.))
        * Mat4::from_scale(Vec3::new(2., 1., 1.))
        * Mat4::from_rotation_y(0.6);
    assert!(decompose_transform(shear).is_err());
    let mut perspective = Mat4::IDENTITY;
    perspective.x_axis.w = 0.1;
    assert!(decompose_transform(perspective).is_err());
}
