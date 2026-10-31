use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use crysirror_runtime::Runtime;
use crysirror_system_nnn::{
    bridge::{Bridge, CallbackProfile, SceneBindings},
    machine::{Machine, Step},
    Program, XorProfile,
};
use serde_json::{json, Value};
fn program(records: Vec<Vec<u32>>) -> Program {
    let mut words = vec![0u32; 32];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    for record in records {
        words.extend([
            3 + record.len() as u32 - 1,
            if record[0] >> 16 == 0x2222 {
                0x66660006
            } else {
                0x66660007
            },
        ]);
        words.extend(record);
    }
    words[18] = words.len() as u32;
    Program::parse(
        &words
            .into_iter()
            .flat_map(|w| (!w).to_le_bytes())
            .collect::<Vec<_>>(),
        XorProfile::default(),
    )
    .unwrap()
}
fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"NNN shared bridge","content_revision":"1","assets":"assets","canvas":[16,16],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]})).unwrap()).unwrap();
    let rt = Runtime::load(&path, &dir.path().join("state")).unwrap();
    (dir, rt)
}
fn bindings() -> SceneBindings {
    SceneBindings {
        root: 1,
        overlap_canvas: 7,
        effect_base: 100,
        size: [16, 16],
        layers: 16,
        music_channel: 0,
    }
}
fn root(rt: &mut Runtime) {
    rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[13,14,15,255]}}}]})).unwrap()).unwrap();
}
fn canvas(rt: &mut Runtime) -> Value {
    serde_json::from_slice(
        &rt.host
            .query_engine(ClientId(1), EngineQuery::Canvas(7))
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn shared_scene_snapshots_fill_and_clear_are_real_and_unknown_receipts_stay_pending() {
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    let p = program(vec![
        vec![0x33330052, 1, 0],
        vec![0x33330012, 0],
        vec![0x33330011, 0],
        vec![0x33330013, 6, 0, 54, 20, 42, 70, 130],
        vec![0x33330011, 0],
        vec![0x33330012, 0],
        vec![0x33330013, 2, 12, 29],
    ]);
    let mut m = Machine::default();
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let mut captured = None;
    for _ in 0..20 {
        let Step::Native(call) = m.poll(&p, 100).unwrap() else {
            panic!("native call")
        };
        if call.code == 0x33330013
            && call.payload
                == (crysirror_system_nnn::machine::Payload::Words {
                    count: 2,
                    words: vec![12, 29],
                })
        {
            assert!(bridge.commit(&mut m, &call, &mut rt.host).is_err());
            assert_eq!(m.poll(&p, 10).unwrap(), Step::Native(call));
            break;
        }
        let mut stale = call.clone();
        stale.serial += 1;
        let before = rt.inspect();
        assert!(bridge.commit(&mut m, &stale, &mut rt.host).is_err());
        assert_eq!(rt.inspect(), before);
        bridge.commit(&mut m, &call, &mut rt.host).unwrap();
        if bridge.overlap_captures == 2 && captured.is_none() {
            let state = canvas(&mut rt);
            assert!(state.to_string().contains("snapshot"));
            assert!(state.to_string().contains("42,70,130,255"));
            captured = Some(state);
        }
    }
    assert_eq!(bridge.overlap_captures, 2);
    assert_eq!(bridge.previous_effect(0), Some(54));
    assert!(rt.host.engine.layers.get(ClientId(1), 100).is_none());
    assert_eq!(
        canvas(&mut rt),
        captured.unwrap(),
        "frozen engine snapshot survives live layer removal"
    );
}
#[test]
fn failed_shared_root_snapshot_does_not_create_canvas_or_acknowledge_native_call() {
    let (_dir, mut rt) = runtime();
    let p = program(vec![vec![0x33330011, 0]]);
    let mut m = Machine::default();
    let Step::Native(call) = m.poll(&p, 100).unwrap() else {
        panic!()
    };
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    assert!(bridge.commit(&mut m, &call, &mut rt.host).is_err());
    assert_eq!(bridge.api_calls, 0);
    assert_eq!(bridge.overlap_captures, 0);
    assert_eq!(m.poll(&p, 1).unwrap(), Step::Native(call));
    assert!(canvas(&mut rt).is_null());
}

#[test]
fn native_suna_initialization_is_a_shared_worker_transaction_on_the_original_rng() {
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::time::{Duration, Instant};
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    let p = program(vec![vec![
        0x33330013, 12, 12, 29, 20, 1, 30, 30, 0, 0, 0, 0, 16, 16,
    ]]);
    let mut vars = Variables::fresh(VariableProfile::Legacy1000);
    vars.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(vars);
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let original_rng = machine.variables().unwrap().random_state().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while !bridge
        .try_commit(&mut machine, &call, &mut rt.host)
        .unwrap()
    {
        assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call.clone()));
        assert_eq!(
            machine.variables().unwrap().random_state().unwrap(),
            original_rng
        );
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    let resource = &bridge.pattern(12).unwrap().resource;
    assert_eq!(resource.client(), ClientId(1));
    assert_eq!(resource.info().size, [16, 16]);
    assert_eq!(resource.info().completed_steps, 0);
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        32
    );
    assert_eq!(machine.pc(), call.next);
    assert_eq!(bridge.previous_effect(12), Some(29));
    assert!(
        rt.host.engine.layers.get(ClientId(1), 112).is_none(),
        "SetParam initializes state; native Print owns presentation timing"
    );
    assert!(
        bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .is_err(),
        "receipt cannot be replayed"
    );
}

#[test]
fn native_audio_lookup_and_shared_decoder_commit_only_after_ready() {
    use crysirror_core::path::{Mount, VfsPath};
    use crysirror_system_nnn::{
        audio_bridge::AudioBridge,
        config::Names,
        native_audio::{AudioBindings, AudioSettings, NativeAudioPolicy},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (dir, mut rt) = runtime();
    // Authored mono16/8000Hz silence; no owner audio or device dependency.
    let mut wave = b"RIFF".to_vec();
    wave.extend(52u32.to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(8000u32.to_le_bytes());
    wave.extend(16000u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend(16u32.to_le_bytes());
    wave.extend([0u8; 16]);
    std::fs::write(dir.path().join("assets/cue.wav"), wave).unwrap();
    let policy = NativeAudioPolicy::new(
        AudioSettings {
            music_volume: 100,
            sound_volume: 100,
            voice_volume: 100,
            sound_voice_volume: 100,
            music_enabled: true,
            sound_enabled: true,
            voice_enabled: true,
            sound_voice_enabled: true,
        },
        AudioBindings {
            music: 0,
            sound_base: 1,
            voice_base: 16,
        },
    )
    .unwrap();
    let audio = AudioBridge::new(
        ClientId(1),
        policy,
        Names { tokens: vec![] },
        Names {
            tokens: vec![b"cue".to_vec(), b"0".to_vec()],
        },
        Arc::new(|source, cancel| {
            cancel.check()?;
            assert_eq!(source.name, b"cue");
            Ok(crysirror_system_nnn::resource_bridge::ResolvedAudio::Asset(
                VfsPath::new(Mount::Asset, "cue.wav")?,
            ))
        }),
    )
    .unwrap();
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    bridge.configure_audio(audio).unwrap();
    let p = program(vec![vec![0x33330009, 2, 0, 0]]);
    let mut machine = Machine::default();
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    let until = Instant::now() + Duration::from_secs(10);
    while !bridge
        .try_commit(&mut machine, &call, &mut rt.host)
        .unwrap()
    {
        assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call.clone()));
        rt.tick(1, Default::default()).unwrap();
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(machine.pc(), call.next);
    assert_eq!(bridge.api_calls, 1);
    assert!(bridge
        .try_commit(&mut machine, &call, &mut rt.host)
        .is_err());
}

#[test]
fn overlap_wait_uses_engine_capture_pattern_mix_and_immutable_display_resources() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.set_canvas_image_presenter(true);
    let p = program(vec![
        vec![0x33330011, 0],
        vec![0x33330013, 6, 0, 54, 20, 42, 70, 130],
        vec![0x33330013, 12, 12, 29, 20, 1, 30, 30, 0, 0, 0, 0, 16, 16],
        vec![0x33330013, 2, 13, 10],
        vec![0x22220005, 2, 0, 2],
        vec![0x33330055, 1, 9],
    ]);
    let mut vars = Variables::fresh(VariableProfile::Legacy1000);
    vars.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(vars);
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let mut reached = false;
    while Instant::now() < until {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        if call.code == 0x33330055 {
            reached = true;
            assert!(bridge
                .try_commit(&mut machine, &call, &mut rt.host)
                .is_err());
            break;
        }
        let complete = bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap();
        if !complete {
            assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call.clone()));
        }
        // An authored presenter supplies a known capture; the independent
        // compositor GPU tests validate actual scene-to-pixel rasterization.
        rt.prepare_canvas_images(|request, _| {
            Ok(Some(Arc::new(MemoryAssetReader(
                [32, 64, 128, 255]
                    .repeat((request.size[0] * request.size[1]) as usize)
                    .into(),
            )) as Arc<dyn AssetReader>))
        })
        .unwrap();
        rt.tick(10, Default::default()).unwrap();
        std::thread::yield_now();
    }
    assert!(reached, "overlap wait failed to complete");
    assert_eq!(
        bridge.pattern(12).unwrap().resource.info().completed_steps,
        1,
        "overlap freezes one effect render rather than consuming RNG every blend frame"
    );
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        48
    );
    let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
    assert_eq!(
        layer.parent, None,
        "display output is outside the source hierarchy"
    );
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    let resource = rt.assets.surface(ClientId(1), asset).unwrap();
    assert!(resource
        .rgba
        .chunks_exact(4)
        .all(|pixel| pixel == [0, 0, 0, 255]));
}

#[test]
fn original_load_picture_uses_shared_dwq_decoder_and_preserves_raw_alpha() {
    use crysirror_core::service::MemoryAssetReader;
    use crysirror_system_nnn::{native_resources::ResourceNames, resource_bridge::NativeResources};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for preload in [false, true] {
        let (_dir, mut rt) = runtime();
        root(&mut rt);
        if !rt.assets.decoders.format_ids().contains(&"black-cyc/dwq") {
            crysirror_formats_blackcyc::register_images(&mut rt.assets.decoders).unwrap();
        }
        rt.tick(0, Default::default()).unwrap();
        let mut dwq = vec![b' '; 64];
        dwq[..4].copy_from_slice(b"PNG ");
        dwq[36..40].copy_from_slice(&2u32.to_le_bytes());
        dwq[40..44].copy_from_slice(&1u32.to_le_bytes());
        dwq[48..64].copy_from_slice(b"PACKTYPE=8      ");
        dwq.extend(
            crysirror_image::encode_png(&crysirror_image::ImageSurface {
                width: 2,
                height: 1,
                rgba: vec![4, 8, 12, 31, 16, 20, 24, 255],
            })
            .unwrap(),
        );
        let source: Arc<[u8]> = dwq.into();
        let resources = NativeResources::blackcyc(
            ClientId(1),
            Arc::new(move |path, cancel| {
                cancel.check()?;
                assert_eq!(path, "dwq/bg/bgcue.dwq");
                Ok(Some(Arc::new(MemoryAssetReader(source.clone()))))
            }),
        )
        .unwrap();
        let mut bridge =
            Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
        bridge
            .configure_resources(resources, ResourceNames::new("").unwrap(), 0)
            .unwrap();
        let mut words = vec![0u32; 32];
        words[0] = 32;
        words[1] = 0x66660001;
        words[2] = 0x55550001;
        words[6] = 1;
        words[7] = 42;
        words[18] = 42;
        words.extend([
            5,
            0x66660007,
            if preload { 0x3333001f } else { 0x33330015 },
            0,
            0,
            5,
            0x66660007,
            0x33330055,
            1,
            9,
            43,
            u32::from_le_bytes(*b"bgcu"),
            u32::from_le_bytes(*b"e\0\0\0"),
        ]);
        let p = Program::parse(
            &words
                .into_iter()
                .flat_map(|v| (!v).to_le_bytes())
                .collect::<Vec<_>>(),
            XorProfile::default(),
        )
        .unwrap();
        let mut machine = Machine::default();
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        let until = Instant::now() + Duration::from_secs(10);
        while !bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap()
        {
            assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call.clone()));
            rt.tick(1, Default::default()).unwrap();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(machine.pc(), call.next);
        let picture = bridge.picture(0).unwrap();
        assert_eq!(picture.raw.rgba().as_ref(), [4, 8, 12, 31, 16, 20, 24, 255]);
        if preload {
            assert!(rt.host.engine.layers.get(ClientId(1), 100).is_none());
        } else {
            let layer = rt.host.engine.layers.get(ClientId(1), 100).unwrap();
            assert_eq!(layer.rect, [0., 0., 2., 1.]);
            let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
                panic!()
            };
            assert_eq!(
                rt.assets.surface(ClientId(1), asset).unwrap().rgba.as_ref(),
                [4, 8, 12, 255, 16, 20, 24, 255]
            );
        }
    }
}

#[test]
fn core_vm_port_runs_control_on_worker_and_only_owner_commits_shared_state() {
    use crysirror_core::vm::VmAdapter;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    let p = program(vec![
        vec![0x33330052, 1, 0],
        vec![0x33330013, 6, 0, 54, 20, 42, 70, 130],
        vec![0x33330011, 0],
        vec![0x33330012, 0],
        vec![0x33330013, 2, 12, 29],
    ]);
    let pc = p.word_count() as u32 - 6;
    let bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let mut vm = crysirror_system_nnn::adapter::NativeAdapter::from_prepared(
        Arc::new(p),
        Machine::default(),
        bridge,
    )
    .unwrap();
    assert!(!vm.dependency_pending());
    vm.step(&mut rt.host, Default::default(), 100).unwrap();
    assert!(vm.dependency_pending());
    let initial: Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    assert_eq!(initial["wait_kind"], "vm_worker");
    assert_eq!(initial["script"], "sys");
    assert_eq!(initial["script_depth"], 0);
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Err(e) = vm.step(&mut rt.host, Default::default(), 100) {
            assert!(e.to_string().contains(&format!("pc={pc:#x}")));
            break;
        }
        rt.tick(1, Default::default()).unwrap();
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    let d: Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    assert_eq!(d["vm"], "original-system-nnn-spt");
    assert_eq!(d["worker_pending"], false);
    assert_eq!(d["dependency_pending"], false);
    assert_eq!(d["engine_api_calls"], 3);
    assert_eq!(d["overlap_captures"], 1);
    assert!(rt.host.engine.layers.get(ClientId(1), 100).is_none());
    assert!(canvas(&mut rt).to_string().contains("42,70,130,255"));
    let before = rt.inspect();
    assert!(vm.step(&mut rt.host, Default::default(), 100).is_err());
    assert_eq!(rt.inspect(), before);
    assert!(vm.snapshot().is_err());
    assert!(vm.restore(&[]).is_err());
}

#[test]
fn native_draw_uses_shared_frames_and_counts_after_print_without_aging_draw_zero() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for frames in [0, 2] {
        let (_dir, mut rt) = runtime();
        root(&mut rt);
        rt.set_canvas_image_presenter(true);
        let p = program(vec![
            vec![0x33330013, 6, 0, 54, 20, 42, 70, 130],
            vec![0x33330013, 2, 1, 0],
            vec![0x33330013, 12, 12, 29, 20, 1, 30, 30, 0, 0, 0, 0, 16, 16],
            vec![0x22220004, 1, frames],
            vec![0x33330055, 1, 9],
        ]);
        let mut vars = Variables::fresh(VariableProfile::Legacy1000);
        vars.replace_random_state(PatternRng::msvc(1)).unwrap();
        let mut machine = Machine::with_variables(vars);
        let mut bridge =
            Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let mut reached = false;
        while Instant::now() < until {
            let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
                panic!()
            };
            if call.code == 0x33330055 {
                let pc = machine.pc();
                assert!(bridge
                    .try_commit(&mut machine, &call, &mut rt.host)
                    .is_err());
                assert_eq!(machine.pc(), pc);
                reached = true;
                break;
            }
            let complete = bridge
                .try_commit(&mut machine, &call, &mut rt.host)
                .unwrap();
            if !complete {
                assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call));
            }
            rt.prepare_canvas_images(|request, _| {
                Ok(Some(Arc::new(MemoryAssetReader(
                    [32, 64, 128, 255]
                        .repeat((request.size[0] * request.size[1]) as usize)
                        .into(),
                )) as Arc<dyn AssetReader>))
            })
            .unwrap();
            rt.tick(10, Default::default()).unwrap();
            std::thread::yield_now();
        }
        assert!(reached, "DRAW {frames} did not reach next native boundary");
        assert_eq!(
            bridge.pattern(12).unwrap().resource.info().completed_steps,
            frames.max(1) as u64
        );
        assert_eq!(bridge.effect_counter(0), Some((frames as i32, 9999)));
        assert_eq!(bridge.effect_counter(1), Some((frames as i32, 99999)));
        assert_eq!(bridge.pattern(12).unwrap().count, frames as i32);
        assert_eq!(
            machine.variables().unwrap().random_state().unwrap().draws,
            32 + frames.max(1) as u64 * 16
        );
        assert!(matches!(
            rt.host.engine.layers.get(ClientId(1), 116).unwrap().paint,
            crysirror_scene::layer::Paint::Image { .. }
        ));
    }
}

#[test]
fn original_function_table_selects_stopf5_and_unknown_function_keeps_receipt() {
    use crysirror_system_nnn::{
        config::Names,
        expression::{VariableProfile, Variables},
    };
    let (_dir, mut rt) = runtime();
    let before = serde_json::to_value(&rt.host.engine).unwrap();
    let mut words = vec![0u32; 32];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words.extend([4, 0x66660004, 0, 0, 4, 0x66660004, 1, 0]);
    words[18] = words.len() as u32;
    let p = Program::parse(
        &words
            .into_iter()
            .flat_map(|v| (!v).to_le_bytes())
            .collect::<Vec<_>>(),
        XorProfile::default(),
    )
    .unwrap();
    let mut machine = Machine::with_variables(Variables::fresh(VariableProfile::Legacy1000));
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    bridge
        .configure_functions(Names::parse(b"stopf5,Stop\nnotimplemented,Other", false).unwrap())
        .unwrap();
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    assert_eq!(
        call.code, 0,
        "lookup follows configured table rather than hard-coded native index2"
    );
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(bridge.skip_policy(), &Default::default());
    let Step::Native(unknown) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    let pc = machine.pc();
    let error = bridge
        .commit(&mut machine, &unknown, &mut rt.host)
        .unwrap_err();
    assert!(error.to_string().contains("name=notimplemented"));
    assert_eq!(machine.pc(), pc);
    assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(unknown));
    assert_eq!(serde_json::to_value(&rt.host.engine).unwrap(), before);
}

#[test]
fn native_start_message_and_frame_controls_preserve_ids_and_window_modes() {
    let (_dir, mut rt) = runtime();
    let p = program(vec![
        vec![0x3333001b, 2, 12, 2],
        vec![0x3333001b, 2, u32::MAX, 99],
        vec![0x3333001c, 3, 4, 3, 0],
        vec![0x3333001b, 1, u32::MAX],
    ]);
    let mut machine = Machine::default();
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    bridge.message_have_voice = true;
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(bridge.last_message_id, Some(12));
    assert_eq!(bridge.next_window_mode, 1);
    assert!(!bridge.message_have_voice);
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(bridge.last_message_id, Some(12));
    assert_eq!(bridge.next_window_mode, 1);
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(bridge.next_message_type, 3);
    assert_eq!(bridge.next_window_mode, 0);
    assert!(!bridge.message_window_visible);
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(bridge.last_message_id, Some(12));
    assert_eq!(bridge.next_window_mode, -1);
    assert_eq!(
        bridge.api_calls, 0,
        "native ID/window triggers do not fabricate graphics calls"
    );
}

#[test]
fn native_layergrey_uses_shared_grouped_luma_and_io_waits_do_not_advance_frame_clock() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.set_canvas_image_presenter(true);
    let p = program(vec![
        vec![0x33330013, 7, 11, 129, 55, 0, 0, 80, 80],
        vec![0x22220004, 1, 2],
        vec![0x33330055, 1, 9],
    ]);
    let mut variables = Variables::fresh(VariableProfile::Legacy1000);
    variables.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(variables);
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let mut io = false;
    let mut timer = false;
    let mut done = false;
    while Instant::now() < until {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        if call.code == 0x33330055 {
            done = true;
            break;
        }
        bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap();
        rt.prepare_canvas_images(|request, _| {
            Ok(Some(Arc::new(MemoryAssetReader(
                [255, 255, 255, 255]
                    .repeat((request.size[0] * request.size[1]) as usize)
                    .into(),
            )) as Arc<dyn AssetReader>))
        })
        .unwrap();
        if bridge.dependency_pending() {
            io = true;
            assert_eq!(bridge.wait_kind(Some(&call)), "resource");
            rt.tick(0, Default::default()).unwrap();
        } else {
            if bridge.wait_kind(Some(&call)) == "timer" {
                timer = true;
            }
            rt.tick(10, Default::default()).unwrap();
        }
        std::thread::yield_now();
    }
    assert!(
        done && io && timer,
        "native wait kind must distinguish real task completion from frame time"
    );
    assert_eq!(bridge.effect_counter(11), Some((2, 55)));
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        0,
        "grey does not consume the global random stream"
    );
    let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    assert!(rt
        .assets
        .surface(ClientId(1), asset)
        .unwrap()
        .rgba
        .chunks_exact(4)
        .all(|p| p == [253, 253, 253, 255]));
}

#[test]
fn native_ctrans_uses_shared_unkeyed_and_masked_q8_composites_and_keeps_source_bytes() {
    use crysirror_core::image_processing::{ImageInput, ImageProcessState, ImageResource};
    use crysirror_system_nnn::{
        native_chara_trans::CharaTransState,
        native_geometry::NativeRects,
        native_presentation::{CharaTransFrame, NativeFrameEffect, NativeFrameWork},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    fn image(
        rt: &mut Runtime,
        input: ImageInput,
        effects: Vec<crysirror_image::effects::ImageEffect>,
    ) -> Arc<dyn ImageResource> {
        let mut job = rt
            .host
            .image_metadata(ClientId(1))
            .unwrap()
            .begin_process(input, serde_json::to_vec(&effects).unwrap().into())
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match job.poll().unwrap() {
                ImageProcessState::Ready(r) => return r,
                ImageProcessState::Failed(e) => panic!("{e}"),
                ImageProcessState::Pending => {}
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
    }
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.tick(0, Default::default()).unwrap();
    let bytes = crysirror_image::encode_png(&crysirror_image::ImageSurface {
        width: 3,
        height: 1,
        rgba: vec![200, 100, 50, 0, 200, 100, 50, 128, 200, 100, 50, 255],
    })
    .unwrap();
    let raw = image(
        &mut rt,
        ImageInput::Encoded {
            name: "authored.png".into(),
            size: [3, 1],
            bytes: bytes.into(),
        },
        vec![],
    );
    let opaque = image(
        &mut rt,
        ImageInput::Resource(raw.clone()),
        vec![crysirror_image::effects::ImageEffect::OverrideAlpha { alpha: 255 }],
    );
    let base = image(
        &mut rt,
        ImageInput::Solid {
            size: [3, 1],
            color: [100, 100, 100, 255],
        },
        vec![],
    );
    for (masked, expected) in [
        (
            true,
            vec![100, 100, 100, 255, 124, 99, 87, 255, 150, 100, 75, 255],
        ),
        (
            false,
            vec![150, 100, 75, 255, 150, 100, 75, 255, 150, 100, 75, 255],
        ),
    ] {
        let state = CharaTransState::parse(&[20, 50, 50], [3, 1]).unwrap();
        let frame = CharaTransFrame {
            state,
            source: Some(if masked { raw.clone() } else { opaque.clone() }),
            rectangles: NativeRects::natural([3, 1]),
        };
        let mut work = NativeFrameWork::with_base(
            ClientId(1),
            base.clone(),
            vec![NativeFrameEffect::Chara(frame)],
            crysirror_image::temporal_pattern::PatternRng::msvc(1),
            &mut rt.host,
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some(r) = work.poll(&mut rt.host).unwrap() {
                break r;
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        };
        assert_eq!(
            result.target.rgba().as_ref(),
            expected.as_slice(),
            "mask metadata chooses native masked or unkeyed blend; no top-left colour key"
        );
        assert_eq!(result.random.draws, 0);
        assert_eq!(
            raw.rgba().as_ref(),
            [200, 100, 50, 0, 200, 100, 50, 128, 200, 100, 50, 255]
        );
        assert_eq!(base.rgba().as_ref(), [100, 100, 100, 255].repeat(3));
    }
}

#[test]
fn native_lamp_calcu_is_transactional_and_draw_zero_does_not_run_it() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for frames in [0, 1] {
        let (_dir, mut rt) = runtime();
        root(&mut rt);
        rt.set_canvas_image_presenter(true);
        let p = program(vec![
            vec![0x33330013, 9, 4, 109, 40, 40, 0, 10, 11, 1, 0],
            vec![0x22220004, 1, frames],
            vec![0x33330055, 1, 9],
        ]);
        let mut variables = Variables::fresh(VariableProfile::Legacy1000);
        variables.replace_random_state(PatternRng::msvc(1)).unwrap();
        let mut machine = Machine::with_variables(variables);
        let mut bridge =
            Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let mut reached = false;
        while Instant::now() < until {
            let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
                panic!()
            };
            if call.code == 0x33330055 {
                reached = true;
                break;
            }
            let previous_draws = machine.variables().unwrap().random_state().unwrap().draws;
            if !bridge
                .try_commit(&mut machine, &call, &mut rt.host)
                .unwrap()
            {
                assert_eq!(
                    machine.variables().unwrap().random_state().unwrap().draws,
                    previous_draws,
                    "pending engine work must not commit candidate lamp/global RNG"
                );
            }
            rt.prepare_canvas_images(|r, _| {
                Ok(Some(Arc::new(MemoryAssetReader(
                    [32, 64, 128, 255]
                        .repeat((r.size[0] * r.size[1]) as usize)
                        .into(),
                )) as Arc<dyn AssetReader>))
            })
            .unwrap();
            rt.tick(
                if bridge.dependency_pending() { 0 } else { 10 },
                Default::default(),
            )
            .unwrap();
            std::thread::yield_now();
        }
        assert!(reached);
        assert_eq!(bridge.effect_counter(4), Some((frames as i32, 999999)));
        assert_eq!(machine.variables().unwrap().random_state().unwrap().draws,if frames==0{0}else{2},"Calcu runs despite missing picture; Print needs a picture before any extra random flip");
        let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
        let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
            panic!()
        };
        assert!(rt
            .assets
            .surface(ClientId(1), asset)
            .unwrap()
            .rgba
            .chunks_exact(4)
            .all(|p| p == [32, 64, 128, 255]));
    }
}

#[test]
fn native_bura_runs_public_image_effect_worker_with_separate_q8_floors_and_native_count() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.set_canvas_image_presenter(true);
    let p = program(vec![
        vec![0x33330013, 9, 4, 74, 20, 8, 8, 100, 100, 10, 10],
        vec![0x22220004, 1, 1],
        vec![0x33330055, 1, 9],
    ]);
    let mut variables = Variables::fresh(VariableProfile::Legacy1000);
    variables.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(variables);
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut done = false;
    while Instant::now() < deadline {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        if call.code == 0x33330055 {
            done = true;
            break;
        }
        bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap();
        rt.prepare_canvas_images(|r, _| {
            Ok(Some(Arc::new(MemoryAssetReader(
                vec![255; (r.size[0] * r.size[1] * 4) as usize].into(),
            )) as Arc<dyn AssetReader>))
        })
        .unwrap();
        rt.tick(
            if bridge.dependency_pending() { 0 } else { 10 },
            Default::default(),
        )
        .unwrap();
        std::thread::yield_now();
    }
    assert!(done);
    assert_eq!(bridge.effect_counter(4), Some((1, 20)));
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        0
    );
    let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    assert!(
        rt.assets
            .surface(ClientId(1), asset)
            .unwrap()
            .rgba
            .chunks_exact(4)
            .all(|p| p == [254, 254, 254, 255]),
        "floor10%*256=25 and floor90%*256=230 retain independent floors"
    );
}

#[test]
fn legacy_mae_keeps_current_effect_count_and_picture_without_any_engine_operation() {
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    let p = program(vec![
        vec![0x33330013, 6, 4, 54, 20, 42, 70, 130],
        vec![0x33330013, 2, 4, 1],
    ]);
    let mut machine = Machine::default();
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    let scene = serde_json::to_value(&rt.host.engine).unwrap();
    let calls = bridge.api_calls;
    let count = bridge.effect_counter(4);
    let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
        panic!()
    };
    bridge.commit(&mut machine, &call, &mut rt.host).unwrap();
    assert_eq!(machine.pc(), call.next);
    assert_eq!(bridge.previous_effect(4), Some(54));
    assert_eq!(bridge.effect_counter(4), count);
    assert_eq!(bridge.api_calls, calls);
    assert_eq!(serde_json::to_value(&rt.host.engine).unwrap(), scene);
}

#[test]
fn pictures_and_fills_between_screen_effects_use_shared_ordered_jobs_each_native_frame() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.set_canvas_image_presenter(true);
    let p = program(vec![
        vec![0x33330013, 6, 0, 54, 20, 0, 0, 255],
        vec![0x33330013, 7, 4, 129, 20, 0, 0, 100, 100],
        vec![0x33330013, 6, 5, 54, 20, 255, 0, 0],
        vec![0x33330013, 7, 6, 129, 20, 0, 0, 100, 100],
        vec![0x22220004, 1, 2],
        vec![0x33330055, 1, 9],
    ]);
    let mut variables = Variables::fresh(VariableProfile::Legacy1000);
    variables.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(variables);
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut retained = None;
    while Instant::now() < deadline {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        if call.code == 0x33330055 {
            retained = Some(call);
            break;
        }
        bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap();
        rt.prepare_canvas_images(|r, _| {
            Ok(Some(Arc::new(MemoryAssetReader(
                [0, 0, 255, 255]
                    .repeat((r.size[0] * r.size[1]) as usize)
                    .into(),
            )) as Arc<dyn AssetReader>))
        })
        .unwrap();
        rt.tick(
            if bridge.dependency_pending() { 0 } else { 10 },
            Default::default(),
        )
        .unwrap();
        std::thread::yield_now();
    }
    let retained = retained.expect("ordered shared native draw completed");
    assert!(bridge
        .try_commit(&mut machine, &retained, &mut rt.host)
        .is_err());
    assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(retained));
    assert_eq!(bridge.effect_counter(4), Some((2, 20)));
    assert_eq!(bridge.effect_counter(6), Some((2, 20)));
    assert_eq!(bridge.effect_counter(5), Some((2, 9999)));
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        0
    );
    let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    assert!(
        rt.assets
            .surface(ClientId(1), asset)
            .unwrap()
            .rgba
            .chunks_exact(4)
            .all(|p| p == [75, 75, 75, 255]),
        "base blue -> grey(slot4) -> full red(slot5) -> exact grey75(slot6), each frame"
    );
    let canvas: Value = serde_json::from_slice(
        &rt.host
            .query_engine(ClientId(1), EngineQuery::Canvas(8))
            .unwrap(),
    )
    .unwrap();
    assert!(
        !canvas.to_string().contains("255,0,0,255"),
        "upper raw layer must not also be flattened into the lower Canvas"
    );
}

#[test]
fn native_fadeout_becomes_exact_fill_at_count_max_and_remains_for_draw_zero() {
    use crysirror_core::service::{AssetReader, MemoryAssetReader};
    use crysirror_image::temporal_pattern::PatternRng;
    use crysirror_system_nnn::expression::{VariableProfile, Variables};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_dir, mut rt) = runtime();
    root(&mut rt);
    rt.set_canvas_image_presenter(true);
    let p = program(vec![
        vec![0x33330013, 6, 0, 54, 20, 200, 100, 50],
        vec![0x33330013, 6, 4, 11, 2, 255, 0, 0],
        vec![0x22220004, 1, 4],
        vec![0x22220004, 1, 0],
        vec![0x33330055, 1, 9],
    ]);
    let mut variables = Variables::fresh(VariableProfile::Legacy1000);
    variables.replace_random_state(PatternRng::msvc(1)).unwrap();
    let mut machine = Machine::with_variables(variables);
    let mut bridge = Bridge::new(ClientId(1), CallbackProfile::Mugen2Legacy, bindings()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut complete = false;
    while Instant::now() < deadline {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!()
        };
        if call.code == 0x33330055 {
            assert!(bridge
                .try_commit(&mut machine, &call, &mut rt.host)
                .is_err());
            assert_eq!(machine.poll(&p, 1).unwrap(), Step::Native(call));
            complete = true;
            break;
        }
        bridge
            .try_commit(&mut machine, &call, &mut rt.host)
            .unwrap();
        rt.prepare_canvas_images(|r, _| {
            Ok(Some(Arc::new(MemoryAssetReader(
                [200, 100, 50, 255]
                    .repeat((r.size[0] * r.size[1]) as usize)
                    .into(),
            )) as Arc<dyn AssetReader>))
        })
        .unwrap();
        rt.tick(
            if bridge.dependency_pending() { 0 } else { 10 },
            Default::default(),
        )
        .unwrap();
        std::thread::yield_now();
    }
    assert!(complete);
    assert_eq!(bridge.effect_counter(4), Some((2, 2)));
    assert_eq!(bridge.previous_effect(4), Some(11));
    assert_eq!(
        machine.variables().unwrap().random_state().unwrap().draws,
        0
    );
    let layer = rt.host.engine.layers.get(ClientId(1), 116).unwrap();
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    assert!(rt.assets.surface(ClientId(1),asset).unwrap().rgba.chunks_exact(4).all(|p|p==[255,0,0,255]),"CountIsMax changes command to Fill54; Q15 endpoint254 or removed fade revealing the background is wrong");
}
