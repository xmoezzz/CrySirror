// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. https://mozilla.org/MPL/2.0/.
//! Authored, VM-independent commands -> shared engine scene -> actual GPU.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::ServiceBatch;
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::sync::Arc;
fn submit(rt: &mut Runtime, commands: serde_json::Value) {
    let batch: ServiceBatch = serde_json::from_value(json!({"layers":commands})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
}
fn node(
    id: u32,
    parent: Option<u32>,
    order: i32,
    rect: [u32; 4],
    color: [u8; 4],
    visible: bool,
) -> serde_json::Value {
    json!({"op":"put","layer":{"id":id,"parent":parent,"order":[0,order],"rect":rect,"paint":{"kind":"solid","color":color},"visible":visible}})
}
fn capture(renderer: &mut Renderer, rt: &mut Runtime) -> image::RgbaImage {
    renderer
        .set_engine_frame(rt.host.engine.frame([8, 4]).unwrap())
        .unwrap();
    renderer
        .capture(&rt.host.scene, &mut rt.assets, [8, 4], [0; 4])
        .unwrap()
}
fn begin(rt: &mut Runtime, effect: serde_json::Value, elapsed: u64) {
    submit(
        rt,
        json!([{"op":"begin_crossfade_effect","id":1,"source":10,"duration_ms":1000,"effect":effect}]),
    );
    rt.tick(elapsed, InputFrame::default()).unwrap();
}
#[test]
fn transition_shaders_validate_without_a_device() {
    for (source, format) in [
        (
            include_str!("../src/shaders/composite_filter.wgsl"),
            "rgba32float",
        ),
        (
            include_str!("../src/shaders/composite_filter.wgsl"),
            "rgba16float",
        ),
        (include_str!("../src/shaders/composite.wgsl"), ""),
    ] {
        let source = source.replace("__OUTPUT_FORMAT__", format);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}
#[test]
#[ignore = "requires a real GPU; execute explicitly; no display or game data"]
fn gpu_box_byte_quantization_dim_max_radius_and_static_live_inputs() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(root.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"shared subtree effects","content_revision":"1","assets":"assets","canvas":[8,4],"clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("shared subtree effect GPU: {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    submit(
        &mut rt,
        json!([
            node(1, None, 0, [0, 0, 8, 4], [255; 4], true),
            node(2, Some(1), 1, [0, 0, 1, 4], [0, 0, 0, 255], true),
            node(3, Some(1), 2, [1, 0, 1, 4], [128, 128, 128, 255], true),
            node(10, None, 3, [0, 0, 8, 4], [255; 4], false),
            node(11, Some(10), 4, [0, 0, 1, 4], [0, 0, 0, 255], true),
            node(12, Some(10), 5, [1, 0, 1, 4], [128, 128, 128, 255], true)
        ]),
    );
    begin(
        &mut rt,
        json!({"kind":"blur","before":[2,0],"after":[2,0],"dynamic":true}),
        500,
    );
    let filtered = capture(&mut renderer, &mut rt);
    // One 2D normalization, integer truncation, encoded RGB domain. A linear
    // mean of these 0/128/255 samples would be around 171, not 127.
    for channel in 0..3 {
        assert!(
            filtered.get_pixel(1, 1)[channel] == 127,
            "byte mean {:?}",
            filtered.get_pixel(1, 1)
        );
    }
    assert!(
        filtered.get_pixel(0, 1)[0] == 42,
        "replicated edge {:?}",
        filtered.get_pixel(0, 1)
    );
    submit(
        &mut rt,
        json!([{"op":"end_crossfade","id":1},{"op":"remove","id":1},{"op":"remove","id":10},node(1,None,0,[0,0,8,4],[255,0,0,255],true),node(10,None,1,[0,0,8,4],[0,0,0,255],false)]),
    );
    begin(
        &mut rt,
        json!({"kind":"blur","before":[0,0],"after":[0,0],"dynamic":false}),
        250,
    );
    // Headless logical ticks freeze effect inputs without a screenshot.
    renderer.advance_frame_histories(&mut rt.assets,rt.host.engine.layers.frame([8,4]).unwrap(),[8,4]).unwrap();
    submit(&mut rt,json!([node(1,None,0,[0,0,8,4],[0,255,0,255],true)]));
    let frozen = capture(&mut renderer, &mut rt);
    assert!(frozen.get_pixel(2,1)[0]>0 && frozen.get_pixel(2,1)[1]==0,"headless input froze only at final capture");
    submit(
        &mut rt,
        json!([node(1, None, 0, [0, 0, 8, 4], [0, 0, 255, 255], true)]),
    );
    assert_eq!(
        capture(&mut renderer, &mut rt),
        frozen,
        "static input was recaptured"
    );
    submit(&mut rt, json!([{"op":"end_crossfade","id":1}]));
    begin(
        &mut rt,
        json!({"kind":"blur","before":[0,0],"after":[0,0],"dynamic":true}),
        250,
    );
    let live = capture(&mut renderer, &mut rt);
    assert_ne!(live, frozen, "new generation reused old snapshots");
    submit(
        &mut rt,
        json!([node(1, None, 0, [0, 0, 8, 4], [0, 255, 0, 255], true)]),
    );
    assert_ne!(
        capture(&mut renderer, &mut rt),
        live,
        "dynamic input was frozen"
    );
    // Coverage remains separate from encoded premultiplied colors.
    submit(
        &mut rt,
        json!([{"op":"end_crossfade","id":1},node(1,None,0,[0,0,8,4],[255,0,0,128],true),node(10,None,1,[0,0,8,4],[0,0,255,64],false)]),
    );
    begin(
        &mut rt,
        json!({"kind":"blur","before":[0,0],"after":[0,0],"dynamic":true}),
        500,
    );
    let alpha = capture(&mut renderer, &mut rt);
    assert!(
        alpha.get_pixel(2, 1)[3] == 96,
        "coverage {:?}",
        alpha.get_pixel(2, 1)
    );
    // Independent scalar u32 rule average at the maximum allowed 257x257
    // kernel: x=0 ->127, x=2 ->128. The one-level wipe distinguishes them.
    let mut rule = image::RgbaImage::from_pixel(8, 4, image::Rgba([255; 4]));
    for y in 0..4 {
        rule.put_pixel(0, y, image::Rgba([0, 0, 0, 255]));
    }
    rule.save(root.path().join("assets/rule.png")).unwrap();
    submit(
        &mut rt,
        json!([{"op":"end_crossfade","id":1},node(1,None,0,[0,0,8,4],[0,0,0,255],true),node(10,None,1,[0,0,8,4],[255;4],false)]),
    );
    let dim = json!({"kind":"dim","rule":{"asset":"rule.png","vague":1},"blur":[128,128],"negative":false,"acceleration":1.});
    begin(&mut rt, dim, 500);
    let mut mask = capture(&mut renderer, &mut rt);
    // Capture's async asset warmup may need another pass, without moving time.
    for _ in 0..100 {
        if mask.get_pixel(0, 1)[0] > 200 {
            break;
        }
        std::thread::yield_now();
        mask = capture(&mut renderer, &mut rt);
    }
    let mean = |x: i32| -> u32 {
        let sum: u32 = (-128..=128)
            .map(|dx| {
                if (x + dx).clamp(0, 7) == 0 {
                    0u32
                } else {
                    255u32
                }
            })
            .sum();
        (sum * 257) / (257 * 257)
    };
    for x in 0..8 {
        let expected = if mean(x as i32) < 128 { 254u8 } else { 0u8 };
        assert!(
            mask.get_pixel(x, 1)[0] == expected,
            "u32 max-radius reference x={x}, level={}, pixel={:?}",
            mean(x as i32),
            mask.get_pixel(x, 1)
        );
    }
    submit(&mut rt, json!([{"op":"end_crossfade","id":1}]));
    begin(
        &mut rt,
        json!({"kind":"dim","rule":{"asset":"rule.png","vague":1},"blur":[128,128],"negative":true,"acceleration":1.}),
        500,
    );
    let neg = capture(&mut renderer, &mut rt);
    for x in 0..8 {
        let expected = if 255 - mean(x as i32) < 128 {
            254u8
        } else {
            0u8
        };
        assert!(
            neg.get_pixel(x, 1)[0] == expected,
            "negative reference x={x}, level={}, pixel={:?}",
            255 - mean(x as i32),
            neg.get_pixel(x, 1)
        );
    }
}
