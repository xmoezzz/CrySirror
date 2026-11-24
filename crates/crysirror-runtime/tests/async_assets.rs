// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{ClientStorage, InputFrame},
    state::ClientId,
};
use crysirror_runtime::{
    Runtime,
    assets::{AssetKind, AssetState, Assets},
};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Slow {
    started: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}
impl ClientStorage for Slow {
    fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        if let Some(tx) = self.started.lock().unwrap().take() {
            tx.send(()).unwrap();
        }
        let (lock, cv) = &*self.gate;
        let mut ready = lock.lock().unwrap();
        while !*ready {
            ready = cv.wait(ready).unwrap();
        }
        Ok(b"old bytes".to_vec())
    }
    fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![])
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
    fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
}
struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        release(&self.0)
    }
}
fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}
fn ready(
    assets: &mut Assets,
    ticket: crysirror_runtime::assets::AssetTicket,
) -> crysirror_runtime::assets::AssetStatus {
    let start = Instant::now();
    loop {
        let s = assets.task_status(ClientId(ticket.client), ticket).unwrap();
        if s.state != AssetState::Loading {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }
}
#[test]
fn slow_io_does_not_block_runtime_and_stale_generation_cannot_publish() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"async","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"}]}"#).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    rt.assets.set_nonblocking(true);
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let _release = Release(gate.clone());
    let storage = Arc::new(Slow {
        started: Mutex::new(Some(tx)),
        gate: gate.clone(),
    });
    rt.assets
        .refresh_storage(ClientId(1), storage.clone())
        .unwrap();
    let ticket = rt
        .assets
        .request(ClientId(1), "asset.bin", AssetKind::Bytes)
        .unwrap();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        rt.assets.task_status(ClientId(1), ticket).unwrap().state,
        AssetState::Loading
    );
    for _ in 0..100 {
        rt.tick(16, InputFrame::default()).unwrap();
    }
    assert_eq!(rt.now_ms(), 1600);
    rt.assets.refresh_storage(ClientId(1), storage).unwrap();
    assert!(rt.assets.task_status(ClientId(1), ticket).is_err());
    release(&gate);
    let new = rt
        .assets
        .request(ClientId(1), "asset.bin", AssetKind::Bytes)
        .unwrap();
    assert_ne!(new.id, ticket.id);
    assert_eq!(ready(&mut rt.assets, new).state, AssetState::Ready);
    assert_eq!(
        rt.assets
            .read_for(ClientId(1), "asset.bin")
            .unwrap()
            .as_ref(),
        b"old bytes"
    );
}
#[test]
fn reads_are_real_futures_scoped_deduplicated_and_failures_are_visible() {
    let t = tempfile::tempdir().unwrap();
    for (name, data) in [("a", b"one"), ("b", b"two")] {
        std::fs::create_dir(t.path().join(name)).unwrap();
        std::fs::write(t.path().join(name).join("same"), data).unwrap();
    }
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "a").unwrap();
    a.register(ClientId(2), "b").unwrap();
    let job = a.read_async(ClientId(1), "same").unwrap();
    assert_eq!(job.wait().unwrap().as_ref(), b"one");
    let first = a.request(ClientId(1), "same", AssetKind::Bytes).unwrap();
    assert_eq!(
        first,
        a.request(ClientId(1), "same", AssetKind::Bytes).unwrap()
    );
    let other = a.request(ClientId(2), "same", AssetKind::Bytes).unwrap();
    assert!(a.task_status(ClientId(2), first).is_err());
    let forged = crysirror_runtime::assets::AssetTicket { client: 1, ..other };
    assert!(a.task_status(ClientId(1), forged).is_err());
    assert!(a.release_task(ClientId(1), forged).is_err());
    assert_eq!(ready(&mut a, other).state, AssetState::Ready);
    assert_eq!(a.read_for(ClientId(2), "same").unwrap().as_ref(), b"two");
    let bad = a.request(ClientId(1), "missing", AssetKind::Image).unwrap();
    let status = ready(&mut a, bad);
    assert_eq!(status.state, AssetState::Failed);
    assert!(status.error.is_some());
    assert!(
        a.request(ClientId(1), "../escape", AssetKind::Bytes)
            .is_err()
    );
    a.release_task(ClientId(1), bad).unwrap();
    assert!(a.task_status(ClientId(1), bad).is_err());
}
struct Decoder(Arc<AtomicUsize>);
impl crysirror_image::ImageDecoder for Decoder {
    fn decode(&self, _: &[u8]) -> anyhow::Result<crysirror_image::ImageSurface> {
        assert!(
            std::thread::current()
                .name()
                .unwrap_or("")
                .starts_with("crysirror-assets-")
        );
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(crysirror_image::ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![10, 20, 30, 255],
        })
    }
}
#[test]
fn renderer_image_and_model_import_use_background_common_cache() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("pixel.fake"), b"fixture").unwrap();
    std::fs::write(
        t.path().join("model.glb"),
        include_bytes!("../../../examples/engine/assets/lantern.glb"),
    )
    .unwrap();
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "").unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    a.decoders
        .register("fake", Arc::new(Decoder(count.clone())))
        .unwrap();
    assert!(a.prefetch_image(ClientId(1), "pixel.fake").unwrap());
    assert!(a.prefetch_image(ClientId(1), "pixel.fake").unwrap());
    let image = a.image_for(ClientId(1), "pixel.fake").unwrap().unwrap();
    assert_eq!(image.rgba, [10, 20, 30, 255]);
    a.image_for(ClientId(1), "pixel.fake").unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    let model = a.model_for(ClientId(1), "model.glb").unwrap().unwrap();
    assert!(!model.meshes.is_empty());
    assert!(a.tasks_inspect()["io"]["completed"].as_u64().unwrap() >= 2);
}

#[cfg(feature = "arc-formats")]
#[test]
fn tlg_metadata_prefetch_and_visible_lookup_share_prepared_pixels() {
    use crysirror_core::service::HostServices;

    struct TlgSource {
        encoded: Vec<u8>,
        reads: AtomicUsize,
    }

    impl ClientStorage for TlgSource {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.encoded.clone())
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
    }

    fn source(blue: u8, alpha: u8) -> Arc<TlgSource> {
        let mut encoded = b"TLG5.0\0raw\x1a\x04".to_vec();
        for value in [1u32, 1, 1, 0] {
            encoded.extend(value.to_le_bytes());
        }
        for value in [blue, 2, 1, alpha] {
            encoded.extend([1, 1, 0, 0, 0, value]);
        }
        Arc::new(TlgSource { encoded, reads: AtomicUsize::new(0) })
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let project = dir.path().join("project.json");
    std::fs::write(&project, br#"{"schema":2,"name":"tlg cache","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"}]}"#).unwrap();
    let mut runtime = Runtime::load(&project, &dir.path().join("state")).unwrap();
    let client = ClientId(1);
    let path = "picture.TLG";
    let meta = runtime.host.image_metadata(client).unwrap();
    for (prefetch_first, blue, alpha) in [(false, 3, 255), (true, 6, 127)] {
        let source = source(blue, alpha);
        runtime.assets.refresh_storage(client, source.clone()).unwrap();
        runtime.assets.set_nonblocking(true);
        if prefetch_first {
            assert!(runtime.assets.prefetch_image(client, path).unwrap());
        }
        let started = Instant::now();
        while !meta.prepare_metadata(path).unwrap() {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert_eq!(meta.size(path).unwrap(), [1, 1]);
        assert_eq!(meta.opaque(path).unwrap(), alpha == 255);
        assert_eq!(meta.fingerprint(path).unwrap(), *blake3::hash(&source.encoded).as_bytes());
        assert!(runtime.assets.prefetch_image(client, path).unwrap());
        let image = runtime.assets.image_for(client, path).unwrap().unwrap();
        assert_eq!(image.rgba, [3, 2, blue + 2, alpha]);
        assert_eq!(source.reads.load(Ordering::Relaxed), 1);
        assert_eq!(runtime.assets.tasks_inspect()["resident_bytes"], 4);
    }
}
#[test]
fn cancelling_running_asset_prevents_publication() {
    let t = tempfile::tempdir().unwrap();
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let _release = Release(gate.clone());
    a.refresh_storage(
        ClientId(1),
        Arc::new(Slow {
            started: Mutex::new(Some(tx)),
            gate: gate.clone(),
        }),
    )
    .unwrap();
    let ticket = a.request(ClientId(1), "same", AssetKind::Bytes).unwrap();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    a.cancel_task(ClientId(1), ticket).unwrap();
    release(&gate);
    assert_eq!(
        a.task_status(ClientId(1), ticket).unwrap().state,
        AssetState::Cancelled
    );
    assert_eq!(a.tasks_inspect()["resident_bytes"], 0);
}

#[test]
fn renderer_defers_backpressure_and_ready_tickets_do_not_consume_loading_quota() {
    let t = tempfile::tempdir().unwrap();
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "").unwrap();
    a.set_nonblocking(true);
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let _release = Release(gate.clone());
    a.refresh_storage(
        ClientId(1),
        Arc::new(Slow {
            started: Mutex::new(Some(tx)),
            gate: gate.clone(),
        }),
    )
    .unwrap();
    let mut tickets = vec![];
    for index in 0..32 {
        tickets.push(
            a.request(ClientId(1), &format!("asset{index}"), AssetKind::Bytes)
                .unwrap(),
        );
    }
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        a.request(ClientId(1), "extra", AssetKind::Bytes)
            .unwrap_err()
            .is::<crysirror_tasks::CapacityExceeded>()
    );
    assert!(a.image_for(ClientId(1), "later.png").unwrap().is_none());
    release(&gate);
    for ticket in tickets {
        assert_eq!(ready(&mut a, ticket).state, AssetState::Ready);
    }
    assert!(a.request(ClientId(1), "extra", AssetKind::Bytes).is_ok());
}

#[test]
fn streaming_leases_pin_shared_models_and_evict_only_after_gpu_style_references_drop() {
    let t = tempfile::tempdir().unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/engine/assets/lantern.glb"),
        t.path().join("lantern.glb"),
    )
    .unwrap();
    let mut assets = Assets::new(t.path()).unwrap();
    assets.register(ClientId(1), "").unwrap();
    let lease = assets
        .lease(ClientId(1), "lantern.glb", AssetKind::Level)
        .unwrap();
    let ticket = lease.ticket();
    assert_eq!(ready(&mut assets, ticket).state, AssetState::Ready);
    assert!(assets.release_task(ClientId(1), ticket).is_err());
    let model = assets
        .model_for(ClientId(1), "lantern.glb")
        .unwrap()
        .unwrap();
    assert_eq!(
        assets.tasks_inspect()["assets"].as_array().unwrap().len(),
        1,
        "level shares decoded model with renderer"
    );
    drop(lease);
    assert_eq!(assets.collect_unused(), 0, "renderer still holds the model");
    drop(model);
    assert_eq!(assets.collect_unused(), 1);
    assert!(assets.task_status(ClientId(1), ticket).is_err());
    let lease = assets
        .lease(ClientId(1), "lantern.glb", AssetKind::Bytes)
        .unwrap();
    let explicit = assets
        .request(ClientId(1), "lantern.glb", AssetKind::Bytes)
        .unwrap();
    ready(&mut assets, explicit);
    drop(lease);
    assert_eq!(
        assets.collect_unused(),
        0,
        "explicit caller retains its request"
    );
    assets.release_task(ClientId(1), explicit).unwrap();
}

#[test]
fn released_streaming_lease_cancels_pending_publication() {
    let t = tempfile::tempdir().unwrap();
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let guard = Release(gate.clone());
    a.refresh_storage(
        ClientId(1),
        Arc::new(Slow {
            started: Mutex::new(Some(tx)),
            gate,
        }),
    )
    .unwrap();
    let lease = a.lease(ClientId(1), "slow", AssetKind::Bytes).unwrap();
    let ticket = lease.ticket();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(lease);
    assert_eq!(a.collect_unused(), 1);
    assert!(a.task_status(ClientId(1), ticket).is_err());
    drop(guard);
}

#[test]
fn host_asset_budget_is_bounded_and_cannot_drop_below_live_residency() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("blob"), vec![7u8; 17 * 1024 * 1024]).unwrap();
    let mut a = Assets::new(t.path()).unwrap();
    a.register(ClientId(1), "").unwrap();
    assert!(a.set_prepared_budget(1).is_err());
    if cfg!(all(target_pointer_width = "64", not(target_family = "wasm"),
        any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "ios", target_os = "android"))) {
        assert!(a.tasks_inspect()["budget_bytes"].is_null());
    } else {
        assert_eq!(a.tasks_inspect()["budget_bytes"], 256 * 1024 * 1024);
    }
    a.set_prepared_budget(32 * 1024 * 1024).unwrap();
    let t = a.request(ClientId(1), "blob", AssetKind::Bytes).unwrap();
    assert_eq!(ready(&mut a, t).state, AssetState::Ready);
    assert!(a.set_prepared_budget(16 * 1024 * 1024).is_err());
    a.release_task(ClientId(1), t).unwrap();
    a.set_prepared_budget(16 * 1024 * 1024).unwrap();
}

#[test]
fn image_sequence_reclaims_old_frames_under_pressure_and_preserves_owners() {
    const FRAME_BYTES: usize = 1024 * 1024 * 4;
    let root = tempfile::tempdir().unwrap();
    let client = ClientId(1);
    let mut assets = Assets::new(root.path()).unwrap();
    assets.register(client, "").unwrap();
    assets.set_prepared_budget(5 * FRAME_BYTES).unwrap();
    for index in 0..8 {
        let image = crysirror_image::ImageSurface {
            width: 1024,
            height: 1024,
            rgba: [index, 0, 0, 255].repeat(1024 * 1024),
        };
        std::fs::write(
            root.path().join(format!("{index}.png")),
            crysirror_image::encode_png(&image).unwrap(),
        )
        .unwrap();
    }

    let explicit = assets.request(client, "0.png", AssetKind::Image).unwrap();
    let lease = assets.lease(client, "1.png", AssetKind::Image).unwrap();
    assert_eq!(
        assets.wait_task(client, explicit).unwrap().state,
        AssetState::Ready
    );
    assert_eq!(
        assets.wait_task(client, lease.ticket()).unwrap().state,
        AssetState::Ready
    );
    let held = assets.image_for(client, "2.png").unwrap().unwrap();
    for index in [3, 4, 3, 5] {
        let image = assets
            .image_for(client, &format!("{index}.png"))
            .unwrap()
            .unwrap();
        assert_eq!(image.rgba[..4], [index, 0, 0, 255]);
    }
    let stats = assets.tasks_inspect();
    let paths: Vec<_> = stats["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["0.png", "1.png", "2.png", "3.png", "5.png"]);
    assert_eq!(stats["resident_bytes"], 5 * FRAME_BYTES);

    let recent = assets.image_for(client, "5.png").unwrap().unwrap();
    let reloaded = assets.image_for(client, "4.png").unwrap().unwrap();
    assert_eq!(reloaded.rgba[..4], [4, 0, 0, 255]);
    let error = assets.image_for(client, "6.png").unwrap_err();
    assert!(error.to_string().contains("prepared assets exceed"));
    assert_eq!(
        assets.wait_task(client, explicit).unwrap().state,
        AssetState::Ready
    );
    assert_eq!(
        assets.wait_task(client, lease.ticket()).unwrap().state,
        AssetState::Ready
    );
    assert_eq!(held.rgba[..4], [2, 0, 0, 255]);
    assert_eq!(recent.rgba[..4], [5, 0, 0, 255]);
    drop(held);
    assets.set_nonblocking(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    let next = loop {
        if let Some(image) = assets.image_for(client, "7.png").unwrap() {
            break image;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(next.rgba[..4], [7, 0, 0, 255]);
    assert_eq!(assets.tasks_inspect()["resident_bytes"], 5 * FRAME_BYTES);
    }

    #[test]
fn image_admission_reclaims_only_the_needed_unowned_cache_entries() {
    struct SizedImage;
    impl crysirror_image::ImageDecoder for SizedImage {
        fn decode(&self, data: &[u8]) -> anyhow::Result<crysirror_image::ImageSurface> {
            let height = u32::from(data[0]) * 256;
            Ok(crysirror_image::ImageSurface {
                width: 1024,
                height,
                rgba: vec![0; height as usize * 4096],
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut assets = Assets::new(dir.path()).unwrap();
    let client = ClientId(1);
    assets.register(client, "").unwrap();
    assets.set_prepared_budget(16 << 20).unwrap();
    assets
        .decoders
        .register("sized", Arc::new(SizedImage))
        .unwrap();
    for (name, mib) in [
        ("held", 4),
        ("pinned", 4),
        ("explicit", 2),
        ("cold", 3),
        ("warm", 3),
        ("next", 3),
        ("too-large", 17),
        ("blocked", 8),
    ] {
        std::fs::write(dir.path().join(format!("{name}.sized")), [mib]).unwrap();
    }
    let held = assets.image_for(client, "held.sized").unwrap().unwrap();
    let lease = assets
        .lease(client, "pinned.sized", AssetKind::Image)
        .unwrap();
    assets.wait_task(client, lease.ticket()).unwrap();
    let explicit = assets
        .request(client, "explicit.sized", AssetKind::Image)
        .unwrap();
    assets.wait_task(client, explicit).unwrap();
    for name in ["cold.sized", "warm.sized"] {
        drop(assets.image_for(client, name).unwrap().unwrap());
    }
    let paths = |a: &Assets| -> Vec<String> {
        a.tasks_inspect()["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["state"] == "ready")
            .map(|s| s["path"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = paths(&assets);
    assert!(assets.image_for(client, "too-large.sized").is_err());
    assert!(assets.image_for(client, "blocked.sized").is_err());
    assert_eq!(
        paths(&assets),
        before,
        "unfulfillable requests preserve cached work"
    );
    assert!(assets.prefetch_image(client, "next.sized").unwrap());
    assets.image_for(client, "next.sized").unwrap().unwrap();
    assert_eq!(
        paths(&assets),
        [
            "held.sized",
            "pinned.sized",
            "explicit.sized",
            "warm.sized",
            "next.sized"
        ]
    );
    assert_eq!(assets.tasks_inspect()["resident_bytes"], 16 << 20);
    assert!(Arc::ptr_eq(
        &held,
        &assets.image_for(client, "held.sized").unwrap().unwrap()
    ));
    assert_eq!(
        assets.task_status(client, lease.ticket()).unwrap().state,
        AssetState::Ready
    );
    assert_eq!(
        assets.task_status(client, explicit).unwrap().state,
        AssetState::Ready
    );
    drop(held);
    // A transient admission failure from a prefetch/renderer request must
    // recover when enough residency becomes reclaimable.
    assets.image_for(client, "blocked.sized").unwrap().unwrap();
    assert!(assets.tasks_inspect()["resident_bytes"].as_u64().unwrap() <= 16 << 20);
}

#[test]
fn querying_an_unpinned_ticket_does_not_fail_an_unrelated_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut assets = Assets::new(dir.path()).unwrap();
    let client = ClientId(1);
    assets.register(client, "").unwrap();
    assets.set_prepared_budget(16 << 20).unwrap();
    for path in ["first.bin", "second.bin"] {
        std::fs::write(dir.path().join(path), vec![0; 9 << 20]).unwrap();
    }
    let lease = assets.lease(client, "first.bin", AssetKind::Bytes).unwrap();
    let first = lease.ticket();
    assert_eq!(
        assets.wait_task(client, first).unwrap().state,
        AssetState::Ready
    );
    drop(lease);
    let second = assets
        .request(client, "second.bin", AssetKind::Bytes)
        .unwrap();
    let start = Instant::now();
    while assets.io_tasks().stats().completed < 2 {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }
    assert_eq!(
        assets.task_status(client, first).unwrap().state,
        AssetState::Ready
    );
    assert_eq!(
        assets.wait_task(client, second).unwrap().state,
        AssetState::Ready
    );
    assert!(assets.task_status(client, first).is_err());
}
