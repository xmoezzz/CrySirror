// SPDX-License-Identifier: MPL-2.0
//! Synthetic shared-engine scenes, compared against the pinned scalar Bayer
//! reveal formula. No source VM, game assets or CPU-composed screen is used.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::ServiceBatch;
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::sync::Arc;

fn expected(x: u32, y: u32, cell: [u32; 2], diagonal: bool, steps: u32, progress: f64) -> [u8; 4] {
    if progress >= 1. {
        return [0, 0, 255, 255];
    }
    let table = [
        [0u8, 8, 2, 10],
        [12, 4, 14, 6],
        [3, 11, 1, 9],
        [15, 7, 13, 5],
    ];
    let (mut i, mut j) = (x / cell[0], y / cell[1]);
    if diagonal {
        (i, j) = (i + j, i + 3 * j);
    }
    let threshold =
        f64::from(table[j as usize % 4][i as usize % 4]) / 16. * (15. / 16.) + 0.5 / 16.;
    let alpha = ((progress - threshold) / (f64::from(steps) / 16.) + 0.5).clamp(0., 1.);
    [
        (255. * (1. - alpha)).round() as u8,
        0,
        (255. * alpha).round() as u8,
        255,
    ]
}
#[test]
fn scalar_oracle_distinguishes_ordered_cells_diagonal_and_soft_width() {
    assert_eq!(expected(0, 0, [1, 1], false, 1, 0.), [255, 0, 0, 255]);
    assert_eq!(expected(0, 0, [1, 1], false, 1, 0.5), [0, 0, 255, 255]);
    assert_eq!(expected(2, 1, [1, 1], false, 1, 0.5), [255, 0, 0, 255]);
    assert_eq!(expected(2, 1, [1, 1], true, 1, 0.5), [0, 0, 255, 255]);
    assert_eq!(
        expected(3, 5, [2, 3], false, 1, 0.5),
        expected(1, 1, [1, 1], false, 1, 0.5)
    );
    assert!(
        expected(0, 0, [1, 1], false, 3, 0.)[2] > 0,
        "soft-width start is not silently forced to old"
    );
}
#[test]
#[ignore = "requires a real wgpu adapter; execute explicitly"]
fn shared_gpu_ordered_dither_matches_scalar_profile_and_is_not_a_crossfade() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"ordered-dither","content_revision":"1","canvas":[16,16],"assets":"assets",
        "clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]});
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
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let put = |color| json!({"op":"put","layer":{"id":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":color}}});
    let mut comparisons = 0;
    for (cell, diagonal, steps) in [
        ([1, 1], false, 1),
        ([2, 3], false, 1),
        ([1, 1], true, 1),
        ([2, 2], true, 3),
    ] {
        for elapsed in [0, 250, 500, 750, 1000] {
            let batch: ServiceBatch = serde_json::from_value(
                json!({"layers":[{"op":"end_transition"},put([255,0,0,255])]}),
            )
            .unwrap();
            rt.host.engine.submit(ClientId(1), &batch).unwrap();
            renderer
                .set_engine_frame(rt.host.engine.frame([16, 16]).unwrap())
                .unwrap();
            renderer.set_transition(None).unwrap();
            renderer
                .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 255])
                .unwrap();
            let batch:ServiceBatch=serde_json::from_value(json!({"layers":[put([0,0,255,255]),{"op":"transition","transition":{
                "duration_ms":1000,"effect":{"kind":"ordered_dither","cell":cell,"diagonal":diagonal,"steps":steps}}}]})).unwrap();
            rt.host.engine.submit(ClientId(1), &batch).unwrap();
            rt.tick(elapsed, InputFrame::default()).unwrap();
            renderer
                .set_engine_frame(rt.host.engine.frame([16, 16]).unwrap())
                .unwrap();
            renderer
                .set_transition(rt.render_transition().unwrap())
                .unwrap();
            let shot = renderer
                .capture(&rt.host.scene, &mut rt.assets, [16, 16], [0, 0, 0, 255])
                .unwrap();
            for (x, y, pixel) in shot.enumerate_pixels() {
                let expected = expected(x, y, cell, diagonal, steps, elapsed as f64 / 1000.);
                for (c, e) in pixel.0.into_iter().zip(expected) {
                    assert!(
                        c.abs_diff(e) <= 1,
                        "ordered dither mismatch at ({x},{y}) cell={cell:?},diagonal={diagonal},steps={steps},elapsed={elapsed}: {:?} vs {expected:?}",
                        pixel.0
                    );
                }
                comparisons += 1;
            }
        }
    }
    assert_eq!(comparisons, 5120);
}
