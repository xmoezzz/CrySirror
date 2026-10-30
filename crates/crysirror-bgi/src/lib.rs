//! BGI native BP semantic adapter over CrySirror resources, Canvas and Layer.
//! No pixel buffer, rasterizer, font algorithm or audio mixer lives here.
#![forbid(unsafe_code)]
pub mod fixture;
use anyhow::{Context, Result, bail, ensure};
use crysirror_bgi_vm::{Machine, NativeCall, Step};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_runtime::{AdapterRegistry, model::ClientSpec, runtime::VmFactory};
use crysirror_tasks::{Task, TaskPool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Capability-relative game root. Empty selects the client's own root.
    #[serde(default)]
    pub root: String,
    /// Native BP module. Buriko scenario scripts are a separate unsupported ABI.
    pub script: String,
    /// Project font alias for native font slot zero in the plain text profile.
    #[serde(default)]
    pub font: Option<String>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if let Some(font) = &self.font {
            ensure!(
                !font.is_empty() && font.len() <= 256 && !font.chars().any(char::is_control),
                "BGI project font alias"
            );
        }
        ensure!(
            self.script.to_ascii_lowercase().ends_with("._bp"),
            "BGI profile requires a native ._bp module"
        );
        for name in [&self.script]
            .into_iter()
            .chain((!self.root.is_empty()).then_some(&self.root))
        {
            ensure!(
                VfsPath::new(Mount::Asset, name)?.relative() == name,
                "noncanonical BGI path"
            );
        }
        Ok(())
    }
    fn asset(&self, name: &str) -> Result<String> {
        let name = name.replace('\\', "/");
        let full = if self.root.is_empty() {
            name
        } else {
            format!("{}/{}", self.root, name)
        };
        Ok(VfsPath::new(Mount::Asset, &full)?.relative().into())
    }
}
struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "BGI runs native BP bytecode, not data-VM actions"
        );
        Ok(Box::new(BgiVm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("bgi", Arc::new(Factory))
}

#[derive(Clone)]
struct Bitmap {
    size: [u32; 2],
    format: i32,
}
#[derive(Clone, Default)]
struct Sprite {
    bitmap: Option<u32>,
    position: [i32; 2],
    priority: i32,
    opacity: u8,
    visible: bool,
}
/// Native handles and task receipts only. Actual drawing/state belongs to engine
/// Canvas/Layer systems, and files stay behind the client storage capability.
pub struct BgiVm {
    client: ClientId,
    config: Config,
    pool: TaskPool,
    load: Option<Task<Machine>>,
    vm: Option<Machine>,
    bitmaps: BTreeMap<u32, Bitmap>,
    sprites: BTreeMap<u32, Sprite>,
    timer: u64,
    wait: Option<VmYield>,
    calls: u64,
    input: InputFrame,
    audio_ticket: Option<(u32, u64)>,
    playback_ticket: Option<(u32, u64)>,
    music: BTreeMap<u32, (crysirror_core::service::AudioResourceHandle, bool)>,
}
impl BgiVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            pool: TaskPool::new("bgi-resource", 1, 2)?,
            load: None,
            vm: None,
            bitmaps: BTreeMap::new(),
            sprites: BTreeMap::new(),
            timer: 0,
            wait: None,
            calls: 0,
            input: InputFrame::default(),
            audio_ticket: None,
            playback_ticket: None,
            music: BTreeMap::new(),
        })
    }
    fn submit(&self, host: &mut dyn HostServices, batch: Value) -> Result<()> {
        host.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn bitmap(&self, id: i32) -> Result<(u32, &Bitmap)> {
        ensure!((0..0x4000).contains(&id), "BGI bitmap handle");
        Ok((
            id as u32,
            self.bitmaps
                .get(&(id as u32))
                .context("unallocated BGI bitmap")?,
        ))
    }
    fn sprite_id(handle: i32) -> Result<u32> {
        let raw = handle as u32;
        ensure!(raw & 0xfffffe00 == 0x80000000, "BGI sprite handle");
        Ok(raw)
    }
    fn present_sprite(&self, host: &mut dyn HostServices, id: u32, sprite: &Sprite) -> Result<()> {
        let bitmap = sprite.bitmap.context("BGI sprite is not configured")?;
        let id = (id & 0x1ff) + 1;
        self.submit(host,json!({"canvases":[{"op":"present","id":bitmap,"layer":id,"order":sprite.priority,"position":sprite.position}],"layers":[{"op":"set_opacity","id":id,"opacity":sprite.opacity},{"op":"set_visible","id":id,"visible":sprite.visible}]}))
    }
    fn native(
        &mut self,
        host: &mut dyn HostServices,
        vm: &Machine,
        call: &NativeCall,
    ) -> Result<Option<(Vec<i32>, Option<VmYield>)>> {
        let a = &call.args;
        let mut output = Vec::new();
        let mut wait = None;
        match (call.group, call.id) {
            (0x80, 0x04) => output.push(host.now_ms() as u32 as i32),
            (0x80, 0x11) => {
                use crysirror_core::service::buttons;
                let bit = match a[0] {
                    1 => buttons::MOUSE_LEFT,
                    2 => buttons::MOUSE_RIGHT,
                    13 => buttons::ENTER,
                    32 => buttons::SPACE,
                    27 => buttons::ESCAPE,
                    37 => buttons::LEFT,
                    38 => buttons::UP,
                    39 => buttons::RIGHT,
                    40 => buttons::DOWN,
                    _ => bail!("BGI physical key descriptor {} is unmapped", a[0]),
                };
                output.push(i32::from(
                    (self.input.buttons_down | self.input.buttons_pressed) & bit != 0,
                ));
            }

            (0x80, 0x58) => {
                ensure!(a[0] >= 0, "BGI timer duration");
                self.timer = host.now_ms().saturating_add(a[0] as u64);
            }
            (0x80, 0x5a) => {
                let remaining = self.timer.saturating_sub(host.now_ms());
                output.push(i32::from(remaining > 0));
                if remaining > 0 {
                    wait = Some(VmYield::WaitUntil(self.timer));
                }
            }
            (0x80, 0x5f) => wait = Some(VmYield::WaitUntil(host.now_ms().saturating_add(1))),
            (0x80, 0x6a) => wait = Some(VmYield::Halted),
            (0xa0, 0x10) => {
                use crysirror_core::service::{AudioAsset, AudioPreparePhase};
                ensure!(
                    (0..16).contains(&a[0]) && a[2] == 128,
                    "BGI BGM profile supports channels 0..15 at native full volume"
                );
                let channel = a[0] as u32;
                let ticket = if let Some((owner, ticket)) = self.audio_ticket {
                    ensure!(owner == channel, "BGI stale audio admission");
                    ticket
                } else {
                    let asset = VfsPath::new(Mount::Asset, &self.config.asset(&vm.string(a[1])?)?)?;
                    let ticket = host.begin_audio_resource(
                        self.client,
                        AudioAsset {
                            path: asset,
                            frame_limit: None,
                        },
                        false,
                    )?;
                    self.audio_ticket = Some((channel, ticket));
                    ticket
                };
                let prepared = host.audio_resource_preparation(self.client, ticket)?;
                match prepared.phase {
                    AudioPreparePhase::Pending => return Ok(None),
                    AudioPreparePhase::Failed => {
                        host.release_audio_resource_preparation(self.client, ticket)?;
                        self.audio_ticket = None;
                        bail!(
                            "BGI BGM preparation failed: {}",
                            prepared.error.unwrap_or_default()
                        );
                    }
                    AudioPreparePhase::Ready => {
                        let resource = prepared
                            .resource
                            .context("BGI audio preparation has no resource")?;
                        host.release_audio_resource_preparation(self.client, ticket)?;
                        self.audio_ticket = None;
                        if let Some((old, _)) = self.music.insert(channel, (resource, false)) {
                            host.release_audio_resource(self.client, old)?;
                        }
                    }
                }
            }
            (0xa0, 0x14) => {
                use crysirror_core::service::{
                    AudioParameters, AudioPreparePhase, AudioRequest, PcmBus,
                };
                ensure!((0..16).contains(&a[0]), "BGI music channel");
                let channel = a[0] as u32;
                if let Some((resource, started)) = self.music.get_mut(&channel) {
                    if a[1] != 0 && !*started {
                        // Prepared resources still enter playback through the engine's
                        // admission boundary. Keep this native call pending until the
                        // owner publishes its channel preparation result.
                        let ticket = if let Some((owner, ticket)) = self.playback_ticket {
                            ensure!(owner == channel, "BGI stale playback admission");
                            ticket
                        } else {
                            let ticket = host.begin_audio(
                                self.client,
                                AudioRequest::PlayResource {
                                    channel,
                                    resource: *resource,
                                    parameters: AudioParameters {
                                        bus: PcmBus::Music,
                                        ..Default::default()
                                    },
                                },
                            )?;
                            self.playback_ticket = Some((channel, ticket));
                            ticket
                        };
                        let prepared = host.audio_preparation(self.client, ticket)?;
                        match prepared.phase {
                            AudioPreparePhase::Pending => return Ok(None),
                            AudioPreparePhase::Failed => {
                                host.release_audio(self.client, ticket)?;
                                self.playback_ticket = None;
                                bail!(
                                    "BGI channel admission failed: {}",
                                    prepared.error.unwrap_or_default()
                                );
                            }
                            AudioPreparePhase::Ready => {
                                host.release_audio(self.client, ticket)?;
                                self.playback_ticket = None;
                                *started = true;
                            }
                        }
                    } else {
                        host.submit_audio(
                            self.client,
                            AudioRequest::Pause {
                                channel,
                                paused: a[1] == 0,
                                fade_ms: 0.,
                            },
                        )?;
                    }
                }
            }
            (0x90, 0x11) => {
                let id = a[0];
                ensure!((0..0x4000).contains(&id), "BGI bitmap handle");
                let [w, h] = [u32::try_from(a[1])?, u32::try_from(a[2])?];
                ensure!(
                    w > 0 && h > 0 && w <= 8192 && h <= 8192,
                    "BGI bitmap dimensions"
                );
                let format = if a[3] == 7 { 1 } else { a[3] };
                ensure!(
                    matches!(format, 1 | 2),
                    "BGI bitmap format {format} needs an engine pixel profile"
                );
                let color = if format == 1 {
                    [0, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                };
                self.submit(host,json!({"canvases":[{"op":"create","id":id,"width":w,"height":h},{"op":"clear","id":id,"color":color}]}))?;
                self.bitmaps.insert(
                    id as u32,
                    Bitmap {
                        size: [w, h],
                        format,
                    },
                );
            }
            (0x90, 0x13) => {
                let (id, bitmap) = self.bitmap(a[0])?;
                ensure!(
                    matches!(bitmap.format, 1 | 2),
                    "BGI fill needs the decoded resource's native bitmap format metadata"
                );
                let packed = a[1] as u32;
                let color = [
                    (packed >> 16) as u8,
                    (packed >> 8) as u8,
                    packed as u8,
                    if bitmap.format == 1 {
                        255
                    } else {
                        (packed >> 24) as u8
                    },
                ];
                self.submit(
                    host,
                    json!({"canvases":[{"op":"clear","id":id,"color":color}]}),
                )?;
            }
            (0x90, 0x10) => {
                let id = a[0];
                ensure!((0..0x4000).contains(&id), "BGI bitmap handle");
                let archive = vm.string(a[1])?;
                ensure!(
                    archive.is_empty(),
                    "BGI archive selection must be mapped to Resources before loading {archive}"
                );
                let asset = self.config.asset(&vm.string(a[2])?)?;
                let size = host.image_size(self.client, &asset)?;
                self.submit(host,json!({"canvases":[{"op":"create","id":id,"width":size[0],"height":size[1]},{"op":"set","id":id,"layer":{"id":0,"rect":[0,0,size[0],size[1]],"paint":{"kind":"image","asset":asset}}}]}))?;
                self.bitmaps.insert(id as u32, Bitmap { size, format: -1 });
                // The non-cached native image-load form installs a cooperative
                // procedure. Publish this completed engine resource, then let
                // the scheduler resume this thread on its next owner tick.
                wait = Some(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
            }
            (0x92, 0x9c) => {
                use crysirror_text::grid::{CellCursor, CellLayoutOptions, layout_cells};
                let (id, bitmap) = self.bitmap(a[0])?;
                let text = vm.string(a[3])?;
                ensure!(
                    a[6] == 0 || vm.string(a[6])?.is_empty(),
                    "BGI ruby text needs a native text mapping"
                );
                ensure!(
                    a[8] == 0
                        && a[10] == 100
                        && a[11] == 0
                        && a[12] == 0
                        && a[13] == 0
                        && a[14] == 100
                        && a[15] == 0,
                    "BGI text only supports native font 0, plain horizontal 100-percent fixed cells without ruby/kinsoku/shadows"
                );
                ensure!(
                    (4..=200).contains(&a[9]) && a[1] >= 0 && a[2] >= 0,
                    "BGI text size/position"
                );
                ensure!(
                    !text
                        .chars()
                        .any(|c| matches!(c, '\\' | '<' | '>' | '[' | ']' | '{' | '}' | '$')),
                    "BGI markup is not yet mapped"
                );
                let font = self
                    .config
                    .font
                    .as_deref()
                    .context("BGI text requires a project font alias")?;
                let metrics = host.font_metrics(font, a[9] as f32, '字')?;
                let options = CellLayoutOptions {
                    bounds: [bitmap.size[0] as i32, bitmap.size[1] as i32],
                    size: a[9],
                    spacing: [0, 0],
                    half_width: true,
                    half_spacing: false,
                    hanging: 0,
                    forbidden_line_start: String::new(),
                    indent_pairs: vec![],
                };
                let mut cursor = CellCursor::at_start();
                cursor.position = [a[1], a[2]];
                let layout = layout_cells(&text, &options, &mut cursor)?;
                ensure!(
                    layout.consumed == text.len(),
                    "BGI text overflows the supported bitmap profile"
                );
                let packed = a[4] as u32;
                let color = [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8, 255];
                let glyphs=layout.glyphs.into_iter().map(|g|json!({"character":g.character,"baseline":[g.position[0]as f32,g.position[1]as f32+metrics.ascent]})).collect::<Vec<_>>();
                self.submit(host,json!({"canvases":[{"op":"draw","id":id,"layer":{"id":0,"rect":[0,0,bitmap.size[0],bitmap.size[1]],"paint":{"kind":"glyph_run","font":font,"size":a[9],"color":color,"glyphs":glyphs}}}]}))?;
                output.push(1 + (cursor.position[1] - a[2]) / a[9]);
            }
            (0x90, 0x40) => {
                let (id, _) = self.bitmap(a[0])?;
                self.submit(
                    host,
                    json!({"canvases":[{"op":"present","id":id,"layer":1024u32,"order":0}]}),
                )?;
            }
            (0xb0, 0x00) => {
                let (id, _) = self.bitmap(a[2])?;
                self.submit(host,json!({"canvases":[{"op":"present","id":id,"layer":1025u32,"position":[a[0],a[1]],"order":0}]}))?;
            }
            (0x90, 0x50) => {
                let slot = (0..512)
                    .find(|n| !self.sprites.contains_key(&(0x8000_0000 | n)))
                    .context("BGI sprite pool exhausted")?;
                let id = 0x8000_0000 | slot;
                self.sprites.insert(
                    id,
                    Sprite {
                        opacity: 255,
                        visible: true,
                        ..Default::default()
                    },
                );
                output.push(id as i32);
            }
            (0x90, 0x56) => {
                let id = Self::sprite_id(a[0])?;
                let (bitmap, _) = self.bitmap(a[3])?;
                ensure!(
                    a[4] == 128,
                    "BGI native blend selector {} needs a shared blend profile",
                    a[4]
                );
                ensure!(
                    (0..=256).contains(&a[5]) && (0..=65535).contains(&a[6]),
                    "BGI sprite transparency/priority"
                );
                ensure!(self.sprites.contains_key(&id), "unallocated BGI sprite");
                let next = Sprite {
                    bitmap: Some(bitmap),
                    position: [a[1], a[2]],
                    priority: a[6],
                    opacity: ((256 - a[5]) * 255 / 256) as u8,
                    visible: true,
                };
                self.present_sprite(host, id, &next)?;
                self.sprites.insert(id, next);
            }
            (0x90, 0x30) | (0x90, 0x32) | (0x90, 0x33) | (0x90, 0x3a) => {
                let id = Self::sprite_id(a[0])?;
                let mut sprite = self
                    .sprites
                    .get(&id)
                    .context("unallocated BGI sprite")?
                    .clone();
                match call.id {
                    0x30 => sprite.visible = a[1] != 0,
                    0x32 => {
                        ensure!((0..=256).contains(&a[1]), "BGI transparency");
                        sprite.opacity = ((256 - a[1]) * 255 / 256) as u8;
                    }
                    0x33 => sprite.position = [a[1], a[2]],
                    _ => {
                        ensure!((0..65536).contains(&a[1]), "BGI priority");
                        sprite.priority = a[1];
                    }
                }
                self.present_sprite(host, id, &sprite)?;
                self.sprites.insert(id, sprite);
            }
            (0x90, 0x51) => {
                let id = Self::sprite_id(a[0])?;
                ensure!(self.sprites.contains_key(&id), "unallocated BGI sprite");
                if self.sprites[&id].bitmap.is_some() {
                    self.submit(
                        host,
                        json!({"canvases":[{"op":"hide","layer":(id&0x1ff)+1}]}),
                    )?;
                }
                self.sprites.remove(&id);
            }
            _ => bail!(
                "unmapped BGI native call {:#04x}:{:#04x}; missing CrySirror semantic mapping",
                call.group,
                call.id
            ),
        }
        ensure!(
            output.len() == call.outputs,
            "BGI adapter native result count"
        );
        self.calls += 1;
        Ok(Some((output, wait)))
    }
    fn run(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        self.input = input;
        if self.vm.is_none() {
            if self.load.is_none() {
                let storage = host.storage(self.client)?;
                let asset = VfsPath::new(Mount::Asset, &self.config.asset(&self.config.script)?)?;
                self.load = Some(self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    let reader = storage.open_asset(&asset)?;
                    let len = reader.len()?;
                    ensure!(
                        (8..=16 * 1024 * 1024).contains(&len),
                        "BGI BP source budget"
                    );
                    let mut bytes = vec![0; len as usize];
                    let mut offset = 0;
                    while offset < bytes.len() {
                        cancel.check()?;
                        let count = reader.read_at(offset as u64, &mut bytes[offset..])?;
                        ensure!(
                            count > 0 && count <= bytes.len() - offset,
                            "BGI short/invalid source read"
                        );
                        offset += count;
                    }
                    cancel.check()?;
                    Machine::from_bp(&bytes)
                })?);
            }
            let Some(vm) = self.load.as_mut().unwrap().try_take()? else {
                return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
            };
            self.vm = Some(vm);
            self.load = None;
        }
        if let Some(wait) = self.wait {
            match wait {
                VmYield::Halted => return Ok(wait),
                VmYield::WaitUntil(t) if host.now_ms() < t => return Ok(wait),
                VmYield::WaitForInput if input.buttons_pressed == 0 => return Ok(wait),
                _ => self.wait = None,
            }
        }
        let mut vm = self.vm.take().unwrap();
        let result = (|| {
            for _ in 0..budget {
                match vm.step()? {
                    Step::Continue => {}
                    Step::Halted => return Ok(VmYield::Halted),
                    Step::Call(call) => {
                        let Some((results, wait)) = self.native(host, &vm, &call)? else {
                            return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
                        };
                        vm.commit_call(&results)?;
                        if let Some(wait) = wait {
                            self.wait = Some(wait);
                            return Ok(wait);
                        }
                    }
                }
            }
            Ok(VmYield::BudgetExhausted)
        })();
        self.vm = Some(vm);
        result
    }
}
impl VmAdapter for BgiVm {
    fn engine_id(&self) -> &'static str {
        "bgi"
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
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        self.run(host, input, budget)
            .map_err(|e| Error::Io(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("BGI native runtime snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("BGI native runtime snapshot"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"profile":"native-bp-v1","loaded":self.vm.is_some(),"pc":self.vm.as_ref().map(Machine::pc),"native_calls":self.calls,"bitmaps":self.bitmaps.len(),"sprites":self.sprites.len(),"bitmap_sizes":self.bitmaps.iter().take(16).map(|(id,b)|(*id,b.size)).collect::<Vec<_>>()})).unwrap_or_default()
    }
}
