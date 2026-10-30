//! Existing CpuWorld owns particle pools, one sequenced RNG and completion.
use crysirror_vfx::falling_atlas::*;
use crysirror_vfx::screen::ScreenSpriteTarget;
use crysirror_vfx::*;
use glam::Mat4;
fn node() -> FallingAtlasSpec {
    FallingAtlasSpec {
        canvas: [32, 12],
        tick_hz: 60,
        x_bands: vec![0, -1, 1],
        fall_range: [2, 4],
        fall_divisor: 2.,
        sway_range: [0, 3],
        phase_modulus: 5,
        phase_step: 0.05,
        wind_step: 0.25,
        rows: vec![0, 1],
        row_frames: vec![2, 3],
        cell: [4, 4],
        stride: [5, 4],
        texture_extent: [16, 8],
        scale: 0.5,
        alpha_base: 0.5,
        alpha_step: 0.1,
        alpha_count: 3,
        alpha_offset: 0.,
        atlas_interval_ms: 300,
        atlas_clock_bits: 32,
    }
}
fn emitter(name: &str, layer: u32) -> EmitterSpec {
    let mut emitter = EmitterSpec::default();
    emitter.id = name.into();
    emitter.capacity = 1;
    emitter.space = SimulationSpace::Local;
    emitter.spawn.rate = 0.0.into();
    emitter.spawn.bursts = vec![Burst { time: 0., count: 1 }];
    emitter.spawn.burst_looping = false;
    emitter.screen = Some(ScreenSpriteTarget {
        canvas: [32, 12],
        layer,
        parent: None,
        order: [0, layer as i32],
        anchor: [0., 0.],
    });
    emitter.falling_atlas = Some(node());
    emitter.render = RenderSpec::Sprite {
        texture: Some("authored.png".into()),
        columns: 1,
        rows: 1,
        frames: 1,
        fps: 0.,
        soft_distance: 0.,
        blend: BlendMode::Alpha,
        facing: FacingMode::Camera,
    };
    emitter
}
fn input() -> SystemInstance {
    SystemInstance {
        key: 77,
        client: 4,
        transform: Mat4::IDENTITY.to_cols_array_2d(),
        spec: SystemSpec {
            emitters: vec![emitter("first", 1), emitter("second", 2)],
            clock: SimulationClock::External {
                tick: 0,
                wall_time_ms: 0,
            },
            sequence_rng: Some(SequenceRng::msvc(1)),
            ..Default::default()
        },
    }
}
#[test]
fn existing_particle_world_initializes_two_layers_from_one_completed_stream() {
    let mut world = CpuWorld::default();
    let input = input();
    let frame = world.advance(&[input.clone()], 0).unwrap();
    assert_eq!(frame.emitters.len(), 2);
    assert_eq!(frame.stats.live_particles, 2);
    let compiled = CompiledFallingAtlas::compile(node()).unwrap();
    let mut expected = SequenceRng::msvc(1);
    for (spec, actual) in input.spec.emitters.iter().zip(&frame.emitters) {
        let mut p = spawn_particle(
            &spec.compile(&Default::default()).unwrap(),
            0,
            input.transform,
        )
        .unwrap();
        let state = compiled.initialize(&mut p, &mut expected, 0).unwrap();
        assert_eq!(actual.particles[0], p);
        assert_eq!(
            actual.screen_uvs.as_ref().unwrap()[0],
            compiled.initial_uv(&state)
        );
    }
    assert_eq!(frame.sequence_states[0].completed_tick, 0);
    assert_eq!(frame.sequence_states[0].sequence_rng, expected);
    let packed = screen::pack_screen_emitter(
        4,
        &frame.emitters[0],
        &screen::ScreenSpriteOptions {
            canvas: [32, 12],
            anchor: [0., 0.],
        },
        frame.emitters[0].screen_uvs.as_deref(),
    )
    .unwrap();
    assert_eq!(packed.parts.len(), 1);
}
#[test]
fn explicit_position_tick_and_wall_atlas_clock_are_independent() {
    let mut world = CpuWorld::default();
    let mut input = input();
    let first = world.advance(&[input.clone()], 0).unwrap();
    let y = first.emitters[0].particles[0].position[1];
    let speed = first.emitters[0].particles[0].velocity[1] / 60.;
    input.spec.clock = SimulationClock::External {
        tick: 1,
        wall_time_ms: 301,
    };
    let next = world.advance(&[input.clone()], 0).unwrap();
    assert!((next.emitters[0].particles[0].position[1] - y - speed).abs() < 0.0001);
    assert_eq!(next.sequence_states[0].completed_tick, 1);
    input.spec.clock = SimulationClock::External {
        tick: 1,
        wall_time_ms: 100000,
    };
    let stopped = world.advance(&[input.clone()], 16).unwrap();
    assert_eq!(stopped.emitters[0].particles, next.emitters[0].particles);
    assert_eq!(stopped.emitters[0].screen_uvs, next.emitters[0].screen_uvs);
    input.spec.clock = SimulationClock::External {
        tick: 2,
        wall_time_ms: 100000,
    };
    let next = world.advance(&[input], 0).unwrap();
    assert!((next.emitters[0].particles[0].position[1] - y - speed * 2.).abs() < 0.0001);
    assert_eq!(next.sequence_states[0].completed_tick, 2);
}
#[test]
fn stream_exchange_requires_acknowledged_base_and_does_not_reinitialize_particles() {
    let mut world = CpuWorld::default();
    let mut input = input();
    let initial = world.advance(&[input.clone()], 0).unwrap();
    let mut borrowed = initial.sequence_states[0].sequence_rng.clone();
    borrowed.next_u32().unwrap();
    input.spec.sequence_exchange = Some(SequenceExchange {
        epoch: 1,
        base_tick: 0,
        state: borrowed.clone(),
    });
    let ack = world.advance(&[input.clone()], 0).unwrap();
    assert_eq!(ack.sequence_states[0].sequence_rng, borrowed);
    assert_eq!(ack.sequence_states[0].exchange_epoch, 1);
    assert_eq!(ack.emitters[0].particles, initial.emitters[0].particles);
    input.spec.clock = SimulationClock::External {
        tick: 1,
        wall_time_ms: 17,
    };
    input.spec.sequence_exchange.as_mut().unwrap().epoch = 2;
    input.spec.sequence_exchange.as_mut().unwrap().base_tick = 1;
    assert!(world.advance(&[input.clone()], 0).is_err());
    assert_eq!(world.snapshot().sequence_states[0].completed_tick, 0);
    input.spec.sequence_exchange = None;
    input.spec.sequence_rng = Some(SequenceRng::msvc(99));
    assert!(world.advance(&[input], 0).is_err());
}
#[test]
fn receipt_counts_completed_steps_instead_of_last_accepted_tick() {
    let mut world = CpuWorld::default();
    let mut input = input();
    input.spec.max_steps_per_advance = 1;
    world.advance(&[input.clone()], 0).unwrap();
    input.spec.clock = SimulationClock::External {
        tick: 3,
        wall_time_ms: 51,
    };
    for expected in 1..=3 {
        let frame = world.advance(&[input.clone()], 0).unwrap();
        assert_eq!(frame.sequence_states[0].completed_tick, expected);
    }
    assert!(world.is_caught_up());
}
