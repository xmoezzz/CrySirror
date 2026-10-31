// SPDX-License-Identifier: MPL-2.0
// `deny`, not `forbid`: only the Windows menu FFI (native_menu.rs) opts out.
#![deny(unsafe_code)]
mod app;
mod audio;
mod cursor;
mod detect;
#[cfg(feature = "gamepad")]
mod gamepad;
mod input_track;
mod launch;
mod native_menu;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use crysirror_wgpu::{Gpu, RenderingPath};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

#[derive(Clone, Copy, Default, clap::ValueEnum)]
enum RenderChoice {
    #[default]
    Raster,
    ComputeRaytrace,
    HardwareRaytrace,
}
impl From<RenderChoice> for RenderingPath {
    fn from(value: RenderChoice) -> Self {
        match value {
            RenderChoice::Raster => Self::Raster,
            RenderChoice::ComputeRaytrace => Self::ComputeRaytrace,
            RenderChoice::HardwareRaytrace => Self::HardwareRaytrace,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "crysirror",
    version,
    about = "Data-driven wgpu/winit host; native VM adapters are explicit plugins"
)]
struct Args {
    /// Without a subcommand: play the game in the folder holding this binary
    /// (double-click launch inside the game folder).
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(clap::Args, Default)]
struct PlayOptions {
    game: Option<PathBuf>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    font: Option<PathBuf>,
    /// Print the detected engine, project and archive mount plan, then exit.
    #[arg(long)]
    detect_only: bool,
    /// Validate/play offscreen, without creating a window or audio device.
    #[arg(long)]
    headless: bool,
    #[arg(long, value_parser=clap::value_parser!(u32).range(1..=1000000))]
    frames: Option<u32>,
    #[arg(long)]
    capture: Option<PathBuf>,
    #[arg(long)]
    input_track: Option<PathBuf>,
    #[arg(long)]
    no_audio: bool,
    /// Allow the game's windowed borderStyle to hide the OS title bar.
    #[arg(long)]
    allow_borderless: bool,
}
#[derive(Subcommand)]
enum Command {
    /// Detect the engine of an installed game and run it in place: the
    /// game folder (or a file inside it, such as its executable).
    Play(PlayOptions),
    /// Bake local E-mote PSB files into an engine puppet (rig JSON + PNG
    /// textures + report). The output never contains PSB data structures.
    #[cfg(feature = "emote-import")]
    PuppetImport {
        /// Main PSB first, then any additional sources.
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// PSB decryption key, if the files are encrypted.
        #[arg(long)]
        key: Option<u32>,
        #[arg(long, default_value_t = 5)]
        samples: usize,
    },
    /// Open a real winit window and render through wgpu.
    Run {
        project: PathBuf,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        font: Option<PathBuf>,
        #[arg(long)]
        no_audio: bool,
        #[arg(long)]
        allow_borderless: bool,
        #[arg(long,value_parser=clap::value_parser!(u64).range(1..=1000000))]
        frames: Option<u64>,
        #[arg(long)]
        capture: Option<PathBuf>,
        /// Export CPU Chrome trace and completed GPU timing diagnostics on exit.
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "raster")]
        renderer: RenderChoice,
        /// Prepared CPU asset-cache budget, separate from total RAM/VRAM.
        #[arg(long,default_value_t=256,value_parser=clap::value_parser!(u32).range(16..=4096))]
        asset_cache_mib: u32,
        /// JS/TS sources run by the engine App (playable games).
        #[arg(long)]
        script: Vec<PathBuf>,
        #[arg(long, default_value = "game")]
        script_client: String,
    },
    /// Use the same GPU renderer offscreen; does not emit sound.
    Render {
        project: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=1000000))]
        frames: u32,
        #[arg(long, default_value_t = 16)]
        step_ms: u64,
        #[arg(long)]
        press_at: Vec<u32>,
        /// JSON input track for warm-up frames (zero-based frame/down/pressed/pointer).
        #[arg(long)]
        input_track: Option<PathBuf>,
        #[arg(long)]
        font: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "raster")]
        renderer: RenderChoice,
        /// Prepared CPU asset-cache budget, separate from total RAM/VRAM.
        #[arg(long,default_value_t=256,value_parser=clap::value_parser!(u32).range(16..=4096))]
        asset_cache_mib: u32,
        /// JS/TS sources run by the engine App before capture (repeatable).
        #[arg(long)]
        script: Vec<PathBuf>,
        /// Project client the scripts act as.
        #[arg(long, default_value = "game")]
        script_client: String,
        /// After the warm-up frames, also write this many consecutive frames
        /// (`step_ms` apart) as PNGs into `--movie-dir`, with one renderer.
        #[arg(long, default_value_t = 0, value_parser=clap::value_parser!(u32).range(0..=100000))]
        movie_frames: u32,
        #[arg(long)]
        movie_dir: Option<PathBuf>,
    },
    /// Write TypeScript declarations generated from the engine class registry.
    ScriptTypes {
        project: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "game")]
        script_client: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Validate a client-scoped WGSL surface asset without creating a GPU.
    ShaderCheck {
        project: PathBuf,
        #[arg(long)]
        client: String,
        #[arg(long)]
        asset: String,
    },
    /// Parse/validate data, without claiming native adapter or asset compatibility.
    Validate { project: PathBuf },
    /// Execute VM ticks without window/GPU/audio and print the service trace.
    Trace {
        project: PathBuf,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=1000000))]
        frames: u32,
        #[arg(long, default_value_t = 16)]
        step_ms: u64,
        #[arg(long)]
        press_at: Vec<u32>,
        /// JSON input track (zero-based frame/down/pressed/pointer).
        #[arg(long)]
        input_track: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// JS/TS sources run by the engine App without a renderer (repeatable).
        #[arg(long)]
        script: Vec<PathBuf>,
        #[arg(long, default_value = "game")]
        script_client: String,
        /// Optional font for CPU text measurement during native callbacks.
        #[arg(long)]
        font: Option<PathBuf>,
    },
    /// Inspect source data and actually registered adapter factories.
    Inspect { project: PathBuf },
    /// Probe actual adapter features separately from implemented renderer paths.
    GpuInfo,
}
fn state_dir(project: &Path, explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    let p = Project::load(project)?;
    let fingerprint = p.fingerprint()?;
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
    }
    .unwrap_or_else(|| PathBuf::from(".crysirror-state"));
    Ok(base.join("CrySirror").join(&fingerprint[..16]))
}
fn state_dir_base() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
    }
    .unwrap_or_else(|| PathBuf::from(".crysirror-state"))
}
fn adapter_registry() -> Result<AdapterRegistry> {
    #[allow(unused_mut)]
    let mut registry = AdapterRegistry::default();
    #[cfg(feature = "reallive")]
    crysirror_reallive::register(&mut registry)?;
    #[cfg(feature = "krkr")]
    crysirror_krkr::register(&mut registry)?;
    #[cfg(feature = "rfvp")]
    crysirror_rfvp::register(&mut registry)?;
    #[cfg(feature = "nscripter")]
    crysirror_nscripter::register(&mut registry)?;
    #[cfg(feature = "bgi")]
    crysirror_bgi::register(&mut registry)?;
    #[cfg(feature = "circus")]
    crysirror_circus::register(&mut registry)?;
    #[cfg(feature = "catsystem2")]
    crysirror_catsystem2::register(&mut registry)?;
    #[cfg(feature = "majiro")]
    crysirror_majiro::adapter::register(&mut registry)?;
    #[cfg(feature = "system-nnn")]
    crysirror_system_nnn::installed::register(&mut registry)?;
    #[cfg(feature = "avg32")]
    crysirror_avg32::register(&mut registry)?;
    Ok(registry)
}
fn load_project(path: &Path) -> Result<Project> {
    let project = Project::load(path)?;
    #[cfg(feature = "system-nnn")]
    for client in project.clients.iter().filter(|client|client.adapter=="system-nnn") {
        serde_json::from_value::<crysirror_system_nnn::installed::Config>(client.config.clone())?.validate()?;
    }
    #[cfg(feature = "krkr")]
    for c in project.clients.iter().filter(|c| c.adapter == "krkr") {
        serde_json::from_value::<crysirror_krkr::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "rfvp")]
    for c in project.clients.iter().filter(|c| c.adapter == "rfvp") {
        serde_json::from_value::<crysirror_rfvp::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "reallive")]
    for c in project.clients.iter().filter(|c| c.adapter == "reallive") {
        serde_json::from_value::<crysirror_reallive::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "nscripter")]
    for c in project.clients.iter().filter(|c| c.adapter == "nscripter") {
        serde_json::from_value::<crysirror_nscripter::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "bgi")]
    for c in project.clients.iter().filter(|c| c.adapter == "bgi") {
        serde_json::from_value::<crysirror_bgi::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "circus")]
    for c in project.clients.iter().filter(|c| c.adapter == "circus") {
        serde_json::from_value::<crysirror_circus::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "catsystem2")]
    for c in project.clients.iter().filter(|c| c.adapter == "catsystem2") {
        serde_json::from_value::<crysirror_catsystem2::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "majiro")]
    for c in project.clients.iter().filter(|c| c.adapter == "majiro") {
        serde_json::from_value::<crysirror_majiro::adapter::Config>(c.config.clone())?.validate()?;
    }
    #[cfg(feature = "avg32")]
    for c in project.clients.iter().filter(|c| c.adapter == "avg32") {
        serde_json::from_value::<crysirror_avg32::Config>(c.config.clone())?.validate()?;
    }
    Ok(project)
}
fn load_runtime(path: &Path, dir: &Path) -> Result<Runtime> {
    let registry = adapter_registry()?;
    let mut rt = Runtime::with_registry(load_project(path)?, path, dir, registry)?;
    #[cfg(feature = "siglus-resources")]
    rt.assets
        .decoders
        .register("g00", Arc::new(crysirror_siglus::SiglusImageDecoder))?;
    Ok(rt)
}
#[cfg(feature = "scripting")]
fn scripted(
    rt: Runtime,
    scripts: &[PathBuf],
    client: &str,
    frames: u32,
    step_ms: u64,
    input: impl Fn(u32) -> InputFrame,
    prepare_render: bool,
    mut each_frame: impl FnMut(&mut Runtime) -> Result<()>,
) -> Result<Runtime> {
    let mut app = start_app(rt, scripts, client)?;
    for frame in 0..frames {
        app.frame(if frame == 0 { 0 } else { step_ms }, input(frame))?;
        each_frame(app.runtime_mut()?)?;
        if app.quit_requested() {
            break;
        }
    }
    for line in &app.engine()?.log {
        log::info!("script: {line}");
    }
    if prepare_render {
        app.prepare_render_offline()?;
    }
    app.into_runtime()
}
#[cfg(feature = "scripting")]
fn start_app(rt: Runtime, scripts: &[PathBuf], client: &str) -> Result<crysirror_engine::App> {
    use crysirror_engine::ScriptHost;
    let mut app = crysirror_engine::App::new(rt, client)?;
    let mut host = crysirror_script_js::JsHost::new("scripts");
    for path in scripts {
        let source = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script.js");
        host.add_source(name, &source)?;
    }
    app.add_script_host(Box::new(host))?;
    app.init()?;
    app.start()?;
    Ok(app)
}
/// Scripted movie: the App keeps running its behaviours between frames.
#[cfg(feature = "scripting")]
struct ScriptedMovie(crysirror_engine::App);
#[cfg(feature = "scripting")]
impl app::MovieSource for ScriptedMovie {
    fn prepare_capture(&mut self) -> Result<()> {
        self.0.prepare_render_offline()
    }
    fn advance(&mut self, step_ms: u64) -> Result<()> {
        self.0.frame(step_ms, InputFrame::default())
    }
    fn runtime(&mut self) -> &mut Runtime {
        self.0.runtime_mut().expect("engine app owns its runtime")
    }
}
#[cfg(feature = "scripting")]
fn script_types(rt: Runtime, client: &str, output: &Path) -> Result<()> {
    let app = crysirror_engine::App::new(rt, client)?;
    std::fs::write(output, app.typescript_declarations()?)?;
    Ok(())
}
#[cfg(not(feature = "scripting"))]
fn script_types(_: Runtime, _: &str, _: &Path) -> Result<()> {
    anyhow::bail!("script-types requires the 'scripting' feature")
}
#[cfg(feature = "scripting")]
fn engine_core(rt: Runtime, scripts: &[PathBuf], client: &str) -> Result<app::Core> {
    use crysirror_engine::ScriptHost;
    let mut app = crysirror_engine::App::new(rt, client)?;
    let mut host = crysirror_script_js::JsHost::new("scripts");
    for path in scripts {
        let source = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script.js");
        host.add_source(name, &source)?;
    }
    app.add_script_host(Box::new(host))?;
    app.init()?;
    app.start()?;
    Ok(app::Core::Scripted(Box::new(app)))
}
#[cfg(not(feature = "scripting"))]
fn engine_core(_: Runtime, _: &[PathBuf], _: &str) -> Result<app::Core> {
    anyhow::bail!("--script requires the 'scripting' feature")
}
#[cfg(not(feature = "scripting"))]
fn scripted(
    _: Runtime,
    _: &[PathBuf],
    _: &str,
    _: u32,
    _: u64,
    _: impl Fn(u32) -> InputFrame,
    _: bool,
    _: impl FnMut(&mut Runtime) -> Result<()>,
) -> Result<Runtime> {
    anyhow::bail!("--script requires the 'scripting' feature")
}
/// `crysirror play`: engine detection, then the ordinary scripted host with
/// the game folder as the explicit asset root.
fn play(options: PlayOptions) -> Result<()> {
    let PlayOptions {
        game,
        data_dir,
        font,
        detect_only,
        headless,
        frames,
        capture,
        input_track,
        no_audio,
        allow_borderless,
    } = options;
    anyhow::ensure!(
        headless || input_track.is_none(),
        "--input-track requires --headless"
    );
    let dir = match game {
        Some(p) if p.is_file() => p
            .parent()
            .context("game file without a folder")?
            .to_path_buf(),
        Some(p) => p,
        None => std::env::current_exe()?
            .parent()
            .context("binary without a folder")?
            .to_path_buf(),
    };
    let dir = dir
        .canonicalize()
        .with_context(|| format!("game folder {}", dir.display()))?;
    let detection_root = dir.clone();
    let plan = std::thread::Builder::new()
        .name("crysirror-detect".into())
        .spawn(move || detect::detect(&detection_root))
        .context("spawn engine detection worker")?
        .join()
        .map_err(|_| anyhow::anyhow!("engine detection worker panicked"))??;
    log::info!("detected {} game in {}", plan.engine, dir.display());
    if detect_only {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    let project: Project = serde_json::from_value(plan.project.clone())?;
    let state_dir = match data_dir {
        Some(d) => d,
        None => {
            // Same per-user location as projects, keyed by the folder too so
            // two copies of a game keep separate saves.
            // FNV-1a 64 over the folder path: stable across builds.
            let folder = dir
                .to_string_lossy()
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                });
            // Player launch options do not change the installed game's save
            // directory. Keep the previous identity while the actual project
            // fingerprint still validates framework snapshots independently.
            let mut identity_project = project.clone();
            for client in &mut identity_project.clients {
                if client.adapter == "krkr" {
                    if let Some(config) = client.config.as_object_mut() {
                        config.remove("arguments");
                    }
                }
            }
            let fingerprint = identity_project.fingerprint()?;
            state_dir_base()
                .join("CrySirror")
                .join(format!("{}-{folder:016x}", &fingerprint[..8]))
        }
    };
    std::fs::create_dir_all(&state_dir)?;
    let mut rt = Runtime::with_asset_root(project, &dir, &state_dir, adapter_registry()?)?;
    #[cfg(feature = "siglus-resources")]
    rt.assets
        .decoders
        .register("g00", Arc::new(crysirror_siglus::SiglusImageDecoder))?;
    rt.set_audio_nonblocking(!headless);
    // Prefer a supplied game font to the host fallback. Do not override it
    // merely because a system CJK font happens to exist.
    let font = font.or_else(|| {
        (!rt.project.fonts.contains_key("default"))
            .then(detect::system_font)
            .flatten()
    });
    app::load_fonts(&mut rt, font.as_deref())?;
    if headless {
        rt.host.display = Some(crysirror_core::service::DisplayInfo {
            screen: [1920, 1080],
            work: [0, 0, 1920, 1080],
        });
    }
    let core = launch::bootstrap(rt, &plan)?;
    if headless {
        return launch::offscreen(
            core,
            frames.unwrap_or(600),
            &capture.unwrap_or_else(|| state_dir.join("last-frame.png")),
            input_track.as_deref(),
            font.as_deref(),
        );
    }

    app::App::run(
        core,
        app::RunOptions {
            font,
            muted: no_audio,
            frames: frames.map(u64::from),
            capture,
            profile_path: None,
            state_dir,
            rendering_path: RenderingPath::Raster,
            allow_borderless,
        },
    )
}
fn run() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();
    let command = args
        .command
        .unwrap_or(Command::Play(PlayOptions::default()));
    match command {
        Command::Play(options) => play(options),
        Command::Run {
            project,
            data_dir,
            font,
            no_audio,
            allow_borderless,
            frames,
            capture,
            profile,
            renderer,
            asset_cache_mib,
            script,
            script_client,
        } => {
            let state_dir = state_dir(&project, data_dir)?;
            let mut rt = load_runtime(&project, &state_dir)?;
            rt.set_audio_nonblocking(true);
            app::load_fonts(&mut rt, font.as_deref())?;
            rt.assets
                .set_prepared_budget(usize::try_from(u64::from(asset_cache_mib) * 1024 * 1024)?)?;
            let core = if script.is_empty() {
                app::Core::Plain(rt)
            } else {
                engine_core(rt, &script, &script_client)?
            };
            app::App::run(
                core,
                app::RunOptions {
                    font,
                    muted: no_audio,
                    frames,
                    capture,
                    profile_path: profile,
                    state_dir,
                    rendering_path: renderer.into(),
                    allow_borderless,
                },
            )
        }
        Command::Render {
            project,
            output,
            frames,
            step_ms,
            press_at,
            input_track,
            font,
            data_dir,
            renderer,
            asset_cache_mib,
            script,
            script_client,
            movie_frames,
            movie_dir,
        } => {
            anyhow::ensure!(step_ms <= 86400000, "step exceeds one day");
            anyhow::ensure!(
                (movie_frames == 0) == movie_dir.is_none(),
                "--movie-frames and --movie-dir go together"
            );
            let dir = state_dir(&project, data_dir)?;
            let mut rt = load_runtime(&project, &dir)?;
            // Headless renders have no monitor; VMs that size their window
            // from the display see a fixed 1920x1080 virtual screen.
            rt.host.display = Some(crysirror_core::service::DisplayInfo {
                screen: [1920, 1080],
                work: [0, 0, 1920, 1080],
            });
            // Native callbacks may measure fonts during the first VM tick.
            app::load_fonts(&mut rt, font.as_deref())?;
            rt.assets
                .set_prepared_budget(usize::try_from(u64::from(asset_cache_mib) * 1024 * 1024)?)?;
            let track = input_track::InputTrack::load(input_track.as_deref())?;
            let input = |frame: u32| {
                let mut input = track.at(frame);
                input.buttons_pressed |= u64::from(press_at.contains(&frame));
                input
            };
            let instance = wgpu::Instance::default();
            let path: RenderingPath = renderer.into();
            let gpu = Arc::new(pollster::block_on(Gpu::request_for(&instance, None, path))?);
            let adapter = format!("{:?}", gpu.adapter.get_info());
            let hardware_raytracing =
                path == RenderingPath::HardwareRaytrace && gpu.capabilities().hardware_raytrace;
            let hardware_rt_report = gpu.hardware_rt_report();
            #[cfg(feature = "scripting")]
            if let (false, Some(dir)) = (script.is_empty(), &movie_dir) {
                // Scripts keep running while the movie is captured.
                let mut app = start_app(rt, &script, &script_client)?;
                for frame in 0..frames {
                    app.frame(if frame == 0 { 0 } else { step_ms }, input(frame))?;
                }
                let mut movie = ScriptedMovie(app);
                let stats = app::movie_on(
                    gpu,
                    &mut movie,
                    font.as_deref(),
                    &output,
                    dir,
                    movie_frames,
                    step_ms,
                    path,
                )?;
                for line in &movie.0.engine()?.log {
                    log::info!("script: {line}");
                }
                let rt = movie.0.into_runtime()?;
                println!(
                    "{}",
                    serde_json::json!({"output":output,"gpu":adapter,"renderer":path.name(),"stats":stats,"hardware_raytracing":hardware_raytracing,"hardware_rt_report":hardware_rt_report,"runtime":rt.inspect(),"audio_emitted":false})
                );
                return Ok(());
            }
            // Compacted canvases are materialized as frames advance, as the
            // window host does after each presented frame.
            app::load_fonts(&mut rt,font.as_deref())?;
            let mut materializer = crysirror_wgpu::Renderer::new(gpu.clone(),
                if std::env::var_os("CRYSIRROR_CAPTURE_BGRA").is_some() { wgpu::TextureFormat::Bgra8UnormSrgb }
                else { wgpu::TextureFormat::Rgba8UnormSrgb });
            materializer.set_rendering_path(path)?;
            rt.set_canvas_image_presenter(true);
            app::settle_vm_preparations(&mut rt)?;
            if movie_dir.is_none() && std::env::var_os("CRYSIRROR_CPU_FRAME_HISTORY").is_none() {
                rt.host.mesh_frame_history = Some(materializer.mesh_frame_history_provider());
            }
            let mut materialize =
                |rt: &mut Runtime| app::materialize_canvases(&mut materializer, rt);
            if script.is_empty() {
                for frame in 0..frames {
                    rt.tick(if frame == 0 { 0 } else { step_ms }, input(frame))?;
                    materialize(&mut rt)?;
                }
            } else {
                rt = scripted(
                    rt,
                    &script,
                    &script_client,
                    frames,
                    step_ms,
                    input,
                    true,
                    &mut materialize,
                )?;
            }
            drop(materialize);
            let stats = match &movie_dir {
                Some(dir) => app::movie_on(
                    gpu,
                    &mut rt,
                    font.as_deref(),
                    &output,
                    dir,
                    movie_frames,
                    step_ms,
                    path,
                )?,
                None => app::capture_existing(&mut materializer, &mut rt, font.as_deref(), &output)?,
            };
            println!(
                "{}",
                serde_json::json!({"output":output,"gpu":adapter,"renderer":path.name(),"stats":stats,"hardware_raytracing":hardware_raytracing,"hardware_rt_report":hardware_rt_report,"runtime":rt.inspect(),"audio_emitted":false})
            );
            Ok(())
        }
        Command::ScriptTypes {
            project,
            output,
            script_client,
            data_dir,
        } => {
            let dir = state_dir(&project, data_dir)?;
            let rt = load_runtime(&project, &dir)?;
            script_types(rt, &script_client, &output)
        }
        Command::ShaderCheck {
            project,
            client,
            asset,
        } => {
            let p = Project::load(&project)?;
            let (index, spec) = p
                .clients
                .iter()
                .enumerate()
                .find(|(_, c)| c.name == client)
                .context("unknown shader client")?;
            let mut assets = crysirror_runtime::assets::Assets::new(
                &crysirror_runtime::runtime::asset_root(&project, &p)?,
            )?;
            let id = crysirror_core::state::ClientId(index as u32 + 1);
            assets.register(id, &spec.asset_prefix)?;
            let bytes = assets.read_for(id, &asset)?;
            crysirror_wgpu::effect::check(std::str::from_utf8(&bytes).context("WGSL UTF-8")?)
                .with_context(|| format!("shader {client}/{asset}"))?;
            println!(
                "{}",
                serde_json::json!({"valid":true,"abi":1,"client":client,"asset":asset,"stages":["color","prepass","shadow"],"gpu_created":false})
            );
            Ok(())
        }
        Command::Validate { project } => {
            let p = load_project(&project)?;
            println!(
                "{}",
                serde_json::json!({"schema_valid":true,"name":p.name,"native_adapter_verification":false})
            );
            Ok(())
        }
        Command::Trace {
            project,
            frames,
            step_ms,
            press_at,
            input_track,
            data_dir,
            script,
            script_client,
            font,
        } => {
            anyhow::ensure!(step_ms <= 86400000, "step exceeds one day");
            let dir = state_dir(&project, data_dir)?;
            let mut rt = load_runtime(&project, &dir)?;
            // Preserve the plain trace path unless font/script support was
            // requested. Native scripts can measure text on their first tick.
            if font.is_some() || !script.is_empty() {
                app::load_fonts(&mut rt, font.as_deref())?;
            }
            let track = input_track::InputTrack::load(input_track.as_deref())?;
            let input = |frame: u32| {
                let mut input = track.at(frame);
                input.buttons_pressed |= u64::from(press_at.contains(&frame));
                input
            };
            if script.is_empty() {
                for frame in 0..frames {
                    rt.tick(if frame == 0 { 0 } else { step_ms }, input(frame))?;
                }
            } else {
                // No rendering preparation, GPU instance/device, window, or
                // audio output device is created by this CPU execution path.
                rt = scripted(
                    rt,
                    &script,
                    &script_client,
                    frames,
                    step_ms,
                    input,
                    false,
                    |_| Ok(()),
                )?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"runtime":rt.inspect(),"events":rt.host.trace,"pixels_rendered":false,"audio_emitted":false})
                )?
            );
            Ok(())
        }
        #[cfg(feature = "emote-import")]
        Command::PuppetImport {
            sources,
            out,
            key,
            samples,
        } => puppet_import(&sources, &out, key, samples),
        Command::GpuInfo => {
            let instance = wgpu::Instance::default();
            // Inspect every available backend/adapter without requesting experimental
            // features. The default device below remains a normal raster device.
            let all_adapters: Vec<_> =
                pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                    .iter()
                    .map(|adapter| crysirror_wgpu::rt_capabilities::report(adapter, None))
                    .collect();
            let gpu = pollster::block_on(Gpu::request(&instance, None))?;
            let caps = gpu.capabilities();
            let features = gpu.adapter.features();
            println!(
                "{}",
                serde_json::json!({"adapter":format!("{:?}",gpu.adapter.get_info()),
                "implemented":{"raster":true,"compute_raytrace":true,"hardware_raytrace":true},
                "default_device_enabled":{"raster":caps.raster,"compute_raytrace":caps.compute_raytrace,"hardware_raytrace":caps.hardware_raytrace},
                "hardware_rt_report":gpu.hardware_rt_report(),"all_adapters":all_adapters,
                "adapter_advertises":{"ray_query":features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),"acceleration_structures":features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)}})
            );
            Ok(())
        }
        Command::Inspect { project } => {
            let p = load_project(&project)?;
            let registry = adapter_registry()?;
            let adapters = registry.names();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"project":p,"registered_adapters":adapters,
                        "rfvp_native_vm":adapters.contains(&"rfvp"),
                        "bgi_native_vm":adapters.contains(&"bgi"),
                        "nscripter_native_vm":adapters.contains(&"nscripter"),
                        "reallive_native_vm":adapters.contains(&"reallive"),
                        "avg32_native_vm":adapters.contains(&"avg32"),
                        "siglus_native_vm":adapters.contains(&"siglus")})
                )?
            );
            Ok(())
        }
    }
}
fn main() -> ExitCode {
    match run().context("CrySirror") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "emote-import")]
fn puppet_import(
    sources: &[PathBuf],
    out: &std::path::Path,
    key: Option<u32>,
    samples: usize,
) -> Result<()> {
    use crysirror_puppet::PuppetSource;
    let bytes: Vec<Vec<u8>> = sources
        .iter()
        .map(|p| std::fs::read(p).with_context(|| format!("read {}", p.display())))
        .collect::<Result<_>>()?;
    let refs: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    let mut source = crysirror_puppet_eluna::ElunaPuppet::from_psb(&refs, key)?;
    let options = crysirror_puppet::BakeOptions {
        samples_per_parameter: samples,
        provenance: "crysirror puppet-import (eluna E-mote bridge)".into(),
        ..Default::default()
    };
    let (rig, textures, report) = crysirror_puppet::bake(&mut source, &options)?;
    std::fs::create_dir_all(out)?;
    for t in &textures {
        let (w, h, rgba) = t
            .rgba
            .clone()
            .with_context(|| format!("texture {} has no pixels", t.source_name))?;
        image::RgbaImage::from_raw(w, h, rgba)
            .context("texture size")?
            .save_with_format(out.join(&t.file), image::ImageFormat::Png)?;
    }
    std::fs::write(out.join("puppet.json"), serde_json::to_vec_pretty(&rig)?)?;
    let diag = source.diagnostics();
    let summary = serde_json::json!({
        "report": report,
        "bridge": {"feedback_sprites": diag.feedback_sprites, "missing_textures": diag.missing_textures, "mixed_mask_chains": diag.mixed_mask_chains},
        "size": source.size(),
        "output": out.join("puppet.json"),
    });
    std::fs::write(
        out.join("import-report.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{summary}");
    Ok(())
}
