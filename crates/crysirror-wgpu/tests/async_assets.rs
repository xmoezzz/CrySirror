// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, InputFrame},
    state::ClientId,
    Error,
};
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, Renderer};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}
struct Slow {
    bytes: Vec<u8>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}
impl ClientStorage for Slow {
    fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        let (l, cv) = &*self.gate;
        let mut ready = l.lock().unwrap();
        while !*ready {
            ready = cv.wait(ready).unwrap();
        }
        Ok(self.bytes.clone())
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
#[test]
#[ignore = "requires real wgpu adapter; explicitly execute"]
fn slow_image_loading_keeps_frames_responsive_then_matches_offline_pixels() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"async-gpu","content_revision":"1","canvas":[16,16],"background":[0,0,0,255],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","program":[{"op":"sprite","id":0,"asset":"pixel.png","rect":[0,0,16,16],"z":0}]}]}"#).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let _release = Release(gate.clone());
    let bytes = include_bytes!("../../../examples/desktop/assets/a/panel.png").to_vec();
    rt.assets
        .refresh_storage(
            ClientId(1),
            Arc::new(Slow {
                bytes,
                gate: gate.clone(),
            }),
        )
        .unwrap();
    let camera:crysirror_scene::ServiceBatch=serde_json::from_value(serde_json::json!({"world":[{"op":"set_camera","camera":{"eye":[0,0,4],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":20}}]})).unwrap();rt.host.engine.submit(ClientId(1),&camera).unwrap();
    rt.assets.set_nonblocking(true);
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("GPU {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    let first = renderer
        .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 255])
        .unwrap();
    assert_eq!(first.get_pixel(8, 8).0, [0, 0, 0, 255]);
    rt.tick(16, InputFrame::default()).unwrap();
    assert_eq!(rt.now_ms(), 16);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let start = Instant::now();
    let loaded = loop {
        let image = renderer
            .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 255])
            .unwrap();
        if image != first {
            break image;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::yield_now();
    };
    rt.assets.set_nonblocking(false);
    let mut offline = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let reference = offline
        .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 255])
        .unwrap();
    assert_eq!(loaded, reference);
    assert_eq!(rt.assets.tasks_inspect()["io"]["completed"], 1);
}

#[test]
#[ignore = "requires real wgpu adapter; explicitly execute"]
fn imported_scene_uses_parallel_poses_and_matches_blocking_gpu_output() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let mut data = Vec::new();
    for v in [-1f32, -1., 0., 1., -1., 0., 0., 1., 0.] {
        data.extend(v.to_le_bytes());
    }
    for i in [0u16, 1, 2] {
        data.extend(i.to_le_bytes());
    }
    std::fs::write(t.path().join("assets/mesh.bin"), data).unwrap();
    let doc = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":42}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-1,-1,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}],"materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1],"metallicFactor":0,"roughnessFactor":1}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    std::fs::write(
        t.path().join("assets/triangle.gltf"),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&serde_json::json!({"schema":2,"name":"async-3d","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","systems":{"environment":{"temporal_aa":false,"bloom":0,"ambient_occlusion":0},"entities":[{"id":1,"name":"triangle","geometry":{"kind":"scene","asset":"triangle.gltf"}}]}}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let camera:crysirror_scene::ServiceBatch=serde_json::from_value(serde_json::json!({"world":[{"op":"set_camera","camera":{"eye":[0,0,4],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":20}}]})).unwrap();rt.host.engine.submit(ClientId(1),&camera).unwrap();
    rt.assets.set_nonblocking(true);
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let start = Instant::now();
    let loaded = loop {
        let image = renderer
            .capture(&rt.host.scene, &mut rt.assets, [64, 64], [0, 0, 0, 255])
            .unwrap();
        let pixel = image.get_pixel(32, 32).0;
        if i16::from(pixel[0]) > i16::from(pixel[1]) + 20 {
            break image;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "scene did not load"
        );
    };
    assert!(
        rt.assets.tasks_inspect()["cpu"]["completed"]
            .as_u64()
            .unwrap()
            > 0
    );
    rt.assets.set_nonblocking(false);
    let mut offline = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    offline.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let reference = offline
        .capture(&rt.host.scene, &mut rt.assets, [64, 64], [0, 0, 0, 255])
        .unwrap();
    assert_eq!(loaded, reference);
}
