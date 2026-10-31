// SPDX-License-Identifier: MPL-2.0
//! Installed-game bootstrap uses the same public engine operations as other
//! clients. It neither executes a launcher script nor extracts an archive.
use crate::{app, detect::Plan, input_track::InputTrack};
use anyhow::{bail, ensure, Context, Result};
use crysirror_runtime::Runtime;
use crysirror_wgpu::{Gpu, RenderingPath};
use std::{path::Path, sync::Arc};

pub fn bootstrap(rt: Runtime, plan: &Plan) -> Result<app::Core> {
    if plan.mounts.is_empty() {
        return Ok(app::Core::Plain(rt));
    }
    #[cfg(feature = "scripting")]
    {
        use crysirror_engine::Value;
        let mut engine = crysirror_engine::App::new(rt, "game")?;
        engine.init()?;
        let cx = engine.context();
        let resources = cx.singleton("Resources")?;
        for (name, profile) in &plan.profiles {
            cx.call(
                resources,
                "defineProfile",
                &[Value::Str(name.clone()), Value::from_json(profile.clone())],
            )?;
        }
        for mount in &plan.mounts {
            let ticket = cx
                .call(
                    resources,
                    "beginDecode",
                    &[
                        Value::Str(mount.source.clone()),
                        Value::Str("archive".into()),
                        Value::from_json(mount.options.clone()),
                    ],
                )
                .with_context(|| format!("index archive {}", mount.source))?;
            let started = std::time::Instant::now();
            let result = (|| -> Result<()> {
                loop {
                    let status = cx
                        .call(resources, "decodeStatus", &[ticket.clone()])?
                        .to_json();
                    match status["state"].as_str() {
                        Some("ready") => break,
                        Some("failed") => bail!("archive {}: {}", mount.source, status["error"]),
                        Some("pending") => {}
                        _ => bail!("invalid resource completion state: {status}"),
                    }
                    ensure!(
                        started.elapsed().as_secs() < 120,
                        "archive indexing timeout: {}",
                        mount.source
                    );
                    // Boot has not started the VM/event loop. CPU/index work
                    // belongs to the engine pool; this owner only polls and
                    // publishes its bounded completion. No decoder runs here.
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                cx.call(
                    resources,
                    "mountDecode",
                    &[ticket.clone(), Value::Str(mount.target.clone()),Value::Bool(mount.fallback)],
                )?;
                Ok(())
            })();
            let release = cx.call(resources, "cancelDecode", &[ticket]);
            result?;
            release?;
            log::info!("mounted {} at {}", mount.source, mount.target);
        }
        engine.start()?;
        Ok(app::Core::Scripted(Box::new(engine)))
    }
    #[cfg(not(feature = "scripting"))]
    bail!("this build lacks the engine Resources bootstrap; enable arc-formats")
}

fn runtime(core: &mut app::Core) -> Result<&mut Runtime> {
    match core {
        app::Core::Plain(rt) => Ok(rt),
        #[cfg(feature = "scripting")]
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
    for frame in 0..frames {
        let dt = if frame == 0 { 0 } else { 16 };
        match &mut core {
            app::Core::Plain(rt) => rt.tick(dt, input.at(frame))?,
            #[cfg(feature = "scripting")]
            app::Core::Scripted(engine) => engine.frame(dt, input.at(frame))?,
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
        #[cfg(feature = "scripting")]
        if let app::Core::Scripted(engine) = &core {
            if engine.quit_requested() {
                break;
            }
        }
    }
    #[cfg(feature = "scripting")]
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
        "runtime":rt.inspect(), "audio_emitted":false, "window_created":false,
        "menu_bar_presentation":if external_bar {"outside_capture"} else {"engine"},
        "os_menu_created":false})
    );
    Ok(())
}

#[cfg(all(test, feature = "arc-formats"))]
mod tests {
    use super::*;
    use crate::detect::ArchiveMount;
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
