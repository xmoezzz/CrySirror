// SPDX-License-Identifier: MPL-2.0
mod audio;
mod audio_asset;
mod buttons;
mod cursor;
mod events;
mod input;
mod movie;
pub(crate) mod object_movie;
mod presentation;
mod proc;
use crate::{
    io_fs, platform_time,
    runtime::CommandContext,
    scene_pck::{ScenePck, ScenePckDecodeOptions, SceneStringCodec},
    scene_stream::SceneStream,
    vm::SceneVm,
};
use anyhow::{Context, Result, ensure};
use bincode::Options;
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
fn codec() -> String {
    "xor".into()
}
fn format() -> String {
    "pck".into()
}
const OPTICAL_DISC_POLICY: &str = "disabled-for-native-host";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Asset path of the compiled scene: a `Scene.pck` or a single `.scn`.
    pub scene: String,
    /// "pck" (scene pack, default) or "scn" (one scene stream).
    #[serde(default = "format")]
    pub format: String,
    /// Entry scene override; otherwise Gameexe START_SCENE (or _start),
    /// or scene 0 for an explicitly unconfigured fixture pack.
    #[serde(default)]
    pub scene_name: Option<String>,
    /// Scene string encryption: "xor" (default) or "plain".
    #[serde(default = "codec")]
    pub string_codec: String,
    /// Asset path of a UTF-8 Gameexe ini or encoded Gameexe.dat, if any.
    #[serde(default)]
    pub gameexe: Option<String>,
    /// Capability-relative key.toml containing caller-owned scene/Gameexe
    /// decode materials. Keys are never guessed or embedded in the adapter.
    #[serde(default)]
    pub key_file: Option<String>,
    /// Native resource path → asset path aliases (at most 4096).
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    /// Explicit mapping for the bounded plain-text MWND profile. If omitted,
    /// supported horizontal Gameexe windows are translated on first use;
    /// unmapped rich/native profiles fail rather than guessing canvas geometry.
    #[serde(default)]
    pub message_window: Option<crate::MessageWindow>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        fn path(p: &str) -> Result<()> {
            let v = VfsPath::new(Mount::Asset, p).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(v.relative() == p, "non-canonical native config path");
            Ok(())
        }
        path(&self.scene)?;
        ensure!(
            matches!(self.format.as_str(), "scn" | "pck")
                && matches!(self.string_codec.as_str(), "plain" | "xor"),
            "unsupported native Siglus format/codec"
        );
        if let Some(name) = &self.scene_name {
            ensure!(
                !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control),
                "native scene name"
            );
        }
        ensure!(self.resources.len() <= 4096, "native resource alias count");
        for (a, b) in &self.resources {
            path(a)?;
            path(b)?;
        }
        if let Some(k) = &self.key_file {
            path(k)?;
        }
        if let Some(g) = &self.gameexe {
            path(g)?;
        }
        if let Some(window) = &self.message_window {
            window.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Machine {
    client: u32,
    optical_disc_policy: String,
    config: Config,
    vm: Option<SceneVm<'static>>,
    #[serde(skip)]
    io: io_fs::State,
    inputs: crysirror_scene::input_buffer::InputInbox,
    pump_continuation: bool,
    now_ms: u64,
    event_cursors: BTreeMap<events::Key, events::Record>,
    event_frame: Option<events::Frame>,
    event_clock_ms: Option<u64>,
    published: BTreeSet<u32>,
}
pub struct SiglusVm {
    state: Machine,
    pending_audio: Option<audio::Pending>,
    events: events::Events,
    cursors: cursor::Cursors,
    presentation: presentation::Presentation,
    focus_observation: Option<bool>,
    buttons: buttons::Buttons,
    object_movies: object_movie::ObjectMovies,
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::Io(format!("{e:#}"))
}
fn binary() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(32 * 1024 * 1024)
        .reject_trailing_bytes()
}
impl SiglusVm {
    fn apply_activity_observation(&mut self) {
        if let Some(vm) = self.state.vm.as_mut() {
            vm.ctx.globals.system.active_flag = self.focus_observation.unwrap_or(true);
        }
    }
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            state: Machine {
                client: client.0,
                optical_disc_policy: OPTICAL_DISC_POLICY.into(),
                config,
                vm: None,
                io: Default::default(),
                inputs: crysirror_scene::input_buffer::InputInbox(
                    crysirror_core::input_buffer::InputBuffer::new(client.0, 1, 256)?,
                ),
                pump_continuation: false,
                now_ms: 0,
                event_cursors: BTreeMap::new(),
                event_frame: None,
                event_clock_ms: None,
                published: Default::default(),
            },
            pending_audio: None,
            events: Default::default(),
            cursors: Default::default(),
            presentation: Default::default(),
            focus_observation: None,
            buttons: Default::default(),
            object_movies: Default::default(),
        })
    }
    fn path(key: &str) -> Result<VfsPath> {
        let (m, p) = key.split_once('/').context("native mount missing")?;
        VfsPath::new(
            match m {
                "asset" => Mount::Asset,
                "save" => Mount::Save,
                _ => anyhow::bail!("invalid native mount"),
            },
            p,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn fetch(&mut self, host: &mut dyn HostServices, key: &str) -> Result<()> {
        let path = Self::path(key)?;
        let alias = if path.mount() == Mount::Asset {
            self.state.config.resources.get(path.relative()).cloned()
        } else {
            None
        };
        let source = if let Some(p) = alias {
            VfsPath::new(Mount::Asset, &p).map_err(err)?
        } else {
            path
        };
        match host.read(ClientId(self.state.client), &source) {
            Ok(b) => {
                let total = self.state.io.files.values().map(|b| b.len()).sum::<usize>();
                ensure!(
                    b.len() <= 64 * 1024 * 1024 && total + b.len() <= 256 * 1024 * 1024,
                    "native asset cache limit"
                );
                self.state.io.files.insert(key.into(), Arc::new(b));
            }
            Err(Error::NotFound) => {
                self.state.io.absent.insert(key.into());
            }
            Err(e) => return Err(anyhow::anyhow!("native resource {key}: {e}")),
        }
        Ok(())
    }
    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let key = format!("asset/{}", self.state.config.scene);
        self.fetch(host, &key)?;
        let bytes = self
            .state
            .io
            .files
            .get(&key)
            .context("native scene resource missing")?
            .as_ref()
            .clone();
        let codec = if self.state.config.string_codec == "plain" {
            SceneStringCodec::Plain
        } else {
            SceneStringCodec::Xor
        };
        let materials = if let Some(path) = self.state.config.key_file.clone() {
            let key = format!("asset/{path}");
            self.fetch(host, &key)?;
            let data = self
                .state
                .io
                .files
                .remove(&key)
                .context("Siglus key resource missing")?;
            ensure!(
                data.len() <= 64 * 1024,
                "Siglus key resource exceeds 64 KiB"
            );
            let parsed = crate::key_toml::parse_key_toml(std::str::from_utf8(&data)?)?;
            for table in [
                &parsed.base_angou_code,
                &parsed.game_angou_code,
                &parsed.scene_angou_code,
            ] {
                if let Some(table) = table {
                    ensure!(
                        !table.is_empty() && table.len() <= 4096,
                        "Siglus decode table length must be 1..=4096"
                    );
                }
            }
            parsed
        } else {
            crate::key_toml::KeyTomlConfig::default()
        };
        let mut ctx = CommandContext::new(PathBuf::from("asset"));
        ctx.ui.window = self.state.config.message_window.clone();
        if let Some(name) = self.state.config.gameexe.clone() {
            let k = format!("asset/{name}");
            self.fetch(host, &k)?;
            let b = self
                .state
                .io
                .files
                .get(&k)
                .context("Gameexe resource missing")?;
            let text = if let Ok(text) = std::str::from_utf8(b)
                && !text.contains('\0')
                && text.trim_start().starts_with(['#', ';', '['])
            {
                text.to_owned()
            } else {
                let options = crate::gameexe::GameexeDecodeOptions {
                    exe_key16: materials.exe_key16,
                    base_angou_code: materials.base_angou_code.clone(),
                    game_angou_code: materials.game_angou_code.clone(),
                    chain_order: materials.chain_order.clone().unwrap_or_else(|| {
                        crate::gameexe::GameexeDecodeOptions::default().chain_order
                    }),
                    ..Default::default()
                };
                crate::gameexe::decode_gameexe_dat_bytes(b, &options)
                    .context("native Gameexe decode")?
                    .0
            };
            let gameexe = crate::gameexe::GameexeConfig::from_text(&text);
            if let Some(size) = gameexe.get_entry("SCREEN_SIZE") {
                let width: u32 = size
                    .item_unquoted(0)
                    .context("SCREEN_SIZE width")?
                    .parse()?;
                let height: u32 = size
                    .item_unquoted(1)
                    .context("SCREEN_SIZE height")?
                    .parse()?;
                ensure!(
                    (1..=8192).contains(&width) && (1..=8192).contains(&height),
                    "native screen size limit"
                );
                ctx.screen_w = width;
                ctx.screen_h = height;
            }
            ctx.tables.gameexe = Some(Arc::new(gameexe));
        }
        crate::character_voice::initialize(&mut ctx);
        crate::runtime::forms::syscom::initialize_config(&mut ctx);
        let mut vm = if self.state.config.format == "scn" {
            let stream = SceneStream::new_owned_with_string_codec(Arc::from(bytes), codec)?;
            let mut vm = SceneVm::new(stream, ctx);
            vm.engine_set_entry(
                self.state
                    .config
                    .scene_name
                    .clone()
                    .unwrap_or_else(|| "entry".into()),
            );
            vm
        } else {
            let mut options = ScenePckDecodeOptions {
                exe_angou_element: materials.exe_key16.map(|key| key.to_vec()),
                easy_angou_code: materials.scene_angou_code,
                ..Default::default()
            };
            options.string_encryption_override = if self.state.config.string_codec == "plain" {
                crate::key_toml::StringEncryptionOverride::None
            } else {
                crate::key_toml::StringEncryptionOverride::Xor
            };
            let header =
                crate::scene_pck::PackScnHeader::read(&bytes, 0, bytes.starts_with(b"pack_scn"))?;
            ensure!(
                header.scn_data_exe_angou_mod == 0 || options.exe_angou_element.is_some(),
                "Scene.pck requires executable decode material; configure a client-scoped key_file"
            );
            let pck = ScenePck::load_lazy_from_bytes(bytes, &options)?;
            let (name, entry_z) = if let Some(name) = self.state.config.scene_name.clone() {
                (name, 0)
            } else if let Some(gameexe) = &ctx.tables.gameexe {
                match gameexe.get_entry("START_SCENE") {
                    Some(entry) => {
                        let name = entry
                            .item_unquoted(0)
                            .context("START_SCENE name")?
                            .trim()
                            .to_owned();
                        ensure!(!name.is_empty(), "empty native START_SCENE");
                        let z = entry
                            .item_unquoted(1)
                            .map(str::parse::<i32>)
                            .transpose()?
                            .unwrap_or(0);
                        ensure!(z >= 0, "negative native START_SCENE label");
                        (name, z)
                    }
                    None => ("_start".into(), 0),
                }
            } else {
                (pck.find_scene_name(0).unwrap_or("0").to_owned(), 0)
            };
            let n = pck
                .find_scene_no(&name)
                .context("native entry scene not found")?;
            let (owner, range) = pck.scn_data_shared(n)?;
            let stream =
                SceneStream::new_shared_range_with_string_codec(owner, range, pck.string_codec)?;
            ctx.install_scene_metadata("", &pck)?;
            let mut vm = SceneVm::new(stream, ctx);
            vm.install_initial_scene_pck(pck, String::new());
            vm.restart_scene_name(&name, entry_z)?;
            vm
        };
        vm.cfg.max_steps = 1_000_000;
        vm.ctx.globals.script.cursor_no = cursor::default_no(&vm.ctx)?;
        vm.engine_validate()?;
        self.state.vm = Some(vm);
        Ok(())
    }
    fn resolve_io(
        &mut self,
        host: &mut dyn HostServices,
        keys: BTreeSet<String>,
        probes: BTreeSet<String>,
    ) -> Result<()> {
        for key in keys {
            self.fetch(host, &key)?;
        }
        for key in probes {
            if key == "asset/" || key == "save/" {
                let mount = if key == "asset/" {
                    Mount::Asset
                } else {
                    Mount::Save
                };
                let storage = host.storage(ClientId(self.state.client)).map_err(err)?;
                let present = storage.metadata(mount, "").map_err(err)?.is_some();
                self.state.io.probes.insert(key, present);
                continue;
            }
            let path = Self::path(&key)?;
            let source = if path.mount() == Mount::Asset {
                self.state
                    .config
                    .resources
                    .get(path.relative())
                    .map(|name| VfsPath::new(Mount::Asset, name).map_err(err))
                    .transpose()?
                    .unwrap_or(path)
            } else {
                path
            };
            let storage = host.storage(ClientId(self.state.client)).map_err(err)?;
            let metadata = storage
                .metadata(source.mount(), source.relative())
                .map_err(err)?;
            self.state.io.probes.insert(key, metadata.is_some());
        }
        Ok(())
    }
    /// Capability retry fence for native frame callbacks. Restore clocks and
    /// callback state before retry, without delivering the input edge twice.
    fn tick_native_frame(&mut self, host: &mut dyn HostServices) -> Result<()> {
        self.commit_button_inputs()?;
        let before = self.state.vm.as_ref().unwrap().clone();
        let mut io_before = self.state.io.clone();
        for _ in 0..32 {
            self.state.io.missing.clear();
            self.state.io.pending_probes.clear();
            self.state.io.writes.clear();
            let result = io_fs::scoped(&mut self.state.io, || {
                self.state.vm.as_mut().unwrap().tick_frame()
            });
            if !self.state.io.missing.is_empty() || !self.state.io.pending_probes.is_empty() {
                self.state.vm = Some(before.clone());
                let keys = std::mem::take(&mut self.state.io.missing);
                let probes = std::mem::take(&mut self.state.io.pending_probes);
                self.state.io = io_before.clone();
                self.resolve_io(host, keys, probes)?;
                io_before = self.state.io.clone();
                continue;
            }
            result?;
            self.state.vm.as_ref().unwrap().engine_reject_unknown()?;
            self.sync_native_proc_requests()?;
            for (key, bytes) in std::mem::take(&mut self.state.io.writes) {
                host.write_atomic(ClientId(self.state.client), &Self::path(&key)?, &bytes)
                    .map_err(err)?;
            }
            return Ok(());
        }
        anyhow::bail!("native frame callback resource retry limit")
    }
    fn execute(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        self.state.now_ms = host.now_ms();
        // File presence is observed anew at each pump; immutable decoded bytes
        // and native save state remain separate from mutable storage metadata.
        self.state.io.probes.clear();
        self.state
            .inputs
            .0
            .push(self.state.client, self.state.now_ms, input)?;
        let now = self.state.now_ms;
        platform_time::scoped(now, || -> Result<VmYield> {
            if self.state.vm.is_none() {
                self.boot(host)?;
            }
            self.focus_observation = host
                .window_focused(ClientId(self.state.client))
                .map_err(err)?;
            // An offscreen native host keeps executing without an OS window.
            // A presenting host's actual focus value takes precedence.
            self.apply_activity_observation();
            self.observe_buttons(host, input)?;
            self.sync_cursor(host)?;
            self.sync_movie(host)?;
            if !self.sync_object_movies(host)? {
                return Ok(VmYield::BudgetExhausted);
            }
            self.state.vm.as_ref().unwrap().engine_reject_unknown()?;
            if !self.poll_audio(host)? {
                return Ok(VmYield::BudgetExhausted);
            }
            self.apply_activity_observation();
            let resumed_events = self.state.event_frame.is_some();
            let has_audio = {
                let vm = self.state.vm.as_ref().unwrap();
                !vm.ctx.bgm.pending.is_empty()
                    || vm.ctx.pcm.pending()
                    || vm.ctx.se.pending()
                    || !vm.ctx.koe.pending.is_empty()
            };
            if has_audio {
                let before = self.state.vm.as_ref().unwrap().clone();
                if !self.begin_audio_batch(host, before)? {
                    return Ok(VmYield::BudgetExhausted);
                }
            }
            if !resumed_events && !self.state.pump_continuation {
                self.sync_audio(host)?;
                crate::message::sync_progress(
                    &mut self.state.vm.as_mut().unwrap().ctx,
                    ClientId(self.state.client),
                    host,
                )?;
                self.apply_queued_input(host)?;
                self.state.vm.as_mut().unwrap().begin_script_proc_pump();
                self.tick_native_frame(host)?;
                self.begin_event_frame()?;
            }
            // A blocked native wait still supplied this frame's clock samples.
            // A callback-created movie can wait for metadata without replaying
            // that callback: retain event_frame and its existing continuation
            // state, then resume drive_events after the front admission gate.
            if !self.sync_object_movies(host)? {
                // begin_event_frame retains only actual PCM event work. With
                // none, continue this SCRIPT pass; otherwise resume that frame.
                self.state.pump_continuation = self.state.event_frame.is_none();
                self.publish(host)?;
                return Ok(VmYield::BudgetExhausted);
            }
            if !self.drive_events(host)? {
                self.publish(host)?;
                return Ok(VmYield::BudgetExhausted);
            }
            self.apply_activity_observation();
            for _ in 0..budget.min(65536) {
                let before = self.state.vm.as_ref().unwrap().clone();
                let mut io_before = self.state.io.clone();
                let mut attempt = 0;
                loop {
                    attempt += 1;
                    ensure!(attempt <= 32, "native resource resolution retry limit");
                    let generation = self.state.vm.as_ref().unwrap().proc_generation();
                    self.state.io.missing.clear();
                    self.state.io.pending_probes.clear();
                    self.state.io.writes.clear();
                    let result = io_fs::scoped(&mut self.state.io, || {
                        self.state.vm.as_mut().unwrap().step()
                    });
                    if !self.state.io.missing.is_empty() || !self.state.io.pending_probes.is_empty()
                    {
                        self.state.vm = Some(before.clone());
                        let keys = std::mem::take(&mut self.state.io.missing);
                        let probes = std::mem::take(&mut self.state.io.pending_probes);
                        self.state.io = io_before.clone();
                        self.resolve_io(host, keys, probes)?;
                        io_before = self.state.io.clone();
                        continue;
                    }
                    result?;
                    self.sync_native_proc_requests()?;
                    self.flush_movie(host)?;
                    if !self.sync_object_movies(host)? {
                        self.state.pump_continuation = true;
                        self.publish(host)?;
                        return Ok(VmYield::BudgetExhausted);
                    }
                    if self
                        .state
                        .vm
                        .as_mut()
                        .unwrap()
                        .take_runtime_load_completed()
                    {
                        self.events.invalidate();
                        self.state.event_cursors.clear();
                        self.state.event_frame = None;
                        self.state.event_clock_ms = Some(self.state.now_ms);
                        self.discard_queued_input()?;
                    }
                    let vm = self.state.vm.as_mut().unwrap();
                    vm.engine_reject_unknown()?;
                    for (key, bytes) in std::mem::take(&mut self.state.io.writes) {
                        host.write_atomic(ClientId(self.state.client), &Self::path(&key)?, &bytes)
                            .map_err(err)?;
                    }
                    if !vm.ctx.bgm.pending.is_empty()
                        || vm.ctx.pcm.pending()
                        || vm.ctx.se.pending()
                        || !vm.ctx.koe.pending.is_empty()
                    {
                        self.state.pump_continuation = true;
                        if !self.begin_audio_batch(host, before.clone())? {
                            self.publish(host)?;
                            return Ok(VmYield::BudgetExhausted);
                        }
                    }
                    let vm = self.state.vm.as_mut().unwrap();
                    let halted = vm.is_halted();
                    let blocked = vm.is_blocked();
                    let boundary = vm.proc_generation() != generation;
                    if halted || blocked || boundary {
                        self.state.pump_continuation = false;
                        self.publish(host)?;
                        let vm = self.state.vm.as_ref().unwrap();
                        let event_work = vm
                            .ctx
                            .globals
                            .pcm_event_lists
                            .values()
                            .flatten()
                            .any(|st| st.is_active());
                        return Ok(if halted && !event_work {
                            VmYield::Halted
                        } else if let Some(t) = vm.ctx.wait.until {
                            VmYield::WaitUntil(t.0)
                        } else if vm.ctx.wait.waiting_for_key {
                            VmYield::WaitForInput
                        } else {
                            VmYield::BudgetExhausted
                        });
                    }
                    break;
                }
            }
            // A host scheduling quota is not a native SCRIPT/proc boundary.
            // Continue this same pass without ticking callbacks or consuming
            // another input frame; only the native branch above ends it.
            self.state.pump_continuation = true;
            self.publish(host)?;
            Ok(VmYield::BudgetExhausted)
        })
    }
    fn publish(&mut self, host: &mut dyn HostServices) -> Result<()> {
        self.apply_activity_observation();
        self.sync_presentation(host)?;
        self.sync_cursor(host)?;
        self.flush_movie(host)?;
        for _ in 0..32 {
            self.state.io.missing.clear();
            let result = io_fs::scoped(&mut self.state.io, || {
                crate::export::publish(
                    &mut self.state.vm.as_mut().unwrap().ctx,
                    ClientId(self.state.client),
                    &mut self.state.published,
                    &mut self.buttons.published,
                    &self.object_movies.pictures,
                    host,
                )
            });
            if self.state.io.missing.is_empty() {
                return result;
            }
            for key in std::mem::take(&mut self.state.io.missing) {
                self.fetch(host, &key)?;
            }
        }
        anyhow::bail!("native frame resource retry limit")
    }
    pub fn export_original_save(&mut self) -> Result<Vec<u8>> {
        ensure!(
            !self.object_movies.active()
                && !self.state.vm.as_ref().is_some_and(|vm| vm.ctx.movie.live
                    || vm.ctx.globals.mov.file_name.is_some()
                    || object_movie::has_movies(&vm.ctx)),
            "native Movie save resource reconstruction is not implemented"
        );
        ensure!(
            self.pending_audio.is_none(),
            "native BGM preparation pending"
        );
        ensure!(
            self.state.config.message_window.is_none()
                && self
                    .state
                    .vm
                    .as_ref()
                    .is_none_or(|vm| vm.ctx.ui.window.is_none()),
            "native original-save MWND reconstruction is not implemented for the plain message profile; runtime snapshots remain supported"
        );
        self.state
            .vm
            .as_mut()
            .context("native VM has not booted")?
            .engine_original_file()
    }
    pub fn import_original_save(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            !self.object_movies.active()
                && !self
                    .state
                    .vm
                    .as_ref()
                    .is_some_and(|vm| vm.ctx.movie.live || object_movie::has_movies(&vm.ctx)),
            "native Movie must be stopped before importing an original save"
        );
        ensure!(
            self.pending_audio.is_none(),
            "native BGM preparation pending"
        );
        ensure!(
            self.state.config.message_window.is_none()
                && self
                    .state
                    .vm
                    .as_ref()
                    .is_none_or(|vm| vm.ctx.ui.window.is_none()),
            "native original-save MWND reconstruction is not implemented for the plain message profile; runtime snapshots remain supported"
        );
        ensure!(bytes.len() <= 32 * 1024 * 1024, "native save file limit");
        let previous = self.state.clone();
        let path = self
            .state
            .vm
            .as_ref()
            .context("native VM has not booted")?
            .engine_original_import_path()?;
        let key = io_fs::canonical(&path)?;
        self.state.io.files.insert(key, Arc::new(bytes.to_vec()));
        let result = platform_time::scoped(self.state.now_ms, || {
            io_fs::scoped(&mut self.state.io, || {
                self.state.vm.as_mut().unwrap().engine_load_original()
            })
        });
        let result = result
            .and_then(|_| {
                ensure!(
                    !object_movie::has_movies(&self.state.vm.as_ref().unwrap().ctx),
                    "native Movie original-save resource reconstruction is not implemented"
                );
                Ok(())
            })
            .and_then(|_| Self::validate_event_state(&self.state))
            .and_then(|_| self.state.vm.as_ref().unwrap().engine_reject_unknown());
        if result.is_err() {
            self.state = previous;
        }
        if result.is_ok() {
            self.state
                .vm
                .as_mut()
                .unwrap()
                .take_runtime_load_completed();
            self.events.invalidate();
            self.state.event_cursors.clear();
            self.state.event_frame = None;
            self.state.event_clock_ms = Some(self.state.now_ms);
            self.state.pump_continuation = false;
            self.discard_queued_input()?;
        }
        result
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        let mut value = self
            .state
            .vm
            .as_ref()
            .map(|v| v.engine_diagnostics())
            .unwrap_or_else(|| serde_json::json!({"vm":"original-siglus","booted":false}));
        value["queued_input"] = serde_json::json!({"pending":self.state.inputs.0.len(),"capacity":self.state.inputs.0.capacity(),"pump_continuation":self.state.pump_continuation});
        value["native_rng"] =
            serde_json::json!(self.state.vm.as_ref().map(|v| v.ctx.globals.rng_state));
        value["pcmevents"] = self.event_diagnostics();
        value["event_frame_pending"] = serde_json::json!(self.state.event_frame.is_some());
        value["audio_preparing"] = serde_json::json!(self.pending_audio.is_some());
        value["ui_buttons"] = serde_json::json!({"engine_widgets":self.buttons.published.len(),"pending_native_intents":self.buttons.pending_count()});
        if let Some(vm) = &self.state.vm {
            value["activity"] = serde_json::json!({"window_focus":self.focus_observation,"effective_active":vm.ctx.globals.system.active_flag,"no_window_policy":"active-offscreen-host"});
            value["optical_disc_check"] = serde_json::json!({"policy":self.state.optical_disc_policy,"applicability":"not-applicable","once_requests":vm.ctx.globals.system.dummy_checks.len(),"authentication_performed":false});
            value["pcmch"] = vm.ctx.pcm.channels.inspect();
            value["koe"] = serde_json::json!({"number":vm.ctx.koe.current_no(),"compression_percent":vm.ctx.koe.rate(),"volume":vm.ctx.koe.volume_raw(),"cached":vm.ctx.koe.bindings.len(),"message_number":vm.ctx.globals.script.cur_koe_no,"message_character":vm.ctx.globals.script.cur_chr_no,"is_ex":vm.ctx.globals.sound_routing.koe_ex_flag});
            value["bgm"] = serde_json::json!({"name":vm.ctx.bgm.current_name(),"volume":vm.ctx.bgm.volume_raw(),"state":vm.ctx.bgm.check_state(),"looping":vm.ctx.bgm.save_loop_flag(),"paused":vm.ctx.bgm.save_pause_flag(),"delay_ms":vm.ctx.bgm.save_delay_time_ms(),"listened":vm.ctx.globals.bgm_table_flags});
        }
        value
    }
}
impl VmAdapter for SiglusVm {
    fn engine_id(&self) -> &'static str {
        "siglus"
    }
    fn snapshot_schema(&self) -> u32 {
        11
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.execute(host, input, budget).map_err(err)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        if self.buttons.pending() {
            return Err(Error::Unsupported(
                "Siglus pending native UI intent snapshot",
            ));
        }
        if self.object_movies.active()
            || self.state.vm.as_ref().is_some_and(|vm| {
                vm.ctx.movie.live
                    || vm.ctx.globals.mov.file_name.is_some()
                    || object_movie::has_movies(&vm.ctx)
            })
        {
            return Err(Error::Unsupported(
                "Siglus Movie runtime resource reconstruction",
            ));
        }
        // Pending preparation leaves the serialized VM at its original command.
        // Restore replays that uncommitted command without saving worker tickets.
        binary().serialize(&self.state).map_err(err)
    }
    fn restore(&mut self, bytes: &[u8]) -> crysirror_core::Result<()> {
        if self.object_movies.active()
            || self
                .state
                .vm
                .as_ref()
                .is_some_and(|vm| vm.ctx.movie.live || object_movie::has_movies(&vm.ctx))
        {
            return Err(Error::Unsupported(
                "stop Siglus Movie before restoring a runtime snapshot",
            ));
        }
        let mut state: Machine = binary().deserialize(bytes).map_err(err)?;
        if state.client != self.state.client || state.config != self.state.config {
            return Err(Error::SnapshotMismatch("native client/config"));
        }
        if state.optical_disc_policy != OPTICAL_DISC_POLICY {
            return Err(Error::SnapshotMismatch("native optical-disc host policy"));
        }
        state.config.validate().map_err(err)?;
        if state.inputs.0.client() != state.client
            || state.inputs.0.observed_time() > state.now_ms
            || (state.pump_continuation && state.event_frame.is_some())
        {
            return Err(Error::SnapshotMismatch("native input/continuation"));
        }
        Self::validate_event_state(&state).map_err(err)?;
        if let Some(vm) = &state.vm {
            if vm.ctx.globals.mov.file_name.is_some() || object_movie::has_movies(&vm.ctx) {
                return Err(Error::Unsupported(
                    "Siglus Movie runtime resource reconstruction",
                ));
            }
            vm.engine_validate().map_err(err)?;
            if vm.ctx.native_proc_waits.len() > 64 {
                return Err(Error::SnapshotMismatch("native EXCALL continuation depth"));
            }
            let mut expected = vm.ctx.clone();
            expected.ui.window = state.config.message_window.clone();
            expected.ui.name_spacing = None;
            if vm.ctx.ui.window.is_some() {
                crate::message::initialize_profile(&mut expected).map_err(err)?;
            }
            if vm.ctx.ui.window != expected.ui.window
                || vm.ctx.ui.name_spacing != expected.ui.name_spacing
            {
                return Err(Error::SnapshotMismatch("native message configuration"));
            }
        }
        if let Some(vm) = state.vm.as_mut() {
            vm.ctx.bgm.rebind().map_err(err)?;
            vm.ctx.koe.rebind().map_err(err)?;
            vm.ctx.pcm.rebind().map_err(err)?;
            vm.ctx.se.rebind().map_err(err)?;
        }
        self.events.invalidate();
        self.state = state;
        self.pending_audio = None;
        self.cursors = Default::default();
        self.presentation = Default::default();
        self.focus_observation = None;
        self.buttons.reset_input();
        Ok(())
    }
    fn restore_resources(&mut self, host: &mut dyn HostServices) -> crysirror_core::Result<()> {
        if let Some(vm) = self.state.vm.as_mut() {
            vm.ctx.images.published.clear();
            for (index, image) in vm.ctx.images.images.iter().enumerate() {
                let path = host.upload_rgba(
                    ClientId(self.state.client),
                    index as u32,
                    image.width,
                    image.height,
                    &image.rgba,
                )?;
                vm.ctx
                    .images
                    .published
                    .insert(index, path.relative().into());
            }
        }
        if self.state.vm.is_some() {
            self.sync_presentation(host).map_err(err)?;
            self.sync_cursor(host).map_err(err)?;
        }
        Ok(())
    }
    fn export_original_save(&mut self) -> crysirror_core::Result<Vec<u8>> {
        SiglusVm::export_original_save(self).map_err(err)
    }
    fn import_original_save(&mut self, bytes: &[u8]) -> crysirror_core::Result<()> {
        SiglusVm::import_original_save(self, bytes).map_err(err)
    }
    fn original_save_import_is_transactional(&self) -> bool {
        true
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&self.diagnostics()).unwrap_or_default()
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use crysirror_core::{
        service::FileSystem,
        state::{Command, Limits, SceneState},
    };
    #[derive(Default)]
    struct Host {
        now: u64,
        files: BTreeMap<String, Vec<u8>>,
        scene: SceneState,
        engine: crysirror_scene::EngineSystems,
    }
    fn key(path: &VfsPath) -> String {
        format!("{:?}/{}", path.mount(), path.relative())
    }
    impl FileSystem for Host {
        fn read(&mut self, client: ClientId, path: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            if client != ClientId(1) {
                return Err(Error::NotFound);
            }
            self.files.get(&key(path)).cloned().ok_or(Error::NotFound)
        }
        fn write_atomic(
            &mut self,
            client: ClientId,
            path: &VfsPath,
            data: &[u8],
        ) -> crysirror_core::Result<()> {
            if client != ClientId(1) {
                return Err(Error::NotFound);
            }
            if path.mount() == Mount::Asset {
                return Err(Error::ReadOnly);
            }
            self.files.insert(key(path), data.to_vec());
            Ok(())
        }
    }
    impl HostServices for Host {
        fn storage(
            &mut self,
            client: ClientId,
        ) -> crysirror_core::Result<Arc<dyn crysirror_core::service::ClientStorage>> {
            if client != ClientId(1) {
                return Err(Error::NotFound);
            }
            struct MetadataOnly(BTreeMap<String, u64>);
            impl crysirror_core::service::ClientStorage for MetadataOnly {
                fn metadata(
                    &self,
                    mount: Mount,
                    name: &str,
                ) -> crysirror_core::Result<Option<crysirror_core::service::StorageMetadata>>
                {
                    Ok(self.0.get(&format!("{mount:?}/{name}")).map(|size| {
                        crysirror_core::service::StorageMetadata {
                            is_directory: false,
                            size: Some(*size),
                            created_ms: None,
                            modified_ms: None,
                            accessed_ms: None,
                        }
                    }))
                }
                fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
                    panic!("SYSTEM metadata query must not read file bytes")
                }
                fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
                    Err(Error::Unsupported("test metadata-only capability"))
                }
                fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
                    Err(Error::Unsupported("test metadata-only capability"))
                }
                fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
                    Err(Error::Unsupported("test metadata-only capability"))
                }
            }
            Ok(Arc::new(MetadataOnly(
                self.files
                    .iter()
                    .map(|(name, bytes)| (name.clone(), bytes.len() as u64))
                    .collect(),
            )))
        }
        fn upload_rgba(
            &mut self,
            client: ClientId,
            key: u32,
            width: u32,
            height: u32,
            pixels: &[u8],
        ) -> crysirror_core::Result<VfsPath> {
            if client != ClientId(1)
                || crysirror_image::rgba_len(width, height).map_err(err)? != pixels.len()
            {
                return Err(Error::Invalid("test image"));
            }
            let path = VfsPath::new(Mount::Asset, &format!("__test/image-{key}"))?;
            self.files
                .insert(super::verification_tests::key(&path), pixels.to_vec());
            Ok(path)
        }
        fn query_engine(
            &mut self,
            client: ClientId,
            query: crysirror_core::service::EngineQuery,
        ) -> crysirror_core::Result<Vec<u8>> {
            match query {
                crysirror_core::service::EngineQuery::UiInteraction(id) => {
                    serde_json::to_vec(&self.engine.ui.interaction(client.0, id)).map_err(err)
                }
                crysirror_core::service::EngineQuery::Layer(id) => {
                    serde_json::to_vec(&self.engine.layers.get(client, id)).map_err(err)
                }
                _ => Err(Error::Unsupported("test host query")),
            }
        }
        fn font_metrics(
            &self,
            font: &str,
            size: f32,
            _: char,
        ) -> crysirror_core::Result<crysirror_core::service::FontMetrics> {
            if font != "message-font" && font != "default" {
                return Err(Error::NotFound);
            }
            // Deterministic layout-contract host; font rasterization is covered
            // by the separate ignored system-font rendering test.
            Ok(crysirror_core::service::FontMetrics {
                ascent: size * 0.75,
                descent: -size * 0.25,
                advance: size,
            })
        }
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn submit(&mut self, client: ClientId, commands: &[Command]) -> crysirror_core::Result<()> {
            self.scene.apply_batch(client, commands, Limits::default())
        }
        fn submit_engine(
            &mut self,
            client: ClientId,
            schema: u32,
            data: &[u8],
        ) -> crysirror_core::Result<()> {
            if schema != 1 {
                return Err(Error::Unsupported("engine batch schema"));
            }
            let batch: crysirror_scene::ServiceBatch = serde_json::from_slice(data).map_err(err)?;
            self.engine.submit(client, &batch).map_err(err)
        }
    }
    fn machine(savepoint: bool) -> (SiglusVm, Host) {
        let config = Config {
            scene: "Scene.pck".into(),
            format: "pck".into(),
            scene_name: None,
            string_codec: "plain".into(),
            gameexe: None,
            key_file: None,
            resources: BTreeMap::new(),
            message_window: None,
        };
        let mut host = Host::default();
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&crate::fixture::key_wait_scene(savepoint)),
        );
        (SiglusVm::new(ClientId(1), config).unwrap(), host)
    }
    #[test]
    fn scheduling_quotas_resume_one_native_frame_and_preserve_input_edges() {
        let mut scene = crate::fixture::SceneBuilder::default();
        for value in 0..400 {
            scene.int(value);
        }
        scene.code.push(crate::runtime::constants::cd::EOF);
        let bytes = crate::fixture::pack_scene(&scene.finish());
        let (mut split, mut split_host) = machine(false);
        split_host
            .files
            .insert("Asset/Scene.pck".into(), bytes.clone());
        let (mut whole, mut whole_host) = machine(false);
        whole_host.files.insert("Asset/Scene.pck".into(), bytes);
        assert_eq!(
            whole
                .step(&mut whole_host, InputFrame::default(), 1000)
                .unwrap(),
            VmYield::Halted
        );
        for tick in 0..=400 {
            split_host.now = tick * 16;
            let input = InputFrame {
                buttons_down: if (20..30).contains(&tick) { 1 } else { 0 },
                ..Default::default()
            };
            let yielded = split.step(&mut split_host, input, 1).unwrap();
            assert_eq!(
                split.state.vm.as_ref().unwrap().ctx.globals.render_frame,
                1,
                "host quota must not rerun native frame callbacks"
            );
            if tick == 200 {
                // Framework saves retain a suspended pass and its queued input.
                let saved = split.snapshot().unwrap();
                split.restore(&saved).unwrap();
            }
            assert_eq!(
                yielded,
                if tick == 400 {
                    VmYield::Halted
                } else {
                    VmYield::BudgetExhausted
                }
            );
        }
        assert_eq!(
            split.diagnostics()["int_stack"],
            whole.diagnostics()["int_stack"]
        );
        // Hundreds of unchanged pumps neither fill the bounded inbox nor
        // overwrite the real down/up pair while SCRIPT is still running.
        let queued = split.state.inputs.0.read(1, 256).unwrap();
        assert_eq!(queued.samples.len(), 2);
        assert_eq!(queued.samples[0].frame.buttons_pressed, 1);
        assert_eq!(queued.samples[1].released, 1);
        assert!(!split.state.pump_continuation);
    }
    #[test]
    fn native_system_file_queries_retry_without_committing_false_or_reading_payload() {
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        scene.command_string(
            &[92, 6],
            0,
            "image.g00",
            &[],
            crate::runtime::constants::fm::INT,
        );
        scene.command_string(
            &[92, 6],
            0,
            "absent.g00",
            &[],
            crate::runtime::constants::fm::INT,
        );
        scene.command_string(
            &[92, 12],
            0,
            "slot.sav",
            &[],
            crate::runtime::constants::fm::INT,
        );
        scene.code.push(crate::runtime::constants::cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        host.files.insert("Asset/image.g00".into(), vec![1]);
        host.files.insert("Save/slot.sav".into(), vec![2]);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 100).unwrap(),
            VmYield::Halted
        );
        let state = vm.state.vm.as_ref().unwrap();
        assert!(
            state.engine_diagnostics()["int_stack"]
                .as_array()
                .unwrap()
                .ends_with(&[
                    serde_json::json!(1),
                    serde_json::json!(0),
                    serde_json::json!(1)
                ])
        );
        assert!(!vm.state.io.files.contains_key("asset/image.g00"));
        assert!(!vm.state.io.files.contains_key("save/slot.sav"));
        assert!(vm.state.io.pending_probes.is_empty());
    }
    #[test]
    fn native_optical_disc_policy_is_void_once_and_explicit_in_snapshots() {
        use crate::runtime::constants::{cd, fm};
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        for _ in 0..2 {
            scene.element(&[92, 2]);
            scene.string("dummy");
            scene.int(139);
            scene.string("authored");
            scene.code.push(cd::COMMAND);
            for value in [0, 3, fm::STR, fm::INT, fm::STR, 0, fm::VOID] {
                scene.i32(value);
            }
        }
        scene.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        let diag = vm.diagnostics();
        assert_eq!(diag["int_stack"], serde_json::json!([]));
        assert_eq!(diag["optical_disc_check"]["once_requests"], 1);
        assert_eq!(
            diag["optical_disc_check"]["authentication_performed"],
            false
        );
        assert_eq!(diag["optical_disc_check"]["policy"], OPTICAL_DISC_POLICY);
        assert_eq!(vm.snapshot_schema(), 11);
        let saved = vm.snapshot().unwrap();
        vm.restore(&saved).unwrap();
        let mut wrong: Machine = binary().deserialize(&saved).unwrap();
        wrong.optical_disc_policy = "original-windows-disc-host".into();
        assert!(vm.restore(&binary().serialize(&wrong).unwrap()).is_err());
        assert_eq!(vm.diagnostics()["optical_disc_check"]["once_requests"], 1);
    }
    #[test]
    fn native_pack_uses_only_client_supplied_decode_material() {
        let (mut vm, mut host) = machine(false);
        let exe = [0x37u8; 16];
        let easy: Vec<u8> = (0..256).map(|i| (i as u8).wrapping_mul(17)).collect();
        fn literal_lzss(bytes: &[u8]) -> Vec<u8> {
            let mut payload = Vec::new();
            for chunk in bytes.chunks(8) {
                payload.push(((1u16 << chunk.len()) - 1) as u8);
                payload.extend(chunk);
            }
            let mut out = ((payload.len() + 8) as u32).to_le_bytes().to_vec();
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(payload);
            out
        }
        let pack = host.files.get_mut("Asset/Scene.pck").unwrap();
        let start = i32::from_le_bytes(pack[19 * 4..20 * 4].try_into().unwrap()) as usize;
        let index = i32::from_le_bytes(pack[17 * 4..18 * 4].try_into().unwrap()) as usize;
        let scene = pack.split_off(start);
        let mut coded = literal_lzss(&scene);
        pack[21 * 4..22 * 4].copy_from_slice(&1i32.to_le_bytes());
        pack[22 * 4..23 * 4].copy_from_slice(&132i32.to_le_bytes());
        pack[index + 4..index + 8].copy_from_slice(&(coded.len() as i32).to_le_bytes());
        for (i, b) in coded.iter_mut().enumerate() {
            *b ^= exe[i % 16] ^ easy[i % 256];
        }
        pack.extend(coded);
        let game_key = [0x19u8; 256];
        let ini = "#START_SCENE = \"entry\", 0\n#SCREEN_SIZE = 320, 200\n#DUMMY = 0\n";
        let text = ini
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut body = literal_lzss(&text);
        for (i, b) in body.iter_mut().enumerate() {
            *b ^= exe[i % 16] ^ game_key[i % 256];
        }
        let mut gameexe = vec![0; 8];
        gameexe[4] = 1;
        gameexe.extend(body);
        host.files.insert("Asset/Gameexe.dat".into(), gameexe);
        vm.state.config.gameexe = Some("Gameexe.dat".into());
        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        host.files.insert(
            "Asset/materials.toml".into(),
            format!(
                "key_hex = \"{}\"\nscene_angou_hex = \"{}\"\ngame_angou_hex = \"{}\"\n",
                hex(&exe),
                hex(&easy),
                hex(&game_key)
            )
            .into_bytes(),
        );
        vm.state.config.key_file = Some("materials.toml".into());
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(5000)
        );
        assert!(!vm.state.io.files.contains_key("asset/materials.toml"));
        assert_eq!(
            (
                vm.state.vm.as_ref().unwrap().ctx.screen_w,
                vm.state.vm.as_ref().unwrap().ctx.screen_h
            ),
            (320, 200)
        );
        let (mut other, _) = machine(false);
        assert!(other.step(&mut host, InputFrame::default(), 1000).is_err());
        assert!(
            SiglusVm::new(
                ClientId(1),
                Config {
                    key_file: Some("../keys.toml".into()),
                    ..other.state.config
                }
            )
            .is_err()
        );
    }

    #[test]
    fn native_decide_wait_requires_down_up_and_snapshot_repeats_the_edge() {
        let (mut vm, mut host) = machine(false);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(5000)
        );
        let saved = vm.snapshot().unwrap();
        host.now = 10;
        vm.step(
            &mut host,
            InputFrame {
                buttons_down: 1,
                buttons_pressed: 1,
                ..Default::default()
            },
            1000,
        )
        .unwrap();
        assert_eq!(vm.diagnostics()["halted"], false);
        assert_eq!(host.engine.frame([320, 180]).unwrap()[0].quad[0][0], 28.);
        host.now = 20;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([1, 123]));
        assert_eq!(host.engine.frame([320, 180]).unwrap()[0].quad[0][0], 180.);
        vm.restore(&saved).unwrap();
        host.now = 10;
        vm.step(
            &mut host,
            InputFrame {
                buttons_down: 1,
                buttons_pressed: 1,
                ..Default::default()
            },
            1000,
        )
        .unwrap();
        host.now = 20;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([1, 123]));
    }
    #[test]
    fn original_savepoint_exports_real_codec_and_failed_import_rolls_back() {
        let (mut vm, mut host) = machine(true);
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let original = vm.export_original_save().unwrap();
        assert!(original.len() > 128);
        let before = vm.diagnostics();
        assert!(vm.import_original_save(b"invalid native save").is_err());
        assert_eq!(vm.diagnostics(), before);
        let (mut no_save, mut other) = machine(false);
        no_save
            .step(&mut other, InputFrame::default(), 1000)
            .unwrap();
        assert!(no_save.export_original_save().is_err());
        vm.import_original_save(&original).unwrap();
        assert_eq!(vm.diagnostics()["halted"], false);
        assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([1]));
    }
    #[test]
    fn native_offset_list_event_uses_the_shared_curve_and_layer_origin() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        let object = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
        ];
        let chain = |tail: &[i32]| [object.as_slice(), tail].concat();
        scene.command(
            &chain(&[codes::ELM_OBJECT_CREATE_RECT]),
            2,
            &[0, 0, 4, 3, 255, 0, 0, 255, 1, 10, 20],
            fm::VOID,
        );
        scene.command(
            &chain(&[
                codes::ELM_OBJECT_Y_REP_EVE,
                codes::ELM_ARRAY,
                0,
                codes::int_event_op::SET,
            ]),
            0,
            &[10, 100, 0, 0],
            fm::VOID,
        );
        scene.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[200], fm::VOID);
        scene.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .rect[1],
            20.
        );
        host.now = 50;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .rect[1],
            25.
        );
        let saved = vm.snapshot().unwrap();
        host.now = 100;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .rect[1],
            30.
        );
        vm.restore(&saved).unwrap();
        host.now = 75;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .rect[1],
            27.
        );
        assert_eq!(vm.diagnostics()["wait"]["until_ms"], 200);
        let before = vm.diagnostics();
        assert_eq!(
            vm.diagnostics(),
            before,
            "inspection never polls/consumes the native wait"
        );
    }
    #[test]
    fn native_object_opacity_event_uses_shared_curve_and_restores_mid_fade() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        let object = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
        ];
        let chain = |tail: &[i32]| [object.as_slice(), tail].concat();
        scene.command(
            &chain(&[codes::ELM_OBJECT_CREATE_RECT]),
            2,
            &[0, 0, 4, 3, 255, 0, 0, 255, 1, 10, 20],
            fm::VOID,
        );
        scene.command(&chain(&[codes::ELM_OBJECT_TR]), 1, &[0], fm::VOID);
        scene.command(
            &chain(&[codes::ELM_OBJECT_TR_EVE, codes::int_event_op::SET]),
            0,
            &[255, 100, 0, 0],
            fm::VOID,
        );
        scene.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[200], fm::VOID);
        scene.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(200)
        );
        let layer = host.engine.layers.get(ClientId(1), 0x80000000).unwrap();
        assert_eq!(layer.opacity, 0);
        assert_eq!(layer.rect, [10., 20., 4., 3.]);
        host.now = 50;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .opacity,
            127
        );
        let saved = vm.snapshot().unwrap();
        host.now = 200;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .opacity,
            255
        );
        vm.restore(&saved).unwrap();
        host.now = 75;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), 0x80000000)
                .unwrap()
                .opacity,
            191
        );
    }
    #[test]
    fn native_disp_publishes_shared_layers_and_resumes_only_after_the_native_frame_boundary() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        use crysirror_scene::layer::Paint;
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        let object = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_CREATE_RECT,
        ];
        scene.command(&object, 2, &[0, 0, 4, 3, 255, 0, 0, 255, 1, 2, 3], fm::VOID);
        scene.command(&[codes::ELM_GLOBAL_DISP], 0, &[], fm::VOID);
        scene.command(&object, 2, &[0, 0, 4, 3, 0, 0, 255, 255, 1, 2, 3], fm::VOID);
        scene.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::BudgetExhausted
        );
        let native = vm.state.vm.as_ref().unwrap();
        assert_eq!(native.last_proc_kind(), crate::runtime::ProcKind::Disp);
        assert_eq!(native.ctx.globals.render_frame, 1);
        assert!(
            !vm.state.pump_continuation,
            "DISP is an actual native boundary, not a host quota"
        );
        assert!(
            host.engine
                .frame([320, 180])
                .unwrap()
                .iter()
                .any(|p| matches!(&p.paint,Paint::Solid{color} if *color==[255,0,0,255]))
        );
        host.now = 1;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(vm.state.vm.as_ref().unwrap().ctx.globals.render_frame, 2);
        assert!(
            host.engine
                .frame([320, 180])
                .unwrap()
                .iter()
                .any(|p| matches!(&p.paint,Paint::Solid{color} if *color==[0,0,255,255]))
        );
    }
    #[test]
    fn native_center_rep_moves_the_scale_origin_without_shifting_unit_scale_xy() {
        use crate::runtime::{constants::fm, forms::codes};
        let (mut vm, mut host) = machine(false);
        let mut scene = crate::fixture::SceneBuilder::default();
        scene.command(
            &[codes::ELM_GLOBAL_FRONT, codes::ELM_STAGE_CREATE_OBJECT],
            0,
            &[1],
            fm::VOID,
        );
        let object = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
        ];
        let mut set = |code, overload, args: &[i32]| {
            let mut chain = object.to_vec();
            chain.push(code);
            scene.command(&chain, overload, args, fm::VOID);
        };
        set(
            codes::ELM_OBJECT_CREATE_RECT,
            2,
            &[0, 0, 80, 40, 35, 180, 220, 255, 1, 28, 28],
        );
        set(codes::ELM_OBJECT_CENTER_REP_X, 1, &[20]);
        set(codes::ELM_OBJECT_CENTER_REP_Y, 1, &[-12]);
        set(codes::ELM_OBJECT_CENTER_REP_Z, 1, &[30]);
        scene.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], fm::VOID);
        for (code, value) in [
            (codes::ELM_OBJECT_SCALE_X, 2000),
            (codes::ELM_OBJECT_SCALE_Y, 1500),
        ] {
            let mut chain = object.to_vec();
            chain.push(code);
            scene.command(&chain, 1, &[value], fm::VOID);
        }
        scene.code.push(crate::runtime::constants::cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(100)
        );
        let first = host.engine.frame([320, 180]).unwrap().remove(0);
        assert_eq!(first.quad[0][..2], [28., 28.]);
        assert_eq!(first.quad[2][..2], [108., 68.]);
        // Pinned collector: Z anchor gains center_rep_z, but pivot_z does not.
        assert!((first.quad[0][2] - (0.5 - 30. / 100000.)).abs() < 1e-6);
        host.now = 100;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        let scaled = host.engine.frame([320, 180]).unwrap().remove(0);
        // Native XY: anchor + rep + (local - center - rep) * scale.
        assert_eq!(scaled.quad[0][..2], [8., 34.]);
        assert_eq!(scaled.quad[2][..2], [168., 94.]);
    }
    #[test]
    fn native_text_and_name_report_the_missing_message_bridge_instead_of_succeeding() {
        for name in [false, true] {
            let (mut vm, mut host) = machine(false);
            let scene = if name {
                let mut builder = crate::fixture::SceneBuilder::default();
                builder.string("Native name");
                builder.code.push(crate::runtime::constants::cd::NAME);
                builder.code.push(crate::runtime::constants::cd::EOF);
                builder.finish()
            } else {
                crate::fixture::message_scene()
            };
            host.files
                .insert("Asset/Scene.pck".into(), crate::fixture::pack_scene(&scene));
            let error = vm.step(&mut host, InputFrame::default(), 1000).unwrap_err();
            assert!(
                error.to_string().contains(
                    "native message layout must be implemented before CD_TEXT/CD_NAME can render"
                ),
                "{error}"
            );
            assert!(host.engine.frame([320, 180]).unwrap().is_empty());
        }
    }

    fn plain_message_machine(texts: &[&str]) -> (SiglusVm, Host) {
        let (mut vm, mut host) = machine(false);
        vm.state.config.message_window = Some(crate::MessageWindow {
            font: "message-font".into(),
            rect: [12, 80, 200, 80],
            name_rect: [12, 50, 200, 25],
            size: 20,
            name_size: 20,
            spacing: [-1, 10],
            color: [255; 4],
            name_color: [250, 210, 70, 255],
        });
        let mut scene = crate::fixture::SceneBuilder::default();
        scene.string("A name");
        scene.code.push(crate::runtime::constants::cd::NAME);
        scene.string("Ignored second name");
        scene.code.push(crate::runtime::constants::cd::NAME);
        for text in texts {
            scene.string(text);
            scene.code.push(crate::runtime::constants::cd::TEXT);
            scene.i32(-1);
        }
        scene.code.push(crate::runtime::constants::cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&scene.finish()),
        );
        (vm, host)
    }
    #[test]
    fn native_font_shadow_modes_publish_shared_faces_and_synchronized_reveal() {
        use crate::message::MESSAGE_LAYER;
        use crysirror_scene::layer::{GlyphCoverageKernel, Paint};
        for mode in 0..=3 {
            let (mut vm, mut host) = plain_message_machine(&["AB"]);
            vm.state.config.gameexe = Some("Gameexe.ini".into());
            host.files.insert("Asset/Gameexe.ini".into(), format!("#START_SCENE=\"entry\"\n#CONFIG.FONT.SHADOW={mode}\n#CONFIG.FONT.FUTOKU=1\n#MWND.000.FUCHI_COLOR=2\n#MWND.000.SHADOW_COLOR=3\n#COLOR_TABLE.002=0,255,0\n#COLOR_TABLE.003=0,0,255\n").into_bytes());
            vm.step(&mut host, InputFrame::default(), 1000).unwrap();
            let body = host.engine.layers.get(ClientId(1), MESSAGE_LAYER).unwrap();
            let Paint::GlyphRun {
                blur: Some(face), ..
            } = &body.paint
            else {
                panic!("native bold face")
            };
            assert_eq!(
                (face.kernel, face.radius),
                (GlyphCoverageKernel::SquareFace, 1)
            );
            let has_shadow = matches!(mode, 1 | 3);
            let has_edge = matches!(mode, 2 | 3);
            for (id, expected, color) in [
                (MESSAGE_LAYER + 4, has_shadow, [0, 0, 255, 255]),
                (MESSAGE_LAYER + 5, has_edge, [0, 255, 0, 255]),
            ] {
                let layer = host.engine.layers.get(ClientId(1), id);
                assert_eq!(layer.is_some(), expected);
                if let Some(layer) = layer {
                    let Paint::GlyphRun {
                        color: actual,
                        reveal,
                        blur: Some(face),
                        ..
                    } = &layer.paint
                    else {
                        panic!("decorated run")
                    };
                    assert_eq!(*actual, color);
                    assert_eq!(face.kernel, GlyphCoverageKernel::SquareFace);
                    assert_eq!(
                        face.radius,
                        if id == MESSAGE_LAYER + 5 || mode == 3 {
                            3
                        } else {
                            1
                        }
                    );
                    assert_eq!(reveal.as_ref().unwrap().visible_count(2), 0);
                }
            }
            host.engine.layers.tick(20);
            vm.step(&mut host, InputFrame::default(), 1000).unwrap();
            for id in [MESSAGE_LAYER, MESSAGE_LAYER + 4, MESSAGE_LAYER + 5] {
                if let Some(status) = host.engine.layers.reveal_status(ClientId(1), id) {
                    assert_eq!(status.visible, 1);
                }
            }
            let bytes = vm.snapshot().unwrap();
            vm.restore(&bytes).unwrap();
            assert_eq!(vm.state.vm.as_ref().unwrap().ctx.ui.visible_chars, 1);
        }
    }
    #[test]
    fn native_text_uses_shared_cells_retained_glyphs_and_engine_reveal_clock() {
        use crate::message::{MESSAGE_LAYER, NAME_LAYER};
        use crysirror_scene::layer::Paint;
        let (mut vm, mut host) = plain_message_machine(&["Aあ", "B"]);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::BudgetExhausted
        );
        let layer = host.engine.layers.get(ClientId(1), MESSAGE_LAYER).unwrap();
        let Paint::GlyphRun {
            glyphs,
            reveal: Some(reveal),
            ..
        } = &layer.paint
        else {
            panic!("engine glyph run")
        };
        assert_eq!(
            glyphs.iter().map(|g| g.baseline).collect::<Vec<_>>(),
            [[0., 15.], [9., 15.]]
        );
        assert_eq!(reveal.visible_count(glyphs.len()), 0);
        let Paint::GlyphRun { glyphs, .. } = &host
            .engine
            .layers
            .get(ClientId(1), NAME_LAYER)
            .unwrap()
            .paint
        else {
            panic!("name glyph run")
        };
        assert_eq!(
            glyphs.iter().map(|g| g.character).collect::<String>(),
            "A name"
        );
        // Advancing the host clock alone must not advance a private VM clock.
        host.now = 1000;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::BudgetExhausted
        );
        assert_eq!(
            host.engine
                .layers
                .reveal_status(ClientId(1), MESSAGE_LAYER)
                .unwrap()
                .visible,
            0
        );
        host.engine.layers.tick(40);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::BudgetExhausted
        );
        let Paint::GlyphRun {
            glyphs,
            reveal: Some(reveal),
            ..
        } = &host
            .engine
            .layers
            .get(ClientId(1), MESSAGE_LAYER)
            .unwrap()
            .paint
        else {
            panic!("appended glyph run")
        };
        assert_eq!(
            glyphs.iter().map(|g| g.character).collect::<String>(),
            "AあB"
        );
        assert_eq!(glyphs[2].baseline, [28., 15.]);
        assert_eq!(reveal.initial_visible, 2);
        assert_eq!(reveal.visible_count(glyphs.len()), 2);
        let saved = vm.snapshot().unwrap();
        let engine_saved = host.engine.clone();
        host.engine.layers.tick(20);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        vm.restore(&saved).unwrap();
        host.engine = engine_saved;
        host.engine.layers.tick(20);
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
            Some("AあB")
        );
        let ctx = &vm.state.vm.as_ref().unwrap().ctx;
        let native = ctx
            .globals
            .stage_forms
            .values()
            .next()
            .unwrap()
            .mwnd_lists
            .get(&1)
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(native.msg_text, "AあB");
        assert_eq!(native.name_text, "A name");
        assert_eq!(native.glyphs.len(), 3);
    }
    #[test]
    fn native_decide_completes_only_the_current_reveal() {
        let (mut vm, mut host) = plain_message_machine(&["AB", "CD"]);
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        vm.step(
            &mut host,
            InputFrame {
                buttons_down: 1,
                buttons_pressed: 1,
                ..Default::default()
            },
            1000,
        )
        .unwrap();
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
            Some("AB")
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
            Some("ABCD")
        );
        assert_eq!(
            host.engine
                .layers
                .reveal_status(ClientId(1), crate::message::MESSAGE_LAYER)
                .unwrap()
                .visible,
            2
        );
        assert!(!vm.state.vm.as_ref().unwrap().is_halted());
    }
    #[test]
    fn newline_only_chunk_keeps_its_cursor_without_an_engine_timer() {
        let (mut vm, mut host) = plain_message_machine(&["\n", "A"]);
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let crysirror_scene::layer::Paint::GlyphRun { glyphs, .. } = &host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER)
            .unwrap()
            .paint
        else {
            panic!("glyph run")
        };
        assert_eq!(glyphs[0].baseline, [0., 45.]);
        assert!(
            vm.export_original_save()
                .unwrap_err()
                .to_string()
                .contains("MWND reconstruction")
        );
        assert!(
            vm.import_original_save(b"native")
                .unwrap_err()
                .to_string()
                .contains("MWND reconstruction")
        );
    }
    #[test]
    fn unsupported_message_forms_fail_before_publishing_partial_text() {
        let (mut vm, mut host) = plain_message_machine(&["＄Ａ１"]);
        let error = vm.step(&mut host, InputFrame::default(), 1000).unwrap_err();
        assert!(error.to_string().contains("markup/emoji"), "{error}");
        assert!(
            host.engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .is_none()
        );
        let (mut vm, mut host) = plain_message_machine(&["あ".repeat(500).as_str()]);
        let error = vm.step(&mut host, InputFrame::default(), 1000).unwrap_err();
        assert!(error.to_string().contains("page overflow"), "{error}");
        assert!(
            host.engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .is_none()
        );
    }

    #[test]
    fn native_gameexe_text_uses_its_grid_position_palette_and_snapshot_profile() {
        use crate::runtime::{constants::cd, forms::codes};
        use crysirror_scene::layer::Paint;
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert("Asset/Gameexe.ini".into(), b"#START_SCENE=\"entry\"\n#SCREEN_SIZE=320,200\n#MWND.000.WINDOW_POS=10,100\n#MWND.000.MESSAGE_POS=4,6\n#MWND.000.MOJI_CNT=6,2\n#MWND.000.MOJI_SIZE=20\n#MWND.000.MOJI_SPACE=-1,10\n#MWND.000.NAME_MOJI_SIZE=20\n#MWND.000.MOJI_COLOR=2\n#COLOR_TABLE.002=12,34,56\n".to_vec());
        let mut s = crate::fixture::SceneBuilder::default();
        s.string("A\u{3042}");
        s.code.push(cd::TEXT);
        s.i32(7);
        s.command(
            &[
                codes::ELM_GLOBAL_FRONT,
                codes::ELM_STAGE_MWND,
                codes::ELM_ARRAY,
                0,
                codes::ELM_MWND_CLOSE_NOWAIT,
            ],
            0,
            &[],
            0,
        );
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let layer = host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER)
            .unwrap();
        assert_eq!(layer.rect, [14., 106., 115., 50.]);
        let Paint::GlyphRun { color, glyphs, .. } = &layer.paint else {
            panic!("engine glyph run")
        };
        assert_eq!(*color, [12, 34, 56, 255]);
        assert_eq!(glyphs[1].baseline[0], 9.);
        let ctx = &vm.state.vm.as_ref().unwrap().ctx;
        assert!(!ctx.globals.read_flag(0, 7));
        let snapshot = vm.snapshot().unwrap();
        host.engine
            .tick(100, InputFrame::default(), [320, 200])
            .unwrap();
        host.now = 100;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert!(
            !host
                .engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .unwrap()
                .visible
        );
        assert!(vm.state.vm.as_ref().unwrap().ctx.globals.read_flag(0, 7));
        vm.restore(&snapshot).unwrap();
        assert!(!vm.state.vm.as_ref().unwrap().ctx.globals.read_flag(0, 7));
    }

    #[test]
    fn native_mwnd_clear_is_deferred_and_close_open_preserve_engine_text() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        let (mut vm, mut host) = plain_message_machine(&[]);
        let mut s = crate::fixture::SceneBuilder::default();
        let chain = |op| {
            [
                codes::ELM_GLOBAL_FRONT,
                codes::ELM_STAGE_MWND,
                codes::ELM_ARRAY,
                0,
                op,
            ]
        };
        s.command_string(&chain(codes::ELM_MWND_PRINT), 0, "Old", &[], 0);
        s.command(&chain(codes::ELM_MWND_CLEAR), 0, &[], 0);
        s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], 0);
        s.command(&chain(codes::ELM_MWND_CLOSE_NOWAIT), 0, &[], 0);
        s.command(&chain(codes::ELM_MWND_CHECK_OPEN), 0, &[], fm::INT);
        s.command(&chain(codes::ELM_MWND_OPEN_NOWAIT), 0, &[], 0);
        s.command_string(&chain(codes::ELM_MWND_NAMAE), 0, "New speaker", &[], 0);
        s.command_string(&chain(codes::ELM_MWND_PRINT), 0, "New", &[], 0);
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        host.engine
            .tick(100, InputFrame::default(), [320, 180])
            .unwrap();
        host.now = 100;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
            Some("Old")
        );
        let saved = vm.snapshot().unwrap();
        host.now = 200;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let ctx = &vm.state.vm.as_ref().unwrap().ctx;
        assert_eq!(ctx.ui.message.as_deref(), Some("New"));
        assert_eq!(ctx.ui.name.as_deref(), Some("New speaker"));
        assert!(
            host.engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .unwrap()
                .visible
        );
        assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([0]));
        vm.restore(&saved).unwrap();
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
            Some("Old")
        );
    }

    #[test]
    fn native_gameexe_complex_text_is_not_silently_downgraded_to_plain() {
        for option in [
            "#TATEYOKO_MODE=1",
            "#MWND.000.OPEN_ANIME_TYPE=99",
            "#MWND.000.NOVEL_MODE=1",
        ] {
            let (mut vm, mut host) = machine(false);
            vm.state.config.gameexe = Some("Gameexe.ini".into());
            host.files.insert(
                "Asset/Gameexe.ini".into(),
                format!("#START_SCENE=\"entry\"\n{option}\n").into_bytes(),
            );
            host.files.insert(
                "Asset/Scene.pck".into(),
                crate::fixture::pack_scene(&crate::fixture::message_scene()),
            );
            let error = vm.step(&mut host, InputFrame::default(), 1000).unwrap_err();
            assert!(error.to_string().contains("native Gameexe"), "{error}");
            assert!(
                host.engine
                    .layers
                    .get(ClientId(1), crate::message::MESSAGE_LAYER)
                    .is_none()
            );
        }
    }

    #[test]
    fn native_waku_uses_scoped_g00_resource_and_engine_image_filter_layers() {
        use crate::runtime::constants::cd;
        use crysirror_scene::layer::Paint;
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert("Asset/Gameexe.ini".into(),b"#START_SCENE=\"entry\"\n#MWND.000.WINDOW_POS=10,100\n#MWND.000.WINDOW_SIZE=300,80\n#WAKU.000.WAKU_FILE=\"frame\"\n#WAKU.000.FILTER_COLOR=12,34,56,200\n#WAKU.000.FILTER_CONFIG_COLOR=1\n#WAKU.000.FILTER_CONFIG_TR=1\n#CONFIG.FILTER_COLOR=7,8,9,77\n".to_vec());
        host.files
            .insert("Asset/g00/frame.g00".into(), crate::fixture::pixel_g00());
        let mut s = crate::fixture::SceneBuilder::default();
        s.string("Engine text");
        s.code.push(cd::TEXT);
        s.i32(-1);
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let frame = host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER + 2)
            .unwrap();
        assert_eq!(frame.rect, [10., 100., 1., 1.]);
        let Paint::Image { asset, .. } = &frame.paint else {
            panic!("engine image")
        };
        assert!(asset.starts_with("__test/image-"));
        let filter = host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER + 3)
            .unwrap();
        assert_eq!(filter.rect, [10., 100., 300., 80.]);
        assert_eq!(filter.opacity, 77);
        assert_eq!(
            filter.filter.as_ref().unwrap().add,
            [7. / 255., 8. / 255., 9. / 255.]
        );
        let text = host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER)
            .unwrap();
        assert!(filter.order < frame.order && frame.order < text.order);
    }

    #[test]
    fn native_r_wait_keeps_text_until_decide_and_snapshot_repeats_deferred_clear() {
        use crate::runtime::{constants::cd, forms::codes};
        let (mut vm, mut host) = plain_message_machine(&[]);
        let mut s = crate::fixture::SceneBuilder::default();
        s.string("Old");
        s.code.push(cd::TEXT);
        s.i32(7);
        s.command(
            &[
                codes::ELM_GLOBAL_FRONT,
                codes::ELM_STAGE_MWND,
                codes::ELM_ARRAY,
                0,
                codes::ELM_MWND_R,
            ],
            0,
            &[],
            0,
        );
        s.string("Next");
        s.code.push(cd::TEXT);
        s.i32(8);
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        for _ in 0..20 {
            host.now += 20;
            host.engine
                .tick(20, InputFrame::default(), [320, 180])
                .unwrap();
            vm.step(&mut host, InputFrame::default(), 1000).unwrap();
            if vm.state.vm.as_ref().unwrap().ctx.wait.message_key_waiting() {
                break;
            }
        }
        let ctx = &vm.state.vm.as_ref().unwrap().ctx;
        assert!(ctx.wait.message_key_waiting());
        assert_eq!(ctx.ui.message.as_deref(), Some("Old"));
        assert!(!ctx.globals.read_flag(0, 7));
        let saved = vm.snapshot().unwrap();
        let engine_saved = host.engine.clone();
        for _ in 0..2 {
            host.now += 1;
            vm.step(
                &mut host,
                InputFrame {
                    buttons_down: 1,
                    buttons_pressed: 1,
                    ..Default::default()
                },
                1000,
            )
            .unwrap();
            assert_eq!(
                vm.state.vm.as_ref().unwrap().ctx.ui.message.as_deref(),
                Some("Old")
            );
            host.now += 1;
            vm.step(&mut host, InputFrame::default(), 1000).unwrap();
            let ctx = &vm.state.vm.as_ref().unwrap().ctx;
            assert_eq!(ctx.ui.message.as_deref(), Some("Next"));
            assert!(ctx.globals.read_flag(0, 7));
            assert!(!ctx.globals.read_flag(0, 8));
            vm.restore(&saved).unwrap();
            host.engine = engine_saved.clone();
            host.now = vm.state.now_ms;
        }
    }

    #[test]
    fn native_slide_three_and_independent_name_grid_use_shared_motion_and_layers() {
        use crate::runtime::constants::cd;
        use crysirror_scene::layer::Paint;
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert("Asset/Gameexe.ini".into(),b"#START_SCENE=\"entry\"\n#SCREEN_SIZE=320,200\n#MWND.000.WINDOW_POS=10,100\n#MWND.000.WINDOW_SIZE=300,80\n#MWND.000.MOJI_SIZE=20\n#MWND.000.MOJI_SPACE=0,12\n#MWND.000.NAME_WINDOW_SIZE=0,0\n#MWND.000.NAME_MOJI_CNT=8\n#MWND.000.NAME_MOJI_SIZE=32\n#MWND.000.NAME_MOJI_SPACE=1,10\n#MWND.000.OPEN_ANIME_TYPE=3\n#MWND.000.OPEN_ANIME_TIME=500\n".to_vec());
        let mut s = crate::fixture::SceneBuilder::default();
        s.string("AB");
        s.code.push(cd::NAME);
        s.string("Text");
        s.code.push(cd::TEXT);
        s.i32(-1);
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        host.now = 250;
        host.engine
            .tick(250, InputFrame::default(), [320, 200])
            .unwrap();
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let layer = host
            .engine
            .layers
            .get(ClientId(1), crate::message::MESSAGE_LAYER)
            .unwrap();
        assert_eq!(layer.rect[1], 170.);
        assert_eq!(layer.opacity, 127);
        let name = host
            .engine
            .layers
            .get(ClientId(1), crate::message::NAME_LAYER)
            .unwrap();
        assert_eq!(name.rect[2..], [263., 32.]);
        let Paint::GlyphRun { glyphs, .. } = &name.paint else {
            panic!("engine name glyphs")
        };
        assert_eq!(glyphs[1].baseline[0], 16.);
        let saved = vm.snapshot().unwrap();
        host.now = 500;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .unwrap()
                .rect[1],
            120.
        );
        vm.restore(&saved).unwrap();
        host.now = 250;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine
                .layers
                .get(ClientId(1), crate::message::MESSAGE_LAYER)
                .unwrap()
                .opacity,
            127
        );
    }

    fn cursor_g00_fixture() -> Vec<u8> {
        let mut data = vec![0, 32, 0, 32, 0];
        let mut payload = vec![];
        for _ in 0..128 {
            payload.push(255);
            for _ in 0..8 {
                payload.extend([11, 22, 33]);
            }
        }
        data.extend(((payload.len() + 8) as u32).to_le_bytes());
        data.extend(4096u32.to_le_bytes());
        data.extend(payload);
        data
    }

    #[test]
    fn native_cursor_default_zero_selects_custom_g00_and_script_changes_and_restore_rebind() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        use crysirror_scene::ui::cursor::{CursorSelect, StandardCursor};
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert("Asset/Gameexe.ini".into(),b"#START_SCENE=\"entry\"\n#MOUSE_CURSOR.CNT=2\n#MOUSE_CURSOR.DEFAULT=0\n#MOUSE_CURSOR.000.FILE=\"mouse\"\n#MOUSE_CURSOR.000.SPEED=100\n".to_vec());
        host.files
            .insert("Asset/g00/mouse.g00".into(), cursor_g00_fixture());
        let mut s = crate::fixture::SceneBuilder::default();
        s.command(
            &[codes::ELM_GLOBAL_SCRIPT, codes::ELM_SCRIPT_GET_CURSOR_NO],
            0,
            &[],
            fm::INT,
        );
        s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], 0);
        s.command(
            &[codes::ELM_GLOBAL_SCRIPT, codes::ELM_SCRIPT_SET_CURSOR_NO],
            0,
            &[-1],
            0,
        );
        s.command(
            &[
                codes::ELM_GLOBAL_SCRIPT,
                codes::ELM_SCRIPT_SET_MOUSE_DISP_OFF,
            ],
            0,
            &[],
            0,
        );
        s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], 0);
        s.command(
            &[codes::ELM_GLOBAL_SCRIPT, codes::ELM_SCRIPT_SET_CURSOR_NO],
            0,
            &[0],
            0,
        );
        s.command(
            &[
                codes::ELM_GLOBAL_SCRIPT,
                codes::ELM_SCRIPT_SET_MOUSE_DISP_ON,
            ],
            0,
            &[],
            0,
        );
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(100)
        );
        assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([0]));
        assert_eq!(
            host.engine.ui.cursors.selected,
            Some((1, CursorSelect::Custom { id: 0 }))
        );
        let cursor = host.engine.ui.cursors.get(1, 0).unwrap();
        assert_eq!(cursor.frames[0].hotspot, [0, 0]);
        assert_eq!(cursor.frames[0].rgba[..4], [33, 22, 11, 255]);
        assert_eq!(cursor.steps, [[0, 100]]);
        assert!(host.engine.ui.cursors.get(2, 0).is_none());
        let saved = vm.snapshot().unwrap();
        host.now = 100;
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            host.engine.ui.cursors.selected,
            Some((
                1,
                CursorSelect::Standard {
                    shape: StandardCursor::Default
                }
            ))
        );
        assert_eq!(host.engine.ui.cursor, Some((1, false)));
        host.now = 200;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(
            host.engine.ui.cursors.selected,
            Some((1, CursorSelect::Custom { id: 0 }))
        );
        assert_eq!(host.engine.ui.cursor, Some((1, true)));
        vm.restore(&saved).unwrap();
        host.engine.ui.cursors = Default::default();
        vm.restore_resources(&mut host).unwrap();
        assert!(host.engine.ui.cursors.get(1, 0).is_some());
        assert_eq!(
            host.engine.ui.cursors.selected,
            Some((1, CursorSelect::Custom { id: 0 }))
        );
    }

    #[test]
    fn invalid_native_cursor_is_not_registered_or_replaced_with_fake_default() {
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert(
            "Asset/Gameexe.ini".into(),
            b"#START_SCENE=\"entry\"\n#MOUSE_CURSOR.DEFAULT=0\n#MOUSE_CURSOR.000.FILE=\"mouse\"\n"
                .to_vec(),
        );
        host.files
            .insert("Asset/g00/mouse.g00".into(), crate::fixture::pixel_g00());
        let error = vm.step(&mut host, InputFrame::default(), 1000).unwrap_err();
        assert!(error.to_string().contains("32x32"), "{error}");
        assert!(host.engine.ui.cursors.get(1, 0).is_none());
        assert!(host.engine.ui.cursors.selected.is_none());
    }

    #[test]
    fn native_lazy_container_and_child_pct_publish_engine_owned_resource_tree() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        let (mut vm, mut host) = machine(false);
        let mut s = crate::fixture::SceneBuilder::default();
        let root = [
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
        ];
        let chain = |tail: &[i32]| [root.as_slice(), tail].concat();
        s.command(&chain(&[codes::ELM_OBJECT_INIT]), 0, &[], fm::VOID);
        s.command(&chain(&[codes::ELM_OBJECT_DISP]), 1, &[1], fm::VOID);
        s.command(&chain(&[codes::ELM_OBJECT_X]), 1, &[10], fm::VOID);
        s.command(
            &chain(&[codes::ELM_OBJECT_CHILD, codes::ELM_OBJECTLIST_RESIZE]),
            0,
            &[1],
            fm::VOID,
        );
        let child = [codes::ELM_OBJECT_CHILD, codes::ELM_ARRAY, 0];
        let member = |tail: &[i32]| chain(&[child.as_slice(), tail].concat());
        s.command_string(
            &member(&[codes::ELM_OBJECT_CREATE]),
            3,
            "pixel",
            &[1, 7, 8, 0],
            fm::VOID,
        );
        s.command(
            &member(&[codes::ELM_OBJECT_X_REP, codes::ELM_INTLIST_RESIZE]),
            0,
            &[1],
            fm::VOID,
        );
        s.command(
            &member(&[codes::ELM_OBJECT_X_REP, codes::ELM_ARRAY, 0]),
            1,
            &[5],
            fm::VOID,
        );
        s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        host.files
            .insert("Asset/g00/pixel.g00".into(), crate::fixture::pixel_g00());
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(1)
        );
        // Native set_value preserves cur_value until the next frame; the shared
        // IntEvent frame, not an eager private export update, commits the offset.
        let before = host.engine.frame([320, 180]).unwrap();
        assert_eq!(
            before
                .iter()
                .find(|p| matches!(p.paint, crysirror_scene::layer::Paint::Image { .. }))
                .unwrap()
                .quad[0][..2],
            [17., 8.]
        );
        host.now = 1;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        let frame = host.engine.frame([320, 180]).unwrap();
        let image = frame
            .iter()
            .find(|p| matches!(p.paint, crysirror_scene::layer::Paint::Image { .. }))
            .unwrap();
        assert_eq!(image.quad[0][..2], [22., 8.]);
        let crysirror_scene::layer::Paint::Image { asset, .. } = &image.paint else {
            unreachable!()
        };
        assert_eq!(
            host.files[&format!("Asset/{asset}")],
            vec![20, 200, 10, 255]
        );
        assert!(host.engine.layers.get(ClientId(1), 0x82000000).is_some());
        assert!(
            host.engine
                .layers
                .get(ClientId(1), 0x82000001)
                .unwrap()
                .parent
                .is_some()
        );
        let saved = vm.snapshot().unwrap();
        vm.restore(&saved).unwrap();
        vm.restore_resources(&mut host).unwrap();
        assert!(
            vm.diagnostics()["unknown_forms"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn native_window_mode_requests_shared_ui_and_restore_republishes() {
        use crate::runtime::{
            constants::{cd, fm},
            forms::codes,
        };
        let (mut vm, mut host) = machine(false);
        let mut s = crate::fixture::SceneBuilder::default();
        s.command(
            &[codes::ELM_GLOBAL_SYSCOM, codes::syscom_op::SET_WINDOW_MODE],
            0,
            &[-7],
            fm::VOID,
        );
        s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[1], fm::VOID);
        s.command(
            &[codes::ELM_GLOBAL_SYSCOM, codes::syscom_op::SET_WINDOW_MODE],
            0,
            &[0],
            fm::VOID,
        );
        s.code.push(cd::EOF);
        host.files.insert(
            "Asset/Scene.pck".into(),
            crate::fixture::pack_scene(&s.finish()),
        );
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(1)
        );
        assert_eq!(host.engine.ui.fullscreen, Some((1, true)));
        let saved = vm.snapshot().unwrap();
        host.now = 1;
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::Halted
        );
        assert_eq!(host.engine.ui.fullscreen, Some((1, false)));
        vm.restore(&saved).unwrap();
        vm.restore_resources(&mut host).unwrap();
        assert_eq!(host.engine.ui.fullscreen, Some((1, true)));
    }

    fn authored_two_scene_pack(main: &[u8], callback: &[u8]) -> Vec<u8> {
        let names = [
            "entry".encode_utf16().collect::<Vec<_>>(),
            "callback".encode_utf16().collect::<Vec<_>>(),
        ];
        let names_ofs = 92 + 16;
        let index_ofs = names_ofs + names.iter().map(|n| n.len() * 2).sum::<usize>();
        let data_ofs = index_ofs + 16;
        let mut h = [0i32; 23];
        h[0] = 92;
        h[13] = 92;
        h[14] = 2;
        h[15] = names_ofs as i32;
        h[16] = 2;
        h[17] = index_ofs as i32;
        h[18] = 2;
        h[19] = data_ofs as i32;
        h[20] = 2;
        let mut out = Vec::new();
        for n in h {
            out.extend(n.to_le_bytes());
        }
        let mut offset = 0i32;
        for name in &names {
            out.extend(offset.to_le_bytes());
            out.extend((name.len() as i32).to_le_bytes());
            offset += name.len() as i32;
        }
        for name in names {
            for word in name {
                out.extend(word.to_le_bytes());
            }
        }
        out.extend(0i32.to_le_bytes());
        out.extend((main.len() as i32).to_le_bytes());
        out.extend((main.len() as i32).to_le_bytes());
        out.extend((callback.len() as i32).to_le_bytes());
        out.extend(main);
        out.extend(callback);
        out
    }
    #[test]
    fn native_waku_button_uses_shared_pointer_edges_and_original_callback_once_per_activation() {
        use crate::runtime::{
            constants::{cd, fm, op},
            forms::codes,
        };
        use crysirror_core::service::buttons::MOUSE_LEFT;
        let (mut vm, mut host) = machine(false);
        vm.state.config.gameexe = Some("Gameexe.ini".into());
        host.files.insert("Asset/Gameexe.ini".into(),b"#START_SCENE=\"entry\"\n#MWND.000.WINDOW_POS=10,10\n#MWND.000.WINDOW_SIZE=100,50\n#WAKU.000.BTN.000.FILE=\"pixel\"\n#WAKU.000.BTN.000.POS=0,5,5\n#WAKU.000.BTN.000.ACTION=0\n#WAKU.000.BTN.000.SE=-1\n#WAKU.000.BTN.000.TYPE=\"none\"\n#WAKU.000.BTN.000.CALL=\"callback\",00\n".to_vec());
        host.files
            .insert("Asset/g00/pixel.g00".into(), crate::fixture::pixel_g00());
        let mut main = crate::fixture::SceneBuilder::default();
        main.command(
            &[
                codes::ELM_GLOBAL_FRONT,
                codes::ELM_STAGE_MWND,
                codes::ELM_ARRAY,
                0,
                codes::ELM_MWND_OPEN_NOWAIT,
            ],
            0,
            &[],
            fm::VOID,
        );
        main.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[5000], fm::VOID);
        main.code.push(cd::EOF);
        let mut callback = crate::fixture::SceneBuilder::default();
        let g = [codes::ELM_GLOBAL_G, codes::ELM_ARRAY, 0];
        callback.element(&g);
        callback.command(&g, 0, &[], fm::INT);
        callback.int(1);
        callback.code.push(cd::OPERATE_2);
        callback.i32(fm::INT);
        callback.i32(fm::INT);
        callback.code.push(op::PLUS);
        callback.code.push(cd::COMMAND);
        for v in [1, 1, fm::INT, 0, fm::VOID] {
            callback.i32(v);
        }
        callback.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[5], fm::VOID);
        callback.code.push(cd::RETURN);
        callback.i32(0);
        host.files.insert(
            "Asset/Scene.pck".into(),
            authored_two_scene_pack(&main.finish(), &callback.finish()),
        );
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        let widget = crate::message::buttons::WIDGET_BASE;
        let spec = host.engine.ui.get(1, widget).unwrap();
        assert!(
            spec.interaction_only && spec.visible && spec.enabled,
            "{spec:?}"
        );
        assert_eq!(spec.rect, [15., 15., 1., 1.]);
        assert!(
            host.engine
                .frame([320, 180])
                .unwrap()
                .iter()
                .any(|p| matches!(p.paint, crysirror_scene::layer::Paint::Image { .. })),
            "engine owns the actual button image"
        );
        let count = |vm: &SiglusVm| {
            vm.state
                .vm
                .as_ref()
                .unwrap()
                .ctx
                .globals
                .int_lists
                .get(&(codes::ELM_GLOBAL_G as u32))
                .and_then(|list| list.first())
                .copied()
                .unwrap_or(0)
        };
        for frame in [
            InputFrame {
                pointer_x: 15,
                pointer_y: 15,
                ..Default::default()
            },
            InputFrame {
                pointer_x: 15,
                pointer_y: 15,
                buttons_down: MOUSE_LEFT,
                buttons_pressed: MOUSE_LEFT,
                ..Default::default()
            },
            InputFrame {
                pointer_x: 15,
                pointer_y: 15,
                ..Default::default()
            },
        ] {
            host.now += 1;
            host.engine.tick(1, frame, [320, 180]).unwrap();
            let observed = host.engine.ui.interaction(1, widget).unwrap();
            eprintln!(
                "shared={observed:?} spec={:?}",
                host.engine.ui.get(1, widget).unwrap()
            );
            vm.step(&mut host, frame, 1000).unwrap();
            eprintln!(
                "native={} pending={} published={}",
                vm.diagnostics(),
                vm.buttons.pending_count(),
                vm.buttons.published.len()
            );
        }
        let observation = host.engine.ui.interaction(1, widget).unwrap();
        assert!(observation.activated, "{observation:?}");
        assert_eq!(
            count(&vm),
            1,
            "original native callback ran after a shared UI activation: {}",
            vm.diagnostics()
        );
        // Same host tick, multiple VM pumps: shared event identity prevents a
        // callback from being admitted again after a BudgetExhausted continuation.
        for _ in 0..3 {
            vm.step(
                &mut host,
                InputFrame {
                    pointer_x: 15,
                    pointer_y: 15,
                    ..Default::default()
                },
                1000,
            )
            .unwrap();
        }
        assert_eq!(count(&vm), 1);
        assert_eq!(
            vm.state.vm.as_ref().unwrap().ctx.native_proc_waits.len(),
            1,
            "main wait is suspended while callback owns its own wait"
        );
        let saved = vm.snapshot().unwrap();
        let engine_saved: crysirror_scene::EngineSystems =
            serde_json::from_slice(&serde_json::to_vec(&host.engine).unwrap()).unwrap();
        vm.restore(&saved).unwrap();
        host.engine = engine_saved;
        vm.restore_resources(&mut host).unwrap();
        host.now += 1;
        host.engine
            .tick(
                1,
                InputFrame {
                    pointer_x: 15,
                    pointer_y: 15,
                    ..Default::default()
                },
                [320, 180],
            )
            .unwrap();
        vm.step(&mut host, InputFrame::default(), 1000).unwrap();
        assert_eq!(
            count(&vm),
            1,
            "restore does not replay the already-consumed activation"
        );
        assert_eq!(vm.state.vm.as_ref().unwrap().ctx.native_proc_waits.len(), 1);
        host.now = 8;
        host.engine
            .tick(
                4,
                InputFrame {
                    pointer_x: 15,
                    pointer_y: 15,
                    ..Default::default()
                },
                [320, 180],
            )
            .unwrap();
        assert_eq!(
            vm.step(&mut host, InputFrame::default(), 1000).unwrap(),
            VmYield::WaitUntil(5000)
        );
        assert!(
            vm.state
                .vm
                .as_ref()
                .unwrap()
                .ctx
                .native_proc_waits
                .is_empty()
        );
        assert_eq!(vm.diagnostics()["scene"], "entry");
        assert_eq!(count(&vm), 1);
    }
}
