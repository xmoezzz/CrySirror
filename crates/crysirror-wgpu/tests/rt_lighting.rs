// SPDX-License-Identifier: MPL-2.0
//! Native hardware behavioural evidence, deliberately opt-in and serialized.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, RenderTarget, Renderer, RenderingPath};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
fn artifacts() -> PathBuf {
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/verification/rt0060");
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn gpu() -> Arc<Gpu> {
    Arc::new(
        pollster::block_on(Gpu::request_for(
            &wgpu::Instance::default(),
            None,
            RenderingPath::HardwareRaytrace,
        ))
        .expect("native ray-query GPU required"),
    )
}
fn room(mode: &str, light: bool) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let slab = |id: u32, t: [f32; 3], s: [f32; 3], color: [f32; 4]| json!({"id":id,"name":format!("slab{id}"),"geometry":{"kind":"cube"},"transform":{"translation":t,"scale":s},"material":{"base_color_factor":color,"roughness_factor":0.9}});
    let mut entities = vec![
        slab(1, [0., -0.1, 0.], [8., 0.2, 8.], [0.8, 0.8, 0.8, 1.]),
        slab(2, [-3., 2., 0.], [0.2, 4., 8.], [0.8, 0.02, 0.01, 1.]),
    ];
    if light {
        entities.push(json!({"id":3,"name":"wall light","transform":{"translation":[-1.,2.,0.],"rotation_degrees":[0,90,0]},"light":{"shape":{"kind":"spot","inner_degrees":20,"outer_degrees":35},"color":[1,1,1],"intensity":30,"range":10,"shadows":true}}));
    }
    if mode == "raster" || mode == "probes" {
        entities.push(json!({"id":4,"name":"gi","transform":{"translation":[0,1.8,0]},"irradiance_volume":{"extent":[2.8,1.7,3.8],"counts":[4,3,4],"probes_per_frame":16,"idle_probes_per_frame":16,"ray_traced":mode=="probes"}}));
    }
    let p = json!({"schema":2,"name":"RT local GI","content_revision":"1","canvas":[160,90],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"test","systems":{"entities":entities}}]});
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut rt =
        Runtime::load(&dir.path().join("project.json"), &dir.path().join("state")).unwrap();
    let mut environment = json!({"sun":{"direction":[0,-1,0],"intensity":0,"shadows":false},"intensity":0,"show_sky":false,"bloom":0,"ambient_occlusion":0,"temporal_aa":false,"tonemap":"none"});
    if mode == "pixels" {
        environment["ray_traced_gi"] = json!({"samples":64,"max_distance":30});
    }
    let batch=serde_json::from_value(json!({"world":[{"op":"set_camera","camera":{"eye":[3,3,5],"target":[-1,0.8,0],"up":[0,1,0],"fov_y_degrees":60,"near":0.05,"far":100}},{"op":"set_environment","environment":environment}]})).unwrap();
    rt.host.engine.submit(ClientId(1), &batch).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    (dir, rt)
}
fn renderer(gpu: Arc<Gpu>, rt: &Runtime) -> Renderer {
    let mut r = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    r.set_rendering_path(RenderingPath::HardwareRaytrace)
        .unwrap();
    r.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    r
}
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn local_light_bounces_through_probe_and_per_pixel_gi_without_reflections() {
    let g = gpu();
    let mut scores = vec![];
    for (mode, on) in [
        ("off", true),
        ("probes", true),
        ("pixels", true),
        ("pixels", false),
    ] {
        let (_d, mut rt) = room(mode, on);
        let mut r = renderer(g.clone(), &rt);
        let img = r
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [160, 90],
                rt.project.background,
            )
            .unwrap();
        img.save(artifacts().join(format!("local-{mode}-{on}.png")))
            .unwrap();
        // Floor only, below the wall-light's spot cone; direct illumination is zero.
        let red: f32 = (64..86)
            .flat_map(|y| (30..130).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = img.get_pixel(x, y);
                (i16::from(p[0]) - i16::from(p[1])).max(0) as f32
            })
            .sum::<f32>()
            / 2200.;
        eprintln!(
            "mode {mode} light {on}: floor red {red}, stats {:?}",
            r.hardware_rt_stats()
        );
        if mode == "pixels" {
            assert!(
                r.hardware_rt_stats().unwrap()["pixel_gi_dispatches"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
        scores.push(red);
    }
    assert!(
        scores[0] < 2. && scores[3] < 2.,
        "without GI / without local light floor stays black: {scores:?}"
    );
    assert!(
        scores[1] > 5. && scores[2] > 5.,
        "both RT GI modes must bounce red local light: {scores:?}"
    );
    let (_d, mut rt) = room("pixels", true);
    let mut raster = Renderer::new(g.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
    raster.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let e = raster
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            [160, 90],
            rt.project.background,
        )
        .unwrap_err();
    assert!(e.to_string().contains("requires the hardware"), "{e}");
}
#[test]
#[ignore = "requires native ray-query/timestamp GPU; explicit benchmark"]
fn compare_raster_probe_rt_probe_and_per_pixel_frame_cost() {
    let g = gpu();
    let mut reports = vec![];
    for mode in ["raster", "probes", "pixels"] {
        let (_d, mut rt) = room(mode, true);
        let mut r = renderer(g.clone(), &rt);
        let warm_start = std::time::Instant::now();
        r.capture(
            &rt.host.scene,
            &mut rt.assets,
            [160, 90],
            rt.project.background,
        )
        .unwrap();
        let warm_ms = warm_start.elapsed().as_secs_f64() * 1000.;
        r.profile_report_offline().unwrap();
        rt.profiler.clear();
        let target = g.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GI benchmark"),
            size: wgpu::Extent3d {
                width: 160,
                height: 90,
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
        for i in 0..24 {
            r.set_profiler(rt.profiler.clone(), i + 1);
            r.render(
                &rt.host.scene,
                &mut rt.assets,
                RenderTarget {
                    view: &view,
                    size: [160, 90],
                },
                [160, 90],
                rt.project.background,
            )
            .unwrap();
            r.profile_report_offline().unwrap();
        }
        let report = r.profile_report_offline().unwrap();
        let stages = report["timings"]["stages"].as_array().unwrap();
        let timing = stages
            .iter()
            .find(|s| s["name"] == "gpu:frame")
            .expect("actual GPU timing required");
        assert!(
            timing["samples"].as_u64().unwrap() >= 20 && timing["mean_us"].as_f64().unwrap() > 0.
        );
        assert_eq!(report["gpu_samples_failed"], 0);
        let record = json!({"mode":mode,"canvas":[160,90],"device":g.hardware_rt_report(),"warmup_wall_ms":warm_ms,"warmup_policy":"raster 3 probe sweeps, RT 12 sweeps, pixels 1 frame; not equal-quality convergence","steady_probe_updates_per_frame":if mode=="pixels"{0}else{16},"pixel_rays":if mode=="pixels"{64}else{0},"samples":24,"profile":report,"stats":r.hardware_rt_stats()});
        eprintln!("mode {mode}: {timing}");
        reports.push(record);
    }
    std::fs::write(
        artifacts().join("gi-benchmark.json"),
        serde_json::to_vec_pretty(&Value::Array(reports)).unwrap(),
    )
    .unwrap();
}
#[test]
#[ignore = "requires native ray-query GPU; execute explicitly"]
fn ggx_rough_reflections_spread_the_lobe_without_fading_it_away() {
    let g = gpu();
    let mut images = vec![];
    for roughness in [0.01, 0.65] {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("assets")).unwrap();
        let p = json!({"schema":2,"name":"GGX reflection","content_revision":"1","canvas":[160,120],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"test","systems":{"entities":[
            {"id":1,"name":"floor","geometry":{"kind":"cube"},"transform":{"translation":[0,-0.05,0],"scale":[10,0.1,10]},"material":{"base_color_factor":[1,1,1,1],"metallic_factor":1,"roughness_factor":roughness}},
            {"id":2,"name":"offscreen emitter","geometry":{"kind":"cube"},"transform":{"translation":[0,2.2,-2],"scale":[2,1,1]},"material":{"base_color_factor":[0,0,0,1],"emissive_factor":[0.8,0.02,0.01]}}
        ]}}]});
        std::fs::write(
            d.path().join("project.json"),
            serde_json::to_vec(&p).unwrap(),
        )
        .unwrap();
        let mut rt =
            Runtime::load(&d.path().join("project.json"), &d.path().join("state")).unwrap();
        let b=serde_json::from_value(json!({"world":[{"op":"set_camera","camera":{"eye":[0,1.2,2.5],"target":[0,0,-0.5],"up":[0,1,0],"fov_y_degrees":40,"near":0.05,"far":100}},{"op":"set_environment","environment":{"intensity":0,"sun":{"direction":[0,-1,0],"intensity":0,"shadows":false},"bloom":0,"temporal_aa":false,"ambient_occlusion":0,"reflections":{"intensity":1,"max_distance":20,"ray_traced":true,"ray_samples":64},"tonemap":"none"}}]})).unwrap();
        rt.host.engine.submit(ClientId(1), &b).unwrap();
        rt.tick(0, InputFrame::default()).unwrap();
        let mut r = renderer(g.clone(), &rt);
        let image = r
            .capture(
                &rt.host.scene,
                &mut rt.assets,
                [160, 120],
                rt.project.background,
            )
            .unwrap();
        image
            .save(artifacts().join(format!("gloss-{roughness}.png")))
            .unwrap();
        images.push(image);
    }
    let mirror = &images[0];
    let rough = &images[1];
    let red = |p: &image::Rgba<u8>| (i16::from(p[0]) - i16::from(p[1])).max(0);
    let spread = mirror
        .pixels()
        .zip(rough.pixels())
        .filter(|(a, b)| red(a) < 3 && red(b) > 15)
        .count();
    let energy = |a: &image::RgbaImage| a.pixels().map(|p| u64::from(red(p) as u16)).sum::<u64>();
    eprintln!(
        "GGX: spread {spread} pixels, mirror energy {}, rough energy {}",
        energy(mirror),
        energy(rough)
    );
    assert!(
        spread > 150,
        "roughness must spread offscreen highlight outside mirror footprint"
    );
    assert!(
        energy(rough) > energy(mirror) / 6,
        "rough native reflection must survive above SSR fade roughness"
    );
}
