// SPDX-License-Identifier: MPL-2.0
//! Explicit device tests for resident VFX; unavailable devices fail execution.
use compute::{GpuSimulation, SimulationStep};
use crysirror_vfx::{EmitterSpec, Module, SimulationTarget, SpawnShape};
use crysirror_wgpu::vfx_gpu as compute;
use crysirror_wgpu::Gpu;
use glam::Mat4;

fn device() -> Gpu {
    pollster::block_on(Gpu::request(&wgpu::Instance::default(), None))
        .expect("explicit VFX compute GPU required")
}
fn step(tick: u64, count: u32, first_id: u64) -> SimulationStep {
    SimulationStep {
        tick,
        dt: 1. / 60.,
        time: tick as f32 / 60.,
        spawn_count: count,
        first_id,
        seed: 77,
        transform: Mat4::IDENTITY.to_cols_array_2d(),
        eye: [0., 0., 10.],
        vertex_count: 6,
        sort_alpha: false,
        parameters: None,
    }
}
#[test]
fn compute_wgsl_is_valid_and_host_layout_is_exact() {
    let module =
        naga::front::wgsl::parse_str(include_str!("../src/shaders/vfx_compute.wgsl")).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    assert_eq!(std::mem::size_of::<compute::GpuParticle>(), 96);
    assert_eq!(std::mem::size_of::<compute::GpuEvent>(), 48);
}
#[test]
fn unsupported_gpu_profiles_fail_without_creating_a_device() {
    let mut e = EmitterSpec::default();
    e.target = SimulationTarget::Cpu;
    e.overflow = crysirror_vfx::OverflowPolicy::KillOldest;
    assert!(
        GpuSimulation::validate_program(&e.compile(&Default::default()).unwrap())
            .unwrap_err()
            .to_string()
            .contains("kill_oldest")
    );
    e.overflow = Default::default();
    e.render = crysirror_vfx::RenderSpec::Ribbon {
        width: 1.,
        texture: None,
        uv_scale: 1.,
        max_segments: 20,
        blend: Default::default(),
    };
    assert!(
        GpuSimulation::validate_program(&e.compile(&Default::default()).unwrap())
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("ribbon")
    );
}

#[test]
#[ignore = "requires real compute GPU; explicit shape/parameter/history parity"]
fn all_shapes_spaces_high_ids_and_parameter_history_match_cpu_births() {
    use crysirror_vfx::{ParameterValue, SimulationSpace};
    let gpu = device();
    let d = &gpu.device;
    for shape in [
        SpawnShape::Point,
        SpawnShape::Sphere {
            radius: 2.,
            surface: false,
        },
        SpawnShape::Box {
            half_extent: [1., 2., 3.],
        },
        SpawnShape::Cone {
            radius: 1.5,
            height: 2.,
        },
        SpawnShape::Line { end: [1., 2., 3.] },
    ] {
        for space in [SimulationSpace::World, SimulationSpace::Local] {
            let mut e = EmitterSpec::default();
            e.target = SimulationTarget::Gpu;
            e.capacity = 16;
            e.seed = 77;
            e.space = space;
            e.initialize.shape = shape.clone();
            e.initialize.direction = if matches!(shape, SpawnShape::Point) {
                [0., -1., 0.]
            } else {
                [0.2, 0.9, -0.3]
            }
            .into();
            e.initialize.color_parameter = Some("color".into());
            e.initialize.size_parameter = Some("size".into());
            e.update = vec![
                Module::Acceleration {
                    value: crysirror_vfx::VectorValue::Parameter(crysirror_vfx::VectorBinding {
                        parameter: "gravity".into(),
                        scale: 1.,
                    }),
                },
                Module::Orbit {
                    axis: [0., 1., 0.].into(),
                    speed: 0.5.into(),
                },
                Module::CollisionSphere {
                    center: [0., 0., 0.].into(),
                    radius: 0.4.into(),
                    restitution: 0.7,
                    kill: false,
                },
            ];
            let params = |gravity, color, size| {
                [
                    ("gravity".to_string(), ParameterValue::Vector(gravity)),
                    ("color".to_string(), ParameterValue::Color(color)),
                    ("size".to_string(), ParameterValue::Vector(size)),
                ]
                .into_iter()
                .collect()
            };
            let a = params([0., -1., 0.], [1., 0., 0., 1.], [1., 2., 0.]);
            let b = params([0., -2., 0.], [0., 0., 1., 1.], [3., 4., 0.]);
            // The latest frame's values deliberately differ from BOTH old ticks.
            let latest = params([0., -99., 0.], [0., 1., 0., 1.], [9., 9., 0.]);
            let plan = e.compile(&latest).unwrap();
            let first = (1u64 << 32) - 2;
            let transform = Mat4::from_scale_rotation_translation(
                glam::Vec3::new(2., 0.7, 1.3),
                glam::Quat::from_rotation_z(0.3),
                glam::Vec3::new(3., 2., -1.),
            )
            .to_cols_array_2d();
            let mut sa = step(1, 4, first);
            sa.dt = 0.;
            sa.time = 0.;
            sa.transform = transform;
            sa.parameters = Some(a.clone());
            let mut sb = step(2, 1, first + 4);
            sb.time = 1. / 60.;
            sb.transform = transform;
            sb.parameters = Some(b.clone());
            let mut cpu = (first..first + 4)
                .map(|id| {
                    crysirror_vfx::spawn_particle(&e.compile(&a).unwrap(), id, transform).unwrap()
                })
                .collect::<Vec<_>>();
            cpu.push(
                crysirror_vfx::spawn_particle(&e.compile(&b).unwrap(), first + 4, transform)
                    .unwrap(),
            );
            for q in &mut cpu {
                assert!(
                    crysirror_vfx::update_particle(
                        q,
                        &e.compile(&b).unwrap(),
                        transform,
                        sb.dt,
                        sb.time
                    )
                    .unwrap()
                    .0
                );
            }
            let mut simulation = GpuSimulation::new(d, 16).unwrap();
            let mut encoder = d.create_command_encoder(&Default::default());
            let token = simulation
                .encode_batch(d, &mut encoder, &plan, &[sa, sb])
                .unwrap();
            gpu.queue.submit([encoder.finish()]);
            simulation.mark_submitted(token).unwrap();
            let snapshot = simulation.readback_offline(d, &gpu.queue).unwrap();
            assert_eq!(snapshot.draw_args[1], 5);
            let mut active = snapshot
                .particles
                .into_iter()
                .filter(|q| q.active())
                .collect::<Vec<_>>();
            active.sort_by_key(|q| q.id());
            for (x, y) in active.iter().zip(&cpu) {
                assert_eq!(x.id(), y.id);
                assert_eq!(x.color, y.color);
                assert_eq!(&x.size[..2], &y.size);
                for (x, y) in x.position_age[..3].iter().zip(y.position) {
                    assert!(
                        (x - y).abs() < 0.003,
                        "shape {shape:?}, space {space:?}: {x} vs {y}"
                    );
                }
                assert!((x.position_age[3] - y.age).abs() < 0.00001);
            }
        }
    }
}
#[test]
#[ignore = "requires a real compute/storage GPU; execute explicitly"]
fn actual_resident_kernels_match_the_cpu_oracle_and_submission_boundaries() {
    let gpu = device();
    let d = &gpu.device;
    let mut spec = EmitterSpec::default();
    spec.target = SimulationTarget::Gpu;
    spec.capacity = 64;
    spec.seed = 77;
    spec.initialize.shape = SpawnShape::Box {
        half_extent: [1., 1., 1.],
    };
    spec.initialize.lifetime = [4., 6.];
    spec.initialize.speed = [0.5, 2.];
    spec.update = vec![
        Module::Acceleration {
            value: [0., -9.8, 0.].into(),
        },
        Module::Drag {
            coefficient: 0.1.into(),
        },
        Module::CurlNoise {
            strength: 0.2.into(),
            frequency: 1.3.into(),
        },
        Module::Attractor {
            point: [0., 0., 0.].into(),
            strength: 0.3.into(),
            radius: 10.0.into(),
        },
        Module::Orbit {
            axis: [0., 1., 0.].into(),
            speed: 0.2.into(),
        },
        Module::Rotate {
            radians_per_second: 0.5.into(),
        },
        Module::CollisionPlane {
            normal: [0., 1., 0.].into(),
            offset: (-2.).into(),
            restitution: 0.5,
            friction: 0.2,
            kill: false,
        },
    ];
    let plan = spec.compile(&Default::default()).unwrap();
    let mut simulation = GpuSimulation::new(d, 64).unwrap();
    let mut abandoned = d.create_command_encoder(&Default::default());
    let token = simulation
        .encode_batch(d, &mut abandoned, &plan, &[step(1, 16, 0)])
        .unwrap();
    assert_eq!(simulation.stats().committed_tick, None);
    drop(abandoned);
    simulation.cancel_pending(token).unwrap();
    let mut cpu = (0..16)
        .map(|id| {
            crysirror_vfx::spawn_particle(&plan, id, Mat4::IDENTITY.to_cols_array_2d()).unwrap()
        })
        .collect::<Vec<_>>();
    for tick in 1..=30 {
        let input = step(tick, if tick == 1 { 16 } else { 0 }, 16);
        let input = SimulationStep {
            first_id: if tick == 1 { 0 } else { 16 },
            ..input
        };
        for p in &mut cpu {
            assert!(
                crysirror_vfx::update_particle(p, &plan, input.transform, input.dt, input.time)
                    .unwrap()
                    .0
            );
        }
        let mut encoder = d.create_command_encoder(&Default::default());
        let token = simulation
            .encode_batch(d, &mut encoder, &plan, &[input])
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        simulation.mark_submitted(token).unwrap();
    }
    let snapshot = simulation.readback_offline(d, &gpu.queue).unwrap();
    assert_eq!(snapshot.draw_args, [6, 16, 0, 0]);
    assert_eq!(snapshot.indices.len(), 16);
    let mut actual = snapshot
        .particles
        .into_iter()
        .filter(|p| p.active())
        .collect::<Vec<_>>();
    actual.sort_by_key(|p| p.id());
    for (a, b) in actual.iter().zip(cpu.iter()) {
        assert_eq!(a.id(), b.id);
        for (x, y) in a.position_age[..3].iter().zip(b.position) {
            assert!((x - y).abs() < 0.002, "position id {}: {x} vs {y}", b.id);
        }
        for (x, y) in a.velocity_lifetime[..3].iter().zip(b.velocity) {
            assert!((x - y).abs() < 0.003, "velocity id {}: {x} vs {y}", b.id);
        }
        assert!((a.position_age[3] - b.age).abs() < 0.0001);
    }
    let mut encoder = d.create_command_encoder(&Default::default());
    assert!(simulation
        .encode_batch(d, &mut encoder, &plan, &[step(32, 0, 16)])
        .unwrap_err()
        .to_string()
        .contains("history gap"));
    let mut fresh = GpuSimulation::new(d, 64).unwrap();
    assert!(fresh
        .encode_batch(d, &mut encoder, &plan, &[step(50, 0, 16)])
        .is_err());
    fresh.acknowledge_visual_restart(50).unwrap();
    assert_eq!(fresh.stats().unreplayed_ticks, 49);
}

#[test]
#[ignore = "requires a real compute/storage GPU; execute explicitly"]
fn gpu_curves_pool_overflow_events_and_depth_sort_are_actual_buffer_results() {
    let gpu = device();
    let d = &gpu.device;
    let mut spec = EmitterSpec::default();
    spec.target = SimulationTarget::Gpu;
    spec.capacity = 32;
    spec.seed = 77;
    spec.initialize.shape = SpawnShape::Line { end: [0., 0., 10.] };
    spec.initialize.speed = [0.; 2];
    spec.initialize.lifetime = [0.1; 2];
    spec.update = vec![
        Module::ColorOverLife {
            curve: crysirror_vfx::ColorCurve::linear([1., 0., 0., 1.], [0., 0., 1., 0.]),
        },
        Module::SizeOverLife {
            curve: crysirror_vfx::Vec2Curve::linear([1., 2.], [3., 4.]),
        },
    ];
    let plan = spec.compile(&Default::default()).unwrap();
    let mut simulation = GpuSimulation::new(d, 32).unwrap();
    let mut s = step(1, 48, 0);
    s.sort_alpha = true;
    s.eye = [0.; 3];
    let mut encoder = d.create_command_encoder(&Default::default());
    let token = simulation
        .encode_batch(d, &mut encoder, &plan, &[s])
        .unwrap();
    gpu.queue.submit([encoder.finish()]);
    simulation.mark_submitted(token).unwrap();
    let r = simulation.readback_offline(d, &gpu.queue).unwrap();
    assert_eq!(r.draw_args[1], 32);
    assert_eq!(r.dropped_births, 16);
    assert_eq!(r.event_count, 32);
    assert_eq!(r.dropped_events, 0);
    let mut last = f32::INFINITY;
    for index in &r.indices {
        let q = &r.particles[*index as usize];
        let z = q.position_age[2];
        assert!(z <= last + 1e-6);
        last = z;
        assert!((q.color[2] - 1. / 6.).abs() < 0.0001);
        assert!((q.size[0] - 4. / 3.).abs() < 0.0001);
    }
    for tick in 2..=6 {
        let mut encoder = d.create_command_encoder(&Default::default());
        let token = simulation
            .encode_batch(d, &mut encoder, &plan, &[step(tick, 0, 48)])
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        simulation.mark_submitted(token).unwrap();
    }
    let dead = simulation.readback_offline(d, &gpu.queue).unwrap();
    assert_eq!(dead.draw_args[1], 0);
    assert_eq!(dead.event_count, 32);
    assert!(dead.events.iter().all(|e| e.identity[2] == 1));
}

#[test]
#[ignore = "requires timestamp-query compute GPU; explicit 100k offline benchmark"]
fn resident_hundred_thousand_particle_update_has_valid_gpu_timestamps() {
    let gpu = device();
    let d = &gpu.device;
    assert!(
        d.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        "actual GPU timestamps required"
    );
    let mut spec = EmitterSpec::default();
    spec.target = SimulationTarget::Gpu;
    spec.capacity = 100_000;
    spec.seed = 77;
    spec.initialize.lifetime = [10.; 2];
    spec.initialize.shape = SpawnShape::Box {
        half_extent: [10., 10., 10.],
    };
    spec.update = vec![
        Module::Acceleration {
            value: [0., -2., 0.].into(),
        },
        Module::Drag {
            coefficient: 0.2.into(),
        },
        Module::CurlNoise {
            strength: 0.4.into(),
            frequency: 0.7.into(),
        },
    ];
    let plan = spec.compile(&Default::default()).unwrap();
    let mut simulation = GpuSimulation::new(d, 100_000).unwrap();
    let cold = std::time::Instant::now();
    let mut encoder = d.create_command_encoder(&Default::default());
    let token = simulation
        .encode_batch(d, &mut encoder, &plan, &[step(1, 100_000, 0)])
        .unwrap();
    gpu.queue.submit([encoder.finish()]);
    simulation.mark_submitted(token).unwrap();
    let initial = simulation.readback_offline(d, &gpu.queue).unwrap();
    assert_eq!(initial.draw_args[1], 100_000);
    assert_eq!(initial.dropped_births, 0);
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.;
    let shader=d.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("VFX benchmark marker"),source:wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var<storage,read_write> marker:u32; @compute @workgroup_size(1) fn main(){marker=marker+1u;}".into())});
    let marker = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("VFX marker"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let scratch = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VFX marker storage"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VFX marker"),
        layout: &marker.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: scratch.as_entire_binding(),
        }],
    });
    let query = d.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("VFX benchmark timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 4,
    });
    let resolve = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VFX timing resolve"),
        size: 256,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VFX timing offline map"),
        size: 32,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let measure = |simulation: &mut GpuSimulation, input: SimulationStep| {
        let mut encoder = d.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("VFX timing begin"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &query,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
            pass.set_pipeline(&marker);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let token = simulation
            .encode_batch(d, &mut encoder, &plan, &[input])
            .unwrap();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("VFX timing end"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &query,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: Some(3),
                }),
            });
            pass.set_pipeline(&marker);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        gpu.queue.submit([encoder.finish()]);
        simulation.mark_submitted(token).unwrap();
        // Metal resolves only after completion; live frames never do this fence.
        d.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
        let mut encoder = d.create_command_encoder(&Default::default());
        encoder.resolve_query_set(&query, 0..4, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 32);
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        d.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(20))
            .unwrap()
            .unwrap();
        let data = read.slice(..).get_mapped_range();
        let t = bytemuck::cast_slice::<u8, u64>(&data);
        assert!(t[1] > 0 && t[2] > t[1], "invalid GPU timing: {t:?}");
        let us = (t[2] - t[1]) as f64 * f64::from(gpu.queue.get_timestamp_period()) / 1000.;
        drop(data);
        read.unmap();
        us
    };
    let mut samples = (2..=25)
        .map(|tick| measure(&mut simulation, step(tick, 0, 100_000)))
        .collect::<Vec<_>>();
    let state = simulation.readback_offline(d, &gpu.queue).unwrap();
    assert_eq!(state.draw_args[1], 100_000);
    assert!(
        state
            .particles
            .iter()
            .all(|q| q.active() && (q.position_age[3] - 25. / 60.).abs() < 0.0001),
        "every one of the 100k slots must actually advance all 25 GPU ticks"
    );
    assert!(state
        .particles
        .iter()
        .all(|q| q.position_age.iter().all(|v| v.is_finite())));
    let stats = simulation.stats();
    drop(state);
    drop(initial);
    drop(simulation);
    let mut burst_samples = vec![];
    for trial in 0..24 {
        let mut burst = GpuSimulation::new(d, 100_000).unwrap();
        let mut input = step(1, 100_000, 0);
        input.dt = 0.;
        input.time = 0.;
        burst_samples.push(measure(&mut burst, input));
        if trial == 23 {
            let born = burst.readback_offline(d, &gpu.queue).unwrap();
            assert_eq!(born.draw_args[1], 100_000);
            assert_eq!(born.dropped_births, 0);
            assert!(born
                .particles
                .iter()
                .all(|q| q.active() && q.position_age[3] == 0.));
        }
    }
    burst_samples.sort_by(f64::total_cmp);
    samples.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let report = serde_json::json!({"adapter":format!("{:?}",gpu.adapter.get_info()),"particles":100_000,"samples":samples.len(),"scope":"GPU simulation commands only: clear/spawn(empty)/parallel update/compact; no draw, CPU encode, upload or readback timing","modules":["acceleration","exponential_drag","curl_noise"],"cold_spawn_and_readback_wall_ms":cold_ms,"mean_us":mean,"p50_us":samples[12],"p95_us":samples[22],"timestamps_us":samples,"stats":stats,
        "burst":{"samples":burst_samples.len(),"mean_us":burst_samples.iter().sum::<f64>()/24.,"p50_us":burst_samples[12],"p95_us":burst_samples[22],"timestamps_us":burst_samples,"scope":"100k births: parallel free-slot scan, block prefix, initialization and compact at dt=0; no draw/CPU encode/readback; buffers allocated outside interval"}});
    let out =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/verification/vfx-gpu");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("benchmark.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("{report}");
}

#[test]
#[ignore = "requires real compute GPU; lowest-free allocation across partial workgroups"]
fn prefix_allocator_reuses_fragmented_slots_with_original_ids() {
    let gpu = device();
    let d = &gpu.device;
    let mut e = EmitterSpec::default();
    e.target = SimulationTarget::Gpu;
    e.capacity = 257;
    e.seed = 77;
    e.initialize.lifetime = [0.1, 0.9];
    e.initialize.speed = [0.; 2];
    let plan = e.compile(&Default::default()).unwrap();
    let mut simulation = GpuSimulation::new(d, 257).unwrap();
    let mut slots: Vec<Option<crysirror_vfx::Particle>> = vec![None; 257];
    for (tick, count, first) in [(1, 257, 0), (2, 60, 257)] {
        let mut s = step(tick, count, first);
        s.dt = 0.5;
        s.time = tick as f32 * 0.5;
        let mut id = first;
        for slot in &mut slots {
            if id >= first + u64::from(count) {
                break;
            }
            if slot.is_none() {
                *slot = Some(crysirror_vfx::spawn_particle(&plan, id, s.transform).unwrap());
                id += 1;
            }
        }
        for slot in &mut slots {
            if let Some(q) = slot {
                if !crysirror_vfx::update_particle(q, &plan, s.transform, s.dt, s.time)
                    .unwrap()
                    .0
                {
                    *slot = None;
                }
            }
        }
        let mut encoder = d.create_command_encoder(&Default::default());
        let token = simulation
            .encode_batch(d, &mut encoder, &plan, &[s])
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        simulation.mark_submitted(token).unwrap();
        let r = simulation.readback_offline(d, &gpu.queue).unwrap();
        assert_eq!(
            r.draw_args[1] as usize,
            slots.iter().filter(|s| s.is_some()).count()
        );
        for (actual, expected) in r.particles.iter().zip(&slots) {
            assert_eq!(actual.active(), expected.is_some());
            if let Some(expected) = expected {
                assert_eq!(actual.id(), expected.id);
            }
        }
    }
}
