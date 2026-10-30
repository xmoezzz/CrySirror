// SPDX-License-Identifier: MPL-2.0
//! Public runtime VFX ownership, worker completion and device-free timelines.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::{ServiceBatch, WorldCommand};
use crysirror_vfx::*;
use serde_json::json;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

fn system(target: SimulationTarget) -> SystemSpec {
    let mut emitter = EmitterSpec::default();
    emitter.capacity = 128;
    emitter.target = target;
    emitter.spawn.rate = ScalarValue::Parameter(ScalarBinding {
        parameter: "rate".into(),
        scale: 1.,
        offset: 0.,
    });
    emitter.spawn.bursts = vec![Burst { time: 0., count: 3 }];
    emitter.spawn.burst_looping = false;
    emitter.initialize.lifetime = [10.; 2];
    emitter.initialize.speed = [0.; 2];
    SystemSpec {
        emitters: vec![emitter],
        parameters: [("rate".into(), ParameterValue::Float(0.))]
            .into_iter()
            .collect(),
        fixed_step_hz: 10,
        ..Default::default()
    }
}
fn runtime(spec: SystemSpec, clients: usize) -> (tempfile::TempDir, Runtime) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let clients=(0..clients).map(|i|json!({"name":format!("client-{i}"),"adapter":"crysirror-script-v1","game":format!("vfx-{i}"),"systems":{"entities":[{"id":1,"name":"effect","geometry":{"kind":"cube"},"vfx":spec}]}})).collect::<Vec<_>>();
    let file = tmp.path().join("project.json");
    std::fs::write(&file,serde_json::to_vec(&json!({"schema":2,"name":"vfx-runtime","content_revision":"1","canvas":[160,90],"assets":"assets","clients":clients})).unwrap()).unwrap();
    let rt = Runtime::load(&file, &tmp.path().join("state")).unwrap();
    (tmp, rt)
}
fn submit(rt: &mut Runtime, client: u32, commands: Vec<WorldCommand>) -> anyhow::Result<()> {
    rt.host.engine.submit(
        ClientId(client),
        &ServiceBatch {
            world: commands,
            ..Default::default()
        },
    )
}
fn set_spec(rt: &mut Runtime, client: u32, spec: SystemSpec) {
    submit(
        rt,
        client,
        vec![WorldCommand::SetVfx {
            id: 1,
            vfx: Some(spec),
        }],
    )
    .unwrap();
}
fn screen_system() -> SystemSpec {
    let mut spec = system(SimulationTarget::Cpu);
    let emitter = &mut spec.emitters[0];
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
    emitter.screen = Some(crysirror_vfx::screen::ScreenSpriteTarget {
        canvas: [160, 90],
        layer: 61,
        parent: None,
        order: [2, 0],
        anchor: [0.5, 0.5],
    });
    spec
}
#[test]
fn screen_worker_publication_is_scoped_idempotent_hidden_and_cleaned() {
    let (_tmp, mut rt) = runtime(screen_system(), 2);
    rt.finish_vfx_offline().unwrap();
    let old = rt
        .host
        .engine
        .layers
        .dynamic_puppet_frame(1, 61)
        .unwrap()
        .clone();
    assert_eq!(old.parts.len(), 3);
    assert_eq!(old.textures, ["authored.png"]);
    assert!(!Arc::ptr_eq(
        &old,
        rt.host.engine.layers.dynamic_puppet_frame(2, 61).unwrap()
    ));
    rt.vfx.publish_screen(&mut rt.host.engine.layers).unwrap();
    assert!(Arc::ptr_eq(
        &old,
        rt.host.engine.layers.dynamic_puppet_frame(1, 61).unwrap()
    ));
    submit(
        &mut rt,
        1,
        vec![WorldCommand::SetVisible {
            id: 1,
            visible: false,
        }],
    )
    .unwrap();
    rt.finish_vfx_offline().unwrap();
    assert!(!rt.host.engine.layers.get(ClientId(1), 61).unwrap().visible);
    assert!(rt.host.engine.layers.get(ClientId(2), 61).unwrap().visible);
    submit(&mut rt, 1, vec![WorldCommand::SetVfx { id: 1, vfx: None }]).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert!(rt.host.engine.layers.get(ClientId(1), 61).is_none());
    assert!(rt.host.engine.layers.get(ClientId(2), 61).is_some());
}
#[test]
fn screen_publication_never_overwrites_an_existing_or_replaced_layer() {
    use crysirror_scene::layer::{LayerCommand, LayerSpec};
    let (_tmp, mut rt) = runtime(screen_system(), 1);
    let existing: LayerSpec = serde_json::from_value(
        json!({"id":61,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[9,8,7,255]}}),
    )
    .unwrap();
    rt.host
        .engine
        .layers
        .apply_batch(ClientId(1), &[LayerCommand::Put { layer: existing }])
        .unwrap();
    assert!(rt.finish_vfx_offline().is_err());
    assert_eq!(
        serde_json::to_value(&rt.host.engine.layers.get(ClientId(1), 61).unwrap().paint).unwrap()
            ["color"],
        json!([9, 8, 7, 255])
    );
    let (_tmp, mut rt) = runtime(screen_system(), 1);
    rt.finish_vfx_offline().unwrap();
    rt.host
        .engine
        .layers
        .apply_batch(
            ClientId(1),
            &[LayerCommand::SetOpacity { id: 61, opacity: 7 }],
        )
        .unwrap();
    assert!(rt.vfx.publish_screen(&mut rt.host.engine.layers).is_err());
    assert_eq!(
        rt.host.engine.layers.get(ClientId(1), 61).unwrap().opacity,
        7
    );
}
#[test]
fn screen_profile_rejects_gpu_and_duplicate_layer_declarations() {
    let mut spec = screen_system();
    spec.emitters[0].target = SimulationTarget::Gpu;
    assert!(spec.compile().is_err());
    let mut spec = screen_system();
    let mut second = spec.emitters[0].clone();
    second.id = "second".into();
    spec.emitters.push(second);
    assert!(spec.compile().is_err());
}
#[test]
fn screen_snapshot_gate_covers_unsampled_live_and_fully_cleaned_producers() {
    let (_tmp, mut rt) = runtime(screen_system(), 1);
    assert!(rt.vfx.has_screen_state(&rt.host.engine.world));
    assert!(rt.capture_snapshot().is_err()); //Declaration is not sampled yet.
    rt.finish_vfx_offline().unwrap();
    assert!(rt.capture_snapshot().is_err());
    submit(&mut rt, 1, vec![WorldCommand::SetVfx { id: 1, vfx: None }]).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert!(!rt.vfx.has_screen_state(&rt.host.engine.world));
    assert!(rt.capture_snapshot().is_ok());
}
#[test]
fn screen_serde_roundtrip_and_reset_cannot_reclaim_a_shape_as_a_producer() {
    let (_tmp, mut rt) = runtime(screen_system(), 1);
    rt.finish_vfx_offline().unwrap();
    let bytes = serde_json::to_vec(&rt.host.engine.layers).unwrap();
    let mut restored: crysirror_scene::layer::LayerSystem = serde_json::from_slice(&bytes).unwrap();
    assert!(restored.dynamic_puppet_frame(1, 61).is_none());
    rt.vfx.reset(rt.now_ms());
    assert!(rt.vfx.publish_screen(&mut restored).is_err());
    assert!(restored.get(ClientId(1), 61).is_some());
    // A new runtime has no producer lease either, even for an identical
    // transparent Solid declaration restored from otherwise valid JSON.
    let mut fresh = crysirror_runtime::vfx::VfxRuntime::default();
    let pool = crysirror_tasks::TaskPool::new("screen-restore-test", 1, 8).unwrap();
    fresh
        .finish_offline(&rt.host.engine.world, rt.now_ms(), &pool)
        .unwrap();
    assert!(fresh.publish_screen(&mut restored).is_err());
    assert!(restored.get(ClientId(1), 61).is_some());
}
#[test]
fn completed_sequence_receipts_normalize_default_budget_and_client_pause() {
    use crysirror_vfx::falling_atlas::{SequenceRng, SimulationClock};
    let mut spec = system(SimulationTarget::Cpu);
    spec.clock = SimulationClock::External {
        tick: 0,
        wall_time_ms: 0,
    };
    spec.sequence_rng = Some(SequenceRng::msvc(1));
    assert!(spec.max_steps_per_advance > 32);
    let (_tmp, mut rt) = runtime(spec.clone(), 2);
    assert!(rt
        .vfx
        .sequence_receipts(&rt.host.engine.world)
        .unwrap()
        .is_empty());
    rt.finish_vfx_offline().unwrap();
    let receipts = rt.vfx.sequence_receipts(&rt.host.engine.world).unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[&(1, 1)]["completed_tick"], 0);
    let expected = blake3::hash(&serde_json::to_vec(&spec).unwrap())
        .to_hex()
        .to_string();
    assert_eq!(receipts[&(1, 1)]["source_fingerprint"], expected);
    submit(&mut rt, 1, vec![WorldCommand::Pause { paused: true }]).unwrap();
    rt.finish_vfx_offline().unwrap();
    let paused = rt.vfx.sequence_receipts(&rt.host.engine.world).unwrap();
    assert_eq!(
        paused[&(1, 1)]["sequence_rng"],
        receipts[&(1, 1)]["sequence_rng"]
    );
    submit(&mut rt, 1, vec![WorldCommand::Pause { paused: false }]).unwrap();
    spec.clock = SimulationClock::External {
        tick: 1,
        wall_time_ms: 100,
    };
    set_spec(&mut rt, 1, spec.clone());
    assert!(!rt
        .vfx
        .sequence_receipts(&rt.host.engine.world)
        .unwrap()
        .contains_key(&(1, 1))); //New declaration is not its own receipt.
    rt.finish_vfx_offline().unwrap();
    assert_eq!(
        rt.vfx.sequence_receipts(&rt.host.engine.world).unwrap()[&(1, 1)]["completed_tick"],
        1
    );
}

#[test]
fn zero_time_initialization_and_offline_fences_do_not_tick_world_or_vm() {
    let mut spec = system(SimulationTarget::Cpu);
    spec.warmup_seconds = 0.5;
    spec.max_steps_per_advance = 2;
    let (_tmp, mut rt) = runtime(spec, 1);
    let time = rt.now_ms();
    let trace = rt.host.trace.len();
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.now_ms(), time);
    assert_eq!(rt.host.trace.len(), trace);
    let frame = rt.vfx.frame();
    assert_eq!(frame.emitters[0].particles.len(), 3);
    assert!((frame.emitters[0].particles[0].age - 0.5).abs() < 0.00001);
    assert_eq!(rt.vfx.inspect(None)["pending"], false);
    let epoch = rt.vfx.epoch();
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.epoch(), epoch);
    assert_eq!(rt.now_ms(), time);
}

#[test]
fn clients_with_equal_local_ids_get_separate_particles_and_events() {
    let (_tmp, mut rt) = runtime(system(SimulationTarget::Cpu), 2);
    rt.finish_vfx_offline().unwrap();
    let frame = rt.vfx.frame();
    assert_eq!(frame.emitters.len(), 2);
    assert_ne!(frame.emitters[0].key, frame.emitters[1].key);
    assert_ne!(frame.emitters[0].system_key, frame.emitters[1].system_key);
    assert_eq!(rt.vfx.inspect(Some(1))["cpu_live_particles"], 3);
    assert_eq!(rt.vfx.inspect(Some(2))["cpu_live_particles"], 3);
    let first = rt.vfx.drain_events(1);
    assert_eq!(first.len(), 3);
    assert!(first.iter().all(|e| e.client == 1));
    let second = rt.vfx.drain_events(2);
    assert_eq!(second.len(), 3);
    assert!(second.iter().all(|e| e.client == 2));
    assert!(rt.vfx.drain_events(1).is_empty());
}

#[test]
fn same_clock_parameter_stop_pause_and_restart_edits_are_published() {
    let mut spec = system(SimulationTarget::Cpu);
    let (_tmp, mut rt) = runtime(spec.clone(), 1);
    rt.finish_vfx_offline().unwrap();
    let first = rt.vfx.frame();
    let old_key = first.emitters[0].key;
    assert_eq!(first.emitters[0].particles.len(), 3);
    spec.parameters
        .insert("rate".into(), ParameterValue::Float(20.));
    set_spec(&mut rt, 1, spec.clone());
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.now_ms(), 0);
    assert_eq!(rt.vfx.frame().emitters[0].key, old_key);
    rt.tick(100, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    let running = rt.vfx.frame();
    assert_eq!(running.emitters[0].particles.len(), 5);
    assert_eq!(
        first.emitters[0].particles.len(),
        3,
        "older snapshot stays immutable"
    );
    spec.enabled = false;
    set_spec(&mut rt, 1, spec.clone());
    rt.finish_vfx_offline().unwrap();
    rt.tick(100, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.frame().emitters[0].particles.len(), 5);
    spec.paused = true;
    set_spec(&mut rt, 1, spec.clone());
    rt.finish_vfx_offline().unwrap();
    let frozen = rt.vfx.frame();
    rt.tick(500, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert_eq!(
        frozen.emitters[0].particles,
        rt.vfx.frame().emitters[0].particles
    );
    spec.paused = false;
    spec.enabled = true;
    spec.revision += 1;
    set_spec(&mut rt, 1, spec);
    rt.finish_vfx_offline().unwrap();
    let restarted = rt.vfx.frame();
    assert_ne!(restarted.emitters[0].key, old_key);
    assert_eq!(restarted.emitters[0].particles.len(), 3);
    assert!(restarted.emitters[0].particles.iter().all(|p| p.age == 0.));
}

#[test]
fn invalid_parameter_update_is_transactional_at_the_public_service_boundary() {
    let mut spec = system(SimulationTarget::Cpu);
    let (_tmp, mut rt) = runtime(spec.clone(), 1);
    rt.finish_vfx_offline().unwrap();
    spec.parameters
        .insert("rate".into(), ParameterValue::Vector([1.; 3]));
    assert!(submit(
        &mut rt,
        1,
        vec![WorldCommand::SetVfx {
            id: 1,
            vfx: Some(spec)
        }]
    )
    .is_err());
    let handle = rt.host.engine.world.handle(ClientId(1), 1).unwrap();
    assert_eq!(
        rt.host
            .engine
            .world
            .entity(handle)
            .unwrap()
            .vfx
            .as_ref()
            .unwrap()
            .parameters["rate"],
        ParameterValue::Float(0.)
    );
}

#[test]
fn gpu_timelines_keep_more_than_thirty_two_unsubmitted_ticks_and_respect_ack() {
    let (_tmp, mut rt) = runtime(system(SimulationTarget::Gpu), 1);
    rt.finish_vfx_offline().unwrap();
    rt.tick(4000, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    let frame = rt.vfx.frame();
    let gpu = &frame.gpu_emitters[0];
    assert_eq!(gpu.steps.len(), 41);
    assert_eq!(gpu.steps[0].tick, 1);
    assert_eq!(gpu.steps.last().unwrap().tick, 41);
    assert!(gpu.steps.windows(2).all(|w| w[1].tick == w[0].tick + 1));
    assert_eq!(frame.stats.gpu_history_dropped, 0);
    assert_eq!(rt.vfx.inspect(Some(1))["gpu_pending_packets"], 41);
    gpu.progress.mark_submitted(gpu.tick);
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.inspect(Some(1))["gpu_pending_packets"], 0);
    assert_eq!(rt.now_ms(), 4000);
}

#[test]
fn hidden_gpu_effects_keep_advancing_and_can_still_ack_commands() {
    let (_tmp, mut rt) = runtime(system(SimulationTarget::Gpu), 1);
    rt.finish_vfx_offline().unwrap();
    submit(
        &mut rt,
        1,
        vec![WorldCommand::SetVisible {
            id: 1,
            visible: false,
        }],
    )
    .unwrap();
    rt.finish_vfx_offline().unwrap();
    rt.tick(300, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    let frame = rt.vfx.frame();
    assert_eq!(frame.gpu_emitters.len(), 1);
    let gpu = &frame.gpu_emitters[0];
    assert_eq!(gpu.tick, 4);
    assert!(!rt.vfx.visible_systems().contains(&gpu.system_key));
    gpu.progress.mark_submitted(gpu.tick);
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.inspect(None)["gpu_pending_packets"], 0);
    submit(
        &mut rt,
        1,
        vec![WorldCommand::SetVisible {
            id: 1,
            visible: true,
        }],
    )
    .unwrap();
    rt.finish_vfx_offline().unwrap();
    assert!(rt.vfx.visible_systems().contains(&gpu.system_key));
}

#[test]
fn restore_replaces_visual_lifetimes_and_does_not_accept_old_worker_results() {
    let (_tmp, mut rt) = runtime(system(SimulationTarget::Cpu), 1);
    rt.finish_vfx_offline().unwrap();
    let old = rt.vfx.frame();
    let epoch = rt.vfx.epoch();
    let saved = rt.snapshot().unwrap();
    rt.tick(100, InputFrame::default()).unwrap();
    rt.restore(&saved).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert_ne!(rt.vfx.epoch(), epoch);
    assert_ne!(
        rt.vfx.frame().emitters[0].system_key,
        old.emitters[0].system_key
    );
    assert!(rt.vfx.frame().emitters[0]
        .particles
        .iter()
        .all(|p| p.age == 0.));
    assert_eq!(rt.now_ms(), 0);
}

struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}

#[test]
fn a_queued_vfx_worker_does_not_join_or_freeze_the_owner_tick() {
    let (_tmp, mut rt) = runtime(system(SimulationTarget::Cpu), 1);
    let pool = rt.assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let (started, rx) = std::sync::mpsc::channel();
    let jobs = (0..pool.stats().workers)
        .map(|_| {
            let gate = gate.clone();
            let started = started.clone();
            pool.spawn(move |_| {
                started.send(()).unwrap();
                let mut ready = gate.0.lock().unwrap();
                while !*ready {
                    ready = gate.1.wait(ready).unwrap();
                }
                Ok(())
            })
            .unwrap()
        })
        .collect::<Vec<_>>();
    for _ in &jobs {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let (returned, done) = std::sync::mpsc::channel();
    let watchdog_gate = gate.clone();
    let watchdog = std::thread::spawn(move || {
        let nonblocking = done.recv_timeout(Duration::from_secs(5)).is_ok();
        *watchdog_gate.0.lock().unwrap() = true;
        watchdog_gate.1.notify_all();
        nonblocking
    });
    rt.tick(0, InputFrame::default()).unwrap();
    rt.tick(100, InputFrame::default()).unwrap();
    assert_eq!(rt.now_ms(), 100);
    assert_eq!(rt.vfx.inspect(None)["pending"], true);
    returned.send(()).unwrap();
    assert!(
        watchdog.join().unwrap(),
        "owner waited for the gated VFX worker"
    );
    drop(release);
    for job in jobs {
        job.wait().unwrap();
    }
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.frame().emitters[0].particles.len(), 3);
}

#[test]
fn queued_transform_and_parameter_boundaries_match_per_frame_offline_simulation() {
    let mut spec = system(SimulationTarget::Cpu);
    spec.max_steps_per_advance = 1;
    let (_baseline_tmp, mut baseline) = runtime(spec.clone(), 1);
    let (_queued_tmp, mut queued) = runtime(spec.clone(), 1);
    baseline.finish_vfx_offline().unwrap();
    queued.finish_vfx_offline().unwrap();

    let pool = queued.assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let (started, rx) = std::sync::mpsc::channel();
    let jobs = (0..pool.stats().workers)
        .map(|_| {
            let gate = gate.clone();
            let started = started.clone();
            pool.spawn(move |_| {
                started.send(()).unwrap();
                let mut ready = gate.0.lock().unwrap();
                while !*ready {
                    ready = gate.1.wait(ready).unwrap();
                }
                Ok(())
            })
            .unwrap()
        })
        .collect::<Vec<_>>();
    for _ in &jobs {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    for (x, rate) in [(1., 20.), (2., 30.), (3., 40.)] {
        spec.parameters
            .insert("rate".into(), ParameterValue::Float(rate));
        for rt in [&mut baseline, &mut queued] {
            submit(
                rt,
                1,
                vec![
                    WorldCommand::SetVfx {
                        id: 1,
                        vfx: Some(spec.clone()),
                    },
                    WorldCommand::SetTransform {
                        id: 1,
                        transform: crysirror_scene::world::Transform {
                            translation: [x, 0., 0.],
                            ..Default::default()
                        },
                    },
                ],
            )
            .unwrap();
            rt.tick(250, InputFrame::default()).unwrap();
        }
        baseline.finish_vfx_offline().unwrap();
    }
    assert_eq!(queued.now_ms(), baseline.now_ms());
    assert!(
        queued.vfx.inspect(None)["pending_input_samples"]
            .as_u64()
            .unwrap()
            >= 2
    );
    drop(release);
    for job in jobs {
        job.wait().unwrap();
    }
    queued.finish_vfx_offline().unwrap();
    let expected = baseline.vfx.frame();
    let actual = queued.vfx.frame();
    assert_eq!(actual.emitters[0].particles, expected.emitters[0].particles);
    assert!(actual.emitters[0]
        .particles
        .iter()
        .any(|p| p.position[0] == 1.));
    assert!(actual.emitters[0]
        .particles
        .iter()
        .any(|p| p.position[0] == 2.));
    assert!(actual.emitters[0]
        .particles
        .iter()
        .any(|p| p.position[0] == 3.));
    assert_eq!(queued.vfx.inspect(None)["pending_input_samples"], 0);
}

#[test]
fn a_full_input_history_fails_visibly_instead_of_dropping_temporal_controls() {
    let spec = system(SimulationTarget::Cpu);
    let (_tmp, mut rt) = runtime(spec.clone(), 1);
    rt.finish_vfx_offline().unwrap();
    let pool = rt.assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let (started, rx) = std::sync::mpsc::channel();
    let jobs = (0..pool.stats().workers)
        .map(|_| {
            let gate = gate.clone();
            let started = started.clone();
            pool.spawn(move |_| {
                started.send(()).unwrap();
                let mut ready = gate.0.lock().unwrap();
                while !*ready {
                    ready = gate.1.wait(ready).unwrap();
                }
                Ok(())
            })
            .unwrap()
        })
        .collect::<Vec<_>>();
    for _ in &jobs {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let mut full = None;
    for x in 1..4096 {
        submit(
            &mut rt,
            1,
            vec![WorldCommand::SetTransform {
                id: 1,
                transform: crysirror_scene::world::Transform {
                    translation: [x as f32, 0., 0.],
                    ..Default::default()
                },
            }],
        )
        .unwrap();
        if let Err(error) = rt.tick(1, InputFrame::default()) {
            full = Some(error);
            break;
        }
    }
    let error = full.expect("changed inputs must have a bounded owner queue");
    assert!(format!("{error:#}").contains("VFX owner input history is full"));
    assert!(
        rt.vfx.inspect(None)["pending_input_samples"]
            .as_u64()
            .unwrap()
            <= 2048
    );
    drop(release);
    for job in jobs {
        job.wait().unwrap();
    }
    // A fence drains accepted history, then admits the current owner sample
    // that overflowed, preserving its temporal boundary without resetting.
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.inspect(None)["pending_input_samples"], 0);
    assert_eq!(rt.vfx.inspect(None)["sampled_at_ms"], rt.now_ms());
}
