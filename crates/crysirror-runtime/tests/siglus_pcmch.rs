// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored original SceneVm ABI; only CrySirror owns decoded sound/transport.
use crysirror_core::{
    service::{HostServices, InputFrame},
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
use serde_json::{Value, json};
use std::time::{Duration, Instant};
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
    for _ in 0..1000 {
        b.extend(8192i16.to_le_bytes());
    }
    b
}
fn fixture(s: SceneBuilder, extra: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("assets/a")).unwrap();
    std::fs::write(
        d.path().join("assets/a/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&s.finish()),
    )
    .unwrap();
    std::fs::write(d.path().join("assets/a/tone.wav"), wave()).unwrap();
    std::fs::write(
        d.path().join("assets/a/Gameexe.ini"),
        format!("#BGM.000=\"Song\",\"tone\",700,800,750\n#SE.000=\"tone\"\n{extra}"),
    )
    .unwrap();
    let project = json!({"schema":2,"name":"native PCMCH","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"siglus","game":"authored","asset_prefix":"a","config":{"scene":"Scene.pck","format":"pck","string_codec":"plain","gameexe":"Gameexe.ini","resources":{"tone":"tone.wav","koe/12":"tone.wav"}}},{"name":"b","adapter":"crysirror-script-v1","game":"other","program":[]}]});
    let p = d.path().join("project.json");
    std::fs::write(&p, serde_json::to_vec(&project).unwrap()).unwrap();
    let r = Runtime::load(&p, &d.path().join("state")).unwrap();
    (d, r)
}
fn chain(ch: i32, op: i32) -> [i32; 4] {
    [c::ELM_GLOBAL_PCMCH, c::ELM_ARRAY, ch, op]
}
fn command(s: &mut SceneBuilder, ch: i32, op: i32, args: &[i32], ret: i32) {
    s.command(&chain(ch, op), 0, args, ret);
}
fn play(s: &mut SceneBuilder, ch: i32, op: i32) {
    s.command_string(&chain(ch, op), 0, "tone", &[], fm::VOID);
}
fn named(s: &mut SceneBuilder, ch: i32, op: i32, strings: &[(i32, &str)], ints: &[(i32, i32)]) {
    s.element(&chain(ch, op));
    for (_, text) in strings {
        s.string(text);
    }
    for (_, n) in ints {
        s.int(*n);
    }
    s.code.push(cd::COMMAND);
    s.i32(0);
    s.i32((strings.len() + ints.len()) as i32);
    for _ in ints {
        s.i32(fm::INT);
    }
    for _ in strings {
        s.i32(fm::STR);
    }
    s.i32((strings.len() + ints.len()) as i32);
    for (id, _) in ints.iter().rev() {
        s.i32(*id);
    }
    for (id, _) in strings.iter().rev() {
        s.i32(*id);
    }
    s.i32(fm::VOID);
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
fn hold(s: &mut SceneBuilder) {
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10000], fm::VOID);
}
fn native(r: &Runtime) -> Value {
    r.inspect()["sessions"][0]["native"].clone()
}
fn slot(r: &Runtime, ch: u32) -> Value {
    native(r)["pcmch"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["channel"] == ch)
        .unwrap()
        .clone()
}
fn tick(r: &mut Runtime, ms: u64) {
    r.tick(ms, InputFrame::default()).unwrap();
}
fn info(r: &Runtime, ch: u32) -> Option<crysirror_core::service::AudioPlaybackInfo> {
    r.host.audio_info(ClientId(1), 4 + ch).unwrap()
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
#[test]
fn global_pcm_and_multiple_pcmch_sources_are_independent_and_slot_gain_is_shared() {
    let mut s = SceneBuilder::default();
    s.command_string(
        &[c::ELM_GLOBAL_PCM, c::ELM_PCM_PLAY],
        0,
        "tone",
        &[],
        fm::VOID,
    );
    command(&mut s, 0, c::ELM_PCMCH_SET_VOLUME, &[128], fm::VOID);
    play(&mut s, 0, c::ELM_PCMCH_PLAY_LOOP);
    play(&mut s, 1, c::ELM_PCMCH_PLAY_LOOP);
    command(&mut s, 0, c::ELM_PCMCH_CHECK, &[], fm::INT);
    command(&mut s, 1, c::ELM_PCMCH_GET_VOLUME, &[], fm::INT);
    s.command(&[c::ELM_GLOBAL_PCM, c::ELM_PCM_STOP], 0, &[], fm::VOID);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([1, 255]));
    assert!(r.host.audio_info(ClientId(1), 1).unwrap().is_none());
    assert!(info(&r, 0).unwrap().looping && info(&r, 1).unwrap().looping);
    assert!(r.host.audio_info(ClientId(2), 4).unwrap().is_none());
    tick(&mut r, 1);
    let pcm = r.host.channel_audio.take_output();
    let expected = 0.25 * (1. + 128. / 255.);
    assert!(pcm.iter().flatten().all(|x| (*x - expected).abs() < 1e-6));
    assert_eq!(info(&r, 0).unwrap().transport_frames, 48);
}
#[test]
fn play_wait_observes_original_integer_ms_completion_without_a_return_value() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY_WAIT);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_ne!(native(&r)["int_stack"], json!([42]));
    tick(&mut r, 19);
    tick(&mut r, 0);
    assert_ne!(native(&r)["int_stack"], json!([42]));
    tick(&mut r, 1);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
    assert!(info(&r, 0).unwrap().source_playing); // 40 physical frames remain.
}
#[test]
fn ready_resume_wait_and_pause_fade_delayed_resume_use_engine_transport() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY);
    command(&mut s, 0, c::ELM_PCMCH_PAUSE, &[10], fm::VOID);
    named(&mut s, 0, c::ELM_PCMCH_RESUME, &[], &[(0, 30)]);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["playing"], false);
    tick(&mut r, 10);
    let pcm = r.host.channel_audio.take_output();
    tick(&mut r, 0);
    let i = info(&r, 0).unwrap();
    assert!(i.paused);
    assert_eq!(i.transport_frames, 480);
    assert_eq!(i.resume_delay_frames, 960);
    assert!(pcm[0][0] > 0.24 && pcm[479][0] < 0.001);
    tick(&mut r, 20);
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["playing"], true);
    assert_eq!(info(&r, 0).unwrap().transport_frames, 480);
    tick(&mut r, 9);
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["playing"], true);
    tick(&mut r, 1);
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["playing"], false);
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY);
    command(&mut s, 0, c::ELM_PCMCH_CHECK, &[], fm::INT);
    named(&mut s, 0, c::ELM_PCMCH_RESUME_WAIT, &[], &[(0, 100)]);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_eq!(info(&r, 0).unwrap().resume_delay_frames, 0);
    assert_eq!(native(&r)["int_stack"], json!([0]));
    tick(&mut r, 20);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
}
#[test]
fn wait_fade_retains_stop_completion_after_eof_but_pause_is_not_stop_fade() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY);
    command(&mut s, 0, c::ELM_PCMCH_STOP, &[40], fm::VOID);
    command(&mut s, 0, c::ELM_PCMCH_CHECK, &[], fm::INT);
    command(&mut s, 0, c::ELM_PCMCH_WAIT_FADE, &[], fm::INT);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0]));
    tick(&mut r, 21);
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["fading"], true);
    assert!(!info(&r, 0).unwrap().source_playing);
    assert_eq!(native(&r)["int_stack"], json!([0]));
    tick(&mut r, 19);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 0, 42]));
    assert!(info(&r, 0).is_none());
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY_LOOP);
    command(&mut s, 0, c::ELM_PCMCH_PAUSE, &[40], fm::VOID);
    command(&mut s, 0, c::ELM_PCMCH_WAIT_FADE, &[], fm::INT);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    assert!(info(&r, 0).unwrap().pausing);
}
#[test]
fn decide_wait_returns_one_after_down_up_and_leaves_loop_running() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY_LOOP);
    command(&mut s, 0, c::ELM_PCMCH_WAIT_KEY, &[], fm::INT);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
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
    assert!(info(&r, 0).unwrap().looping);
}
#[test]
fn source_priority_bgm_filename_only_numeric_sources_and_se_loop_policy_are_preserved() {
    let mut s = SceneBuilder::default();
    named(
        &mut s,
        0,
        c::ELM_PCMCH_PLAY,
        &[(7, "tone"), (10, "missing")],
        &[(8, 999), (9, 999)],
    );
    named(&mut s, 1, c::ELM_PCMCH_READY_LOOP, &[(10, " SONG ")], &[]);
    named(&mut s, 2, c::ELM_PCMCH_PLAY_LOOP, &[], &[(8, 12)]);
    named(&mut s, 3, c::ELM_PCMCH_PLAY_LOOP, &[], &[(9, 0)]);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["resource"], "tone");
    assert_eq!(info(&r, 1).unwrap().position_frames, 0);
    assert_eq!(info(&r, 1).unwrap().source_frames, Some(1000));
    assert!(info(&r, 1).unwrap().paused);
    assert!(info(&r, 2).unwrap().looping);
    assert!(!info(&r, 3).unwrap().looping);
    assert_eq!(native(&r)["koe"]["number"], -1);
}
#[test]
fn stop_and_eof_discard_sources_and_resume_does_not_reload_them() {
    for stopped in [true, false] {
        let mut s = SceneBuilder::default();
        play(&mut s, 0, c::ELM_PCMCH_PLAY);
        if stopped {
            command(&mut s, 0, c::ELM_PCMCH_STOP, &[], fm::VOID);
        } else {
            command(&mut s, 0, c::ELM_PCMCH_WAIT, &[], fm::VOID);
        }
        s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[10], fm::VOID);
        command(&mut s, 0, c::ELM_PCMCH_RESUME, &[], fm::VOID);
        finish(&mut s);
        let (d, mut r) = fixture(s, "");
        tick(&mut r, 0);
        if !stopped {
            tick(&mut r, 21);
            tick(&mut r, 0);
        }
        std::fs::remove_file(d.path().join("assets/a/tone.wav")).unwrap();
        tick(&mut r, 10);
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([42]));
        assert!(info(&r, 0).is_none());
    }
}
#[test]
fn runtime_snapshot_reconstructs_ready_loop_and_paused_declarations_and_original_save_persists_only_native_cues()
 {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY_LOOP);
    play(&mut s, 1, c::ELM_PCMCH_PLAY_LOOP);
    play(&mut s, 2, c::ELM_PCMCH_PLAY);
    command(&mut s, 2, c::ELM_PCMCH_PAUSE, &[], fm::VOID);
    command(&mut s, 1, c::ELM_PCMCH_SET_VOLUME, &[100], fm::VOID);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    let snapshot = r.snapshot().unwrap();
    let original = r.export_original_save(ClientId(1)).unwrap();
    r.restore(&snapshot).unwrap();
    tick(&mut r, 0);
    assert!(info(&r, 0).unwrap().paused);
    assert!(info(&r, 1).unwrap().looping);
    assert!(info(&r, 2).unwrap().paused);
    assert_eq!(slot(&r, 1)["volume"], 100);
    r.import_original_save(ClientId(1), &original).unwrap();
    tick(&mut r, 0);
    assert!(info(&r, 0).unwrap().paused);
    assert!(info(&r, 1).unwrap().looping);
    assert!(info(&r, 2).is_none());
    assert_eq!(slot(&r, 1)["volume"], 100);
}
#[test]
fn unsupported_routing_unknown_operation_bounds_and_missing_alias_fail_before_arithmetic() {
    for variant in 0..6 {
        let mut s = SceneBuilder::default();
        match variant {
            0 => command(&mut s, 0, 9876, &[], fm::VOID),
            1 => play(&mut s, 16, c::ELM_PCMCH_PLAY),
            2 => named(&mut s, 0, c::ELM_PCMCH_PLAY, &[(7, "tone")], &[(3, 1)]),
            3 => named(&mut s, 0, c::ELM_PCMCH_PLAY, &[(7, "tone")], &[(11, 1)]),
            4 => s.command_string(&chain(0, c::ELM_PCMCH_PLAY), 0, "missing", &[], fm::VOID),
            _ => command(&mut s, 0, c::ELM_PCMCH_PLAY, &[], fm::VOID),
        }
        finish(&mut s);
        let (_d, mut r) = fixture(s, "");
        assert!(r.tick(0, InputFrame::default()).is_err());
        assert_ne!(native(&r)["int_stack"], json!([42]));
        assert!(info(&r, 0).is_none());
    }
}
#[test]
fn async_resource_preparation_keeps_original_command_pending_and_publishes_only_after_workers() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    r.set_audio_nonblocking(true);
    let pool = r.assets.io_tasks();
    let (tx, rx) = std::sync::mpsc::channel();
    let (start, ready) = std::sync::mpsc::channel();
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let mut gates = vec![];
    for _ in 0..pool.stats().workers {
        let rx = rx.clone();
        let start = start.clone();
        gates.push(
            pool.spawn(move |_| {
                start.send(()).unwrap();
                rx.lock().unwrap().recv_timeout(Duration::from_secs(10))?;
                Ok(())
            })
            .unwrap(),
        );
    }
    for _ in 0..gates.len() {
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    tick(&mut r, 0);
    let pending = native(&r);
    assert_eq!(pending["audio_preparing"], true);
    assert_ne!(pending["int_stack"], json!([42]));
    assert!(info(&r, 0).is_none());
    for _ in 0..gates.len() {
        tx.send(()).unwrap();
    }
    for gate in gates {
        gate.wait().unwrap();
    }
    until(&mut r, |r| native(r)["int_stack"] == json!([42]));
    assert!(info(&r, 0).unwrap().paused);
    assert_eq!(slot(&r, 0)["ready"], true);
}

#[test]
fn declared_last_channel_waits_use_u8_identity_and_natural_key_fade_waits_return_zero() {
    let mut s = SceneBuilder::default();
    play(&mut s, 255, c::ELM_PCMCH_PLAY_LOOP);
    command(&mut s, 255, c::ELM_PCMCH_CHECK, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "#PCMCH.CNT=256\n");
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([1]));
    assert!(info(&r, 255).unwrap().looping);
    for operation in [
        c::ELM_PCMCH_WAIT,
        c::ELM_PCMCH_WAIT_KEY,
        c::ELM_PCMCH_WAIT_FADE_KEY,
    ] {
        let mut s = SceneBuilder::default();
        play(&mut s, 0, c::ELM_PCMCH_PLAY);
        if operation == c::ELM_PCMCH_WAIT_FADE_KEY {
            command(&mut s, 0, c::ELM_PCMCH_STOP, &[40], fm::VOID);
        }
        command(&mut s, 0, operation, &[], fm::INT);
        finish(&mut s);
        let (_d, mut r) = fixture(s, "");
        tick(&mut r, 0);
        tick(
            &mut r,
            if operation == c::ELM_PCMCH_WAIT_FADE_KEY {
                40
            } else {
                20
            },
        );
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([0, 42]));
    }
}

#[test]
fn stopped_ready_flag_survives_original_file_load_but_stopped_runtime_snapshot_stays_stopped() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY_LOOP);
    command(&mut s, 0, c::ELM_PCMCH_STOP, &[], fm::VOID);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    let snapshot = r.snapshot().unwrap();
    let original = r.export_original_save(ClientId(1)).unwrap();
    assert!(info(&r, 0).is_none());
    r.restore(&snapshot).unwrap();
    tick(&mut r, 0);
    assert!(info(&r, 0).is_none());
    r.import_original_save(ClientId(1), &original).unwrap();
    tick(&mut r, 0);
    let i = info(&r, 0).unwrap();
    assert!(i.paused && !i.looping);
    assert_eq!(slot(&r, 0)["ready"], true);
}

#[test]
fn remaining_resume_delay_reconstructs_in_runtime_but_is_only_a_native_file_restore_predicate() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY);
    named(&mut s, 0, c::ELM_PCMCH_RESUME, &[], &[(0, 100)]);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    tick(&mut r, 20);
    tick(&mut r, 0);
    assert_eq!(slot(&r, 0)["delay_ms"], 80);
    let snapshot = r.snapshot().unwrap();
    let original = r.export_original_save(ClientId(1)).unwrap();
    r.restore(&snapshot).unwrap();
    tick(&mut r, 0);
    assert_eq!(info(&r, 0).unwrap().resume_delay_frames, 3840);
    tick(&mut r, 80);
    tick(&mut r, 0);
    assert!(!info(&r, 0).unwrap().paused);
    assert_eq!(info(&r, 0).unwrap().transport_frames, 0);
    r.import_original_save(ClientId(1), &original).unwrap();
    tick(&mut r, 0);
    let i = info(&r, 0).unwrap();
    assert!(!i.paused);
    assert_eq!(i.resume_delay_frames, 0);
    assert_eq!(i.position_frames, 0);
}

#[test]
fn failed_channel_replacement_preserves_old_voice_original_pc_and_retry_after_resource_repair() {
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_PLAY_LOOP);
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[2], fm::VOID);
    play(&mut s, 0, c::ELM_PCMCH_PLAY);
    finish(&mut s);
    let (d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    let old = info(&r, 0).unwrap();
    std::fs::write(d.path().join("assets/a/tone.wav"), b"broken").unwrap();
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
    assert!(info(&r, 0).unwrap().looping && old.looping);
    assert_eq!(r.host.channel_audio.resources.resource_count(), 0);
    std::fs::write(d.path().join("assets/a/tone.wav"), wave()).unwrap();
    until(&mut r, |r| native(r)["halted"] == true);
    assert_eq!(native(&r)["int_stack"], json!([42]));
    assert!(!info(&r, 0).unwrap().looping);
}

#[test]
fn native_stop_deadline_survives_later_pause_fades_without_a_vm_timer() {
    for fade in [1, 100] {
        let mut s = SceneBuilder::default();
        play(&mut s, 0, c::ELM_PCMCH_PLAY_LOOP);
        command(&mut s, 0, c::ELM_PCMCH_STOP, &[40], fm::VOID);
        command(&mut s, 0, c::ELM_PCMCH_PAUSE, &[fade], fm::VOID);
        command(&mut s, 0, c::ELM_PCMCH_WAIT_FADE, &[], fm::INT);
        finish(&mut s);
        let (_d, mut r) = fixture(s, "");
        tick(&mut r, 0);
        tick(&mut r, 20);
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([]));
        assert_eq!(info(&r, 0).unwrap().fade_out_frames, 960);
        tick(&mut r, 20);
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([0, 42]));
        assert!(info(&r, 0).is_none());
    }
}

#[test]
fn native_volume_replaces_initial_play_fade_and_resume_restarts_latest_gain_target() {
    let mut s = SceneBuilder::default();
    s.command_string(
        &chain(0, c::ELM_PCMCH_PLAY_LOOP),
        0,
        "tone",
        &[100],
        fm::VOID,
    );
    command(&mut s, 0, c::ELM_PCMCH_SET_VOLUME_MAX, &[], fm::VOID);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    tick(&mut r, 1);
    assert!(
        r.host
            .channel_audio
            .take_output()
            .iter()
            .flatten()
            .all(|x| (*x - 0.25).abs() < 1e-6)
    );
    let mut s = SceneBuilder::default();
    play(&mut s, 0, c::ELM_PCMCH_READY_LOOP);
    command(&mut s, 0, c::ELM_PCMCH_SET_VOLUME, &[128, 100], fm::VOID);
    command(&mut s, 0, c::ELM_PCMCH_RESUME, &[], fm::VOID);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s, "");
    tick(&mut r, 0);
    tick(&mut r, 1);
    let expected = 0.25 * 128. / 255.;
    assert!(
        r.host
            .channel_audio
            .take_output()
            .iter()
            .flatten()
            .all(|x| (*x - expected).abs() < 1e-6)
    );
}
