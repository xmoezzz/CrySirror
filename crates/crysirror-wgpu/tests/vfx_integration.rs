// SPDX-License-Identifier: MPL-2.0
//! Device test of the public data/runtime/snapshot/PBR/submission boundary.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::{world::WorldCommand, ServiceBatch};
use crysirror_wgpu::{pbr::PbrRenderer, Gpu, Renderer};
use serde_json::json;
use std::sync::Arc;

#[test]
#[ignore = "requires a real GPU; engine-owned fixed timeline, actual PBR pixels and submission receipts"]
fn engine_gpu_effect_replays_all_ticks_and_pauses_without_resimulating_draws() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("assets")).unwrap();
    let effect = json!({"emitters":[{"id":"resident","target":"gpu","capacity":64,
        "spawn":{"rate":0,"bursts":[{"time":0,"count":64}],"looping":false},
        "initialize":{"shape":{"kind":"box","half_extent":[0.7,0.7,0.1]},"lifetime":[5,5],"speed":[0,0],"size":[0.09,0.09],"color":[0.1,0.5,2,1]},
        "render":{"kind":"sprite","blend":"additive"}}]});
    std::fs::write(temp.path().join("project.json"),serde_json::to_vec(&json!({
        "schema":2,"name":"vfx-gpu-integration","content_revision":"1","canvas":[96,96],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"vfx","systems":{"entities":[{"id":1,"name":"fx","vfx":effect}]}}]
    })).unwrap()).unwrap();
    let mut rt = Runtime::load(
        &temp.path().join("project.json"),
        &temp.path().join("state"),
    )
    .unwrap();
    rt.host.engine.submit(ClientId(1),&serde_json::from_value(json!({"world":[
        {"op":"set_camera","camera":{"eye":[0,0,3],"target":[0,0,0],"up":[0,1,0],"fov_y_degrees":45,"near":0.1,"far":20}},
        {"op":"set_environment","environment":{"sun":null,"intensity":0,"show_sky":false,"temporal_aa":false,"bloom":0,"ambient_occlusion":0}}
    ]})).unwrap()).unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    rt.tick(1000, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    let frame = rt.vfx.frame();
    assert!(
        frame.emitters.is_empty(),
        "GPU target must not simulate a CPU particle array"
    );
    let gpu_frame = &frame.gpu_emitters[0];
    assert_eq!(gpu_frame.tick, 61);
    assert_eq!(gpu_frame.steps.len(), 61);
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    // Desktop surfaces commonly use BGRA. Capture must retain this renderer's
    // resident simulation while converting readback into ordinary RGBA pixels.
    let mut renderer = Renderer::new(gpu.clone(), wgpu::TextureFormat::Bgra8UnormSrgb);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let picture = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    let lit = picture
        .pixels()
        .filter(|p| p.0[2] > 40 && u16::from(p.0[2]) > u16::from(p.0[0]) * 2)
        .count();
    assert!(
        lit > 200,
        "GPU indirect particles must produce visible pixels, got {lit}"
    );
    assert_eq!(gpu_frame.progress.submitted_tick(), 61);
    let stats = renderer.pbr().unwrap().vfx_stats();
    assert_eq!(stats["gpu_simulations"][0]["submitted_steps"], 61);
    assert_eq!(stats["gpu_simulations"][0]["submitted_spawn_requests"], 64);
    assert!(stats["render"]["gpu_live_particles"].is_null());
    let h = rt.host.engine.world.handle(ClientId(1), 1).unwrap();
    let mut spec = rt.host.engine.world.entity(h).unwrap().vfx.clone().unwrap();
    spec.paused = true;
    rt.host
        .engine
        .submit(
            ClientId(1),
            &ServiceBatch {
                world: vec![WorldCommand::SetVfx {
                    id: 1,
                    vfx: Some(spec),
                }],
                ..Default::default()
            },
        )
        .unwrap();
    rt.tick(500, InputFrame::default()).unwrap();
    rt.finish_vfx_offline().unwrap();
    assert_eq!(rt.vfx.frame().gpu_emitters[0].tick, 61);
    renderer.set_pbr_scene(rt.render_pbr().unwrap()).unwrap();
    let paused = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    assert_eq!(
        picture, paused,
        "drawing a paused effect must retain resident state without advancing ticks"
    );
    assert_eq!(
        renderer.pbr().unwrap().vfx_stats()["gpu_simulations"][0]["submitted_steps"],
        61
    );

    // Direct encoders must not silently execute the same historical tick twice.
    // A fresh runtime has an unsubmitted timeline that can exercise this guard.
    rt.vfx.reset(rt.now_ms());
    rt.finish_vfx_offline().unwrap();
    // Restart the paused instance, then resume so its time-zero burst exists.
    let mut spec = rt.host.engine.world.entity(h).unwrap().vfx.clone().unwrap();
    spec.paused = false;
    spec.revision += 1;
    rt.host
        .engine
        .submit(
            ClientId(1),
            &ServiceBatch {
                world: vec![WorldCommand::SetVfx {
                    id: 1,
                    vfx: Some(spec),
                }],
                ..Default::default()
            },
        )
        .unwrap();
    rt.finish_vfx_offline().unwrap();
    let mut pbr = PbrRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb).unwrap();
    pbr.set_scene(rt.render_pbr().unwrap());
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("VFX direct encode target"),
        size: wgpu::Extent3d {
            width: 96,
            height: 96,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = target.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pbr.encode(
        &mut rt.assets,
        &mut encoder,
        &target,
        [96, 96],
        [0., 0., 96., 96.],
    )
    .unwrap();
    assert!(pbr
        .encode(
            &mut rt.assets,
            &mut encoder,
            &target,
            [96, 96],
            [0., 0., 96., 96.]
        )
        .unwrap_err()
        .to_string()
        .contains("previous PBR VFX encoding is pending"));
    drop(encoder);
    pbr.cancel_unsubmitted_vfx().unwrap();
    assert_eq!(
        rt.vfx.frame().gpu_emitters[0].progress.submitted_tick(),
        0,
        "abandoned encoding must not acknowledge commands"
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pbr.encode(
        &mut rt.assets,
        &mut encoder,
        &target,
        [96, 96],
        [0., 0., 96., 96.],
    )
    .unwrap();
    gpu.queue.submit([encoder.finish()]);
    pbr.after_submit().unwrap();
    assert_eq!(rt.vfx.frame().gpu_emitters[0].progress.submitted_tick(), 1);
}
