// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "reallive")]
//! Opt-in owner-provided content, never committed fixtures. The assertions test
//! engine-owned resources and edits at a paused native PC, beyond a screenshot.
#[path = "../src/input_track.rs"]
mod input_track;
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_runtime::{AdapterRegistry, Runtime};
use crysirror_wgpu::{Gpu, Renderer};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
#[test]
#[ignore = "requires CRYSIRROR_REALLIVE_PROJECT, CRYSIRROR_REALLIVE_INPUT and GPU/system font"]
fn rusur_opening_uses_retained_engine_nodes_and_can_edit_them_without_vm() {
    let project =
        PathBuf::from(std::env::var_os("CRYSIRROR_REALLIVE_PROJECT").expect("local project"));
    let input = PathBuf::from(std::env::var_os("CRYSIRROR_REALLIVE_INPUT").expect("local input"));
    let out = PathBuf::from(
        std::env::var_os("CRYSIRROR_REALLIVE_OUTPUT").expect("private capture directory"),
    );
    std::fs::create_dir_all(&out).unwrap();
    let state = tempfile::tempdir().unwrap();
    let p = crysirror_runtime::Project::load(&project).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_reallive::register(&mut registry).unwrap();
    let mut rt = Runtime::with_registry(p, &project, state.path(), registry).unwrap();
    let track = input_track::InputTrack::load(Some(&input)).unwrap();
    let gpu = Arc::new(pollster::block_on(Gpu::request(&wgpu::Instance::default(), None)).unwrap());
    eprintln!("GPU {:?}", gpu.adapter.get_info());
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    let start = Instant::now();
    let mut reached = false;
    let mut movie_frames = 0;
    let mut title = false;
    for frame in 0..100_000 {
        rt.tick(if frame == 0 { 0 } else { 16 }, track.at(frame))
            .unwrap();
        if frame == 1689 {
            capture(&mut rt, &mut renderer, &out.join("title.png"));
            title = true;
        }
        if let Some(media) = rt.host.media.status(ClientId(1), 0) {
            movie_frames = movie_frames.max(media.decoded_frames);
            if matches!(
                media.state,
                crysirror_runtime::media::MediaState::Loading
                    | crysirror_runtime::media::MediaState::Playing
            ) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if frame % 30 == 0 {
            let view = rt.inspect();
            let vm = &view["sessions"][0]["native"];
            if vm["scene"] == 6030
                && vm["line"] == 65
                && vm["wait"].as_str().is_some_and(|s| s.contains("Pause"))
            {
                reached = true;
                break;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(180),
            "opening checkpoint timeout: {}",
            rt.inspect()["sessions"]
        );
    }
    assert!(
        title && reached,
        "expected story checkpoint: {}",
        rt.inspect()["sessions"]
    );
    assert!(movie_frames > 2900, "full movie should decode");
    let client = ClientId(1);
    let id = 10008;
    let layer = rt
        .host
        .engine
        .layers
        .get(client, id)
        .expect("retained story glyphs");
    let paint = serde_json::to_value(&layer.paint).unwrap();
    assert_eq!(paint["kind"], "glyph_run");
    assert!(paint["glyphs"].as_array().unwrap().len() > 10);
    assert!(
        rt.host.engine.layers.get(client, 500000).is_none(),
        "movie layer retired at EOF"
    );
    std::fs::write(
        out.join("story-before-inspect.json"),
        serde_json::to_vec_pretty(&rt.inspect()).unwrap(),
    )
    .unwrap();
    let before = capture(&mut rt, &mut renderer, &out.join("story.png"));
    let native = rt.inspect()["sessions"][0]["native"].clone();
    // Leave native PC and clock untouched. A normal engine operation removes text.
    let ops = vec![
        json!({"op":"remove","id":id}),
        json!({"op":"remove","id":id+1}),
    ];
    rt.host
        .submit_engine(
            client,
            1,
            &serde_json::to_vec(&json!({"layers":ops})).unwrap(),
        )
        .unwrap();
    let after = capture(&mut rt, &mut renderer, &out.join("story-without-text.png"));
    assert_ne!(before, after, "engine edit must change actual GPU output");
    assert_eq!(
        native,
        rt.inspect()["sessions"][0]["native"],
        "no native redraw/execution required"
    );
    std::fs::write(
        out.join("inspect.json"),
        serde_json::to_vec_pretty(&rt.inspect()).unwrap(),
    )
    .unwrap();
    eprintln!(
        "native checkpoint + independent GPU edit passed; movie_frames={movie_frames}, elapsed={:?}",
        start.elapsed()
    );
}
fn capture(rt: &mut Runtime, renderer: &mut Renderer, path: &std::path::Path) -> Vec<u8> {
    renderer
        .set_engine_frame(rt.host.engine.frame(rt.project.canvas).unwrap())
        .unwrap();
    renderer
        .set_transition(rt.render_transition().unwrap())
        .unwrap();
    let image = renderer
        .capture(
            &rt.host.scene,
            &mut rt.assets,
            rt.project.canvas,
            rt.project.background,
        )
        .unwrap();
    image.save(path).unwrap();
    image.into_raw()
}
