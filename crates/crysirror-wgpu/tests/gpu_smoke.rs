// SPDX-License-Identifier: MPL-2.0
use crysirror_core::service::InputFrame;
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, Renderer, RenderingPath};
use std::{path::Path, sync::Arc};

#[test]
#[ignore = "requires a real wgpu adapter or Mesa software Vulkan; CI GPU job runs this explicitly"]
fn shared_scene_reaches_actual_pixels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/project.json");
    let state = tempfile::tempdir().unwrap();
    let mut rt = Runtime::load(&root, state.path()).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let instance = wgpu::Instance::default();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&instance, None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_scene3d(rt.project.scene3d.as_ref()).unwrap();
    let pixels = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    assert_eq!(pixels.dimensions(), (960, 540));
    let left = pixels.get_pixel(35, 35).0;
    let right = pixels.get_pixel(700, 35).0;
    assert!(
        left[1] > left[0],
        "left client must show its green asset: {left:?}"
    );
    assert!(
        right[0] > right[1],
        "right client must show its orange asset: {right:?}"
    );
    assert_ne!(
        pixels.get_pixel(480, 270).0,
        rt.project.background,
        "3D scene must affect center pixels"
    );
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn siglus_resource_uses_shared_upload_and_compute_rays_keep_overlays() {
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("assets")).unwrap();
    let mut g00 = vec![0, 1, 0, 1, 0];
    g00.extend(0u32.to_le_bytes());
    g00.extend(4u32.to_le_bytes());
    g00.extend([1, 10, 200, 20]);
    std::fs::write(state.path().join("assets/panel.g00"), g00).unwrap();
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/desktop/arena.json")).unwrap();
    project["canvas"] = serde_json::json!([320, 180]);
    project["clients"][0]["program"] = serde_json::json!([
        {"op":"sprite","id":0,"asset":"panel.g00","rect":[0,0,24,24],"z":1}, {"op":"halt"}
    ]);
    let anchor = state.path().join("project.json");
    std::fs::write(&anchor, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut rt = Runtime::load(&anchor, &state.path().join("state")).unwrap();
    rt.project.background = [16, 20, 30, 128];
    rt.assets
        .decoders
        .register("g00", Arc::new(crysirror_siglus::SiglusImageDecoder))
        .unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let instance = wgpu::Instance::default();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&instance, None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_scene3d(rt.project.scene3d.as_ref()).unwrap();
    renderer
        .set_rendering_path(RenderingPath::ComputeRaytrace)
        .unwrap();
    assert!(renderer
        .set_rendering_path(RenderingPath::HardwareRaytrace)
        .is_err());
    let pixels = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    let green = pixels.get_pixel(12, 12).0;
    let background = pixels.get_pixel(319, 179).0;
    for (actual, expected) in background.into_iter().zip(rt.project.background) {
        assert!(
            actual.abs_diff(expected) <= 1,
            "ray background must preserve sRGB and alpha: {background:?}"
        );
    }
    assert!(
        green[1] > 180 && green[0] < 30 && green[2] < 30,
        "native resource channels: {green:?}"
    );
    assert_ne!(
        pixels.get_pixel(160, 110).0,
        rt.project.background,
        "ray traversal must hit 3D geometry"
    );
    assert!(
        pixels.pixels().any(|p| p[0] > 30 && p[1] > 40 && p[2] > 50),
        "3D shaded pixels expected"
    );
    renderer.set_rendering_path(RenderingPath::Raster).unwrap();
    let raster = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    assert_eq!(
        pixels.get_pixel(12, 12),
        raster.get_pixel(12, 12),
        "native overlay must survive renderer changes"
    );
    assert_ne!(
        pixels.as_raw(),
        raster.as_raw(),
        "compute shading/shadow path must execute independently"
    );
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn retained_layers_blend_and_clip_match_linear_color_equations() {
    use crysirror_core::state::ClientId;
    use crysirror_scene::{
        layer::{LayerSpec, LayerSystem},
        world::WorldCommand,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/systems.json");
    let state = tempfile::tempdir().unwrap();
    let mut rt = Runtime::load(&root, state.path()).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let modes = ["normal", "add", "subtract", "multiply", "screen"];
    let layers:Vec<LayerSpec>=modes.iter().enumerate().map(|(i,mode)|serde_json::from_value(serde_json::json!({"id":i,"rect":[i*24,0,20,20],"paint":{"kind":"solid","color":[100,160,200,128]},"blend":mode,"clip":[0,0,10,20]})).unwrap()).collect();
    let mut system = LayerSystem::default();
    system.install(ClientId(1), &layers).unwrap();
    renderer
        .set_engine_frame(system.frame([128, 32]).unwrap())
        .unwrap();
    let bg = [80, 100, 120, 255];
    let image = renderer
        .capture(&rt.host.scene, &mut rt.assets, [128, 32], bg)
        .unwrap();
    let linear = crysirror_wgpu::geometry::linear;
    let encode = |v: f32| {
        let v = v.clamp(0., 1.);
        ((if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        }) * 255.)
            .round() as u8
    };
    for (i, mode) in modes.iter().enumerate() {
        let actual = image.get_pixel(i as u32 * 24 + 5, 5).0;
        for ch in 0..3 {
            let src = linear([100, 160, 200][ch]);
            let dst = linear(bg[ch]);
            let a = 128. / 255.;
            let expected = encode(match *mode {
                "normal" => src * a + dst * (1. - a),
                "add" => src * a + dst,
                "subtract" => dst - src * a,
                "multiply" => dst * src * a + dst * (1. - a),
                "screen" => src * a + dst * (1. - src * a),
                _ => unreachable!(),
            });
            assert!(
                actual[ch].abs_diff(expected) <= 2,
                "{mode} channel {ch}: actual {actual:?}, expected {expected}"
            );
        }
        assert_eq!(
            image.get_pixel(i as u32 * 24 + 15, 5).0,
            bg,
            "scissor must exclude the right half of {mode}"
        );
    }
    // Dynamic world extraction reaches both actual renderer paths; motion changes pixels.
    for path in [RenderingPath::Raster, RenderingPath::ComputeRaytrace] {
        renderer.set_rendering_path(path).unwrap();
        renderer
            .set_engine_frame(rt.host.engine.frame(rt.project.canvas).unwrap())
            .unwrap();
        renderer
            .set_scene3d(rt.render_scene3d().unwrap().as_ref())
            .unwrap();
        let a = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .unwrap();
        rt.host
            .engine
            .world
            .apply_batch(
                ClientId(1),
                &[WorldCommand::SetVisible {
                    id: 1,
                    visible: false,
                }],
            )
            .unwrap();
        renderer
            .set_scene3d(rt.render_scene3d().unwrap().as_ref())
            .unwrap();
        let b = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .unwrap();
        assert_ne!(
            a.as_raw(),
            b.as_raw(),
            "world visibility/hierarchy must reach {path:?}"
        );
        assert_eq!(
            a.get_pixel(730, 50),
            b.get_pixel(730, 50),
            "UI overlay must survive world changes"
        );
        rt.host
            .engine
            .world
            .apply_batch(
                ClientId(1),
                &[WorldCommand::SetVisible {
                    id: 1,
                    visible: true,
                }],
            )
            .unwrap();
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn original_siglus_pck_and_g00_reach_shared_raster_and_compute_pixels() {
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir(state.path().join("assets")).unwrap();
    std::fs::write(
        state.path().join("assets/Scene.pck"),
        crysirror_siglus_vm::fixture::pack_scene(&crysirror_siglus_vm::fixture::image_scene()),
    )
    .unwrap();
    std::fs::write(
        state.path().join("assets/native.g00"),
        crysirror_siglus_vm::fixture::pixel_g00(),
    )
    .unwrap();
    let mut p: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/desktop/arena.json")).unwrap();
    p["canvas"] = serde_json::json!([320, 180]);
    p["clients"] = serde_json::json!([{"name":"native","adapter":"siglus","game":"native","config":{"scene":"Scene.pck","string_codec":"plain"}}]);
    let path = state.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&path, &state.path().join("state")).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    assert_eq!(rt.inspect()["sessions"][0]["engine"], "siglus");
    // Restore into a fresh asset store and render immediately: no VM tick may
    // be required to recover generated native G00 textures.
    let save = rt.snapshot().unwrap();
    let mut restored = Runtime::load(&path, &state.path().join("restored")).unwrap();
    restored.restore(&save).unwrap();
    rt = restored;
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    for mode in [RenderingPath::Raster, RenderingPath::ComputeRaytrace] {
        renderer.set_rendering_path(mode).unwrap();
        renderer
            .set_scene3d(rt.render_scene3d().unwrap().as_ref())
            .unwrap();
        renderer
            .set_engine_frame(rt.host.engine.frame(rt.project.canvas).unwrap())
            .unwrap();
        let pixels = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .unwrap();
        let color = pixels.get_pixel(40, 40).0;
        assert!(
            color[1] > 180 && color[0] < 30 && color[2] < 30,
            "native interpreted object and decoded G00 on {mode:?}: {color:?}"
        );
    }
}

fn pbr_runtime(dir: &Path, shadows: bool) -> Runtime {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"pbr","content_revision":"1","canvas":[320,180],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"pbr","systems":{"entities":[
            {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[8,0.1,8]},"color":[200,200,200,255]},
            {"id":2,"name":"box","geometry":{"kind":"cube"},"transform":{"translation":[0,0.5,0]},"color":[220,60,40,255]}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
    let env = serde_json::json!({"world":[
        {"op":"set_camera","camera":{"eye":[3,2.5,3],"target":[0,1.2,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
        {"op":"set_environment","environment":{"sun":{"direction":[1,-1,0.3],"intensity":4,"shadows":shadows}}}]});
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(env).unwrap();
    rt.host
        .engine
        .submit(rt.client_id("game").unwrap(), &batch)
        .unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    rt
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn pbr_path_renders_sky_lit_geometry_and_sun_shadows() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut shots = Vec::new();
    for shadows in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let mut rt = pbr_runtime(tmp.path(), shadows);
        let scene = rt.render_pbr().unwrap().expect("environment selects PBR");
        assert_eq!(scene.instances.len(), 2);
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(scene)).unwrap();
        let img = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .unwrap();
        // Compute ray tracing does not silently ignore PBR content.
        renderer
            .set_rendering_path(RenderingPath::ComputeRaytrace)
            .ok();
        if renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .is_ok()
        {
            panic!("PBR scene must not be rendered by an unsupported path");
        }
        shots.push(img);
    }
    let lit = &shots[0];
    std::fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist")).ok();
    lit.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/pbr-test.png"))
        .ok();
    // Procedural sky at the top is blue-dominant.
    let sky = lit.get_pixel(160, 2).0;
    assert!(sky[2] > sky[0] && sky[2] > 60, "sky {sky:?}");
    // The red box is lit (red channel dominant near the center).
    let center = lit.get_pixel(160, 95).0;
    assert!(
        center[0] > center[1] && center[0] > center[2],
        "box {center:?}"
    );
    // Shadows darken some floor pixels relative to the unshadowed render.
    let darker = lit
        .pixels()
        .zip(shots[1].pixels())
        .filter(|(a, b)| (b.0[1] as i32 - a.0[1] as i32) > 20)
        .count();
    assert!(darker > 200, "shadowed pixels: {darker}");
    std::fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist")).ok();
    lit.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/pbr-test.png"))
        .ok();
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn terrain_and_hdr_particles_render() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"t","content_revision":"1","canvas":[320,180],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"t","systems":{"entities":[
            {"id":1,"name":"land","geometry":{"kind":"terrain","resolution":97,"size":[60,60],"height":10,"seed":4}},
            {"id":2,"name":"fire","transform":{"translation":[0,8,0]},
             "particles":{"rate":600,"burst":300,"lifetime":[0.8,1.4],"speed":[2,5],"spread_degrees":30,"gravity":[0,1.5,0],"size":[1.2,0.3],"color_start":[30,12,3,1],"color_end":[4,0.5,0.1,0]}}]}}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut rt =
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap();
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
        {"op":"set_camera","camera":{"eye":[0,16,20],"target":[0,9,0],"up":[0,1,0],"fov_y_degrees":55,"near":0.1,"far":300}},
        {"op":"set_environment","environment":{"bloom":0.06,"exposure":-1.0,"tonemap":"aces","sun":{"direction":[0.4,-1,0.3],"intensity":4}}}]})).unwrap();
    rt.host
        .engine
        .submit(rt.client_id("game").unwrap(), &batch)
        .unwrap();
    for _ in 0..40 {
        rt.tick(16, InputFrame::default()).unwrap();
    }
    rt.finish_vfx_offline().unwrap();
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let img = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    img.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/terrain-particles-test.png"))
        .ok();
    // Terrain fills the lower half with lit, non-sky colors.
    let low = img.get_pixel(40, 170).0;
    assert!(
        low[1] > 30 && u16::from(low[2]) < u16::from(low[1]) + 40,
        "terrain {low:?}"
    );
    // The HDR fire saturates near the emitter (projected around the image center).
    let bright = (100..220)
        .flat_map(|x| (20..130).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y).0;
            p[0] > 230 && p[1] > 120
        })
        .count();
    assert!(bright > 150, "fire pixels {bright}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn engine_material_alpha_sort_depth_opacity_and_unlit_reach_pixels() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let quad = serde_json::json!({"kind":"triangles","vertices":[[-2,-2,0],[2,-2,0],[2,2,0],[-2,2,0]],"indices":[0,1,2,0,2,3]});
    for (name, mode, opacity, channel, front, expected) in [
        ("sorted", "blend", false, false, false, [0.5, 0.25, 0.25]),
        (
            "premultiplied",
            "premultiplied",
            false,
            false,
            false,
            [0.5, 0.25, 0.25],
        ),
        ("additive", "additive", false, false, false, [0.5, 0.5, 0.5]),
        (
            "independent opacity red",
            "blend",
            true,
            false,
            false,
            [0.25098, 0.37451, 0.37451],
        ),
        (
            "independent opacity alpha",
            "blend",
            true,
            true,
            false,
            [0.5, 0.25, 0.25],
        ),
        (
            "opaque occlusion",
            "blend",
            false,
            false,
            true,
            [0., 1., 0.],
        ),
        (
            "depth fade",
            "blend",
            false,
            false,
            false,
            [0.25, 0.375, 0.375],
        ),
        (
            "TAA history",
            "blend",
            false,
            false,
            false,
            [0.5, 0.25, 0.25],
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 128, 128, 255]))
            .save(tmp.path().join("assets/opacity.png"))
            .unwrap();
        let near = serde_json::json!({"base_color_factor":[1,0,0,0.5],"alpha_mode":mode,"unlit":true,"opacity_texture":if opacity{Some("opacity.png")}else{None},"opacity_from_alpha":channel,"depth_fade":if name=="depth fade"{4.0}else{0.0}});
        let p = serde_json::json!({"schema":2,"name":"material-pixels","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"materials","systems":{"entities":[
         {"id":1,"name":"background","geometry":quad,"transform":{"translation":[0,0,if front{2.}else{-1.}]},"material":{"base_color_factor":[0,1,0,1],"unlit":true}},
         {"id":2,"name":"near-red-inserted-first","geometry":quad,"transform":{"translation":[0,0,1]},"material":near},
         {"id":3,"name":"far-blue-inserted-last","geometry":quad,"material":{"base_color_factor":[0,0,1,0.5],"alpha_mode":"blend","unlit":true}},
         {"id":4,"name":"invisible-mask-in-front","geometry":quad,"transform":{"translation":[0,0,3]},"material":{"base_color_factor":[1,1,1,0.1],"alpha_mode":"mask","alpha_cutoff":0.5,"unlit":true}}
        ]}}]});
        std::fs::write(
            tmp.path().join("project.json"),
            serde_json::to_vec(&p).unwrap(),
        )
        .unwrap();
        let mut rt =
            Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap();
        let commands:crysirror_scene::ServiceBatch=serde_json::from_value(serde_json::json!({"world":[{"op":"set_camera","camera":{"eye":[0,0,5],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":100}},{"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":name=="TAA history","tonemap":"none"}}]})).unwrap();
        rt.host
            .engine
            .submit(rt.client_id("game").unwrap(), &commands)
            .unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        let image = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                rt.project.canvas,
                rt.project.background,
            )
            .unwrap();
        let actual = image.get_pixel(32, 32).0;
        let srgb = |v: f32| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            }) * 255.)
                .round() as u8
        };
        for i in 0..3 {
            assert!(
                actual[i].abs_diff(srgb(expected[i])) <= 5,
                "{name}: actual {actual:?}, expected linear {expected:?}"
            );
        }
        if name == "TAA history" {
            let remove: crysirror_scene::ServiceBatch = serde_json::from_value(
                serde_json::json!({"world":[{"op":"despawn","id":2},{"op":"despawn","id":3}]}),
            )
            .unwrap();
            rt.host
                .engine
                .submit(rt.client_id("game").unwrap(), &remove)
                .unwrap();
            renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
            let cleared = renderer
                .capture(
                    &rt.host.scene,
                    &mut rt.assets,
                    rt.project.canvas,
                    rt.project.background,
                )
                .unwrap()
                .get_pixel(32, 32)
                .0;
            assert!(
                cleared[0] < 5 && cleared[1] > 250 && cleared[2] < 5,
                "removed transparent surfaces leaked into opaque TAA history: {cleared:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn alpha_mask_invisible_surface_does_not_cast_solid_shadow() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    for (name, material) in [
        (
            "base alpha",
            serde_json::json!({"alpha_mode":"mask","base_color_factor":[1,0,0,0],"alpha_cutoff":0.5}),
        ),
        (
            "dissolve",
            serde_json::json!({"alpha_mode":"mask","effect":{"kind":"dissolve","threshold":1}}),
        ),
        (
            "developer coverage",
            serde_json::json!({"alpha_mode":"mask","effect":{"kind":"custom","asset":"invisible.wgsl"}}),
        ),
    ] {
        let mut shots = Vec::new();
        for shadows in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let mut rt = pbr_runtime(tmp.path(), shadows);
            std::fs::write(tmp.path().join("assets/invisible.wgsl"),"fn crysirror_surface(i:SurfaceInput)->SurfaceOutput{var o=surface_default(i);o.color.a=0.0;return o;}").unwrap();
            let batch = serde_json::from_value(
                serde_json::json!({"world":[{"op":"set_material","id":2,"material":material}]}),
            )
            .unwrap();
            rt.host
                .engine
                .submit(rt.client_id("game").unwrap(), &batch)
                .unwrap();
            let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
            renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
            shots.push(
                renderer
                    .capture(
                        &rt.host.scene,
                        &mut rt.assets,
                        rt.project.canvas,
                        rt.project.background,
                    )
                    .unwrap(),
            );
        }
        let darker = shots[0]
            .pixels()
            .zip(shots[1].pixels())
            .filter(|(a, b)| b.0[1] as i32 - a.0[1] as i32 > 20)
            .count();
        assert!(
            darker < 40,
            "{name}: invisible surface left a solid shadow: {darker} pixels"
        );
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn material_opacity_textures_resolve_under_each_client_capability_root() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let t = tempfile::tempdir().unwrap();
    for (name, value) in [("a", 0), ("b", 255)] {
        std::fs::create_dir_all(t.path().join(format!("assets/{name}"))).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([value, value, value, 255]))
            .save(t.path().join(format!("assets/{name}/opacity.png")))
            .unwrap();
    }
    let mut clients = Vec::new();
    for (name, x) in [("a", -1.3), ("b", 1.3)] {
        clients.push(serde_json::json!({"name":name,"adapter":"crysirror-script-v1","game":"materials","asset_prefix":name,"systems":{"entities":[{"id":1,"name":"pane","geometry":{"kind":"triangles","vertices":[[-1,-1,0],[1,-1,0],[1,1,0],[-1,1,0]],"indices":[0,1,2,0,2,3]},"transform":{"translation":[x,0,0]},"material":{"alpha_mode":"blend","unlit":true,"base_color_factor":[1,0,0,1],"opacity_texture":"opacity.png"}}]}}));
    }
    let p = serde_json::json!({"schema":2,"name":"scoped-materials","content_revision":"1","canvas":[128,64],"assets":"assets","clients":clients});
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut rt = Runtime::load(&t.path().join("project.json"), &t.path().join("state")).unwrap();
    let batch=serde_json::from_value(serde_json::json!({"world":[{"op":"set_camera","camera":{"eye":[0,0,5],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":100}},{"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"temporal_aa":false,"tonemap":"none"}}]})).unwrap();
    rt.host
        .engine
        .submit(rt.client_id("a").unwrap(), &batch)
        .unwrap();
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let image = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    assert!(image.get_pixel(46, 32).0[0] < 5);
    assert!(image.get_pixel(82, 32).0[0] > 250);
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn developer_surface_shaders_scope_parameters_time_and_errors_reach_pixels() {
    let t = tempfile::tempdir().unwrap();
    for (name, color) in [("a", "1.0,0.0,0.0"), ("b", "0.0,0.0,1.0")] {
        std::fs::create_dir_all(t.path().join(format!("assets/{name}"))).unwrap();
        let code=format!("fn crysirror_surface(i:SurfaceInput)->SurfaceOutput {{var o=surface_default(i);o.color=vec4<f32>({color},clamp(i.parameters[0].x+i.time*0.25,0.0,1.0));return o;}}");
        std::fs::write(t.path().join(format!("assets/{name}/effect.wgsl")), code).unwrap();
    }
    let clients:Vec<_>=[("a",-1.3,0.5),("b",1.3,1.0)].into_iter().map(|(name,x,alpha)|serde_json::json!({"name":name,"adapter":"crysirror-script-v1","game":"effects","asset_prefix":name,"systems":{"entities":[{"id":1,"name":"pane","geometry":{"kind":"triangles","vertices":[[-1,-1,0],[1,-1,0],[1,1,0],[-1,1,0]],"indices":[0,1,2,0,2,3]},"transform":{"translation":[x,0,0]},"material":{"alpha_mode":"blend","unlit":true,"effect":{"kind":"custom","asset":"effect.wgsl","parameters":[[alpha,0,0,0],[0,0,0,0],[0,0,0,0],[0,0,0,0]]}}}]}})).collect();
    let p = serde_json::json!({"schema":2,"name":"shader-scope","content_revision":"1","canvas":[128,64],"assets":"assets","clients":clients});
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut rt = Runtime::load(&t.path().join("project.json"), &t.path().join("state")).unwrap();
    let setup=serde_json::from_value(serde_json::json!({"world":[{"op":"set_camera","camera":{"eye":[0,0,5],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":100}},{"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"temporal_aa":false,"tonemap":"none"}}]})).unwrap();
    let a = rt.client_id("a").unwrap();
    rt.host.engine.submit(a, &setup).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let before = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            [0, 0, 0, 255],
        )
        .unwrap();
    assert!(
        before.get_pixel(46, 32).0[0].abs_diff(188) < 4,
        "half-alpha red {:?}",
        before.get_pixel(46, 32)
    );
    assert!(
        before.get_pixel(82, 32).0[2] > 250,
        "client b uses its blue code asset"
    );
    rt.tick(2000, InputFrame::default()).unwrap();
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let after = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            [0, 0, 0, 255],
        )
        .unwrap();
    assert!(
        after.get_pixel(46, 32).0[0] > 250,
        "shader time comes from world simulation"
    );
    std::fs::write(t.path().join("assets/a/bad.wgsl"), "fn broken shader").unwrap();
    let bad=serde_json::from_value(serde_json::json!({"world":[{"op":"set_material","id":1,"material":{"effect":{"kind":"custom","asset":"bad.wgsl"}}}]})).unwrap();
    rt.host.engine.submit(a, &bad).unwrap();
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let error = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            [0, 0, 0, 255],
        )
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("bad.wgsl") && message.contains("ABI v1"),
        "{message}"
    );
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn shader_presets_and_authored_uv_custom_waves_animate_in_native_data_project() {
    let t = tempfile::tempdir().unwrap();
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shader-effects/project.json");
    let mut rt = Runtime::load(&path, t.path()).unwrap();
    rt.project.canvas = [640, 270];
    rt.tick(0, InputFrame::default()).unwrap();
    let setup=serde_json::from_value(serde_json::json!({"world":[{"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"temporal_aa":false,"tonemap":"none"}}]})).unwrap();
    let c = rt.client_id("game").unwrap();
    rt.host.engine.submit(c, &setup).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut frames = Vec::new();
    for time in [0, 3000] {
        rt.tick(time, InputFrame::default()).unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        frames.push(
            renderer
                .capture(
                    &rt.host.scene,
                    &mut rt.assets,
                    rt.project.canvas,
                    [0, 0, 0, 255],
                )
                .unwrap(),
        );
    }
    let changed = |left: u32, right: u32| {
        (left..right)
            .flat_map(|x| (50..220).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                frames[0].get_pixel(x, y).0[..3]
                    .iter()
                    .zip(frames[1].get_pixel(x, y).0[..3].iter())
                    .any(|(a, b)| a.abs_diff(*b) > 10)
            })
            .count()
    };
    assert!(
        changed(30, 175) > 200,
        "fog noise animates through authored UVs"
    );
    assert!(changed(470, 620) > 200, "developer WGSL waves animate");
    assert_eq!(
        changed(185, 310),
        0,
        "fresnel is stable when camera and geometry are static"
    );
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn clustered_point_and_spot_lights_light_only_their_regions() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |lights_on: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let p = serde_json::json!({"schema":2,"name":"lights","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"lights","systems":{"entities":[
                {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[10,0.1,10]},"color":[230,230,230,255]},
                {"id":2,"name":"red point","transform":{"translation":[-2,0.5,0]},
                 "light":{"color":[1,0.1,0.05],"intensity":0.6,"range":4,"enabled":lights_on}},
                {"id":3,"name":"blue spot","transform":{"translation":[2,2,0],"rotation_degrees":[-90,0,0]},
                 "light":{"shape":{"kind":"spot","inner_degrees":15,"outer_degrees":25},"color":[0.1,0.3,1],"intensity":12,"range":6,"enabled":lights_on}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let env = serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,6,6],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
            {"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false}}]});
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(env).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let scene = rt.render_pbr().unwrap().expect("environment selects PBR");
        assert_eq!(scene.lights.len(), if lights_on { 2 } else { 0 });
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(scene)).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let lit = shot(true);
    let dark = shot(false);
    std::fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist")).ok();
    lit.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/lights-test.png")).ok();
    let view = glam::Mat4::look_at_rh(glam::Vec3::new(0., 6., 6.), glam::Vec3::ZERO, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(50f32.to_radians(), 320. / 180., 0.05, 100.);
    let px = |x: f32, z: f32| {
        let c = proj * view * glam::Vec4::new(x, 0., z, 1.);
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        (((nx + 1.) * 0.5 * 320.) as u32, ((1. - ny) * 0.5 * 180.) as u32)
    };
    let at = |img: &image::RgbaImage, (x, y): (u32, u32)| img.get_pixel(x, y).0.map(u32::from);
    // Without lights, sun or IBL the floor is black.
    let black = at(&dark, px(-2., 0.));
    assert!(black[0] < 8 && black[1] < 8 && black[2] < 8, "unlit floor {black:?}");
    // Under the point light: red dominant and bright.
    let red = at(&lit, px(-2., 0.));
    assert!(red[0] > 120 && red[0] > red[2] * 3, "point light {red:?}");
    // Under the spot: blue dominant.
    let blue = at(&lit, px(2., 0.));
    assert!(blue[2] > 120 && blue[2] > blue[0] * 2, "spot light {blue:?}");
    // Outside the spot cone (radius ≈ 0.93 m) but within range: dark.
    let outside = at(&lit, px(2., 1.6));
    assert!(outside[2] < 25, "spot cone must cut off: {outside:?}");
    // Beyond the point light's range: dark.
    let far = at(&lit, px(-4.8, -3.5));
    assert!(far[0] < 10, "range window must end the light: {far:?}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn directional_and_ambient_volumes_and_exponent_falloff_stay_inside_their_ranges() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"volumes","content_revision":"1","canvas":[320,180],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"volumes","systems":{"entities":[
            {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[10,0.1,10]},"color":[230,230,230,255]},
            {"id":2,"name":"green directional volume","transform":{"translation":[-2.5,0,0],"rotation_degrees":[-90,0,0]},
             "light":{"shape":{"kind":"directional"},"color":[0.1,1,0.1],"intensity":3,"range":1.2}},
            {"id":3,"name":"blue ambient volume","transform":{"translation":[0,0,0]},
             "light":{"shape":{"kind":"ambient"},"color":[0.1,0.2,1],"intensity":6,"range":1.2}},
            {"id":4,"name":"red exponent point","transform":{"translation":[2.5,0.3,0]},
             "light":{"color":[1,0.1,0.1],"intensity":3,"range":1.2,"falloff":{"kind":"exponent","exponent":2}}}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
    let env = serde_json::json!({"world":[
        {"op":"set_camera","camera":{"eye":[0,6,6],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
        {"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false}}]});
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(env).unwrap();
    rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let scene = rt.render_pbr().unwrap().unwrap();
    assert_eq!(scene.lights.len(), 3);
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(Some(scene)).unwrap();
    let img = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    std::fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist")).ok();
    img.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/light-volumes-test.png")).ok();
    let view = glam::Mat4::look_at_rh(glam::Vec3::new(0., 6., 6.), glam::Vec3::ZERO, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(50f32.to_radians(), 320. / 180., 0.05, 100.);
    let at = |x: f32, z: f32| {
        let c = proj * view * glam::Vec4::new(x, 0., z, 1.);
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        img.get_pixel(((nx + 1.) * 0.5 * 320.) as u32, ((1. - ny) * 0.5 * 180.) as u32).0.map(u32::from)
    };
    let green = at(-2.5, 0.);
    assert!(green[1] > 100 && green[1] > green[0] * 2 && green[1] > green[2] * 2, "directional volume {green:?}");
    let blue = at(0., 0.);
    assert!(blue[2] > 100 && blue[2] > blue[0] * 2, "ambient volume {blue:?}");
    let red = at(2.5, 0.);
    assert!(red[0] > 100 && red[0] > red[1] * 2, "exponent point {red:?}");
    // Outside every range (between the volumes, and far away) stays black.
    for (x, z) in [(-1.25, 0.), (1.25, 0.), (0., 2.5)] {
        let c = at(x, z);
        assert!(c[0] < 10 && c[1] < 10 && c[2] < 10, "outside volumes at ({x},{z}): {c:?}");
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn two_channel_normal_maps_and_specular_strength_change_shading() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |material: serde_json::Value, sun: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 128, 255, 230])).save(dir.join("assets/n.png")).unwrap();
        let p = serde_json::json!({"schema":2,"name":"mat","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"mat","systems":{"entities":[
                {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[20,0.1,20]},"material":material}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let env = if sun {
            serde_json::json!({"sun":{"direction":[0,-1,0],"intensity":3,"shadows":false},"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false})
        } else {
            serde_json::json!({"sun":null,"intensity":1,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false})
        };
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,1.5,6],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        let img = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
        let p = img.get_pixel(80, 70).0;
        p[0] as u32 + p[1] as u32 + p[2] as u32
    };
    let flat = shot(serde_json::json!({}), true);
    let rgb = shot(serde_json::json!({"normal_texture":"n.png","normal_encoding":"rgb"}), true);
    let ag = shot(serde_json::json!({"normal_texture":"n.png","normal_encoding":"ag"}), true);
    // (128,128,255) is flat as RGB; alpha 230 tilts X by ~0.8 as DXT5nm.
    assert!(rgb.abs_diff(flat) <= 6, "rgb flat {rgb} vs {flat}");
    assert!(ag + 60 < flat, "ag tilt must darken an overhead-lit floor: {ag} vs {flat}");
    // Black albedo: only specular reflection of the environment remains.
    let spec = shot(serde_json::json!({"base_color_factor":[0,0,0,1],"roughness_factor":0.2}), false);
    let none = shot(serde_json::json!({"base_color_factor":[0,0,0,1],"roughness_factor":0.2,"specular_factor":0}), false);
    assert!(spec > none + 15 && none <= 3, "specular strength: {spec} vs {none}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn local_point_and_spot_shadows_darken_only_occluded_floor() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |shape: serde_json::Value, shadows: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let p = serde_json::json!({"schema":2,"name":"shadows","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"shadows","systems":{"entities":[
                {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[10,0.1,10]},"color":[230,230,230,255]},
                {"id":2,"name":"blocker","geometry":{"kind":"cube"},"transform":{"translation":[0,1.5,0],"scale":[0.8,0.1,0.8]},"color":[230,230,230,255]},
                {"id":3,"name":"lamp","transform":{"translation":[0,3,0],"rotation_degrees":[-90,0,0]},
                 "light":{"shape":shape,"intensity":20,"range":8,"shadows":shadows}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,7,5],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
            {"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false}}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let view = glam::Mat4::look_at_rh(glam::Vec3::new(0., 7., 5.), glam::Vec3::ZERO, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(50f32.to_radians(), 320. / 180., 0.05, 100.);
    let lum = |img: &image::RgbaImage, x: f32, z: f32| {
        let c = proj * view * glam::Vec4::new(x, 0., z, 1.);
        let p = img.get_pixel(((c.x / c.w + 1.) * 160.) as u32, ((1. - c.y / c.w) * 90.) as u32).0;
        p[0] as u32 + p[1] as u32 + p[2] as u32
    };
    for (name, shape) in [
        ("point", serde_json::json!({"kind":"point"})),
        ("spot", serde_json::json!({"kind":"spot","inner_degrees":40,"outer_degrees":60})),
    ] {
        let lit = shot(shape.clone(), false);
        let shadowed = shot(shape, true);
        shadowed.save(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../dist/local-shadow-{name}.png"))).ok();
        // Under the blocker (umbra radius ≈ 0.8 m on the floor) it is dark.
        let (u_lit, u_sh) = (lum(&lit, 0.3, 0.3), lum(&shadowed, 0.3, 0.3));
        assert!(u_lit > 150 && u_sh * 4 < u_lit, "{name} umbra: lit {u_lit} shadowed {u_sh}");
        // Away from the blocker both renders agree (no acne / no leaking dark).
        let (o_lit, o_sh) = (lum(&lit, 2.0, 0.), lum(&shadowed, 2.0, 0.));
        assert!(o_lit > 30 && o_sh.abs_diff(o_lit) * 10 <= o_lit, "{name} open floor: lit {o_lit} shadowed {o_sh}");
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn height_fog_and_auto_exposure_reach_pixels() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |env: serde_json::Value| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let p = serde_json::json!({"schema":2,"name":"atmo","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"atmo","systems":{"entities":[
                {"id":1,"name":"ground","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,-100],"scale":[400,0.1,400]},"color":[128,128,128,255]}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,2,0],"target":[0,1.6,-10],"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":500}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let base = serde_json::json!({"sun":{"direction":[0.2,-1,0.3],"intensity":3,"shadows":false},"intensity":0.3,"bloom":0,"ambient_occlusion":0,"temporal_aa":false,"tonemap":"none"});
    let mut fogged = base.clone();
    fogged["fog"] = serde_json::json!({"color":[1,0,0],"density":0.2,"height_falloff":0.05,"max_opacity":1});
    let clear = shot(base.clone());
    let fog = shot(fogged);
    fog.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/fog-test.png")).ok();
    let px = |i: &image::RgbaImage, x, y| i.get_pixel(x, y).0;
    // Near ground (bottom rows, ~2.5 m away) barely changes; the horizon turns red.
    let (near_c, near_f) = (px(&clear, 80, 88), px(&fog, 80, 88));
    assert!(near_f[1].abs_diff(near_c[1]) < 70, "near {near_c:?} vs {near_f:?}");
    let far = px(&fog, 80, 47);
    assert!(far[0] > 200 && far[1] < 60 && far[2] < 60, "fogged horizon {far:?}");
    // Auto exposure brings a 1/64-brightness scene back toward middle gray.
    let mut dim = base.clone();
    dim["sun"]["intensity"] = serde_json::json!(3.0 / 64.0);
    dim["intensity"] = serde_json::json!(0.3 / 64.0);
    let mean = |i: &image::RgbaImage| i.pixels().map(|p| p.0[1] as u64).sum::<u64>() / (160 * 90);
    let dark = mean(&shot(dim.clone()));
    dim["auto_exposure"] = serde_json::json!({});
    let adapted = mean(&shot(dim));
    assert!(dark < 20 && adapted > 60, "auto exposure: fixed {dark} adapted {adapted}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn siglus_style_layer_filters_match_reference_arithmetic() {
    use crysirror_scene::layer::{LayerSpec, LayerSystem};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    // Inverting tone curve, a white mask with half alpha, and a left-red/right-blue image.
    image::RgbaImage::from_fn(256, 1, |x, _| image::Rgba([255 - x as u8, 255 - x as u8, 255 - x as u8, 255])).save(dir.join("assets/curve.png")).unwrap();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 128])).save(dir.join("assets/mask.png")).unwrap();
    image::RgbaImage::from_fn(16, 16, |x, _| if x < 8 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) }).save(dir.join("assets/split.png")).unwrap();
    let p = serde_json::json!({"schema":2,"name":"filters","content_revision":"1","canvas":[256,32],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"filters"}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let client = rt.client_id("game").unwrap();
    let base = [128u8, 64, 32, 255];
    let filters = [
        serde_json::json!({"mono":1}),
        serde_json::json!({"invert":1}),
        serde_json::json!({"bright":0.2}),
        serde_json::json!({"dark":0.1}),
        serde_json::json!({"color_rate":0.5,"color":[0,0,1]}),
        serde_json::json!({"add":[0.1,0,-0.1]}),
        serde_json::json!({"tone_curve":{"asset":"curve.png"}}),
        serde_json::json!({"mask":{"asset":"mask.png","gray_alpha":true}}),
    ];
    let mut layers: Vec<LayerSpec> = filters
        .iter()
        .enumerate()
        .map(|(i, f)| serde_json::from_value(serde_json::json!({"id":i,"rect":[i*20,0,16,16],"paint":{"kind":"solid","color":base},"filter":f})).unwrap())
        .collect();
    // Threshold fade measures the texture sample (as Siglus `sample_shimi`):
    // red (luma 0.299) stays, blue (0.115) fades to the background.
    layers.push(serde_json::from_value(serde_json::json!({"id":21,"rect":[160,0,32,32],"paint":{"kind":"image","asset":"split.png"},
        "filter":{"distortion":{"kind":"threshold_fade","fade":0,"threshold":0.2}}})).unwrap());
    // Mosaic with one cell spanning the whole image: every pixel takes the top-left (red) sample.
    layers.push(serde_json::from_value(serde_json::json!({"id":20,"rect":[200,0,32,32],"paint":{"kind":"image","asset":"split.png"},
        "filter":{"distortion":{"kind":"mosaic","cut":1.0}}})).unwrap());
    let mut system = LayerSystem::default();
    system.install(client, &layers).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_engine_frame(system.frame([256, 32]).unwrap()).unwrap();
    let bg = [20u8, 40, 60, 255];
    let img = renderer.capture(&rt.host.scene, &mut rt.assets, [256, 32], bg).unwrap();
    img.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/layer-filters-test.png")).ok();
    let g = |v: u8| v as f32 / 255.;
    let y = 0.2989 * g(128) + 0.5886 * g(64) + 0.1145 * g(32);
    let px = |v: [f32; 3]| v.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
    let expected = [
        px([y, y, y]),
        px([1. - g(128), 1. - g(64), 1. - g(32)]),
        px([g(128) + 0.2, g(64) + 0.2, g(32) + 0.2]),
        px([g(128) - 0.1, g(64) - 0.1, g(32) - 0.1]),
        px([g(128) * 0.5, g(64) * 0.5, g(32) * 0.5 + 0.5]),
        px([g(128) + 0.1, g(64), g(32) - 0.1]),
        px([1. - g(128), 1. - g(64), 1. - g(32)]),
    ];
    for (i, e) in expected.iter().enumerate() {
        let a = img.get_pixel(i as u32 * 20 + 8, 8).0;
        for c in 0..3 {
            assert!(a[c].abs_diff(e[c]) <= 3, "filter {i} ({}): actual {a:?} expected {e:?}", filters[i]);
        }
    }
    // Gray-as-alpha: the final gray becomes coverage (mask alpha is replaced).
    let linear = crysirror_wgpu::geometry::linear;
    let enc = |v: f32| (if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1. / 2.4) - 0.055 } * 255.).round() as u8;
    let a = img.get_pixel(7 * 20 + 8, 8).0;
    for c in 0..3 {
        let e = enc(linear(base[c]) * y + linear(bg[c]) * (1. - y));
        assert!(a[c].abs_diff(e) <= 3, "gray alpha channel {c}: {a:?} vs {e}");
    }
    let red = img.get_pixel(160 + 4, 16).0;
    assert!(red[0] > 240 && red[2] < 10, "threshold keeps red {red:?}");
    assert_eq!(img.get_pixel(160 + 28, 16).0, bg, "threshold fades blue");
    // Mosaic cell = whole image → right half shows the top-left red, not blue.
    let m = img.get_pixel(200 + 28, 16).0;
    assert!(m[0] > 240 && m[2] < 10, "mosaic {m:?}");
}

#[test]
#[ignore = "requires a real wgpu adapter or Mesa software Vulkan; CI GPU job runs this explicitly"]
fn screen_transitions_compose_old_and_new_frames_with_siglus_arithmetic() {
    use crysirror_scene::ServiceBatch;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    // Left half black, right half white: white reveals first.
    image::RgbaImage::from_fn(8, 8, |x, _| if x < 4 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255; 4]) })
        .save(dir.join("assets/halves.png"))
        .unwrap();
    let p = serde_json::json!({"schema":2,"name":"wipes","content_revision":"1","canvas":[64,32],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"wipes"}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let bg = [0u8, 0, 0, 255];
    let run = |effect: serde_json::Value, elapsed: u64| {
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("state-{elapsed}-{}", effect["kind"]))).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let client = rt.client_id("game").unwrap();
        let put = |color: [u8; 4]| serde_json::json!({"op":"put","layer":{"id":1,"rect":[0,0,64,32],"paint":{"kind":"solid","color":color}}});
        let submit = |rt: &mut Runtime, ops: serde_json::Value| {
            let batch: ServiceBatch = serde_json::from_value(serde_json::json!({ "layers": ops })).unwrap();
            rt.host.engine.submit(client, &batch).unwrap();
        };
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        submit(&mut rt, serde_json::json!([put([255, 0, 0, 255])]));
        renderer.set_engine_frame(rt.host.engine.frame([64, 32]).unwrap()).unwrap();
        renderer.set_transition(rt.render_transition().unwrap()).unwrap();
        let old = renderer.capture(&rt.host.scene, &mut rt.assets, [64, 32], bg).unwrap();
        assert_eq!(old.get_pixel(32, 16).0, [255, 0, 0, 255]);
        submit(&mut rt, serde_json::json!([put([0, 0, 255, 255]), {"op":"transition","transition":{"effect":effect,"duration_ms":1000}}]));
        rt.tick(elapsed, InputFrame::default()).unwrap();
        renderer.set_engine_frame(rt.host.engine.frame([64, 32]).unwrap()).unwrap();
        renderer.set_transition(rt.render_transition().unwrap()).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, [64, 32], bg).unwrap()
    };
    // Crossfade midpoint blends gamma-encoded bytes (Siglus), not linear light.
    let img = run(serde_json::json!({"kind":"crossfade"}), 500);
    img.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/transition-crossfade-test.png")).ok();
    let c = img.get_pixel(32, 16).0;
    assert!(c[0].abs_diff(128) <= 2 && c[1] <= 2 && c[2].abs_diff(128) <= 2, "gamma-space crossfade {c:?}");
    // Hard-edged image mask at 50%: white half shows the new scene, black half the old.
    let img = run(serde_json::json!({"kind":"mask","asset":"halves.png"}), 500);
    assert_eq!(img.get_pixel(8, 16).0, [255, 0, 0, 255], "black mask still old");
    assert_eq!(img.get_pixel(56, 16).0, [0, 0, 255, 255], "white mask already new");
    // Generated Siglus pattern mask (type 5, original wipe_mask generator).
    let img = run(serde_json::json!({"kind":"siglus","code":5}), 500);
    img.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/transition-pattern-test.png")).ok();
    let reds = img.pixels().filter(|p| p.0[0] > 200 && p.0[2] < 50).count();
    let blues = img.pixels().filter(|p| p.0[2] > 200 && p.0[0] < 50).count();
    assert!(reds > 64 && blues > 64, "pattern mid-wipe shows both scenes: {reds} red / {blues} blue");
    // Finished transitions are removed and present the new frame directly.
    let img = run(serde_json::json!({"kind":"crossfade"}), 1000);
    assert_eq!(img.get_pixel(32, 16).0, [0, 0, 255, 255]);
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn screen_space_reflections_show_the_object_in_a_mirror_floor() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |env: serde_json::Value| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let p = serde_json::json!({"schema":2,"name":"ssr","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"ssr","systems":{"entities":[
                {"id":1,"name":"mirror","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,-10],"scale":[40,0.1,40]},
                 "material":{"base_color_factor":[1,1,1,1],"metallic_factor":1,"roughness_factor":0.02}},
                {"id":2,"name":"red","geometry":{"kind":"cube"},"transform":{"translation":[0,1,-6],"scale":[2,2,0.5]},
                 "material":{"base_color_factor":[1,0,0,1],"unlit":true}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,1,0],"target":[0,0.6,-10],"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":200}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let base = serde_json::json!({"sun":{"direction":[0.2,-1,0.3],"intensity":0,"shadows":false},"intensity":1,"bloom":0,
        "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none"});
    let mut with = base.clone();
    with["reflections"] = serde_json::json!({});
    let off = shot(base);
    let on = shot(with);
    on.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/ssr-test.png")).ok();
    off.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/ssr-off-test.png")).ok();
    let px = |i: &image::RgbaImage, x, y| i.get_pixel(x, y).0;
    // The floor point mirroring the box center (~3 m ahead) turns red only with SSR.
    let (a, b) = (px(&off, 80, 67), px(&on, 80, 67));
    assert!(b[0] > 150 && b[0] as i32 > b[1] as i32 + 80 && b[0] as i32 > b[2] as i32 + 80, "SSR reflection {b:?} (off {a:?})");
    assert!((a[0] as i32) < a[2] as i32 + 40, "environment-only mirror is not red {a:?}");
    // Floor beside the box keeps the environment reflection (no false hits).
    let (c, d) = (px(&off, 8, 86), px(&on, 8, 86));
    for k in 0..3 {
        assert!(c[k].abs_diff(d[k]) <= 12, "side floor {c:?} vs {d:?}");
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn box_reflection_probes_capture_offscreen_walls_in_the_right_directions() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let shot = |target: [f32; 3], probe: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let wall = |id: u32, t: [f32; 3], s: [f32; 3], c: [f32; 4]| serde_json::json!({"id":id,"name":format!("wall{id}"),"geometry":{"kind":"cube"},
            "transform":{"translation":t,"scale":s},"material":{"base_color_factor":c,"unlit":true}});
        let mut entities = vec![
            serde_json::json!({"id":1,"name":"mirror","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[16,0.1,16]},
                "material":{"base_color_factor":[1,1,1,1],"metallic_factor":1,"roughness_factor":0.05}}),
            wall(2, [0., 5., -8.1], [16., 10., 0.2], [1., 0., 0., 1.]),
            wall(3, [-8.1, 5., 0.], [0.2, 10., 16.], [0., 1., 0., 1.]),
            wall(4, [8.1, 5., 0.], [0.2, 10., 16.], [0., 0., 1., 1.]),
        ];
        if probe {
            entities.push(serde_json::json!({"id":5,"name":"probe","transform":{"translation":[0,4,0]},
                "reflection_probe":{"extent":[8,6,8],"blend_distance":0.5}}));
        }
        let p = serde_json::json!({"schema":2,"name":"probe","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"probe","systems":{"entities":entities}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("state")).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,2,0],"target":target,"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":200}},
            {"op":"set_environment","environment":{"sun":{"direction":[0.2,-1,0.3],"intensity":0,"shadows":false},"intensity":1,"bloom":0,
                "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none"}}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    // Each camera looks steeply down at the mirror toward one wall; the walls
    // themselves are off-screen, so only the capture can reflect them.
    for (target, channel, name) in [([0., 0., -1.5], 0, "front/red"), ([-1.5, 0., 0.], 1, "left/green"), ([1.5, 0., 0.], 2, "right/blue")] {
        let without = shot(target, false);
        let with = shot(target, true);
        with.save(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../dist/probe-test-{channel}.png"))).ok();
        let c = with.get_pixel(80, 45).0;
        let o = without.get_pixel(80, 45).0;
        let dominant = (0..3).filter(|k| *k != channel).all(|k| c[channel] as i32 > c[k] as i32 + 60);
        assert!(dominant && c[channel] > 120, "{name}: probe reflection {c:?} (sky only {o:?})");
        let sky_dominant = (0..3).filter(|k| *k != channel).all(|k| o[channel] as i32 > o[k] as i32 + 60);
        assert!(!sky_dominant, "{name}: environment-only mirror must not show the wall {o:?}");
        // The walls are not directly visible.
        assert!(without.pixels().all(|p| !(0..3).filter(|k| *k != channel).all(|k| p.0[channel] as i32 > p.0[k] as i32 + 100)), "{name}: wall on screen");
    }
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn authored_equirect_probe_without_parallax_uses_the_prefilter_direction_convention() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    // Flat RGBE: red around the -Z direction (u≈0.25, upper hemisphere), blue elsewhere.
    let (w, h) = (128u32, 64u32);
    let mut hdr = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {h} +X {w}\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let red = (0.12..0.38).contains(&u) && v < 0.5;
            hdr.extend_from_slice(if red { &[128, 0, 0, 129] } else { &[0, 0, 128, 129] });
        }
    }
    std::fs::write(dir.join("assets/env.hdr"), hdr).unwrap();
    let shot = |target: [f32; 3]| {
        let p = serde_json::json!({"schema":2,"name":"authored","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"authored","systems":{"entities":[
                {"id":1,"name":"mirror","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[40,0.1,40]},
                 "material":{"base_color_factor":[1,1,1,1],"metallic_factor":1,"roughness_factor":0.05}},
                {"id":2,"name":"probe","transform":{"translation":[0,0,0]},
                 "reflection_probe":{"extent":[100,100,100],"source":"env.hdr","parallax":false}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("state{}", target[0]))).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,2,0],"target":target,"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":200}},
            {"op":"set_environment","environment":{"sun":{"direction":[0.2,-1,0.3],"intensity":0,"shadows":false},"intensity":1,"bloom":0,
                "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false}}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let toward_minus_z = shot([0., 0., -1.5]).get_pixel(80, 45).0;
    let toward_plus_x = shot([1.5, 0., 0.]).get_pixel(80, 45).0;
    assert!(toward_minus_z[0] > 150 && toward_minus_z[2] < 60, "-Z reflection {toward_minus_z:?}");
    assert!(toward_plus_x[2] > 150 && toward_plus_x[0] < 60, "+X reflection {toward_plus_x:?}");
}

/// A dense UV sphere glTF (positions, normals, UVs, u32 indices) in `dir`.
fn write_sphere_gltf(dir: &Path, rings: u32, segments: u32) -> usize {
    let mut pos = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for r in 0..=rings {
        let th = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let ph = std::f32::consts::TAU * s as f32 / segments as f32;
            pos.push([th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()]);
        }
    }
    let at = |r: u32, s: u32| r * (segments + 1) + s;
    for r in 0..rings {
        for s in 0..segments {
            idx.extend([at(r, s), at(r, s + 1), at(r + 1, s), at(r, s + 1), at(r + 1, s + 1), at(r + 1, s)]);
        }
    }
    let mut bin = Vec::new();
    for p in &pos {
        for c in p {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for p in &pos {
        for c in p {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    let nv = pos.len();
    for i in &idx {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    std::fs::write(dir.join("sphere.bin"), &bin).unwrap();
    let g = serde_json::json!({
        "asset":{"version":"2.0"},
        "buffers":[{"uri":"sphere.bin","byteLength":bin.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":nv*12},
            {"buffer":0,"byteOffset":nv*12,"byteLength":nv*12},
            {"buffer":0,"byteOffset":nv*24,"byteLength":idx.len()*4}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":nv,"type":"VEC3","min":[-1,-1,-1],"max":[1,1,1]},
            {"bufferView":1,"componentType":5126,"count":nv,"type":"VEC3"},
            {"bufferView":2,"componentType":5125,"count":idx.len(),"type":"SCALAR"}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.8,0.3,0.2,1],"metallicFactor":0,"roughnessFactor":0.5}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    std::fs::write(dir.join("sphere.gltf"), serde_json::to_vec(&g).unwrap()).unwrap();
    idx.len() / 3
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn cluster_lod_draws_fewer_triangles_far_away_with_matching_pixels() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let source = write_sphere_gltf(&dir.join("assets"), 96, 192);
    let shot = |distance: f32, lod: bool| {
        let p = serde_json::json!({"schema":2,"name":"lod","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"lod","systems":{"entities":[
                {"id":1,"name":"sphere","geometry":{"kind":"model","asset":"sphere.gltf","mesh":0}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("s{distance}{lod}"))).unwrap();
        let mut env = serde_json::json!({"sun":{"direction":[-0.4,-1,-0.6],"intensity":3,"shadows":false},"intensity":0.3,"bloom":0,
            "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false});
        if lod {
            // CPU reference path (its statistics are exact per frame).
            env["geometry_lod"] = serde_json::json!({"pixel_error":1,"gpu":false});
        }
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[0,0,distance],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":40,"near":0.05,"far":500}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        let img = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
        (img, renderer.pbr().unwrap().lod_stats())
    };
    let (near_lod, near_stats) = shot(2.5, true);
    let (far_lod, far_stats) = shot(40., true);
    let (far_full, off_stats) = shot(40., false);
    far_lod.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/lod-far-test.png")).ok();
    near_lod.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/lod-near-test.png")).ok();
    assert_eq!(off_stats.lod_primitives, 0);
    // Zero-area pole triangles (degenerate after position welding) are dropped.
    assert!(far_stats.source_triangles <= source && far_stats.source_triangles * 100 >= source * 99, "{far_stats:?}");
    assert!(far_stats.drawn_triangles * 10 < source, "far cut {far_stats:?}");
    assert!(near_stats.drawn_triangles > far_stats.drawn_triangles * 4, "near {near_stats:?} far {far_stats:?}");
    // Within the 1 px error bound the far images agree except along the silhouette.
    let differing = far_lod.pixels().zip(far_full.pixels()).filter(|(a, b)| (0..3).any(|k| a.0[k].abs_diff(b.0[k]) > 24)).count();
    let covered = far_full.pixels().filter(|p| p.0[0] > 10).count();
    assert!(covered > 100 && differing * 10 < covered, "{differing} differing of {covered} covered pixels");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn virtual_shadow_maps_shadow_beyond_cascade_range_and_cache_pages() {
    use glam::{Mat4, Vec3};
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let (eye, target, size) = (Vec3::new(0., 8., 30.), Vec3::ZERO, [320u32, 180u32]);
    let shot = |vsm: bool| {
        let p = serde_json::json!({"schema":2,"name":"vsm","content_revision":"1","canvas":size,"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"vsm","systems":{"entities":[
                {"id":1,"name":"ground","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[200,0.1,200]},"color":[200,200,200,255]},
                {"id":2,"name":"blocker","geometry":{"kind":"cube"},"transform":{"translation":[0,1,0],"scale":[3,2,3]},"color":[200,200,200,255]}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("s{vsm}"))).unwrap();
        let mut env = serde_json::json!({"sun":{"direction":[0.6,-1,0],"intensity":3,"shadows":true},"intensity":0.2,"bloom":0,
            "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","shadow_distance":10,"show_sky":false});
        if vsm {
            env["virtual_shadows"] = serde_json::json!({"texel_size":0.02,"levels":10});
        }
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":eye.to_array(),"target":target.to_array(),"up":[0,1,0],"fov_y_degrees":40,"near":0.1,"far":500}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        let img = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    let first = renderer.pbr().unwrap().vsm_stats();
        let again = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
        let cached = renderer.pbr().unwrap().vsm_stats();
        (img, again, first, cached)
    };
    let vp = Mat4::perspective_rh(40f32.to_radians(), size[0] as f32 / size[1] as f32, 0.1, 500.) * Mat4::look_at_rh(eye, target, Vec3::Y);
    let px = |img: &image::RgbaImage, w: Vec3| {
        let c = vp.project_point3(w);
        let (x, y) = (((c.x * 0.5 + 0.5) * size[0] as f32) as u32, ((0.5 - c.y * 0.5) * size[1] as f32) as u32);
        img.get_pixel(x, y).0[1] as f32
    };
    // Shadow falls toward +X (light travels +X, down): x in (1.5, 2.7) behind the box edge.
    let (shadowed, lit) = (Vec3::new(2.2, 0., 0.5), Vec3::new(-4., 0., 0.5));
    let (vsm, vsm_again, first, cached) = shot(true);
    let (csm, ..) = shot(false);
    vsm.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/vsm-test.png")).ok();
    csm.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/vsm-csm-test.png")).ok();
    assert!(px(&vsm, shadowed) < 0.5 * px(&vsm, lit), "VSM shadow {} vs lit {}", px(&vsm, shadowed), px(&vsm, lit));
    assert!(px(&csm, shadowed) > 0.8 * px(&csm, lit), "cascades end at 10 m: {} vs {}", px(&csm, shadowed), px(&csm, lit));
    assert!(first.resident > 0 && first.dropped == 0, "{first:?}");
    assert!(cached.rendered == 0 && cached.resident == first.resident, "second capture reuses pages: {cached:?}");
    assert_eq!(vsm.as_raw(), vsm_again.as_raw(), "cached pages give the same image");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn moving_casters_rerender_only_their_virtual_shadow_pages() {
    use glam::{Mat4, Vec3};
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let (eye, target, size) = (Vec3::new(0., 25., 25.), Vec3::ZERO, [320u32, 180u32]);
    let p = serde_json::json!({"schema":2,"name":"vsm-move","content_revision":"1","canvas":size,"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"vsm","systems":{"entities":[
            {"id":1,"name":"ground","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[200,0.1,200]},"color":[200,200,200,255]},
            {"id":2,"name":"mover","geometry":{"kind":"cube"},"transform":{"translation":[-8,1,0],"scale":[2,2,2]},"color":[200,200,200,255]},
            {"id":3,"name":"pillar","geometry":{"kind":"cube"},"transform":{"translation":[8,2,0],"scale":[1,4,1]},"color":[200,200,200,255]}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("s")).unwrap();
    let client = rt.client_id("game").unwrap();
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
        {"op":"set_camera","camera":{"eye":eye.to_array(),"target":target.to_array(),"up":[0,1,0],"fov_y_degrees":50,"near":0.1,"far":500}},
        {"op":"set_environment","environment":{"sun":{"direction":[0,-1,0.5],"intensity":3,"shadows":true},"intensity":0.2,"bloom":0,
            "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false,"virtual_shadows":{}}}]})).unwrap();
    rt.host.engine.submit(client, &batch).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
    let before = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    before.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/vsm-move-before-test.png")).ok();
    let first = renderer.pbr().unwrap().vsm_stats();
    let resident = first.resident;
    // Move the box 6 m along +Z; the pillar stays.
    let mv: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
        {"op":"set_transform","id":2,"transform":{"translation":[-8,1,6],"scale":[2,2,2]}}]})).unwrap();
    rt.host.engine.submit(client, &mv).unwrap();
    rt.tick(16, InputFrame::default()).unwrap();
    renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
    let after = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    let stats = renderer.pbr().unwrap().vsm_stats();
    after.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/vsm-move-test.png")).ok();
    let vp = Mat4::perspective_rh(50f32.to_radians(), size[0] as f32 / size[1] as f32, 0.1, 500.) * Mat4::look_at_rh(eye, target, Vec3::Y);
    let px = |img: &image::RgbaImage, w: Vec3| {
        let c = vp.project_point3(w);
        img.get_pixel(((c.x * 0.5 + 0.5) * size[0] as f32) as u32, ((0.5 - c.y * 0.5) * size[1] as f32) as u32).0[1] as f32
    };
    // Light travels toward +Z: each shadow extends 0.5 × height beyond the
    // caster's +Z face (box: z 1..2, pillar: z 0.5..2.5).
    let lit = px(&before, Vec3::new(0., 0., 3.));
    let pillar_shadow = |img| px(img, Vec3::new(8., 0., 1.5));
    assert!(stats.invalidations == 0, "no full invalidation: {stats:?}");
    let rerendered = stats.total_rendered - first.total_rendered;
    eprintln!("VSM move: re-rendered {rerendered} of {resident} resident pages, dirtied {}", stats.total_dirtied - first.total_dirtied);
    assert!(stats.total_dirtied > first.total_dirtied && rerendered > 0 && (rerendered as usize) < resident,
        "only pages near the box re-render: {rerendered} of {resident} ({stats:?})");
    assert!(pillar_shadow(&before) < 0.6 * lit, "pillar casts a shadow");
    assert_eq!(pillar_shadow(&before), pillar_shadow(&after), "unmoved pillar shadow is identical");
    // Inside the new shadow (its far edge is softened by the level-4 texels
    // this 180-px-high view selects).
    assert!(px(&after, Vec3::new(-8., 0., 7.2)) < 0.6 * lit, "shadow follows the box");
    // The box's old ground footprint edge (still on screen) is lit again.
    let old_edge = Vec3::new(-8., 0., 1.5);
    assert!(px(&before, old_edge) < 0.6 * lit && px(&after, old_edge) > 0.9 * lit, "old shadow cleared: {} -> {} (lit {lit})", px(&before, old_edge), px(&after, old_edge));
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn gpu_cluster_selection_matches_the_cpu_cut_and_frustum_culls_the_main_view() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    write_sphere_gltf(&dir.join("assets"), 96, 192);
    let shot = |eye: [f32; 3], target: [f32; 3], gpu_mode: bool| {
        let p = serde_json::json!({"schema":2,"name":"lod-gpu","content_revision":"1","canvas":[320,180],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"lod","systems":{"entities":[
                {"id":1,"name":"sphere","geometry":{"kind":"model","asset":"sphere.gltf","mesh":0}}]}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("s{eye:?}{gpu_mode}"))).unwrap();
        let env = serde_json::json!({"sun":{"direction":[-0.4,-1,-0.6],"intensity":3,"shadows":false},"intensity":0.3,"bloom":0,
            "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false,"geometry_lod":{"pixel_error":1,"gpu":gpu_mode}});
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":eye,"target":target,"up":[0,1,0],"fov_y_degrees":40,"near":0.01,"far":500}},
            {"op":"set_environment","environment":env}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        let img = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
        let pbr = renderer.pbr().unwrap();
        let counts = if gpu_mode { pbr.debug_gpu_lod_counts(1).unwrap() } else { vec![] };
        (img, pbr.lod_stats(), counts)
    };
    for eye in [[0., 0., 2.5], [0., 0., 12.], [0., 0., 40.]] {
        let (cpu_img, cpu, _) = shot(eye, [0., 0., 0.], false);
        let (gpu_img, _, counts) = shot(eye, [0., 0., 0.], true);
        let (main, shadow) = counts[0];
        assert_eq!(shadow as usize, cpu.drawn_triangles * 3, "unculled GPU cut = CPU cut at {eye:?}");
        // At 2.5 m the sphere (angular radius 23.6°) exceeds the 40° view, so
        // the main view may cull; from 12 m on it is fully inside.
        if eye[2] > 10. {
            assert_eq!(main, shadow, "whole sphere visible at {eye:?}");
        } else {
            assert!(main <= shadow);
        }
        let differing = cpu_img.pixels().zip(gpu_img.pixels()).filter(|(a, b)| (0..3).any(|k| a.0[k].abs_diff(b.0[k]) > 2)).count();
        assert!(differing <= 4, "{differing} pixels differ at {eye:?}");
    }
    // Close-up of one side: the main view culls clusters outside the frustum,
    // the shadow cut keeps them.
    let (_, _, counts) = shot([0., 0., 1.3], [0.6, 0., 0.], true);
    let (main, shadow) = counts[0];
    assert!(main * 2 < shadow, "frustum culling main {main} vs shadow {shadow}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn irradiance_volume_bounces_emissive_light_onto_the_room() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let shot = |gi: bool| {
        let white = serde_json::json!({"base_color_factor":[0.8,0.8,0.8,1],"roughness_factor":0.9});
        let slab = |id: u32, t: [f32; 3], s: [f32; 3], m: &serde_json::Value| serde_json::json!({"id":id,"name":format!("slab{id}"),
            "geometry":{"kind":"cube"},"transform":{"translation":t,"scale":s},"material":m});
        let mut entities = vec![
            slab(1, [0., -0.1, 0.], [10., 0.2, 10.], &white),
            slab(2, [0., 4.1, 0.], [10., 0.2, 10.], &white),
            slab(3, [5.1, 2., 0.], [0.2, 4., 10.], &white),
            slab(4, [0., 2., 5.1], [10., 4., 0.2], &white),
            slab(5, [0., 2., -5.1], [10., 4., 0.2], &white),
            slab(6, [-5.1, 2., 0.], [0.2, 4., 10.], &serde_json::json!({"base_color_factor":[0,0,0,1],"emissive_factor":[1,0.05,0.05],"emissive_strength":4})),
        ];
        if gi {
            entities.push(serde_json::json!({"id":7,"name":"gi","transform":{"translation":[0,2,0]},
                "irradiance_volume":{"extent":[4.9,1.95,4.9],"counts":[6,3,6],"probes_per_frame":108}}));
        }
        let p = serde_json::json!({"schema":2,"name":"gi","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"gi","systems":{"entities":entities}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("s{gi}"))).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":[3,3,3],"target":[-2,0,0],"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":100}},
            {"op":"set_environment","environment":{"sun":{"direction":[0,-1,0],"intensity":0,"shadows":false},"intensity":0,"bloom":0,
                "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false}}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap()
    };
    let off = shot(false);
    let on = shot(true);
    on.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/gi-test.png")).ok();
    off.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/gi-off-test.png")).ok();
    // Floor centre of the image (in front of the red wall).
    let (a, b) = (off.get_pixel(80, 70).0, on.get_pixel(80, 70).0);
    assert!(a[0] < 5 && a[1] < 5, "no GI: emissive light does not reach the floor {a:?}");
    assert!(b[0] > 40 && b[0] as i32 > b[1] as i32 + 25, "GI: floor lit red by the emissive wall {b:?}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn irradiance_visibility_stops_light_leaking_through_a_wall_and_disables_buried_probes() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    // Two closed rooms split by a 0.2 m wall at x = 0. The left room has a red
    // emissive wall; the right room has no light at all. Probes at x = ±0.98
    // straddle the wall, and a pillar in the right room buries three probes.
    let shot = |gi: Option<bool>, eye: [f32; 3], target: [f32; 3]| {
        let white = serde_json::json!({"base_color_factor":[0.8,0.8,0.8,1],"roughness_factor":0.9});
        let slab = |id: u32, t: [f32; 3], s: [f32; 3], m: &serde_json::Value| serde_json::json!({"id":id,"name":format!("slab{id}"),
            "geometry":{"kind":"cube"},"transform":{"translation":t,"scale":s},"material":m});
        let mut entities = vec![
            slab(1, [0., -0.1, 0.], [10.4, 0.2, 10.4], &white),
            slab(2, [0., 4.1, 0.], [10.4, 0.2, 10.4], &white),
            slab(3, [5.1, 2., 0.], [0.2, 4., 10.], &white),
            slab(4, [0., 2., 5.1], [10., 4., 0.2], &white),
            slab(5, [0., 2., -5.1], [10., 4., 0.2], &white),
            slab(6, [-5.1, 2., 0.], [0.2, 4., 10.], &serde_json::json!({"base_color_factor":[0,0,0,1],"emissive_factor":[1,0.05,0.05],"emissive_strength":4})),
            slab(8, [0., 2., 0.], [0.2, 4., 10.], &white),
            slab(9, [2.94, 2., 2.94], [1.2, 4., 1.2], &white),
        ];
        if let Some(visibility) = gi {
            entities.push(serde_json::json!({"id":7,"name":"gi","transform":{"translation":[0,2,0]},
                "irradiance_volume":{"extent":[4.9,1.95,4.9],"counts":[6,3,6],"probes_per_frame":108,"visibility":visibility}}));
        }
        let p = serde_json::json!({"schema":2,"name":"gi","content_revision":"1","canvas":[160,90],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"gi","systems":{"entities":entities}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        let mut rt = Runtime::load(&dir.join("project.json"), &dir.join(format!("s{gi:?}{eye:?}"))).unwrap();
        let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
            {"op":"set_camera","camera":{"eye":eye,"target":target,"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":100}},
            {"op":"set_environment","environment":{"sun":{"direction":[0,-1,0],"intensity":0,"shadows":false},"intensity":0,"bloom":0,
                "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false}}]})).unwrap();
        rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
        let image = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
        let pbr = renderer.pbr().unwrap();
        let states = if gi.is_some() { pbr.debug_gi_probe_states().unwrap() } else { Vec::new() };
        (image, states, pbr.gi_stats())
    };
    let red = |img: &image::RgbaImage| img.pixels().map(|p| p.0[0] as f64).sum::<f64>() / img.pixels().len() as f64;
    // The dark room, looking at the floor along the dividing wall.
    let (dark_eye, dark_target) = ([4.5, 3.5, -4.], [0.3, 0., 0.]);
    let (none, ..) = shot(None, dark_eye, dark_target);
    let (leaky, ..) = shot(Some(false), dark_eye, dark_target);
    let (tight, states, stats) = shot(Some(true), dark_eye, dark_target);
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    leaky.save(out.join("gi-leak-off-test.png")).ok();
    tight.save(out.join("gi-leak-on-test.png")).ok();
    let (r0, r1, r2) = (red(&none), red(&leaky), red(&tight));
    println!("dark room mean red: no GI {r0:.2}, no visibility {r1:.2}, visibility {r2:.2}");
    assert!(r0 < 1.0, "no light source in the dark room: {r0}");
    assert!(r1 > 8.0, "without visibility the red light leaks through the wall: {r1}");
    assert!(r2 < r1 * 0.25, "visibility removes most of the leak: {r2} vs {r1}");
    // The lit room keeps its bounce light.
    let (lit, ..) = shot(Some(true), [-1., 3., 3.], [-4., 0., 0.]);
    lit.save(out.join("gi-leak-lit-test.png")).ok();
    assert!(red(&lit) > 30.0, "the lit room is still lit by GI: {}", red(&lit));
    // Probe classification: exactly the three probes inside the pillar
    // (x = 2.94, z = 2.94 at every height) are disabled.
    assert_eq!(states.len(), 108);
    assert!(states.iter().all(|s| s[2] == 1.0), "every probe was refreshed");
    let disabled: Vec<usize> = (0..108).filter(|&i| states[i][0] < 0.5).collect();
    let expected: Vec<usize> = (0..3).map(|y| 4 + 6 * (y + 3 * 4)).collect();
    println!("disabled probes {disabled:?}, back-face fractions {:?}", disabled.iter().map(|&i| states[i][1]).collect::<Vec<_>>());
    assert_eq!(disabled, expected, "buried probes are disabled");
    assert!(stats.converged && stats.sweeps >= 3, "{stats:?}");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn irradiance_volume_idles_when_converged_and_restarts_after_a_change() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let white = serde_json::json!({"base_color_factor":[0.8,0.8,0.8,1],"roughness_factor":0.9});
    let slab = |id: u32, t: [f32; 3], s: [f32; 3], m: &serde_json::Value| serde_json::json!({"id":id,"name":format!("slab{id}"),
        "geometry":{"kind":"cube"},"transform":{"translation":t,"scale":s},"material":m});
    let entities = vec![
        slab(1, [0., -0.1, 0.], [10., 0.2, 10.], &white),
        slab(2, [0., 4.1, 0.], [10., 0.2, 10.], &white),
        slab(3, [-5.1, 2., 0.], [0.2, 4., 10.], &serde_json::json!({"base_color_factor":[0,0,0,1],"emissive_factor":[1,0.05,0.05],"emissive_strength":4})),
        serde_json::json!({"id":7,"name":"gi","transform":{"translation":[0,2,0]},
            "irradiance_volume":{"extent":[4.9,1.95,4.9],"counts":[4,2,4],"probes_per_frame":32}}),
    ];
    let p = serde_json::json!({"schema":2,"name":"gi","content_revision":"1","canvas":[96,54],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"gi","systems":{"entities":entities}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("s")).unwrap();
    let client = rt.client_id("game").unwrap();
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
        {"op":"set_camera","camera":{"eye":[3,3,3],"target":[-2,0,0],"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":100}},
        {"op":"set_environment","environment":{"sun":{"direction":[0,-1,0],"intensity":0,"shadows":false},"intensity":0,"bloom":0,
            "ambient_occlusion":0,"temporal_aa":false,"tonemap":"none","show_sky":false}}]})).unwrap();
    rt.host.engine.submit(client, &batch).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
    let first = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    let s1 = renderer.pbr().unwrap().gi_stats();
    assert!(s1.converged && s1.total_refreshed >= 3 * 32, "{s1:?}");
    // Nothing changed: no refreshes at all (idle_probes_per_frame = 0).
    let again = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    let s2 = renderer.pbr().unwrap().gi_stats();
    assert_eq!(s2.total_refreshed, s1.total_refreshed, "converged volume idles");
    assert!(first == again, "idle frames reproduce the image");
    // Moving the emissive wall restarts the refresh sweeps.
    let mv: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"world":[
        {"op":"set_transform","id":3,"transform":{"translation":[-3.,2.,0.],"scale":[0.2,4.,10.]}}]})).unwrap();
    rt.host.engine.submit(client, &mv).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    renderer.set_pbr_scene(Some(rt.render_pbr().unwrap().unwrap())).unwrap();
    let moved = renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, rt.project.background).unwrap();
    let s3 = renderer.pbr().unwrap().gi_stats();
    assert!(s3.total_refreshed >= s2.total_refreshed + 3 * 32, "change restarts refreshes: {s3:?}");
    assert!(first != moved, "the moved light source changes the lighting");
}

#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn puppet_layers_render_meshes_with_masks_blends_and_deformation() {
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("assets/p")).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(dir.join("assets/p/white.png")).unwrap();
    // 100×100 rig: a red face, an invisible mask window (40..60), a blue
    // dot (30..70) clipped to the window, a green ring (30..70) drawn
    // outside the window only, and a red arm rotated by `angle`.
    let quad = |id: &str, x: f32, y: f32, w: f32, h: f32, rgba: [f32; 4], order: i32| {
        serde_json::json!({"id":id,"texture":0,"positions":[[x,y],[x+w,y],[x,y+h],[x+w,y+h]],
            "uvs":[[0,0],[1,0],[0,1],[1,1]],"indices":[0,1,2,2,1,3],"colors":[rgba,rgba,rgba,rgba],"order":order})
    };
    let mut window = quad("window", 40., 40., 20., 20., [1.; 4], 1);
    window["visible"] = false.into();
    let mut dot = quad("dot", 30., 30., 40., 40., [0., 0., 1., 1.], 3);
    dot["masks"] = serde_json::json!(["window"]);
    let mut ring = quad("ring", 30., 30., 40., 40., [0., 1., 0., 1.], 2);
    ring["masks"] = serde_json::json!(["window"]);
    ring["invert_mask"] = true.into();
    let mut arm = quad("arm", 50., 0., 4., 20., [1., 1., 0., 1.], 4);
    arm["deformer"] = "shoulder".into();
    let rig = serde_json::json!({"format":"crysirror-puppet","version":1,"size":[100,100],"textures":["white.png"],
        "parameters":[{"id":"angle","min":-90,"max":90,"default":0}],
        "deformers":[{"id":"shoulder","pivot":[52,20],"keyforms":[{"parameter":"angle","keys":[{"value":-90,"rotation":-90},{"value":90,"rotation":90}]}]}],
        "parts":[quad("face", 10., 10., 80., 80., [1., 0., 0., 1.], 0), window, ring, dot, arm]});
    std::fs::write(dir.join("assets/p/rig.json"), serde_json::to_vec(&rig).unwrap()).unwrap();
    let p = serde_json::json!({"schema":2,"name":"puppet","content_revision":"1","canvas":[100,100],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"puppet",
            "systems":{"layers":[{"id":1,"rect":[0,0,100,100],"paint":{"kind":"puppet","asset":"p/rig.json"}}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut rt = Runtime::load(&dir.join("project.json"), &dir.join("s")).unwrap();
    rt.tick(16, InputFrame::default()).unwrap();
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut shot = |rt: &mut Runtime| {
        renderer.set_engine_frame(rt.host.engine.frame(rt.project.canvas).unwrap()).unwrap();
        renderer.capture(&rt.host.scene, &mut rt.assets, rt.project.canvas, [0, 0, 0, 255]).unwrap()
    };
    let rest = shot(&mut rt);
    rest.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/puppet-test.png")).ok();
    let px = |img: &image::RgbaImage, x, y| img.get_pixel(x, y).0;
    assert_eq!(px(&rest, 50, 50)[..3], [0, 0, 255], "dot inside the mask window");
    assert_eq!(px(&rest, 35, 50)[..3], [0, 255, 0], "inverted ring outside the window, dot clipped");
    assert_eq!(px(&rest, 20, 80)[..3], [255, 0, 0], "face");
    assert_eq!(px(&rest, 3, 3)[..3], [0, 0, 0], "transparent outside the puppet");
    assert_eq!(px(&rest, 52, 10)[..3], [255, 255, 0], "arm hangs up at rest");
    // Rotate the arm 90°: it now points left of the shoulder.
    let batch: crysirror_scene::ServiceBatch = serde_json::from_value(serde_json::json!({"layers":[
        {"op":"puppet","id":1,"command":{"kind":"set_parameter","parameter":"angle","value":90}}]})).unwrap();
    rt.host.engine.submit(rt.client_id("game").unwrap(), &batch).unwrap();
    rt.tick(16, InputFrame::default()).unwrap();
    let turned = shot(&mut rt);
    turned.save(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/puppet-turned-test.png")).ok();
    assert_eq!(px(&turned, 52, 10)[..3], [255, 0, 0], "arm left its rest position");
    let yellow = |img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32| {
        (x0..x1).flat_map(|x| (y0..y1).map(move |y| (x, y))).filter(|(x, y)| px(img, *x, *y)[..3] == [255, 255, 0]).count()
    };
    assert!(yellow(&turned, 53, 75, 17, 24) > 30, "rotated arm extends sideways from the shoulder");
}
