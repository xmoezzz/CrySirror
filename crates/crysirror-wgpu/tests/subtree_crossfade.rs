// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! VM-independent subtree crossfade acceptance on an actual GPU.
use crysirror_core::state::ClientId;
use crysirror_runtime::Runtime;
use crysirror_scene::ServiceBatch;
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::sync::Arc;
fn submit(rt: &mut Runtime, layers: serde_json::Value) {
    let batch: ServiceBatch = serde_json::from_value(json!({"layers":layers})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
}
fn put(
    id: u32,
    parent: Option<u32>,
    order: i32,
    rect: [u32; 4],
    color: [u8; 4],
    visible: bool,
) -> serde_json::Value {
    json!({"op":"put","layer":{"id":id,"parent":parent,"order":[0,order],"rect":rect,"visible":visible,"paint":{"kind":"solid","color":color}}})
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn isolated_subtree_crossfade_preserves_overlap_alpha_endpoints_and_recovers() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(root.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"isolated-subtrees","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("subtree crossfade GPU: {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    let capture = |r: &mut Renderer, rt: &mut Runtime| {
        r.set_engine_frame(rt.host.engine.frame([16, 16]).unwrap())
            .unwrap();
        r.capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 0])
            .unwrap()
    };
    // The red root and opaque green overlapping child must be flattened before
    // fading. Fading children separately leaks red into the overlap at t=.5.
    submit(
        &mut rt,
        json!([
            put(1, None, 0, [0, 0, 12, 12], [255, 0, 0, 255], true),
            put(2, Some(1), 1, [4, 4, 4, 4], [0, 255, 0, 255], true),
            put(3, None, 2, [0, 0, 12, 12], [0, 0, 255, 255], false),
            put(4, None, 3, [14, 0, 2, 2], [255, 255, 0, 255], true)
        ]),
    );
    submit(
        &mut rt,
        json!([
            {"op":"set_clip","id":1,"clip":[0,0,10,10]},
            {"op":"set_clip","id":3,"clip":[0,0,10,10]}
        ]),
    );
    let plain = capture(&mut renderer, &mut rt);
    submit(
        &mut rt,
        json!([{"op":"begin_crossfade","id":1,"source":3,"duration_ms":1000}]),
    );
    let start = capture(&mut renderer, &mut rt);
    for (a, b) in plain.pixels().zip(start.pixels()) {
        for (c, d) in a.0.into_iter().zip(b.0) {
            assert!(c.abs_diff(d) <= 2, "start endpoint {:?} {:?}", a, b);
        }
    }
    rt.tick(500, crysirror_core::service::InputFrame::default())
        .unwrap();
    let half = capture(&mut renderer, &mut rt);
    assert_eq!(
        half.get_pixel(11, 11).0,
        [0, 0, 0, 0],
        "subtree clip leaked"
    );
    let overlap = half.get_pixel(5, 5).0;
    assert!(
        overlap[0] <= 2
            && overlap[1].abs_diff(128) <= 2
            && overlap[2].abs_diff(128) <= 2
            && overlap[3] == 255,
        "isolated overlap {overlap:?}"
    );
    assert_eq!(
        half.get_pixel(15, 0).0,
        [255, 255, 0, 255],
        "external layer changed"
    );
    assert_eq!(
        half.get_pixel(15, 15).0,
        [0, 0, 0, 0],
        "canvas leaked alpha"
    );
    rt.tick(500, crysirror_core::service::InputFrame::default())
        .unwrap();
    let end = capture(&mut renderer, &mut rt);
    assert_eq!(end.get_pixel(5, 5).0, [0, 0, 255, 255]);
    submit(&mut rt, json!([{"op":"end_crossfade","id":1}]));
    assert_eq!(
        capture(&mut renderer, &mut rt),
        plain,
        "ending changed original nodes"
    );
    // Transparent roots: mix coverage and premultiplied color, then unpremultiply
    // only once for the final normal layer draw.
    submit(
        &mut rt,
        json!([
            {"op":"remove","id":2},
            put(1,None,0,[0,0,12,12],[255,0,0,128],true),
            put(3,None,2,[0,0,12,12],[0,0,255,64],false),
            {"op":"begin_crossfade","id":1,"source":3,"duration_ms":1000}
        ]),
    );
    rt.tick(500, crysirror_core::service::InputFrame::default())
        .unwrap();
    let alpha = capture(&mut renderer, &mut rt);
    let p = alpha.get_pixel(5, 5).0;
    assert!(p[3].abs_diff(96) <= 2, "coverage {p:?}");
    // capture returns a premultiplied linear framebuffer encoded as sRGB.
    let linear = |v: f64| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let gamma = |v: f64| {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        }
    };
    for (actual, straight) in [(p[0], 2. / 3.), (p[2], 1. / 3.)] {
        let expected = (gamma(linear(straight) * 96. / 255.) * 255.).round() as u8;
        assert!(
            actual.abs_diff(expected) <= 3,
            "alpha {p:?}, expected {expected}"
        );
    }
    // Bad untrusted composite input must fail before GPU allocation and leave
    // the renderer usable on the next valid frame.
    let mut invalid = rt.host.engine.frame([16, 16]).unwrap();
    Arc::make_mut(
        invalid
            .iter_mut()
            .find(|p| p.composite.is_some())
            .unwrap()
            .composite
            .as_mut()
            .unwrap(),
    )
    .progress = f32::NAN;
    renderer.set_engine_frame(invalid).unwrap();
    assert!(
        renderer
            .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0; 4])
            .is_err()
    );
    let recovered = capture(&mut renderer, &mut rt);
    assert_eq!(recovered, alpha);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("budget rejection target"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let error = renderer
        .render(
            &rt.host.scene,
            &mut rt.assets,
            crysirror_wgpu::RenderTarget {
                view: &view,
                size: [16, 16],
            },
            [8192, 8192],
            [0; 4],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("working set")
            || error.to_string().contains("device dimensions"),
        "{error}"
    );
}
