// SPDX-License-Identifier: MPL-2.0
use crate::audio::Audio;
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::{buttons, InputFrame};
use crysirror_runtime::{vfs, Runtime};
use crysirror_wgpu::{geometry::Viewport, Gpu, RenderTarget, Renderer, RenderingPath};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

/// What the window loop drives: a plain runtime, or the engine App with
/// behaviours/scripts (which owns the runtime).
pub enum Core {
    Plain(Runtime),
    #[cfg(feature = "scripting")]
    Scripted(Box<crysirror_engine::App>),
}
impl Core {
    pub fn rt(&mut self) -> &mut Runtime {
        match self {
            Core::Plain(rt) => rt,
            #[cfg(feature = "scripting")]
            Core::Scripted(app) => app
                .runtime_mut()
                .expect("engine context present between frames"),
        }
    }
    fn tick(&mut self, ms: u64, input: InputFrame) -> Result<bool> {
        match self {
            Core::Plain(rt) => {
                rt.tick(ms, input)?;
                Ok(false)
            }
            #[cfg(feature = "scripting")]
            Core::Scripted(app) => {
                app.frame(ms, input)?;
                Ok(app.quit_requested())
            }
        }
    }
}

pub struct RunOptions {
    pub font: Option<PathBuf>,
    pub muted: bool,
    pub frames: Option<u64>,
    pub capture: Option<PathBuf>,
    pub profile_path: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub rendering_path: RenderingPath,
}
struct WindowState {
    pending_snapshot: Option<crysirror_runtime::frame::PendingFrame>,
    render_snapshot: Option<crysirror_runtime::frame::RenderSnapshot>,
    snapshot_store: u64,
    snapshot_restored: u64,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    renderer: Renderer,
    config: wgpu::SurfaceConfiguration,
    minimized: bool,
}
pub struct App {
    core: Core,
    options: RunOptions,
    instance: wgpu::Instance,
    window: Option<WindowState>,
    audio: Audio,
    #[cfg(feature = "gamepad")]
    gamepad: Option<crate::gamepad::Backend>,
    input: InputFrame,
    focused: bool,
    ime_composing: bool,
    last_tick: Instant,
    remainder: Duration,
    presented: u64,
    error: Option<anyhow::Error>,
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
/// What a movie advances between frames: the bare runtime, or an engine
/// App whose scripts keep running.
pub trait MovieSource {
    fn advance(&mut self, step_ms: u64) -> Result<()>;
    fn prepare_capture(&mut self) -> Result<()> {
        Ok(())
    }
    fn runtime(&mut self) -> &mut Runtime;
}
impl MovieSource for Runtime {
    fn prepare_capture(&mut self) -> Result<()> {
        self.finish_vfx_offline()
    }
    fn advance(&mut self, step_ms: u64) -> Result<()> {
        self.tick(step_ms, crysirror_core::service::InputFrame::default())
    }
    fn runtime(&mut self) -> &mut Runtime {
        self
    }
}

/// Renders `frames` consecutive frames with one renderer (caches, TAA and
/// probe state carry over like a running game), advancing the source by
/// `step_ms` between them. `output` receives the last frame.
#[allow(clippy::too_many_arguments)]
pub fn movie_on(
    gpu: Arc<Gpu>,
    source: &mut dyn MovieSource,
    font: Option<&Path>,
    output: &Path,
    dir: &Path,
    frames: u32,
    step_ms: u64,
    rendering_path: RenderingPath,
) -> Result<serde_json::Value> {
    std::fs::create_dir_all(dir)?;
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_rendering_path(rendering_path)?;
    load_fonts(source.runtime(), font)?;
    let mut last = None;
    let mut frame_samples = std::collections::VecDeque::new();
    for i in 0..frames {
        if i > 0 {
            source.advance(step_ms)?;
        }
        source.prepare_capture()?;
        let rt = source.runtime();
        let snapshot = rt.schedule_render_snapshot()?.wait(rt)?;
        renderer.set_profiler(rt.profiler.clone(), snapshot.frame);
        renderer.set_scene3d(snapshot.simple.as_ref())?;
        renderer.set_pbr_scene(snapshot.pbr)?;
        renderer.set_engine_frame(snapshot.layers)?;
        renderer.set_transition(snapshot.transition)?;
        let capture_start = Instant::now();
        let image = rt.assets.with_frame_surfaces(snapshot.surfaces, |assets| {
            renderer.capture(
                &snapshot.scene,
                assets,
                rt.project.canvas,
                rt.project.background,
            )
        })?;
        let capture_ms = capture_start.elapsed().as_secs_f64() * 1000.;
        if frame_samples.len() == 256 {
            frame_samples.pop_front();
        }
        frame_samples.push_back(serde_json::json!({"frame":i,"time_ms":rt.now_ms(),"capture_wall_ms":capture_ms,"world_entities":rt.host.engine.world.len(),"cpu_asset_bytes":rt.assets.tasks_inspect()["resident_bytes"],"rt":renderer.hardware_rt_stats(),"vfx":renderer.pbr().map(|p|p.vfx_stats())}));
        image
            .save_with_format(
                dir.join(format!("frame_{i:05}.png")),
                image::ImageFormat::Png,
            )
            .context("save movie frame")?;
        last = Some(image);
    }
    if let Some(image) = last {
        image.save_with_format(output, image::ImageFormat::Png)?;
    }
    Ok(
        serde_json::json!({"movie_frames": frames, "movie_dir": dir, "step_ms": step_ms,
        "hardware_rt_report":renderer.gpu.hardware_rt_report(),
        "hardware_rt_stats":renderer.hardware_rt_stats(),"vfx":renderer.pbr().map(|p|p.vfx_stats()),"frame_samples":frame_samples,"frame_samples_dropped":frames.saturating_sub(256),"profile":renderer.profile_report_offline()?}),
    )
}
pub fn capture_on(
    gpu: Arc<Gpu>,
    rt: &mut Runtime,
    font: Option<&Path>,
    path: &Path,
    rendering_path: RenderingPath,
) -> Result<serde_json::Value> {
    let previous = rt.assets.is_nonblocking();
    rt.assets.set_nonblocking(false);
    let result = capture_ready(gpu, rt, font, path, rendering_path);
    rt.assets.set_nonblocking(previous);
    result
}
fn capture_ready(
    gpu: Arc<Gpu>,
    rt: &mut Runtime,
    font: Option<&Path>,
    path: &Path,
    rendering_path: RenderingPath,
) -> Result<serde_json::Value> {
    let mut renderer = Renderer::new(gpu, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.set_rendering_path(rendering_path)?;
    capture_ready_with_renderer(&mut renderer, rt, font, path)
}
/// Explicit capture fence on the live renderer, retaining GPU simulation state.
fn capture_existing(
    renderer: &mut Renderer,
    rt: &mut Runtime,
    font: Option<&Path>,
    path: &Path,
) -> Result<serde_json::Value> {
    let previous = rt.assets.is_nonblocking();
    rt.assets.set_nonblocking(false);
    let result = capture_ready_with_renderer(renderer, rt, font, path);
    rt.assets.set_nonblocking(previous);
    result
}
fn capture_ready_with_renderer(
    renderer: &mut Renderer,
    rt: &mut Runtime,
    font: Option<&Path>,
    path: &Path,
) -> Result<serde_json::Value> {
    rt.finish_vfx_offline()?;
    let snapshot = rt.schedule_render_snapshot()?.wait(rt)?;
    renderer.set_profiler(rt.profiler.clone(), snapshot.frame);
    renderer.set_scene3d(snapshot.simple.as_ref())?;
    renderer.set_pbr_scene(snapshot.pbr)?;
    renderer.set_engine_frame(snapshot.layers)?;
    renderer.set_transition(snapshot.transition)?;
    load_fonts(rt, font)?;
    let image = rt.assets.with_frame_surfaces(snapshot.surfaces, |assets| {
        renderer.capture(
            &snapshot.scene,
            assets,
            rt.project.canvas,
            rt.project.background,
        )
    })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    image
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("save {}", path.display()))?;
    // Resource reports and encoded-work counters accompany the captured frame;
    // capability advertisement alone is not evidence that rays were dispatched.
    let mut stats = match renderer.pbr() {
        Some(p) => {
            let (l, r, v, g) = (p.lod_stats(), p.probe_stats(), p.vsm_stats(), p.gi_stats());
            serde_json::json!({
                "geometry_lod": {"primitives": l.lod_primitives, "source_triangles": l.source_triangles,
                    "drawn_triangles": l.drawn_triangles, "clusters_drawn": l.clusters_drawn},
                "reflection_probes": {"submitted": r.submitted, "active": r.active},
                "virtual_shadows": {"requested": v.requested, "resident": v.resident, "rendered": v.rendered, "dropped": v.dropped, "invalidations": v.invalidations},
                "irradiance_volume": {"probes": g.probes, "refreshed": g.refreshed, "total_refreshed": g.total_refreshed, "sweeps": g.sweeps, "converged": g.converged},
                "vfx": p.vfx_stats(),
            })
        }
        None => serde_json::json!({}),
    };
    stats["profile"] = renderer.profile_report_offline()?;
    stats["hardware_rt_report"] = serde_json::to_value(renderer.gpu.hardware_rt_report())?;
    stats["hardware_rt_stats"] = renderer
        .hardware_rt_stats()
        .unwrap_or(serde_json::Value::Null);
    Ok(stats)
}
impl App {
    pub fn run(core: Core, options: RunOptions) -> Result<()> {
        let audio = Audio::new(options.muted);
        #[cfg(feature = "gamepad")]
        let gamepad = match crate::gamepad::Backend::new() {
            Ok(backend) => Some(backend),
            Err(error) => {
                log::warn!("gamepad backend unavailable: {error:#}; keyboard/mouse remain active");
                None
            }
        };
        let event_loop = EventLoop::new()?;
        let mut app = Self {
            core,
            options,
            instance: wgpu::Instance::default(),
            window: None,
            audio,
            #[cfg(feature = "gamepad")]
            gamepad,
            input: InputFrame::default(),
            focused: true,
            ime_composing: false,
            last_tick: Instant::now(),
            remainder: Duration::ZERO,
            presented: 0,
            error: None,
        };
        event_loop.run_app(&mut app)?;
        if let Some(path) = &app.options.profile_path {
            let mut report = match &mut app.window {
                Some(w) => {
                    let mut r = w.renderer.profile_report_offline()?;
                    r["hardware_rt_stats"] = w
                        .renderer
                        .hardware_rt_stats()
                        .unwrap_or(serde_json::Value::Null);
                    r
                }
                None => serde_json::json!({"timings":app.core.rt().profiler.snapshot()}),
            };
            let mut trace = report
                .as_object_mut()
                .and_then(|m| m.remove("cpu_trace"))
                .unwrap_or_else(|| app.core.rt().profiler.chrome_trace());
            trace["engine_profile"] = report;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(path, serde_json::to_vec(&trace)?)?;
        }
        match app.error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    fn committed_text(&mut self, text: &str) -> Result<()> {
        let rt = self.core.rt();
        if let Some(id) = rt.host.engine.ui.focus {
            if rt.host.engine.ui.focused_text_field() {
                crysirror_core::service::HostServices::submit_engine(
                    &mut rt.host,
                    crysirror_core::state::ClientId(id.owner),
                    1,
                    &serde_json::to_vec(
                        &serde_json::json!({"ui":[{"op":"insert_text","text":text}]}),
                    )?,
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        }
        Ok(())
    }
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        if self.window.is_some() {
            return Ok(());
        }
        // Native adapters retain their first-use resource/wait contract until
        // they explicitly integrate asynchronous readiness fences.
        let progressive = self
            .core
            .rt()
            .project
            .clients
            .iter()
            .all(|client| client.adapter == "crysirror-script-v1");
        self.core.rt().assets.set_nonblocking(progressive);
        let canvas = self.core.rt().project.canvas;
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title(format!("CrySirror | {}", self.core.rt().project.name))
                    .with_inner_size(LogicalSize::new(canvas[0] as f64, canvas[1] as f64)),
            )?,
        );
        window.set_ime_allowed(true);
        let surface = self.instance.create_surface(window.clone())?;
        let gpu = Arc::new(pollster::block_on(Gpu::request_for(
            &self.instance,
            Some(&surface),
            self.options.rendering_path,
        ))?);
        log::info!("GPU: {:?}", gpu.adapter.get_info());
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .context("surface has no sRGB format")?;
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: *caps.alpha_modes.first().context("surface alpha modes")?,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &config);
        let mut renderer = Renderer::new(gpu, format);
        renderer.set_scene3d(self.core.rt().render_scene3d()?.as_ref())?;
        renderer.set_pbr_scene(self.core.rt().render_pbr()?)?;
        renderer.set_rendering_path(self.options.rendering_path)?;
        load_fonts(self.core.rt(), self.options.font.as_deref())?;
        self.window = Some(WindowState {
            pending_snapshot: None,
            render_snapshot: None,
            snapshot_store: 0,
            snapshot_restored: 0,
            surface,
            window,
            renderer,
            config,
            minimized: false,
        });
        self.last_tick = Instant::now();
        self.remainder = Duration::ZERO;
        self.audio.resume();
        Ok(())
    }
    fn fail(&mut self, el: &ActiveEventLoop, error: anyhow::Error) {
        log::error!("{error:#}");
        self.error = Some(error);
        el.exit();
    }
    fn save(&mut self) -> Result<()> {
        let bytes = self.core.rt().snapshot()?;
        std::fs::create_dir_all(&self.options.state_dir)?;
        // App-selected state root. VM writes still go through per-client capabilities.
        let dir = cap_dir(&self.options.state_dir)?;
        vfs::write_atomic(&dir, "quick.save", &bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
        log::info!(
            "saved {}",
            self.options.state_dir.join("quick.save").display()
        );
        Ok(())
    }
    fn load(&mut self) -> Result<()> {
        let dir = cap_dir(&self.options.state_dir)?;
        let bytes = vfs::read_limited(&dir, "quick.save").map_err(|e| anyhow::anyhow!("{e}"))?;
        self.core.rt().restore(&bytes)?;
        self.input.buttons_pressed = 0;
        self.last_tick = Instant::now();
        self.remainder = Duration::ZERO;
        log::info!("restored quick.save; audio restarts at sample zero");
        Ok(())
    }
    fn button(&mut self, bit: u64, down: bool) {
        if down {
            if self.input.buttons_down & bit == 0 {
                self.input.buttons_pressed |= bit;
            }
            self.input.buttons_down |= bit;
        } else {
            self.input.buttons_down &= !bit;
        }
    }
    fn redraw(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let Some(w) = self.window.as_mut() else {
            return Ok(());
        };
        if w.minimized {
            return Ok(());
        }
        let now = Instant::now();
        let delta = now
            .duration_since(self.last_tick)
            .min(Duration::from_millis(250))
            + self.remainder;
        self.last_tick = now;
        let ms = if self.options.frames.is_some() {
            16
        } else {
            delta.as_millis() as u64
        };
        self.remainder = if self.options.frames.is_some() {
            Duration::ZERO
        } else {
            delta - Duration::from_millis(ms)
        };
        #[allow(unused_mut)]
        let mut frame_input = self.input;
        #[cfg(feature = "gamepad")]
        if let Some(gamepad) = &mut self.gamepad {
            let events = gamepad.poll()?;
            if let Core::Scripted(app) = &mut self.core {
                for event in events {
                    if self.focused
                        || matches!(
                            event,
                            crysirror_engine::input_api::GamepadEvent::Connected { .. }
                                | crysirror_engine::input_api::GamepadEvent::Disconnected { .. }
                        )
                    {
                        app.context().state()?.actions.inject_gamepad(event)?;
                    }
                }
            }
            if self.focused {
                frame_input = gamepad.navigation_frame(frame_input);
            } else {
                gamepad.reset_navigation();
            }
        }
        if !self.focused {
            frame_input = InputFrame::default();
        }
        if self
            .core
            .tick(if self.presented == 0 { 0 } else { ms }, frame_input)?
        {
            el.exit();
        }
        if let Some((_, visible)) = self.core.rt().host.engine.ui.cursor {
            w.window.set_cursor_visible(visible);
        }
        if let Some((_, title)) = &self.core.rt().host.engine.ui.title {
            w.window.set_title(title);
        }
        self.input.buttons_pressed = 0;
        match &mut self.core {
            #[cfg(feature = "scripting")]
            Core::Scripted(app) => {
                let frames = app.context().state()?.mixer_audio.take_output();
                let voices = !app.runtime()?.host.scene.voices().is_empty();
                let restored = app.runtime()?.restored;
                self.audio.sync_mixed(&frames, voices, restored)?;
            }
            Core::Plain(runtime) => self.audio.sync(runtime)?,
        }
        let rt = self.core.rt();
        if w.snapshot_store != rt.assets.store_id() || w.snapshot_restored != rt.restored {
            w.pending_snapshot = None;
            w.render_snapshot = None;
            w.snapshot_store = rt.assets.store_id();
            w.snapshot_restored = rt.restored;
        }
        if w.pending_snapshot.is_none() {
            w.pending_snapshot = Some(rt.schedule_render_snapshot()?);
        }
        if let Some(pending) = &mut w.pending_snapshot {
            if let Some(snapshot) = pending.poll(rt)? {
                w.render_snapshot = Some(snapshot);
                w.pending_snapshot = None;
            }
        }
        let Some(snapshot) = &w.render_snapshot else {
            return Ok(());
        };
        w.renderer.set_profiler(rt.profiler.clone(), snapshot.frame);
        let (frame, reconfigure) = match w.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Lost => {
                w.surface = self.instance.create_surface(w.window.clone())?;
                w.surface.configure(&w.renderer.gpu.device, &w.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                w.surface.configure(&w.renderer.gpu.device, &w.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                bail!("surface acquisition validation failed")
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let rt = self.core.rt();
        w.renderer.set_scene3d(snapshot.simple.as_ref())?;
        w.renderer.set_pbr_scene(snapshot.pbr.clone())?;
        w.renderer.set_engine_frame(snapshot.layers.clone())?;
        w.renderer.set_transition(snapshot.transition.clone())?;
        rt.assets
            .with_frame_surfaces(snapshot.surfaces.clone(), |assets| {
                w.renderer.render(
                    &snapshot.scene,
                    assets,
                    RenderTarget {
                        view: &view,
                        size: [w.config.width, w.config.height],
                    },
                    rt.project.canvas,
                    rt.project.background,
                )
            })?;
        w.window.pre_present_notify();
        frame.present();
        if reconfigure {
            w.surface.configure(&w.renderer.gpu.device, &w.config);
        }
        self.presented += 1;
        if self
            .options
            .frames
            .is_some_and(|limit| self.presented >= limit)
        {
            if let Some(path) = &self.options.capture {
                capture_existing(
                    &mut w.renderer,
                    self.core.rt(),
                    self.options.font.as_deref(),
                    path,
                )?;
            }
            el.exit();
        }
        Ok(())
    }
    fn event(&mut self, el: &ActiveEventLoop, event: WindowEvent) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.redraw(el)?,
            WindowEvent::Resized(size) => {
                if let Some(w) = self.window.as_mut() {
                    w.minimized = size.width == 0 || size.height == 0;
                    if !w.minimized {
                        ensure!(
                            size.width <= w.renderer.gpu.device.limits().max_texture_dimension_2d
                                && size.height
                                    <= w.renderer.gpu.device.limits().max_texture_dimension_2d,
                            "window exceeds GPU target dimensions"
                        );
                        w.config.width = size.width;
                        w.config.height = size.height;
                        w.surface.configure(&w.renderer.gpu.device, &w.config);
                    }
                }
                self.last_tick = Instant::now();
                self.remainder = Duration::ZERO;
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.input = InputFrame::default();
                #[cfg(feature = "gamepad")]
                if let Some(backend) = &mut self.gamepad {
                    backend.reset_navigation();
                }
                #[cfg(feature = "scripting")]
                if let Core::Scripted(app) = &mut self.core {
                    let actions = &mut app.context().state()?.actions;
                    actions.reset_devices();
                    actions.cancel_rebind();
                }
            }
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                self.ime_composing = !text.is_empty();
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.committed_text(&text)?;
                self.ime_composing = false;
            }
            WindowEvent::Ime(winit::event::Ime::Disabled) => {
                self.ime_composing = false;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if !self.focused {
                    return Ok(());
                }
                let pressed = event.state == ElementState::Pressed;
                if pressed && !self.ime_composing {
                    if let Some(text) = event.text.as_ref() {
                        self.committed_text(text.as_str())?;
                    }
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if pressed && !event.repeat {
                        match code {
                            KeyCode::Escape => {
                                if !self.core.rt().host.engine.ui.focused_text_field() {
                                    el.exit();
                                }
                            }
                            KeyCode::F5 => self.save()?,
                            KeyCode::F9 => self.load()?,
                            KeyCode::F12 => {
                                let path = self.options.capture.clone().unwrap_or_else(|| {
                                    self.options.state_dir.join(format!(
                                        "capture-{}.png",
                                        self.core.rt().host.scene.frame()
                                    ))
                                });
                                if let Some(w) = &mut self.window {
                                    capture_existing(
                                        &mut w.renderer,
                                        self.core.rt(),
                                        self.options.font.as_deref(),
                                        &path,
                                    )?;
                                }
                            }
                            _ => {}
                        }
                    }
                    let bit = match code {
                        KeyCode::Enter | KeyCode::NumpadEnter => Some(1),
                        KeyCode::Space => Some(2),
                        KeyCode::ArrowLeft => Some(4),
                        KeyCode::ArrowRight => Some(8),
                        KeyCode::ArrowUp => Some(16),
                        KeyCode::ArrowDown => Some(32),
                        KeyCode::Tab => Some(64),
                        KeyCode::KeyW => Some(1 << 7),
                        KeyCode::KeyA => Some(1 << 8),
                        KeyCode::KeyS => Some(1 << 9),
                        KeyCode::KeyD => Some(1 << 10),
                        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(1 << 11),
                        KeyCode::ControlLeft | KeyCode::ControlRight => Some(1 << 12),
                        KeyCode::KeyE => Some(1 << 13),
                        KeyCode::KeyQ => Some(1 << 14),
                        KeyCode::KeyF => Some(1 << 15),
                        KeyCode::KeyR => Some(1 << 18),
                        KeyCode::Escape => Some(buttons::ESCAPE),
                        KeyCode::Backspace => Some(buttons::BACKSPACE),
                        KeyCode::Home => Some(buttons::HOME),
                        KeyCode::End => Some(buttons::END),
                        KeyCode::PageUp => Some(buttons::PAGE_UP),
                        KeyCode::PageDown => Some(buttons::PAGE_DOWN),
                        KeyCode::Digit0 | KeyCode::Numpad0 => Some(buttons::KEY_0),
                        KeyCode::Digit1 | KeyCode::Numpad1 => Some(buttons::KEY_1),
                        KeyCode::Digit2 | KeyCode::Numpad2 => Some(buttons::KEY_2),
                        KeyCode::Digit3 | KeyCode::Numpad3 => Some(buttons::KEY_3),
                        KeyCode::Digit4 | KeyCode::Numpad4 => Some(buttons::KEY_4),
                        KeyCode::Digit5 | KeyCode::Numpad5 => Some(buttons::KEY_5),
                        KeyCode::Digit6 | KeyCode::Numpad6 => Some(buttons::KEY_6),
                        KeyCode::Digit7 | KeyCode::Numpad7 => Some(buttons::KEY_7),
                        KeyCode::Digit8 | KeyCode::Numpad8 => Some(buttons::KEY_8),
                        KeyCode::Digit9 | KeyCode::Numpad9 => Some(buttons::KEY_9),
                        KeyCode::F1 => Some(buttons::F1),
                        KeyCode::F2 => Some(buttons::F2),
                        KeyCode::F3 => Some(buttons::F3),
                        KeyCode::F4 => Some(buttons::F4),
                        KeyCode::F5 => Some(buttons::F5),
                        KeyCode::F6 => Some(buttons::F6),
                        KeyCode::F7 => Some(buttons::F7),
                        KeyCode::F8 => Some(buttons::F8),
                        KeyCode::F9 => Some(buttons::F9),
                        KeyCode::F10 => Some(buttons::F10),
                        KeyCode::F11 => Some(buttons::F11),
                        KeyCode::F12 => Some(buttons::F12),
                        _ => None,
                    };
                    if let Some(bit) = bit {
                        self.button(bit, pressed);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.focused {
                    let y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(p) => p.y,
                    };
                    if y != 0.0 {
                        self.input.buttons_pressed |= if y > 0.0 {
                            buttons::WHEEL_UP
                        } else {
                            buttons::WHEEL_DOWN
                        };
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let bit = match button {
                    MouseButton::Left => Some(1 << 16),
                    MouseButton::Right => Some(1 << 17),
                    _ => None,
                };
                if let Some(bit) = bit {
                    self.button(bit, state == ElementState::Pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(w) = &self.window {
                    if !w.minimized {
                        let vp = Viewport::fit(
                            self.core.rt().project.canvas,
                            [w.config.width, w.config.height],
                        );
                        let [x, y] =
                            vp.pointer(position.x, position.y, self.core.rt().project.canvas);
                        self.input.pointer_x = x;
                        self.input.pointer_y = y;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn cap_dir(path: &Path) -> Result<cap_std::fs::Dir> {
    Ok(cap_std::fs::Dir::open_ambient_dir(
        path,
        cap_std::ambient_authority(),
    )?)
}
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if let Err(e) = self.init(el) {
            self.fail(el, e);
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.window = None;
        self.audio.pause();
        self.input = InputFrame::default();
        #[cfg(feature = "gamepad")]
        if let Some(backend) = &mut self.gamepad {
            backend.reset_navigation();
        }
        #[cfg(feature = "scripting")]
        if let Core::Scripted(app) = &mut self.core {
            if let Ok(state) = app.context().state() {
                state.actions.reset_devices();
                state.actions.cancel_rebind();
            }
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_some_and(|w| w.window.id() == id) {
            if let Err(e) = self.event(el, event) {
                self.fail(el, e);
            }
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        el.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(16),
        ));
        if let Some(w) = &self.window {
            if !w.minimized {
                w.window.request_redraw();
            }
        }
    }
}
