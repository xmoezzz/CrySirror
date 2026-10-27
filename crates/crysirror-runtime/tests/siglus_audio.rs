// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored native Scene.scn bytes call shared audio services; no old mixer.
use crysirror_core::{
    service::{AudioParameters, AudioRequest, HostServices, InputFrame, PcmAudio},
    state::ClientId,
};
use crysirror_runtime::Runtime;
use crysirror_siglus_vm::{
    fixture::SceneBuilder,
    runtime::{
        constants::{cd, fm, op},
        forms::codes as c,
    },
};
use serde_json::{Value as Json, json};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
fn wave() -> Vec<u8> {
    let size = 2000u32;
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
    for i in 0..1000i16 {
        b.extend((i * 20).to_le_bytes());
    }
    b
}
fn fixture(scene: SceneBuilder, end: u32) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets/a")).unwrap();
    std::fs::write(
        dir.path().join("assets/a/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&scene.finish()),
    )
    .unwrap();
    std::fs::write(dir.path().join("assets/a/tone.wav"), wave()).unwrap();
    std::fs::write(
        dir.path().join("assets/a/Gameexe.ini"),
        format!("#BGM.000=\"Song\",\"native-music\",7,{end},20\n"),
    )
    .unwrap();
    let project = json!({"schema":2,"name":"native audio","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"siglus","game":"authored","asset_prefix":"a","config":{"scene":"Scene.pck","format":"pck","string_codec":"plain","gameexe":"Gameexe.ini","resources":{"native-music":"tone.wav"}}},{"name":"b","adapter":"crysirror-script-v1","game":"other","program":[]}]});
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let runtime = Runtime::load(&path, &dir.path().join("state")).unwrap();
    (dir, runtime)
}
fn bgm(s: &mut SceneBuilder, operation: i32, args: &[i32], ret: i32) {
    s.command(&[c::ELM_GLOBAL_BGM, operation], 0, args, ret);
}
fn play(s: &mut SceneBuilder, operation: i32) {
    s.command_string(&[c::ELM_GLOBAL_BGM, operation], 0, " SONG ", &[], fm::VOID);
}
fn finish(s: &mut SceneBuilder) {
    s.int(6);
    s.int(7);
    s.code.push(cd::OPERATE_2);
    s.i32(fm::INT);
    s.i32(fm::INT);
    s.code.push(op::MULTIPLE);
    s.code.push(cd::EOF);
}
fn native(r: &Runtime) -> Json {
    r.inspect()["sessions"][0]["native"].clone()
}
fn tick(r: &mut Runtime, ms: u64) {
    r.tick(ms, InputFrame::default()).unwrap();
}
fn until(r: &mut Runtime, mut done: impl FnMut(&Runtime) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        tick(r, 0);
        if done(r) {
            return;
        }
        assert!(Instant::now() < end, "{}", native(r));
        std::thread::sleep(Duration::from_millis(1));
    }
}
struct Gate(Arc<(Mutex<bool>, Condvar)>, Vec<crysirror_tasks::Task<()>>);
impl Gate {
    fn block(pool: crysirror_tasks::TaskPool) -> Self {
        let state = Arc::new((Mutex::new(false), Condvar::new()));
        let mut jobs = vec![];
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..pool.stats().workers {
            let s = state.clone();
            let tx = tx.clone();
            jobs.push(
                pool.spawn(move |_| {
                    tx.send(())?;
                    let (v, _) = s
                        .1
                        .wait_timeout_while(s.0.lock().unwrap(), Duration::from_secs(10), |v| !*v)
                        .unwrap();
                    anyhow::ensure!(*v, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..jobs.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self(state, jobs)
    }
    fn release(mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
        for j in self.1.drain(..) {
            j.wait().unwrap();
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}

#[test]
fn native_play_wait_preserves_command_during_workers_and_waits_for_source_not_effect_tails() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY_WAIT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 61);
    r.set_audio_nonblocking(true);
    let gate = Gate::block(r.assets.io_tasks());
    r.host
        .submit_audio(
            ClientId(2),
            AudioRequest::PlayPcm {
                channel: 0,
                source: PcmAudio::new(48000, vec![[0.25; 2]; 4800]).unwrap(),
                parameters: AudioParameters::default(),
            },
        )
        .unwrap();
    tick(&mut r, 0);
    let pc = native(&r)["pc"].clone();
    let checkpoint = r.snapshot().unwrap();
    for _ in 0..3 {
        tick(&mut r, 1);
        assert_eq!(native(&r)["pc"], pc);
        assert_ne!(native(&r)["int_stack"], json!([42]));
        assert!(r.host.audio_info(ClientId(1), 0).unwrap().is_none());
    }
    assert!(
        r.host
            .audio_info(ClientId(2), 0)
            .unwrap()
            .unwrap()
            .position_frames
            > 0
    );
    gate.release();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 0).unwrap().is_some()
    });
    assert_eq!(
        r.host
            .audio_info(ClientId(1), 0)
            .unwrap()
            .unwrap()
            .position_frames,
        7
    );
    assert_ne!(native(&r)["int_stack"], json!([42]));
    let scene=crysirror_acoustics::AcousticScene::build(serde_json::from_value(json!({"materials":[{"absorption":[0,0,0],"transmission":[0,0,0],"scattering":0}],"meshes":[{"id":1,"vertices":[[-50,-50,-12],[50,-50,-12],[50,50,-12],[-50,50,-12]],"triangles":[[0,1,2],[0,2,3]]}]})).unwrap()).unwrap();
    let query = serde_json::from_value(json!({"source":[0,0,2],"listener":[0,0,0]})).unwrap();
    let paths = crysirror_acoustics::reflections::simulate(&scene, &query).unwrap();
    r.host
        .channel_audio
        .set_reflections(
            1,
            0,
            Some(
                crysirror_acoustics::reflection_dsp::EarlyReflections::prepare(48000, paths, 0)
                    .unwrap(),
            ),
        )
        .unwrap();
    tick(&mut r, 2);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
    assert!(
        !r.host
            .audio_info(ClientId(1), 0)
            .unwrap()
            .unwrap()
            .source_playing
    );
    tick(&mut r, 80);
    assert!(
        r.host
            .channel_audio
            .take_output()
            .iter()
            .flatten()
            .any(|s| s.abs() > 0.00001)
    );
    // Pending worker receipts were absent from the checkpoint; restoring replays
    // the original uncommitted instruction through the same engine resource API.
    r.restore(&checkpoint).unwrap();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 0).unwrap().is_some()
    });
    tick(&mut r, 2);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}
#[test]
fn native_ready_delayed_resume_pause_fade_and_resume_wait_use_engine_transport() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_READY_ONESHOT);
    bgm(&mut s, c::ELM_BGM_CHECK, &[], fm::INT);
    s.element(&[c::ELM_GLOBAL_BGM, c::ELM_BGM_RESUME]);
    s.int(0);
    s.int(2);
    s.code.push(cd::COMMAND);
    for n in [0, 2, fm::INT, fm::INT, 1, 0, fm::VOID] {
        s.i32(n);
    }
    bgm(&mut s, c::ELM_BGM_CHECK, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[4], fm::VOID);
    bgm(&mut s, c::ELM_BGM_PAUSE, &[2], fm::VOID);
    bgm(&mut s, c::ELM_BGM_CHECK, &[], fm::INT);
    bgm(&mut s, c::ELM_BGM_WAIT_FADE, &[], fm::VOID);
    bgm(&mut s, c::ELM_BGM_CHECK, &[], fm::INT);
    bgm(&mut s, c::ELM_BGM_GET_PLAY_POS, &[], fm::INT);
    bgm(&mut s, c::ELM_BGM_RESUME_WAIT, &[0], fm::VOID);
    bgm(&mut s, c::ELM_BGM_CHECK, &[], fm::INT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 500);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([3, 3]));
    tick(&mut r, 2);
    let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert_eq!(info.position_frames, 7);
    assert!(!info.paused);
    tick(&mut r, 2);
    let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert!(info.paused);
    assert_eq!(info.position_frames, 103);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([3, 3, 2, 3, 103]));
    tick(&mut r, 10);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([3, 3, 2, 3, 103, 0, 42]));
}
#[test]
fn failed_audio_keeps_original_pc_and_repaired_resource_can_retry_without_stack_loss() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY_ONESHOT);
    finish(&mut s);
    let (dir, mut r) = fixture(s, 61);
    std::fs::write(dir.path().join("assets/a/tone.wav"), b"broken").unwrap();
    r.set_audio_nonblocking(true);
    tick(&mut r, 0);
    let pc = native(&r)["pc"].clone();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if r.tick(0, InputFrame::default()).is_err() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(native(&r)["pc"], pc);
    assert_ne!(native(&r)["int_stack"], json!([42]));
    std::fs::write(dir.path().join("assets/a/tone.wav"), wave()).unwrap();
    until(&mut r, |r| native(r)["halted"] == true);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}
#[test]
fn same_looping_name_is_idempotent_and_active_checkpoint_rebinds_shared_audio() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[2], fm::VOID);
    play(&mut s, c::ELM_BGM_PLAY);
    s.command(
        &[c::ELM_GLOBAL_BGMTABLE, c::ELM_BGMTABLE_GET_BGM_CNT],
        0,
        &[],
        fm::INT,
    );
    s.command_string(
        &[c::ELM_GLOBAL_BGMTABLE, c::ELM_BGMTABLE_GET_LISTEN_BY_NAME],
        0,
        "song",
        &[],
        fm::INT,
    );
    finish(&mut s);
    let (dir, mut r) = fixture(s, 61);
    tick(&mut r, 0);
    let saved = r.snapshot().unwrap();
    std::fs::remove_file(dir.path().join("assets/a/tone.wav")).unwrap();
    tick(&mut r, 2);
    assert_eq!(native(&r)["int_stack"], json!([1, 1, 42]));
    assert!(r.host.audio_info(ClientId(1), 0).unwrap().unwrap().looping);
    std::fs::write(dir.path().join("assets/a/tone.wav"), wave()).unwrap();
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert_eq!(
        r.host
            .audio_info(ClientId(1), 0)
            .unwrap()
            .unwrap()
            .position_frames,
        7
    );
    tick(&mut r, 2);
    assert_eq!(native(&r)["int_stack"], json!([1, 1, 42]));
}
#[test]
fn unknown_audio_opcode_fails_instead_of_becoming_a_fake_property() {
    let mut s = SceneBuilder::default();
    bgm(&mut s, 9876, &[], fm::VOID);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 61);
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("unsupported Siglus BGM opcode"), "{error}");
    let mut s = SceneBuilder::default();
    s.command(
        &[c::ELM_GLOBAL_BGMTABLE, c::ELM_BGMTABLE_GET_BGM_CNT],
        0,
        &[],
        fm::INT,
    );
    let (dir, mut r) = fixture(s, 61);
    std::fs::write(
        dir.path().join("assets/a/Gameexe.ini"),
        "#BGM.CNT=999999999\n",
    )
    .unwrap();
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("exceeds 4096"), "{error}");
}

#[test]
fn original_save_codec_restores_bgm_declaration_through_shared_preparation() {
    for paused in [false, true] {
        let mut s = SceneBuilder::default();
        play(
            &mut s,
            if paused {
                c::ELM_BGM_READY
            } else {
                c::ELM_BGM_PLAY
            },
        );
        bgm(&mut s, c::ELM_BGM_SET_VOLUME, &[128], fm::VOID);
        s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
        s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10000], fm::VOID);
        finish(&mut s);
        let (_dir, mut r) = fixture(s, 61);
        tick(&mut r, 0);
        let saved = r.export_original_save(ClientId(1)).unwrap();
        assert!(saved.len() > 128);
        tick(&mut r, 2);
        let pcm = r.host.channel_audio.take_output();
        let expected = if paused {
            0.
        } else {
            140. / 32768. * 128. / 255.
        };
        assert!((pcm[0][0] - expected).abs() < 1e-6);
        let before = native(&r);
        assert!(
            r.import_original_save(ClientId(1), b"invalid original save")
                .is_err()
        );
        assert_eq!(native(&r), before);
        r.import_original_save(ClientId(1), &saved).unwrap();
        tick(&mut r, 0);
        let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
        assert!(info.looping);
        assert_eq!(info.position_frames, 7);
        assert_eq!(native(&r)["bgm"]["name"], "song");
        assert_eq!(native(&r)["bgm"]["state"], if paused { 3 } else { 1 });
        assert_eq!(native(&r)["bgm"]["volume"], 128);
        for _ in 0..10 {
            tick(&mut r, 1000);
        }
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([1, 42]));
    }
}

#[test]
fn native_audio_wait_key_returns_decide_or_natural_value_without_stopping_a_loop() {
    for looping in [false, true] {
        let mut s = SceneBuilder::default();
        play(
            &mut s,
            if looping {
                c::ELM_BGM_PLAY
            } else {
                c::ELM_BGM_PLAY_ONESHOT
            },
        );
        bgm(&mut s, c::ELM_BGM_WAIT_KEY, &[], fm::INT);
        finish(&mut s);
        let (_dir, mut r) = fixture(s, 61);
        tick(&mut r, 0);
        if looping {
            r.tick(
                0,
                InputFrame {
                    buttons_down: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_ne!(native(&r)["int_stack"], json!([1, 42]));
            r.tick(0, InputFrame::default()).unwrap();
            assert_eq!(native(&r)["int_stack"], json!([1, 42]));
            assert!(
                r.host
                    .audio_info(ClientId(1), 0)
                    .unwrap()
                    .unwrap()
                    .source_playing
            );
        } else {
            tick(&mut r, 2);
            tick(&mut r, 0);
            assert_eq!(native(&r)["int_stack"], json!([0, 42]));
        }
    }
}

#[test]
fn stopped_and_eof_resume_reuse_engine_resource_after_original_file_is_removed() {
    for stop in [false, true] {
        let mut s = SceneBuilder::default();
        play(&mut s, c::ELM_BGM_PLAY_ONESHOT);
        s.command(
            &[c::ELM_GLOBAL_TIMEWAIT],
            0,
            &[if stop { 2 } else { 4 }],
            fm::VOID,
        );
        if stop {
            bgm(&mut s, c::ELM_BGM_STOP, &[0], fm::VOID);
        }
        bgm(&mut s, c::ELM_BGM_RESUME_WAIT, &[0], fm::VOID);
        finish(&mut s);
        let (dir, mut r) = fixture(s, 61);
        tick(&mut r, 0);
        assert_eq!(r.host.channel_audio.resources.resource_count(), 1);
        std::fs::remove_file(dir.path().join("assets/a/tone.wav")).unwrap();
        if !stop {
            tick(&mut r, 2); // the first source reaches EOF before RESUME executes
            assert!(!r.host.channel_audio.is_playing(1, 0));
        }
        tick(&mut r, 2); // resumed source starts at seven and finishes in this mix
        assert_eq!(r.host.channel_audio.resources.resource_count(), 1);
        let out = r.host.channel_audio.take_output();
        assert!((out[0][0] - 140. / 32768.).abs() < 1e-6);
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([42]));
    }
}
#[test]
fn stopped_snapshot_rebinds_resource_without_hidden_playback_and_can_resume() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY);
    bgm(&mut s, c::ELM_BGM_STOP, &[0], fm::VOID);
    s.command(
        &[c::ELM_GLOBAL_BGMTABLE, c::ELM_BGMTABLE_SET_ALL_FLAG],
        0,
        &[0],
        fm::VOID,
    );
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10], fm::VOID);
    bgm(&mut s, c::ELM_BGM_RESUME, &[0], fm::VOID);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[1000], fm::VOID);
    finish(&mut s);
    let (dir, mut r) = fixture(s, 61);
    tick(&mut r, 0);
    assert_eq!(native(&r)["bgm"]["state"], 0);
    let saved = r.snapshot().unwrap();
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert_eq!(r.host.channel_audio.resources.resource_count(), 1);
    assert!(!r.host.channel_audio.is_playing(1, 0));
    assert_eq!(native(&r)["bgm"]["listened"], json!([false]));
    std::fs::remove_file(dir.path().join("assets/a/tone.wav")).unwrap();
    tick(&mut r, 10);
    let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert_eq!(info.position_frames, 36); // 480 output frames, then the [20,61) loop
    assert!(info.looping);
    // Original STOP clears the outer loop/name flags, while its prepared slot
    // retains the actual loop region. RESUME must not invent a new name.
    assert_eq!(native(&r)["bgm"]["looping"], false);
    assert!(native(&r)["bgm"]["name"].is_null());
    assert!((r.host.channel_audio.take_output()[0][0] - 140. / 32768.).abs() < 1e-6);
}

#[test]
fn ended_named_slot_snapshot_prepares_without_restarting_and_resume_keeps_its_start() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY_ONESHOT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10], fm::VOID);
    bgm(&mut s, c::ELM_BGM_RESUME_WAIT, &[0], fm::VOID);
    finish(&mut s);
    let (dir, mut r) = fixture(s, 61);
    tick(&mut r, 0);
    tick(&mut r, 2);
    tick(&mut r, 0);
    assert_eq!(native(&r)["bgm"]["state"], 0);
    assert_eq!(native(&r)["bgm"]["name"], "song");
    let saved = r.snapshot().unwrap();
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert_eq!(r.host.channel_audio.resources.resource_count(), 1);
    assert!(!r.host.channel_audio.is_playing(1, 0));
    std::fs::remove_file(dir.path().join("assets/a/tone.wav")).unwrap();
    tick(&mut r, 8);
    assert!((r.host.channel_audio.take_output()[0][0] - 140. / 32768.).abs() < 1e-6);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}

#[test]
fn failed_voice_stage_releases_new_resource_and_preserves_the_previous_prepared_slot() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY);
    s.command_string(
        &[c::ELM_GLOBAL_BGM, c::ELM_BGM_PLAY_ONESHOT],
        0,
        "bad",
        &[],
        fm::VOID,
    );
    finish(&mut s);
    let (dir, mut r) = fixture(s, 61);
    std::fs::write(
        dir.path().join("assets/a/Gameexe.ini"),
        "#BGM.000=\"Song\",\"native-music\",7,61,20\n#BGM.001=\"bad\",\"native-music\",61,61,20\n",
    )
    .unwrap();
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("range"), "{error}");
    assert_eq!(r.host.channel_audio.resources.resource_count(), 1);
    assert_eq!(native(&r)["bgm"]["name"], "song");
    assert_eq!(native(&r)["bgm"]["listened"], json!([true, false]));
    let info = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert!(info.source_playing);
    assert!(info.looping);
    assert_eq!(info.position_frames, 7);
}

fn se(s: &mut SceneBuilder, operation: i32, args: &[i32], ret: i32) {
    s.command(&[c::ELM_GLOBAL_SE, operation], 0, args, ret);
}
fn sound_file(s: &mut SceneBuilder, global: i32, operation: i32) {
    s.command_string(&[global, operation], 0, "native-music", &[], fm::VOID);
}
#[test]
fn native_pcm_and_se_mix_independently_and_category_fade_preserves_the_voice_envelope() {
    let mut s = SceneBuilder::default();
    sound_file(&mut s, c::ELM_GLOBAL_PCM, c::ELM_PCM_PLAY);
    se(&mut s, c::ELM_SE_SET_VOLUME, &[128], fm::VOID);
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    se(&mut s, c::ELM_SE_GET_VOLUME, &[], fm::INT);
    se(&mut s, c::ELM_SE_CHECK, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
    se(&mut s, c::ELM_SE_SET_VOLUME_MIN, &[1], fm::VOID);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
    se(&mut s, c::ELM_SE_CHECK, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_PCM, c::ELM_PCM_STOP], 0, &[], fm::VOID);
    se(&mut s, c::ELM_SE_STOP, &[0], fm::VOID);
    se(&mut s, c::ELM_SE_CHECK, &[], fm::INT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([128, 1]));
    tick(&mut r, 1);
    let pcm = r.host.channel_audio.take_output();
    assert_eq!(pcm.len(), 48);
    for (i, s) in pcm.iter().enumerate() {
        let input = i as f32 * 20. / 32768.;
        let expected = input * (1. + 128. / 255. * (1. - i as f32 / 48.));
        assert!(
            (s[0] - expected).abs() < 1e-6,
            "{i}: {} != {expected}",
            s[0]
        );
    }
    let group = r
        .host
        .audio_mix_group(ClientId(1), "siglus.se")
        .unwrap()
        .unwrap();
    assert_eq!(group.fade_frames, 0);
    assert!(group.gain.abs() < 1e-6);
    tick(&mut r, 1);
    assert_eq!(native(&r)["int_stack"], json!([128, 1, 1, 0, 42]));
    assert!(
        r.host
            .channel_audio
            .take_output()
            .iter()
            .flatten()
            .all(|x| *x == 0.)
    );
}
#[test]
fn native_sfx_worker_gate_preserves_original_instruction_and_pending_snapshot() {
    let mut s = SceneBuilder::default();
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    se(&mut s, c::ELM_SE_WAIT, &[], fm::INT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    r.set_audio_nonblocking(true);
    let gate = Gate::block(r.assets.io_tasks());
    tick(&mut r, 0);
    let pc = native(&r)["pc"].clone();
    let saved = r.snapshot().unwrap();
    for _ in 0..3 {
        tick(&mut r, 1);
        assert_eq!(native(&r)["pc"], pc);
        assert_ne!(native(&r)["int_stack"], json!([0, 42]));
        assert!(r.host.audio_info(ClientId(1), 2).unwrap().is_none());
    }
    gate.release();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 2).unwrap().is_some()
    });
    assert_eq!(native(&r)["int_stack"], json!([0]));
    tick(&mut r, 25);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    r.restore(&saved).unwrap();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 2).unwrap().is_some()
    });
    tick(&mut r, 25);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
}
#[test]
fn native_numbered_se_and_active_snapshot_rebind_use_explicit_table_and_shared_groups() {
    let mut s = SceneBuilder::default();
    se(&mut s, c::ELM_SE_SET_VOLUME, &[64], fm::VOID);
    se(&mut s, c::ELM_SE_PLAY_BY_SE_NO, &[0], fm::VOID);
    se(&mut s, c::ELM_SE_WAIT, &[], fm::INT);
    se(&mut s, c::ELM_SE_CHECK, &[], fm::INT);
    finish(&mut s);
    let (dir, mut r) = fixture(s, 1000);
    std::fs::write(
        dir.path().join("assets/a/Gameexe.ini"),
        "#SE.CNT=16\n#SE.000=\"native-music\"\n",
    )
    .unwrap();
    tick(&mut r, 0);
    let saved = r.snapshot().unwrap();
    assert_eq!(native(&r)["int_stack"], json!([0]));
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert!(
        r.host
            .audio_info(ClientId(1), 2)
            .unwrap()
            .unwrap()
            .source_playing
    );
    let group = r
        .host
        .audio_mix_group(ClientId(1), "siglus.se")
        .unwrap()
        .unwrap();
    assert!((group.gain - 64. / 255.).abs() < 1e-6);
    tick(&mut r, 25);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 0, 42]));
}
#[test]
fn native_original_import_stops_nonpersistent_pcm_and_se_before_restoring_group_volumes() {
    let mut s = SceneBuilder::default();
    se(&mut s, c::ELM_SE_SET_VOLUME, &[128], fm::VOID);
    sound_file(&mut s, c::ELM_GLOBAL_PCM, c::ELM_PCM_PLAY);
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10000], fm::VOID);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    tick(&mut r, 0);
    let saved = r.export_original_save(ClientId(1)).unwrap();
    assert!(r.host.audio_info(ClientId(1), 1).unwrap().is_some());
    assert!(r.host.audio_info(ClientId(1), 2).unwrap().is_some());
    r.import_original_save(ClientId(1), &saved).unwrap();
    tick(&mut r, 0);
    assert!(!r.host.channel_audio.is_playing(1, 1));
    assert!(!r.host.channel_audio.is_playing(1, 2));
    assert!(
        (r.host
            .audio_mix_group(ClientId(1), "siglus.se")
            .unwrap()
            .unwrap()
            .gain
            - 128. / 255.)
            .abs()
            < 1e-6
    );
}
#[test]
fn unsupported_sfx_and_missing_table_or_alias_fail_visibly_without_following_arithmetic() {
    for (op, args) in [
        (9876, vec![]),
        (c::ELM_SE_PLAY, vec![0]),
        (c::ELM_SE_PLAY_BY_KOE_NO, vec![7]),
    ] {
        let mut s = SceneBuilder::default();
        se(&mut s, op, &args, fm::VOID);
        finish(&mut s);
        let (_dir, mut r) = fixture(s, 1000);
        assert!(r.tick(0, InputFrame::default()).is_err());
        assert_ne!(native(&r)["int_stack"], json!([42]));
        assert!(r.host.audio_info(ClientId(1), 2).unwrap().is_none());
    }
}

#[test]
fn native_sound_wait_uses_integer_millisecond_age_while_last_pcm_frames_still_play() {
    let mut s = SceneBuilder::default();
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    se(&mut s, c::ELM_SE_WAIT, &[], fm::INT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    tick(&mut r, 0);
    tick(&mut r, 20);
    let info = r.host.audio_info(ClientId(1), 2).unwrap().unwrap();
    assert!(info.source_playing);
    assert_eq!(info.age_frames, 960);
    assert_eq!(info.position_frames, 960);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    assert!(
        r.host
            .audio_info(ClientId(1), 2)
            .unwrap()
            .unwrap()
            .source_playing
    );
    assert_eq!(r.host.channel_audio.resources.resource_count(), 0);
}
#[test]
fn failed_sfx_resource_keeps_old_source_and_original_command_then_retries_after_repair() {
    let mut s = SceneBuilder::default();
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[2], fm::VOID);
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    finish(&mut s);
    let (dir, mut r) = fixture(s, 1000);
    tick(&mut r, 0);
    std::fs::write(dir.path().join("assets/a/tone.wav"), b"broken").unwrap();
    r.set_audio_nonblocking(true);
    tick(&mut r, 2);
    let pc = native(&r)["pc"].clone();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if r.tick(0, InputFrame::default()).is_err() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(native(&r)["pc"], pc);
    assert_ne!(native(&r)["int_stack"], json!([42]));
    assert!(
        r.host
            .audio_info(ClientId(1), 2)
            .unwrap()
            .unwrap()
            .source_playing
    );
    assert_eq!(r.host.channel_audio.resources.resource_count(), 0);
    std::fs::write(dir.path().join("assets/a/tone.wav"), wave()).unwrap();
    until(&mut r, |r| native(r)["halted"] == true);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}
#[test]
fn se_voice_number_uses_explicit_engine_alias_and_wait_key_keeps_native_zero_return() {
    let mut s = SceneBuilder::default();
    se(&mut s, c::ELM_SE_PLAY_BY_KOE_NO, &[7], fm::VOID);
    se(&mut s, c::ELM_SE_WAIT_KEY, &[], fm::INT);
    finish(&mut s);
    let (dir, initial) = fixture(s, 1000);
    drop(initial);
    let path = dir.path().join("project.json");
    let mut project: Json = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    project["clients"][0]["config"]["resources"]["koe/7"] = json!("tone.wav");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut r = Runtime::load(&path, &dir.path().join("state")).unwrap();
    tick(&mut r, 0);
    r.tick(
        0,
        InputFrame {
            buttons_down: 1,
            ..Default::default()
        },
    )
    .unwrap();
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    assert!(
        r.host
            .audio_info(ClientId(1), 2)
            .unwrap()
            .unwrap()
            .source_playing
    );
}

#[test]
fn original_import_preserves_cross_category_stop_order_before_blocked_bgm_reprepare() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY);
    sound_file(&mut s, c::ELM_GLOBAL_PCM, c::ELM_PCM_PLAY);
    sound_file(&mut s, c::ELM_GLOBAL_SE, c::ELM_SE_PLAY_BY_FILE_NAME);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10000], fm::VOID);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    tick(&mut r, 0);
    let saved = r.export_original_save(ClientId(1)).unwrap();
    assert!(r.host.channel_audio.is_playing(1, 1));
    assert!(r.host.channel_audio.is_playing(1, 2));
    r.set_audio_nonblocking(true);
    let gate = Gate::block(r.assets.io_tasks());
    r.import_original_save(ClientId(1), &saved).unwrap();
    tick(&mut r, 0);
    assert!(!r.host.channel_audio.is_playing(1, 0));
    assert!(!r.host.channel_audio.is_playing(1, 1));
    assert!(!r.host.channel_audio.is_playing(1, 2));
    assert_eq!(native(&r)["audio_preparing"], true);
    gate.release();
    until(&mut r, |r| {
        r.host.channel_audio.is_playing(1, 0) && native(r)["audio_preparing"] == false
    });
    assert!(!r.host.channel_audio.is_playing(1, 1));
    assert!(!r.host.channel_audio.is_playing(1, 2));
}

#[test]
fn original_wait_and_restore_use_worker_mix_publication_without_repeating_native_tick() {
    let mut s = SceneBuilder::default();
    play(&mut s, c::ELM_BGM_PLAY_WAIT);
    finish(&mut s);
    let (_dir, mut r) = fixture(s, 1000);
    r.set_audio_nonblocking(true);
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 0).unwrap().is_some()
    });
    let save = r.snapshot().unwrap();
    let before = r.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    let gate = Gate::block(r.assets.cpu_tasks());
    r.begin_tick(5, InputFrame::default()).unwrap();
    assert!(r.audio_frame_pending());
    assert!(r.host.channel_audio.mixer.is_rendering());
    let diagnostic = native(&r);
    for _ in 0..3 {
        assert!(!r.poll_audio_frame().unwrap());
        assert_eq!(native(&r), diagnostic);
        assert_eq!(
            r.host
                .audio_info(ClientId(1), 0)
                .unwrap()
                .unwrap()
                .age_frames,
            before.age_frames
        );
        assert!(r.tick(5, InputFrame::default()).is_err());
        assert_eq!(r.now_ms(), 5);
    }
    gate.release();
    let end = Instant::now() + Duration::from_secs(5);
    while !r.poll_audio_frame().unwrap() {
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    assert_eq!(
        r.host
            .audio_info(ClientId(1), 0)
            .unwrap()
            .unwrap()
            .age_frames,
        before.age_frames + 240
    );
    assert_eq!(r.host.take_pcm_output().len(), 240);
    assert!(!r.all_halted());
    let gate = Gate::block(r.assets.cpu_tasks());
    r.begin_tick(5, InputFrame::default()).unwrap();
    assert!(r.audio_frame_pending());
    r.restore(&save).unwrap();
    assert!(r.poll_audio_frame().unwrap());
    assert!(r.host.audio_info(ClientId(1), 0).unwrap().is_none());
    assert!(r.host.take_pcm_output().is_empty());
    gate.release();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 0).unwrap().is_some()
    });
    for _ in 0..100 {
        r.begin_tick(1, InputFrame::default()).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while !r.poll_audio_frame().unwrap() {
            assert!(Instant::now() < end);
            std::thread::yield_now();
        }
        if r.all_halted() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(r.all_halted(), "{}", native(&r));
    assert_eq!(native(&r)["int_stack"], json!([42]));
}

fn voice(s: &mut SceneBuilder, code: i32, args: &[i32], ret: i32) {
    s.command(&[code], 0, args, ret);
}
#[test]
fn original_koe_wait_uses_shared_resource_voice_bus_and_source_completion() {
    let mut s = SceneBuilder::default();
    voice(&mut s, c::ELM_GLOBAL_KOE_PLAY_WAIT, &[12, 7], fm::VOID);
    finish(&mut s);
    let (dir, _) = fixture(s, 61);
    let path = dir.path().join("project.json");
    let mut project: Json = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    project["clients"][0]["config"]["resources"]["koe/12"] = json!("tone.wav");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut r = Runtime::load(&path, &dir.path().join("state")).unwrap();
    r.set_audio_nonblocking(true);
    let gate = Gate::block(r.assets.io_tasks());
    tick(&mut r, 0);
    assert_ne!(native(&r)["int_stack"], json!([42]));
    assert!(r.host.audio_info(ClientId(1), 3).unwrap().is_none());
    gate.release();
    until(&mut r, |r| {
        r.host.audio_info(ClientId(1), 3).unwrap().is_some()
    });
    tick(&mut r, 0);
    assert_eq!(native(&r)["koe"]["number"], json!(12));
    assert_eq!(native(&r)["koe"]["cached"], json!(1));
    r.host
        .channel_audio
        .mixer
        .set_bus(
            1,
            crysirror_audio::mixer::Bus::Voice,
            crysirror_audio::mixer::BusSettings {
                gain: 1.,
                muted: true,
            },
        )
        .unwrap();
    tick(&mut r, 1);
    assert!(r.host.take_pcm_output().iter().flatten().all(|s| *s == 0.));
    r.host
        .channel_audio
        .mixer
        .set_bus(
            1,
            crysirror_audio::mixer::Bus::Voice,
            crysirror_audio::mixer::BusSettings {
                gain: 1.,
                muted: false,
            },
        )
        .unwrap();
    tick(&mut r, 1);
    assert!(r.host.take_pcm_output().iter().flatten().any(|s| *s != 0.));
    tick(&mut r, 21);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}
#[test]
fn original_koe_jitan_config_prepares_transformed_resource_and_cache_replays_without_file() {
    let mut s = SceneBuilder::default();
    s.command(
        &[c::ELM_GLOBAL_SYSCOM, c::syscom_op::SET_JITAN_SPEED],
        0,
        &[200],
        fm::VOID,
    );
    s.command(
        &[c::ELM_GLOBAL_SYSCOM, c::syscom_op::SET_JITAN_NORMAL_ONOFF],
        0,
        &[1],
        fm::VOID,
    );
    voice(&mut s, c::ELM_GLOBAL_KOE_PLAY_WAIT, &[12], fm::VOID);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[50], fm::VOID);
    voice(&mut s, c::ELM_GLOBAL_KOE_PLAY_WAIT, &[12], fm::VOID);
    finish(&mut s);
    let (dir, _) = fixture(s, 61);
    let path = dir.path().join("project.json");
    let mut p: Json = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    p["clients"][0]["config"]["resources"]["koe/12"] = json!("tone.wav");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut r = Runtime::load(&path, &dir.path().join("state")).unwrap();
    tick(&mut r, 0);
    assert_eq!(native(&r)["koe"]["compression_percent"], json!(200));
    assert_eq!(
        r.host
            .audio_info(ClientId(1), 3)
            .unwrap()
            .unwrap()
            .source_frames,
        Some(1000)
    ); // <100 ms legacy silent cue
    let pcm = {
        tick(&mut r, 21);
        r.host.take_pcm_output()
    };
    assert!(pcm.iter().flatten().all(|s| *s == 0.));
    tick(&mut r, 0);
    std::fs::remove_file(dir.path().join("assets/a/tone.wav")).unwrap();
    tick(&mut r, 51);
    tick(&mut r, 0);
    assert_eq!(native(&r)["koe"]["cached"], json!(1));
    assert_eq!(native(&r)["int_stack"], json!([42]));
}

fn koe_fixture(s: SceneBuilder) -> (tempfile::TempDir, Runtime) {
    let (dir, initial) = fixture(s, 1000);
    drop(initial);
    let path = dir.path().join("project.json");
    let mut p: Json = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for n in [12, 13] {
        p["clients"][0]["config"]["resources"][format!("koe/{n}")] = json!("tone.wav");
    }
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let r = Runtime::load(&path, &dir.path().join("state")).unwrap();
    (dir, r)
}
fn config(s: &mut SceneBuilder, op: i32, args: &[i32]) {
    s.command(&[c::ELM_GLOBAL_SYSCOM, op], 0, args, fm::VOID);
}

#[test]
fn original_koe_long_compression_waits_on_native_integer_ms_boundary() {
    let mut s = SceneBuilder::default();
    config(&mut s, c::syscom_op::SET_JITAN_SPEED, &[200]);
    config(&mut s, c::syscom_op::SET_JITAN_NORMAL_ONOFF, &[1]);
    voice(&mut s, c::ELM_GLOBAL_KOE_PLAY_WAIT, &[12], fm::INT);
    finish(&mut s);
    let (dir, mut r) = koe_fixture(s);
    let pcm = (0..8000usize)
        .map(|i| (((i * 7919 + 12345) % 60001) as i32 - 30000) as i16)
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    std::fs::write(
        dir.path().join("assets/a/tone.wav"),
        crysirror_resource::WaveSpec {
            sample_rate: 8000,
            ..Default::default()
        }
        .encode(&pcm)
        .unwrap(),
    )
    .unwrap();
    tick(&mut r, 0);
    let info = r.host.audio_info(ClientId(1), 3).unwrap().unwrap();
    assert_eq!((info.source_frames, info.source_rate), (Some(4130), 8000));
    tick(&mut r, 515);
    tick(&mut r, 0);
    assert_ne!(native(&r)["int_stack"], json!([0, 42]));
    tick(&mut r, 1);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    assert!(
        r.host
            .audio_info(ClientId(1), 3)
            .unwrap()
            .unwrap()
            .source_playing
    );
}

#[test]
fn original_exkoe_named_policy_keeps_message_identity_and_decide_wait_does_not_stop_voice() {
    let mut s = SceneBuilder::default();
    config(&mut s, c::syscom_op::SET_JITAN_SPEED, &[200]);
    voice(&mut s, c::ELM_GLOBAL_KOE, &[12, 7], fm::VOID);
    // Original CD_COMMAND stores named ids in reverse argument order.
    s.element(&[c::ELM_GLOBAL_EXKOE]);
    for v in [13, 9, 1, 1, 1] {
        s.int(v);
    }
    s.code.push(cd::COMMAND);
    s.i32(0);
    s.i32(5);
    for _ in 0..5 {
        s.i32(fm::INT);
    }
    s.i32(5);
    for id in [4, 3, 2, 1, 0] {
        s.i32(id);
    }
    s.i32(fm::INT);
    finish(&mut s);
    let (_dir, mut r) = koe_fixture(s);
    tick(&mut r, 0);
    let koe = native(&r)["koe"].clone();
    assert_eq!(koe["number"], 13);
    assert_eq!(koe["message_number"], 12);
    assert_eq!(koe["message_character"], 7);
    assert_eq!(koe["compression_percent"], 200);
    assert_eq!(koe["is_ex"], true);
    assert_eq!(koe["cached"], 2);
    // Arrow input is not a completed DECIDE. Space is also DECIDE.
    r.tick(
        0,
        InputFrame {
            buttons_down: 4,
            ..Default::default()
        },
    )
    .unwrap();
    tick(&mut r, 0);
    assert_ne!(native(&r)["int_stack"], json!([1, 42]));
    r.tick(
        0,
        InputFrame {
            buttons_down: 1,
            ..Default::default()
        },
    )
    .unwrap();
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([1, 42]));
    assert!(
        r.host
            .audio_info(ClientId(1), 3)
            .unwrap()
            .unwrap()
            .source_playing
    );
}

#[test]
fn failed_new_koe_stops_previous_source_retains_original_command_and_retries() {
    let mut s = SceneBuilder::default();
    voice(&mut s, c::ELM_GLOBAL_KOE, &[12], fm::VOID);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[2], fm::VOID);
    voice(&mut s, c::ELM_GLOBAL_KOE_PLAY_WAIT, &[13], fm::INT);
    finish(&mut s);
    let (dir, mut r) = koe_fixture(s);
    tick(&mut r, 0);
    assert!(r.host.channel_audio.is_playing(1, 3));
    std::fs::write(dir.path().join("assets/a/tone.wav"), b"broken").unwrap();
    r.set_audio_nonblocking(true);
    tick(&mut r, 2);
    let pc = native(&r)["pc"].clone();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if r.tick(0, InputFrame::default()).is_err() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    assert_eq!(native(&r)["pc"], pc);
    assert!(!r.host.channel_audio.is_playing(1, 3));
    assert_ne!(native(&r)["int_stack"], json!([0, 42]));
    assert_eq!(native(&r)["koe"]["cached"], 1);
    std::fs::write(dir.path().join("assets/a/tone.wav"), wave()).unwrap();
    until(&mut r, |r| native(r)["koe"]["number"] == 13);
    assert!(r.host.channel_audio.is_playing(1, 3));
    tick(&mut r, 21);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
}

#[test]
fn koe_runtime_restore_rebuilds_handles_and_original_save_restores_volume_without_voice() {
    let mut s = SceneBuilder::default();
    voice(&mut s, c::ELM_GLOBAL_KOE_SET_VOLUME, &[128], fm::VOID);
    voice(&mut s, c::ELM_GLOBAL_KOE, &[12, 7], fm::VOID);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10000], fm::VOID);
    finish(&mut s);
    let (_dir, mut r) = koe_fixture(s);
    tick(&mut r, 0);
    let save = r.snapshot().unwrap();
    let original = r.export_original_save(ClientId(1)).unwrap();
    assert_eq!(native(&r)["koe"]["volume"], 128);
    r.restore(&save).unwrap();
    assert!(r.host.audio_info(ClientId(1), 3).unwrap().is_none());
    tick(&mut r, 0);
    assert!(r.host.channel_audio.is_playing(1, 3));
    assert_eq!(native(&r)["koe"]["cached"], 1);
    assert_eq!(native(&r)["koe"]["message_number"], 12);
    let group = r
        .host
        .channel_audio
        .mixer
        .mix_group(1, "siglus.koe")
        .unwrap()
        .unwrap();
    assert!((group.settings.gain - 128. / 255.).abs() < 1e-6);
    r.import_original_save(ClientId(1), &original).unwrap();
    tick(&mut r, 0);
    assert!(!r.host.channel_audio.is_playing(1, 3));
    assert_eq!(native(&r)["koe"]["volume"], 128);
}

#[test]
fn koe_with_trailing_read_flag_fails_before_audio_instead_of_marking_unread_text_read() {
    let mut s = SceneBuilder::default();
    voice(&mut s, c::ELM_GLOBAL_KOE, &[12], fm::VOID);
    s.i32(0);
    finish(&mut s);
    let mut scene = s.finish();
    let flags = scene.len() as i32;
    scene[31 * 4..32 * 4].copy_from_slice(&flags.to_le_bytes());
    scene[32 * 4..33 * 4].copy_from_slice(&1i32.to_le_bytes());
    scene.extend(0i32.to_le_bytes());
    let (dir, mut r) = koe_fixture(SceneBuilder::default());
    std::fs::write(
        dir.path().join("assets/a/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&scene),
    )
    .unwrap();
    for _ in 0..2 {
        let error = format!("{:#}",r.tick(0, InputFrame::default()).unwrap_err());
        assert!(error.contains("MWND/read-skip"), "{error}");
        assert!(r.host.audio_info(ClientId(1), 3).unwrap().is_none());
        assert_eq!(r.host.channel_audio.resources.resource_count(), 0);
        assert_ne!(native(&r)["int_stack"], json!([42]));
    }
}
