// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, AppState};
use crysirror_runtime::Runtime;
use crysirror_script_js::JsHost;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
fn app(source: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets/a")).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/b")).unwrap();
    std::fs::write(dir.path().join("assets/a/line.txt"), b"hello").unwrap();
    let image = crysirror_image::ImageSurface::filled(2, 2, [100, 20, 90, 255]).unwrap();
    std::fs::write(
        dir.path().join("assets/a/image.png"),
        crysirror_image::encode_png_uncompressed(&image).unwrap(),
    )
    .unwrap();
    let project = serde_json::json!({"schema":2,"name":"async JS","content_revision":"1","canvas":[128,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]});
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&dir.path().join("project.json"), &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    app.add_script_host(Box::new(
        JsHost::with_source("async", "main.ts", source).unwrap(),
    ))
    .unwrap();
    (dir, app)
}
fn until(app: &mut App, needle: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        app.frame(16, InputFrame::default()).unwrap();
        if app
            .engine()
            .unwrap()
            .log
            .iter()
            .any(|line| line.contains(needle))
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "missing {needle}; {:?}",
            app.engine().unwrap().log
        );
        std::thread::yield_now();
    }
}
#[test]
fn promises_await_native_jobs_fence_initialization_and_remain_client_scoped() {
    let (_dir, mut app) = app(r#"
      async function onInit() {
        const image = await resources.imageAsync('image.png');
        resources.publishImage(image, 6); resources.cancelDecode(image);
        const asset = await assets.loadAsync('line.txt','bytes'); assets.release(asset);
        const size = await save.saveAsync('one.save');
        if (size <= 0 || !(await save.existsAsync('one.save'))) throw Error('save missing');
        engine.log('init-ready');
      }
      function onStart() { engine.log('start-after-ready'); }
    "#);
    // Block all CPU workers: onInit returns a pending Promise, not a join.
    let pool = app.runtime().unwrap().assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut jobs = vec![];
    let (tx, rx) = std::sync::mpsc::channel();
    for _ in 0..pool.stats().workers {
        let gate = gate.clone();
        let tx = tx.clone();
        jobs.push(
            pool.spawn(move |_| {
                tx.send(())?;
                let (lock, wake) = &*gate;
                let value = lock.lock().unwrap();
                let (value, _) = wake
                    .wait_timeout_while(value, Duration::from_secs(10), |ready| !*ready)
                    .unwrap();
                anyhow::ensure!(*value, "gate timeout");
                Ok(())
            })
            .unwrap(),
        );
    }
    for _ in 0..jobs.len() {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    assert!(app.engine().unwrap().log.is_empty());
    let owner = app.engine().unwrap().client;
    let other = app.runtime().unwrap().client_id("b").unwrap();
    app.context().state().unwrap().client = other;
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    for job in jobs {
        job.wait().unwrap();
    }
    until(&mut app, "start-after-ready");
    assert_eq!(app.engine().unwrap().client, other);
    assert_eq!(
        app.engine()
            .unwrap()
            .log
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["init-ready", "start-after-ready"]
    );
    assert!(
        app.runtime()
            .unwrap()
            .assets
            .surface(owner, "__surface/6")
            .is_some()
    );
    assert!(
        app.runtime()
            .unwrap()
            .assets
            .surface(other, "__surface/6")
            .is_none()
    );
}
#[test]
fn cancellation_and_failed_jobs_reject_catchably_and_async_hook_errors_fault() {
    let (_dir, mut app) = app(r#"
      async function onInit() {
        try { await text.measureAsync('missing',20,'hello'); throw Error('accepted bad font'); }
        catch(e) { engine.log('font-rejected'); }
        const p=assets.loadAsync('line.txt','bytes');p.cancel();
        try { await p; throw Error('accepted cancellation'); } catch(e) { engine.log('cancelled'); }
      }
      function onStart(){engine.log('recovered');}
    "#);
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "recovered");
    assert_eq!(app.state(), AppState::Running);
    let (_dir, mut failed) =
        self::app("async function onInit(){await text.measureAsync('missing',20,'hello');}");
    // A worker rejection can become observable at init, start or frame.
    // Every lifecycle entry must preserve the same fault/error contract.
    match failed.init().and_then(|_| failed.start()) {
        Err(error) => assert!(format!("{error:#}").contains("rejected")),
        Ok(()) => {
            let end = Instant::now() + Duration::from_secs(5);
            loop {
                if let Err(error) = failed.frame(0, InputFrame::default()) {
                    assert!(format!("{error:#}").contains("rejected"));
                    break;
                }
                assert!(Instant::now() < end);
            }
        }
    }
    assert_eq!(failed.state(), AppState::Faulted);
}
#[test]
fn self_replenishing_microtasks_have_a_bounded_turn_and_shutdown_cancels_pending_work() {
    let (_dir, mut app) = app(r#"
      let count=0;
      function spin(){count++;Promise.resolve().then(spin);}
      function onInit(){spin();}
      function onUpdate(){engine.log('count='+count);}
    "#);
    app.init().unwrap();
    app.start().unwrap();
    for _ in 0..3 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    for line in &app.engine().unwrap().log {
        let count = line.strip_prefix("count=").unwrap().parse::<u64>().unwrap();
        assert!(count > 0 && count < 10000);
    }
    app.shutdown().unwrap();
    assert_eq!(app.state(), AppState::Stopped);
}

#[test]
#[ignore = "requires an explicitly supplied system font at runtime"]
fn standalone_async_menu_text_demo_uses_engine_surfaces_without_a_native_vm() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/engine/async-services.ts"),
    )
    .unwrap();
    let (_directory, mut app) = app(&source);
    let font =
        std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").expect("system font path")).unwrap();
    app.runtime_mut()
        .unwrap()
        .assets
        .text
        .add_font("default", font)
        .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "async-demo-ready");
    let owner = app.engine().unwrap().client;
    assert!(
        app.runtime()
            .unwrap()
            .host
            .engine
            .ui
            .menus
            .get(owner.0, 1)
            .is_some()
    );
    assert!(
        app.runtime()
            .unwrap()
            .host
            .engine
            .layers
            .get(owner, 100)
            .is_some()
    );
    until(&mut app, "text marker: shown");
    let surface = app
        .runtime()
        .unwrap()
        .assets
        .surface(owner, "__surface/50000")
        .unwrap();
    assert_eq!([surface.width, surface.height], [900, 150]);
    assert!(surface.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
    app.frame(
        16,
        InputFrame {
            pointer_x: 110,
            pointer_y: 10,
            buttons_pressed: crysirror_core::service::buttons::MOUSE_LEFT,
            ..Default::default()
        },
    )
    .unwrap();
    until(&mut app, "saved ");
    for _ in 0..30 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    assert_eq!(
        app.engine()
            .unwrap()
            .log
            .iter()
            .filter(|line| line.starts_with("saved "))
            .count(),
        1,
        "one menu activation must save exactly once"
    );
    app.shutdown().unwrap();
}

#[test]
fn acoustic_promises_publish_engine_owned_scenes_and_report_native_errors() {
    let (dir, mut app) = app(r#"
      async function onInit() {
        const scene = await audio.prepareAcousticsAsync('room.json');
        if (scene.triangles !== 2) throw Error('scene missing');
        const path = await audio.traceAcousticsAsync({source:[0,0,-4],listener:[0,0,0],settings:{air_absorption:[0,0,0]}});
        if (path.visibility !== 0 || path.gains[0] !== 0.5 || path.revision !== scene.revision) throw Error('wrong propagation');
        if (audio.acousticStats().scene.revision !== scene.revision) throw Error('released scene');
        try { await audio.prepareAcousticsAsync('missing.json'); throw Error('missing asset accepted'); }
        catch(error) { if (String(error).includes('missing asset accepted')) throw error; }
        engine.log('acoustics-ready');
      }
    "#);
    std::fs::write(dir.path().join("assets/a/room.json"),serde_json::to_vec(&serde_json::json!({"materials":[{"transmission":[0.5,0.25,0.125]}],"meshes":[{"id":1,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]})).unwrap()).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "acoustics-ready");
}

#[test]
fn hrtf_promises_prepare_scoped_engine_resources_and_release_owned_tickets() {
    let (dir, mut app) = app(r#"
      async function onInit() {
        const profile=await audio.prepareHrtfAsync('ears.json',{max_delay_seconds:0});
        if(profile.taps!==1 || profile.sample_rate!==48000 || profile.latency_frames!==64) throw Error('bad HRTF metadata');
        if(audio.hrtfStats().dataset.revision!==profile.revision) throw Error('profile was released');
        let released=false; try {audio.hrtfStatus(profile.revision);}catch(error){released=true;}
        if(!released) throw Error('helper leaked a retained operation');
        try {await audio.prepareHrtfAsync('missing.json',{});throw Error('missing file accepted');}
        catch(error){if(String(error).includes('missing file accepted'))throw error;}
        audio.clearHrtf();if(audio.hrtfStats().dataset!==null)throw Error('profile not cleared');
        engine.log('hrtf-ready');
      }
    "#);
    std::fs::write(
        dir.path().join("assets/a/ears.json"),
        br#"{"sample_rate":48000,"measurements":[{"direction":[0,0,-1],"left":[1],"right":[1]}]}"#,
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "hrtf-ready");
}

#[test]
fn audio_promises_prepare_play_seek_release_and_preserve_shared_mixer_handles() {
    let (dir, mut app) = app(r#"
      async function onInit() {
        let forbidden=false;try {audio.playSfx({asset:'tone.wav'});}catch(e){forbidden=true;}
        if(!forbidden)throw Error('cold realtime SFX did synchronous decoding');
        const preload=audio.preloadSfxAsync('tone.wav');const ticket=preload.ticket;
        const clip=await preload;if(clip.frames!==16000)throw Error('bad decoded clip');
        let released=false;try{audio.playbackStatus(ticket);}catch(e){released=true;}
        if(!released)throw Error('retained preload ticket');
        const voice=await audio.playSfxAsync({asset:'tone.wav',looping:true});
        if(!audio.voice(voice).playing)throw Error('SFX not published');
        const transport=audio.playbackInfo(voice);
        if(transport.source_rate!==8000 || transport.source_frames!==16000)throw Error('bad source transport units');
        if(transport.transport_frames!==0 || transport.stopping || transport.pausing)throw Error('bad initial transport state');
        audio.scheduleVoiceResumeWithOptions(voice,2,1,{policy:'preserve',reset_gain:true});
        if(audio.playbackInfo(voice).paused)throw Error('preserve froze input');
        let badPolicy=false;try{audio.scheduleVoiceResumeWithOptions(voice,2,1,{policy:'unknown'});}catch(e){badPolicy=true;}
        if(!badPolicy)throw Error('unknown resume policy accepted');
        if(audio.playbackInfo(voice).resume_delay_frames!==96)throw Error('bad output clock delay');
        audio.pauseVoice(voice,false);
        audio.clearSfxCache();if(!audio.voice(voice).playing)throw Error('cache clear stopped live PCM');
        audio.stopVoice(voice,0);
        const music=await audio.playMusicAsync('bgm',{asset:'tone.wav'},0);
        if(audio.sourceStatus(music)===null)throw Error('music not worker-buffered');
        const seek=await audio.seekVoiceAsync(music,9000);
        if(seek.source_frame!==9000)throw Error('seek not exact');
        try{await audio.playMusicAsync('bgm',{asset:'missing.wav'},0);throw Error('missing accepted');}
        catch(e){if(String(e).includes('missing accepted'))throw e;}
        if(!audio.voice(music).playing)throw Error('failed replacement stopped music');
        engine.log('audio-async-ready');
      }
    "#);
    let size = 32000u32;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for _ in 0..16000 {
        bytes.extend(8192i16.to_le_bytes());
    }
    std::fs::write(dir.path().join("assets/a/tone.wav"), bytes).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "audio-async-ready");
}

#[test]
fn reverb_promise_publishes_engine_bus_resources_and_releases_its_ticket() {
    let (_dir, mut app) = app(r#"
      async function onInit() {
        const result = await audio.prepareReverbAsync({room:{decay_seconds:0.3},sends:[{bus:'sfx',gain:0.5}]});
        if(result.kind!=='parametric_late_reverb' || result.sample_rate!==48000) throw Error('wrong reverb result');
        if(audio.reverbSettings().sends[0].gain!==0.5) throw Error('resource not published');
        let released=false;try {audio.reverbStatus(result.revision);}catch(error){released=true;}
        if(!released) throw Error('helper leaked its preparation receipt');
        try {await audio.prepareReverbAsync({room:{damping:2}});throw Error('invalid room accepted');}
        catch(error){if(String(error).includes('invalid room accepted'))throw error;}
        audio.clearReverb();if(audio.reverbSettings()!==null)throw Error('room survived clear');
        engine.log('reverb-ready');
      }
    "#);
    app.init().unwrap();app.start().unwrap();until(&mut app,"reverb-ready");
}

#[test]
fn reflection_promises_publish_captured_paths_and_a_prepared_shared_voice() {
    let (dir,mut app)=app(r#"
      let movingVoice=null, moved=false;
      async function onInit() {
        await audio.prepareAcousticsAsync('wall.json');
        const result=await audio.traceReflectionsAsync({source:[0,0,2],listener:[0,0,0]});
        if(result.paths.paths.length!==1 || Math.abs(result.paths.paths[0].length_metres-6)>1e-5)throw Error('wrong image source path');
        const voice=await audio.playSfxAsync({asset:'pulse.wav',paused:true,spatial:{position:[0,0,2]}});
        const pending=audio.prepareVoiceReflectionsAsync(voice,{});const ticket=pending.ticket;
        const echoes=await pending;
        if(echoes.paths.paths.length!==1 || audio.reflectionsVoice(voice).paths===null)throw Error('effect not published');
        let released=false;try{audio.reflectionStatus(ticket);}catch(error){released=true;}
        if(!released)throw Error('helper leaked its actual receipt');
        try{await audio.traceReflectionsAsync({source:[0,0,2],listener:[0,0,0],options:{triangle_budget:1}});throw Error('incomplete query accepted');}
        catch(error){if(String(error).includes('incomplete query accepted'))throw error;}
        audio.clearVoiceReflections(voice);if(audio.reflectionsVoice(voice).paths!==null)throw Error('echo survived clear');
        await audio.prepareVoiceReflectionsAsync(voice,{});
        audio.pauseVoice(voice,false);engine.log('reflections-ready');
        movingVoice=await audio.playSfxAsync({asset:'pulse.wav',looping:true,spatial:{position:[0,0,2]},reflections:{}});
        audio.trackVoiceReflections(movingVoice,{refresh_interval_ms:16,crossfade_ms:1});
      }
      function onUpdate(dt) {
        if(movingVoice===null)return;
        const state=audio.reflectionsVoice(movingVoice).tracking;
        if(state.error!==null)throw Error(state.error);
        if(state.completed_updates===0)return;
        if(!moved){audio.setListener({position:[1,0,0],forward:[0,0,-1],up:[0,1,0]});moved=true;return;}
        if(state.published.query.listener[0]!==1)return;
        audio.clearVoiceReflections(movingVoice);
        if(audio.reflectionsVoice(movingVoice).tracking!==null)throw Error('tracking survived clear');
        audio.stopVoice(movingVoice,0);movingVoice=null;engine.log('tracking-ready');
      }
    "#);
    std::fs::write(dir.path().join("assets/a/wall.json"),serde_json::to_vec(&serde_json::json!({"materials":[{"absorption":[0,0,0],"transmission":[0,0,0],"scattering":0}],"meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]})).unwrap()).unwrap();
    let mut wave=Vec::new();let size=32u32;
    wave.extend(b"RIFF");wave.extend((size+36).to_le_bytes());wave.extend(b"WAVEfmt ");wave.extend(16u32.to_le_bytes());wave.extend(1u16.to_le_bytes());wave.extend(1u16.to_le_bytes());wave.extend(48000u32.to_le_bytes());wave.extend(96000u32.to_le_bytes());wave.extend(2u16.to_le_bytes());wave.extend(16u16.to_le_bytes());wave.extend(b"data");wave.extend(size.to_le_bytes());
    for _ in 0..16 {wave.extend(8192i16.to_le_bytes());}
    std::fs::write(dir.path().join("assets/a/pulse.wav"),wave).unwrap();
    app.init().unwrap();app.start().unwrap();until(&mut app,"tracking-ready");
}

#[test]
fn prepared_audio_promises_publish_resources_independent_of_receipts_and_voices() {
    let (dir, mut app) = app(r#"
        async function onInit() {
            const prepared=await audio.prepareSourceAsync('tone.wav',{measure_frames:true});
            const resource=prepared.resource;
            if(prepared.info.source_frames!==64 || audio.sourceInfo(resource).source_rate!==8000) throw Error('source metadata');
            const voice=await audio.playSourceAsync(resource,{paused:true,looping:true});
            audio.releaseSource(resource);
            let stale=false;try{audio.sourceInfo(resource);}catch(e){stale=true;}
            if(!stale || !audio.voice(voice).source_playing) throw Error('lease missing');
            audio.pauseVoice(voice,false);audio.stopVoice(voice,0);
            engine.log('source-promises-ready');
        }
    "#);
    let size = 128u32;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for _ in 0..64 {
        b.extend(8192i16.to_le_bytes());
    }
    std::fs::write(dir.path().join("assets/a/tone.wav"), b).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "source-promises-ready");
    assert_eq!(
        app.runtime()
            .unwrap()
            .host
            .channel_audio
            .resources
            .resource_count(),
        0
    );
}

#[test]
fn prepared_voice_compression_options_cross_the_js_promise_boundary() {
    let (dir, mut app) = app(r#"
        async function onInit(){
            let rejected=false;
            try{await audio.prepareSourceAsync('tone.wav',{transform:{kind:'siglus_jitan',percent:99}});}catch(e){rejected=true;}
            if(!rejected)throw Error('invalid transform accepted');
            const source=await audio.prepareSourceAsync('tone.wav',{frame_limit:400,transform:{kind:'siglus_jitan',percent:200}});
            if(source.info.source_frames!==400 || source.info.source_rate!==8000)throw Error('transformed metadata');
            const voice=await audio.playSourceAsync(source.resource,{paused:true,bus:'voice'});
            audio.releaseSource(source.resource);
            if(audio.playbackInfo(voice).source_frames!==400)throw Error('voice used original source');
            audio.stopVoice(voice,0);engine.log('compression-promises-ready');
        }
    "#);
    let mut b = b"RIFF".to_vec();
    b.extend(1636u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(1600u32.to_le_bytes());
    for _ in 0..800 {
        b.extend(8192i16.to_le_bytes());
    }
    std::fs::write(dir.path().join("assets/a/tone.wav"), b).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    until(&mut app, "compression-promises-ready");
    assert_eq!(
        app.runtime()
            .unwrap()
            .host
            .channel_audio
            .resources
            .resource_count(),
        0
    );
}

#[test]
fn acoustic_instance_promises_and_entity_facade_drive_shared_world_geometry() {
    let (dir,mut app)=app(r#"
        let reported=false;
        async function onInit(){
            const initial=await audio.prepareAcousticsAsync('door.json');
            const moved=await audio.updateAcousticsAsync([{id:7,transform:[1,0,0,0,0,1,0,0,0,0,1,0,30,0,0,1]}]);
            if(moved.scene_generation!==initial.scene_generation || moved.revision===initial.revision || moved.geometry.reused_mesh_bvhs!==1)throw Error('instance commit');
            const path=await audio.traceAcousticsAsync({source:[0,0,-4],listener:[0,0,0],settings:{air_absorption:[0,0,0]}});
            if(path.gains[0]!==1)throw Error('door did not move');
            const door=world.spawn({name:'door'});
            audio.bindAcousticMesh({mesh:7,entity:door});
        }
        function onUpdate(){
            if(!reported&&audio.acousticStats().instances.completed_updates>0){reported=true;engine.log('acoustic-entity-followed');}
        }
    "#);
    std::fs::write(dir.path().join("assets/a/door.json"),serde_json::to_vec(&serde_json::json!({"materials":[{"transmission":[0,0,0]}],"meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]})).unwrap()).unwrap();
    app.init().unwrap();app.start().unwrap();until(&mut app,"acoustic-entity-followed");
}


#[test]
fn js_gain_groups_route_prepared_sources_and_preserve_partial_parent_settings(){
    let (dir,mut app)=app(r#"
        async function onInit(){
            audio.setMixGroup('game',{gain:0.5});audio.setMixGroup('voice',{parent:'game',gain:0.5});
            if(audio.nativeMixGroup('voice').effective_gain!==0.25)throw Error('native graph missing');
            const resource=await audio.prepareSourceAsync('group.wav');
            const voice=await audio.playSourceAsync(resource.resource,{paused:true,mix_group:'voice'});
            audio.releaseSource(resource.resource);audio.setMixGroup('voice',{gain:0.25});
            if(audio.mixGroup('voice').settings.parent!=='game')throw Error('parent dropped');
            let rejected=false;try{audio.setMixGroup('game',{parent:'voice'});}catch(e){rejected=true;}
            if(!rejected)throw Error('cycle accepted');audio.setVoiceMixGroup(voice,null);
            audio.removeMixGroup('voice');audio.removeMixGroup('game');audio.stopVoice(voice,0);
            engine.log('mix-groups-ready');
        }
    "#);
    let mut b=b"RIFF".to_vec();b.extend(164u32.to_le_bytes());b.extend(b"WAVEfmt ");b.extend(16u32.to_le_bytes());b.extend(1u16.to_le_bytes());b.extend(1u16.to_le_bytes());b.extend(8000u32.to_le_bytes());b.extend(16000u32.to_le_bytes());b.extend(2u16.to_le_bytes());b.extend(16u16.to_le_bytes());b.extend(b"data");b.extend(128u32.to_le_bytes());for _ in 0..64{b.extend(8192i16.to_le_bytes());}std::fs::write(dir.path().join("assets/a/group.wav"),b).unwrap();app.init().unwrap();app.start().unwrap();until(&mut app,"mix-groups-ready");
}

#[test]
fn doppler_options_and_motion_inspection_cross_the_real_js_promise_boundary() {
    let (dir,mut app)=app(r#"
      let voice=null,reported=false;
      async function onInit() {
        audio.setMotionSettings({max_interval_ms:200,teleport_distance:20});
        if(audio.motionSettings().teleport_distance!==20)throw Error('motion policy missing');
        let invalid=false;try{audio.setMotionSettings({unknown:true});}catch(e){invalid=true;}
        if(!invalid)throw Error('unknown motion policy accepted');
        audio.setListener({position:[0,0,0],forward:[0,0,-1],up:[0,1,0],velocity:[0,0,0]});
        voice=await audio.playSfxAsync({asset:'doppler.wav',looping:true,spatial:{position:[0,0,-10],velocity:[0,0,50],min_distance:20,max_distance:100,doppler:{speed_of_sound:100,smoothing_ms:0}}});
      }
      function onUpdate(dt) {
        if(!voice||reported)return;
        const state=audio.dopplerVoice(voice);
        if(Math.abs(state.ratio-2)>1e-5)return;
        if(state.effective_rate!==2 || state.motion.source.status!=='explicit')throw Error('Doppler did not reach engine DSP');
        audio.resetMotion();reported=true;engine.log('doppler-ready');
      }
    "#);
    let size=32000u32;let mut b=b"RIFF".to_vec();b.extend((36+size).to_le_bytes());b.extend(b"WAVEfmt ");b.extend(16u32.to_le_bytes());b.extend(1u16.to_le_bytes());b.extend(1u16.to_le_bytes());b.extend(8000u32.to_le_bytes());b.extend(16000u32.to_le_bytes());b.extend(2u16.to_le_bytes());b.extend(16u16.to_le_bytes());b.extend(b"data");b.extend(size.to_le_bytes());for _ in 0..16000{b.extend(8192i16.to_le_bytes());}
    std::fs::write(dir.path().join("assets/a/doppler.wav"),b).unwrap();
    app.init().unwrap();app.start().unwrap();until(&mut app,"doppler-ready");
}
