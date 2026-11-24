// SPDX-License-Identifier: MPL-2.0
//! Installed-game bootstrap uses the same public engine operations as other
//! clients. It neither executes a launcher script nor extracts an archive.
use crate::{app, input_track::InputTrack};
pub use crysirror_launch::core::{bootstrap, bootstrap_interruptible};
use anyhow::{bail, ensure, Context, Result};
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, RenderingPath};
use std::{path::Path, sync::Arc};

fn runtime(core: &mut app::Core) -> Result<&mut Runtime> {
    match core {
        app::Core::Plain(rt) => Ok(rt),
        app::Core::Scripted(engine) => engine.runtime_mut(),
    }
}

/// Exercise the detected launch plan with no window or output device. Canvas
/// compaction is materialized between ticks, exactly as in the desktop host.
pub fn offscreen(
    mut core: app::Core,
    frames: u32,
    output: &Path,
    track: Option<&Path>,
    font: Option<&Path>,
) -> Result<()> {
    let input = InputTrack::load(track)?;
    // A replay supplies committed text to the same capture sessions as the
    // window presenter. No OS IME or preedit events are synthesized.
    if track.is_some() {
        let text = &mut runtime(&mut core)?.host.text_input;
        text.set_presenter_available(true);
        text.focus(true);
    }
    // Optional wall pacing gives asynchronous decoder/resource workers time
    // during reproducible offline runs. It never changes native elapsed time,
    // input frames or decoder EOF/completion receipts.
    let wall_pacing_ms = std::env::var("CRYSIRROR_OFFSCREEN_PACING_MS").ok()
        .map(|s| s.parse::<u64>()).transpose()?.unwrap_or(0);
    ensure!(wall_pacing_ms <= 16, "offscreen pacing must be 0..=16ms");
    let mut virtual_window = crate::virtual_window::VirtualWindow::new(runtime(&mut core)?.project.canvas);
    virtual_window.publish(runtime(&mut core)?);
    // Native text callbacks may query metrics before the first presented frame.
    app::load_fonts(runtime(&mut core)?,font)?;
    // Match the desktop canvas: on OS-bar platforms the menu lives outside
    // this render target. Keep its model/events, but do not replace an absent
    // OS bar with a different engine-drawn bar in the captured game picture.
    let external_bar = cfg!(any(target_os = "macos", target_os = "windows"))
        && std::env::var_os("CRYSIRROR_ENGINE_MENU").as_deref() != Some(std::ffi::OsStr::new("1"));
    runtime(&mut core)?
        .host
        .engine
        .ui
        .menus
        .set_native_bars(external_bar);
    let instance = wgpu::Instance::default();
    let gpu = Arc::new(pollster::block_on(Gpu::request_for(
        &instance,
        None,
        RenderingPath::Raster,
    ))?);
    let adapter = format!("{:?}", gpu.adapter.get_info());
    let mut materializer = crysirror_wgpu::Renderer::new(
        gpu.clone(),
        if std::env::var_os("CRYSIRROR_CAPTURE_BGRA").is_some() {
            wgpu::TextureFormat::Bgra8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        },
    );
    if std::env::var_os("CRYSIRROR_CPU_FRAME_HISTORY").is_none() {
        runtime(&mut core)?.host.mesh_frame_history =
            Some(materializer.mesh_frame_history_provider());
    }
    runtime(&mut core)?.set_canvas_image_presenter(true);
    app::settle_vm_preparations(runtime(&mut core)?)?;
    // Developer diagnostic: also write `<output>-<frame>.png` every N frames.
    let capture_every = std::env::var("CRYSIRROR_HEADLESS_CAPTURE_EVERY")
        .ok()
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| *n > 0);
    let close_frame = std::env::var("CRYSIRROR_HEADLESS_CLOSE_FRAME")
        .ok()
        .and_then(|frame| frame.parse::<u32>().ok());
    for frame in 0..frames {
        let dt = if frame == 0 { 0 } else { 16 };
        let frame_input = input.at(frame);
        let rt = runtime(&mut core)?;
        virtual_window.begin_tick(rt);
        if let Some(text) = input.text_at(frame) {
            if rt.host.engine.ui.focused_text_field() {
                let focus = rt.host.engine.ui.focus.context("missing focused text field")?;
                crysirror_core::service::HostServices::submit_engine(
                    &mut rt.host, crysirror_core::state::ClientId(focus.owner), 1,
                    &serde_json::to_vec(&serde_json::json!({"ui":[{"op":"insert_text","text":text}]}))?,
                )?;
            } else {
                ensure!(rt.host.text_input.commit(text)?, "input track text has no active recipient");
            }
        }
        // The offline host explicitly supplies a virtual pointer, including
        // its initial origin. This is a backend policy, never a VM fallback
        // from missing native/window observations.
        if rt.host.pointer_activity.is_none()
            || rt.host.pointer_observation.position != Some([frame_input.pointer_x, frame_input.pointer_y]) {
            rt.host.note_pointer_activity()?;
        }
        rt.host.pointer_observation = crysirror_core::service::PointerObservation {
            position: Some([frame_input.pointer_x, frame_input.pointer_y]),
            inside: Some(frame_input.pointer_x >= 0 && frame_input.pointer_y >= 0
                && frame_input.pointer_x < rt.project.canvas[0] as i32
                && frame_input.pointer_y < rt.project.canvas[1] as i32),
        };
        rt.host.publish_input_sources(crysirror_core::service::InputSourcesFrame{keyboard_pointer:frame_input,controller_backend:false,controllers:Vec::new()})?;
        // Developer diagnostic: the window close button, without a window.
        if close_frame == Some(frame) {
            if let app::Core::Plain(rt) = &mut core {
                if !rt.request_close() {
                    break;
                }
            }
        }
        let completion_deadline = std::time::Instant::now() + std::time::Duration::from_millis(16);
        let step = crysirror_tasks::with_completion_deadline(completion_deadline, || match &mut core {
            app::Core::Plain(rt) => rt.tick(dt, frame_input),
            app::Core::Scripted(engine) => engine.frame(dt, frame_input),
        });
        if let Err(error) = step {
            // Preserve the real VM frontier once, before propagating its
            // failure. No image is presented as a completed capture here.
            println!("{}", serde_json::json!({
                "output":output,"gpu":adapter,"error":format!("{error:#}"),
                "failed_frame":frame,"frames_completed":frame,"requested_frames":frames,
                "runtime":runtime(&mut core)?.inspect(),"capture_written":false,
                "audio_emitted":false,"window_created":false,"virtual_window":true,"wall_pacing_ms":wall_pacing_ms,
                "os_menu_created":false
            }));
            return Err(error);
        }
        // Restoring native/framework state must not restore the host's
        // presentation choice. The window host also reapplies this per frame.
        runtime(&mut core)?
            .host
            .engine
            .ui
            .menus
            .set_native_bars(external_bar);
        app::materialize_canvases(&mut materializer, runtime(&mut core)?)?;
        if let Some(every) = capture_every.filter(|n| frame > 0 && frame % n == 0) {
            let _ = every;
            if let app::Core::Scripted(engine) = &mut core {
                engine.prepare_render_offline()?;
            }
            let stem = output.with_extension("");
            let path = std::path::PathBuf::from(format!("{}-{frame:06}.png", stem.display()));
            app::capture_existing(&mut materializer, runtime(&mut core)?, font, &path)?;
        }
        if wall_pacing_ms != 0 {
            std::thread::sleep(std::time::Duration::from_millis(wall_pacing_ms));
        }
        if let app::Core::Scripted(engine) = &core {
            if engine.quit_requested() {
                break;
            }
        }
    }
    if let app::Core::Scripted(engine) = &mut core {
        engine.prepare_render_offline()?;
    }
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let rt = runtime(&mut core)?;
    let stats = app::capture_existing(&mut materializer, rt, font, output)?;
    println!(
        "{}",
        serde_json::json!({"output":output, "gpu":adapter, "stats":stats,
        "runtime":rt.inspect(), "audio_emitted":false, "window_created":false, "virtual_window":true, "wall_pacing_ms":wall_pacing_ms,
        "menu_bar_presentation":if external_bar {"outside_capture"} else {"engine"},
        "os_menu_created":false})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{ArchiveMount, Plan};
    use crysirror_core::{
        path::{Mount, VfsPath},
        service::FileSystem,
    };

    #[test]
    fn typed_bootstrap_indexes_on_worker_and_publishes_without_unpacking() {
        let dir = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let member = b"hello.txt\0";
        let body = b"shared resource";
        let mut sar = Vec::new();
        sar.extend(1u16.to_be_bytes());
        sar.extend((6u32 + member.len() as u32 + 8).to_be_bytes());
        sar.extend(member);
        sar.extend(0u32.to_be_bytes());
        sar.extend((body.len() as u32).to_be_bytes());
        sar.extend(body);
        std::fs::write(dir.path().join("arc.sar"), sar).unwrap();
        let project = serde_json::json!({"schema":2,"name":"authored bootstrap","content_revision":"1",
            "assets":"game","canvas":[64,64],"clients":[{"name":"game","game":"authored",
            "adapter":"crysirror-script-v1","program":[{"op":"halt"}]}]});
        let plan = Plan {
            engine: "authored",
            project: project.clone(),
            profiles: Default::default(),
            evidence: Vec::new(),
            mounts: vec![ArchiveMount {
                fallback:false,
                source: "arc.sar".into(),
                target: "__nscript/arc.sar".into(),
                options: serde_json::json!({"format":"nscripter/sar"}),
            }],
        };
        let rt = Runtime::with_asset_root(
            serde_json::from_value(project).unwrap(),
            dir.path(),
            saves.path(),
            Default::default(),
        )
        .unwrap();
        let mut core = bootstrap(rt, &plan).unwrap();
        let rt = runtime(&mut core).unwrap();
        let id = rt.client_id("game").unwrap();
        let path = VfsPath::new(Mount::Asset, "__nscript/arc.sar/hello.txt").unwrap();
        assert_eq!(rt.host.read(id, &path).unwrap(), body);
        assert!(!dir.path().join("hello.txt").exists());
        assert!(!dir.path().join("play-mount.ts").exists());
        // Mount publication happens before the first native instruction.
        assert_eq!(rt.inspect()["frame"], serde_json::json!(0));
        assert_eq!(rt.inspect()["time_ms"], serde_json::json!(0));
    }
}
