// SPDX-License-Identifier: MPL-2.0
//! Host-independent player core: the adapter registry, project validation,
//! installed-game bootstrap and the renderer-side frame duties every host
//! (desktop window, offscreen, console) performs between ticks.
use crate::detect::Plan;
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    image_processing::RgbaImage,
    service::HostServices,
    state::ClientId,
    Error,
};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use crysirror_wgpu::Renderer;
use std::{io::Read, path::Path, sync::Arc, time::{Duration, Instant}};

/// What a host drives: a plain runtime, or the engine App with
/// behaviours/scripts (which owns the runtime).
pub enum Core {
    Plain(Runtime),
    Scripted(Box<crysirror_engine::App>),
}
impl Core {
    pub fn rt(&mut self) -> &mut Runtime {
        match self {
            Core::Plain(rt) => rt,
            Core::Scripted(app) => app
                .runtime_mut()
                .expect("engine context present between frames"),
        }
    }
}

pub fn adapter_registry() -> Result<AdapterRegistry> {
    #[allow(unused_mut)]
    let mut registry = AdapterRegistry::default();
    crysirror_siglus_integrated::register(&mut registry)?;
    crysirror_reallive::register(&mut registry)?;
    crysirror_krkr::register(&mut registry)?;
    crysirror_rfvp_integrated::register(&mut registry)?;
    crysirror_nscripter::register(&mut registry)?;
    crysirror_bgi_integrated::register(&mut registry)?;
    crysirror_softpal::register(&mut registry)?;
    crysirror_circus::register(&mut registry)?;
    crysirror_cmvs::adapter::register(&mut registry)?;
    crysirror_catsystem2::register(&mut registry)?;
    crysirror_majiro::adapter::register(&mut registry)?;
    crysirror_qlie::adapter::register(&mut registry)?;
    crysirror_minori::register(&mut registry)?;
    crysirror_sugarpot::register(&mut registry)?;
    crysirror_nsystem::register(&mut registry)?;
    crysirror_rscript::register(&mut registry)?;
    crysirror_n2system::register(&mut registry)?;
    crysirror_malie::register(&mut registry)?;
    crysirror_system_nnn::installed::register(&mut registry)?;
    crysirror_system4::factory::register(&mut registry)?;
    crysirror_avg32::register(&mut registry)?;
    crysirror_willplus::adapter::register(&mut registry)?;
    crysirror_cm3d2::register(&mut registry)?;
    crysirror_exhibit::register(&mut registry)?;
    crysirror_amanatsu::register(&mut registry)?;
    crysirror_naughty_dog::register(&mut registry)?;
    Ok(registry)
}
pub fn load_project(path: &Path) -> Result<Project> {
    let project = Project::load(path)?;
    for client in project.clients.iter().filter(|client|client.adapter=="system4") {
        serde_json::from_value::<crysirror_system4::worker_adapter::Config>(client.config.clone())?.validate()?;
    }
    for client in project.clients.iter().filter(|client|client.adapter=="system-nnn") {
        serde_json::from_value::<crysirror_system_nnn::installed::Config>(client.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "willplus") {
        serde_json::from_value::<crysirror_willplus::adapter::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "krkr") {
        serde_json::from_value::<crysirror_krkr::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "rfvp") {
        serde_json::from_value::<crysirror_rfvp_integrated::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "reallive") {
        serde_json::from_value::<crysirror_reallive::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "nscripter") {
        serde_json::from_value::<crysirror_nscripter::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "circus") {
        crysirror_circus::Config::from_execution_value(c.config.clone())?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "catsystem2") {
        serde_json::from_value::<crysirror_catsystem2::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "majiro") {
        serde_json::from_value::<crysirror_majiro::adapter::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "cm3d2") {
        serde_json::from_value::<crysirror_cm3d2::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "amanatsu") {
        serde_json::from_value::<crysirror_amanatsu::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == crysirror_naughty_dog::ENGINE_ID) {
        serde_json::from_value::<crysirror_naughty_dog::Config>(c.config.clone())?.validate()?;
    }
    for c in project.clients.iter().filter(|c| c.adapter == "avg32") {
        serde_json::from_value::<crysirror_avg32::Config>(c.config.clone())?.validate()?;
    }
    Ok(project)
}
pub fn bootstrap(rt: Runtime, plan: &Plan) -> Result<Core> {
    bootstrap_interruptible(rt, plan, || false)
}
pub fn bootstrap_interruptible(mut rt: Runtime, plan: &Plan, cancelled: impl Fn() -> bool) -> Result<Core> {
    ensure!(!cancelled(), "native launch cancelled");
    // Installed-game launch presents the selected scoped directory as virtual
    // media. The executable stem is its compatibility label, not an assertion
    // about an OS optical drive or the source disc's physical volume label.
    // Native scripts may keep using searchCD and the returned D:/ paths.
    if plan.engine == "KiriKiri" {
        for (index, client) in rt.project.clients.iter().enumerate() {
            if client.adapter != "krkr" { continue; }
            if let Some(exe)=client.config.get("executable").and_then(|v|v.as_str()) {
                let label=exe.rsplit_once('.').map_or(exe, |(stem,_)|stem);
                let root=client.config.get("root").and_then(|v|v.as_str()).unwrap_or("");
                rt.host.publish_mounted_media(crysirror_core::state::ClientId((index+1) as u32),
                    vec![crysirror_core::service::MountedMedia{label:label.into(),drive:'D',root:root.into()}])
                    .map_err(|e|anyhow::anyhow!("installed media: {e}"))?;
            }
        }
    }
    if plan.mounts.is_empty() {
        return Ok(Core::Plain(rt));
    }
    {
        use crysirror_engine::Value;
        let mut sources = std::collections::BTreeMap::<String, String>::new();
        for mount in &plan.mounts {
            if mount.options["format"] == "kiss/warc" && mount.source.to_ascii_lowercase().ends_with(".bz2") {
                #[cfg(target_arch = "wasm32")]
                bail!("bzip2 archive preparation requires a disk cache");
                #[cfg(not(target_arch = "wasm32"))]
                sources.insert(mount.source.clone(),
                    crate::compressed_archive::mount(&mut rt, &mount.source, &cancelled)?);
            }
        }
        let mut engine = crysirror_engine::App::new(rt, "game")?;
        engine.init()?;
        let cx = engine.context();
        if plan.engine=="N2System" {
            use crysirror_core::service::HostServices;
            let state=cx.state()?;
            let client=crysirror_core::state::ClientId(1);
            let config:crysirror_n2system::Config=serde_json::from_value(state.runtime.project.clients[0].config.clone())?;
            if let Some(material)=config.archive_material {
                state.resources.keys=material.provider(client,state.runtime.host.storage(client)?)?;
            }
        }
        if plan.engine=="Malie" {
            use crysirror_core::{service::HostServices,path::{Mount,VfsPath}};
            let state=cx.state()?;
            let client=crysirror_core::state::ClientId(1);
            let config:crysirror_malie::adapter::Config=serde_json::from_value(state.runtime.project.clients[0].config.clone())?;
            let storage=state.runtime.host.storage(client)?;
            let source=storage.open_read(&VfsPath::new(Mount::Asset,&config.executable)?)?;
            let material=crysirror_malie::material::archive_key(&crysirror_resource::read_all(source.as_ref(),16*1024*1024)?)?;
            let keys=std::sync::Arc::new(crysirror_resource::MemoryKeyProvider::default());
            keys.insert(client,"malie-game",material)?;
            state.resources.keys=keys;
        }
        let resources = cx.singleton("Resources")?;
        for (name, profile) in &plan.profiles {
            cx.call(
                resources,
                "defineProfile",
                &[Value::Str(name.clone()), Value::from_json(profile.clone())],
            )?;
        }
        for mount in &plan.mounts {
            ensure!(!cancelled(), "native launch cancelled");
            let ticket = cx
                .call(
                    resources,
                    "beginDecode",
                    &[
                        Value::Str(sources.get(&mount.source).unwrap_or(&mount.source).clone()),
                        Value::Str("archive".into()),
                        Value::from_json(mount.options.clone()),
                    ],
                )
                .with_context(|| format!("index archive {}", mount.source))?;
            let started = std::time::Instant::now();
            let result = (|| -> Result<bool> {
                loop {
                    ensure!(!cancelled(), "native launch cancelled");
                    let status = cx
                        .call(resources, "decodeStatus", &[ticket.clone()])?
                        .to_json();
                    match status["state"].as_str() {
                        Some("ready") => break,
                        Some("failed") if mount.is_hxv4_companion() => {
                            log::warn!("cannot index optional archive {}: {}", mount.source, status["error"]);
                            return Ok(false);
                        }
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
                Ok(true)
            })();
            let release = cx.call(resources, "cancelDecode", &[ticket]);
            let mounted = result?;
            release?;
            if mounted {
                log::info!("mounted {} at {}", mount.source, mount.target);
            }
        }
        ensure!(!cancelled(), "native launch cancelled");
        engine.start()?;
        Ok(Core::Scripted(Box::new(engine)))
    }
}

pub fn load_fonts(rt: &mut Runtime, override_font: Option<&Path>) -> Result<()> {
    for (alias, path) in &rt.project.fonts {
        let bytes = rt.assets.read(path)?;
        rt.assets.text.add_font(alias, bytes.to_vec())?;
        rt.host.text.add_font(alias, bytes.to_vec())?;
    }
    if let Some(path) = override_font {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .with_context(|| format!("font {}", path.display()))?
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 32 * 1024 * 1024, "font exceeds 32 MiB");
        rt.host.text.add_font("default", bytes.clone())?;
        rt.assets.text.add_font("default", bytes)?;
    }
    Ok(())
}
/// Compacted canvases (scene compaction requests) are rasterized once by the
/// renderer and republished as immutable engine PNGs, so their frozen
/// snapshot trees stop nesting. A request whose images are still loading is
/// retried next frame.
pub fn materialize_canvases(renderer: &mut Renderer, rt: &mut Runtime) -> Result<()> {
    let profiler = rt.profiler.clone();
    let frame = profiler.frame();
    let _span = profiler.span(frame, "canvas.materialize");
    if rt.has_pending_target_images() {
        let _span = profiler.span(frame, "canvas.target_readback");
        let canvas = rt.project.canvas;
        renderer.set_engine_frame(rt.host.engine.frame(canvas)?)?;
        renderer.flush_target_writes(&rt.host.scene,&mut rt.assets,canvas,rt.project.background)?;
        rt.prepare_target_images(|target,assets|renderer.render_target_readback(assets,target))?;
    }
    rt.prepare_frame_captures(|rt| {
        let _span = profiler.span(frame, "canvas.frame_capture");
        let canvas = rt.project.canvas;
        renderer.set_engine_frame(rt.host.engine.frame(canvas)?)?;
        let image = renderer.capture(&rt.host.scene, &mut rt.assets, canvas, rt.project.background)?;
        Ok(image.into_raw())
    })?;
    rt.prepare_canvas_images(|request,assets| {
        let _span = profiler.span(frame, "canvas.region_readback");
        renderer.snapshot_readback(assets,request.client,request.size,&request.layers)
    })?;
    for request in rt.host.engine.canvases.live_materialize_requests() {
        let _span = profiler.span(frame, "canvas.compaction");
        let image = profiler.scope(frame, || {
            renderer.materialize_snapshot(&mut rt.assets, request.client, request.size, &request.layers)
        })?;
        let Some(image) = image
        else {
            continue;
        };
        let _publish = profiler.span(frame, "canvas.compaction.publish");
        let client = ClientId(request.client);
        // A parked bitmap redraw often overwrites these pixels within a few
        // ticks. Keep its compacted surface as a bounded resident image rather
        // than encoding PNG on the presentation thread and decoding it again.
        if rt.presentation_pending() {
            let bytes = Arc::from(image.as_raw().as_slice());
            let resource = Arc::new(RgbaImage::new(client, request.size, bytes)?);
            match rt.host.publish_image_resource(client, request.surface, resource) {
                Ok(image) => {
                    let path = image.path.relative();
                    let engine = &mut rt.host.engine;
                    let resolved = engine.canvases.resolve_materialized(&request, path, &mut engine.layers);
                    let retired = rt.host.release_image_resource(client, path);
                    resolved?;
                    retired?;
                    continue;
                }
                // Keep the existing storage path when the resident-image
                // budget is full. Both paths preserve the same frozen pixels.
                Err(Error::Limit(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut png = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        // FNV-1a over the pixels names the immutable image.
        let hash = image.as_raw().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        let path = format!(
            "__crysirror/canvas-{}-{}x{}-{hash:016x}.png",
            request.surface, request.size[0], request.size[1]
        );
        rt.assets.register_generated(client, &path, png)?;
        let engine = &mut rt.host.engine;
        engine.canvases.resolve_materialized(&request, &path, &mut engine.layers)?;
        log::debug!("canvas {} materialized as {path}", request.surface);
    }
    {
        let _span = profiler.span(frame, "layers.transition_history");
        renderer.advance_layer_histories(&mut rt.assets, &rt.host.engine.layers, rt.project.canvas)?;
    }
    Ok(())
}

/// Offline startup has an explicit dependency fence. Counting simulated frames
/// while a cold resource worker is still preparing a VM can otherwise produce
/// a successful empty capture before any original instruction has run.
pub fn settle_vm_preparations(rt:&mut Runtime)->Result<()> {
    let started=Instant::now();
    while rt.poll_vm_preparations()? {
        ensure!(started.elapsed()<Duration::from_secs(120),"native startup preparation timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
