// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Public engine-only frozen Canvas snapshots; no native VM or game data.
use crysirror_core::state::ClientId;
use crysirror_runtime::Runtime;
use crysirror_scene::{
    Primitive, ServiceBatch,
    layer::{Paint, rectangle},
};
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture() -> (tempfile::TempDir, Runtime, Renderer) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(root.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"immutable-snapshot","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]})).unwrap()).unwrap();
    let rt = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("Canvas snapshot GPU: {:?}", gpu.adapter.get_info());
    (
        root,
        rt,
        Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb),
    )
}
fn submit(rt: &mut Runtime, value: serde_json::Value) {
    let batch: ServiceBatch = serde_json::from_value(value).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
}
fn capture(rt: &mut Runtime, r: &mut Renderer) -> image::RgbaImage {
    r.set_engine_frame(rt.host.engine.frame([16, 16]).unwrap())
        .unwrap();
    r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4])
        .unwrap()
}
fn bakes(r: &mut Renderer) -> u64 {
    r.profile_report().unwrap()["snapshot_rasterizations"]
        .as_u64()
        .unwrap()
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn copied_subtree_is_one_frozen_original_resolution_texture_under_opacity_and_scale() {
    let (_root, mut rt, mut r) = fixture();
    submit(
        &mut rt,
        json!({"layers":[
            {"op":"put","layer":{"id":1,"rect":[0,0,8,8],"order":[0,0],"paint":{"kind":"solid","color":[255,0,0,255]}}},
            {"op":"put","layer":{"id":2,"parent":1,"rect":[2,2,4,4],"order":[0,1],"paint":{"kind":"solid","color":[0,255,0,255]}}},
            {"op":"put","layer":{"id":100,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]}}}
        ]}),
    );
    submit(
        &mut rt,
        json!({"canvases":[
            {"op":"create","id":10,"width":8,"height":8},
            {"op":"clear","id":10,"color":[0,0,0,0]},
            {"op":"present","id":10,"layer":10,"parent":100,"order":10},
            {"op":"copy_subtree","source":1,"destination":10,"rect":[0,0,8,8],"position":[0,0]}
        ]}),
    );
    submit(
        &mut rt,
        json!({"layers":[{"op":"set_visible","id":1,"visible":false},{"op":"set_opacity","id":10,"opacity":128}]}),
    );
    let half = capture(&mut rt, &mut r);
    let overlap = half.get_pixel(3, 3).0;
    assert!(
        overlap[0] <= 1 && overlap[1].abs_diff(188) <= 2 && overlap[2] <= 1 && overlap[3] == 128,
        "per-child alpha leaked color: {overlap:?}"
    );
    assert_eq!(bakes(&mut r), 1);
    let frozen = rt
        .host
        .engine
        .frame([16, 16])
        .unwrap()
        .into_iter()
        .find(|p| matches!(p.paint, Paint::Snapshot { .. }))
        .unwrap()
        .paint;
    if let Paint::Snapshot { size, .. } = &frozen {
        assert_eq!(*size, [8, 8]);
    } else {
        panic!()
    }
    // Changing source state cannot change the captured graph or its GPU cache.
    submit(
        &mut rt,
        json!({"layers":[{"op":"put","layer":{"id":2,"parent":1,"rect":[2,2,4,4],"order":[0,1],"paint":{"kind":"solid","color":[0,0,255,255]}}}]}),
    );
    assert_eq!(capture(&mut rt, &mut r), half);
    assert_eq!(bakes(&mut r), 1);
    submit(
        &mut rt,
        json!({"layers":[{"op":"set_transform","id":10,"transform":{"scale_x":2,"scale_y":2}}]}),
    );
    let scaled = capture(&mut rt, &mut r);
    assert_eq!(scaled.get_pixel(7, 7).0, overlap);
    assert_eq!(bakes(&mut r), 1, "outer scaling rebaked snapshot");
    submit(
        &mut rt,
        json!({"layers":[{"op":"set_opacity","id":100,"opacity":128}]}),
    );
    let quarter = capture(&mut rt, &mut r);
    let p = quarter.get_pixel(7, 7).0;
    assert!(
        p[0] <= 1 && p[1].abs_diff(137) <= 2 && p[2] <= 1 && p[3] == 64,
        "ancestor opacity leaked: {p:?}"
    );
    assert_eq!(bakes(&mut r), 1);
    let now = rt
        .host
        .engine
        .frame([16, 16])
        .unwrap()
        .into_iter()
        .find(|p| matches!(p.paint, Paint::Snapshot { .. }))
        .unwrap()
        .paint;
    assert_eq!(
        serde_json::to_value(frozen).unwrap(),
        serde_json::to_value(now).unwrap()
    );
}
fn snapshot_image(name: &str) -> Primitive {
    let layer=serde_json::from_value(json!({"id":1,"rect":[0,0,4,4],"paint":{"kind":"image","asset":name,"tint":[255,255,255,255]}})).unwrap();
    rectangle(
        1,
        50,
        0,
        [0., 0., 4., 4.],
        Paint::Snapshot {
            size: [4, 4],
            layers: vec![layer].into(),
        },
        None,
        [0, 0],
    )
}
fn png(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 4, image::Rgba(color))
        .write_to(&mut bytes, image::ImageOutputFormat::Png)
        .unwrap();
    bytes.into_inner()
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn only_registered_immutable_images_can_publish_and_pending_is_not_cached_blank() {
    let (root, mut rt, mut r) = fixture();
    let name = "__crysirror/snapshot-blue.png";
    rt.assets
        .register_generated(ClientId(1), name, png([0, 0, 255, 255]))
        .unwrap();
    rt.assets.set_nonblocking(true);
    // Occupy the engine IO pool until one frame has observed Loading. This
    // proves the pending branch without relying on decoder/thread timing.
    let pool = rt.assets.io_tasks();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let (started, arrivals) = std::sync::mpsc::channel();
    let mut blockers = Vec::new();
    for _ in 0..pool.stats().workers {
        let gate = gate.clone();
        let started = started.clone();
        blockers.push(
            pool.spawn(move |_| {
                started.send(()).unwrap();
                let (lock, wake) = &*gate;
                let _held = wake
                    .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(30), |released| {
                        !*released
                    })
                    .unwrap();
                Ok(())
            })
            .unwrap(),
        );
    }
    for _ in 0..blockers.len() {
        arrivals.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    r.set_engine_frame(vec![snapshot_image(name)]).unwrap();
    let pending = r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4]);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    for task in blockers {
        task.wait().unwrap();
    }
    assert_eq!(pending.unwrap().get_pixel(1, 1).0, [0; 4]);
    assert_eq!(
        bakes(&mut r),
        0,
        "Loading was published as a completed blank snapshot"
    );
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        r.set_engine_frame(vec![snapshot_image(name)]).unwrap();
        let shot = r
            .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4])
            .unwrap();
        if shot.get_pixel(1, 1).0 == [0, 0, 255, 255] {
            assert_eq!(bakes(&mut r), 1);
            break;
        }
        assert_eq!(bakes(&mut r), 0, "pending frame published a blank product");
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
    assert!(
        rt.assets
            .register_generated(ClientId(1), name, png([255, 0, 0, 255]))
            .is_err()
    );
    let dynamic = rt
        .assets
        .put_surface(ClientId(1), 1, 4, 4, vec![255; 64])
        .unwrap();
    std::fs::create_dir_all(root.path().join("assets/__crysirror")).unwrap();
    std::fs::write(
        root.path().join("assets/__crysirror/not-published.png"),
        png([255; 4]),
    )
    .unwrap();
    std::fs::write(root.path().join("assets/plain.png"), png([255; 4])).unwrap();
    // A plain project PNG is self-contained and pinned by content, so it may
    // enter a snapshot; unpublished engine names and dynamic surfaces may not.
    r.set_engine_frame(vec![snapshot_image("plain.png")]).unwrap();
    assert!(r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4]).is_ok());
    for forbidden in ["__crysirror/not-published.png", &dynamic] {
        r.set_engine_frame(vec![snapshot_image(forbidden)]).unwrap();
        assert!(
            r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4])
                .is_err(),
            "unproved resource {forbidden}"
        );
    }
    r.set_engine_frame(vec![snapshot_image(name)]).unwrap();
    assert_eq!(
        r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4])
            .unwrap()
            .get_pixel(1, 1)
            .0,
        [0, 0, 255, 255]
    );
    // The plain PNG snapshot and this one.
    assert_eq!(bakes(&mut r), 2);
}

#[test]
#[ignore = "requires a real wgpu adapter and a system font; execute explicitly"]
fn frozen_canvas_text_rasterizes_from_the_shared_font() {
    let Ok(font) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") else {
        eprintln!("no system font; skipped");
        return;
    };
    let (_root, mut rt, mut r) = fixture();
    rt.assets.text.add_font("ui", font).unwrap();
    submit(
        &mut rt,
        json!({"layers":[{"op":"put","layer":{"id":100,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]}}}]}),
    );
    submit(
        &mut rt,
        json!({"canvases":[
            {"op":"create","id":10,"width":16,"height":16},
            {"op":"clear","id":10,"color":[0,0,0,255]},
            {"op":"draw","id":10,"layer":{"id":0,"rect":[0,0,16,16],"paint":{"kind":"text","text":"M","font":"ui","size":14,"color":[255,255,255,255],"layout":"simple-horizontal-v1"}}},
            {"op":"fade","id":10,"rect":[0,0,16,16],"opacity":200},
            {"op":"present","id":10,"layer":10,"parent":100,"order":10}
        ]}),
    );
    let frame = capture(&mut rt, &mut r);
    assert!(frame.pixels().any(|p| p.0[0] > 64), "frozen text is missing");
    assert_eq!(bakes(&mut r), 1);
}
