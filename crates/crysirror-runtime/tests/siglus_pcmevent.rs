// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored original Scene.pck ABI + actual shared sound/scheduler state.
use crysirror_core::{
    service::{AudioSequenceHandle, HostServices, InputFrame},
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
fn chain(index: i32, op: i32) -> [i32; 4] {
    [c::ELM_GLOBAL_PCMEVENT, c::ELM_ARRAY, index, op]
}
fn start(
    s: &mut SceneBuilder,
    index: i32,
    op: i32,
    target: i32,
    lines: &[(&str, &[i32])],
    time: bool,
) {
    s.element(&chain(index, op));
    s.int(target);
    for (key, nums) in lines {
        s.string(key);
        for n in *nums {
            s.int(*n);
        }
    }
    s.int(i32::from(time));
    s.code.push(cd::COMMAND);
    s.i32(0);
    s.i32((lines.len() + 2) as i32);
    s.i32(fm::INT);
    for (_, nums) in lines.iter().rev() {
        s.i32(fm::LIST);
        s.i32((nums.len() + 1) as i32);
        for _ in *nums {
            s.i32(fm::INT);
        }
        s.i32(fm::STR);
    }
    s.i32(fm::INT);
    s.i32(1);
    s.i32(11);
    s.i32(fm::VOID);
}
fn command(s: &mut SceneBuilder, index: i32, op: i32, args: &[i32], ret: i32) {
    s.command(&chain(index, op), 0, args, ret);
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
fn wave(value: i16) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(164u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(128u32.to_le_bytes());
    for _ in 0..64 {
        b.extend(value.to_le_bytes());
    }
    b
}
fn fixture(s: SceneBuilder) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("assets/a")).unwrap();
    std::fs::write(
        d.path().join("assets/a/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&s.finish()),
    )
    .unwrap();
    std::fs::write(d.path().join("assets/a/a.wav"), wave(8192)).unwrap();
    std::fs::write(d.path().join("assets/a/b.wav"), wave(-16384)).unwrap();
    let p = d.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"PCMEVENT","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"siglus","game":"authored","asset_prefix":"a","config":{"scene":"Scene.pck","string_codec":"plain","resources":{"a":"a.wav","b":"b.wav"}}}]})).unwrap()).unwrap();
    let r = Runtime::load(&p, &d.path().join("state")).unwrap();
    (d, r)
}
fn native(r: &Runtime) -> Value {
    r.inspect()["sessions"][0]["native"].clone()
}
fn event(r: &Runtime, index: usize) -> Value {
    native(r)["pcmevents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["index"] == index)
        .unwrap()
        .clone()
}
fn tick(r: &mut Runtime, ms: u64) {
    r.tick(ms, InputFrame::default()).unwrap();
}
fn until(r: &mut Runtime, done: impl Fn(&Runtime) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(r) {
        tick(r, 0);
        assert!(Instant::now() < deadline, "{}", native(r));
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn shared(r: &Runtime, index: usize) -> crysirror_core::service::AudioSequenceInfo {
    let h = event(r, index)["shared"].clone();
    r.host
        .audio_sequence_info(
            ClientId(1),
            AudioSequenceHandle {
                client: 1,
                generation: h["generation"].as_u64().unwrap(),
            },
        )
        .unwrap()
}
#[test]
fn original_oneshot_waits_last_interval_and_real_pcm_uses_engine_channels() {
    for target in [-1, 0] {
        let mut s = SceneBuilder::default();
        start(
            &mut s,
            0,
            c::ELM_PCMEVENT_START_ONESHOT,
            target,
            &[("a", &[2]), ("b", &[4])],
            true,
        );
        command(&mut s, 0, c::ELM_PCMEVENT_CHECK, &[], fm::INT);
        command(&mut s, 0, c::ELM_PCMEVENT_WAIT, &[], fm::VOID);
        finish(&mut s);
        let (_d, mut r) = fixture(s);
        tick(&mut r, 0);
        assert_eq!(native(&r)["int_stack"], json!([1]));
        tick(&mut r, 0);
        assert_eq!(event(&r, 0)["current_line"], 0);
        assert_eq!(shared(&r, 0).next_ms, 10);
        tick(&mut r, 1);
        let pcm = r.host.channel_audio.take_output();
        assert!(pcm.iter().flatten().all(|v| (*v - 0.25).abs() < 1e-6));
        assert_eq!(pcm.len(), 48);
        tick(&mut r, 8);
        assert_eq!(event(&r, 0)["current_line"], 0);
        assert_ne!(native(&r)["int_stack"], json!([1, 42]));
        tick(&mut r, 1);
        assert_eq!(event(&r, 0)["current_line"], 1);
        assert_eq!(shared(&r, 0).next_ms, 12);
        tick(&mut r, 11);
        assert_eq!(native(&r)["pcmevents"].as_array().unwrap().len(), 1);
        tick(&mut r, 1);
        until(&mut r, |r| native(r)["int_stack"] == json!([1, 42]));
        assert!(native(&r)["pcmevents"].as_array().unwrap().is_empty());
    }
}
#[test]
fn wait_key_releases_the_script_without_canceling_the_event_or_its_source() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_ONESHOT,
        0,
        &[("a", &[100]), ("b", &[])],
        true,
    );
    command(&mut s, 0, c::ELM_PCMEVENT_WAIT_KEY, &[], fm::INT);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    r.tick(
        0,
        InputFrame {
            buttons_down: 1,
            buttons_pressed: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_ne!(native(&r)["int_stack"], json!([1, 42]));
    tick(&mut r, 0);
    until(&mut r, |r| native(r)["int_stack"] == json!([1, 42]));
    assert!(shared(&r, 0).active);
    assert!(
        r.host
            .audio_info(ClientId(1), 4)
            .unwrap()
            .unwrap()
            .source_playing
    );
    tick(&mut r, 108);
    assert_eq!(event(&r, 0)["current_line"], 1);
    assert_eq!(shared(&r, 0).emissions, 2);
    assert_eq!(native(&r)["int_stack"], json!([1, 42]));
}
#[test]
fn loop_wait_does_not_block_and_runtime_cursor_differs_from_original_restart_policy() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_LOOP,
        1,
        &[("a", &[]), ("b", &[])],
        true,
    );
    command(&mut s, 0, c::ELM_PCMEVENT_WAIT, &[], fm::VOID);
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    tick(&mut r, 8);
    tick(&mut r, 3);
    assert_eq!(event(&r, 0)["current_line"], 1);
    assert_eq!(shared(&r, 0).elapsed_ms, 3);
    let saved = r.snapshot().unwrap();
    let original = r.export_original_save(ClientId(1)).unwrap();
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert_eq!(event(&r, 0)["current_line"], 1);
    assert_eq!(shared(&r, 0).elapsed_ms, 3);
    tick(&mut r, 5);
    assert_eq!(event(&r, 0)["current_line"], 0);
    r.import_original_save(ClientId(1), &original).unwrap();
    tick(&mut r, 0);
    assert_eq!(event(&r, 0)["current_line"], 0);
    assert_eq!(shared(&r, 0).elapsed_ms, 0);
}
#[test]
fn stopping_scheduling_leaves_the_shared_voice_until_explicit_stop_pcm() {
    for stop in [0, 1] {
        let mut s = SceneBuilder::default();
        start(
            &mut s,
            0,
            c::ELM_PCMEVENT_START_LOOP,
            0,
            &[("a", &[])],
            true,
        );
        s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
        command(&mut s, 0, c::ELM_PCMEVENT_STOP, &[stop], fm::VOID);
        hold(&mut s);
        finish(&mut s);
        let (_d, mut r) = fixture(s);
        tick(&mut r, 0);
        tick(&mut r, 0);
        let old = event(&r, 0)["shared"].clone();
        tick(&mut r, 1);
        assert!(
            r.host
                .audio_sequence_info(
                    ClientId(1),
                    AudioSequenceHandle {
                        client: 1,
                        generation: old["generation"].as_u64().unwrap()
                    }
                )
                .is_err(),
            "STOP left a shared scheduler active until a later frame"
        );
        until(&mut r, |r| {
            native(r)["pcmevents"].as_array().unwrap().is_empty()
        });
        let playing = r
            .host
            .audio_info(ClientId(1), 4)
            .unwrap()
            .is_some_and(|v| v.source_playing);
        assert_eq!(playing, stop == 0);
    }
}
#[test]
fn async_pending_frame_snapshot_replays_selection_once_without_double_clock_or_rng() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_RANDOM,
        0,
        &[("a", &[1, 20, 30]), ("b", &[2, 20, 30])],
        true,
    );
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    r.set_audio_nonblocking(true);
    let pool = r.assets.io_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    tick(&mut r, 3);
    assert_eq!(native(&r)["audio_preparing"], true);
    assert_eq!(native(&r)["event_frame_pending"], true);
    let before = native(&r);
    for _ in 0..3 {
        tick(&mut r, 1);
        assert_eq!(event(&r, 0)["current_line"], -1);
        assert_eq!(native(&r)["native_rng"], before["native_rng"]);
    }
    let snapshot = r.snapshot().unwrap();
    drop(permits);
    until(&mut r, |r| native(r)["event_frame_pending"] == false);
    tick(&mut r, 0);
    let expected = shared(&r, 0);
    assert_eq!(
        expected.elapsed_ms, 0,
        "dependency time was charged on the next owner frame"
    );
    let rng = native(&r)["native_rng"].clone();
    r.restore(&snapshot).unwrap();
    until(&mut r, |r| native(r)["event_frame_pending"] == false);
    tick(&mut r, 0);
    assert_eq!(shared(&r, 0), expected);
    assert_eq!(native(&r)["native_rng"], rng);
    assert_eq!(shared(&r, 0).emissions, 1);
    assert_eq!(shared(&r, 0).elapsed_ms, 0);
}
#[test]
fn malformed_layout_unknown_parameters_missing_alias_and_nondefault_routes_fail_visibly() {
    for mode in 0..8 {
        let mut s = SceneBuilder::default();
        match mode {
            0 => command(&mut s, 128, c::ELM_PCMEVENT_CHECK, &[], fm::INT),
            1 => start(
                &mut s,
                0,
                c::ELM_PCMEVENT_START_LOOP,
                16,
                &[("a", &[])],
                true,
            ),
            2 => start(
                &mut s,
                0,
                c::ELM_PCMEVENT_START_LOOP,
                -2,
                &[("a", &[])],
                true,
            ),
            3 => start(
                &mut s,
                0,
                c::ELM_PCMEVENT_START_LOOP,
                0,
                &[("missing", &[])],
                true,
            ),
            4 => command(&mut s, 0, 99999, &[], fm::VOID),
            5 | 6 => {
                s.element(&chain(0, c::ELM_PCMEVENT_START_LOOP));
                s.int(0);
                s.string("a");
                s.int(1);
                s.code.push(cd::COMMAND);
                s.i32(0);
                s.i32(3);
                s.i32(fm::INT);
                s.i32(fm::STR);
                s.i32(fm::INT);
                s.i32(1);
                s.i32(if mode == 5 { 3 } else { 17 });
                s.i32(fm::VOID);
            }
            _ => s.command(
                &[
                    c::ELM_GLOBAL_PCMEVENT,
                    c::ELM_ARRAY,
                    0,
                    c::ELM_PCMEVENT_CHECK,
                    0,
                ],
                0,
                &[],
                fm::INT,
            ),
        }
        hold(&mut s);
        let (_d, mut r) = fixture(s);
        let failed =
            r.tick(0, InputFrame::default()).is_err() || r.tick(0, InputFrame::default()).is_err();
        assert!(failed, "mode {mode}");
    }
}

#[test]
fn native_math_and_event_selection_reconcile_one_shared_script_rng() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_RANDOM,
        0,
        &[("a", &[1, 10, 20]), ("b", &[2, 10, 20])],
        false,
    );
    s.command(&[c::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
    s.command(
        &[c::ELM_GLOBAL_MATH, c::ELM_MATH_RAND],
        0,
        &[0, 999],
        fm::INT,
    );
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    assert_eq!(event(&r, 0)["current_line"], 1);
    assert_eq!(shared(&r, 0).next_ms, 11);
    assert_eq!(native(&r)["native_rng"], 358294691u32);
    tick(&mut r, 1);
    assert_eq!(native(&r)["int_stack"], json!([364]));
    assert_eq!(native(&r)["native_rng"], 1210119364u32);
    tick(&mut r, 10);
    assert_eq!(event(&r, 0)["current_line"], 0);
    assert_eq!(shared(&r, 0).next_ms, 18);
    assert_eq!(native(&r)["native_rng"], 1882851208u32);
}
#[test]
fn empty_oneshot_natural_key_wait_returns_zero_without_an_audio_source() {
    let mut s = SceneBuilder::default();
    start(&mut s, 0, c::ELM_PCMEVENT_START_ONESHOT, -1, &[], true);
    command(&mut s, 0, c::ELM_PCMEVENT_WAIT_KEY, &[], fm::INT);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    until(&mut r, |r| native(r)["int_stack"] == json!([0, 42]));
    assert!(r.host.audio_info(ClientId(1), 1).unwrap().is_none());
}
#[test]
fn original_file_discards_oneshot_but_runtime_snapshot_retains_its_interval() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_ONESHOT,
        -1,
        &[("a", &[100])],
        true,
    );
    s.command(&[c::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    tick(&mut r, 3);
    let saved = r.snapshot().unwrap();
    let file = r.export_original_save(ClientId(1)).unwrap();
    r.restore(&saved).unwrap();
    tick(&mut r, 0);
    assert_eq!(shared(&r, 0).elapsed_ms, 3);
    assert_eq!(shared(&r, 0).next_ms, 108);
    r.import_original_save(ClientId(1), &file).unwrap();
    tick(&mut r, 0);
    assert!(native(&r)["pcmevents"].as_array().unwrap().is_empty());
}

#[test]
fn immediate_last_interval_still_commits_its_random_draw_before_native_reinit() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_ONESHOT,
        -1,
        &[("a", &[-2, -1])],
        false,
    );
    hold(&mut s);
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    tick(&mut r, 0);
    assert!(native(&r)["pcmevents"].as_array().unwrap().is_empty());
    assert_eq!(native(&r)["native_rng"], 2274908837u32);
}

#[test]
fn a_loop_keeps_its_owner_service_running_after_the_original_script_reaches_eof() {
    let mut s = SceneBuilder::default();
    start(
        &mut s,
        0,
        c::ELM_PCMEVENT_START_LOOP,
        0,
        &[("a", &[]), ("b", &[])],
        true,
    );
    finish(&mut s);
    let (_d, mut r) = fixture(s);
    tick(&mut r, 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
    assert_eq!(native(&r)["halted"], true);
    tick(&mut r, 0);
    assert_eq!(event(&r, 0)["current_line"], 0);
    tick(&mut r, 8);
    assert_eq!(event(&r, 0)["current_line"], 1);
    tick(&mut r, 8);
    assert_eq!(event(&r, 0)["current_line"], 0);
    assert_eq!(native(&r)["int_stack"], json!([42]));
}
