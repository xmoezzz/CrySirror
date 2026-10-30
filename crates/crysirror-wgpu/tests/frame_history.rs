//! Shared mesh history exercised without a native VM or proprietary assets.
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_puppet::{FramePart, PartBlend, PuppetFrame};
use crysirror_runtime::Runtime;
use crysirror_scene::{
    layer::{rectangle, Paint},
    Primitive,
};
use crysirror_wgpu::{Gpu, Renderer};
use std::sync::Arc;
fn frame(texture: String, color: [f32; 4]) -> Arc<PuppetFrame> {
    Arc::new(PuppetFrame {
        size: [4., 2.],
        textures: vec![texture],
        parts: vec![FramePart {
            id: "quad".into(),
            texture: 0,
            positions: vec![[0., 0.], [4., 0.], [0., 2.], [4., 2.]],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            indices: vec![0, 1, 2, 2, 1, 3],
            colors: vec![color; 4],
            blend: PartBlend::Normal,
            masks: vec![],
            invert_mask: false,
            visible: true,
        }],
    })
}
fn layer(mesh: Arc<PuppetFrame>) -> Primitive {
    let mut primitive = rectangle(
        1,
        7,
        0,
        [0., 0., 4., 2.],
        Paint::Solid { color: [0; 4] },
        None,
        [0, 0],
    );
    primitive.puppet = Some(mesh);
    primitive
}
#[test]
#[ignore = "requires an actual GPU; execute explicitly"]
fn mesh_history_is_gpu_resident_scoped_and_advances_once_per_declaration() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let project = serde_json::json!({"schema":2,"name":"history","content_revision":"1","assets":"assets","canvas":[4,2],"clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]});
    std::fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut rt = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("mesh history GPU: {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let provider = renderer.mesh_frame_history_provider();
    rt.host.mesh_frame_history = Some(provider.clone());
    assert!(rt.host.mesh_frame_history(ClientId(999)).is_err());
    let history = rt.host.mesh_frame_history(ClientId(1)).unwrap().unwrap();
    let path = history.texture(7, [4, 2]).unwrap();
    assert!(history.texture(8, [0, 2]).is_err());
    assert!(history.readback(7, [4, 2]).is_err());
    let foreign = provider.clone().client(ClientId(2)).unwrap();
    foreign.texture(7, [4, 2]).unwrap();
    assert!(foreign.readback(7, [4, 2]).is_err());
    assert!(rt
        .host
        .present_puppet_frame(
            ClientId(1),
            7,
            &serde_json::to_vec(
                frame(crysirror_core::service::frame_history_path(88), [1.; 4]).as_ref()
            )
            .unwrap()
        )
        .is_err());
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    rt.assets
        .register_generated(ClientId(1), "__crysirror/white.png", png.into_inner())
        .unwrap();
    let first = layer(frame("__crysirror/white.png".into(), [1., 0., 0., 1.]));
    renderer
        .advance_frame_histories(&mut rt.assets, vec![first.clone()], [4, 2])
        .unwrap();
    let next = layer(frame(path.clone(), [0.5, 1., 1., 1.]));
    renderer
        .advance_frame_histories(&mut rt.assets, vec![next.clone()], [4, 2])
        .unwrap();
    let stats = renderer.profile_report().unwrap();
    assert_eq!(
        stats["frame_history"],
        serde_json::json!({"renders":2,"copies":1,"readbacks":0})
    );
    // Capturing the same retained declaration must not recursively darken it.
    renderer.set_engine_frame(vec![next.clone()]).unwrap();
    let captured = renderer
        .capture(&rt.host.scene, &mut rt.assets, [4, 2], [0; 4])
        .unwrap();
    let rgba = history.readback(7, [4, 2]).unwrap();
    assert_eq!(captured.as_raw(), &rgba);
    assert!(rgba
        .chunks_exact(4)
        .all(|p| p[0].abs_diff(188) <= 1 && p[1] == 0 && p[2] == 0 && p[3] == 255));
    renderer
        .advance_frame_histories(&mut rt.assets, vec![next], [4, 2])
        .unwrap();
    assert_eq!(
        renderer.profile_report().unwrap()["frame_history"]["renders"],
        2
    );
    assert!(foreign.readback(7, [4, 2]).is_err());
    // A new, equal-valued declaration is a new logical frame.
    let third = layer(frame(path.clone(), [0.5, 1., 1., 1.]));
    renderer
        .advance_frame_histories(&mut rt.assets, vec![third.clone()], [4, 2])
        .unwrap();
    assert!(history.readback(7, [2, 1]).unwrap()[0].abs_diff(137) <= 2);
    // An unchanged source must still expose its latest picture to a new
    // consumer, even if it was rendered before anybody requested feedback.
    history.texture(8, [4, 2]).unwrap();
    let mut consumer = layer(frame(path, [1.; 4]));
    consumer.id = 8;
    renderer
        .advance_frame_histories(&mut rt.assets, vec![third, consumer], [4, 2])
        .unwrap();
    assert!(history.readback(8, [4, 2]).unwrap()[0].abs_diff(137) <= 2);
    // Retaining geometry must not hide newly published surface pixels.
    history.texture(9, [4, 2]).unwrap();
    let dynamic_path = rt
        .assets
        .put_surface(ClientId(1), 99, 1, 1, vec![255, 0, 0, 255])
        .unwrap();
    let mut dynamic = layer(frame(dynamic_path.clone(), [1.; 4]));
    dynamic.id = 9;
    renderer
        .advance_frame_histories(&mut rt.assets, vec![dynamic.clone()], [4, 2])
        .unwrap();
    assert_eq!(
        &history.readback(9, [4, 2]).unwrap()[..4],
        &[255, 0, 0, 255]
    );
    rt.assets
        .put_surface(ClientId(1), 99, 1, 1, vec![0, 255, 0, 255])
        .unwrap();
    renderer
        .advance_frame_histories(&mut rt.assets, vec![dynamic], [4, 2])
        .unwrap();
    assert_eq!(
        &history.readback(9, [4, 2]).unwrap()[..4],
        &[0, 255, 0, 255]
    );
    history.texture(7, [2, 1]).unwrap();
    assert!(history.readback(7, [2, 1]).is_err());
    drop(history);
    assert!(!provider.registered(ClientId(1), 7));
    assert!(provider.registered(ClientId(2), 7));
    drop(foreign);
    assert!(!provider.registered(ClientId(2), 7));
}
