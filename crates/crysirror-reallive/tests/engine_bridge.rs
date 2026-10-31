// SPDX-License-Identifier: MPL-2.0
//! Native bytecode -> public engine services. No original engine, fonts or GPU.
use crysirror_core::{
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_reallive::vm::{
    archive,
    bytecode::{Opcode, asm},
    expr::encode,
    scenario,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn command(module: u8, opcode: u16, args: &[Vec<u8>]) -> Vec<u8> {
    asm::command(Opcode::new(1, module, opcode, 0), args.len() as u16, args)
}
fn fixture(code: Vec<u8>) -> (tempfile::TempDir, Runtime) {
    fixture_config(code, json!({}))
}
fn fixture_config(code: Vec<u8>, config: serde_json::Value) -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(
        t.path().join("assets/Gameexe.ini"),
        "#SEEN_START=1\n#REGNAME=\"synthetic\"\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("assets/Seen.txt"),
        archive::build(&[(1, scenario::build(&code, &[1_000_000], &[]))]),
    )
    .unwrap();
    let p:crysirror_runtime::Project=serde_json::from_value(json!({"schema":2,"name":"bridge test","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"reallive","game":"synthetic","config":config}]})).unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_reallive::register(&mut registry).unwrap();
    let r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    (t, r)
}
#[test]
fn native_draw_and_wait_produce_engine_owned_nodes_editable_without_vm() {
    let mut code = command(4, 0, &[encode::quoted(b"native engine title")]);
    code.extend(command(
        33,
        31,
        &[
            encode::int(0),
            encode::int(12),
            encode::int(34),
            encode::int(56),
        ],
    ));
    code.extend(command(4, 100, &[encode::int(1000)]));
    code.extend(command(4, 0, &[encode::quoted(b"after wait")]));
    let (_t, mut r) = fixture(code);
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(
        r.host.engine.ui.title.as_ref().unwrap().1,
        "native engine title"
    );
    let surface = r.host.engine.canvases.surface(ClientId(1), 0).unwrap();
    assert_eq!(surface.nodes.len(), 1);
    let before = r.inspect()["sessions"][0]["native"]["instructions"].clone();
    r.tick(999, InputFrame::default()).unwrap();
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
    // Public service call changes the live retained drawing while native PC waits.
    r.host
        .submit_engine(
            ClientId(1),
            1,
            &serde_json::to_vec(&json!({"canvases":[{"op":"clear","id":0,"color":[255,0,0,255]}]}))
                .unwrap(),
        )
        .unwrap();
    let value = serde_json::to_value(r.host.engine.canvases.surface(ClientId(1), 0)).unwrap();
    assert_eq!(
        value["nodes"][0]["layer"]["paint"]["color"],
        json!([255, 0, 0, 255])
    );
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
    r.tick(1, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1, "after wait");
    assert_eq!(
        r.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
    assert!(
        r.host
            .query_engine(
                ClientId(999),
                crysirror_core::service::EngineQuery::Canvas(0)
            )
            .is_err()
    );
}

#[test]
fn native_get_dc_pixel_reads_shared_canvas_rgb_outputs() {
    let mut code = command(33,31,&[encode::int(0),encode::int(17),encode::int(81),encode::int(202)]);
    code.extend(command(30,31,&[encode::int(0),encode::int(0),encode::int(0),encode::int_a(0),encode::int_a(1),encode::int_a(2)]));
    for i in 0..3 {
        code.extend(command(4,0,&[encode::int_a(i)]));
        code.extend(command(4,100,&[encode::int(100)]));
    }
    let (_t,mut r)=fixture(code);
    r.tick(0,InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1,"17");
    r.tick(100,InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1,"81");
    r.tick(100,InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1,"202");
}
#[test]
fn unsupported_operation_fails_at_original_pc_and_never_acknowledges() {
    let code = command(99, 60000, &[]);
    let (_t, mut r) = fixture(code);
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("unsupported native syscall"), "{error}");
    let before = r.inspect()["sessions"][0]["native"]["instructions"].clone();
    assert!(r.tick(16, InputFrame::default()).is_err());
    assert_eq!(before, r.inspect()["sessions"][0]["native"]["instructions"]);
}

#[test]
fn portable_disc_policy_is_explicit_and_does_not_report_authentication() {
    let mut code = command(5, 114, &[encode::quoted(b"disc-marker"),encode::int(124),encode::quoted(b"native-code")]);
    code.extend(command(4, 0, &[encode::quoted(b"continued under policy")]));
    let (_t, mut r) = fixture(code.clone());
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1, "continued under policy");
    let inspection = r.inspect();
    let policy = &inspection["sessions"][0]["native"]["optical_disc_check"];
    assert_eq!(policy["status"], "policy-disabled");
    assert_eq!(policy["authentication_performed"], false);
    assert!(policy["verification_result"].is_null());
    assert_eq!(policy["policy_continuation_store"], 1);
    let (_t, mut strict) = fixture_config(code, json!({"optical_disc_check":"strict"}));
    assert!(format!("{:#}",strict.tick(0, InputFrame::default()).unwrap_err()).contains("strict optical-disc policy"));
    assert!(strict.host.engine.ui.title.is_none());
    assert_eq!(strict.inspect()["sessions"][0]["native"]["optical_disc_check"]["calls"], 0);
}

#[test]
fn portable_disc_policy_rejects_unknown_or_malformed_abi() {
    for code in [
        command(5, 114, &[encode::quoted(b"marker"),encode::int(124)]),
        command(5, 114, &[encode::int(7),encode::int(124),encode::quoted(b"code")]),
        command(5, 120, &[]),
        asm::command(Opcode::new(1,5,114,1),3,&[encode::quoted(b"marker"),encode::int(124),encode::quoted(b"code")]),
    ] {
        let (_t,mut r) = fixture(code);
        assert!(r.tick(0,InputFrame::default()).is_err());
        assert_eq!(r.inspect()["sessions"][0]["native"]["optical_disc_check"]["calls"],0);
    }
}

#[test]
fn gameexe_preloaded_dll_slots_are_available_to_original_call_dll() {
    let mut code=asm::command(Opcode::new(2,1,12,5),6,&[encode::int(1),encode::int(0),encode::int(1),encode::int(0),encode::int(0),encode::int(1)]);
    code.extend(command(4,0,&[encode::quoted(b"native DLL returned")]));
    let (t,mut r)=fixture(code);
    std::fs::write(t.path().join("assets/Gameexe.ini"),"#SEEN_START=1\n#DLL.001=\"EF00\"\n").unwrap();
    r.tick(0,InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1,"native DLL returned");
}
#[test]
fn native_name_dialog_reads_shared_text_field_and_resumes_only_on_submit() {
    let mut code = command(4, 1302, &[encode::int(0), encode::quoted(b"Name")]);
    code.extend(command(4, 0, &[encode::quoted(b"submitted")]));
    let (_t, mut r) = fixture(code);
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.get(1, 610000).unwrap().text, "");
    r.host
        .submit_engine(
            ClientId(1),
            1,
            &serde_json::to_vec(&json!({"ui":[{"op":"insert_text","text":"Alice"}]})).unwrap(),
        )
        .unwrap();
    r.tick(
        16,
        InputFrame {
            buttons_pressed: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
    r.tick(16, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1, "submitted");
    assert!(r.host.engine.ui.get(1, 610000).is_none());
}

use crysirror_tasks::{Task, TaskPool};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
struct Gate {
    gate: Arc<(Mutex<bool>, Condvar)>,
    tasks: Vec<Task<()>>,
}
impl Gate {
    fn block(p: &TaskPool) -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut tasks = vec![];
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..p.stats().workers {
            let g = gate.clone();
            let tx = tx.clone();
            tasks.push(
                p.spawn(move |_| {
                    tx.send(())?;
                    let (v, _) = g
                        .1
                        .wait_timeout_while(g.0.lock().unwrap(), Duration::from_secs(5), |v| !*v)
                        .unwrap();
                    anyhow::ensure!(*v, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..tasks.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self { gate, tasks }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
        for t in self.tasks.drain(..) {
            t.wait().unwrap();
        }
    }
}

fn tone() -> Vec<u8> {
    let size = 9600u32;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(48000u32.to_le_bytes());
    b.extend(96000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for _ in 0..4800 {
        b.extend(8192i16.to_le_bytes());
    }
    b
}

#[test]
fn native_dstrack_segment_uses_exact_shared_source_frames_and_loops() {
    let mut code = command(20, 0, &[encode::quoted(b"segment")]);
    code.extend(command(4, 100, &[encode::int(10000)]));
    let (t, mut r) = fixture(code);
    std::fs::write(t.path().join("assets/Gameexe.ini"), "#SEEN_START=1\n#REGNAME=\"synthetic\"\n#DSTRACK=960,2880,1920,\"tone\",\"segment\"\n").unwrap();
    std::fs::create_dir(t.path().join("assets/BGM")).unwrap();
    std::fs::write(t.path().join("assets/BGM/tone.wav"), tone()).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert_eq!(info.position_frames, 960);
    assert_eq!(info.source_frames, Some(2880));
    assert_eq!(info.source_rate, 48000);
    assert!(info.looping);
    r.tick(30, InputFrame::default()).unwrap();
    assert_eq!(r.host.audio_info(ClientId(1), 0).unwrap().unwrap().position_frames, 2400);
    r.tick(20, InputFrame::default()).unwrap();
    let looped = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert_eq!(looped.position_frames, 2400);
    assert!(looped.source_playing);
    assert!(r.host.take_pcm_output().iter().flatten().any(|s| s.abs() > 0.01));
}

#[test]
fn native_gameexe_sprite_cursor_is_engine_owned_and_native_selection_is_observed() {
    use crysirror_scene::ui::cursor::{CursorSelect, StandardCursor};
    let mut code = command(4, 100, &[encode::int(100)]);
    code.extend(command(4, 207, &[encode::int(99)]));
    code.extend(command(4, 100, &[encode::int(100)]));
    let (t, mut r) = fixture(code);
    std::fs::write(t.path().join("assets/Gameexe.ini"), "#SEEN_START=1\n#MOUSE_CURSOR=3\n#MOUSE_CURSOR.003.NAME=\"Pointer\"\n#MOUSE_CURSOR.003.CONT=2\n#MOUSE_CURSOR.003.SPEED=300\n").unwrap();
    std::fs::create_dir(t.path().join("assets/PDT")).unwrap();
    // Independent BMP fixture: magenta colour key, two 32x32 frames, and a
    // white marker at hotspot (4,6) in the native separate marker rectangle.
    let mut bmp = vec![0u8; 54 + 80 * 80 * 3];
    bmp[..2].copy_from_slice(b"BM");
    let len = bmp.len() as u32;
    bmp[2..6].copy_from_slice(&len.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&80i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&80i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    for p in bmp[54..].chunks_exact_mut(3) { p.copy_from_slice(&[255, 0, 255]); }
    for (x, y, rgb) in [(8, 8, [10,20,30]), (40, 8, [40,50,60]), (12, 54, [255;3])] {
        let at = 54 + ((79-y) * 80 + x) * 3;
        bmp[at..at+3].copy_from_slice(&[rgb[2],rgb[1],rgb[0]]);
    }
    std::fs::write(t.path().join("assets/PDT/Pointer.bmp"), bmp).unwrap();
    let pool=r.assets.cpu_tasks();
    let gate=Gate::block(&pool);
    r.tick(0, InputFrame::default()).unwrap();
    assert!(r.host.engine.ui.cursors.selected.is_none());
    assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],0);
    for _ in 0..3 {
        r.tick(0,InputFrame::default()).unwrap();
        assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],0);
    }
    drop(gate);
    let deadline=Instant::now()+Duration::from_secs(5);
    while r.host.engine.ui.cursors.selected.is_none() {
        r.tick(0,InputFrame::default()).unwrap();
        assert!(Instant::now()<deadline,"cursor worker did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
    let cursors = &r.host.engine.ui.cursors;
    assert_eq!(cursors.selected, Some((1, CursorSelect::Custom { id:3 })));
    let cursor = cursors.get(1,3).unwrap();
    assert_eq!(cursor.steps, [[0,150],[1,150]]);
    assert_eq!(cursor.frames[0].hotspot, [4,6]);
    assert_eq!(&cursor.frames[0].rgba[..8], &[10,20,30,255,255,0,255,0]);
    assert!(cursors.get(2,3).is_none());
    r.tick(100, InputFrame::default()).unwrap();
    assert_eq!(r.host.engine.ui.cursors.selected, Some((1, CursorSelect::Standard {shape:StandardCursor::Default})));
    assert!(r.host.engine.ui.cursors.get(1,3).is_some());
}

#[test]
fn native_cursor_worker_failure_does_not_execute_the_original_script() {
    let (t,mut r)=fixture(command(4,0,&[encode::quoted(b"must not execute")]));
    std::fs::write(t.path().join("assets/Gameexe.ini"),"#SEEN_START=1\n#MOUSE_CURSOR=0\n#MOUSE_CURSOR.000.NAME=\"broken\"\n").unwrap();
    std::fs::create_dir(t.path().join("assets/PDT")).unwrap();
    let mut header=b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    header.extend(80u32.to_be_bytes());header.extend(80u32.to_be_bytes());
    std::fs::write(t.path().join("assets/PDT/broken.png"),header).unwrap();
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        if let Err(e)=r.tick(0,InputFrame::default()) {
            assert!(format!("{e:#}").contains("shared cursor image preparation failed"));
            break;
        }
        assert!(Instant::now()<deadline,"cursor failure was not observed");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(r.host.engine.ui.title.is_none());
    assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],0);
}
#[test]
fn native_audio_event_stays_at_original_pc_until_shared_preparation_and_play_wait_complete() {
    let mut code = command(21, 1, &[encode::quoted(b"tone"), encode::int(0)]);
    code.extend(command(4, 0, &[encode::quoted(b"after native audio")]));
    let (t, mut r) = fixture(code);
    std::fs::create_dir(t.path().join("assets/WAV")).unwrap();
    std::fs::write(t.path().join("assets/WAV/tone.wav"), tone()).unwrap();
    // The VM still waits for source completion, while the shared bus owns its tail.
    let effect = crysirror_audio::reverb::PreparedReverb::prepare(
        48000,
        crysirror_audio::reverb::ReverbSettings {
            sends: vec![crysirror_audio::reverb::ReverbSend {
                bus: crysirror_audio::mixer::Bus::Sfx,
                gain: 1.0,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    r.host
        .channel_audio
        .mixer
        .set_reverb(1, Some(effect))
        .unwrap();
    r.set_audio_nonblocking(true);
    assert!(!r.assets.is_nonblocking());
    let pool = r.assets.io_tasks();
    let gate = Gate::block(&pool);
    r.tick(0, InputFrame::default()).unwrap();
    let first = r.inspect()["sessions"][0]["native"]["instructions"].clone();
    assert!(!r.host.audio_playing(ClientId(1), 100).unwrap());
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
        assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"], first);
        assert!(r.host.engine.ui.title.is_none());
    }
    drop(gate);
    let end = Instant::now() + Duration::from_secs(5);
    while !r.host.audio_playing(ClientId(1), 100).unwrap() {
        r.tick(0, InputFrame::default()).unwrap();
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    // A real geometric echo remains in the shared mixer after the source EOF.
    let scene=crysirror_acoustics::AcousticScene::build(serde_json::from_value(json!({
        "materials":[{"absorption":[0,0,0],"transmission":[0,0,0],"scattering":0}],
        "meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]}]
    })).unwrap()).unwrap();
    let query: crysirror_acoustics::reflections::ReflectionQuery =
        serde_json::from_value(json!({"source":[0,0,2],"listener":[0,0,0]})).unwrap();
    let paths = crysirror_acoustics::reflections::simulate(&scene, &query).unwrap();
    assert_eq!(paths.paths.len(), 1);
    let echoes =
        crysirror_acoustics::reflection_dsp::EarlyReflections::prepare(48000, paths, 0).unwrap();
    r.host
        .channel_audio
        .set_reflections(1, 100, Some(echoes))
        .unwrap();
    r.tick(99, InputFrame::default()).unwrap();
    assert!(r.host.engine.ui.title.is_none());
    r.tick(1, InputFrame::default()).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(
        r.host.engine.ui.title.as_ref().unwrap().1,
        "after native audio"
    );
    assert!(!r.host.audio_playing(ClientId(1), 100).unwrap());
    assert!(
        r.host.channel_audio.mixer.stats().active > 0,
        "native wait incorrectly consumed the derived echo tail"
    );
    r.tick(50, InputFrame::default()).unwrap();
    assert!(
        r.host
            .take_pcm_output()
            .iter()
            .flatten()
            .any(|sample| sample.abs() > 0.001),
        "original native source ended without a shared reverb tail"
    );
}

#[test]
fn native_audio_worker_failure_never_acknowledges_following_instruction() {
    let mut code = command(21, 0, &[encode::quoted(b"bad"), encode::int(0)]);
    code.extend(command(4, 0, &[encode::quoted(b"must not run")]));
    let (t, mut r) = fixture(code);
    std::fs::create_dir(t.path().join("assets/WAV")).unwrap();
    std::fs::write(t.path().join("assets/WAV/bad.wav"), b"invalid wave").unwrap();
    r.assets.set_nonblocking(true);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match r.tick(0, InputFrame::default()) {
            Ok(()) => {}
            Err(e) => {
                assert!(format!("{e:#}").contains("shared audio preparation failed"));
                break;
            }
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(r.host.engine.ui.title.is_none());
}
fn tone_fixture_assets(t:&tempfile::TempDir,valid:bool){
 let assets=t.path().join("assets");std::fs::create_dir(assets.join("PDT")).unwrap();std::fs::create_dir(assets.join("dat")).unwrap();std::fs::write(assets.join("Gameexe.ini"),"#SEEN_START=1\n#REGNAME=\"synthetic\"\n#TONECURVE_FILENAME=\"curve.tcc\"\n").unwrap();let image=crysirror_image::ImageSurface{width:2,height:1,rgba:vec![10,20,30,0,70,80,90,128]};std::fs::write(assets.join("PDT/Base.png"),crysirror_image::encode_png(&image).unwrap()).unwrap();let mut bytes=vec![0u8;0xfa8+2*0x340];bytes[..4].copy_from_slice(&(if valid{1000i32}else{1001i32}).to_le_bytes());bytes[4..8].copy_from_slice(&2i32.to_le_bytes());for table in 0..2{for c in 0..3{for n in 0..256{bytes[0xfe8+table*0x340+c*256+n]=if table==0{255-n as u8}else{(n as u8)^0x55};}}}std::fs::write(assets.join("dat/curve.tcc"),bytes).unwrap();
}
#[test]
fn native_tone_variants_wait_and_publish_distinct_shared_images(){
 use std::time::{Duration,Instant};
 let mut code=command(71,1000,&[encode::int(0),encode::quoted(b"base?010"),encode::int(1)]);code.extend(command(4,0,&[encode::quoted(b"tone one ready")]));code.extend(command(4,100,&[encode::int(100)]));code.extend(command(71,1000,&[encode::int(1),encode::quoted(b"base?020"),encode::int(1)]));code.extend(command(4,0,&[encode::quoted(b"tone two ready")]));code.extend(command(4,100,&[encode::int(100)]));let(t,mut r)=fixture(code);tone_fixture_assets(&t,true);let gate=Gate::block(&r.assets.cpu_tasks());r.tick(0,InputFrame::default()).unwrap();assert!(r.host.engine.ui.title.is_none());let before=r.inspect()["sessions"][0]["native"]["instructions"].clone();for _ in 0..3{r.tick(0,InputFrame::default()).unwrap();assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],before);}drop(gate);let end=Instant::now()+Duration::from_secs(10);while r.host.engine.ui.title.is_none(){r.tick(0,InputFrame::default()).unwrap();if r.host.engine.ui.title.is_none(){assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],before);}assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(1));}assert_eq!(r.host.engine.ui.title.as_ref().unwrap().1,"tone one ready");let layer=serde_json::to_value(r.host.engine.layers.get(ClientId(1),1_000_000).unwrap()).unwrap();let first=layer["paint"]["asset"].as_str().unwrap().to_owned();let one=tone_pixels(&r,&first);assert_eq!(one.as_ref(),[245,235,225,0,185,175,165,128]);assert!(r.assets.surface(ClientId(999),&first).is_none());std::fs::remove_file(t.path().join("assets/dat/curve.tcc")).unwrap();r.tick(100,InputFrame::default()).unwrap();while r.host.engine.ui.title.as_ref().unwrap().1!="tone two ready"{r.tick(0,InputFrame::default()).unwrap();assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(1));}let layer=serde_json::to_value(r.host.engine.layers.get(ClientId(1),1_004_112).unwrap()).unwrap();let second=layer["paint"]["asset"].as_str().unwrap();assert_ne!(second,first);let two=tone_pixels(&r,second);assert_eq!(two.as_ref(),[95,65,75,0,19,5,15,128]);assert_eq!(tone_pixels(&r,&first).as_ref(),one.as_ref());assert_eq!(r.inspect()["sessions"][0]["native"]["tone_images"]["ready"],2);
 let inspect=r.inspect();let ops=inspect["sessions"][0]["native"]["opcodes"].as_object().unwrap();let count:u64=ops.iter().filter(|(k,_)|k.contains("71")&&k.contains("1000")).map(|(_,v)|v.as_u64().unwrap()).sum();assert_eq!(count,2);
}
#[test]
fn native_tone_invalid_tcc_keeps_original_command_pending_without_publication(){
 use std::time::{Duration,Instant};let mut code=command(71,1000,&[encode::int(0),encode::quoted(b"base?010"),encode::int(1)]);code.extend(command(4,0,&[encode::quoted(b"must not execute")]));let(t,mut r)=fixture(code);tone_fixture_assets(&t,false);let end=Instant::now()+Duration::from_secs(10);loop{match r.tick(0,InputFrame::default()){Err(e)=>{assert!(format!("{e:#}").contains("TCC version"));break},Ok(())=>{assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(1));}}}assert!(r.host.engine.ui.title.is_none());assert_eq!(r.inspect()["sessions"][0]["native"]["instructions"],1);assert!(r.host.engine.layers.get(ClientId(1),1_000_000).is_none());assert_eq!(r.inspect()["sessions"][0]["native"]["tone_images"]["ready"],0);
}

fn tone_pixels(r:&Runtime,path:&str)->std::sync::Arc<[u8]>{if let Some(s)=r.assets.surface(ClientId(1),path){return s.rgba.clone()}let bytes=r.assets.generated_for(ClientId(1),path).unwrap().unwrap();r.assets.decode_image(ClientId(1),path,&bytes).unwrap().rgba.into()}

#[test]
fn native_empty_object_clip_draws_nothing_and_can_be_restored() {
    let mut code = command(71, 1000, &[encode::int(0), encode::quoted(b"clip"), encode::int(1)]);
    let clip = |w| command(81, 1034, &[encode::int(0), encode::int(0), encode::int(0), encode::int(w), encode::int(1)]);
    for (width, title) in [(0, "empty"), (2, "visible"), (-1, "empty again")] {
        code.extend(clip(width));
        code.extend(command(4, 0, &[encode::quoted(title.as_bytes())]));
        code.extend(command(4, 100, &[encode::int(100)]));
    }
    let (root, mut runtime) = fixture(code);
    std::fs::create_dir(root.path().join("assets/PDT")).unwrap();
    let image = crysirror_image::ImageSurface {width:2,height:1,rgba:vec![255,0,0,255,0,255,0,255]};
    std::fs::write(root.path().join("assets/PDT/clip.png"), crysirror_image::encode_png(&image).unwrap()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while runtime.host.engine.ui.title.is_none() {
        runtime.tick(0, InputFrame::default()).unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(runtime.host.engine.ui.title.as_ref().unwrap().1, "empty");
    assert!(runtime.host.engine.layers.get(ClientId(1), 1_000_000).is_none());
    runtime.tick(100, InputFrame::default()).unwrap();
    assert_eq!(runtime.host.engine.ui.title.as_ref().unwrap().1, "visible");
    assert!(runtime.host.engine.layers.get(ClientId(1), 1_000_000).is_some());
    assert!(runtime.host.engine.layers.frame([640,480]).unwrap().iter().any(|p| matches!(&p.paint,crysirror_scene::layer::Paint::Image{asset,..} if asset.ends_with("clip.png"))));
    runtime.tick(100, InputFrame::default()).unwrap();
    assert_eq!(runtime.host.engine.ui.title.as_ref().unwrap().1, "empty again");
    assert!(runtime.host.engine.layers.get(ClientId(1), 1_000_000).is_none());
    assert_eq!(runtime.inspect()["sessions"][0]["native"]["error"], serde_json::Value::Null);
}
