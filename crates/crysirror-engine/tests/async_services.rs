// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use anyhow::Result;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{FileSystem, InputFrame},
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use crysirror_tasks::{Task, TaskPool};
use serde_json::{Value as Json, json};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets/a")).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/b")).unwrap();
    let project = json!({"schema":2,"name":"async services","content_revision":"1","assets":"assets","canvas":[128,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]});
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let runtime =
        Runtime::load(&dir.path().join("project.json"), &dir.path().join("state")).unwrap();
    let mut app = App::new(runtime, "a").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn call(app: &mut App, class: &'static str, method: &str, args: Json) -> Result<Json> {
    let cx = app.context();
    let id = cx.singleton(class)?;
    let list = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    Ok(cx.call(id, method, &list)?.to_json())
}
fn ready(app: &mut App, class: &'static str, method: &str, id: u64) -> Json {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = call(app, class, method, json!([id])).unwrap();
        if result["state"] != "pending" {
            return result;
        }
        assert!(Instant::now() < deadline, "{class}: {result}");
        app.frame(0, InputFrame::default()).unwrap();
        std::thread::yield_now();
    }
}
struct Gate {
    shared: Arc<(Mutex<bool>, Condvar)>,
    jobs: Vec<Task<()>>,
}
impl Gate {
    fn block(pool: TaskPool) -> Self {
        let shared = Arc::new((Mutex::new(false), Condvar::new()));
        let mut jobs = vec![];
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..pool.stats().workers {
            let gate = shared.clone();
            let tx = tx.clone();
            jobs.push(
                pool.spawn(move |_| {
                    tx.send(())?;
                    let (lock, wake) = &*gate;
                    let released = lock.lock().unwrap();
                    let (released, _) = wake
                        .wait_timeout_while(released, Duration::from_secs(10), |ready| !*ready)
                        .unwrap();
                    anyhow::ensure!(*released, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..jobs.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self { shared, jobs }
    }
    fn release(&self) {
        let (lock, wake) = &*self.shared;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
    fn finish(mut self) {
        self.release();
        for job in self.jobs.drain(..) {
            job.wait().unwrap();
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
#[test]
fn snapshot_cpu_io_phases_preserve_frame_progress_and_publish_load_only_at_commit() {
    let (_dir, mut app) = app();
    app.frame(32, InputFrame::default()).unwrap();
    let captured = app.runtime().unwrap().now_ms();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    for (method, args) in [
        ("save", json!(["one.save"])),
        ("load", json!(["one.save"])),
        ("exists", json!(["one.save"])),
    ] {
        assert!(
            call(&mut app, "Save", method, args)
                .unwrap_err()
                .to_string()
                .contains("offline-only")
        );
    }
    let cpu = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let io = Gate::block(app.runtime().unwrap().assets.io_tasks());
    let id = call(&mut app, "Save", "beginSave", json!(["one.save"]))
        .unwrap()
        .as_u64()
        .unwrap();
    for _ in 0..3 {
        app.frame(16, InputFrame::default()).unwrap();
        assert_eq!(
            call(&mut app, "Save", "status", json!([id])).unwrap()["state"],
            "pending"
        );
    }
    assert!(app.runtime().unwrap().now_ms() > captured);
    cpu.finish();
    for _ in 0..20 {
        app.frame(0, InputFrame::default()).unwrap();
        assert_eq!(
            call(&mut app, "Save", "status", json!([id])).unwrap()["state"],
            "pending"
        );
        std::thread::yield_now();
    }
    io.finish();
    let result = ready(&mut app, "Save", "status", id);
    assert_eq!(result["state"], "ready");
    assert!(result["bytes"].as_u64().unwrap() > 0);
    let load = call(&mut app, "Save", "beginLoad", json!(["one.save"]))
        .unwrap()
        .as_u64()
        .unwrap();
    let before = app.runtime().unwrap().now_ms();
    let result = ready(&mut app, "Save", "status", load);
    assert_eq!(result["prepared"], true);
    assert_eq!(app.runtime().unwrap().now_ms(), before);
    let owner = app.engine().unwrap().client;
    app.context().state().unwrap().client = app.runtime().unwrap().client_id("b").unwrap();
    assert!(call(&mut app, "Save", "commitLoad", json!([load])).is_err());
    app.context().state().unwrap().client = owner;
    call(&mut app, "Save", "commitLoad", json!([load])).unwrap();
    assert_eq!(app.runtime().unwrap().now_ms(), captured);
    assert!(call(&mut app, "Save", "status", json!([id])).is_err());
}
#[test]
fn invalid_load_and_cancelled_work_do_not_mutate_live_state() {
    let (_dir, mut app) = app();
    let client = app.engine().unwrap().client;
    app.runtime_mut()
        .unwrap()
        .host
        .fs
        .write_atomic(
            client,
            &VfsPath::new(Mount::Save, "bad.save").unwrap(),
            b"corrupt",
        )
        .unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let before = (
        app.runtime().unwrap().restored,
        app.runtime().unwrap().now_ms(),
        serde_json::to_value(&app.runtime().unwrap().host.engine).unwrap(),
    );
    let bad = call(&mut app, "Save", "beginLoad", json!(["bad.save"]))
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(ready(&mut app, "Save", "status", bad)["state"], "failed");
    assert!(call(&mut app, "Save", "commitLoad", json!([bad])).is_err());
    assert_eq!(
        (
            app.runtime().unwrap().restored,
            app.runtime().unwrap().now_ms(),
            serde_json::to_value(&app.runtime().unwrap().host.engine).unwrap()
        ),
        before
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let cancelled = call(&mut app, "Save", "beginSave", json!(["cancel.save"]))
        .unwrap()
        .as_u64()
        .unwrap();
    call(&mut app, "Save", "cancel", json!([cancelled])).unwrap();
    gate.finish();
    let exists = call(&mut app, "Save", "beginExists", json!(["cancel.save"]))
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(ready(&mut app, "Save", "status", exists)["exists"], false);
}
struct Source {
    entries: Vec<crysirror_resource::ArchiveEntry>,
    bytes: Vec<u8>,
    opens: Arc<AtomicUsize>,
    owner: std::thread::ThreadId,
}
impl crysirror_resource::Archive for Source {
    fn entries(&self) -> &[crysirror_resource::ArchiveEntry] {
        &self.entries
    }
    fn open(
        &self,
        _: u32,
        cx: &crysirror_resource::DecodeContext,
    ) -> Result<Arc<dyn crysirror_core::service::AssetReader>> {
        assert_ne!(std::thread::current().id(), self.owner);
        self.opens.fetch_add(1, Ordering::SeqCst);
        cx.decode_with(self.bytes.len() as u64, || Ok(self.bytes.clone()))
    }
}
#[test]
fn image_and_probe_open_mounted_sources_on_workers_and_publish_shared_pixels() {
    let (_dir, mut app) = app();
    let client = app.engine().unwrap().client;
    let expected = crysirror_image::ImageSurface::filled(2, 2, [10, 40, 90, 255]).unwrap();
    let bytes = crysirror_image::encode_png_uncompressed(&expected).unwrap();
    let opens = Arc::new(AtomicUsize::new(0));
    let source = Source {
        entries: vec![crysirror_resource::ArchiveEntry {
            id: 0,
            path: "pixel.png".into(),
            size: bytes.len() as u64,
            stored_size: bytes.len() as u64,
            metadata: Json::Null,
        }],
        bytes,
        opens: opens.clone(),
        owner: std::thread::current().id(),
    };
    let mount = crysirror_resource::ArchiveMount::new(
        Box::new(source),
        crysirror_resource::DecodeContext::new(client),
    )
    .unwrap();
    app.runtime_mut()
        .unwrap()
        .mount_provider(client, "pack", Arc::new(mount))
        .unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    for (method, args) in [
        ("probe", json!(["pack/pixel.png"])),
        ("list", json!(["pack/pixel.png"])),
        ("mount", json!(["pack/pixel.png", "other"])),
        ("probeImage", json!(["pack/pixel.png"])),
        ("decodeImage", json!([1, "pack/pixel.png"])),
    ] {
        assert!(
            call(&mut app, "Resources", method, args)
                .unwrap_err()
                .to_string()
                .contains("offline-only")
        );
    }
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let image = call(
        &mut app,
        "Resources",
        "beginImage",
        json!(["pack/pixel.png"]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(
        call(&mut app, "Resources", "decodeStatus", json!([image])).unwrap()["state"],
        "pending"
    );
    gate.finish();
    assert_eq!(
        ready(&mut app, "Resources", "decodeStatus", image)["state"],
        "ready"
    );
    call(&mut app, "Resources", "publishImage", json!([image, 7])).unwrap();
    let snapshot = app.runtime().unwrap().assets.freeze_surfaces();
    let retained = snapshot
        .surface(client, "__surface/7")
        .unwrap()
        .rgba
        .clone();
    assert_eq!(retained.as_ref(), expected.rgba);
    call(&mut app, "Resources", "cancelDecode", json!([image])).unwrap();
    assert_eq!(
        app.runtime()
            .unwrap()
            .assets
            .surface(client, "__surface/7")
            .unwrap()
            .rgba
            .as_ref(),
        expected.rgba
    );
    let probe = call(
        &mut app,
        "Resources",
        "beginProbe",
        json!(["pack/pixel.png", "image"]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    ready(&mut app, "Resources", "decodeStatus", probe);
    assert_eq!(
        call(&mut app, "Resources", "readDecode", json!([probe])).unwrap()["kind"],
        "image"
    );
    let saved = app.runtime().unwrap().snapshot().unwrap();
    let stale = call(
        &mut app,
        "Resources",
        "beginImage",
        json!(["pack/pixel.png"]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    app.runtime_mut().unwrap().restore(&saved).unwrap();
    assert!(call(&mut app, "Resources", "publishImage", json!([stale, 8])).is_err());
}
#[test]
fn text_preparation_reports_failure_and_cancellation_without_faulting_frames() {
    let (_dir, mut app) = app();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    assert!(
        call(&mut app, "Text", "draw", json!(["line", "hello", {}]))
            .unwrap_err()
            .to_string()
            .contains("offline-only")
    );
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let id = call(&mut app, "Text", "beginDraw", json!(["line", "hello", {}]))
        .unwrap()
        .as_u64()
        .unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(
        call(&mut app, "Text", "status", json!([id])).unwrap()["state"],
        "pending"
    );
    call(&mut app, "Text", "cancel", json!([id])).unwrap();
    gate.finish();
    let failure = call(
        &mut app,
        "Text",
        "beginMeasure",
        json!(["missing", 24, "hello"]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    assert_eq!(
        ready(&mut app, "Text", "status", failure)["state"],
        "failed"
    );
    assert_eq!(app.state(), crysirror_engine::lifecycle::AppState::Running);
}
#[test]
#[ignore = "requires an explicitly supplied system font, read only at runtime"]
fn rich_text_workers_match_offline_pixels_and_publish_events_with_ready_frames() {
    let (_dir, mut app) = app();
    let owner = app.engine().unwrap().client;
    let font = std::env::var_os("CRYSIRROR_TEST_FONT").expect("explicit system font path");
    app.runtime_mut()
        .unwrap()
        .assets
        .text
        .add_font("default", std::fs::read(font).unwrap())
        .unwrap();
    let options = json!({"layout":{"canvas":[128,64]},"style":{"fonts":["default"],"size":20,"color":[220,100,20,255],"bold":false},"raster":{"outline":{"width":1,"color":[0,0,0,255]}}});
    let path = call(
        &mut app,
        "Text",
        "draw",
        json!(["line", "[b]Hello[/b]", options]),
    )
    .unwrap()
    .as_str()
    .unwrap()
    .to_owned();
    let expected = app
        .runtime()
        .unwrap()
        .assets
        .surface(owner, &path)
        .unwrap()
        .rgba
        .clone();
    call(&mut app, "Text", "remove", json!(["line"])).unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    let id = call(
        &mut app,
        "Text",
        "beginDraw",
        json!(["line", "[b]Hello[/b]", options]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(
        call(&mut app, "Text", "status", json!([id])).unwrap()["state"],
        "pending"
    );
    assert!(
        app.runtime()
            .unwrap()
            .assets
            .surface(owner, &path)
            .is_none()
    );
    gate.finish();
    let result = ready(&mut app, "Text", "status", id);
    assert_eq!(result["result"]["surface"], path);
    assert!(
        app.runtime()
            .unwrap()
            .assets
            .surface(owner, &path)
            .unwrap()
            .rgba
            == expected,
        "async/offline rich text pixel mismatch"
    );
    call(&mut app, "Text", "cancel", json!([id])).unwrap();
    assert!(
        app.runtime()
            .unwrap()
            .assets
            .surface(owner, &path)
            .is_some()
    );
    let mut reveal_options = options.clone();
    reveal_options["graphemes_per_second"] = json!(10);
    let line = call(
        &mut app,
        "Text",
        "beginDraw",
        json!(["line", "Hello[event=done]", reveal_options]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    ready(&mut app, "Text", "status", line);
    call(&mut app, "Text", "cancel", json!([line])).unwrap();
    let before = app
        .runtime()
        .unwrap()
        .assets
        .surface(owner, &path)
        .unwrap()
        .revision;
    let gate = Gate::block(app.runtime().unwrap().assets.cpu_tasks());
    app.frame(100, InputFrame::default()).unwrap();
    app.frame(100, InputFrame::default()).unwrap();
    assert_eq!(
        app.runtime()
            .unwrap()
            .assets
            .surface(owner, &path)
            .unwrap()
            .revision,
        before
    );
    assert_eq!(
        call(&mut app, "Text", "drainEvents", json!(["line"])).unwrap(),
        json!([])
    );
    gate.finish();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.frame(0, InputFrame::default()).unwrap();
        if app
            .runtime()
            .unwrap()
            .assets
            .surface(owner, &path)
            .unwrap()
            .revision
            > before
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(
        !call(&mut app, "Text", "drainEvents", json!(["line"]))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    call(&mut app, "Text", "finish", json!(["line"])).unwrap();
    loop {
        app.frame(0, InputFrame::default()).unwrap();
        let state = call(&mut app, "Text", "state", json!(["line"])).unwrap();
        if state["render_pending"] == false {
            assert_eq!(state["visible_text"], "Hello");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let events = call(&mut app, "Text", "drainEvents", json!(["line"])).unwrap();
    assert!(events.to_string().contains("finished"));
    assert!(events.to_string().contains("done"));
}

#[test]
fn image_ready_results_obey_profile_residency_and_ui_events_stay_client_scoped() {
    let (dir, mut app) = app();
    let image = crysirror_image::ImageSurface::filled(2, 2, [255, 0, 0, 255]).unwrap();
    std::fs::write(
        dir.path().join("assets/a/large.png"),
        crysirror_image::encode_png_uncompressed(&image).unwrap(),
    )
    .unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    call(
        &mut app,
        "Resources",
        "defineProfile",
        json!(["tiny",{"limits":{"max_resident_bytes":8}}]),
    )
    .unwrap();
    let ticket = call(
        &mut app,
        "Resources",
        "beginImage",
        json!(["large.png",{"profile":"tiny"}]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    let result = ready(&mut app, "Resources", "decodeStatus", ticket);
    assert_eq!(result["state"], "failed");
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("budget exceeded")
    );
    assert!(call(&mut app, "Resources", "publishImage", json!([ticket, 3])).is_err());
    call(&mut app, "Resources", "cancelDecode", json!([ticket])).unwrap();
    let owner = app.engine().unwrap().client;
    let other = app.runtime().unwrap().client_id("b").unwrap();
    let event = |client: crysirror_core::state::ClientId| crysirror_scene::ui::UiEvent {
        widget: crysirror_scene::ui::WidgetId {
            owner: client.0,
            id: 2,
        },
        kind: "menu_activate".into(),
        value: 0.,
        menu_bar: Some(1),
    };
    app.runtime_mut().unwrap().host.engine.ui.events = vec![event(owner), event(other)];
    let cx = app.context();
    let ui = cx.singleton("Ui").unwrap();
    let events = cx.get(ui, "events").unwrap().to_json();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["widget"]["owner"], owner.0);
    cx.state().unwrap().client = other;
    let events = cx.get(ui, "events").unwrap().to_json();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["widget"]["owner"], other.0);
    let consumed = cx.call(ui, "drainEvents", &[]).unwrap().to_json();
    assert_eq!(consumed.as_array().unwrap().len(), 1);
    assert_eq!(
        cx.call(ui, "drainEvents", &[]).unwrap().to_json(),
        json!([])
    );
    cx.state().unwrap().client = owner;
    let retained = cx.call(ui, "drainEvents", &[]).unwrap().to_json();
    assert_eq!(retained.as_array().unwrap().len(), 1);
    assert_eq!(retained[0]["widget"]["owner"], owner.0);
}
