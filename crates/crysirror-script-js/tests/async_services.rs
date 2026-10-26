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
    match failed.init() {
        Err(error) => assert!(format!("{error:#}").contains("rejected")),
        Ok(()) => {
            failed.start().unwrap();
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
