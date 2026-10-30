// SPDX-License-Identifier: GPL-2.0-or-later
//! First-class bounded NScripter text VM. Rendering, text layout/reveal, images,
//! audio and UI state are CrySirror services; no legacy engine is hosted.
#![forbid(unsafe_code)]
mod native;
mod program;
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        buttons, AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, EngineQuery,
        HostServices, InputFrame, PcmBus,
    },
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use crysirror_runtime::{model::ClientSpec, runtime::VmFactory, AdapterRegistry};
use crysirror_text::grid::{layout_cells, CellCursor, CellLayoutOptions};
use program::{Program, Token, Tokens, Value, Variables, MAX_SCRIPT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};

const TEXT: u32 = 0x6000_0000;
const BG: u32 = 0x6000_0001;
const SPRITE: u32 = 0x1000;
const CHOICES: u32 = 0x6200_0000;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Capability-relative native 0.txt or XOR-encoded nscript.dat source.
    pub script: String,
    /// Explicit source codec: sjis or utf8; no guessing from game names.
    #[serde(default = "sjis")]
    pub encoding: String,
    /// Licensed font alias declared by the engine project.
    pub font: String,
    /// Native logical canvas; does not select a device or window size.
    #[serde(default = "canvas")]
    pub canvas: [u32; 2],
    /// Explicit mapping of the plain native message window to engine text.
    #[serde(default)]
    pub text: TextConfig,
    /// Engine container mount prefixes, searched after loose files, in priority order.
    #[serde(default)]
    pub archives: Vec<String>,
}
fn sjis() -> String {
    "sjis".into()
}
fn canvas() -> [u32; 2] {
    [640, 480]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextConfig {
    /// Message bounds [x,y,width,height] in logical canvas pixels.
    pub rect: [i32; 4],
    /// Fixed-cell font size, 1 through 512 pixels.
    pub size: u16,
    /// Engine RGBA8 glyph color.
    pub color: [u8; 4],
}
impl Default for TextConfig {
    fn default() -> Self {
        Self {
            rect: [0, 320, 640, 160],
            size: 24,
            color: [255; 4],
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            VfsPath::new(Mount::Asset, &self.script)?.relative() == self.script,
            "noncanonical native script path"
        );
        ensure!(self.archives.len() <= 128, "native archive count");
        for path in &self.archives {
            ensure!(
                VfsPath::new(Mount::Asset, path)?.relative() == path,
                "noncanonical native archive prefix"
            );
        }
        ensure!(
            matches!(self.encoding.as_str(), "sjis" | "utf8"),
            "native encoding must be sjis or utf8"
        );
        ensure!(
            !self.font.is_empty()
                && self.font.len() <= 256
                && !self.font.chars().any(char::is_control),
            "native font alias"
        );
        ensure!(
            self.canvas.iter().all(|n| (1..=8192).contains(n)),
            "native canvas size"
        );
        ensure!(
            (1..=512).contains(&self.text.size)
                && self.text.rect[2..].iter().all(|n| (1..=32767).contains(n))
                && self.text.rect[..2]
                    .iter()
                    .all(|n| (-32767..=32767).contains(n)),
            "native text window"
        );
        Ok(())
    }
}
struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "NScripter executes native source, not data VM actions"
        );
        Ok(Box::new(NscripterVm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("nscripter", Arc::new(Factory))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Wait {
    None,
    Click {
        page: bool,
    },
    Time(u64),
    Delay(u64),
    Image {
        next: usize,
    },
    Buttons {
        variable: u16,
    },
    AutoClick {
        until: u64,
        page: bool,
    },
    Reveal {
        page: bool,
        click: bool,
    },
    Audio {
        ticket: u64,
        next: usize,
    },
    Choices {
        targets: Vec<String>,
        number: Option<u16>,
        gosub: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Call {
    next: usize,
    params: Vec<Value>,
    cursor: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ForLoop {
    var: u16,
    to: i32,
    step: i32,
    body: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    native: native::NativeState,
    pc: usize,
    vars: Variables,
    calls: Vec<Call>,
    loops: Vec<ForLoop>,
    subroutines: BTreeSet<String>,
    define: bool,
    halted: bool,
    wait: Wait,
    text: String,
    cursor: CellCursor,
    glyphs: Vec<Json>,
    text_published: bool,
    speed: u64,
    layers: BTreeMap<u32, Json>,
    dirty: BTreeSet<u32>,
    removed: BTreeSet<u32>,
    published: BTreeSet<u32>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            native: native::NativeState::default(),
            pc: 0,
            vars: Variables::default(),
            calls: vec![],
            loops: vec![],
            subroutines: BTreeSet::new(),
            define: false,
            halted: false,
            wait: Wait::None,
            text: String::new(),
            cursor: CellCursor::at_start(),
            glyphs: vec![],
            text_published: false,
            speed: 20,
            layers: BTreeMap::new(),
            dirty: BTreeSet::new(),
            removed: BTreeSet::new(),
            published: BTreeSet::new(),
        }
    }
}
struct Loading {
    receiver: Receiver<Result<(String, Program, Option<native::Persistent>), String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    config: Config,
    source: String,
    state: State,
}
pub struct NscripterVm {
    client: ClientId,
    config: Config,
    program: Option<Program>,
    source: String,
    loading: Option<Loading>,
    state: State,
    last_error: Option<String>,
    pending_image: Option<native::PendingImage>,
    loaded_persistent: Option<native::Persistent>,
    persistent_write: Option<Box<dyn crysirror_core::service::StorageWrite>>,
    persistent_dirty: bool,
}
impl NscripterVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            program: None,
            source: String::new(),
            loading: None,
            state: State::default(),
            last_error: None,
            pending_image: None,
            loaded_persistent: None,
            persistent_write: None,
            persistent_dirty: false,
        })
    }
    fn load(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        if self.program.is_some() {
            return Ok(true);
        }
        if self.loading.is_none() {
            let storage = host.storage(self.client)?;
            let path = VfsPath::new(Mount::Asset, &self.config.script)?;
            let config = self.config.clone();
            let (sender, receiver) = mpsc::sync_channel(1);
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            std::thread::Builder::new().name("crysirror-nscript-source".into()).spawn(move|| {
                let result=(||->Result<(String,Program,Option<native::Persistent>)>{
                    let reader=storage.open_asset(&path)?;
                    let length=usize::try_from(reader.len()?)?;ensure!(length>0 && length<=MAX_SCRIPT,"native script byte budget");
                    let mut bytes=vec![0;length];let mut at=0;
                    while at<length {
                        ensure!(!stop.load(Ordering::Acquire),"native script load cancelled");
                        let end=(at+65536).min(length);
                        let count=reader.read_at(at as u64,&mut bytes[at..end])?;
                        ensure!(count>0 && count<=end-at,"native script reader truncated/invalid count");at+=count;
                    }
                    if config.script.to_ascii_lowercase().ends_with("nscript.dat") || config.script.to_ascii_lowercase().ends_with("pscript.dat") {
                        crysirror_resource::transforms::xor_cycle_in_place(&mut bytes,&[0x84],0);
                    } else {ensure!(!config.script.ends_with(".___") && !config.script.ends_with("nscr_sec.dat"),"unsupported native script protection requires a provider transform");}
                    let source=if config.encoding=="utf8" {String::from_utf8(bytes)?} else {
                        let (text,_,errors)=encoding_rs::SHIFT_JIS.decode(&bytes);ensure!(!errors,"invalid native Shift-JIS script");text.into_owned()
                    };
                    let program=Program::parse(&source)?;
                    ensure!(!stop.load(Ordering::Acquire),"native script parse cancelled");
                    let save=VfsPath::new(Mount::Save,"nscripter-state.json")?;
                    let persistent=if storage.metadata(Mount::Save,"nscripter-state.json")?.is_some(){let bytes=storage.read(&save)?;ensure!(bytes.len()<=4*1024*1024,"native persistent metadata byte budget");let p:native::Persistent=serde_json::from_slice(&bytes)?;p.validate()?;Some(p)}else{None};
                    Ok((source,program,persistent))
                })().map_err(|e|format!("{e:#}"));
                let _=sender.send(result);
            })?;
            self.loading = Some(Loading { receiver, cancel });
            return Ok(false);
        }
        let result = match self.loading.as_ref().unwrap().receiver.try_recv() {
            Ok(result) => result.map_err(anyhow::Error::msg)?,
            Err(TryRecvError::Empty) => return Ok(false),
            Err(TryRecvError::Disconnected) => bail!("native source worker ended without a result"),
        };
        let (source, program, persistent) = result;
        self.loaded_persistent = persistent;
        self.state.define = program.labels.contains_key("define");
        self.state.pc = if self.state.define {
            program.label("define")?
        } else if program.labels.contains_key("start") {
            program.label("start")?
        } else {
            0
        };
        self.program = Some(program);
        self.source = source;
        self.loading = None;
        Ok(true)
    }
    fn submit(&self, host: &mut dyn HostServices, layers: Vec<Json>, ui: Vec<Json>) -> Result<()> {
        host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"layers":layers,"ui":ui}))?,
        )?;
        Ok(())
    }
    fn flush(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let mut ops = self
            .state
            .removed
            .iter()
            .map(|id| json!({"op":"remove","id":id}))
            .collect::<Vec<_>>();
        ops.extend(
            self.state
                .dirty
                .iter()
                .filter_map(|id| self.state.layers.get(id))
                .map(|layer| json!({"op":"put","layer":layer})),
        );
        self.submit(host, ops, vec![])?;
        self.state
            .published
            .retain(|id| !self.state.removed.contains(id));
        self.state
            .published
            .extend(self.state.dirty.iter().copied());
        self.state.dirty.clear();
        self.state.removed.clear();
        Ok(())
    }
    fn layer(&mut self, id: u32, layer: Json) {
        self.state.layers.insert(id, layer);
        self.state.dirty.insert(id);
        self.state.removed.remove(&id);
    }
    fn layout(&self) -> CellLayoutOptions {
        CellLayoutOptions {
            bounds: [self.window_rect()[2], self.window_rect()[3]],
            size: i32::from(self.text_size()),
            spacing: self.state.native.spacing,
            half_width: true,
            half_spacing: false,
            hanging: 0,
            forbidden_line_start: "、。，．！？）」』】ぁぃぅぇぉっゃゅょ".into(),
            indent_pairs: vec![],
        }
    }
    fn clear_text(&mut self, host: &mut dyn HostServices) -> Result<()> {
        if self.state.text_published {
            self.submit(host, vec![json!({"op":"remove","id":TEXT})], vec![])?;
        }
        if self.state.native.shadow_published {
            self.submit(host, vec![json!({"op":"remove","id":TEXT+3})], vec![])?;
            self.state.native.shadow_published = false;
        }
        self.state.text_published = false;
        self.state.text.clear();
        self.state.glyphs.clear();
        self.state.cursor = CellCursor::at_start();
        Ok(())
    }
    fn text(&mut self, text: &str, host: &mut dyn HostServices) -> Result<()> {
        ensure!(!self.state.define, "native dialogue in define section");
        let text = text.strip_prefix('`').unwrap_or(text);
        ensure!(
            !text.chars().any(|c| matches!(c, '@' | '\\' | '`')),
            "inline native text control is outside plain-terminal-marker profile"
        );
        let mut text = text.to_owned();
        let mut page = false;
        let mut click = false;
        // Terminal markers were removed by dispatch before this function.
        if text.ends_with('\u{1}') {
            text.pop();
            click = true;
        }
        if text.ends_with('\u{2}') {
            text.pop();
            page = true;
            click = true;
        }
        ensure!(
            self.state.text.len() + text.len() <= 65536,
            "native page byte budget"
        );
        let options = self.layout();
        let layout = layout_cells(&text, &options, &mut self.state.cursor)?;
        ensure!(
            layout.consumed == text.len(),
            "native text page overflow; no silent truncation"
        );
        let initial = self.state.glyphs.len();
        let size = self.text_size();
        let color = self.state.native.color.unwrap_or(self.config.text.color);
        let bold = self.state.native.bold;
        self.state.glyphs.extend(layout.glyphs.into_iter().map(|g|json!({"character":g.character,"baseline":[g.position[0],g.position[1]+i32::from(size)],"color":color,"bold":bold})));
        self.state.text += &text;
        let layer = json!({"id":TEXT,"rect":self.window_rect(),"order":[10000,0],"paint":{"kind":"glyph_run","font":self.config.font,"size":self.text_size(),"color":self.state.native.color.unwrap_or(self.config.text.color),"bold":self.state.native.bold,"glyphs":self.state.glyphs,"reveal":{"interval_ms":self.state.speed,"initial_visible":initial}}});
        self.publish_text(layer, host)?;
        self.state.text_published = true;
        self.state.wait = Wait::Reveal { page, click };
        Ok(())
    }
    fn poll_wait(
        &mut self,
        host: &mut dyn HostServices,
        input: &mut InputFrame,
    ) -> Result<Option<VmYield>> {
        let pressed =
            input.buttons_pressed & (buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT) != 0;
        match self.state.wait.clone() {
            Wait::Choices {
                targets,
                number,
                gosub,
            } => {
                let events: Vec<crysirror_scene::ui::UiEvent> = serde_json::from_slice(
                    &host.query_engine(self.client, EngineQuery::UiEvents)?,
                )?;
                let selected = events.iter().find(|e| {
                    e.kind == "activate"
                        && e.widget.owner == self.client.0
                        && (CHOICES + 1..CHOICES + 1 + targets.len() as u32).contains(&e.widget.id)
                });
                let Some(event) = selected else {
                    return Ok(Some(VmYield::WaitForInput));
                };
                let index = (event.widget.id - CHOICES - 1) as usize;
                let target = if number.is_none() {
                    ensure!(
                        !gosub || self.state.calls.len() < 128,
                        "native gosub nesting"
                    );
                    Some(self.program.as_ref().unwrap().label(&targets[index])?)
                } else {
                    None
                };
                let layers = if self.state.text_published {
                    vec![json!({"op":"remove","id":TEXT})]
                } else {
                    vec![]
                };
                self.submit(host, layers, vec![json!({"op":"remove","id":CHOICES})])?;
                self.state.text_published = false;
                self.state.text.clear();
                self.state.glyphs.clear();
                self.state.cursor = CellCursor::at_start();
                if let Some(variable) = number {
                    self.state.vars.ints.insert(variable, index as i32);
                } else {
                    if gosub {
                        self.state.calls.push(Call {
                            next: self.state.pc,
                            params: vec![],
                            cursor: 0,
                        });
                    }
                    self.state.pc = target.unwrap();
                }
                self.state.wait = Wait::None;
                input.buttons_pressed &= !(buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT);
                Ok(None)
            }
            Wait::Image { next } => self.poll_image(host, next),
            Wait::Buttons { variable } => self.poll_buttons(host, input, variable),
            Wait::Delay(until) if host.now_ms() < until && !pressed => {
                Ok(Some(VmYield::WaitUntil(until)))
            }
            Wait::Delay(_) => {
                input.buttons_pressed &= !(buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT);
                self.state.wait = Wait::None;
                Ok(None)
            }
            Wait::AutoClick { until, page: _ } if host.now_ms() < until && !pressed => {
                Ok(Some(VmYield::WaitUntil(until)))
            }
            Wait::AutoClick { page, .. } => {
                if page {
                    self.clear_text(host)?;
                }
                self.state.wait = Wait::None;
                input.buttons_pressed &= !(buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT);
                Ok(None)
            }
            Wait::None => Ok(None),
            Wait::Time(until) if host.now_ms() < until => Ok(Some(VmYield::WaitUntil(until))),
            Wait::Time(_) => {
                self.state.wait = Wait::None;
                Ok(None)
            }
            Wait::Click { page } => {
                if let Some(interval) = self.state.native.autoclick.filter(|v| *v > 0) {
                    self.state.wait = Wait::AutoClick {
                        until: host.now_ms().saturating_add(interval),
                        page,
                    };
                    return Ok(Some(VmYield::WaitUntil(
                        host.now_ms().saturating_add(interval),
                    )));
                }
                if !pressed {
                    return Ok(Some(VmYield::WaitForInput));
                }
                input.buttons_pressed &= !(buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT);
                if page {
                    self.clear_text(host)?;
                }
                self.state.wait = Wait::None;
                Ok(None)
            }
            Wait::Reveal { page, click } => {
                let layer: crysirror_scene::layer::LayerSpec = serde_json::from_slice(
                    &host.query_engine(self.client, EngineQuery::Layer(TEXT))?,
                )?;
                let status = layer
                    .paint
                    .glyph_reveal_status()
                    .context("native text layer replaced externally")?;
                if !status.complete {
                    if pressed {
                        self.submit(
                            host,
                            if self.state.native.shadow_published {
                                vec![
                                    json!({"op":"complete_reveal","id":TEXT}),
                                    json!({"op":"complete_reveal","id":TEXT+3}),
                                ]
                            } else {
                                vec![json!({"op":"complete_reveal","id":TEXT})]
                            },
                            vec![],
                        )?;
                    }
                    return Ok(Some(VmYield::WaitForInput));
                }
                self.state.wait = if click {
                    Wait::Click { page }
                } else {
                    Wait::None
                };
                if click {
                    self.poll_wait(host, input)
                } else {
                    Ok(None)
                }
            }
            Wait::Audio { ticket, next } => {
                let result = host.audio_preparation(self.client, ticket)?;
                match result.phase {
                    AudioPreparePhase::Pending => Ok(Some(VmYield::BudgetExhausted)),
                    AudioPreparePhase::Failed => {
                        host.release_audio(self.client, ticket)?;
                        bail!(
                            "native audio preparation: {}",
                            result
                                .error
                                .unwrap_or_else(|| "failed without diagnostic".into())
                        )
                    }
                    AudioPreparePhase::Ready => {
                        host.release_audio(self.client, ticket)?;
                        self.state.pc = next;
                        self.state.wait = Wait::None;
                        Ok(None)
                    }
                }
            }
        }
    }
    fn run(
        &mut self,
        host: &mut dyn HostServices,
        mut input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        if !self.load(host)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if input.buttons_pressed & (buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT) != 0
            && matches!(
                self.state.wait,
                Wait::Reveal { .. } | Wait::Click { .. } | Wait::AutoClick { .. }
            )
        {
            if let Some(target) = self.state.native.trap.take() {
                self.state.pc = self.program.as_ref().unwrap().label(&target)?;
                self.state.wait = Wait::None;
                input.buttons_pressed &= !(buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT);
            }
        }
        self.poll_persistent(host)?;
        self.update_cursors(host)?;
        if let Some(yielded) = self.poll_wait(host, &mut input)? {
            return Ok(yielded);
        }
        for _ in 0..budget.min(4096) {
            if self.state.halted {
                if self.persistent_write.is_some() || self.persistent_dirty {
                    return Ok(VmYield::BudgetExhausted);
                }
                return Ok(VmYield::Halted);
            }
            let stmt = self
                .program
                .as_ref()
                .unwrap()
                .statements
                .get(self.state.pc)
                .cloned()
                .context("native execution fell off script without end")?;
            let before = self.state.clone();
            self.state.pc += 1;
            if let Err(e) = self.execute(&stmt.body, host) {
                self.state = before;
                return Err(e.context(format!("NScripter line {}: {}", stmt.line, stmt.body)));
            }
            if self.state.native.globals_enabled
                && (before.vars.ints.iter().filter(|(n, _)| **n >= 200).ne(self
                    .state
                    .vars
                    .ints
                    .iter()
                    .filter(|(n, _)| **n >= 200))
                    || before
                        .vars
                        .strings
                        .iter()
                        .filter(|(n, _)| **n >= 200)
                        .ne(self.state.vars.strings.iter().filter(|(n, _)| **n >= 200)))
            {
                self.persistent_dirty = true;
            }
            if !matches!(self.state.wait, Wait::None) {
                if matches!(self.state.wait, Wait::Audio { .. } | Wait::Image { .. }) {
                    self.state.pc = before.pc;
                }
                return Ok(match self.state.wait {
                    Wait::Time(t) => VmYield::WaitUntil(t),
                    Wait::Audio { .. } => VmYield::BudgetExhausted,
                    _ => VmYield::WaitForInput,
                });
            }
        }
        Ok(VmYield::BudgetExhausted)
    }
    fn execute(&mut self, body: &str, host: &mut dyn HostServices) -> Result<()> {
        if let Some(text) = body.strip_prefix('\u{1f}') {
            return self.text(text, host);
        }
        if body.starts_with('`') || !body.as_bytes()[0].is_ascii() {
            let mut text = body.to_owned();
            if text.ends_with('\\') {
                text.pop();
                text.push('\u{2}');
            } else if text.ends_with('@') {
                text.pop();
                text.push('\u{1}');
            } else if text.ends_with('/') {
                text.pop();
            } else {
                text.push('\n');
            }
            return self.text(&text, host);
        }
        let mut t = Tokens::new(body)?;
        let command = t.word()?.trim_start_matches('_').to_owned();
        if self.native_command(&command, &mut t, host)? {
            return Ok(());
        }
        match command.as_str() {
            "select" | "selnum" | "selgosub" => self.select(&mut t, host, command.as_str())?,
            "game" => {
                t.end()?;
                ensure!(self.state.define, "game outside define section");
                self.state.pc = self.program.as_ref().unwrap().label("start")?;
                self.state.define = false;
                self.layer(BG,json!({"id":BG,"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"order":[-10000,0],"paint":{"kind":"solid","color":[0,0,0,255]}}));
                self.flush(host)?;
            }
            "caption" => {
                let title = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                self.submit(host, vec![], vec![json!({"op":"set_title","text":title})])?;
            }
            "numalias" | "stralias" => {
                ensure!(self.state.define, "alias outside define section");
                let name = t.word()?;
                t.comma()?;
                let value = t.expr(&self.state.vars)?;
                t.end()?;
                if command == "numalias" {
                    value.int()?;
                } else {
                    value.string()?;
                }
                self.state.vars.aliases.insert(name, value);
            }
            "defsub" => {
                ensure!(self.state.define, "defsub outside define section");
                let name = t.word()?;
                t.end()?;
                self.program.as_ref().unwrap().label(&name)?;
                self.state.subroutines.insert(name);
            }
            "mov" => {
                let dest = t.variable()?;
                t.comma()?;
                let value = t.expr(&self.state.vars)?;
                t.end()?;
                self.state.vars.set(dest, value)?;
            }
            "inc" | "dec" | "add" | "sub" | "mul" | "div" | "mod" => {
                let dest = self.state.vars.resolve(t.variable()?)?;
                let mut read = Tokens {
                    values: vec![dest.clone()],
                    at: 0,
                };
                let left = read.expr(&self.state.vars)?;
                let right = if command == "inc" || command == "dec" {
                    Value::Int(1)
                } else {
                    t.comma()?;
                    t.expr(&self.state.vars)?
                };
                t.end()?;
                let value = if command == "add" && matches!(left, Value::Str(_)) {
                    let a = left.string()?;
                    let b = right.string()?;
                    ensure!(a.len() + b.len() <= 65536, "native string budget");
                    Value::Str(a + &b)
                } else {
                    let a = left.int()?;
                    let b = right.int()?;
                    Value::Int(match command.as_str() {
                        "inc" | "add" => a.wrapping_add(b),
                        "dec" | "sub" => a.wrapping_sub(b),
                        "mul" => a.wrapping_mul(b),
                        "div" => {
                            ensure!(b != 0, "native division by zero");
                            a.wrapping_div(b)
                        }
                        "mod" => {
                            ensure!(b != 0, "native modulo by zero");
                            a.wrapping_rem(b)
                        }
                        _ => unreachable!(),
                    })
                };
                self.state.vars.set(dest, value)?;
            }
            "goto" => {
                let label = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                self.state.pc = self.program.as_ref().unwrap().label(&label)?;
            }
            "gosub" => {
                let label = t.expr(&self.state.vars)?.string()?;
                self.call(&label, &mut t, true)?;
            }
            "getparam" => loop {
                let dest = t.variable()?;
                let frame = self
                    .state
                    .calls
                    .last_mut()
                    .context("getparam outside gosub")?;
                let value = frame
                    .params
                    .get(frame.cursor)
                    .cloned()
                    .context("missing gosub parameter")?;
                frame.cursor += 1;
                self.state.vars.set(dest, value)?;
                if t.peek().is_none() {
                    break;
                }
                t.comma()?;
            },
            "return" => {
                let frame = self.state.calls.pop().context("return outside gosub")?;
                self.state.pc = if t.peek().is_none() {
                    frame.next
                } else {
                    let label = t.expr(&self.state.vars)?.string()?;
                    t.end()?;
                    self.program.as_ref().unwrap().label(&label)?
                };
            }
            "if" | "notif" => {
                self.condition(&mut t, command == "notif", host)?;
            }
            "for" => {
                let Token::IntVar(var) = self.state.vars.resolve(t.variable()?)? else {
                    bail!("for requires integer variable")
                };
                ensure!(t.take()? == Token::Op("=".into()), "for requires =");
                let from = t.expr(&self.state.vars)?.int()?;
                ensure!(t.word()? == "to", "for requires to");
                let to = t.expr(&self.state.vars)?.int()?;
                let step = if t.peek().is_some() {
                    ensure!(t.word()? == "step", "for requires step");
                    t.expr(&self.state.vars)?.int()?
                } else {
                    1
                };
                t.end()?;
                ensure!(step != 0, "zero for step");
                self.state.vars.ints.insert(var, from);
                ensure!(self.state.loops.len() < 128, "native loop nesting");
                if step > 0 && from > to || step < 0 && from < to {
                    self.skip_loop()?;
                } else {
                    self.state.loops.push(ForLoop {
                        var,
                        to,
                        step,
                        body: self.state.pc,
                    });
                }
            }
            "next" => {
                t.end()?;
                let f = self
                    .state
                    .loops
                    .last()
                    .cloned()
                    .context("next outside for")?;
                let value = self.state.vars.ints[&f.var].wrapping_add(f.step);
                self.state.vars.ints.insert(f.var, value);
                if f.step > 0 && value > f.to || f.step < 0 && value < f.to {
                    self.state.loops.pop();
                } else {
                    self.state.pc = f.body;
                }
            }
            "break" => {
                t.end()?;
                self.state.loops.pop().context("break outside for")?;
                self.skip_loop()?;
            }
            "wait" => {
                let ms = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                ensure!((0..=600000).contains(&ms), "native wait duration");
                self.state.wait = Wait::Time(host.now_ms().saturating_add(ms as u64));
            }
            "click" => {
                t.end()?;
                self.state.wait = Wait::Click { page: false };
            }
            "textspeed" => {
                let speed = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                ensure!((0..=1000).contains(&speed), "native text speed");
                self.state.speed = speed as u64;
            }
            "textclear" => {
                t.end()?;
                self.clear_text(host)?;
            }
            "br" => {
                t.end()?;
                self.text("\n", host)?;
            }
            "bg" => {
                let value = if let Some(Token::Word(w)) = t.peek() {
                    if matches!(w.as_str(), "white" | "black") {
                        let w = w.clone();
                        t.at += 1;
                        w
                    } else {
                        t.expr(&self.state.vars)?.string()?
                    }
                } else {
                    t.expr(&self.state.vars)?.string()?
                };
                t.comma()?;
                let effect = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                ensure!(
                    effect == 1,
                    "NScripter background effect requires shared transition mapping"
                );
                let paint = match value.as_str() {
                    "white" => json!({"kind":"solid","color":[255,255,255,255]}),
                    "black" => json!({"kind":"solid","color":[0,0,0,255]}),
                    _ => {
                        let asset = asset(&value)?;
                        ensure!(
                            host.image_is_opaque(self.client, &asset)?,
                            "native background requires opaque shared image"
                        );
                        ensure!(
                            host.image_size(self.client, &asset)? == self.config.canvas,
                            "native background profile requires canvas-sized image"
                        );
                        json!({"kind":"image","asset":asset})
                    }
                };
                self.layer(BG,json!({"id":BG,"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"order":[-10000,0],"paint":paint}));
                self.flush(host)?;
            }
            "lsp" | "lsph" => {
                let no = number(&mut t, &self.state.vars, 0, 999)? as u32;
                t.comma()?;
                let name = t.expr(&self.state.vars)?.string()?;
                t.comma()?;
                let x = number(&mut t, &self.state.vars, -32767, 32767)?;
                t.comma()?;
                let y = number(&mut t, &self.state.vars, -32767, 32767)?;
                let opacity = if t.peek().is_some() {
                    t.comma()?;
                    number(&mut t, &self.state.vars, 0, 255)?
                } else {
                    255
                };
                t.end()?;
                let path=name.strip_prefix(":a;").context("native sprite profile requires :a; RGBA PNG; color-key/mask-sheet tags need shared extraction")?;
                ensure!(
                    path.to_ascii_lowercase().ends_with(".png"),
                    "native sprite profile requires RGBA PNG"
                );
                let path = asset(path)?;
                let [w, h] = host.image_size(self.client, &path)?;
                self.layer(SPRITE+no,json!({"id":SPRITE+no,"rect":[x,y,w,h],"order":[1000-no as i32,0],"visible":command=="lsp","opacity":opacity,"paint":{"kind":"image","asset":path}}));
            }
            "vsp" => {
                let no = number(&mut t, &self.state.vars, 0, 999)? as u32;
                t.comma()?;
                let visible = number(&mut t, &self.state.vars, 0, 1)?;
                t.end()?;
                let l = self
                    .state
                    .layers
                    .get_mut(&(SPRITE + no))
                    .context("vsp sprite missing")?;
                l["visible"] = json!(visible == 1);
                self.state.dirty.insert(SPRITE + no);
            }
            "csp" => {
                let no = number(&mut t, &self.state.vars, -1, 999)?;
                t.end()?;
                let ids = if no == -1 {
                    self.state
                        .layers
                        .keys()
                        .copied()
                        .filter(|id| (SPRITE..SPRITE + 1000).contains(id))
                        .collect::<Vec<_>>()
                } else {
                    vec![SPRITE + no as u32]
                };
                for id in ids {
                    self.state.layers.remove(&id);
                    self.state.dirty.remove(&id);
                    if self.state.published.contains(&id) {
                        self.state.removed.insert(id);
                    }
                }
            }
            "print" => {
                let effect = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                ensure!(
                    effect == 1,
                    "NScripter print effect requires shared transition mapping"
                );
                self.flush(host)?;
            }
            "bgm" | "bgmonce" | "dwave" | "dwaveloop" => {
                let channel = if command.starts_with("dwave") {
                    let ch = number(&mut t, &self.state.vars, 0, 49)? as u32;
                    t.comma()?;
                    ch
                } else {
                    100
                };
                let path = asset(&t.expr(&self.state.vars)?.string()?)?;
                t.end()?;
                let ticket = host.begin_audio(
                    self.client,
                    AudioRequest::PlayAsset {
                        channel,
                        source: AudioAsset {
                            path: VfsPath::new(Mount::Asset, &path)?,
                            frame_limit: None,
                        },
                        intro: None,
                        parameters: AudioParameters {
                            looping: matches!(command.as_str(), "bgm" | "dwaveloop"),
                            bus: if channel == 100 {
                                PcmBus::Music
                            } else {
                                PcmBus::Sfx
                            },
                            ..Default::default()
                        },
                    },
                )?;
                self.state.wait = Wait::Audio {
                    ticket,
                    next: self.state.pc,
                };
            }
            "bgmstop" | "dwavestop" => {
                let channel = if command == "bgmstop" {
                    100
                } else {
                    number(&mut t, &self.state.vars, 0, 49)? as u32
                };
                t.end()?;
                host.submit_audio(
                    self.client,
                    AudioRequest::Stop {
                        channel,
                        fade_ms: 0.,
                    },
                )?;
            }
            "end" => {
                t.end()?;
                self.state.halted = true;
            }
            name if self.state.subroutines.contains(name) => self.call(name, &mut t, false)?,
            _ => bail!("unsupported NScripter native command `{command}`"),
        }
        Ok(())
    }
    fn call(&mut self, name: &str, t: &mut Tokens, comma_first: bool) -> Result<()> {
        ensure!(self.state.calls.len() < 128, "native gosub nesting");
        let pc = self.program.as_ref().unwrap().label(name)?;
        let mut params = vec![];
        while t.peek().is_some() {
            if comma_first || !params.is_empty() {
                t.comma()?;
            }
            params.push(t.expr(&self.state.vars)?);
            ensure!(params.len() <= 64, "native argument budget");
        }
        self.state.calls.push(Call {
            next: self.state.pc,
            params,
            cursor: 0,
        });
        self.state.pc = pc;
        Ok(())
    }
    fn select(&mut self, t: &mut Tokens, host: &mut dyn HostServices, mode: &str) -> Result<()> {
        let number = if mode == "selnum" {
            let Token::IntVar(n) = self.state.vars.resolve(t.variable()?)? else {
                bail!("selnum requires integer variable")
            };
            t.comma()?;
            Some(n)
        } else {
            None
        };
        let mut choices = Vec::new();
        let mut targets = Vec::new();
        loop {
            let text = t.expr(&self.state.vars)?.string()?;
            let target = if number.is_some() {
                String::new()
            } else {
                t.comma()?;
                let target = t.expr(&self.state.vars)?.string()?;
                self.program.as_ref().unwrap().label(&target)?;
                target
            };
            ensure!(text.len() <= 4096, "native choice text budget");
            choices.push(text);
            targets.push(target);
            ensure!(choices.len() <= 64, "native choice count");
            if t.peek().is_none() {
                break;
            }
            t.comma()?;
        }
        let visible = choices.iter().filter(|s| !s.is_empty()).count();
        ensure!(visible > 0, "native selection has no visible choices");
        let row = u32::from(self.text_size()) + 8;
        let total = row * visible as u32;
        ensure!(
            total <= self.window_rect()[3] as u32,
            "native selection exceeds mapped message window"
        );
        let mut rect = self.window_rect();
        let cursor_y = self.state.cursor.position[1];
        if cursor_y + total as i32 <= rect[3] {
            rect[1] += cursor_y;
        } else {
            self.clear_text(host)?;
        }
        let mut ui = vec![
            json!({"op":"put","widget":{"id":CHOICES,"rect":[rect[0],rect[1],rect[2],total],"kind":"panel","flow":"column","modal":true,"z":30000,"color":[0,0,0,0]}}),
        ];
        for (index, text) in choices.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let id = CHOICES + 1 + index as u32;
            ui.push(json!({"op":"put","widget":{"id":id,"parent":CHOICES,"rect":[0,0,self.window_rect()[2],row],"kind":"button","text":text,"font":self.config.font,"text_size":self.text_size(),"color":[0,0,0,0]}}));
        }
        self.submit(host, vec![], ui)?;
        self.state.wait = Wait::Choices {
            targets,
            number,
            gosub: mode == "selgosub",
        };
        Ok(())
    }
    fn skip_loop(&mut self) -> Result<()> {
        let program = self.program.as_ref().unwrap();
        let mut nesting = 0;
        while let Some(s) = program.statements.get(self.state.pc) {
            self.state.pc += 1;
            let command = s.body.split_whitespace().next().unwrap_or("");
            if command == "for" {
                nesting += 1;
            } else if command == "next" {
                if nesting == 0 {
                    return Ok(());
                }
                nesting -= 1;
            }
        }
        bail!("for without matching next")
    }
    fn condition(
        &mut self,
        t: &mut Tokens,
        invert: bool,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        let mut combined = None;
        let mut mode = None;
        loop {
            let a = t.expr(&self.state.vars)?;
            let Token::Op(op) = t.take()? else {
                bail!("native if comparison required")
            };
            let b = t.expr(&self.state.vars)?;
            let ord = match (a, b) {
                (Value::Int(a), Value::Int(b)) => a.cmp(&b),
                (Value::Str(a), Value::Str(b)) => a.cmp(&b),
                _ => bail!("mixed native comparison"),
            };
            let mut test = match op.as_str() {
                "=" | "==" => ord.is_eq(),
                "!=" | "<>" => !ord.is_eq(),
                ">" => ord.is_gt(),
                "<" => ord.is_lt(),
                ">=" => !ord.is_lt(),
                "<=" => !ord.is_gt(),
                _ => bail!("invalid native comparison"),
            };
            if invert {
                test = !test;
            }
            combined = Some(match (combined, mode) {
                (None, _) => test,
                (Some(a), Some(true)) => a && test,
                (Some(a), _) => a || test,
            });
            let Some(Token::Op(op)) = t.peek() else { break };
            if !matches!(op.as_str(), "&" | "&&" | "|" | "||") {
                break;
            }
            let and = op.starts_with('&');
            if let Some(previous) = mode {
                ensure!(previous == and, "native if cannot mix and/or");
            }
            mode = Some(and);
            t.at += 1;
        }
        if combined.unwrap_or(false) {
            ensure!(t.at < t.values.len(), "native if command missing");
            let remaining = t.values[t.at..]
                .iter()
                .map(token_source)
                .collect::<Vec<_>>()
                .join(" ");
            self.execute(&remaining, host)?;
        } else {
            let program = self.program.as_ref().unwrap();
            let line = program.statements[self.state.pc - 1].line;
            while program
                .statements
                .get(self.state.pc)
                .is_some_and(|s| s.line == line)
            {
                self.state.pc += 1;
            }
        }
        Ok(())
    }
}
fn token_source(t: &Token) -> String {
    match t {
        Token::Num(n) => n.to_string(),
        Token::Str(s) => format!("\"{s}\""),
        Token::Label(s) => format!("*{s}"),
        Token::IntVar(n) => format!("%{n}"),
        Token::StrVar(n) => format!("${n}"),
        Token::IntAlias(n) => format!("%{n}"),
        Token::StrAlias(n) => format!("${n}"),
        Token::Word(s) | Token::Op(s) => s.clone(),
        Token::Comma => ",".into(),
    }
}
fn number(t: &mut Tokens, v: &Variables, min: i32, max: i32) -> Result<i32> {
    let n = t.expr(v)?.int()?;
    ensure!(
        (min..=max).contains(&n),
        "native numeric argument {n} outside {min}..{max}"
    );
    Ok(n)
}
fn asset(name: &str) -> Result<String> {
    let name = name.replace('\\', "/");
    let p = VfsPath::new(Mount::Asset, &name)?;
    ensure!(p.relative() == name, "noncanonical native resource path");
    Ok(name)
}
impl VmAdapter for NscripterVm {
    fn engine_id(&self) -> &'static str {
        "nscripter"
    }
    fn snapshot_schema(&self) -> u32 {
        2
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.run(host, input, budget).map_err(|e| {
            let error = format!("{e:#}");
            self.last_error = Some(error.clone());
            Error::Io(error)
        })
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        if self.persistent_write.is_some() || self.persistent_dirty {
            return Err(Error::Unsupported(
                "NScripter snapshot during persistent metadata write",
            ));
        }
        if self.state.native.render_products {
            return Err(Error::Unsupported(
                "NScripter prepared-image snapshot reconstruction",
            ));
        }
        if self.program.is_none()
            || matches!(self.state.wait, Wait::Audio { .. } | Wait::Image { .. })
        {
            return Err(Error::Unsupported(
                "NScripter snapshot during pending source/audio work",
            ));
        }
        serde_json::to_vec(&Snapshot {
            version: 2,
            config: self.config.clone(),
            source: self.source.clone(),
            state: self.state.clone(),
        })
        .map_err(|e| Error::Io(e.to_string()))
    }
    fn restore(&mut self, bytes: &[u8]) -> crysirror_core::Result<()> {
        let validate = (|| -> Result<(Snapshot, Program)> {
            ensure!(
                bytes.len() <= 8 * 1024 * 1024,
                "native snapshot byte budget"
            );
            let data: Snapshot = serde_json::from_slice(bytes)?;
            ensure!(
                data.version == 2 && data.config == self.config,
                "native snapshot version/config mismatch"
            );
            data.config.validate()?;
            let program = Program::parse(&data.source)?;
            let s = &data.state;
            ensure!(
                s.pc < program.statements.len() || s.halted && s.pc == program.statements.len(),
                "native saved PC"
            );
            ensure!(
                s.calls.len() <= 128
                    && s.loops.len() <= 128
                    && s.layers.len() <= 1001
                    && s.glyphs.len() <= 65536
                    && s.text.len() <= 65536,
                "native saved state budgets"
            );
            ensure!(
                s.calls.iter().all(|c| c.next <= program.statements.len()
                    && c.params.len() <= 64
                    && c.cursor <= c.params.len())
                    && s.loops
                        .iter()
                        .all(|f| f.var < 10000 && f.body < program.statements.len() && f.step != 0),
                "native saved control stack"
            );
            ensure!(
                !matches!(s.wait, Wait::Audio { .. } | Wait::Image {..}),
                "native worker handles are not serializable"
            );
            if let Wait::Choices {targets,number,..}=&s.wait {
                ensure!(!targets.is_empty() && targets.len()<=64 && number.is_none_or(|n|n<10000),"native saved choice state");
                if number.is_none(){for target in targets {program.label(target)?;}}
            }
            ensure!(s.vars.aliases.len()<=10000 && s.subroutines.len()<=1024 && s.speed<=1000,"native saved alias/text settings");
            for name in &s.subroutines {program.label(name)?;}
            let _:Vec<crysirror_scene::layer::PositionedGlyph>=serde_json::from_value(Json::Array(s.glyphs.clone()))?;
            let paint:crysirror_scene::layer::Paint=serde_json::from_value(json!({"kind":"glyph_run","font":self.config.font,"size":self.config.text.size,"glyphs":s.glyphs}))?;
            paint.validate()?;
            let mut cursor=s.cursor.clone();layout_cells("",&self.layout(),&mut cursor)?;
            ensure!(s.dirty.len()<=1001 && s.removed.len()<=1001 && s.dirty.iter().all(|id|s.layers.contains_key(id)) && s.removed.iter().all(|id|(*id==BG || (SPRITE..SPRITE+1000).contains(id)) && !s.layers.contains_key(id)),"native saved staging state");
            ensure!(
                s.vars
                    .ints
                    .keys()
                    .chain(s.vars.strings.keys())
                    .all(|n| *n < 10000)
                    && s.vars.strings.values().all(|s| s.len() <= 65536),
                "native saved variables"
            );
            for (id, value) in &s.layers {
                let layer: crysirror_scene::layer::LayerSpec =
                    serde_json::from_value(value.clone())?;
                ensure!(
                    layer.id == *id && (*id == BG || (SPRITE..SPRITE + 1000).contains(id)),
                    "native saved layer identity"
                );
                layer.paint.validate()?;
            }
            Ok((data, program))
        })()
        .map_err(|e| Error::Io(format!("{e:#}")))?;
        self.loading = None;
        self.pending_image = None;
        self.source = validate.0.source;
        self.state = validate.0.state;
        self.program = Some(validate.1);
        self.last_error = None;
        Ok(())
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"profile":"native-nscript-v2","pc":self.state.pc,"loaded":self.program.is_some(),"halted":self.state.halted,"line":self.program.as_ref().and_then(|p|p.statements.get(self.state.pc)).map(|s|s.line),"integers":self.state.vars.ints,"strings":self.state.vars.strings,"text":self.state.text,"wait":self.state.wait,"error":self.last_error,"native":self.state.native})).unwrap_or_default()
    }
}
