// SPDX-License-Identifier: MPL-2.0
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Asset path of the compiled scene: a `Scene.pck` or a single `.scn`.
    pub scene: String,
    /// "pck" (scene pack, default) or "scn" (one scene stream).
    #[serde(default = "format")]
    pub format: String,
    /// Entry scene; default "entry" for scn, the pack's scene 0 for pck.
    #[serde(default)]
    pub scene_name: Option<String>,
    /// Scene string encryption: "xor" (default) or "plain".
    #[serde(default = "codec")]
    pub string_codec: String,
    /// Asset path of the decoded UTF-8 Gameexe ini, if any.
    #[serde(default)]
    pub gameexe: Option<String>,
    /// Native resource path → asset path aliases (at most 4096).
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    /// Explicit mapping for the bounded plain-text MWND profile. Native
    /// Gameexe windows/markup are not guessed from a project canvas.
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
        if let Some(g) = &self.gameexe {
            path(g)?;
        }
        if let Some(window) = &self.message_window {
            window.validate()?;
            ensure!(
                self.gameexe.is_none(),
                "native Gameexe MWND conversion is not implemented; use an explicit plain-window profile without Gameexe"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Machine {
    client: u32,
    config: Config,
    vm: Option<SceneVm<'static>>,
    #[serde(skip)]
    io: io_fs::State,
    last_buttons: u64,
    now_ms: u64,
    published: BTreeSet<u32>,
}
pub struct SiglusVm {
    state: Machine,
}
fn err(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}
fn binary() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(32 * 1024 * 1024)
        .reject_trailing_bytes()
}
impl SiglusVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            state: Machine {
                client: client.0,
                config,
                vm: None,
                io: Default::default(),
                last_buttons: 0,
                now_ms: 0,
                published: Default::default(),
            },
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
            let text = String::from_utf8(b.as_ref().clone())
                .context("Gameexe input must be decoded UTF-8 ini")?;
            ctx.tables.gameexe = Some(crate::gameexe::GameexeConfig::from_text(&text));
        }
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
            let mut options = ScenePckDecodeOptions::default();
            options.string_encryption_override = if self.state.config.string_codec == "plain" {
                crate::key_toml::StringEncryptionOverride::None
            } else {
                crate::key_toml::StringEncryptionOverride::Xor
            };
            let pck = ScenePck::load_lazy_from_bytes(bytes, &options)?;
            let name = self
                .state
                .config
                .scene_name
                .clone()
                .or_else(|| pck.find_scene_name(0).map(str::to_owned))
                .unwrap_or_else(|| "0".into());
            let n = pck
                .find_scene_no(&name)
                .context("native entry scene not found")?;
            let (owner, range) = pck.scn_data_shared(n)?;
            let stream =
                SceneStream::new_shared_range_with_string_codec(owner, range, pck.string_codec)?;
            ctx.install_scene_metadata("", &pck)?;
            let mut vm = SceneVm::new(stream, ctx);
            vm.install_initial_scene_pck(pck, String::new());
            vm.restart_scene_name(&name, 0)?;
            vm
        };
        vm.cfg.max_steps = 1_000_000;
        vm.engine_validate()?;
        self.state.vm = Some(vm);
        Ok(())
    }
    fn execute(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        self.state.now_ms = host.now_ms();
        let now = self.state.now_ms;
        platform_time::scoped(now, || -> Result<VmYield> {
            if self.state.vm.is_none() {
                self.boot(host)?;
            }
            {
                let vm = self.state.vm.as_mut().unwrap();
                crate::message::sync_progress(&mut vm.ctx, ClientId(self.state.client), host)?;
                vm.ctx.input.next_frame();
                for (bit, key) in [
                    (1, crysirror_scene::input::VmKey::Enter),
                    (2, crysirror_scene::input::VmKey::Space),
                    (4, crysirror_scene::input::VmKey::ArrowLeft),
                    (8, crysirror_scene::input::VmKey::ArrowRight),
                    (16, crysirror_scene::input::VmKey::ArrowUp),
                    (32, crysirror_scene::input::VmKey::ArrowDown),
                    (64, crysirror_scene::input::VmKey::Tab),
                ] {
                    if input.buttons_pressed & bit != 0
                        || (input.buttons_down & bit != 0 && self.state.last_buttons & bit == 0)
                    {
                        vm.ctx.input.on_key_down(key);
                    }
                    if input.buttons_down & bit == 0
                        && (self.state.last_buttons & bit != 0 || input.buttons_pressed & bit != 0)
                    {
                        vm.ctx.input.on_key_up(key);
                        // Original on_key_up only releases wait-key procs after a
                        // completed DECIDE down-up, not an arbitrary key-down.
                        if matches!(
                            key,
                            crysirror_scene::input::VmKey::Enter
                                | crysirror_scene::input::VmKey::Space
                        ) {
                            let vk = crysirror_scene::input::vmkey_to_vk_code(key).unwrap();
                            if vm.ctx.input.vk_down_up_stock(vk) {
                                if crate::message::complete_on_decide(
                                    &mut vm.ctx,
                                    ClientId(self.state.client),
                                    host,
                                )? {
                                    continue;
                                }
                                let skipped = vm.ctx.wait.notify_movie_down_up(
                                    &mut vm.ctx.globals,
                                    &vm.ctx.ids,
                                    1,
                                );
                                if !skipped {
                                    vm.ctx.wait.notify_key(&mut vm.ctx.globals, &vm.ctx.ids);
                                }
                            }
                        }
                    }
                }
                vm.ctx.input.on_mouse_move(input.pointer_x, input.pointer_y);
                let bit = 1 << 16;
                let mouse = crate::runtime::input::VmMouseButton::Left;
                if input.buttons_pressed & bit != 0
                    || (input.buttons_down & bit != 0 && self.state.last_buttons & bit == 0)
                {
                    vm.ctx.input.on_mouse_down(mouse);
                }
                if input.buttons_down & bit == 0
                    && (self.state.last_buttons & bit != 0 || input.buttons_pressed & bit != 0)
                {
                    vm.ctx.input.on_mouse_up(mouse);
                    if vm.ctx.input.vk_down_up_stock(0x01) {
                        let revealed = crate::message::complete_on_decide(
                            &mut vm.ctx,
                            ClientId(self.state.client),
                            host,
                        )?;
                        let skipped = !revealed
                            && vm.ctx.wait.notify_movie_down_up(
                                &mut vm.ctx.globals,
                                &vm.ctx.ids,
                                1,
                            );
                        if !skipped && !revealed {
                            vm.ctx.wait.notify_key(&mut vm.ctx.globals, &vm.ctx.ids);
                        }
                    }
                }
                vm.ctx.sync_script_input_from_runtime();
                vm.begin_script_proc_pump();
                vm.tick_frame()?;
            }
            self.state.last_buttons = input.buttons_down;
            for _ in 0..budget.min(65536) {
                let before = self.state.vm.as_ref().unwrap().clone();
                let mut io_before = self.state.io.clone();
                let mut attempt = 0;
                loop {
                    attempt += 1;
                    ensure!(attempt <= 32, "native resource resolution retry limit");
                    let generation = self.state.vm.as_ref().unwrap().proc_generation();
                    self.state.io.missing.clear();
                    self.state.io.writes.clear();
                    let result = io_fs::scoped(&mut self.state.io, || {
                        self.state.vm.as_mut().unwrap().step()
                    });
                    if !self.state.io.missing.is_empty() {
                        self.state.vm = Some(before.clone());
                        let keys = std::mem::take(&mut self.state.io.missing);
                        self.state.io = io_before.clone();
                        for key in keys {
                            self.fetch(host, &key)?;
                        }
                        io_before = self.state.io.clone();
                        continue;
                    }
                    result?;
                    let vm = self.state.vm.as_mut().unwrap();
                    vm.engine_reject_unknown()?;
                    for (key, bytes) in std::mem::take(&mut self.state.io.writes) {
                        host.write_atomic(ClientId(self.state.client), &Self::path(&key)?, &bytes)
                            .map_err(err)?;
                    }
                    let halted = vm.is_halted();
                    let blocked = vm.is_blocked();
                    let boundary = vm.proc_generation() != generation;
                    if halted || blocked || boundary {
                        self.publish(host)?;
                        let vm = self.state.vm.as_ref().unwrap();
                        return Ok(if halted {
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
            self.publish(host)?;
            Ok(VmYield::BudgetExhausted)
        })
    }
    fn publish(&mut self, host: &mut dyn HostServices) -> Result<()> {
        for _ in 0..32 {
            self.state.io.missing.clear();
            let result = io_fs::scoped(&mut self.state.io, || {
                crate::export::publish(
                    &mut self.state.vm.as_mut().unwrap().ctx,
                    ClientId(self.state.client),
                    &mut self.state.published,
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
            self.state.config.message_window.is_none(),
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
            self.state.config.message_window.is_none(),
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
        if result.is_err() {
            self.state = previous;
        }
        result
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        self.state
            .vm
            .as_ref()
            .map(|v| v.engine_diagnostics())
            .unwrap_or_else(|| serde_json::json!({"vm":"original-siglus","booted":false}))
    }
}
impl VmAdapter for SiglusVm {
    fn engine_id(&self) -> &'static str {
        "siglus"
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
        self.execute(host, input, budget).map_err(err)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        binary().serialize(&self.state).map_err(err)
    }
    fn restore(&mut self, bytes: &[u8]) -> crysirror_core::Result<()> {
        let state: Machine = binary().deserialize(bytes).map_err(err)?;
        if state.client != self.state.client || state.config != self.state.config {
            return Err(Error::SnapshotMismatch("native client/config"));
        }
        state.config.validate().map_err(err)?;
        if let Some(vm) = &state.vm {
            vm.engine_validate().map_err(err)?;
            if vm.ctx.ui.window != state.config.message_window {
                return Err(Error::SnapshotMismatch("native message configuration"));
            }
        }
        self.state = state;
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
        Ok(())
    }
    fn export_original_save(&mut self) -> crysirror_core::Result<Vec<u8>> {
        SiglusVm::export_original_save(self).map_err(err)
    }
    fn import_original_save(&mut self, bytes: &[u8]) -> crysirror_core::Result<()> {
        SiglusVm::import_original_save(self, bytes).map_err(err)
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
        fn query_engine(
            &mut self,
            client: ClientId,
            query: crysirror_core::service::EngineQuery,
        ) -> crysirror_core::Result<Vec<u8>> {
            match query {
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
            if font != "message-font" {
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
        let crysirror_scene::layer::Paint::GlyphRun { glyphs, .. } =
            &host.engine.layers.get(ClientId(1), crate::message::MESSAGE_LAYER).unwrap().paint
            else { panic!("glyph run") };
        assert_eq!(glyphs[0].baseline, [0., 45.]);
        assert!(vm.export_original_save().unwrap_err().to_string().contains("MWND reconstruction"));
        assert!(vm.import_original_save(b"native").unwrap_err().to_string().contains("MWND reconstruction"));
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
}
