// SPDX-License-Identifier: MPL-2.0
//! The Ren'Py VM adapter: runs the native runtime (`crate::rt`) on its
//! own script thread and applies each tick's output to `HostServices`.
//!
//! The script thread owns every Python object. The owner thread prepares
//! the client's scoped files, starts `rt::boot::run_session` with the
//! shared image and font services, then exchanges one tick at a time over
//! the bounded link (`rt::link`): host time, input, audio status and image
//! publication results go to the script; retained layer operations, audio
//! commands, image resources to publish and the next wait come back.
use crate::{
    files::{GameFiles, ScopedFiles},
    host_audio::HostAudio,
    rt::{
        self, boot,
        link::{self, FromScript, Output, Tick, ToScript, Wait},
        state::Profile,
    },
};
use crysirror_core::{
    Error, Result,
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_runtime::{AdapterRegistry, model::ClientSpec, runtime::VmFactory};
use crysirror_tasks::{Task, TaskPool};
use serde::Deserialize;
use serde_json::{Value as J, json};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const ENGINE_ID: &str = "renpy";

/// Script-thread stack: Ren'Py nests interactions inside native frames.
const SCRIPT_STACK: usize = 256 << 20;
/// How long one owner step waits for the script thread's tick.
const TICK_WAIT: Duration = Duration::from_millis(12);

#[derive(Deserialize)]
pub struct Config {
    pub version: [u32; 4],
    pub canvas: [i32; 2],
    #[serde(default)]
    pub archives: Vec<String>,
}

impl Config {
    fn validate(&self) -> Result<Profile> {
        if !self.canvas.iter().all(|side| (240..=16384).contains(side)) {
            return Err(Error::Invalid("Ren'Py canvas"));
        }
        if self.archives.len() > 64 {
            return Err(Error::Limit("Ren'Py archive count"));
        }
        if !Profile::supported(self.version) {
            return Err(Error::Unsupported("Ren'Py runtime version"));
        }
        Ok(Profile::new(self.version))
    }
}

struct Factory;

impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> anyhow::Result<Box<dyn VmAdapter>> {
        let config: Config = serde_json::from_value(spec.config.clone())?;
        let profile = config.validate()?;
        Ok(Box::new(Adapter {
            client,
            config,
            profile,
            preparation: None,
            pool: None,
            files: None,
            session: None,
            layers: Default::default(),
            audio: HostAudio::default(),
            published: Vec::new(),
            halted: false,
            diagnostics: VecDeque::new(),
            title: None,
            text_rect: None,
            text_session: None,
        }))
    }
}

pub fn register(registry: &mut AdapterRegistry) -> anyhow::Result<()> {
    registry.register(ENGINE_ID, Arc::new(Factory))
}

/// The runtime's read-only view of the client's game files.
struct RtFiles(Arc<ScopedFiles>);

impl rt::state::Files for RtFiles {
    fn list(&self) -> Vec<String> {
        self.0.list()
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.read(name)
    }
    fn common_list(&self) -> Vec<String> {
        self.0.common_list()
    }
    fn common_read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.common_read(name)
    }
    fn temporary_storage(&self) -> Option<&dyn crysirror_core::service::ClientStorage> {
        self.0.temporary_storage()
    }
}

struct Session {
    to_script: SyncSender<ToScript>,
    from_script: Receiver<FromScript>,
    thread: Option<JoinHandle<()>>,
    /// A tick was sent and its output has not arrived.
    awaiting: bool,
    wait: Wait,
    last_input: InputFrame,
}

struct Adapter {
    client: ClientId,
    config: Config,
    profile: Profile,
    preparation: Option<Task<ScopedFiles>>,
    pool: Option<TaskPool>,
    files: Option<Arc<ScopedFiles>>,
    session: Option<Session>,
    /// Layer ids put by the last frame, with their parents.
    layers: std::collections::BTreeMap<u64, Option<u64>>,
    audio: HostAudio,
    /// Image publications to report in the next tick.
    published: Vec<(u32, std::result::Result<String, String>)>,
    halted: bool,
    diagnostics: VecDeque<String>,
    title: Option<String>,
    text_rect: Option<[i32; 4]>,
    /// Host text-input session while a Ren'Py input is focused, with the
    /// cursor area it was configured for.
    text_session: Option<(crysirror_core::service::TextInputSessionHandle, [i32; 4])>,
}

impl Adapter {
    fn note(&mut self, msg: String) {
        if self.diagnostics.len() >= 256 {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(msg);
    }

    /// Indexes the client's files on a worker. True once they are ready.
    fn prepare(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        if self.files.is_some() {
            return Ok(true);
        }
        if self.preparation.is_none() {
            let storage = host.storage(self.client)?;
            let archives = self.config.archives.clone();
            let pool =
                TaskPool::new("renpy-prepare", 1, 1).map_err(|e| Error::Io(e.to_string()))?;
            let task = pool
                .spawn(move |cancel| {
                    Ok(ScopedFiles::load(storage, &archives, || {
                        cancel.check().map_err(|e| Error::Io(e.to_string()))
                    })?)
                })
                .map_err(|e| Error::Io(e.to_string()))?;
            self.preparation = Some(task);
            self.pool = Some(pool);
            return Ok(false);
        }
        let task = self
            .preparation
            .as_mut()
            .expect("pending Ren'Py preparation");
        let Some(files) = task.try_take().map_err(|e| Error::Io(e.to_string()))? else {
            return Ok(false);
        };
        self.preparation = None;
        self.pool = None;
        self.files = Some(Arc::new(files));
        Ok(true)
    }

    fn start(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let files = self.files.clone().expect("prepared files");
        let images = host.image_metadata(self.client).ok();
        let fonts = host.font_library(self.client).ok();
        let saves = host.storage(self.client).ok();
        let canvas = self.config.canvas;
        let profile = self.profile.clone();
        let (to_script, from_owner) = mpsc::sync_channel::<ToScript>(1);
        let (to_owner, from_script) = mpsc::sync_channel::<FromScript>(1);
        let first = Tick {
            now_ms: host.now_ms(),
            ..Tick::default()
        };
        let thread = std::thread::Builder::new()
            .name("renpy-script".into())
            .stack_size(SCRIPT_STACK)
            .spawn(move || {
                let rt_files: Arc<dyn rt::state::Files> = Arc::new(RtFiles(files.clone()));
                let l = link::Link::new(from_owner, to_owner, first);
                boot::run_session(profile, rt_files, l, &|vm| {
                    let h = rt::host(vm);
                    let mut h = h.borrow_mut();
                    h.images = images.clone();
                    h.fonts = fonts.clone();
                    h.saves = saves.clone();
                    h.canvas = canvas;
                    let f = files.clone();
                    h.assets = Some(std::rc::Rc::new(move |name: &str| {
                        f.asset_path(name).map(|p| p.relative().to_owned())
                    }));
                });
            })
            .map_err(|e| Error::Io(format!("Ren'Py script thread: {e}")))?;
        self.session = Some(Session {
            to_script,
            from_script,
            thread: Some(thread),
            awaiting: true,
            wait: Wait::NextFrame,
            last_input: InputFrame::default(),
        });
        Ok(())
    }

    /// Applies one tick's output to the host.
    fn apply(&mut self, host: &mut dyn HostServices, out: Output) -> Result<()> {
        let files = self.files.clone().expect("prepared files");
        for d in out.diagnostics {
            self.note(d);
        }
        if let Some(ops) = out.frame {
            let mut ids = std::collections::BTreeMap::new();
            for op in &ops {
                if op["op"] == "put"
                    && let Some(id) = op["layer"]["id"].as_u64()
                {
                    ids.insert(id, op["layer"]["parent"].as_u64());
                }
            }
            // Removing a layer removes its subtree, so only the topmost
            // layer of each vanished subtree is removed.
            let gone: BTreeSet<u64> = self
                .layers
                .keys()
                .filter(|id| !ids.contains_key(id))
                .copied()
                .collect();
            let under_gone = |mut id: u64| {
                while let Some(Some(p)) = self.layers.get(&id) {
                    if gone.contains(p) {
                        return true;
                    }
                    id = *p;
                }
                false
            };
            let mut layers: Vec<J> = gone
                .iter()
                .filter(|id| !under_gone(**id))
                .map(|id| json!({"op": "remove", "id": id}))
                .collect();
            layers.extend(ops);
            self.layers = ids;
            if let Some(path) = std::env::var_os("CRYSIRROR_RENPY_DUMP_FRAME") {
                let _ =
                    std::fs::write(path, serde_json::to_vec_pretty(&layers).unwrap_or_default());
            }
            host.submit_engine(
                self.client,
                1,
                json!({"layers": layers}).to_string().as_bytes(),
            )?;
        }
        if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some() {
            for c in &out.audio {
                self.note(format!("trace: audio {c}"));
            }
        }
        self.audio.apply(host, self.client, &files, out.audio)?;
        for d in std::mem::take(&mut self.audio.diagnostics) {
            self.note(d);
        }
        self.published
            .extend(rt::im::publish(host, self.client, out.publish));
        if let Some(t) = out.title {
            self.title = Some(t);
        }
        self.text_rect = out.text_rect;
        self.sync_text_session(host)?;
        for s in out.speech {
            self.note(format!("self-voicing: {s}"));
        }
        Ok(())
    }

    /// Opens, moves or releases the host text-input session to follow the
    /// focused input's rectangle (`pygame.key.start_text_input`,
    /// `set_text_input_rect`, `stop_text_input`).
    fn sync_text_session(&mut self, host: &mut dyn HostServices) -> Result<()> {
        use crysirror_core::service::TextInputSessionSpec;
        let spec = |rect: [i32; 4]| TextInputSessionSpec {
            capture: true,
            ime_allowed: true,
            cursor_area: Some([rect[0], rect[1], rect[2].max(1), rect[3].max(1)]),
            ..Default::default()
        };
        match (self.text_rect, self.text_session) {
            (Some(rect), None) => {
                if let Ok(h) = host.open_text_input_session(self.client, spec(rect)) {
                    self.text_session = Some((h, rect));
                }
            }
            (Some(rect), Some((h, old))) if rect != old => {
                host.configure_text_input_session(self.client, h, spec(rect))?;
                self.text_session = Some((h, rect));
            }
            (None, Some((h, _))) => {
                self.text_session = None;
                host.release_text_input_session(self.client, h)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn input_changed(last: &InputFrame, input: &InputFrame) -> bool {
        input.buttons_pressed != 0
            || input.buttons_down != last.buttons_down
            || input.pointer_x != last.pointer_x
            || input.pointer_y != last.pointer_y
    }

    fn shutdown(&mut self, host: &mut dyn HostServices) {
        if let Some((h, _)) = self.text_session.take() {
            let _ = host.release_text_input_session(self.client, h);
        }
        if let Some(mut s) = self.session.take() {
            let _ = s.to_script.try_send(ToScript::Shutdown);
            drop(s.to_script);
            // The script thread unwinds with QuitException; draining lets it
            // deliver its final message.
            while s
                .from_script
                .recv_timeout(Duration::from_millis(500))
                .is_ok()
            {}
            if let Some(t) = s.thread.take() {
                let _ = t.join();
            }
        }
        self.audio.stop_all(host, self.client);
    }
}

impl VmAdapter for Adapter {
    fn engine_id(&self) -> &'static str {
        ENGINE_ID
    }

    fn snapshot_schema(&self) -> u32 {
        2
    }

    fn snapshot(&self) -> Result<Vec<u8>> {
        Err(Error::Unsupported(
            "Ren'Py runtime snapshot (use the game's own save slots)",
        ))
    }

    fn restore(&mut self, _: &[u8]) -> Result<()> {
        Err(Error::Unsupported(
            "Ren'Py runtime snapshot (use the game's own save slots)",
        ))
    }

    fn preparation_pending(&self) -> bool {
        self.session.is_none() && !self.halted
    }

    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        _budget: u32,
    ) -> Result<VmYield> {
        if self.halted {
            return Ok(VmYield::Halted);
        }
        if self.session.is_none() {
            if self.prepare(host)? {
                self.start(host)?;
            }
            return Ok(VmYield::BudgetExhausted);
        }
        let now = host.now_ms();
        let files = self.files.clone().expect("prepared files");
        let text_pending = self.text_session.is_some_and(|(h, _)| {
            host.text_input_session_info(self.client, h).is_ok_and(|i| !i.committed.is_empty())
        });
        let send = {
            let s = self.session.as_ref().expect("session");
            let woken = text_pending || Self::input_changed(&s.last_input, &input);
            !s.awaiting
                && match s.wait {
                    Wait::NextFrame => true,
                    Wait::Until(t) => now >= t || woken,
                    Wait::Input => woken,
                }
        };
        if send {
            let audio = self.audio.status(host, self.client, &files)?;
            let published = std::mem::take(&mut self.published);
            let text = match self.text_session {
                Some((h, _)) => host.take_text_input_committed(self.client, h).map(|t| (*t).clone()).unwrap_or_default(),
                None => String::new(),
            };
            if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some()
                && Self::input_changed(&self.session.as_ref().expect("session").last_input, &input)
            {
                self.note(format!("trace: tick {now} input {input:?}"));
            }
            let s = self.session.as_mut().expect("session");
            s.last_input = input;
            if s.to_script
                .send(ToScript::Tick(Tick {
                    now_ms: now,
                    input,
                    audio,
                    published,
                    text,
                }))
                .is_err()
            {
                self.halted = true;
                return Err(Error::Io(
                    "the Ren'Py script thread ended unexpectedly".into(),
                ));
            }
            s.awaiting = true;
        }
        let s = self.session.as_mut().expect("session");
        if !s.awaiting {
            return Ok(match s.wait {
                Wait::NextFrame => VmYield::BudgetExhausted,
                _ => VmYield::WaitForInput,
            });
        }
        match s.from_script.recv_timeout(TICK_WAIT) {
            Ok(FromScript::Yield { out, wait }) => {
                s.awaiting = false;
                s.wait = wait;
                self.apply(host, out)?;
                Ok(match wait {
                    Wait::NextFrame => VmYield::BudgetExhausted,
                    _ => VmYield::WaitForInput,
                })
            }
            Ok(FromScript::Exit { out, result }) => {
                self.apply(host, out)?;
                self.halted = true;
                self.shutdown(host);
                match result {
                    Ok(()) => Ok(VmYield::Halted),
                    Err(e) => {
                        self.note(e.clone());
                        Err(Error::Io(format!("Ren'Py: {e}")))
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => Ok(VmYield::BudgetExhausted),
            Err(RecvTimeoutError::Disconnected) => {
                self.halted = true;
                Err(Error::Io(
                    "the Ren'Py script thread ended unexpectedly".into(),
                ))
            }
        }
    }

    fn diagnostics(&self) -> Vec<u8> {
        json!({
            "version": self.config.version,
            "title": self.title,
            "running": self.session.is_some(),
            "text_rect": self.text_rect,
            "log": self.diagnostics.iter().rev().take(64).collect::<Vec<_>>(),
        })
        .to_string()
        .into_bytes()
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.to_script.try_send(ToScript::Shutdown);
            drop(s.to_script);
            while s
                .from_script
                .recv_timeout(Duration::from_millis(200))
                .is_ok()
            {}
            if let Some(t) = s.thread.take() {
                let _ = t.join();
            }
        }
    }
}
