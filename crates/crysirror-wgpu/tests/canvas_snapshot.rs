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
fn canvas_image_job_uses_engine_compositor_and_freezes_its_admitted_generation() {
    use crysirror_core::{image_processing::ImageProcessState,service::HostServices};
    let (_root,mut rt,mut renderer)=fixture();
    rt.set_canvas_image_presenter(true);
    submit(&mut rt,json!({"layers":[
        {"op":"put","layer":{"id":1,"rect":[0,0,4,4],"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":2,"parent":1,"rect":[0,0,4,4],"clip":[0,0,4,4],"opacity":128,"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":3,"parent":2,"rect":[0,0,3,4],"paint":{"kind":"solid","color":[255,0,0,255]}}},
        {"op":"put","layer":{"id":4,"parent":2,"rect":[1,0,3,4],"order":[0,2],"paint":{"kind":"solid","color":[255,0,0,255]}}}
    ]}));
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":10,"width":4,"height":4},
        {"op":"copy_subtree","source":1,"destination":10,"rect":[0,0,4,4],"position":[0,0]}
    ]}));
    let mut job=rt.host.begin_canvas_image(ClientId(1),10).unwrap();
    submit(&mut rt,json!({"canvases":[{"op":"clear","id":10,"color":[0,0,255,255]}],"layers":[{"op":"remove","id":1}]}));
    let until=Instant::now()+Duration::from_secs(30);
    let image=loop {
        rt.tick(0,Default::default()).unwrap();
        rt.prepare_canvas_images(|request,assets|renderer.snapshot_readback(assets,request.client,request.size,&request.layers)).unwrap();
        match job.poll().unwrap() {
            ImageProcessState::Ready(image)=>break image,
            ImageProcessState::Failed(reason)=>panic!("{reason}"),
            ImageProcessState::Pending=>{assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));},
        }
    };
    assert_eq!(image.size(),[4,4]);assert_eq!(image.client(),ClientId(1));assert!(!image.opaque());
    let rgba=image.rgba();
    for pixel in rgba.chunks_exact(4) {
        assert_eq!(pixel[3],128,"group alpha flattened per child: {pixel:?}");
        // ImageResources contain straight RGBA; unlike the final framebuffer,
        // the stored RGB has not yet been blended into a destination.
        assert_eq!(pixel,&[255,0,0,128],"frozen straight-alpha red");
    }
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn frozen_canvas_composes_overlapping_children_before_group_opacity() {
    let (_root, mut rt, mut r) = fixture();
    submit(&mut rt,json!({"layers":[
        {"op":"put","layer":{"id":1,"rect":[0,0,4,4],"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":2,"parent":1,"rect":[0,0,4,4],"clip":[0,0,4,4],"opacity":128,"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":3,"parent":2,"rect":[0,0,3,4],"order":[0,1],"paint":{"kind":"solid","color":[255,0,0,255]}}},
        {"op":"put","layer":{"id":4,"parent":2,"rect":[1,0,3,4],"order":[0,2],"paint":{"kind":"solid","color":[255,0,0,255]}}}
    ]}));
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":10,"width":4,"height":4},
        {"op":"copy_subtree","source":1,"destination":10,"rect":[0,0,4,4],"position":[0,0]},
        {"op":"present","id":10,"layer":10,"order":10}
    ],"layers":[{"op":"set_visible","id":1,"visible":false}]}));
    let image=capture(&mut rt,&mut r);
    for x in 0..4 {
        let p=image.get_pixel(x,2).0;
        assert_eq!(p[3],128,"group alpha was applied independently to overlapping children at x={x}: {p:?}");
        assert!(p[0].abs_diff(188)<=2&&p[1]<=1&&p[2]<=1,"isolated red: {p:?}");
    }
    assert_eq!(image.get_pixel(5,5).0,[0,0,0,0]);
    submit(&mut rt,json!({"layers":[{"op":"put","layer":{"id":4,"parent":2,"rect":[1,0,3,4],"order":[0,2],"paint":{"kind":"solid","color":[0,255,0,255]}}}]}));
    assert_eq!(capture(&mut rt,&mut r),image,"frozen group changed with the live source");
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn native_canvas_colour_copy_ignores_source_alpha_and_keeps_destination_alpha_and_clip() {
    let (_root, mut rt, mut r) = fixture();
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":1,"width":8,"height":8},
        {"op":"clear","id":1,"color":[255,0,0,0]},
        {"op":"create","id":2,"width":8,"height":8},
        {"op":"clear","id":2,"color":[0,0,255,128]},
        {"op":"copy","source":1,"destination":2,"rect":[0,0,8,8],
            "position":[0,0],"opacity":255,"clip":[2,2,4,4],"atop":"copy"},
        {"op":"present","id":2,"layer":10,"order":0,"position":[0,0]}
    ]}));
    let image=capture(&mut rt,&mut r);
    let copied=image.get_pixel(3,3).0;
    assert_eq!(copied[3],128,"copy changed destination alpha: {copied:?}");
    assert!(copied[0].abs_diff(188)<=2 && copied[1]<=1 && copied[2]<=1,
        "source alpha incorrectly masked the RGB copy: {copied:?}");
    let outside=image.get_pixel(1,1).0;
    assert_eq!(outside[3],128);
    assert!(outside[0]<=1 && outside[1]<=1 && outside[2].abs_diff(188)<=2,
        "copy escaped its destination clip: {outside:?}");
    submit(&mut rt,json!({"canvases":[
        {"op":"clear","id":2,"color":[0,0,255,128]},
        {"op":"draw","id":2,"atop":"copy","layer":{
            "id":0,"rect":[2,2,4,4],"paint":{"kind":"solid","color":[255,0,0,0]}
        }}
    ]}));
    assert_eq!(capture(&mut rt,&mut r).get_pixel(3,3).0,copied,
        "direct colour-only draws must also preserve zero-alpha source RGB");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn native_canvas_compaction_materialization_preserves_pixels() {
    let (_root, mut rt, mut r) = fixture();
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":1,"width":8,"height":8},
        {"op":"present","id":1,"layer":10,"order":0,"position":[0,0]}
    ]}));
    let draws=vec![json!({"op":"draw","id":1,"layer":{
        "id":0,"rect":[0,0,8,8],"paint":{"kind":"solid","color":[200,40,10,1]}
    }});crysirror_scene::canvas::COMPACT_NODES+1];
    submit(&mut rt,json!({"canvases":draws}));
    let request=rt.host.engine.canvases.materialize_requests().pop().unwrap();
    let before=capture(&mut rt,&mut r);
    let image=r.materialize_snapshot(&mut rt.assets,request.client,request.size,&request.layers)
        .unwrap().expect("immutable solid dependencies are ready");
    let mut bytes=std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes,image::ImageOutputFormat::Png).unwrap();
    let name="__crysirror/materialized.png";
    rt.assets.register_generated(ClientId(1),name,bytes.into_inner()).unwrap();
    let engine=&mut rt.host.engine;
    engine.canvases.resolve_materialized(&request,name,&mut engine.layers).unwrap();
    let after=capture(&mut rt,&mut r);
    for (old,new) in before.as_raw().iter().zip(after.as_raw()) {
        assert!(old.abs_diff(*new)<=1,"materialization changed a pixel: {old} -> {new}");
    }
    assert!(rt.host.engine.canvases.materialize_requests().is_empty());
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn native_canvas_resize_does_not_resurrect_cropped_pixels() {
    let (_root,mut rt,mut r)=fixture();
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":1,"width":8,"height":8},
        {"op":"clear","id":1,"color":[255,0,0,255]},
        {"op":"resize","id":1,"width":4,"height":4},
        {"op":"resize","id":1,"width":8,"height":8},
        {"op":"present","id":1,"layer":10,"order":0,"position":[0,0]}
    ]}));
    let image=capture(&mut rt,&mut r);
    assert_eq!(image.get_pixel(1,1).0,[255,0,0,255]);
    assert_eq!(image.get_pixel(6,6).0,[0;4]);
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

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn bgra_presenter_materializes_rgba_snapshot_without_swapping_red_and_blue() {
    let (_root,mut rt,r)=fixture();
    let mut renderer=Renderer::new(r.gpu.clone(),wgpu::TextureFormat::Bgra8UnormSrgb);
    let layers=serde_json::from_value::<Vec<crysirror_scene::layer::LayerSpec>>(json!([
        {"id":1,"rect":[0,0,2,2],"paint":{"kind":"solid","color":[255,0,0,255]}}
    ])).unwrap();
    let layers=Arc::new(layers);
    let image=renderer.materialize_snapshot(&mut rt.assets,1,[2,2],&layers).unwrap().unwrap();
    assert!(image.pixels().all(|pixel|pixel.0==[255,0,0,255]));
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn completed_dynamic_mesh_freezes_geometry_and_uses_shared_renderer_after_source_removal() {
    let (_root, mut rt, mut r)=fixture();
    let asset="__crysirror/frozen-mesh-white.png";
    rt.assets.register_generated(ClientId(1),asset,png([255;4])).unwrap();
    submit(&mut rt,json!({"layers":[
        {"op":"put","layer":{"id":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":2,"parent":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]},"order":[0,1]}}
    ]}));
    let frame=crysirror_puppet::PuppetFrame {
        size:[16.,16.],textures:vec![asset.into()],parts:vec![crysirror_puppet::FramePart{
            id:"quad".into(),texture:0,positions:vec![[2.,2.],[10.,2.],[10.,10.],[2.,10.]],
            uvs:vec![[0.,0.],[1.,0.],[1.,1.],[0.,1.]],indices:vec![0,1,2,0,2,3],
            colors:vec![[1.,0.,0.,1.];4],blend:crysirror_puppet::PartBlend::Normal,
            masks:vec![],invert_mask:false,visible:true,
        }],
    };
    rt.host.engine.layers.set_dynamic_puppet(1,2,Arc::new(frame.clone())).unwrap();
    let before=capture(&mut rt,&mut r);
    assert!(before.get_pixel(5,5).0[0]>=250);
    submit(&mut rt,json!({"canvases":[
        {"op":"create","id":10,"width":16,"height":16},
        {"op":"clear","id":10,"color":[0,0,0,0]},
        {"op":"copy_subtree","source":1,"destination":10,"rect":[0,0,16,16],"position":[0,0]},
        {"op":"present","id":10,"layer":10,"order":10}
    ],"layers":[{"op":"remove","id":1}]}));
    let frozen=capture(&mut rt,&mut r);
    assert_eq!(before,frozen,"mesh geometry/blend changed during engine snapshot capture");
    assert_eq!(bakes(&mut r),1);
    submit(&mut rt,json!({"layers":[{"op":"set_opacity","id":10,"opacity":128}]}));
    let half=capture(&mut rt,&mut r).get_pixel(5,5).0;
    assert_eq!(half[3],128);assert!(half[0].abs_diff(188)<=2&&half[1]<=1&&half[2]<=1,"{half:?}");
    assert_eq!(bakes(&mut r),1,"outer opacity should not re-evaluate captured mesh");
    submit(&mut rt,json!({"layers":[
        {"op":"put","layer":{"id":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]}}},
        {"op":"put","layer":{"id":2,"parent":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,0]},"order":[0,1]}}
    ]}));
    let mut changed=frame;changed.parts[0].positions=vec![[4.,4.],[12.,4.],[12.,12.],[4.,12.]];
    changed.parts[0].colors=vec![[0.,0.,1.,1.];4];
    rt.host.engine.layers.set_dynamic_puppet(1,2,Arc::new(changed)).unwrap();
    submit(&mut rt,json!({"canvases":[
        {"op":"copy_subtree","source":1,"destination":10,"rect":[0,0,16,16],"position":[0,0]}
    ],"layers":[{"op":"remove","id":1},{"op":"set_opacity","id":10,"opacity":255}]}));
    let revised=capture(&mut rt,&mut r);
    assert_eq!(revised.get_pixel(3,3).0,[0;4]);
    assert!(revised.get_pixel(11,11).0[2]>=250,"mesh content was omitted from the snapshot cache identity");
    assert_eq!(bakes(&mut r),2);

}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn published_image_capture_keeps_admitted_resource_after_live_release_without_png_encoding(){
    use crysirror_core::{image_processing::{ImageInput,ImageProcessState},service::HostServices};
    let (_root,mut rt,mut renderer)=fixture();rt.set_canvas_image_presenter(true);
    let mut source=rt.host.image_metadata(ClientId(1)).unwrap().begin_process(ImageInput::Solid{size:[4,4],color:[255,0,0,255]},b"[]".as_slice().into()).unwrap();
    let deadline=Instant::now()+Duration::from_secs(10);
    let image=loop{match source.poll().unwrap(){ImageProcessState::Ready(r)=>break r,ImageProcessState::Failed(e)=>panic!("{e}"),ImageProcessState::Pending=>{}}assert!(Instant::now()<deadline);std::thread::yield_now();};
    let published=rt.host.publish_image_resource(ClientId(1),100,image).unwrap();let path=published.path.relative();
    submit(&mut rt,json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,4,4],"paint":{"kind":"image","asset":path}}}]}));
    submit(&mut rt,json!({"canvases":[{"op":"create","id":7,"width":4,"height":4},{"op":"copy_subtree","source":1,"destination":7,"rect":[0,0,4,4],"position":[0,0]}]}));
    let mut capture=rt.host.begin_canvas_image(ClientId(1),7).unwrap();
    submit(&mut rt,json!({"layers":[{"op":"remove","id":1}],"canvases":[{"op":"clear","id":7,"color":[0,0,255,255]}]}));
    rt.host.release_image_resource(ClientId(1),path).unwrap();rt.tick(0,Default::default()).unwrap();
    assert!(rt.assets.surface(ClientId(1),path).is_none(),"only the admitted capture retains this resource generation");
    let image=loop{
        rt.prepare_canvas_images(|request,assets|renderer.snapshot_readback(assets,request.client,request.size,&request.layers)).unwrap();
        match capture.poll().unwrap(){ImageProcessState::Ready(r)=>break r,ImageProcessState::Failed(e)=>panic!("{e}"),ImageProcessState::Pending=>{}}
        assert!(Instant::now()<deadline);std::thread::yield_now();
    };
    assert_eq!(image.size(),[4,4]);assert!(image.rgba().chunks_exact(4).all(|p|p==[255,0,0,255]));
    assert_eq!(rt.host.now_ms(),0);
}
