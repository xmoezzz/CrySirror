// SPDX-License-Identifier: MPL-2.0
use crysirror_core::service::InputFrame;
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, Renderer};
use std::sync::Arc;
#[test]
#[ignore = "requires a real timestamp-query GPU; unavailable is a failed explicit run"]
fn completed_gpu_durations_are_nonzero_and_cpu_trace_contains_real_frames() {
    let state = tempfile::tempdir().unwrap();
    let project =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/arena.json");
    let mut runtime = Runtime::load(&project, state.path()).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    assert!(
        gpu.device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY),
        "explicit timestamp validation needs a capable device"
    );
    eprintln!("timestamp device {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer
        .set_scene3d(runtime.render_scene3d().unwrap().as_ref())
        .unwrap();
    for _ in 0..8 {
        runtime.tick(16, InputFrame::default()).unwrap();
        renderer.set_profiler(runtime.profiler.clone(), runtime.profiler.frame());
        renderer
            .capture(
                &runtime.host.scene,
                &mut runtime.assets,
                [64, 64],
                runtime.project.background,
            )
            .unwrap();
    }
    let report = renderer.profile_report_offline().unwrap();
    assert_eq!(report["gpu_timestamps_enabled"], true);
    let stages = report["timings"]["stages"].as_array().unwrap();
    let gpu = stages
        .iter()
        .find(|s| s["name"] == "gpu:frame")
        .expect("valid completed GPU samples required");
    assert!(gpu["samples"].as_u64().unwrap() > 0);
    assert!(gpu["mean_us"].as_f64().unwrap() > 0.);
    assert!(
        report["cpu_trace"]["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "renderer.prepare_encode_submit"
                && e["args"]["frame"].as_u64().unwrap() > 0)
    );
    eprintln!(
        "timestamp diagnostics {}",
        serde_json::json!({"gpu":gpu,"pending":report["gpu_samples_pending"],"dropped":report["gpu_samples_dropped"],"failed":report["gpu_samples_failed"]})
    );
}
