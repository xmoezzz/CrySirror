// SPDX-License-Identifier: MPL-2.0
//! Original AVG32 interpreter (vendored in [`vm`]) running an installed game
//! through CrySirror services:
//!
//! - PDT buffers, copies and transitions: the shared `crysirror_image::avg32`
//!   raster device; the presented screen is one engine surface layer.
//! - Message/choice text, menus, backlog and name entry: retained engine
//!   GlyphRun and solid layers (no private font rasteriser).
//! - BGM, effects and voices: engine audio channels (`AudioRequest`).
//! - Files and the original `SAVE.INI`: client-scoped capability storage.
#![forbid(unsafe_code)]
pub mod vm;

use anyhow::{Context, Result, ensure};
use crysirror_core::{
    Error,
    service::{
        AudioParameters, AudioRequest, EncodedAudio, HostServices, InputFrame, PcmBus, buttons,
    },
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_scene::layer::PositionedGlyph;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc};
use vm::{
    engine::Avg32Engine,
    nls::Nls,
    sound::{AudioCommand, CHANNEL_BGM, CHANNEL_KOE, CHANNEL_SE},
    system::{HostRequest, Key},
    text::TextGlyph,
};

fn encoding() -> String {
    "sjis".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Asset-relative directory of the installed game (holds `Gameexe.ini`).
    pub root: String,
    /// Project font alias for message, choice and menu text.
    pub font: String,
    /// Native string codec (`sjis`, `gbk`, `big5`, `utf8`).
    #[serde(default = "encoding")]
    pub encoding: String,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.font.is_empty() && self.font.len() <= 256,
            "AVG32 font alias"
        );
        ensure!(self.root.len() <= 1024, "AVG32 game root");
        self.encoding.parse::<Nls>()?;
        Ok(())
    }
}

/// Client-local engine layer ids.
const SCREEN: u32 = 1;
const TEXT: u32 = 2;
const OVERLAY_TEXT: u32 = 3;
const OVERLAY_RECT: u32 = 100;
const MAX_OVERLAY_RECTS: usize = 256;
/// Dynamic surface slot holding the presented PDT screen.
const SCREEN_SLOT: u32 = 0x4156_0000;
const WARNINGS: usize = 64;

pub struct Avg32Vm {
    client: ClientId,
    config: Config,
    nls: Nls,
    storage: Option<vm::fs::Storage>,
    engine: Option<Avg32Engine>,
    last_buttons: u64,
    screen_revision: Option<u64>,
    text_revision: Option<u64>,
    shake: (i32, i32),
    overlay: vm::menu::Overlay,
    overlay_rects: usize,
    title: String,
    warnings: VecDeque<String>,
    api_calls: u64,
    frames: u64,
    audio_requests: u64,
    recent_audio: VecDeque<String>,
}

fn error(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}

fn glyphs(source: &[TextGlyph], base: f32) -> Vec<PositionedGlyph> {
    source
        .iter()
        .map(|g| PositionedGlyph {
            character: g.character,
            baseline: [g.x as f32, g.baseline as f32],
            size: (g.size as f32 != base).then_some(g.size as f32),
            color: Some([g.colour[0], g.colour[1], g.colour[2], 255]),
            bold: None,
        })
        .collect()
}

impl Avg32Vm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            nls: config.encoding.parse()?,
            config,
            storage: None,
            engine: None,
            last_buttons: 0,
            screen_revision: None,
            text_revision: None,
            shake: (0, 0),
            overlay: Default::default(),
            overlay_rects: 0,
            title: String::new(),
            warnings: VecDeque::new(),
            api_calls: 0,
            frames: 0,
            audio_requests: 0,
            recent_audio: VecDeque::new(),
        })
    }

    fn submit(&mut self, host: &mut dyn HostServices, batch: Value) -> Result<()> {
        host.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        self.api_calls += 1;
        Ok(())
    }

    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let storage = vm::fs::Storage {
            storage: host.storage(self.client).map_err(|e| anyhow::anyhow!("{e}"))?,
            root: self.config.root.trim_matches('/').to_owned(),
        };
        let now = host.now_ms();
        let engine = vm::fs::with(&storage, || Avg32Engine::open(self.nls, now))
            .context("open AVG32 game")?;
        self.storage = Some(storage);
        self.engine = Some(engine);
        Ok(())
    }

    fn input(&mut self, input: &InputFrame) {
        let engine = self.engine.as_mut().expect("booted");
        let pressed = (input.buttons_down & !self.last_buttons) | input.buttons_pressed;
        let released = self.last_buttons & !input.buttons_down;
        self.last_buttons = input.buttons_down;
        engine.mouse_move(input.pointer_x, input.pointer_y);
        engine.set_skip(input.buttons_down & (buttons::CTRL | buttons::SHIFT) != 0);
        for (bit, key) in [
            (buttons::ENTER, Key::Enter),
            (buttons::UP, Key::Up),
            (buttons::DOWN, Key::Down),
            (buttons::LEFT, Key::Left),
            (buttons::RIGHT, Key::Right),
            (buttons::ESCAPE, Key::Escape),
            (buttons::SPACE, Key::Space),
            (buttons::PAGE_UP, Key::PageUp),
            (buttons::PAGE_DOWN, Key::PageDown),
            (buttons::BACKSPACE, Key::Backspace),
        ] {
            if pressed & bit != 0 {
                engine.key_down(key);
            }
        }
        // The original reacts to button release (WM_LBUTTONUP / WM_RBUTTONUP).
        if released & buttons::MOUSE_LEFT != 0 {
            engine.mouse_up(false);
        }
        if released & buttons::MOUSE_RIGHT != 0 {
            engine.mouse_up(true);
        }
    }

    fn audio(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let engine = self.engine.as_mut().expect("booted");
        let commands = std::mem::take(&mut engine.system_mut().sound.commands);
        for command in commands {
            let summary = match &command {
                AudioCommand::Play {
                    channel, clip, looped, ..
                } => format!("play {channel} {} bytes looped={looped}", clip.wav.len()),
                AudioCommand::Stop { channel, fade_ms } => format!("stop {channel} fade={fade_ms}"),
                AudioCommand::Gain { channel, volume } => format!("gain {channel} {volume:.2}"),
            };
            if self.recent_audio.len() == WARNINGS {
                self.recent_audio.pop_front();
            }
            self.recent_audio.push_back(summary);
            self.audio_requests += 1;
            let request = match command {
                AudioCommand::Play {
                    channel,
                    clip,
                    looped,
                    fade_in_ms,
                    volume,
                    bus,
                } => {
                    let frame_bytes = vm::sound::frame_bytes(&clip);
                    let loop_start = match (clip.loop_start, frame_bytes) {
                        (Some(bytes), Some(frame)) if frame > 0 => (bytes / frame) as u64,
                        _ => 0,
                    };
                    AudioRequest::Play {
                        channel,
                        source: EncodedAudio {
                            bytes: Arc::from(clip.wav),
                            frame_limit: None,
                        },
                        intro: None,
                        parameters: AudioParameters {
                            gain: volume as f32,
                            looping: looped,
                            loop_start,
                            fade_ms: fade_in_ms as f32,
                            restart: true,
                            bus: match bus {
                                CHANNEL_BGM => PcmBus::Music,
                                CHANNEL_KOE => PcmBus::Voice,
                                CHANNEL_SE => PcmBus::Ui,
                                _ => PcmBus::Sfx,
                            },
                            ..AudioParameters::default()
                        },
                    }
                }
                AudioCommand::Stop { channel, fade_ms } => AudioRequest::Stop {
                    channel,
                    fade_ms: fade_ms as f32,
                },
                AudioCommand::Gain { channel, volume } => AudioRequest::SetGain {
                    channel,
                    gain: volume as f32,
                    fade_ms: 0.,
                },
            };
            host.submit_audio(self.client, request)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            self.api_calls += 1;
        }
        // "Playing" is the engine's channel state.
        let sound = &mut engine.system_mut().sound;
        let active: Vec<u32> = sound.active.iter().copied().collect();
        for channel in active {
            if !host
                .audio_playing(self.client, channel)
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                sound.active.remove(&channel);
            }
        }
        Ok(())
    }

    fn publish(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let engine = self.engine.as_mut().expect("booted");
        let system = engine.system_mut();
        let damage = system.gfx.take_damage();
        system.text.apply_damage(&damage);
        let shake = system.shake_offset();
        let screen_revision = system.gfx.revision;
        let text_revision = system.text.revision;
        let overlay = system.overlay();
        let title = system.window_title.clone();
        let mut layers = Vec::new();
        if self.screen_revision != Some(screen_revision) || self.shake != shake {
            let rgba = system.screen_rgba();
            let path = host
                .present_surface(self.client, SCREEN_SLOT, 640, 480, &rgba)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            layers.push(json!({"op":"put","layer":{"id":SCREEN,"rect":[shake.0,shake.1,640,480],"order":[0,0],"paint":{"kind":"image","asset":path.relative()}}}));
            self.screen_revision = Some(screen_revision);
        }
        let font_size = engine.system().ini().font_size.max(1) as f32;
        if self.text_revision != Some(text_revision) || self.shake != shake {
            let system = engine.system();
            layers.push(json!({"op":"put","layer":{"id":TEXT,"rect":[shake.0,shake.1,640,480],"order":[1,0],"visible":!system.text.glyphs.is_empty(),"paint":{"kind":"glyph_run","font":self.config.font,"size":font_size,"glyphs":glyphs(&system.text.glyphs,font_size)}}}));
            self.text_revision = Some(text_revision);
        }
        self.shake = shake;
        if overlay != self.overlay {
            let rects = overlay.rects.len().min(MAX_OVERLAY_RECTS);
            for (index, (rect, colour, alpha)) in overlay.rects.iter().take(rects).enumerate() {
                let [x1, y1, x2, y2] = *rect;
                layers.push(json!({"op":"put","layer":{"id":OVERLAY_RECT+index as u32,"rect":[x1,y1,x2-x1,y2-y1],"order":[2,index as i32],"paint":{"kind":"solid","color":[colour[0],colour[1],colour[2],*alpha as u8]}}}));
            }
            for index in rects..self.overlay_rects {
                layers.push(json!({"op":"remove","id":OVERLAY_RECT+index as u32}));
            }
            self.overlay_rects = rects;
            layers.push(json!({"op":"put","layer":{"id":OVERLAY_TEXT,"rect":[0,0,640,480],"order":[3,0],"visible":!overlay.glyphs.is_empty(),"paint":{"kind":"glyph_run","font":self.config.font,"size":font_size,"glyphs":glyphs(&overlay.glyphs,font_size)}}}));
            self.overlay = overlay;
        }
        if !layers.is_empty() {
            self.submit(host, json!({ "layers": layers }))?;
        }
        if title != self.title {
            self.submit(host, json!({"ui":[{"op":"set_title","text":title}]}))?;
            self.title = title;
        }
        Ok(())
    }

    fn run(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        vm::nls::set(self.nls);
        if self.engine.is_none() {
            self.boot(host)?;
        }
        let storage = self.storage.clone().expect("booted");
        let now = host.now_ms();
        self.input(&input);
        let engine = self.engine.as_mut().expect("booted");
        engine.system_mut().clock = vm::system::Clock::Virtual(now);
        vm::fs::with(&storage, || engine.tick());
        for warning in engine.take_warnings() {
            if self.warnings.len() == WARNINGS {
                self.warnings.pop_front();
            }
            self.warnings.push_back(warning);
        }
        let quit = engine
            .take_requests()
            .into_iter()
            .any(|request| request == HostRequest::Quit);
        self.audio(host)?;
        self.publish(host)?;
        self.frames += 1;
        if quit || !self.engine.as_ref().expect("booted").running() {
            return Ok(VmYield::Halted);
        }
        // One original frame per engine frame.
        Ok(VmYield::WaitUntil(now.saturating_add(1)))
    }
}

impl VmAdapter for Avg32Vm {
    fn engine_id(&self) -> &'static str {
        "avg32"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        _budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.run(host, input).map_err(|e| error(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "AVG32 keeps its original SAVE.INI slots; framework snapshots are not implemented",
        ))
    }
    fn restore(&mut self, _bytes: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "AVG32 keeps its original SAVE.INI slots; framework snapshots are not implemented",
        ))
    }
    fn request_close(&mut self) -> bool {
        if let (Some(engine), Some(storage)) = (self.engine.as_mut(), self.storage.as_ref()) {
            vm::fs::with(storage, || engine.shutdown());
        }
        false
    }
    fn diagnostics(&self) -> Vec<u8> {
        let engine = self.engine.as_ref();
        serde_json::to_vec(&json!({
            "vm": "original-avg32",
            "source": "siglus_rs 5f358ef1360b0439e92aed2b749e8f25d8721954",
            "owns_framebuffer": false,
            "raster": "crysirror_image::avg32",
            "frames": self.frames,
            "engine_api_calls": self.api_calls,
            "version": engine.map(|e| e.system().version),
            "scene": engine.map(|e| e.scenario().seen),
            "position": engine.map(|e| e.scenario().pos),
            "title": self.title,
            "glyphs": engine.map(|e| e.system().text.glyphs.len()),
            "warnings": self.warnings,
            "audio_requests": self.audio_requests,
            "recent_audio": self.recent_audio,
        }))
        .unwrap_or_default()
    }
}

pub struct Avg32Factory;
impl crysirror_runtime::runtime::VmFactory for Avg32Factory {
    fn create(
        &self,
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "AVG32 executes the game's original TPC32 scenario"
        );
        Ok(Box::new(Avg32Vm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}

pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register("avg32", Arc::new(Avg32Factory))
}
