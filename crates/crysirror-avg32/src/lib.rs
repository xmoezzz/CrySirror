// SPDX-License-Identifier: MPL-2.0
//! Original upstream AVG32 Scenario VM, with semantic operations sent to
//! CrySirror's retained Canvas/Layer/GlyphRun/UI and audio services.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail, ensure};
use avg32::{
    nls::Nls,
    portable::{AudioEvent, Event, System},
    scenario::Scenario,
};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{
        AudioParameters, AudioRequest, EncodedAudio, EngineQuery, HostServices, InputFrame, PcmBus,
        buttons,
    },
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_scene::layer::{GlyphReveal, GlyphRevealUnit, PositionedGlyph};
use crysirror_text::grid::{CellCursor, CellLayoutOptions, layout_cells};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

fn font() -> String {
    "default".into()
}
fn encoding() -> String {
    "sjis".into()
}
fn version() -> i32 {
    1604
}
fn entry() -> i32 {
    1
}
fn bounds() -> [i32; 4] {
    [20, 340, 600, 120]
}
fn size() -> u16 {
    24
}
fn interval() -> i32 {
    30
}
fn cell_step() -> [i32; 2] {
    [12, 26]
}
fn profile() -> String {
    "plain-adv-v1".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Explicit native profile: non-novel, no custom frame/effects, plain text.
    #[serde(default = "profile")]
    pub profile: String,
    /// Native scene number to capability-relative TPC32 file.
    pub scenes: BTreeMap<i32, String>,
    /// Scene number used by the original Scenario at boot.
    #[serde(default = "entry")]
    pub start_scene: i32,
    /// Original opcode revision; choose explicitly for the title being adapted.
    #[serde(default = "version")]
    pub version: i32,
    /// Native string codec accepted by the original NLS parser (default sjis).
    #[serde(default = "encoding")]
    pub encoding: String,
    /// Installed engine font alias; no font file or native font is bundled.
    #[serde(default = "font")]
    pub font: String,
    /// Absolute position and cell-layout width/height in logical pixels.
    #[serde(default = "bounds")]
    pub message_rect: [i32; 4],
    /// Native nominal glyph size in logical pixels (1..256).
    #[serde(default = "size")]
    pub font_size: u16,
    /// Native horizontal half-cell and line pitch in pixels; never inferred from font metrics.
    #[serde(default = "cell_step")]
    pub cell_step: [i32; 2],
    /// Original interpreter print interval in milliseconds (0 displays immediately).
    #[serde(default = "interval")]
    pub message_interval_ms: i32,
    /// Native resource names to engine image/audio paths. No filesystem bypass.
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    /// Explicit native SE table; a referenced missing entry fails visibly.
    #[serde(default)]
    pub sound_effects: BTreeMap<i32, String>,
    /// Native voice IDs mapped to shared supported audio containers.
    #[serde(default)]
    pub voices: BTreeMap<i32, String>,
    /// Explicit engine image used for native message-wait icons.
    #[serde(default)]
    pub message_icon: Option<String>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.profile == "plain-adv-v1",
            "unsupported AVG32 native profile"
        );
        ensure!(
            (1604..=1714).contains(&self.version),
            "unsupported AVG32 opcode revision"
        );
        ensure!(
            self.scenes.len() <= 256 && self.scenes.contains_key(&self.start_scene),
            "AVG32 scene table/start"
        );
        ensure!(
            !self.font.is_empty()
                && self.font.len() <= 256
                && !self.font.chars().any(char::is_control),
            "AVG32 font alias"
        );
        ensure!(
            (1..=256).contains(&self.font_size) && (0..=60000).contains(&self.message_interval_ms),
            "AVG32 message metrics"
        );
        ensure!(
            self.message_rect[..2]
                .iter()
                .all(|v| i64::from(*v).abs() <= 1_000_000)
                && self.message_rect[2..]
                    .iter()
                    .all(|v| (1..=32767).contains(v)),
            "AVG32 message bounds"
        );
        ensure!(
            self.cell_step.iter().all(|v| (1..=2048).contains(v)),
            "AVG32 native cell advance"
        );
        self.encoding.parse::<Nls>()?;
        ensure!(
            self.resources.len() <= 4096
                && self.sound_effects.len() <= 256
                && self.voices.len() <= 4096,
            "AVG32 resource map limit"
        );
        for path in self
            .scenes
            .values()
            .chain(self.resources.values())
            .chain(self.sound_effects.values())
            .chain(self.voices.values())
            .chain(self.message_icon.iter())
        {
            let p = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(p.relative() == path, "noncanonical AVG32 resource path");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Machine {
    scenario: Scenario,
    system: System,
    clock_origin_ms: u64,
    pending: Option<Event>,
    glyphs: Vec<PositionedGlyph>,
    cursor: CellCursor,
    visible: usize,
    message_visible: bool,
    icon_visible: bool,
    choices: Vec<(u32, bool)>,
    choice_window: bool,
    waves: BTreeMap<i32, u32>,
    api_calls: u64,
}
pub struct Avg32Vm {
    client: ClientId,
    config: Config,
    machine: Option<Machine>,
    last_buttons: u64,
}
fn error(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}
const TEXT: u32 = 40_000;
const ICON: u32 = 40_001;
const CHOICE: u32 = 41_000;
const MUSIC: u32 = 20_000;
const VOICE: u32 = 20_001;
const SE: u32 = 20_002;
fn horizontal_spacing(config: &Config) -> i32 {
    2 * config.cell_step[0] - i32::from(config.font_size)
}
impl Avg32Vm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            machine: None,
            last_buttons: 0,
        })
    }
    fn read(&self, host: &mut dyn HostServices, path: &str) -> Result<Vec<u8>> {
        host.read(
            self.client,
            &VfsPath::new(Mount::Asset, path).map_err(error)?,
        )
        .map_err(error)
        .map_err(Into::into)
    }
    fn scenes(&self, host: &mut dyn HostServices) -> Result<BTreeMap<i32, Vec<u8>>> {
        let mut total = 0usize;
        let mut scenes = BTreeMap::new();
        for (&id, path) in &self.config.scenes {
            let bytes = self.read(host, path)?;
            total = total
                .checked_add(bytes.len())
                .context("AVG32 scene bytes overflow")?;
            ensure!(total <= 64 * 1024 * 1024, "AVG32 scene resource budget");
            avg32::scene::Avg32SceneHeader::parse(&bytes)?;
            scenes.insert(id, bytes);
        }
        Ok(scenes)
    }
    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let mut system = System::default();
        // Match the original native System's initial scenario RNG state.
        system.random_seed = 0x853c_49e6_748f_ea9b;
        system.version = self.config.version;
        system.nls = self.config.encoding.parse()?;
        system.ini.mes_wait = self.config.message_interval_ms;
        system.scenes = self.scenes(host)?;
        let scenario = Scenario::new(&mut system, self.config.start_scene);
        ensure!(
            system.error.is_none(),
            "native AVG32 boot: {:?}",
            system.error
        );
        let operations = (0..32)
            .map(|id| json!({"op":"create","id":id,"width":640,"height":480}))
            .collect::<Vec<_>>();
        host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"canvases":operations}))?,
        )
        .map_err(error)?;
        self.machine = Some(Machine {
            scenario,
            system,
            clock_origin_ms: host.now_ms(),
            pending: None,
            glyphs: Vec::new(),
            cursor: CellCursor::at_start(),
            visible: 0,
            message_visible: false,
            icon_visible: false,
            choices: Vec::new(),
            choice_window: false,
            waves: BTreeMap::new(),
            api_calls: 1,
        });
        Ok(())
    }
    fn submit(
        client: ClientId,
        m: &mut Machine,
        host: &mut dyn HostServices,
        batch: Value,
    ) -> Result<()> {
        host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)
            .map_err(error)?;
        m.api_calls += 1;
        Ok(())
    }
    fn text_layer(
        client: ClientId,
        config: &Config,
        m: &mut Machine,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        let reveal = GlyphReveal {
            unit: GlyphRevealUnit::UnicodeScalar,
            interval_ms: 1,
            initial_visible: m.visible,
            elapsed_ms: 0,
            forced_complete: false,
            paused: true,
        };
        Self::submit(
            client,
            m,
            host,
            json!({"layers":[{"op":"put","layer":{"id":TEXT,"rect":config.message_rect,"visible":m.message_visible,"order":[1000,0],"paint":{"kind":"glyph_run","font":config.font,"size":config.font_size,"glyphs":m.glyphs,"reveal":reveal}}}]}),
        )
    }
    fn decode(config: &Config, bytes: &[u8]) -> Result<String> {
        config
            .encoding
            .parse::<Nls>()?
            .decode_strict(bytes)
            .context("invalid native text encoding")
    }
    fn asset<'a>(config: &'a Config, name: &str) -> Result<&'a str> {
        config
            .resources
            .get(name)
            .map(String::as_str)
            .with_context(|| format!("AVG32 resource alias {name:?} is absent"))
    }
    fn play(
        client: ClientId,
        config: &Config,
        m: &mut Machine,
        host: &mut dyn HostServices,
        channel: u32,
        path: &str,
        looping: bool,
        fade: u32,
        bus: PcmBus,
    ) -> Result<()> {
        let p = VfsPath::new(Mount::Asset, path).map_err(error)?;
        let bytes = host.read(client, &p).map_err(error)?;
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "AVG32 audio resource budget"
        );
        let _ = config;
        host.submit_audio(
            client,
            AudioRequest::Play {
                channel,
                source: EncodedAudio {
                    bytes: Arc::from(bytes),
                    frame_limit: None,
                },
                intro: None,
                parameters: AudioParameters {
                    looping,
                    fade_ms: fade as f32,
                    bus,
                    ..Default::default()
                },
            },
        )
        .map_err(error)?;
        m.api_calls += 1;
        Ok(())
    }
    fn event(
        client: ClientId,
        config: &Config,
        m: &mut Machine,
        host: &mut dyn HostServices,
        event: &Event,
    ) -> Result<()> {
        match event {
            Event::Message(packet) => {
                ensure!(
                    !m.system.mes.double_text,
                    "AVG32 doubled text needs a host profile"
                );
                let (&kind, bytes) = packet
                    .split_first()
                    .context("empty native message packet")?;
                ensure!(
                    matches!(kind, 0xfe | 0xff | 0x0d),
                    "AVG32 message control {kind:#x} needs a host mapping"
                );
                let text = if kind == 0x0d {
                    "\n".into()
                } else {
                    Self::decode(config, bytes)?
                };
                ensure!(
                    text.len() <= 65536
                        && !text
                            .chars()
                            .any(|c| (c.is_control() && c != '\n') || "＊【】「」『』".contains(c)),
                    "AVG32 inline name/icon/indent/control needs a host mapping"
                );
                let options = CellLayoutOptions {
                    bounds: [config.message_rect[2], config.message_rect[3]],
                    size: i32::from(config.font_size),
                    spacing: [
                        self::horizontal_spacing(config),
                        config.cell_step[1] - i32::from(config.font_size),
                    ],
                    half_width: kind == 0xfe,
                    half_spacing: true,
                    hanging: 0,
                    forbidden_line_start: String::new(),
                    indent_pairs: Vec::new(),
                };
                let mut cursor = m.cursor.clone();
                let layout = layout_cells(&text, &options, &mut cursor)?;
                ensure!(
                    layout.consumed == text.len()
                        && cursor.position[1] == m.cursor.position[1]
                        && !text.contains('\n'),
                    "AVG32 wrapping/paragraph needs a host profile"
                );
                let ascent = f32::from(config.font_size);
                let mut glyphs = m.glyphs.clone();
                glyphs.extend(layout.glyphs.into_iter().map(|g| PositionedGlyph {
                    character: g.character,
                    baseline: [g.position[0] as f32, g.position[1] as f32 + ascent],
                    size: None,
                    color: None,
                    bold: None,
                }));
                ensure!(glyphs.len() <= 65536, "AVG32 glyph budget");
                let old_glyphs = std::mem::replace(&mut m.glyphs, glyphs);
                let old_visible = m.message_visible;
                m.message_visible = true;
                if let Err(e) = Self::text_layer(client, config, m, host) {
                    m.glyphs = old_glyphs;
                    m.message_visible = old_visible;
                    return Err(e);
                }
                m.cursor = cursor;
            }
            Event::Print { all } => {
                let old = m.visible;
                m.visible = if *all || config.message_interval_ms == 0 {
                    m.glyphs.len()
                } else {
                    (m.visible + 1).min(m.glyphs.len())
                };
                if let Err(e) = Self::text_layer(client, config, m, host) {
                    m.visible = old;
                    return Err(e);
                }
                m.scenario
                    .complete_message_print(i32::from(m.visible < m.glyphs.len()))?;
            }
            Event::ClearMessage | Event::LineFeed => {
                let old = (m.glyphs.clone(), m.cursor.clone(), m.visible);
                m.glyphs.clear();
                m.cursor = CellCursor::at_start();
                m.visible = 0;
                if let Err(e) = Self::text_layer(client, config, m, host) {
                    (m.glyphs, m.cursor, m.visible) = old;
                    return Err(e);
                }
            }
            Event::HideMessage => {
                let old = m.message_visible;
                m.message_visible = false;
                if !m.glyphs.is_empty() {
                    if let Err(e) = Self::text_layer(client, config, m, host) {
                        m.message_visible = old;
                        return Err(e);
                    }
                }
            }
            Event::MessageIcon(icon) => {
                if icon.is_some() {
                    let path = config
                        .message_icon
                        .as_ref()
                        .context("AVG32 message icon requires an explicit engine asset mapping")?;
                    let size = host.image_size(client, path).map_err(error)?;
                    Self::submit(
                        client,
                        m,
                        host,
                        json!({"layers":[{"op":"put","layer":{"id":ICON,"rect":[config.message_rect[0]+config.message_rect[2]-size[0] as i32,config.message_rect[1]+config.message_rect[3]-size[1] as i32,size[0],size[1]],"order":[1001,0],"paint":{"kind":"image","asset":path}}}]}),
                    )?;
                    m.icon_visible = true;
                } else if m.icon_visible {
                    Self::submit(
                        client,
                        m,
                        host,
                        json!({"layers":[{"op":"remove","id":ICON}]}),
                    )?;
                    m.icon_visible = false;
                }
            }
            Event::LoadImage { name, buffer } => {
                ensure!(
                    (0..32).contains(buffer),
                    "AVG32 negative/invalid image buffer requires explicit presentation policy"
                );
                let name = Self::decode(config, name)?;
                let asset = Self::asset(config, &name)?;
                ensure!(
                    host.image_size(client, asset).map_err(error)? == [640, 480]
                        && host.image_is_opaque(client, asset).map_err(error)?,
                    "AVG32 partial/masked image loads require associated coverage canvas operations"
                );
                let mut ops = vec![
                    json!({"op":"set","id":buffer,"layer":{"id":0,"rect":[0,0,640,480],"paint":{"kind":"image","asset":asset}}}),
                ];
                if *buffer == 0 {
                    ops.push(json!({"op":"present","id":0,"layer":0}));
                }
                Self::submit(client, m, host, json!({"canvases":ops}))?;
            }
            Event::ChoiceClear => {
                let ops = m
                    .choices
                    .iter()
                    .filter(|(_, visible)| *visible)
                    .map(|(id, _)| json!({"op":"remove","id":id}))
                    .collect::<Vec<_>>();
                Self::submit(client, m, host, json!({"ui":ops}))?;
                m.choices.clear();
            }
            Event::Choice {
                text,
                enabled,
                color,
            } => {
                ensure!(
                    *color == 0,
                    "AVG32 colored choices need an explicit palette/text-color mapping"
                );
                ensure!(m.choices.len() < 128, "AVG32 choice limit");
                let id = CHOICE + m.choices.len() as u32;
                if *enabled >= 0 {
                    Self::submit(
                        client,
                        m,
                        host,
                        json!({"ui":[{"op":"put","widget":{"id":id,"kind":"button","rect":[config.message_rect[0],config.message_rect[1]+m.choices.iter().filter(|(_,visible)|*visible).count() as i32*32,config.message_rect[2],30],"text":Self::decode(config,text)?,"font":config.font,"text_size":config.font_size,"enabled":*enabled>0}}]}),
                    )?;
                }
                m.choices.push((id, *enabled >= 0));
            }
            Event::ChoiceWindow(open) => {
                if *open {
                    Self::submit(
                        client,
                        m,
                        host,
                        json!({"ui":[{"op":"put","widget":{"id":CHOICE-1,"kind":"panel","rect":config.message_rect,"color":[20,24,32,220],"z":-1}}]}),
                    )?;
                } else if m.choice_window {
                    let mut ops = m
                        .choices
                        .iter()
                        .filter(|(_, visible)| *visible)
                        .map(|(id, _)| json!({"op":"remove","id":id}))
                        .collect::<Vec<_>>();
                    ops.push(json!({"op":"remove","id":CHOICE-1}));
                    Self::submit(client, m, host, json!({"ui":ops}))?;
                    m.choices.clear();
                }
                m.choice_window = *open;
            }
            Event::Title(title) => Self::submit(
                client,
                m,
                host,
                json!({"ui":[{"op":"set_title","text":Self::decode(config,title)?}]}),
            )?,
            Event::SoundEffect(id) => {
                let asset = config
                    .sound_effects
                    .get(id)
                    .with_context(|| format!("AVG32 SE {id} has no engine audio mapping"))?;
                Self::play(client, config, m, host, SE, asset, false, 0, PcmBus::Ui)?;
            }
            Event::Audio(audio) => match audio {
                AudioEvent::Bgm {
                    name,
                    looping,
                    fade_ms,
                } => Self::play(
                    client,
                    config,
                    m,
                    host,
                    MUSIC,
                    Self::asset(config, name)?,
                    *looping,
                    *fade_ms,
                    PcmBus::Music,
                )?,
                AudioEvent::Wave {
                    name,
                    looping,
                    channel,
                } => {
                    ensure!((-1..=255).contains(channel), "AVG32 wave channel");
                    let handle = 21_000 + (*channel + 1) as u32;
                    Self::play(
                        client,
                        config,
                        m,
                        host,
                        handle,
                        Self::asset(config, name)?,
                        *looping,
                        0,
                        PcmBus::Sfx,
                    )?;
                    m.waves.insert(*channel, handle);
                }
                AudioEvent::Voice(id) => {
                    let asset = config
                        .voices
                        .get(id)
                        .context("AVG32 voice resource mapping absent")?;
                    Self::play(
                        client,
                        config,
                        m,
                        host,
                        VOICE,
                        asset,
                        false,
                        0,
                        PcmBus::Voice,
                    )?;
                }
                AudioEvent::StopBgm { fade_ms } => {
                    host.submit_audio(
                        client,
                        AudioRequest::Stop {
                            channel: MUSIC,
                            fade_ms: *fade_ms as f32,
                        },
                    )
                    .map_err(error)?;
                    m.api_calls += 1;
                }
                AudioEvent::StopWave { channel } => {
                    let channels = match channel {
                        Some(id) => m.waves.get(id).copied().into_iter().collect::<Vec<_>>(),
                        None => m.waves.values().copied().collect(),
                    };
                    for channel in channels {
                        host.submit_audio(
                            client,
                            AudioRequest::Stop {
                                channel,
                                fade_ms: 0.,
                            },
                        )
                        .map_err(error)?;
                        m.api_calls += 1;
                    }
                }
            },
        }
        Ok(())
    }
    fn run(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        if self.machine.is_none() {
            self.boot(host)?;
        }
        let m = self.machine.as_mut().unwrap();
        m.system.time = host.now_ms().saturating_sub(m.clock_origin_ms);
        m.system.skip_pressed = input.buttons_down & buttons::CTRL != 0;
        let new = (input.buttons_down & !self.last_buttons) | input.buttons_pressed;
        self.last_buttons = input.buttons_down;
        if new & (buttons::ENTER | buttons::SPACE | buttons::MOUSE_LEFT) != 0 {
            m.system
                .mouse
                .press(input.pointer_x, input.pointer_y, 0, m.system.time);
        }
        m.system.sound.bgm_active = host.audio_playing(self.client, MUSIC).map_err(error)?;
        m.system.sound.voice_active = host.audio_playing(self.client, VOICE).map_err(error)?;
        m.system.sound.wave_active = false;
        for &channel in m.waves.values() {
            m.system.sound.wave_active |=
                host.audio_playing(self.client, channel).map_err(error)?;
        }
        if !m.choices.is_empty() {
            let events: Value = serde_json::from_slice(
                &host
                    .query_engine(self.client, EngineQuery::UiEvents)
                    .map_err(error)?,
            )?;
            if let Some(events) = events.as_array() {
                for e in events {
                    if e["kind"] == "activate" {
                        if let Some(id) = e["widget"]["id"].as_u64() {
                            if let Some(index) = m
                                .choices
                                .iter()
                                .position(|(choice, visible)| *choice == id as u32 && *visible)
                            {
                                m.system.choice_result = Some(index as i32 + 1);
                            }
                        }
                    }
                }
            }
        }
        let before = m.system.instructions;
        let limit = budget.min(100_000) as u64;
        loop {
            if m.pending.is_none() {
                m.pending = m.system.next_event();
            }
            if let Some(event) = m.pending.clone() {
                Self::event(self.client, &self.config, m, host, &event)?;
                m.pending = None;
                continue;
            }
            if let Some(error) = &m.system.error {
                bail!("native AVG32: {error}");
            }
            if !m.system.running {
                return Ok(VmYield::Halted);
            }
            let used = m.system.instructions.saturating_sub(before);
            if used >= limit {
                return Ok(VmYield::BudgetExhausted);
            }
            if m.system.frame_yielded {
                m.system.frame_yielded = false;
                return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
            }
            m.scenario.run_budget(&mut m.system, (limit - used) as u32);
        }
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
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.run(host, input, budget).map_err(error)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "AVG32 portable checkpoints require native state validation",
        ))
    }
    fn restore(&mut self, _bytes: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "AVG32 portable checkpoints require native state validation",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"vm":"original-avg32-scenario","profile":self.config.profile,"owns_framebuffer":false,"engine_api_calls":self.machine.as_ref().map_or(0,|m|m.api_calls),"scene":self.machine.as_ref().map(|m|m.scenario.seen),"position":self.machine.as_ref().map(|m|m.scenario.pos),"instructions":self.machine.as_ref().map_or(0,|m|m.system.instructions),"values":self.machine.as_ref().map(|m|(0..16).map(|i|m.system.flags.value(i)).collect::<Vec<_>>()),"error":self.machine.as_ref().and_then(|m|m.system.error.clone())})).unwrap_or_default()
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
            "AVG32 executes original TPC32 bytecode"
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
