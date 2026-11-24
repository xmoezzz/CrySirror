// SPDX-License-Identifier: MPL-2.0
//! Exercise the shared prepared cache at its desktop budget using generated assets.
use crysirror_core::state::ClientId;
use crysirror_runtime::{
    Runtime,
    assets::{AssetKind, AssetState},
};
use crysirror_wgpu::{Gpu, Renderer};
use image::{Rgba, RgbaImage};
use serde_json::json;
use std::{fs, sync::Arc};

#[test]
#[ignore = "requires a wgpu adapter and a 256 MiB prepared cache"]
fn render_with_a_full_prepared_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets = dir.path().join("assets");
    fs::create_dir(&assets).unwrap();
    let colors = [
        [200, 40, 60, 255],
        [50, 175, 90, 255],
        [25, 50, 155, 255],
        [180, 155, 60, 255],
        [35, 170, 180, 255],
        [170, 70, 140, 255],
    ];
    let mut layers = Vec::new();
    for (i, color) in colors.iter().enumerate() {
        let name = format!("stripe{i}.png");
        RgbaImage::from_pixel(1024, 1024, Rgba(*color))
            .save(assets.join(&name))
            .unwrap();
        layers.push(json!({
            "id": i + 1, "rect": [32 * i, 0, 32, 48],
            "paint": {"kind": "image", "asset": name}
        }));
    }
    let canvas = [192, 48];
    let project_path = dir.path().join("project.json");
    let project = json!({
        "schema": 2, "name": "cache pressure", "content_revision": "1", "canvas": canvas,
        "assets": "assets", "clients": [{"name": "app", "game": "fixture",
            "adapter": "crysirror-script-v1", "systems": {"layers": layers}}]
    });
    fs::write(&project_path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut rt = Runtime::load(&project_path, &dir.path().join("state")).unwrap();

    // Leave room for four of the six 4 MiB decoded images. The five
    // explicitly owned requests must survive all renderer lookups.
    let client = ClientId(1);
    let mut tickets = Vec::new();
    for i in 0..5 {
        let name = format!("explicit{i}.bin");
        fs::File::create(assets.join(&name))
            .unwrap()
            .set_len(48 << 20)
            .unwrap();
        let ticket = rt.assets.request(client, &name, AssetKind::Bytes).unwrap();
        assert_eq!(
            rt.assets.wait_task(client, ticket).unwrap().state,
            AssetState::Ready
        );
        tickets.push(ticket);
    }
    rt.tick(0, Default::default()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("Prepared-cache test adapter: {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer
        .set_engine_frame(rt.host.engine.frame(canvas).unwrap())
        .unwrap();
    let frame = renderer
        .capture(&rt.host.scene, &mut rt.assets, canvas, [0; 4])
        .unwrap();
    for (x, _, pixel) in frame.enumerate_pixels() {
        assert_eq!(pixel.0, colors[x as usize / 32]);
    }
    for ticket in tickets {
        assert_eq!(
            rt.assets.task_status(client, ticket).unwrap().state,
            AssetState::Ready
        );
    }
    let stats = rt.assets.tasks_inspect();
    assert_eq!(stats["budget_bytes"], 256 << 20);
    assert!(stats["resident_bytes"].as_u64().unwrap() <= 256 << 20);
}
