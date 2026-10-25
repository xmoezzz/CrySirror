//! CrySirror port addition (not upstream): a synchronous embedding host.
//!
//! Derived from `rfvp_os_host.rs` at the pinned commit. Differences: the game
//! directory, packs and saves come from a host-supplied [`RfvpStorage`]
//! instead of ambient paths; frame time is injected; the VM is advanced
//! synchronously (as the UEFI `VmWorker` does) instead of on a worker thread;
//! the host font directory scan is skipped (fonts come from the host).
use std::sync::Arc;

use anyhow::{bail, ensure, Context, Result};

use crate::{
    host_api::RfvpStorage,
    script::{global::GLOBAL, parser::{Nls, Parser}},
    soft_render::{create_soft_renderer, PixelFormat, SoftFramebuffer, SoftRenderer},
    subsystem::{
        anzu_scene::AnzuScene,
        resources::{
            input_manager::KeyCode,
            motion_manager::DissolveType,
            thread_manager::ThreadManager,
            vfs::Vfs,
            window::Window as EngineWindow,
        },
        scene::{SceneAction, SceneMachine},
        scheduler::Scheduler,
        world::GameData,
    },
    vm_runner::VmRunner,
};

pub use crate::subsystem::resources::text_manager::{
    builtin_fonts_installed, install_builtin_fonts, BuiltinFonts,
};

/// Keys and pointer buttons an embedding host can deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKey {
    Enter,
    Space,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Esc,
    Ctrl,
    Shift,
    MouseLeft,
    MouseRight,
}
impl HostKey {
    fn keycode(self) -> KeyCode {
        match self {
            Self::Enter => KeyCode::Enter,
            Self::Space => KeyCode::Space,
            Self::Left => KeyCode::LeftArrow,
            Self::Right => KeyCode::RightArrow,
            Self::Up => KeyCode::UpArrow,
            Self::Down => KeyCode::DownArrow,
            Self::Tab => KeyCode::Tab,
            Self::Esc => KeyCode::Esc,
            Self::Ctrl => KeyCode::Ctrl,
            Self::Shift => KeyCode::Shift,
            Self::MouseLeft => KeyCode::MouseLeft,
            Self::MouseRight => KeyCode::MouseRight,
        }
    }
    fn is_mouse(self) -> bool {
        matches!(self, Self::MouseLeft | Self::MouseRight)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent {
    Down(HostKey),
    Up(HostKey),
    PointerMove { x: i32, y: i32, inside: bool },
    Wheel(i32),
}

pub struct EmbeddedRuntime {
    game: Box<GameData>,
    parser: Parser,
    vm: VmRunner,
    scheduler: Scheduler,
    scenes: SceneMachine,
    renderer: SoftRenderer,
    size: (u32, u32),
    title: String,
    last_dissolve: DissolveType,
    last_dissolve2: bool,
}

impl EmbeddedRuntime {
    /// `storage` is the game directory: packs (`*.bin`), loose folders, the
    /// `.hcb` and the `save/` folder. Built-in fonts must be installed first.
    pub fn new(storage: Arc<dyn RfvpStorage>, nls: Nls) -> Result<Self> {
        ensure!(
            builtin_fonts_installed(),
            "RFVP built-in font missing: the host must call install_builtin_fonts"
        );
        let names = storage
            .list("")
            .map_err(|e| anyhow::anyhow!("list game directory: {e:?}"))?;
        let mut hcbs: Vec<_> = names
            .iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".hcb"))
            .collect();
        hcbs.sort();
        let Some(hcb) = hcbs.first() else {
            bail!("no .hcb file in the game directory")
        };
        let bytes = storage
            .read(hcb)
            .map_err(|e| anyhow::anyhow!("read {hcb}: {e:?}"))?;
        let parser = Parser::from_bytes(bytes, nls)?;
        let size = parser.get_screen_size();
        ensure!(
            size.0 > 0 && size.1 > 0 && size.0 <= 8192 && size.1 <= 8192,
            "HCB screen size {size:?}"
        );

        let mut game = Box::<GameData>::default();
        game.vfs = Vfs::from_dir(nls, storage.clone())?;
        game.save_manager.set_storage(Some(storage));
        game.nls = parser.nls;
        game.set_can_fullscreen(false);
        game.set_window(EngineWindow::new(size, 1.0));
        if let Err(e) = crate::subsystem::global_savedata::try_load_global_savedata_v1(&mut game) {
            log::error!("Failed to load global savedata: {:#}", e);
        }
        GLOBAL
            .lock()
            .map_err(|_| anyhow::anyhow!("RFVP GLOBAL poisoned"))?
            .init_with(
                parser.get_non_volatile_global_count(),
                parser.get_volatile_global_count(),
            );
        let mut threads = ThreadManager::new();
        threads.start_main(parser.get_entry_point());
        let mut scenes = SceneMachine {
            current_scene: Some(Box::<AnzuScene>::default()),
        };
        scenes.apply_scene_action(SceneAction::Start, &mut game);
        let renderer = create_soft_renderer(size.0, size.1, PixelFormat::Rgba8)
            .context("create software renderer")?;
        let title = parser.get_title();
        Ok(Self {
            game,
            parser,
            vm: VmRunner::new(threads),
            scheduler: Scheduler::default(),
            scenes,
            renderer,
            size,
            title,
            last_dissolve: DissolveType::None,
            last_dissolve2: false,
        })
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn framebuffer(&self) -> &SoftFramebuffer {
        self.renderer.framebuffer()
    }
    pub fn should_exit(&self) -> bool {
        self.game.get_lock_scripter() && self.game.get_main_thread_exited()
    }
    pub fn main_thread_exited(&self) -> bool {
        self.game.get_main_thread_exited()
    }
    /// (state, down edges, cursor x, cursor y) frozen for the current frame.
    pub fn input_snapshot(&self) -> (u32, u32, i32, i32) {
        let i = &self.game.inputs_manager;
        (i.get_input_state(), i.get_input_down(), i.get_cursor_x(), i.get_cursor_y())
    }
    /// Raw status bits of each script context, for diagnostics.
    pub fn thread_states(&self) -> Vec<u32> {
        let tm = self.vm.thread_manager();
        (0..tm.total_contexts() as u32)
            .map(|t| tm.get_context_status(t).bits() as u32)
            .collect()
    }
    pub fn push_event(&mut self, event: HostEvent) {
        let input = &mut self.game.inputs_manager;
        match event {
            HostEvent::Down(k) if k.is_mouse() => input.notify_mouse_down(k.keycode()),
            HostEvent::Down(k) => input.notify_keycode_down(k.keycode(), false),
            HostEvent::Up(k) if k.is_mouse() => input.notify_mouse_up(k.keycode()),
            HostEvent::Up(k) => input.notify_keycode_up(k.keycode()),
            HostEvent::PointerMove { x, y, inside } => {
                input.notify_mouse_move(x, y);
                input.set_mouse_in(inside);
            }
            HostEvent::Wheel(d) => input.notify_mouse_wheel(d),
        }
    }

    /// One frame in rfvp_os_host order with an injected delta.
    pub fn step_frame(&mut self, delta_ms: u64) -> Result<&SoftFramebuffer> {
        let gd = &mut *self.game;
        gd.motion_manager.text_manager.set_render_scale(1.0);
        gd.time_mut_ref()
            .set_external_delta(crate::platform_time::Duration::from_millis(delta_ms));
        let frame_us = gd.time_mut_ref().frame().as_micros() as u64;
        let frame_ms = frame_us.div_ceil(1000);
        gd.timer_manager.tick(frame_ms.min(u32::MAX as u64) as u32);
        gd.inputs_manager.begin_frame();
        let mut video_failed = false;
        {
            let (video, motion) = (&mut gd.video_manager, &mut gd.motion_manager);
            if let Err(e) = video.tick(motion) {
                log::error!("VideoPlayerManager::tick failed: {:?}", e);
                video_failed = true;
            }
        }
        if video_failed {
            let (video, motion) = (&mut gd.video_manager, &mut gd.motion_manager);
            video.stop(motion);
            gd.set_halt(false);
        }
        let mut dissolve_done = false;
        let cur = gd.motion_manager.get_dissolve_type();
        if (self.last_dissolve != DissolveType::None && self.last_dissolve != DissolveType::Static)
            && (cur == DissolveType::None || cur == DissolveType::Static)
        {
            dissolve_done = true;
        }
        let cur2 = gd.motion_manager.is_dissolve2_transitioning();
        if self.last_dissolve2 && !cur2 {
            dissolve_done = true;
        }
        self.last_dissolve2 = cur2;
        self.last_dissolve = cur;
        gd.set_current_thread(0);
        if gd.get_halt() {
            gd.set_halt(false);
        }
        // Same calls the threaded VmWorker receives: dissolve-done, then frame.
        if dissolve_done && !gd.get_halt() {
            self.vm.tick(gd, &mut self.parser, 0)?;
        }
        if !gd.get_halt() {
            self.vm.tick(gd, &mut self.parser, frame_ms)?;
        }
        self.scenes.apply_scene_action(SceneAction::Update, gd);
        self.scheduler.execute(gd);
        self.scheduler.execute(gd);
        self.scenes.apply_scene_action(SceneAction::LateUpdate, gd);
        gd.set_current_thread(0);
        self.scenes.apply_scene_action(SceneAction::EndFrame, gd);
        self.renderer.render_frame(&gd.motion_manager)?;
        gd.inputs_manager.frame_reset();
        Ok(self.renderer.framebuffer())
    }
}
