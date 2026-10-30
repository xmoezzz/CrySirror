//! Real sampler aliases over a shared engine image; no native game resources.
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::ServiceBatch;
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::sync::Arc;
fn submit(rt: &mut Runtime, batch: serde_json::Value) {
    let batch: ServiceBatch = serde_json::from_value(batch).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
}
fn capture(rt: &mut Runtime, renderer: &mut Renderer) -> image::RgbaImage {
    renderer
        .set_engine_frame(rt.host.engine.frame([16, 4]).unwrap())
        .unwrap();
    renderer
        .capture(&rt.host.scene, &mut rt.assets, [16, 4], [0; 4])
        .unwrap()
}
#[test]
#[ignore = "requires an actual wgpu adapter; execute explicitly"]
fn repeat_mirror_and_default_are_independent_for_the_same_asset() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let file = dir.path().join("project.json");
    std::fs::write(&file,serde_json::to_vec(&json!({"schema":2,"name":"image-sampling","content_revision":"1","canvas":[16,4],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(&file, &dir.path().join("state")).unwrap();
    let path = rt
        .host
        .upload_rgba(ClientId(1), 1, 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255])
        .unwrap();
    // Resource creation is owner-published at the normal runtime boundary.
    rt.tick(0,Default::default()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("image sampling GPU: {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    submit(
        &mut rt,
        json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,8,4],"paint":{"kind":"image","asset":path.relative()}}},{"op":"put","layer":{"id":2,"rect":[8,0,8,4],"paint":{"kind":"image","asset":path.relative()},"uv":[0,0,2,1],"sampling":{"address_u":"repeat"}}}]}),
    );
    let output = capture(&mut rt, &mut renderer);
    assert!(
        output.get_pixel(0, 1)[0] > 250 && output.get_pixel(0, 1)[2] < 3,
        "default sampler changed: {:?}",
        output.get_pixel(0, 1)
    );
    assert!(output.get_pixel(7, 1)[2] > 250);
    for x in 0..4 {
        let a = output.get_pixel(8 + x, 1).0;
        let b = output.get_pixel(12 + x, 1).0;
        for (a, b) in a.into_iter().zip(b) {
            assert!(a.abs_diff(b) <= 2, "repeat period differs at {x}: {a} {b}");
        }
    }
    submit(
        &mut rt,
        json!({"layers":[{"op":"remove","id":1},{"op":"put","layer":{"id":2,"rect":[0,0,16,4],"paint":{"kind":"image","asset":path.relative()},"uv":[0,0,2,1],"sampling":{"address_u":"mirror_repeat"}}}]}),
    );
    let mirrored = capture(&mut rt, &mut renderer);
    for x in 0..8 {
        let a = mirrored.get_pixel(x, 1).0;
        let b = mirrored.get_pixel(15 - x, 1).0;
        for (a, b) in a.into_iter().zip(b) {
            assert!(a.abs_diff(b) <= 2, "mirror differs at {x}: {a} {b}");
        }
    }
    // Reuse the original image after aliases have populated the cache.
    submit(
        &mut rt,
        json!({"layers":[{"op":"remove","id":2},{"op":"put","layer":{"id":1,"rect":[0,0,8,4],"paint":{"kind":"image","asset":path.relative()}}}]}),
    );
    let again = capture(&mut rt, &mut renderer);
    assert_eq!(again.get_pixel(0, 1), output.get_pixel(0, 1));
    assert_eq!(again.get_pixel(7, 1), output.get_pixel(7, 1));
}
