// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Frame dependency, actual PCM, lifecycle and restore acceptance without a device.
use crysirror_core::{
    service::{AudioParameters, AudioRequest, HostServices, InputFrame, PcmAudio},
    state::ClientId,
};
use crysirror_engine::{App, AppState, EngineContext, Lifecycle, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn app() -> (tempfile::TempDir, App) {
    app_counter(None)
}
fn app_counter(counter: Option<Arc<AtomicUsize>>) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"mixframe","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"test"}]})).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&path, &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    if let Some(counter) = counter {
        app.add_behaviour(Box::new(Count(counter))).unwrap();
    }
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn native(app: &mut App) {
    app.runtime_mut()
        .unwrap()
        .host
        .submit_audio(
            ClientId(1),
            AudioRequest::PlayPcm {
                channel: 7,
                source: PcmAudio::new(48000, vec![[0.25; 2]; 48000]).unwrap(),
                parameters: AudioParameters::default(),
            },
        )
        .unwrap();
}
fn collect(app: &mut App) {
    let limit = Instant::now() + Duration::from_secs(5);
    while !app.poll_frame().unwrap() {
        assert!(Instant::now() < limit);
        std::thread::yield_now();
    }
}
fn rendering(app: &mut App) -> bool {
    let cx = app.context();
    let h = cx.singleton("Audio").unwrap();
    cx.get(h, "rendering").unwrap().to_json() == json!(true)
}
struct Count(Arc<AtomicUsize>);
impl Lifecycle for Count {
    fn name(&self) -> &str {
        "counter"
    }
    fn on_update(&mut self, _: &mut EngineContext, _: u64) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn capacity_backpressure_accepts_callbacks_input_and_time_once_and_blocks_controls() {
    let count = Arc::new(AtomicUsize::new(0));
    let (_dir, mut a) = app_counter(Some(count.clone()));
    native(&mut a);
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    a.begin_frame(16, InputFrame::default()).unwrap();
    assert!(a.frame_pending());
    assert!(rendering(&mut a));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(a.runtime().unwrap().now_ms(), 16);
    assert_eq!(a.engine().unwrap().frame, 0);
    for _ in 0..3 {
        assert!(!a.poll_frame().unwrap());
        assert!(a.begin_frame(16, InputFrame::default()).is_err());
        assert!(a.frame(16, InputFrame::default()).is_err());
    }
    let cx = a.context();
    let h = cx.singleton("Audio").unwrap();
    assert!(
        cx.call(
            h,
            "setBus",
            &[
                Value::from_json(json!("sfx")),
                Value::from_json(json!({"gain":0.5}))
            ]
        )
        .is_err()
    );
    assert!(
        cx.call(h, "nativeChannelInfo", &[Value::Int(7)])
            .unwrap()
            .to_json()["age_frames"]
            == json!(0)
    );
    drop(permits);
    collect(&mut a);
    assert!(!rendering(&mut a));
    assert_eq!(a.engine().unwrap().frame, 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let pcm = a.context().state().unwrap().mixer_audio.take_output();
    assert_eq!(pcm.len(), 768);
    assert!(pcm.iter().all(|p| *p == [0.25; 2]));
    assert!(a.poll_frame().unwrap());
    assert!(
        a.context()
            .state()
            .unwrap()
            .mixer_audio
            .take_output()
            .is_empty()
    );
}
#[test]
fn paired_engine_worker_mix_matches_synchronous_reference_and_pause_is_deferred() {
    let (_d, mut a) = app();
    let (_e, mut b) = app();
    native(&mut a);
    native(&mut b);
    for app in [&mut a, &mut b] {
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .play(
                1,
                crysirror_audio::decoder::open_pcm(
                    PcmAudio::new(48000, vec![[0.125; 2]; 48000]).unwrap(),
                ),
                Default::default(),
            )
            .unwrap();
    }
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    a.begin_frame(17, InputFrame::default()).unwrap();
    a.pause().unwrap();
    assert_eq!(a.state(), AppState::Running);
    drop(permits);
    collect(&mut a);
    assert_eq!(a.state(), AppState::Paused);
    b.frame(17, InputFrame::default()).unwrap();
    let actual = a.context().state().unwrap().mixer_audio.take_output();
    let expected = b.context().state().unwrap().mixer_audio.take_output();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 816);
    assert_eq!(actual[10], [0.375; 2]);
    a.resume().unwrap();
    a.begin_frame(1, InputFrame::default()).unwrap();
    collect(&mut a);
    assert_eq!(a.engine().unwrap().frame, 2);
}
#[test]
fn restore_and_shutdown_discard_pending_audio_without_stale_publication() {
    for shutdown in [false, true] {
        let (_dir, mut a) = app();
        let save = a.runtime().unwrap().snapshot().unwrap();
        native(&mut a);
        let pool = a.runtime().unwrap().assets.cpu_tasks();
        let permits = (0..pool.stats().capacity)
            .map(|_| pool.reserve().unwrap())
            .collect::<Vec<_>>();
        a.begin_frame(16, InputFrame::default()).unwrap();
        assert!(a.frame_pending());
        if shutdown {
            a.shutdown().unwrap();
            assert_eq!(a.state(), AppState::Stopped);
        } else {
            a.runtime_mut().unwrap().restore(&save).unwrap();
            assert!(a.poll_frame().unwrap());
            assert_eq!(a.engine().unwrap().frame, 0);
        }
        assert!(!a.frame_pending());
        assert!(!rendering(&mut a));
        assert!(
            a.context()
                .state()
                .unwrap()
                .mixer_audio
                .take_output()
                .is_empty()
        );
        drop(permits);
    }
}
#[test]
fn runtime_worker_frame_matches_native_pcm_and_never_repeats_tick_while_pending() {
    let (_d, mut a) = app();
    native(&mut a);
    let rt = a.runtime_mut().unwrap();
    let pool = rt.assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    rt.begin_tick(10, InputFrame::default()).unwrap();
    assert!(rt.audio_frame_pending());
    assert_eq!(rt.now_ms(), 10);
    assert!(!rt.poll_audio_frame().unwrap());
    assert!(rt.tick(10, InputFrame::default()).is_err());
    assert_eq!(rt.now_ms(), 10);
    drop(permits);
    let limit = Instant::now() + Duration::from_secs(5);
    while !rt.poll_audio_frame().unwrap() {
        assert!(Instant::now() < limit);
        std::thread::yield_now();
    }
    let pcm = rt.host.take_pcm_output();
    assert_eq!(pcm.len(), 480);
    assert_eq!(pcm[5], [0.25; 2]);
    assert!(!rt.audio_frame_pending());
}

#[test]
fn completion_failure_faults_the_app_before_it_can_accept_another_tick() {
    let (_dir, mut a) = app();
    a.context().state().unwrap().frame = u64::MAX;
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    a.begin_frame(1, InputFrame::default()).unwrap();
    drop(permits);
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        match a.poll_frame() {
            Err(_) => break,
            Ok(false) => {
                assert!(Instant::now() < limit);
                std::thread::yield_now();
            }
            Ok(true) => panic!("overflow must fail"),
        }
    }
    assert_eq!(a.state(), AppState::Faulted);
    assert!(a.fault().unwrap().contains("frame counter overflow"));
    assert!(a.begin_frame(1, InputFrame::default()).is_err());
    assert_eq!(a.runtime().unwrap().now_ms(), 1);
}
