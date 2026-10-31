// SPDX-License-Identifier: MPL-2.0
//! The VM-state part of the original `System` (settings, system commands,
//! RNG, default image names, DLL slots). Images, text, sound and windows are
//! engine services driven by the adapter, not fields here.
use crate::vm::gameexe::Gameexe;
use crate::vm::settings::{Settings, Syscom};

#[derive(Debug, Clone)]
pub struct Sys {
    pub settings: Settings,
    pub defaults: Settings,
    pub syscom: Syscom,
    pub rng: u64,
    pub default_grp: String,
    pub default_bgr: String,
    /// Set when a volume or mute setting changed; the adapter applies it.
    pub volumes_changed: bool,
    pub dlls: crysirror_reallive_dll::Dlls,
    /// Retained objects (state only; the adapter projects them to layers).
    pub gfx: crate::vm::graphics::Gfx,
    /// Engine clock in ms, set by the adapter before each step.
    pub clock_ms: u64,
    /// Skip / fast-forward in effect (Ctrl, skip mode).
    pub fast_forward: bool,
    pub input: Input,
    pub frames: crate::vm::clock::FrameCounters,
    pub backlog_enabled: bool,
    pub read_jump_cancel: bool,
    pub ctrl_key_skip: bool,
    pub key_cursor: i32,
    pub mouse_cursor: i32,
    pub misc_flags: Vec<i32>,
    pub system_menu_animation: bool,
}

/// Clicks the adapter received and VM waits may consume.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub clicks: Vec<(i32, i32)>,
    pub key_mouse: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl Input {
    pub fn take_click(&mut self) -> Option<(i32, i32)> {
        (!self.clicks.is_empty()).then(|| self.clicks.remove(0))
    }
}

impl Sys {
    pub fn new(gameexe: &Gameexe) -> Self {
        let settings = Settings::from_gameexe(gameexe);
        Self {
            defaults: settings.clone(),
            settings,
            syscom: Syscom::from_gameexe(gameexe),
            // Upstream's deterministic (virtual clock) seed; the adapter runs
            // on the engine clock, so replays stay reproducible.
            rng: 0x853c_49e6_748f_ea9b,
            default_grp: String::new(),
            default_bgr: String::new(),
            volumes_changed: false,
            dlls: Default::default(),
            gfx: Default::default(),
            clock_ms: 0,
            fast_forward: false,
            input: Input::default(),
            frames: Default::default(),
            backlog_enabled: true,
            read_jump_cancel: false,
            ctrl_key_skip: true,
            key_cursor: 0,
            mouse_cursor: 0,
            misc_flags: Vec::new(),
            system_menu_animation: true,
        }
    }

    pub fn now(&self) -> u64 {
        self.clock_ms
    }

    pub fn should_fast_forward(&self) -> bool {
        self.fast_forward
    }

    // Copied from siglus_rs `System::random` (5f358ef).
    pub fn random(&mut self, min: i32, max: i32) -> i32 {
        let (low, high) = if min <= max { (min, max) } else { (max, min) };
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let span = (i64::from(high) - i64::from(low) + 1) as u64;
        (i64::from(low) + (z % span) as i64) as i32
    }
}
