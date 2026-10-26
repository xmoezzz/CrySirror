// SPDX-License-Identifier: MPL-2.0
//! CPU-only behavior and ownership: no renderer, device, filesystem or game.
use crysirror_vfx::*;
use glam::{Mat4, Vec3};

fn emitter(id: &str) -> EmitterSpec {
    EmitterSpec {
        id: id.into(),
        capacity: 128,
        spawn: SpawnSpec {
            rate: 10.0.into(),
            duration: 10.,
            ..Default::default()
        },
        initialize: InitSpec {
            lifetime: [10.; 2],
            speed: [0.; 2],
            spread_degrees: 0.,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn input() -> SystemInstance {
    SystemInstance {
        key: 77,
        client: 4,
        transform: Mat4::IDENTITY.to_cols_array_2d(),
        spec: SystemSpec {
            emitters: vec![emitter("main")],
            fixed_step_hz: 10,
            ..Default::default()
        },
    }
}
fn particles(frame: &FrameSnapshot) -> &[Particle] {
    &frame.emitters[0].particles
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}

#[test]
fn fixed_tick_partition_and_worker_move_have_the_same_particles() {
    let mut i = input();
    i.spec.emitters[0].update.push(Module::Acceleration {
        value: [0., 10., 0.].into(),
    });
    let mut one = CpuWorld::default();
    let a = one.advance(&[i.clone()], 1000).unwrap();
    let b = std::thread::spawn(move || {
        let mut world = CpuWorld::default();
        for _ in 0..100 {
            world.advance(&[i.clone()], 10).unwrap();
        }
        world.snapshot().clone()
    })
    .join()
    .unwrap();
    assert_eq!(particles(&a), particles(&b));
    assert_eq!(particles(&a).len(), 10);
    near(particles(&a)[0].position[1], 5.5);
    near(particles(&a)[0].velocity[1], 10.);
}
#[test]
fn stop_preserves_live_particles_pause_freezes_and_parameters_do_not_reset() {
    let mut i = input();
    i.spec
        .parameters
        .insert("accel".into(), ParameterValue::Vector([0., 1., 0.]));
    i.spec.emitters[0].update.push(Module::Acceleration {
        value: VectorValue::Parameter(VectorBinding {
            parameter: "accel".into(),
            scale: 1.,
        }),
    });
    let mut world = CpuWorld::default();
    let first = world.advance(&[i.clone()], 300).unwrap();
    let held = first.clone();
    i.spec.enabled = false;
    let stopped = world.advance(&[i.clone()], 100).unwrap();
    assert_eq!(particles(&stopped).len(), 3);
    assert!(particles(&stopped)[0].age > particles(&first)[0].age);
    i.spec.paused = true;
    let frozen = world.advance(&[i.clone()], 1000).unwrap();
    assert_eq!(particles(&stopped), particles(&frozen));
    i.spec.paused = false;
    i.spec
        .parameters
        .insert("accel".into(), ParameterValue::Vector([0., 9., 0.]));
    let changed = world.advance(&[i], 100).unwrap();
    assert_eq!(particles(&changed)[0].id, particles(&first)[0].id);
    assert!(particles(&changed)[0].velocity[1] > particles(&stopped)[0].velocity[1] + 0.8);
    assert_eq!(
        particles(&held)[0].age,
        particles(&first)[0].age,
        "immutable snapshots keep their resource generations"
    );
}
#[test]
fn emitter_enabled_parameter_stops_births_without_despawning() {
    let mut i = input();
    i.spec
        .parameters
        .insert("emit".into(), ParameterValue::Bool(true));
    i.spec.emitters[0].enabled_parameter = Some("emit".into());
    let mut world = CpuWorld::default();
    let first = world.advance(&[i.clone()], 200).unwrap();
    i.spec
        .parameters
        .insert("emit".into(), ParameterValue::Bool(false));
    let second = world.advance(&[i], 200).unwrap();
    assert_eq!(particles(&first).len(), 2);
    assert_eq!(particles(&second).len(), 2);
    assert!(particles(&second)[0].age > particles(&first)[0].age);
}
#[test]
fn global_reservation_and_client_validation_happen_before_mutation() {
    let budget = WorldBudget {
        max_particles: 128,
        ..Default::default()
    };
    let mut world = CpuWorld::new(budget).unwrap();
    let i = input();
    let old = world.advance(&[i.clone()], 100).unwrap();
    let mut over = i.clone();
    over.spec.emitters[0].capacity = 129;
    assert!(world.advance(&[over], 100).is_err());
    assert_eq!(world.snapshot(), &old);
    let mut forged = i;
    forged.client = 5;
    assert!(world.advance(&[forged], 100).is_err());
    assert_eq!(world.snapshot(), &old);
}
#[test]
fn bounded_advances_keep_backlog_and_offline_zero_delta_drains_it() {
    let mut i = input();
    i.spec.max_steps_per_advance = 2;
    let mut world = CpuWorld::default();
    let frame = world.advance(&[i.clone()], 1000).unwrap();
    assert_eq!(frame.stats.simulated_ticks, 2);
    assert!(!world.is_caught_up());
    for _ in 0..4 {
        world.advance(&[i.clone()], 0).unwrap();
    }
    assert!(world.is_caught_up());
    assert_eq!(particles(world.snapshot()).len(), 10);
    near(particles(world.snapshot())[0].age, 1.);
}
#[test]
fn warmup_is_explicit_bounded_work_not_a_renderer_side_effect() {
    let mut i = input();
    i.spec.warmup_seconds = 0.5;
    i.spec.max_steps_per_advance = 2;
    let mut world = CpuWorld::default();
    let a = world.advance(&[i.clone()], 0).unwrap();
    assert_eq!(a.stats.simulated_ticks, 2);
    world.advance(&[i.clone()], 0).unwrap();
    let b = world.advance(&[i], 0).unwrap();
    assert_eq!(particles(&b).len(), 5);
    assert!(world.is_caught_up());
}
#[test]
fn local_particles_follow_the_transform_world_particles_do_not() {
    let mut world_input = input();
    world_input.spec.emitters[0].spawn.rate = 0.0.into();
    world_input.spec.emitters[0].spawn.bursts = vec![Burst { time: 0., count: 1 }];
    world_input.transform = Mat4::from_translation(Vec3::new(10., 0., 0.)).to_cols_array_2d();
    let mut local = world_input.clone();
    local.key = 78;
    local.spec.emitters[0].space = SimulationSpace::Local;
    let mut world = CpuWorld::default();
    let a = world
        .advance(&[world_input.clone(), local.clone()], 100)
        .unwrap();
    near(a.emitters[0].particles[0].position[0], 10.);
    near(a.emitters[1].particles[0].position[0], 0.);
    world_input.transform = Mat4::from_translation(Vec3::new(20., 0., 0.)).to_cols_array_2d();
    local.transform = world_input.transform;
    let b = world.advance(&[world_input, local], 100).unwrap();
    near(b.emitters[0].particles[0].position[0], 10.);
    near(b.emitters[1].particles[0].position[0], 0.);
    assert_eq!(b.emitters[1].transform[3][0], 20.);
}
#[test]
fn collisions_bounce_kill_and_publish_engine_owned_events() {
    let mut i = input();
    let e = &mut i.spec.emitters[0];
    e.spawn.rate = 0.0.into();
    e.spawn.bursts = vec![Burst { time: 0., count: 1 }];
    e.initialize.speed = [1.; 2];
    e.initialize.direction = [0., -1., 0.].into();
    e.update.push(Module::CollisionPlane {
        normal: [0., 1., 0.].into(),
        offset: 0.0.into(),
        restitution: 1.,
        friction: 0.,
        kill: false,
    });
    let mut world = CpuWorld::default();
    let f = world.advance(&[i.clone()], 100).unwrap();
    near(particles(&f)[0].position[1], 0.);
    near(particles(&f)[0].velocity[1], 1.);
    assert!(f
        .events
        .iter()
        .any(|e| e.kind == EventKind::Collision && e.system_key == 77));
    if let Module::CollisionPlane { kill, .. } = &mut i.spec.emitters[0].update[0] {
        *kill = true;
    }
    let f = world.advance(&[i], 100).unwrap();
    assert!(particles(&f).is_empty());
    assert!(f.events.iter().any(|e| e.kind == EventKind::Death));
}
#[test]
fn death_and_birth_subemitters_keep_space_and_defer_chain_work() {
    let mut i = input();
    i.transform = Mat4::from_translation(Vec3::new(10., 0., 0.)).to_cols_array_2d();
    let mut parent = emitter("parent");
    parent.space = SimulationSpace::Local;
    parent.spawn.rate = 0.0.into();
    parent.spawn.bursts = vec![Burst { time: 0., count: 1 }];
    parent.initialize.speed = [1.; 2];
    parent.initialize.lifetime = [0.05; 2];
    parent.events.push(EventHandler {
        event: EventKind::Death,
        emitter: "child".into(),
        count: 2,
        inherit_velocity: 2.,
        probability: 1.,
    });
    let mut child = emitter("child");
    child.spawn.auto_start = false;
    child.events.push(EventHandler {
        event: EventKind::Birth,
        emitter: "grandchild".into(),
        count: 1,
        inherit_velocity: 0.,
        probability: 1.,
    });
    let mut grandchild = emitter("grandchild");
    grandchild.spawn.auto_start = false;
    i.spec.emitters = vec![parent, child, grandchild];
    let mut world = CpuWorld::default();
    let first = world.advance(&[i.clone()], 100).unwrap();
    assert_eq!(first.emitters[1].particles.len(), 2);
    assert!(first.emitters[2].particles.is_empty());
    near(first.emitters[1].particles[0].position[0], 10.);
    near(first.emitters[1].particles[0].velocity[1], 2.);
    let death = first
        .events
        .iter()
        .find(|e| e.emitter_id == "parent" && e.kind == EventKind::Death)
        .unwrap();
    near(death.position[0], 10.);
    assert_eq!(death.client, 4);
    let next = world.advance(&[i], 100).unwrap();
    assert_eq!(next.emitters[2].particles.len(), 2);
}
#[test]
fn overflow_and_events_are_bounded_with_visible_diagnostics() {
    let mut i = input();
    let e = &mut i.spec.emitters[0];
    e.capacity = 4;
    e.overflow = OverflowPolicy::KillOldest;
    e.spawn.rate = 0.0.into();
    e.spawn.bursts = vec![Burst {
        time: 0.,
        count: 100,
    }];
    let mut world = CpuWorld::new(WorldBudget {
        max_events: 2,
        ..Default::default()
    })
    .unwrap();
    let f = world.advance(&[i], 100).unwrap();
    assert_eq!(
        particles(&f).iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![96, 97, 98, 99]
    );
    assert_eq!(f.stats.dropped_spawns, 96);
    assert_eq!(f.events.len(), 2);
    assert_eq!(f.stats.dropped_events, 2);
}
#[test]
fn typed_bindings_curves_and_unknown_operations_fail_visibly() {
    let mut i = input();
    i.spec.emitters[0].update.push(Module::Drag {
        coefficient: ScalarValue::Parameter(ScalarBinding {
            parameter: "x".into(),
            scale: 1.,
            offset: 0.,
        }),
    });
    assert!(i.spec.validate().is_err());
    i.spec
        .parameters
        .insert("x".into(), ParameterValue::Vector([1.; 3]));
    assert!(i.spec.validate().is_err());
    i.spec
        .parameters
        .insert("x".into(), ParameterValue::Float(2.));
    i.spec.validate().unwrap();
    assert!(i
        .spec
        .with_parameters(
            &[("x".into(), ParameterValue::Bool(true))]
                .into_iter()
                .collect()
        )
        .is_err());
    assert!(serde_json::from_str::<Module>(r#"{"op":"pretend_success","value":1}"#).is_err());
    let mut curve = ScalarCurve::linear(2., 4.);
    near(curve.sample(0.5), 3.);
    curve.keys[1].time = 0.;
    assert!(curve.validate().is_err());
}
#[test]
fn shape_randomness_and_birth_parameter_bindings_are_reproducible() {
    let mut e = emitter("shapes");
    e.initialize.shape = SpawnShape::Sphere {
        radius: 2.,
        surface: true,
    };
    e.initialize.color_parameter = Some("color".into());
    e.initialize.size_parameter = Some("size".into());
    let p = [
        ("color".into(), ParameterValue::Color([3., 2., 1., 0.5])),
        ("size".into(), ParameterValue::Vector([0.4, 0.8, 0.])),
    ]
    .into_iter()
    .collect();
    let c = e.compile(&p).unwrap();
    let a = spawn_particle(&c, 19, Mat4::IDENTITY.to_cols_array_2d()).unwrap();
    let b = spawn_particle(&c, 19, Mat4::IDENTITY.to_cols_array_2d()).unwrap();
    assert_eq!(a, b);
    near(Vec3::from_array(a.position).length(), 2.);
    assert_eq!(a.color, [3., 2., 1., 0.5]);
    assert_eq!(a.size, [0.4, 0.8]);
    assert_ne!(random_u32(1, 19, 0), random_u32(1, 19, 1));
    assert_ne!(random_u32(1, 19, 0), random_u32(1, (1 << 32) | 19, 0));
}
#[test]
fn gpu_planner_is_owned_tick_history_not_cpu_particle_simulation() {
    let mut i = input();
    i.spec.emitters[0].target = SimulationTarget::Gpu;
    let mut world = CpuWorld::default();
    let first = world.advance(&[i.clone()], 100).unwrap();
    assert!(first.emitters.is_empty());
    assert_eq!(first.stats.live_particles, 0);
    assert_eq!(first.gpu_emitters[0].steps.len(), 1);
    let receipt = first.gpu_emitters[0].progress.clone();
    receipt.mark_submitted(1);
    assert_eq!(receipt.submitted_tick(), 1);
    i.spec
        .parameters
        .insert("value".into(), ParameterValue::Float(3.));
    let next = world.advance(&[i.clone()], 100).unwrap();
    assert_eq!(next.gpu_emitters[0].progress, receipt);
    assert!(!next.gpu_emitters[0].steps[0]
        .parameters
        .contains_key("value"));
    assert_eq!(
        next.gpu_emitters[0].steps[1].parameters["value"],
        ParameterValue::Float(3.)
    );
    i.spec.revision += 1;
    let reset = world.advance(&[i], 100).unwrap();
    assert_ne!(reset.gpu_emitters[0].progress, receipt);
    assert_eq!(reset.gpu_emitters[0].steps[0].tick, 1);
}

#[test]
fn initial_burst_is_visible_at_zero_time_and_is_not_replayed_next_tick() {
    for target in [SimulationTarget::Cpu, SimulationTarget::Gpu] {
        let mut i = input();
        let e = &mut i.spec.emitters[0];
        e.target = target;
        e.spawn.rate = 0.0.into();
        e.spawn.bursts = vec![Burst { time: 0., count: 3 }];
        let mut world = CpuWorld::default();
        let first = world.advance(&[i.clone()], 0).unwrap();
        assert!(world.is_caught_up());
        if target == SimulationTarget::Cpu {
            assert_eq!(particles(&first).len(), 3);
            assert!(particles(&first).iter().all(|p| p.age == 0.));
            assert_eq!(first.stats.simulated_ticks, 0);
        } else {
            assert_eq!(first.gpu_emitters[0].steps[0].count, 3);
            assert_eq!(first.gpu_emitters[0].steps[0].dt, 0.);
        }
        let next = world.advance(&[i], 100).unwrap();
        if target == SimulationTarget::Cpu {
            assert_eq!(particles(&next).len(), 3);
        } else {
            assert_eq!(next.gpu_emitters[0].steps[1].count, 0);
        }
    }
}

#[test]
fn gpu_bootstrap_counts_towards_the_retained_command_window() {
    let mut i = input();
    let e = &mut i.spec.emitters[0];
    e.target = SimulationTarget::Gpu;
    e.spawn.bursts = vec![Burst { time: 0., count: 1 }];
    i.spec.max_steps_per_advance = 64;
    let mut world = CpuWorld::default();
    let frame = world.advance(&[i], 4000).unwrap();
    assert_eq!(frame.gpu_emitters[0].steps.len(), 32);
    assert_eq!(frame.gpu_emitters[0].steps[0].dt, 0.);
    assert_eq!(frame.stats.gpu_history_dropped, 0);
    assert!(!world.is_caught_up());
}

#[test]
fn stopping_suppresses_child_births_but_keeps_death_events_observable() {
    let mut i = input();
    let mut parent = emitter("parent");
    parent.spawn.rate = 0.0.into();
    parent.spawn.bursts = vec![Burst { time: 0., count: 1 }];
    parent.initialize.lifetime = [0.05; 2];
    parent.events.push(EventHandler {
        event: EventKind::Death,
        emitter: "child".into(),
        count: 1,
        inherit_velocity: 0.,
        probability: 1.,
    });
    let mut child = emitter("child");
    child.spawn.auto_start = false;
    i.spec.emitters = vec![parent, child];
    let mut world = CpuWorld::default();
    world.advance(&[i.clone()], 0).unwrap();
    i.spec.enabled = false;
    let frame = world.advance(&[i], 100).unwrap();
    assert!(frame.emitters.iter().all(|e| e.particles.is_empty()));
    assert!(frame.events.iter().any(|e| e.kind == EventKind::Death));
}

#[test]
fn compilation_rejects_untextured_flipbooks_unordered_forces_and_unsupported_gpu_features() {
    let mut e = emitter("invalid");
    e.render = RenderSpec::Sprite {
        texture: None,
        columns: 2,
        rows: 2,
        frames: 4,
        fps: 1.,
        soft_distance: 0.,
        blend: BlendMode::Alpha,
        facing: FacingMode::Camera,
    };
    assert!(e.compile(&Default::default()).is_err());
    e.render = Default::default();
    e.update = vec![
        Module::CollisionSphere {
            center: [0.; 3].into(),
            radius: 1.0.into(),
            restitution: 1.,
            kill: false,
        },
        Module::Drag {
            coefficient: 1.0.into(),
        },
    ];
    assert!(e.compile(&Default::default()).is_err());
    e.update.clear();
    e.target = SimulationTarget::Gpu;
    e.overflow = OverflowPolicy::KillOldest;
    assert!(e.compile(&Default::default()).is_err());
    e.overflow = Default::default();
    e.render = RenderSpec::Ribbon {
        width: 0.1,
        texture: None,
        uv_scale: 1.,
        max_segments: 32,
        blend: BlendMode::Alpha,
    };
    assert!(e.compile(&Default::default()).is_err());
}

#[test]
fn one_shot_bursts_can_coexist_with_continuous_rate() {
    let mut i = input();
    let e = &mut i.spec.emitters[0];
    e.spawn.rate = 10.0.into();
    e.spawn.duration = 0.5;
    e.spawn.bursts = vec![Burst { time: 0., count: 3 }];
    e.spawn.burst_looping = false;
    let mut world = CpuWorld::default();
    let a = world.advance(&[i], 1000).unwrap();
    assert_eq!(particles(&a).len(), 13);
}
