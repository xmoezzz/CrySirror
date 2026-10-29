// SPDX-License-Identifier: MPL-2.0
//! RFVP (FVP/HCB) over CrySirror services.
//!
//! The original interpreter and its native systems (syscalls, prim tree,
//! motions, text, sound slots, saves) run in the vendored portable rfvp core
//! (`crates/rfvp-upstream`, xmoezzz/rfvp 062fec8, `no_std` profile). This
//! adapter is its host: files come from the client's capability storage,
//! textures are engine images, each frame's sprites are one engine dynamic
//! puppet mesh on a single engine layer (drawn by the engine renderer), sound
//! slots are engine channel audio, time is the engine clock, saves go through
//! the save mount. No pixels are composed here.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{AudioParameters, AudioRequest, ClientStorage, EncodedAudio, HostServices, InputFrame, buttons},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_puppet::{FramePart, PartBlend, PuppetFrame};
use crysirror_runtime::{AdapterRegistry, model::ClientSpec, runtime::VmFactory};
use rfvp::host_api::{
    AudioParams, AudioStreamDesc, AudioStreamId, BlendMode, ColorRgba, DrawSolidCommand, DrawSpriteCommand,
    EncodedAudioKind, InputModifiers, KeyCode, PixelFormat, PointerButton, RfvpAudio, RfvpClock, RfvpError,
    RfvpEvent, RfvpFile, RfvpFileInfo, RfvpFileSystem, RfvpHost, RfvpLogLevel, RfvpRenderer, RfvpResult,
    RfvpStorage, TextureDesc, TextureId, TextureRect, WallTime,
};
use rfvp::{RfvpBootConfig, RfvpCore, RfvpCoreConfig, RfvpCoreRunState};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::Arc,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Nls {
    #[default]
    Sjis,
    Gbk,
    Utf8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Game directory inside the asset mount ("" = the mount root).
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub nls: Nls,
    /// Asset path served as the core's required `default.ttf`.
    pub default_font: String,
    /// Virtual screen size; the HCB header's size when absent.
    #[serde(default)]
    pub screen: Option<[u32; 2]>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        for p in [&self.default_font].into_iter().chain((!self.root.is_empty()).then_some(&self.root)) {
            ensure!(VfsPath::new(Mount::Asset, p)?.relative() == p.as_str(), "noncanonical RFVP asset path `{p}`");
        }
        if let Some([w, h]) = self.screen {
            ensure!((1..=8192).contains(&w) && (1..=8192).contains(&h), "RFVP screen size");
        }
        Ok(())
    }
}

struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(spec.program.is_empty(), "RFVP runs the original HCB, not data instructions");
        Ok(Box::new(RfvpVm::new(client, serde_json::from_value(spec.config.clone())?)?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("rfvp", Arc::new(Factory))
}

/// Engine layer that carries the frame mesh.
const SCREEN_LAYER: u32 = 1;
/// Surface slot of the 1x1 white texture solids are drawn with.
const WHITE: u32 = 0x00FF_FFFF;

struct Texture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    path: Option<String>,
    dirty: bool,
}

/// State the core reaches through the host while it ticks.
struct HostState {
    client: ClientId,
    root: String,
    default_font: String,
    storage: Option<Arc<dyn ClientStorage>>,
    now_us: u64,
    textures: BTreeMap<u32, Texture>,
    parts: Vec<FramePart>,
    part_textures: Vec<String>,
    texture_index: BTreeMap<String, u32>,
    frame_size: [u32; 2],
    presented: bool,
    released: Vec<u32>,
    sounds: BTreeMap<u32, (Arc<[u8]>, AudioParams)>,
    audio: Vec<AudioRequest>,
    errors: Vec<String>,
    log: Vec<String>,
}

pub struct RfvpVm {
    config: Config,
    core: RfvpCore,
    state: HostState,
    booted: bool,
    last_down: u64,
    last_pointer: [i32; 2],
    frames: u64,
}

impl RfvpVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        let [w, h] = config.screen.unwrap_or([1024, 640]);
        Ok(Self {
            core: RfvpCore::new(RfvpCoreConfig { virtual_width: w, virtual_height: h, max_pending_events: 256 }),
            state: HostState {
                client,
                root: config.root.trim_matches('/').to_owned(),
                default_font: config.default_font.clone(),
                storage: None,
                now_us: 0,
                textures: BTreeMap::new(),
                parts: Vec::new(),
                part_textures: Vec::new(),
                texture_index: BTreeMap::new(),
                frame_size: [w, h],
                presented: false,
                released: Vec::new(),
                sounds: BTreeMap::new(),
                audio: Vec::new(),
                errors: Vec::new(),
                log: Vec::new(),
            },
            config,
            booted: false,
            last_down: 0,
            last_pointer: [-1, -1],
            frames: 0,
        })
    }

    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        self.state.storage = Some(host.storage(self.state.client)?);
        let layer = serde_json::json!({"layers": [{"op": "put", "layer": {
            "id": SCREEN_LAYER, "rect": [0, 0, self.state.frame_size[0], self.state.frame_size[1]],
            "paint": {"kind": "solid", "color": [0, 0, 0, 0]}}}]});
        host.submit_engine(self.state.client, 1, &serde_json::to_vec(&layer)?)?;
        let nls = match self.config.nls {
            Nls::Sjis => rfvp::script::parser::Nls::ShiftJIS,
            Nls::Gbk => rfvp::script::parser::Nls::GBK,
            Nls::Utf8 => rfvp::script::parser::Nls::UTF8,
        };
        let mut h = TickHost { st: &mut self.state };
        let boot = RfvpBootConfig { asset_root: ".", nls, ..RfvpBootConfig::default() };
        let result = self.core.boot(&mut h, boot);
        if let Err(e) = result {
            let detail = self.core.last_error_detail().unwrap_or_default().to_owned();
            anyhow::bail!("RFVP boot failed: {e:?} {detail}");
        }
        ensure!(self.core.run_state() == RfvpCoreRunState::Booted, "RFVP core did not boot");
        self.booted = true;
        Ok(())
    }

    fn push_input(&mut self, input: InputFrame) -> Result<()> {
        let pointer = [input.pointer_x, input.pointer_y];
        let in_screen = pointer[0] >= 0
            && pointer[1] >= 0
            && (pointer[0] as u32) < self.state.frame_size[0]
            && (pointer[1] as u32) < self.state.frame_size[1];
        let mut events = Vec::new();
        if pointer != self.last_pointer {
            events.push(RfvpEvent::PointerMove { x: pointer[0], y: pointer[1], in_screen });
            self.last_pointer = pointer;
        }
        let down = input.buttons_down | input.buttons_pressed;
        let pressed = (down & !self.last_down) | input.buttons_pressed;
        let released = self.last_down & !input.buttons_down;
        let modifiers = if down & buttons::CTRL != 0 { InputModifiers::CONTROL } else { InputModifiers::empty() };
        for (bit, button) in [(buttons::MOUSE_LEFT, PointerButton::Left), (buttons::MOUSE_RIGHT, PointerButton::Right)] {
            if pressed & bit != 0 {
                events.push(RfvpEvent::PointerDown { button, x: pointer[0], y: pointer[1] });
            }
            if released & bit != 0 {
                events.push(RfvpEvent::PointerUp { button, x: pointer[0], y: pointer[1] });
            }
        }
        for (bit, key) in [
            (buttons::ENTER, KeyCode::Return),
            (buttons::SPACE, KeyCode::Space),
            (buttons::ESCAPE, KeyCode::Escape),
            (buttons::LEFT, KeyCode::Left),
            (buttons::RIGHT, KeyCode::Right),
            (buttons::UP, KeyCode::Up),
            (buttons::DOWN, KeyCode::Down),
            (buttons::CTRL, KeyCode::Control),
            (buttons::SHIFT, KeyCode::Shift),
            (buttons::TAB, KeyCode::Tab),
            (buttons::BACKSPACE, KeyCode::Backspace),
        ] {
            if pressed & bit != 0 {
                events.push(RfvpEvent::KeyDown { key, repeat: false, modifiers });
            }
            if released & bit != 0 {
                events.push(RfvpEvent::KeyUp { key, modifiers });
            }
        }
        self.last_down = input.buttons_down;
        for event in events {
            self.core.push_event(event).map_err(|e| anyhow::anyhow!("RFVP input: {e:?}"))?;
        }
        Ok(())
    }

    fn run(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        self.state.now_us = host.now_ms().saturating_mul(1000);
        if !self.booted {
            self.boot(host)?;
        }
        self.push_input(input)?;
        self.state.parts.clear();
        self.state.part_textures.clear();
        self.state.texture_index.clear();
        self.state.presented = false;
        {
            let mut h = TickHost { st: &mut self.state };
            if let Err(e) = self.core.tick(&mut h) {
                let detail = self.core.last_error_detail().unwrap_or_default().to_owned();
                anyhow::bail!("RFVP tick failed: {e:?} {detail}");
            }
        }
        if let Some(e) = self.state.errors.first() {
            anyhow::bail!("RFVP host: {e}");
        }
        for request in std::mem::take(&mut self.state.audio) {
            host.submit_audio(self.state.client, request)?;
        }
        for slot in std::mem::take(&mut self.state.released) {
            if !self.state.textures.contains_key(&slot) {
                host.release_surface(self.state.client, slot)?;
            }
        }
        if self.state.presented {
            self.publish(host)?;
        }
        self.frames += 1;
        if self.core.quit_requested() {
            return Ok(VmYield::Halted);
        }
        Ok(VmYield::BudgetExhausted)
    }

    /// Upload changed textures and present the frame mesh.
    fn publish(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let client = self.state.client;
        for (id, t) in &mut self.state.textures {
            if t.dirty {
                let path = host.present_surface(client, *id, t.width, t.height, &t.rgba)?;
                t.path = Some(path.relative().to_owned());
                t.dirty = false;
            }
        }
        let mut textures = Vec::with_capacity(self.state.part_textures.len());
        for key in &self.state.part_textures {
            let id: u32 = key.parse().context("RFVP texture key")?;
            let path = self.state.textures.get(&id).and_then(|t| t.path.clone()).context("RFVP texture not uploaded")?;
            textures.push(path);
        }
        let [w, h] = self.state.frame_size;
        let frame = PuppetFrame { size: [w as f32, h as f32], parts: std::mem::take(&mut self.state.parts), textures };
        host.present_puppet_frame(client, SCREEN_LAYER, &serde_json::to_vec(&frame)?)?;
        Ok(())
    }
}

impl VmAdapter for RfvpVm {
    fn engine_id(&self) -> &'static str {
        "rfvp"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(&mut self, host: &mut dyn HostServices, input: InputFrame, budget: u32) -> crysirror_core::Result<VmYield> {
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        self.run(host, input).map_err(|e| Error::Io(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("RFVP runtime snapshot (use the game's own save slots)"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("RFVP runtime snapshot (use the game's own save slots)"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "frames": self.frames,
            "booted": self.booted,
            "textures": self.state.textures.len(),
            "log_tail": self.state.log.iter().rev().take(16).collect::<Vec<_>>(),
        }))
        .unwrap_or_default()
    }
}

/// The host the core sees during one call.
struct TickHost<'a> {
    st: &'a mut HostState,
}

impl HostState {
    /// Game-relative path to an asset path; `default.ttf` is the configured font.
    fn asset(&self, path: &str) -> RfvpResult<VfsPath> {
        let path = path.replace('\\', "/");
        let path = path.trim_start_matches("./").trim_start_matches('/');
        let path = if path.eq_ignore_ascii_case("default.ttf") {
            self.default_font.clone()
        } else if self.root.is_empty() {
            path.to_owned()
        } else {
            format!("{}/{path}", self.root)
        };
        VfsPath::new(Mount::Asset, &path).map_err(|_| RfvpError::InvalidArgument)
    }
    fn storage(&self) -> RfvpResult<&Arc<dyn ClientStorage>> {
        self.storage.as_ref().ok_or(RfvpError::Unsupported)
    }
}

/// A file opened from the capability storage.
pub struct AssetFile(Arc<dyn crysirror_core::service::AssetReader>);
impl RfvpFile for AssetFile {
    fn len(&mut self) -> RfvpResult<u64> {
        self.0.len().map_err(|_| RfvpError::Io)
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> RfvpResult<usize> {
        self.0.read_at(offset, buf).map_err(|_| RfvpError::Io)
    }
}

/// Saves under the save mount (`rfvp/<game path>`).
struct SaveStorage(Arc<dyn ClientStorage>);
impl SaveStorage {
    fn path(path: &str) -> RfvpResult<VfsPath> {
        let path = path.replace('\\', "/");
        let path = path.trim_start_matches("./").trim_start_matches('/');
        VfsPath::new(Mount::Save, &format!("rfvp/{path}")).map_err(|_| RfvpError::InvalidArgument)
    }
}
impl RfvpStorage for SaveStorage {
    fn read(&self, path: &str) -> RfvpResult<Vec<u8>> {
        self.0.read(&Self::path(path)?).map_err(|_| RfvpError::NotFound)
    }
    fn write(&self, path: &str, bytes: &[u8]) -> RfvpResult<()> {
        self.0.write_atomic(&Self::path(path)?, bytes).map_err(|_| RfvpError::Io)
    }
    fn exists(&self, path: &str) -> bool {
        Self::path(path).is_ok_and(|p| self.0.exists(&p))
    }
    fn remove(&self, path: &str) -> RfvpResult<()> {
        self.0.remove(&Self::path(path)?).map_err(|_| RfvpError::Io)
    }
    fn list(&self, dir: &str) -> RfvpResult<Vec<String>> {
        let dir = dir.replace('\\', "/");
        let dir = dir.trim_matches('/');
        let rel = if dir.is_empty() { "rfvp".to_owned() } else { format!("rfvp/{dir}") };
        self.0.list(Mount::Save, &rel).map_err(|_| RfvpError::NotFound)
    }
}

impl RfvpFileSystem for TickHost<'_> {
    type File = AssetFile;
    fn open(&mut self, path: &str) -> RfvpResult<AssetFile> {
        let p = self.st.asset(path)?;
        let reader = self.st.storage()?.open_asset(&p).map_err(|_| RfvpError::NotFound)?;
        Ok(AssetFile(reader))
    }
    fn storage(&mut self) -> Option<Arc<dyn RfvpStorage>> {
        self.st.storage.clone().map(|s| Arc::new(SaveStorage(s)) as Arc<dyn RfvpStorage>)
    }
    fn metadata(&mut self, path: &str) -> RfvpResult<RfvpFileInfo> {
        let p = self.st.asset(path)?;
        let storage = self.st.storage()?;
        let meta = storage.metadata(Mount::Asset, p.relative()).map_err(|_| RfvpError::Io)?.ok_or(RfvpError::NotFound)?;
        Ok(if meta.is_directory {
            RfvpFileInfo { len: 0, kind: rfvp::host_api::RfvpFileKind::Directory }
        } else {
            RfvpFileInfo::file(meta.size.unwrap_or(0))
        })
    }
    fn enumerate_by_extension(
        &mut self,
        root: &str,
        extension_without_dot: &str,
        visitor: &mut dyn FnMut(&str, RfvpFileInfo) -> RfvpResult<()>,
    ) -> RfvpResult<()> {
        let dir = self.st.asset(if root.is_empty() || root == "." { "." } else { root });
        let dir = match dir {
            Ok(d) => d.relative().to_owned(),
            Err(_) => self.st.root.clone(),
        };
        let dir = if root.is_empty() || root == "." { self.st.root.clone() } else { dir };
        let storage = self.st.storage()?.clone();
        let names = storage.list(Mount::Asset, &dir).map_err(|_| RfvpError::NotFound)?;
        let ext = format!(".{}", extension_without_dot.to_ascii_lowercase());
        for name in names {
            if name.to_ascii_lowercase().ends_with(&ext) {
                let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
                let len = storage
                    .metadata(Mount::Asset, &rel)
                    .ok()
                    .flatten()
                    .and_then(|m| m.size)
                    .unwrap_or(0);
                visitor(&name, RfvpFileInfo::file(len))?;
            }
        }
        Ok(())
    }
}

fn to_rgba(desc: &TextureDesc, pixels: &[u8]) -> RfvpResult<Vec<u8>> {
    let n = desc.width as usize * desc.height as usize;
    let (stride, convert): (usize, fn(&[u8]) -> [u8; 4]) = match desc.format {
        PixelFormat::Rgba8 => (4, |p| [p[0], p[1], p[2], p[3]]),
        PixelFormat::Bgra8 => (4, |p| [p[2], p[1], p[0], p[3]]),
        PixelFormat::Rgb8 => (3, |p| [p[0], p[1], p[2], 255]),
        PixelFormat::Luma8 => (1, |p| [p[0], p[0], p[0], 255]),
        PixelFormat::LumaA8 => (2, |p| [p[0], p[0], p[0], p[1]]),
        PixelFormat::Alpha8 => (1, |p| [255, 255, 255, p[0]]),
        #[allow(unreachable_patterns)]
        _ => return Err(RfvpError::Unsupported),
    };
    if pixels.len() < n * stride {
        return Err(RfvpError::InvalidArgument);
    }
    let mut out = Vec::with_capacity(n * 4);
    for px in pixels.chunks_exact(stride).take(n) {
        out.extend_from_slice(&convert(px));
    }
    Ok(out)
}

impl RfvpRenderer for TickHost<'_> {
    fn create_texture(&mut self, id: TextureId, desc: TextureDesc, pixels: Option<&[u8]>) -> RfvpResult<()> {
        if !(1..=8192).contains(&desc.width) || !(1..=8192).contains(&desc.height) {
            return Err(RfvpError::InvalidArgument);
        }
        let rgba = match pixels {
            Some(p) => to_rgba(&desc, p)?,
            None => vec![0; desc.width as usize * desc.height as usize * 4],
        };
        self.st.textures.insert(id.0, Texture { width: desc.width, height: desc.height, rgba, path: None, dirty: true });
        Ok(())
    }
    fn update_texture(&mut self, id: TextureId, rect: TextureRect, pixels: &[u8]) -> RfvpResult<()> {
        let t = self.st.textures.get_mut(&id.0).ok_or(RfvpError::NotFound)?;
        if rect.x + rect.width > t.width || rect.y + rect.height > t.height {
            return Err(RfvpError::InvalidArgument);
        }
        let desc = TextureDesc { width: rect.width, height: rect.height, format: PixelFormat::Rgba8, mip_count: 1 };
        // Updates arrive in the texture's own format; the core creates
        // updatable textures as RGBA8.
        let rgba = to_rgba(&desc, pixels)?;
        let row = rect.width as usize * 4;
        for y in 0..rect.height as usize {
            let dst = ((rect.y as usize + y) * t.width as usize + rect.x as usize) * 4;
            t.rgba[dst..dst + row].copy_from_slice(&rgba[y * row..y * row + row]);
        }
        t.dirty = true;
        Ok(())
    }
    fn destroy_texture(&mut self, id: TextureId) {
        if self.st.textures.remove(&id.0).is_some_and(|t| t.path.is_some()) {
            self.st.released.push(id.0);
        }
    }
    fn begin_frame(&mut self, width: u32, height: u32, clear: Option<ColorRgba>) -> RfvpResult<()> {
        self.st.parts.clear();
        self.st.part_textures.clear();
        self.st.texture_index.clear();
        if [width, height] != self.st.frame_size {
            self.st.errors.push(format!("RFVP frame size {width}x{height} differs from the layer"));
        }
        if let Some(c) = clear {
            if c.a > 0. {
                self.draw_solid(&DrawSolidCommand {
                    rect: rfvp::host_api::RectI32 { x: 0, y: 0, width: width as i32, height: height as i32 },
                    color: c,
                    blend: BlendMode::Opaque,
                    scissor: None,
                })?;
            }
        }
        Ok(())
    }
    fn draw_sprite(&mut self, c: &DrawSpriteCommand) -> RfvpResult<()> {
        self.push_quad(c.texture.0, c.vertices.map(|v| v.position), c.vertices.map(|v| v.tex_coord),
            c.vertices.map(|v| [v.color.r, v.color.g, v.color.b, v.color.a]), c.blend, c.scissor)
    }
    fn draw_solid(&mut self, c: &DrawSolidCommand) -> RfvpResult<()> {
        if !self.st.textures.contains_key(&WHITE) {
            self.st.textures.insert(WHITE, Texture { width: 1, height: 1, rgba: vec![255; 4], path: None, dirty: true });
        }
        let (x0, y0) = (c.rect.x as f32, c.rect.y as f32);
        let (x1, y1) = (x0 + c.rect.width as f32, y0 + c.rect.height as f32);
        let color = [c.color.r, c.color.g, c.color.b, c.color.a];
        self.push_quad(WHITE, [[x0, y1], [x0, y0], [x1, y1], [x1, y0]], [[0., 1.], [0., 0.], [1., 1.], [1., 0.]], [color; 4], c.blend, c.scissor)
    }
    fn end_frame(&mut self) -> RfvpResult<()> {
        Ok(())
    }
    fn present(&mut self) -> RfvpResult<()> {
        self.st.presented = true;
        Ok(())
    }
}

impl TickHost<'_> {
    fn push_quad(
        &mut self,
        texture: u32,
        positions: [[f32; 2]; 4],
        uvs: [[f32; 2]; 4],
        colors: [[f32; 4]; 4],
        blend: BlendMode,
        scissor: Option<rfvp::host_api::RectI32>,
    ) -> RfvpResult<()> {
        if !self.st.textures.contains_key(&texture) {
            return Err(RfvpError::NotFound);
        }
        if self.st.parts.len() >= crysirror_puppet::MAX_PARTS {
            self.st.errors.push("RFVP frame exceeds the engine mesh part limit".into());
            return Ok(());
        }
        let key = texture.to_string();
        let index = match self.st.texture_index.get(&key) {
            Some(i) => *i,
            None => {
                let i = self.st.part_textures.len() as u32;
                self.st.part_textures.push(key.clone());
                self.st.texture_index.insert(key, i);
                i
            }
        };
        let (blend, colors) = match blend {
            BlendMode::Alpha => (PartBlend::Normal, colors),
            // Opaque draws ignore source alpha: full vertex alpha (texture
            // alpha of opaque art is 255).
            BlendMode::Opaque => (PartBlend::Normal, colors.map(|c| [c[0], c[1], c[2], 1.])),
            BlendMode::Add => (PartBlend::Add, colors),
            BlendMode::Multiply => (PartBlend::Multiply, colors),
            BlendMode::Screen => (PartBlend::Screen, colors),
        };
        let mut part = FramePart {
            id: format!("p{}", self.st.parts.len()),
            texture: index,
            positions: positions.to_vec(),
            uvs: uvs.to_vec(),
            indices: vec![0, 1, 2, 2, 1, 3],
            colors: colors.to_vec(),
            blend,
            masks: Vec::new(),
            invert_mask: false,
            visible: true,
        };
        if let Some(s) = scissor {
            let rect = [s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32];
            if crysirror_puppet::clip_part_rect(&mut part, rect).is_err() {
                return Ok(());
            }
            if part.indices.is_empty() {
                return Ok(());
            }
        }
        self.st.parts.push(part);
        Ok(())
    }
}

impl RfvpAudio for TickHost<'_> {
    fn load_encoded(&mut self, id: AudioStreamId, _kind: EncodedAudioKind, bytes: &[u8]) -> RfvpResult<()> {
        self.st.sounds.insert(id.0, (Arc::from(bytes), AudioParams::default()));
        Ok(())
    }
    fn create_stream(&mut self, _: AudioStreamId, _: AudioStreamDesc) -> RfvpResult<()> {
        Err(RfvpError::Unsupported)
    }
    fn submit_i16(&mut self, _: AudioStreamId, _: &[i16]) -> RfvpResult<()> {
        Err(RfvpError::Unsupported)
    }
    fn submit_f32(&mut self, _: AudioStreamId, _: &[f32]) -> RfvpResult<()> {
        Err(RfvpError::Unsupported)
    }
    fn play(&mut self, id: AudioStreamId, params: AudioParams, fade_in_ms: u32) -> RfvpResult<()> {
        let (bytes, stored) = self.st.sounds.get_mut(&id.0).ok_or(RfvpError::NotFound)?;
        *stored = params;
        let parameters = AudioParameters {
            gain: params.volume.clamp(0., 4.),
            pan: ((params.pan + 1.) / 2.).clamp(0., 1.),
            looping: params.repeat,
            fade_ms: fade_in_ms as f32,
            restart: true,
            ..AudioParameters::default()
        };
        self.st.audio.push(AudioRequest::Play {
            channel: id.0,
            source: EncodedAudio { bytes: bytes.clone(), frame_limit: None },
            intro: None,
            parameters,
        });
        Ok(())
    }
    fn stop(&mut self, id: AudioStreamId, fade_ms: u32) -> RfvpResult<()> {
        self.st.audio.push(AudioRequest::Stop { channel: id.0, fade_ms: fade_ms as f32 });
        Ok(())
    }
    fn pause(&mut self, id: AudioStreamId) -> RfvpResult<()> {
        self.st.audio.push(AudioRequest::Pause { channel: id.0, paused: true, fade_ms: 0. });
        Ok(())
    }
    fn resume(&mut self, id: AudioStreamId) -> RfvpResult<()> {
        self.st.audio.push(AudioRequest::Pause { channel: id.0, paused: false, fade_ms: 0. });
        Ok(())
    }
    fn set_params(&mut self, id: AudioStreamId, params: AudioParams) -> RfvpResult<()> {
        if let Some((_, stored)) = self.st.sounds.get_mut(&id.0) {
            *stored = params;
        }
        self.st.audio.push(AudioRequest::SetGain { channel: id.0, gain: params.volume.clamp(0., 4.), fade_ms: 0. });
        self.st.audio.push(AudioRequest::SetPan { channel: id.0, pan: ((params.pan + 1.) / 2.).clamp(0., 1.), fade_ms: 0. });
        Ok(())
    }
    fn set_master_volume(&mut self, _volume: f32) -> RfvpResult<()> {
        Ok(())
    }
    fn destroy_stream(&mut self, id: AudioStreamId) {
        self.st.sounds.remove(&id.0);
    }
    fn tick(&mut self, _delta_us: u64) -> RfvpResult<()> {
        Ok(())
    }
}

impl RfvpClock for TickHost<'_> {
    fn ticks_us(&mut self) -> u64 {
        self.st.now_us
    }
    /// Local calendar time, shown next to saves.
    fn wall_time(&mut self) -> Option<WallTime> {
        use chrono::{Datelike, Timelike};
        let now = chrono::Local::now();
        Some(WallTime {
            year: u16::try_from(now.year()).ok()?,
            month: now.month() as u8,
            day: now.day() as u8,
            day_of_week: now.weekday().num_days_from_sunday() as u8,
            hour: now.hour() as u8,
            minute: now.minute() as u8,
        })
    }
}

impl RfvpHost for TickHost<'_> {
    type FileSystem = Self;
    type Renderer = Self;
    type Audio = Self;
    type Clock = Self;
    fn fs(&mut self) -> &mut Self {
        self
    }
    fn renderer(&mut self) -> &mut Self {
        self
    }
    fn audio(&mut self) -> &mut Self {
        self
    }
    fn clock(&mut self) -> &mut Self {
        self
    }
    fn log(&mut self, level: RfvpLogLevel, message: &str) {
        if matches!(level, RfvpLogLevel::Error | RfvpLogLevel::Warn | RfvpLogLevel::Info) {
            if std::env::var_os("CRYSIRROR_RFVP_LOG").is_some() {
                eprintln!("[rfvp {level:?}] {message}");
            }
            if self.st.log.len() >= 256 {
                self.st.log.remove(0);
            }
            self.st.log.push(format!("{level:?}: {message}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(format: PixelFormat) -> TextureDesc {
        TextureDesc { width: 2, height: 1, format, mip_count: 1 }
    }

    #[test]
    fn pixel_formats_convert_to_rgba() {
        assert_eq!(to_rgba(&desc(PixelFormat::Bgra8), &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap(), [3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(to_rgba(&desc(PixelFormat::Rgb8), &[1, 2, 3, 4, 5, 6]).unwrap(), [1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(to_rgba(&desc(PixelFormat::Alpha8), &[9, 10]).unwrap(), [255, 255, 255, 9, 255, 255, 255, 10]);
        assert_eq!(to_rgba(&desc(PixelFormat::Rgba8), &[0; 7]), Err(RfvpError::InvalidArgument));
    }

    #[test]
    fn config_rejects_noncanonical_paths_and_sizes() {
        let ok = Config { root: String::new(), nls: Nls::Sjis, default_font: "font/a.ttf".into(), screen: Some([1024, 640]) };
        ok.validate().unwrap();
        assert!(Config { default_font: "../a.ttf".into(), ..ok.clone() }.validate().is_err());
        assert!(Config { screen: Some([0, 640]), ..ok }.validate().is_err());
    }

    #[test]
    fn sprites_become_engine_mesh_parts() {
        let mut st = RfvpVm::new(ClientId(1), Config {
            root: String::new(),
            nls: Nls::Sjis,
            default_font: "f.ttf".into(),
            screen: Some([64, 32]),
        })
        .unwrap()
        .state;
        let mut h = TickHost { st: &mut st };
        h.create_texture(TextureId(7), desc(PixelFormat::Rgba8), Some(&[255; 8])).unwrap();
        h.begin_frame(64, 32, Some(ColorRgba::BLACK)).unwrap();
        let v = |x: f32, y: f32| rfvp::host_api::Vertex2D { position: [x, y], tex_coord: [0., 0.], color: ColorRgba::BLACK };
        h.draw_sprite(&DrawSpriteCommand {
            texture: TextureId(7),
            vertices: [v(0., 10.), v(0., 0.), v(10., 10.), v(10., 0.)],
            blend: BlendMode::Add,
            filter: rfvp::host_api::TextureFilter::Linear,
            scissor: Some(rfvp::host_api::RectI32 { x: 0, y: 0, width: 5, height: 20 }),
        })
        .unwrap();
        // Clear solid + the clipped sprite, each referencing its own texture.
        assert_eq!(st.parts.len(), 2);
        assert_eq!(st.part_textures, [WHITE.to_string(), "7".to_string()]);
        assert_eq!(st.parts[1].blend, PartBlend::Add);
        assert!(st.parts[1].positions.iter().all(|p| p[0] <= 5.));
        let mut h = TickHost { st: &mut st };
        assert_eq!(h.draw_sprite(&DrawSpriteCommand {
            texture: TextureId(99),
            vertices: [v(0., 0.); 4],
            blend: BlendMode::Alpha,
            filter: rfvp::host_api::TextureFilter::Linear,
            scissor: None,
        }), Err(RfvpError::NotFound));
    }
}
