// SPDX-License-Identifier: MPL-2.0
//! Native syscall translation only. Draws, glyph reveal, UI, audio and media
//! are engine-owned services. No pixel buffer, renderer or mixer lives here.
use crate::{
    resources::Resources,
    vm::{
        Archive, Gameexe, Machine, Next, Nls, RunResult, VmEvent,
        bytecode::{Command, CommandKind},
        machine::{FrameKind, IntTarget},
    },
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{
        AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, EngineQuery, HostServices,
        InputFrame, MediaPhase, MediaRequest, PcmBus, buttons,
    },
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
mod commands;
use std::{collections::BTreeMap, rc::Rc, sync::Arc};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub root: String,
    #[serde(default = "font")]
    pub font_alias: String,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub start_scene: Option<i32>,
}
fn font() -> String {
    "default".into()
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.root.is_empty() {
            let p = VfsPath::new(Mount::Asset, &self.root).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                p.relative() == self.root,
                "canonical RealLive root required"
            );
        }
        ensure!(!self.font_alias.is_empty(), "font alias required");
        if let Some(n) = &self.encoding {
            n.parse::<Nls>()?;
        }
        Ok(())
    }
}
#[derive(Debug)]
enum Wait {
    Time {
        until: u64,
        cancel: bool,
    },
    Click {
        record: Option<(IntTarget, IntTarget)>,
    },
    Media {
        cancel: bool,
    },
    Audio {
        channel: u32,
    },
    Reveal {
        layer: u32,
    },
    Pause {
        kind: u16,
        window: u32,
    },
    Name {
        local: bool,
        fields: Vec<(i32, u32)>,
    },
    Choice {
        indices: Vec<usize>,
        ids: Vec<u32>,
        enabled: Vec<bool>,
    },
}
pub struct RealLiveVm {
    client: ClientId,
    config: Config,
    machine: Option<Machine>,
    resources: Option<Resources>,
    wait: Option<Wait>,
    pending_audio: Option<(u64, Option<u32>)>,
    width: u32,
    height: u32,
    last_down: u64,
    button_state: [i32; 2],
    timers: [[u64; 256]; 2],
    title: String,
    menu: bool,
    syscom: BTreeMap<i32, i32>,
    default_grp: String,
    window: u32,
    pen: BTreeMap<u32, [i32; 2]>,
    sizes: BTreeMap<u32, i32>,
    color: u32,
    fast_text: bool,
    hide_cursor: bool,
    grp_closes: bool,
    wipe_closes: bool,
    draw_stack: Vec<String>,
    frames: u64,
    api_calls: u64,
    opcode_counts: BTreeMap<String, u64>,
    last_error: Option<String>,
    input_used: bool,
}
impl RealLiveVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            machine: None,
            resources: None,
            wait: None,
            pending_audio: None,
            width: 640,
            height: 480,
            last_down: 0,
            button_state: [0; 2],
            timers: [[0; 256]; 2],
            title: String::new(),
            menu: true,
            syscom: BTreeMap::new(),
            default_grp: String::new(),
            window: 0,
            pen: BTreeMap::new(),
            sizes: BTreeMap::new(),
            color: 0,
            fast_text: false,
            hide_cursor: false,
            grp_closes: false,
            wipe_closes: false,
            draw_stack: vec![],
            frames: 0,
            api_calls: 0,
            opcode_counts: BTreeMap::new(),
            last_error: None,
            input_used: false,
        })
    }
    fn boot(&mut self, h: &mut dyn HostServices) -> Result<()> {
        let storage = h.storage(self.client).map_err(err)?;
        let mut r = Resources::new(storage, self.config.root.clone());
        let ini = r.file("", "Gameexe.ini")?.context("Gameexe.ini absent")?;
        let seen = r.file("", "Seen.txt")?.context("Seen.txt absent")?;
        let configured = self.config.encoding.as_deref().unwrap_or("sjis").parse()?;
        let ini_bytes = r.read(&ini)?;
        let mut exe = Gameexe::from_bytes(&ini_bytes, configured);
        let mut loose = BTreeMap::new();
        for name in r.storage.list(Mount::Asset, &r.root).map_err(err)? {
            let upper = name.to_ascii_uppercase();
            if upper.is_ascii()
                && upper.len() == 12
                && upper.starts_with("SEEN")
                && upper.ends_with(".TXT")
            {
                if let Ok(number) = upper[4..8].parse::<i32>() {
                    let path = if r.root.is_empty() {
                        name
                    } else {
                        format!("{}/{}", r.root, name)
                    };
                    loose.insert(number, r.read(&path)?);
                }
            }
        }
        let bytes = r.read(&seen)?;
        let mut archive = Archive::from_parts(
            bytes.clone(),
            loose.clone(),
            exe.str("REGNAME").unwrap_or(""),
            configured,
        )?;
        if self.config.encoding.is_none() {
            if let Some(nls) = archive.probable_encoding() {
                if nls != configured {
                    exe = Gameexe::from_bytes(&ini_bytes, nls);
                    archive =
                        Archive::from_parts(bytes, loose, exe.str("REGNAME").unwrap_or(""), nls)?;
                }
            }
        }
        let exe = Rc::new(exe);
        let archive = Rc::new(archive);
        let mut vm = Machine::new(archive, exe.clone())?;
        if let Some(s) = self.config.start_scene {
            vm.jump(s, 0)?;
        }
        let mode = exe.int("SCREENSIZE_MOD").unwrap_or(0);
        [self.width, self.height] = match mode {
            1 => [800, 600],
            2 => [1024, 768],
            3 => [1280, 960],
            4 => [1600, 1200],
            _ => [640, 480],
        };
        self.grp_closes = exe.int("GRPCOM_WINDOWCLOSE").unwrap_or(0) != 0;
        self.wipe_closes = exe.int("WAIP_WINDOWCLOSE").unwrap_or(0) != 0;
        self.resources = Some(r);
        let mut ops = Vec::new();
        for id in 0..16 {
            ops.push(json!({"op":"create","id":id,"width":self.width,"height":self.height}));
        }
        ops.push(json!({"op":"present","id":0,"layer":0}));
        self.submit(h, json!({"canvases":ops}))?;
        self.machine = Some(vm);
        Ok(())
    }
    fn submit(&mut self, h: &mut dyn HostServices, batch: Value) -> Result<()> {
        h.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)
            .map_err(err)?;
        self.api_calls += 1;
        Ok(())
    }
    fn canvas(&mut self, h: &mut dyn HostServices, ops: Vec<Value>) -> Result<()> {
        self.submit(h, json!({"canvases":ops}))
    }
    fn layer(&mut self, h: &mut dyn HostServices, layer: Value) -> Result<()> {
        self.submit(h, json!({"layers":[{"op":"put","layer":layer}]}))
    }
    fn query(&self, h: &mut dyn HostServices, q: EngineQuery) -> Result<Value> {
        Ok(serde_json::from_slice(
            &h.query_engine(self.client, q).map_err(err)?,
        )?)
    }
    fn extent(&self, h: &mut dyn HostServices, id: i32) -> Result<[u32; 2]> {
        let v = self.query(h, EngineQuery::Canvas(u32::try_from(id)?))?;
        Ok([
            v["width"].as_u64().context("canvas width")? as u32,
            v["height"].as_u64().context("canvas height")? as u32,
        ])
    }
    fn resource(&mut self, m: &Machine, kind: &str, name: &str) -> Result<String> {
        self.resources
            .as_mut()
            .unwrap()
            .find(&m.gameexe, kind, name)
    }
    fn image_layer(&self, asset: String, rect: [f32; 4], uv: Option<[f32; 4]>, alpha: u8) -> Value {
        json!({"id":0,"rect":rect,"paint":{"kind":"image","asset":asset},"uv":uv,"opacity":alpha})
    }
    fn tick_wait(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        input: InputFrame,
    ) -> Result<bool> {
        let Some(wait) = self.wait.take() else {
            return Ok(true);
        };
        let pressed = !self.input_used
            && (input.buttons_pressed & (buttons::MOUSE_LEFT | buttons::ENTER | buttons::SPACE)
                != 0
                || input.buttons_down & buttons::MOUSE_LEFT != 0
                    && self.last_down & buttons::MOUSE_LEFT == 0);
        let done = match &wait {
            Wait::Time { until, cancel } => {
                if *cancel && pressed {
                    self.input_used = true;
                    m.store = 1;
                    true
                } else if h.now_ms() >= *until {
                    if *cancel {
                        m.store = 0;
                    }
                    true
                } else {
                    false
                }
            }
            Wait::Click { record } => {
                if pressed {
                    self.input_used = true;
                    if let Some((x, y)) = record {
                        m.set_target(*x, input.pointer_x)?;
                        m.set_target(*y, input.pointer_y)?;
                    }
                    m.store = 1;
                    true
                } else {
                    false
                }
            }
            Wait::Media { cancel } => {
                let status = match h.media_info(self.client, 0) {
                    Ok(v) => Some(v),
                    Err(Error::NotFound) => None,
                    Err(e) => return Err(err(e)),
                };
                let cancelled = status
                    .is_some_and(|s| matches!(s.phase, MediaPhase::Loading | MediaPhase::Playing))
                    && *cancel
                    && !self.input_used
                    && (input.buttons_pressed
                        & (buttons::MOUSE_LEFT
                            | buttons::MOUSE_RIGHT
                            | buttons::ENTER
                            | buttons::SPACE
                            | buttons::ESCAPE)
                        != 0);
                if status.is_none_or(|s| matches!(s.phase, MediaPhase::Ended | MediaPhase::Stopped))
                    || cancelled
                {
                    self.stop_movie(h)?;
                    self.input_used |= cancelled;
                    true
                } else if status.is_some_and(|s| s.phase == MediaPhase::Failed) {
                    bail!("shared movie decoder failed")
                } else {
                    false
                }
            }
            Wait::Audio { channel } => !h.audio_playing(self.client, *channel).map_err(err)?,
            Wait::Reveal { layer } => {
                if pressed {
                    self.input_used = true;
                    let mut ops = vec![json!({"op":"complete_reveal","id":layer})];
                    if !self.query(h, EngineQuery::Layer(layer + 1))?.is_null() {
                        ops.push(json!({"op":"complete_reveal","id":layer+1}));
                    }
                    self.submit(h, json!({"layers":ops}))?;
                    true
                } else {
                    let value = self.query(h, EngineQuery::Layer(*layer))?;
                    let p: crysirror_scene::layer::Paint =
                        serde_json::from_value(value["paint"].clone())?;
                    match p {
                        crysirror_scene::layer::Paint::GlyphRun { glyphs, reveal, .. } => reveal
                            .as_ref()
                            .is_none_or(|r| r.visible_count(glyphs.len()) >= glyphs.len()),
                        _ => bail!("native text layer no longer glyph run"),
                    }
                }
            }
            Wait::Pause { kind, window } => {
                if pressed {
                    self.input_used = true;
                    self.finish_pause(m, h, *kind, *window)?;
                    true
                } else {
                    false
                }
            }
            Wait::Name { local, fields } => {
                let events: Vec<crysirror_scene::ui::UiEvent> =
                    serde_json::from_value(self.query(h, EngineQuery::UiEvents)?)?;
                let submitted = events.iter().any(|e| {
                    e.kind == "submit" && fields.last().is_some_and(|(_, id)| *id == e.widget.id)
                });
                if submitted {
                    let mut values = vec![];
                    for (index, id) in fields {
                        let value = self.query(h, EngineQuery::UiWidget(*id))?;
                        let text = value["text"]
                            .as_str()
                            .context("native name field missing")?
                            .trim()
                            .to_string();
                        if text.is_empty() {
                            bail!("name submission requires nonempty fields");
                        }
                        values.push((*index, text));
                    }
                    for (index, text) in values {
                        m.memory.set_name(*local, index.max(0) as usize, text);
                    }
                    self.submit(h,json!({"ui":fields.iter().flat_map(|(_,id)|[json!({"op":"remove","id":id}),json!({"op":"remove","id":id+1000})]).collect::<Vec<_>>()}))?;
                    true
                } else {
                    false
                }
            }
            Wait::Choice {
                indices,
                ids,
                enabled,
            } => {
                let events: Vec<crysirror_scene::ui::UiEvent> =
                    serde_json::from_value(self.query(h, EngineQuery::UiEvents)?)?;
                let mut pick = events
                    .iter()
                    .find(|e| {
                        matches!(e.kind.as_str(), "activate" | "click")
                            && ids.contains(&e.widget.id)
                    })
                    .and_then(|e| ids.iter().position(|id| *id == e.widget.id));
                for (at, bit) in [
                    buttons::KEY_1,
                    buttons::KEY_2,
                    buttons::KEY_3,
                    buttons::KEY_4,
                    buttons::KEY_5,
                    buttons::KEY_6,
                    buttons::KEY_7,
                    buttons::KEY_8,
                    buttons::KEY_9,
                ]
                .into_iter()
                .enumerate()
                {
                    if input.buttons_pressed & bit != 0 && at < indices.len() && enabled[at] {
                        pick = Some(at);
                    }
                }
                if let Some(at) = pick {
                    m.store = indices[at] as i32;
                    self.input_used = true;
                    self.submit(h,json!({"ui":ids.iter().map(|id|json!({"op":"remove","id":id})).collect::<Vec<_>>()}))?;
                    true
                } else {
                    false
                }
            }
        };
        if !done {
            self.wait = Some(wait);
        }
        Ok(done)
    }
    fn poll_audio(&mut self, h: &mut dyn HostServices) -> Result<bool> {
        let Some((ticket, wait_channel)) = self.pending_audio else {
            return Ok(true);
        };
        let status = h.audio_preparation(self.client, ticket).map_err(err)?;
        match status.phase {
            AudioPreparePhase::Pending => Ok(false),
            AudioPreparePhase::Ready => {
                h.release_audio(self.client, ticket).map_err(err)?;
                self.pending_audio = None;
                if let Some(channel) = wait_channel {
                    self.wait = Some(Wait::Audio { channel });
                }
                self.api_calls += 1;
                Ok(true)
            }
            AudioPreparePhase::Failed => {
                let _ = h.release_audio(self.client, ticket);
                self.pending_audio = None;
                bail!(
                    "shared audio preparation failed: {}",
                    status.error.unwrap_or_default()
                )
            }
        }
    }
    fn execute(
        &mut self,
        h: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        if self.last_error.is_some() {
            bail!("RealLive cannot resume after failed service call");
        }
        if self.machine.is_none() {
            self.boot(h)?;
        }
        self.input_used = false;
        match h.media_info(self.client, 0) {
            Ok(s) if matches!(s.phase, MediaPhase::Ended | MediaPhase::Stopped) => {
                self.stop_movie(h)?
            }
            Ok(s) if s.phase == MediaPhase::Failed => bail!("shared movie decoder failed"),
            Ok(_) | Err(Error::NotFound) => {}
            Err(e) => return Err(err(e)),
        }
        for (i, bit) in [buttons::MOUSE_LEFT, buttons::MOUSE_RIGHT]
            .into_iter()
            .enumerate()
        {
            let down = input.buttons_down & bit != 0;
            let was = self.last_down & bit != 0;
            if down && !was || input.buttons_pressed & bit != 0 {
                self.button_state[i] = 1;
            }
            if !down && was {
                self.button_state[i] = 2;
            }
        }
        let mut m = self.machine.take().unwrap();
        m.begin_frame();
        let result = (|| {
            if self.pending_audio.is_some() {
                if !self.poll_audio(h)? {
                    return Ok(VmYield::BudgetExhausted);
                }
                m.ack(Next::Advance)?;
            }
            if !self.tick_wait(&mut m, h, input)? {
                return Ok(VmYield::BudgetExhausted);
            }
            let before = m.instructions;
            while m.instructions - before < u64::from(budget) {
                let remaining = (u64::from(budget) - (m.instructions - before)) as usize;
                match m.run_budget(remaining)? {
                    RunResult::Halted => return Ok(VmYield::Halted),
                    RunResult::BudgetExhausted => break,
                    RunResult::Event(event) => {
                        let next = match event {
                            VmEvent::Command(c) => {
                                *self.opcode_counts.entry(c.op.to_string()).or_default() += 1;
                                self.command(&mut m, h, input, &c).with_context(|| {
                                    format!(
                                        "{} scene {} line {} parameters {:?}",
                                        c.op,
                                        m.scene_number(),
                                        m.line,
                                        c.params
                                    )
                                })?
                            }
                            VmEvent::Text(text) => {
                                self.text(&mut m, h, &text)?;
                                Next::Advance
                            }
                        };
                        if self.pending_audio.is_some() {
                            return Ok(VmYield::BudgetExhausted);
                        }
                        m.ack(next)?;
                        if self.wait.is_some() && !self.tick_wait(&mut m, h, input)? {
                            break;
                        }
                        if m.yield_frame {
                            break;
                        }
                    }
                }
            }
            Ok(VmYield::BudgetExhausted)
        })();
        self.last_down = input.buttons_down;
        self.frames += 1;
        self.machine = Some(m);
        result
    }
}
fn err(e: Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}
impl VmAdapter for RealLiveVm {
    fn engine_id(&self) -> &'static str {
        "reallive"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        h: &mut dyn HostServices,
        i: InputFrame,
        b: u32,
    ) -> crysirror_core::Result<VmYield> {
        if b == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        match self.execute(h, i, b) {
            Ok(y) => Ok(y),
            Err(e) => {
                let message = format!("{e:#}");
                self.last_error = Some(message.clone());
                Err(Error::Io(message))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("RealLive VM checkpoint not implemented"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("RealLive VM checkpoint not implemented"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"vm":"original-reallive-bytecode","frames":self.frames,"engine_api_calls":self.api_calls,"wait":format!("{:?}",self.wait),"scene":self.machine.as_ref().map(Machine::scene_number),"line":self.machine.as_ref().map(|m|m.line),"instructions":self.machine.as_ref().map(|m|m.instructions),"opcodes":self.opcode_counts,"error":self.last_error,"rendering":"engine retained Canvas/Layer/GlyphRun/Ui/Media","owns_framebuffer":false})).unwrap_or_default()
    }
}
pub struct RealLiveFactory;
impl crysirror_runtime::runtime::VmFactory for RealLiveFactory {
    fn create(
        &self,
        c: ClientId,
        s: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            s.program.is_empty(),
            "RealLive executes original Seen bytecode"
        );
        Ok(Box::new(RealLiveVm::new(
            c,
            serde_json::from_value(s.config.clone())?,
        )?))
    }
}
pub fn register(r: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    r.register("reallive", Arc::new(RealLiveFactory))
}
