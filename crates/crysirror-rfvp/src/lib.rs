// SPDX-License-Identifier: MPL-2.0
//! RFVP compatibility frontend (ADR 0008).
//!
//! The vendored original engine (`crates/rfvp`, headless `soft-render-core`
//! profile) keeps its HCB parser, VM, thread manager, syscalls, motion/text
//! managers, pack VFS, save manager and its own software renderer. CrySirror
//! supplies only the audited host seams:
//!
//! | RFVP seam | CrySirror service |
//! |---|---|
//! | `RfvpStorage` (packs, loose files, `save/`) | `ClientStorage` capability handle |
//! | `Time::set_external_delta` | runtime clock (`HostServices::now_ms`) |
//! | `InputManager::notify_*` | `InputFrame` + `buttons` |
//! | `SoftFramebuffer` | `present_surface` + one retained Layer |
//! | built-in MS fonts (not redistributed) | host-supplied font asset |
//!
//! Limits (reported, not hidden): one RFVP client per process because the
//! original keeps its globals in a process-wide `GLOBAL`; no audio output
//! (`no-audio` profile); no movie playback; CrySirror runtime snapshots are
//! refused — use the game's own save slots, which reach the client save mount.
#![forbid(unsafe_code)]
use anyhow::{bail, ensure, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{buttons, ClientStorage, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use rfvp::{
    crysirror_host::{install_builtin_fonts, BuiltinFonts, EmbeddedRuntime, HostEvent, HostKey},
    host_api::{RfvpError, RfvpResult, RfvpStorage},
    script::parser::Nls,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const ENGINE_ID: &str = "rfvp";
static INSTANCE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Asset directory holding the game's `.hcb`, packs and loose folders.
    #[serde(default)]
    pub root: String,
    /// Script text encoding: "sjis" (default), "gbk" or "utf8".
    #[serde(default)]
    pub nls: Option<String>,
    /// Asset path of a host-supplied TrueType/OpenType font used for the
    /// original's built-in MS Gothic/Mincho faces. Required; never bundled.
    pub font: String,
}
impl Config {
    fn nls(&self) -> Result<Nls> {
        Ok(match self.nls.as_deref().unwrap_or("sjis") {
            "sjis" => Nls::ShiftJIS,
            "gbk" => Nls::GBK,
            "utf8" => Nls::UTF8,
            other => bail!("unknown RFVP nls '{other}'"),
        })
    }
    pub fn validate(&self) -> Result<()> {
        if !self.root.is_empty() {
            VfsPath::new(Mount::Asset, &self.root).map_err(|e| anyhow::anyhow!("root: {e}"))?;
        }
        VfsPath::new(Mount::Asset, &self.font).map_err(|e| anyhow::anyhow!("font: {e}"))?;
        self.nls()?;
        Ok(())
    }
}

/// `RfvpStorage` over one client's capability storage: `save/...` maps to the
/// save mount, everything else to the read-only asset mount under `root`.
struct Bridge {
    storage: Arc<dyn ClientStorage>,
    root: String,
}
fn rfvp_err(e: Error) -> RfvpError {
    match e {
        Error::NotFound => RfvpError::NotFound,
        Error::Invalid(_) => RfvpError::InvalidArgument,
        Error::Unsupported(_) | Error::ReadOnly => RfvpError::Unsupported,
        Error::Limit(_) => RfvpError::CapacityExceeded,
        _ => RfvpError::Io,
    }
}
impl Bridge {
    fn map(&self, path: &str) -> RfvpResult<VfsPath> {
        let path = path.replace('\\', "/");
        let path = path.trim_start_matches("./");
        let (mount, rel) = match path.strip_prefix("save/") {
            Some(rest) => (Mount::Save, format!("save/{rest}")),
            None if path == "save" => (Mount::Save, "save".into()),
            None if self.root.is_empty() => (Mount::Asset, path.to_owned()),
            None => (Mount::Asset, format!("{}/{path}", self.root)),
        };
        VfsPath::new(mount, &rel).map_err(rfvp_err)
    }
}
impl RfvpStorage for Bridge {
    fn read(&self, path: &str) -> RfvpResult<Vec<u8>> {
        self.storage.read(&self.map(path)?).map_err(rfvp_err)
    }
    fn write(&self, path: &str, bytes: &[u8]) -> RfvpResult<()> {
        self.storage
            .write_atomic(&self.map(path)?, bytes)
            .map_err(rfvp_err)
    }
    fn exists(&self, path: &str) -> bool {
        self.map(path).is_ok_and(|p| self.storage.exists(&p))
    }
    fn remove(&self, path: &str) -> RfvpResult<()> {
        self.storage.remove(&self.map(path)?).map_err(rfvp_err)
    }
    fn list(&self, dir: &str) -> RfvpResult<Vec<String>> {
        let dir = dir.trim_matches('/');
        if dir == "save" || dir.starts_with("save/") {
            return self.storage.list(Mount::Save, dir).map_err(rfvp_err);
        }
        let full = match (self.root.is_empty(), dir.is_empty()) {
            (true, _) => dir.to_owned(),
            (false, true) => self.root.clone(),
            (false, false) => format!("{}/{dir}", self.root),
        };
        self.storage.list(Mount::Asset, &full).map_err(rfvp_err)
    }
}

pub struct RfvpVm {
    client: ClientId,
    config: Config,
    machine: Option<EmbeddedRuntime>,
    last_ms: u64,
    last_buttons: u64,
    last_pointer: (i32, i32),
    frames: u64,
    layer_published: bool,
    owns_instance: bool,
}
impl RfvpVm {
    /// Dormant construction: no file, global or device side effects.
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            machine: None,
            last_ms: 0,
            last_buttons: 0,
            last_pointer: (i32::MIN, i32::MIN),
            frames: 0,
            layer_published: false,
            owns_instance: false,
        })
    }

    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        ensure!(
            !INSTANCE.swap(true, Ordering::SeqCst),
            "only one RFVP client per process: the original engine keeps process-wide globals"
        );
        self.owns_instance = true;
        let storage = host
            .storage(self.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let bridge = Arc::new(Bridge {
            storage: storage.clone(),
            root: self.config.root.clone(),
        });

        let font = storage
            .read(
                &VfsPath::new(Mount::Asset, &self.config.font)
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            )
            .map_err(|e| anyhow::anyhow!("RFVP font asset '{}': {e}", self.config.font))?;
        let font = rfvp::font::Font::from_vec(font)
            .map_err(|_| anyhow::anyhow!("RFVP font asset is not a valid font"))?;
        install_builtin_fonts(BuiltinFonts {
            gothic: font,
            mincho: None,
            pgothic: None,
            pmincho: None,
        });

        let runtime = EmbeddedRuntime::new(bridge, self.config.nls()?)?;
        self.machine = Some(runtime);
        Ok(())
    }

    fn input(&mut self, input: InputFrame) {
        let m = self.machine.as_mut().unwrap();
        let (w, h) = m.size();
        let p = (input.pointer_x, input.pointer_y);
        if p != self.last_pointer {
            let inside = p.0 >= 0 && p.1 >= 0 && (p.0 as u32) < w && (p.1 as u32) < h;
            m.push_event(HostEvent::PointerMove {
                x: p.0,
                y: p.1,
                inside,
            });
            self.last_pointer = p;
        }
        let keys = [
            (buttons::ENTER, HostKey::Enter),
            (buttons::SPACE, HostKey::Space),
            (buttons::LEFT, HostKey::Left),
            (buttons::RIGHT, HostKey::Right),
            (buttons::UP, HostKey::Up),
            (buttons::DOWN, HostKey::Down),
            (buttons::TAB, HostKey::Tab),
            (buttons::MOUSE_LEFT, HostKey::MouseLeft),
            (buttons::MOUSE_RIGHT, HostKey::MouseRight),
        ];
        for (bit, key) in keys {
            let was = self.last_buttons & bit != 0;
            let pressed = input.buttons_pressed & bit != 0;
            let down = input.buttons_down & bit != 0;
            // A press within one host frame is delivered as down then up.
            if (down || pressed) && !was {
                m.push_event(HostEvent::Down(key));
            }
            if !down && (was || pressed) {
                m.push_event(HostEvent::Up(key));
            }
        }
        self.last_buttons = input.buttons_down;
    }

    fn present(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let m = self.machine.as_ref().unwrap();
        let fb = m.framebuffer();
        let (w, h) = m.size();
        let path = host
            .present_surface(self.client, 0, w, h, fb.pixels())
            .map_err(|e| anyhow::anyhow!("present RFVP frame: {e}"))?;
        if !self.layer_published {
            let batch = serde_json::json!({"layers":[{"op":"put","layer":{
                "id":0,"rect":[0.0,0.0,w as f32,h as f32],
                "paint":{"kind":"image","asset":path.relative()}}}]});
            host.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)
                .map_err(|e| anyhow::anyhow!("publish RFVP layer: {e}"))?;
            self.layer_published = true;
        }
        Ok(())
    }

    fn execute(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        let now = host.now_ms();
        if self.machine.is_none() {
            self.boot(host)?;
            self.last_ms = now;
        }
        let delta = now.saturating_sub(self.last_ms);
        self.last_ms = now;
        self.input(input);
        self.machine.as_mut().unwrap().step_frame(delta)?;
        self.frames += 1;
        self.present(host)?;
        Ok(if self.machine.as_ref().unwrap().should_exit() {
            VmYield::Halted
        } else {
            VmYield::BudgetExhausted
        })
    }
}
impl Drop for RfvpVm {
    fn drop(&mut self) {
        if self.owns_instance {
            INSTANCE.store(false, Ordering::SeqCst);
        }
    }
}
fn core_err(e: anyhow::Error) -> Error {
    Error::Io(format!("{e:#}"))
}
impl VmAdapter for RfvpVm {
    fn engine_id(&self) -> &'static str {
        ENGINE_ID
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
        // RFVP is frame-driven: its VM runs until every thread yields a frame.
        self.execute(host, input).map_err(core_err)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "RFVP runtime snapshot; use the game's own save slots",
        ))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "RFVP runtime snapshot; use the game's own save slots",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let v = match &self.machine {
            None => serde_json::json!({"vm":"original-rfvp","booted":false}),
            Some(m) => serde_json::json!({
                "vm":"original-rfvp","booted":true,"title":m.title(),"screen":[m.size().0,m.size().1],
                "frames":self.frames,"main_thread_exited":m.main_thread_exited(),
                "input":m.input_snapshot(),"threads":m.thread_states()}),
        };
        serde_json::to_vec(&v).unwrap_or_default()
    }
}

/// Runtime factory: `"adapter": "rfvp"` with an [`Config`] object.
pub struct RfvpFactory;
impl crysirror_runtime::runtime::VmFactory for RfvpFactory {
    fn create(
        &self,
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "RFVP executes its own HCB bytecode; the data-VM program must be empty"
        );
        let config: Config = serde_json::from_value(spec.config.clone())
            .map_err(|e| anyhow::anyhow!("RFVP config: {e}"))?;
        Ok(Box::new(RfvpVm::new(client, config)?))
    }
}
/// Register the RFVP frontend in a runtime adapter registry.
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register(ENGINE_ID, Arc::new(RfvpFactory))
}
