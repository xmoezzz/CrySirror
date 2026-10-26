// SPDX-License-Identifier: MPL-2.0
//! Explicit real-device integration; unavailable hardware is not a passing skip.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, Renderer, RenderingPath};
use serde_json::json;
use std::sync::Arc;
fn artifact_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/verification/hardware-rt")
}
fn gpu() -> Arc<Gpu> {
    let gpu = pollster::block_on(Gpu::request_for(
        &wgpu::Instance::default(),
        None,
        RenderingPath::HardwareRaytrace,
    ))
    .expect("explicit native ray-query device required");
    eprintln!(
        "{}",
        serde_json::to_string(&gpu.hardware_rt_report()).unwrap()
    );
    assert!(gpu.capabilities().hardware_raytrace);
    Arc::new(gpu)
}
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn hardware_primary_matches_compute_geometry_and_composes_engine_overlay() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/desktop/arena.json")).unwrap();
    project["canvas"] = json!([160, 90]);
    std::fs::write(&p, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    let batch=serde_json::from_value(json!({"layers":[{"op":"put","layer":{"id":42,"rect":[0,0,12,12],"paint":{"kind":"solid","color":[0,255,0,255]}}}]})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
    let gpu = gpu();
    let mut images = vec![];
    let mut hw_stats = None;
    for path in [
        RenderingPath::ComputeRaytrace,
        RenderingPath::HardwareRaytrace,
    ] {
        eprintln!("integration: creating {path:?} renderer");
        let mut r = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        r.set_rendering_path(path).unwrap();
        r.set_scene3d(rt.render_scene3d().unwrap().as_ref())
            .unwrap();
        r.set_engine_frame(rt.host.engine.frame([160, 90]).unwrap())
            .unwrap();
        eprintln!("integration: capture {path:?}");
        let img = r
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [160, 90],
                rt.project.background,
            )
            .unwrap();
        eprintln!("integration: capture {path:?} complete");
        assert_eq!(img.get_pixel(4, 4).0, [0, 255, 0, 255]);
        if path == RenderingPath::HardwareRaytrace {
            let stats = r.hardware_rt_stats().unwrap();
            assert!(stats["primary_dispatches"].as_u64().unwrap() > 0);
            let builds = stats["builds_encoded"].clone();
            r.capture(
                &rt.host.scene,
                &mut rt.assets,
                [160, 90],
                rt.project.background,
            )
            .unwrap();
            assert_eq!(r.hardware_rt_stats().unwrap()["builds_encoded"], builds);
            hw_stats = Some(stats);
        }
        images.push(img);
    }
    let different = images[0]
        .pixels()
        .zip(images[1].pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 3))
        .count();
    assert!(
        different < 160 * 90 / 20,
        "hardware and compute disagreed at {different} pixels"
    );
    eprintln!("hardware encoded stats {}", hw_stats.unwrap());
    std::fs::create_dir_all(artifact_dir()).unwrap();
    images[1].save(artifact_dir().join("primary.png")).unwrap();
}
fn pbr(root: &std::path::Path, shadow: bool) -> Runtime {
    std::fs::create_dir(root.join("assets")).unwrap();
    let p = root.join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"hardware-pbr","content_revision":"1","canvas":[192,128],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"fixture","systems":{"entities":[{"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[8,0.1,8]},"color":[200,200,200,255]},{"id":2,"name":"box","geometry":{"kind":"cube"},"transform":{"translation":[0,0.5,0]},"color":[220,60,40,255]}]}}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(&p, &root.join("state")).unwrap();
    let batch=serde_json::from_value(json!({"world":[{"op":"set_camera","camera":{"eye":[3,2.5,3],"target":[0,0.5,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},{"op":"set_environment","environment":{"sun":{"direction":[1,-1,0.3],"intensity":4,"shadows":shadow},"bloom":0,"temporal_aa":false,"ambient_occlusion":0.7}}]})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    rt
}
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn pbr_hardware_sun_shadows_change_actual_pixels_and_alpha_test_masked_casters() {
    let gpu = gpu();
    let mut shots = vec![];
    for shadows in [true, false] {
        let t = tempfile::tempdir().unwrap();
        let mut rt = pbr(t.path(), shadows);
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer
            .set_rendering_path(RenderingPath::HardwareRaytrace)
            .unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        let image = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [192, 128],
                rt.project.background,
            )
            .unwrap();
        let stats = renderer.hardware_rt_stats().unwrap();
        eprintln!("PBR shadow stats {stats}");
        assert_eq!(
            stats["static_blas"], 1,
            "identical cube topology shares one BLAS"
        );
        assert_eq!(stats["dynamic_blas"], 0);
        assert_eq!(stats["instances"], 2);
        if shadows {
            renderer
                .capture(
                    &rt.host.scene,
                    &mut rt.assets,
                    [192, 128],
                    rt.project.background,
                )
                .unwrap();
            assert_eq!(
                renderer.hardware_rt_stats().unwrap()["tlas_builds_encoded"],
                stats["tlas_builds_encoded"],
                "static frame must reuse TLAS"
            );
            let moved=serde_json::from_value(json!({"world":[{"op":"set_transform","id":2,"transform":{"translation":[0.8,0.5,0]}}]})).unwrap();
            rt.host.engine.submit(ClientId(1), &moved).unwrap();
            renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
            let moved_image = renderer
                .capture(
                    &rt.host.scene,
                    &mut rt.assets,
                    [192, 128],
                    rt.project.background,
                )
                .unwrap();
            let moved_stats = renderer.hardware_rt_stats().unwrap();
            assert_eq!(
                moved_stats["static_builds_encoded"], stats["static_builds_encoded"],
                "rigid move rebuilds only TLAS"
            );
            assert!(
                moved_stats["tlas_builds_encoded"].as_u64().unwrap()
                    > stats["tlas_builds_encoded"].as_u64().unwrap()
            );
            assert!(
                image
                    .pixels()
                    .zip(moved_image.pixels())
                    .filter(|(a, b)| a != b)
                    .count()
                    > 100
            );
        }
        shots.push(image.clone());
        if shadows {
            // Alpha-tested caster: covered (alpha 1) still casts; cut out
            // (alpha 0.2 < cutoff 0.5) casts nothing, like the raster shadow pass.
            let mut masked = vec![];
            for alpha in [1.0, 0.2] {
                let mask = serde_json::from_value(json!({"world":[{"op":"set_material","id":2,
                    "material":{"alpha_mode":"mask","base_color_factor":[0.9,0.25,0.15,alpha]}}]}))
                .unwrap();
                rt.host.engine.submit(ClientId(1), &mask).unwrap();
                renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
                masked.push(
                    renderer
                        .capture(&rt.host.scene, &mut rt.assets, [192, 128], rt.project.background)
                        .unwrap(),
                );
                let s = renderer.hardware_rt_stats().unwrap();
                assert_eq!(s["alpha_instances"], 1, "{s}");
            }
            // Floor pixels lit again once the box is cut out.
            let relit = masked[0]
                .pixels()
                .zip(masked[1].pixels())
                .filter(|(a, b)| i16::from(b[1]) - i16::from(a[1]) > 20)
                .count();
            assert!(relit > 30, "cut-out caster still shadows ({relit} pixels relit)");
            std::fs::create_dir_all(artifact_dir()).unwrap();
            masked[1].save(artifact_dir().join("pbr-mask-cutout.png")).unwrap();
        }
        renderer.set_rendering_path(RenderingPath::Raster).unwrap();
        renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [192, 128],
                rt.project.background,
            )
            .unwrap();
        assert!(
            renderer.hardware_rt_stats().is_none(),
            "explicit raster selection releases RT state and accepts masked materials"
        );
    }
    let darkened = shots[0]
        .pixels()
        .zip(shots[1].pixels())
        .filter(|(a, b)| i16::from(b[1]) - i16::from(a[1]) > 20)
        .count();
    assert!(
        darkened > 30,
        "hardware shadow changed only {darkened} pixels"
    );
    std::fs::create_dir_all(artifact_dir()).unwrap();
    shots[0]
        .save(artifact_dir().join("pbr-shadows.png"))
        .unwrap();
    shots[1]
        .save(artifact_dir().join("pbr-no-shadows.png"))
        .unwrap();
}

/// Skinned geometry goes through the GPU skinning pass into a resident
/// dynamic BLAS that is rebuilt only when the pose changes.
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn pbr_hardware_skinned_caster_uses_resident_dynamic_blas() {
    let gpu = gpu();
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../crysirror-asset/tests/data/bar.fbx"),
        t.path().join("assets/bar.fbx"),
    )
    .unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p, serde_json::to_vec(&json!({"schema":2,"name":"hardware-skin","content_revision":"1","canvas":[160,120],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"fixture","systems":{"entities":[
            {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[6,0.1,6]},"color":[200,200,200,255]},
            {"id":2,"name":"bar","geometry":{"kind":"scene","asset":"bar.fbx"},"animation":{"layers":[{"clip":"Rig|RigAction","time":0.0,"speed":1.0,"weight":1.0,"target_weight":1.0,"fade":0.0,"looping":true}]}}]}}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    let batch = serde_json::from_value(json!({"world":[{"op":"set_camera","camera":{"eye":[2.5,1.8,2.5],"target":[0,0.8,0],"up":[0,1,0],"fov_y_degrees":50,"near":0.05,"far":100}},
        {"op":"set_environment","environment":{"sun":{"direction":[1,-1,0.3],"intensity":4,"shadows":true},"bloom":0,"temporal_aa":false}}]})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_rendering_path(RenderingPath::HardwareRaytrace).unwrap();
    let mut shot = |rt: &mut Runtime, ms: u64| {
        rt.tick(ms, InputFrame::default()).unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        let image = renderer.capture(&rt.host.scene, &mut rt.assets, [160, 120], rt.project.background).unwrap();
        (image, renderer.hardware_rt_stats().unwrap())
    };
    let (start, s0) = shot(&mut rt, 0);
    eprintln!("skinned RT stats {s0}");
    assert_eq!(s0["dynamic_blas"], 1, "{s0}");
    let (_, s1) = shot(&mut rt, 0);
    assert_eq!(s1["dynamic_rebuilds_encoded"], s0["dynamic_rebuilds_encoded"], "unchanged pose reuses the dynamic BLAS: {s1}");
    let (bent, s2) = shot(&mut rt, 900);
    assert!(s2["dynamic_rebuilds_encoded"].as_u64() > s1["dynamic_rebuilds_encoded"].as_u64(), "{s2}");
    let changed = start.pixels().zip(bent.pixels()).filter(|(a, b)| a != b).count();
    assert!(changed > 100, "bent pose changed only {changed} pixels");
    std::fs::create_dir_all(artifact_dir()).unwrap();
    bent.save(artifact_dir().join("pbr-skinned-shadow.png")).unwrap();
}

/// A plane whose texture alpha is a 2x2 checker: with alpha testing about
/// half of its ray-traced shadow disappears, following the texture via the
/// gathered UVs and the baked alpha atlas.
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn pbr_hardware_alpha_texture_cuts_holes_in_the_shadow() {
    use crysirror_asset::interchange::glb;
    let gpu = gpu();
    let mut dark = vec![];
    for (mode, name) in [("OPAQUE", "opaque"), ("MASK", "mask")] {
        let mut checker = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([255, 255, 255, if (x + y) % 2 == 0 { 255 } else { 0 }])
        }))
        .write_to(&mut std::io::Cursor::new(&mut checker), image::ImageFormat::Png)
        .unwrap();
        let mut doc = glb::Document { generator: "rt alpha test".into(), ..Default::default() };
        doc.images.push((checker, "image/png"));
        doc.materials.push(glb::Material {
            base_color_texture: Some(0),
            alpha_mode: mode,
            double_sided: true,
            ..Default::default()
        });
        doc.meshes.push((
            "plane".into(),
            vec![glb::Primitive {
                positions: vec![[-0.6, 0., -0.6], [0.6, 0., -0.6], [0.6, 0., 0.6], [-0.6, 0., 0.6]],
                normals: vec![[0., 1., 0.]; 4],
                uvs: vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
                indices: vec![0, 2, 1, 0, 3, 2],
                material: Some(0),
                ..Default::default()
            }],
        ));
        let mut node = glb::Node::named("plane");
        node.mesh = Some(0);
        doc.nodes.push(node);
        doc.roots.push(0);
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("assets")).unwrap();
        std::fs::write(t.path().join("assets/plane.glb"), doc.to_glb().unwrap()).unwrap();
        let p = t.path().join("project.json");
        std::fs::write(&p, serde_json::to_vec(&json!({"schema":2,"name":"hardware-alpha","content_revision":"1","canvas":[160,120],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"fixture","systems":{"entities":[
                {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[8,0.1,8]},"color":[220,220,220,255]},
                {"id":2,"name":"plane","geometry":{"kind":"scene","asset":"plane.glb"},"transform":{"translation":[0,1.0,0]}}]}}]})).unwrap()).unwrap();
        let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
        // Sun from straight above: the plane's shadow lies directly beneath it;
        // the camera looks at the floor from the side.
        let batch = serde_json::from_value(json!({"world":[{"op":"set_camera","camera":{"eye":[0,0.6,3.2],"target":[0,0.0,0],"up":[0,1,0],"fov_y_degrees":45,"near":0.05,"far":100}},
            {"op":"set_environment","environment":{"sun":{"direction":[0.0,-1,0.0001],"intensity":4,"shadows":true},"bloom":0,"temporal_aa":false,"ambient_occlusion":0}}]})).unwrap();
        rt.host.engine.submit(ClientId(1), &batch).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
        renderer.set_rendering_path(RenderingPath::HardwareRaytrace).unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        let image = renderer.capture(&rt.host.scene, &mut rt.assets, [160, 120], rt.project.background).unwrap();
        let stats = renderer.hardware_rt_stats().unwrap();
        eprintln!("{name} {stats}");
        if mode == "MASK" {
            assert_eq!(stats["alpha_instances"], 1);
            assert_eq!(stats["alpha_slots"], 1);
        }
        // Floor pixels in shadow: well below the lit floor's brightness.
        let lit = image.get_pixel(150, 110).0;
        let lit = u32::from(lit[0]) + u32::from(lit[1]) + u32::from(lit[2]);
        let count = image
            .pixels()
            .filter(|p| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) * 2 < lit)
            .count();
        std::fs::create_dir_all(artifact_dir()).unwrap();
        image.save(artifact_dir().join(format!("pbr-alpha-texture-{name}.png"))).unwrap();
        dark.push(count);
    }
    eprintln!("shadowed floor pixels: opaque {} masked {}", dark[0], dark[1]);
    assert!(dark[0] > 200, "opaque plane must cast a shadow: {dark:?}");
    let ratio = dark[1] as f32 / dark[0] as f32;
    assert!((0.25..0.75).contains(&ratio), "checker alpha should remove about half the shadow: {dark:?}");
}
