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
fn pbr_hardware_sun_shadows_change_actual_pixels_and_reject_masked_materials() {
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
        assert!(stats["shadow_dispatches"].as_u64().unwrap() > 0);
        assert_eq!(stats["primary_dispatches"], 0);
        eprintln!("PBR shadow stats {stats}");
        assert_eq!(
            stats["resident_blas"], 1,
            "identical cube topology shares one BLAS"
        );
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
                moved_stats["blas_builds_encoded"], stats["blas_builds_encoded"],
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
        shots.push(image);
        let mask = serde_json::from_value(
            json!({"world":[{"op":"set_material","id":2,"material":{"alpha_mode":"mask"}}]}),
        )
        .unwrap();
        rt.host.engine.submit(ClientId(1), &mask).unwrap();
        renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
        let error = renderer
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [192, 128],
                rt.project.background,
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("opaque casters"));
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
