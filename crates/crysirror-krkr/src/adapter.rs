// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `krkr` VmAdapter: runs original TJS2 scripts on the vendored Kirakira VM
//! and translates KiriKiri native objects onto engine Layer/Canvas services.
use crate::{
    host::{Batch, KrkrHost},
    natives::{self, Rt, has_member},
    storage::Storage,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_tjs2::runtime::{ObjectHandle, Runtime, Variant};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Game directory inside the client's asset root ("" is the root).
    #[serde(default)]
    pub root: String,
    /// First script executed, KiriKiri's `startup.tjs`.
    #[serde(default = "startup")]
    pub startup: String,
    /// Project font alias used for every KiriKiri font face.
    #[serde(default = "font")]
    pub font_alias: String,
    /// Logical canvas size; must equal the project canvas.
    pub canvas: [u32; 2],
    /// Explicit registration-only native modules, with no callable exports.
    /// Loading a shell does not establish implementation of that plugin.
    #[serde(default)]
    pub plugin_shells: Vec<String>,
    /// Companion executable basename reported by `System.exeName`.
    #[serde(default)]
    pub executable: Option<String>,
    /// Engine-published protected-bootstrap profile (`hxv4_bootstrap_profile`
    /// for `Plugins.linkZ`, or `cxdec_bootstrap_profile` for the startup
    /// `Storages.setupArchiveData` facade) the natives must match.
    #[serde(default)]
    pub bootstrap_profile: Option<String>,
}
fn startup() -> String {
    "startup.tjs".into()
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
                "canonical KiriKiri root required"
            );
        }
        ensure!(
            !self.startup.is_empty() && !self.font_alias.is_empty(),
            "startup and font alias required"
        );
        ensure!(
            self.canvas.iter().all(|v| (1..=8192).contains(v)),
            "KiriKiri canvas size"
        );
        ensure!(
            self.plugin_shells.len() <= 128,
            "KiriKiri plugin shell limit"
        );
        if let Some(exe) = &self.executable {
            ensure!(
                !exe.is_empty() && exe.len() <= 255 && !exe.contains(['/', '\\', ':', '\0']) && exe != "." && exe != "..",
                "KiriKiri executable must be a basename"
            );
        }
        for name in &self.plugin_shells {
            let name = crate::plugins::canonical(name).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                !matches!(name.as_str(), "savestruct.dll" | "fstat.dll" | "menu.dll" | "kagparser.dll" | "layerexraster.dll" | "json.dll" | "layerexdraw.dll" | "krmovie.dll" | "layereximage.dll" | "kagparserex.dll" | "textrender.dll" | "motionplayer.dll" | "motionplayer_nod3d.dll" | "csvparser.dll" | "scriptsex.dll" | "addfont.dll" | "windowex.dll" | "win32dialog.dll" | "extrans.dll" | "extnagano.dll"),
                "engine facade cannot be replaced by a plugin shell"
            );
        }
        Ok(())
    }
}

struct FramePass {
    fence: u64,
    phase: u8,
    handlers: Vec<Variant>,
    handler: usize,
    now: u64,
}

pub struct KrkrVm {
    client: ClientId,
    config: Config,
    rt: Option<Rt>,
    inbox: crysirror_core::input_buffer::InputBuffer,
    pass: Option<FramePass>,
    last_down: u64,
    last_pointer: [i32; 2],
    pressed: Option<usize>,
    /// Live TJS object count that triggers the next collection.
    gc_threshold: usize,
    frames: u64,
    last_error: Option<String>,
    halted: bool,
    events: BTreeMap<&'static str, u64>,
}

impl KrkrVm {
    /// Reclaim unreachable TJS objects between steps (outside execution).
    /// Collects when live objects doubled since the last collection; the
    /// adapter's own references (pending frame handlers, pressed layer) are
    /// extra roots.
    fn collect_garbage(&mut self) {
        let Some(rt) = self.rt.as_mut() else { return };
        // Diagnostic: CRYSIRROR_KRKR_GC=0 disables reclamation.
        if std::env::var_os("CRYSIRROR_KRKR_GC").is_some_and(|v| v == "0") {
            return;
        }
        let live = rt.live_object_count();
        if live < self.gc_threshold {
            return;
        }
        let mut extra: Vec<Variant> = self.pass.as_ref().map(|p| p.handlers.clone()).unwrap_or_default();
        extra.extend(self.pressed.map(|p| Variant::Object(ObjectHandle(p))));
        let started = std::time::Instant::now();
        let freed = rt.collect_garbage(&extra);
        let live = rt.live_object_count();
        self.gc_threshold = live + (live / 2).max(50_000);
        if std::env::var_os("CRYSIRROR_KRKR_TRACE_GC").is_some() {
            let line = format!("[gc] freed {freed}, live {live}, {:?}", started.elapsed());
            rt.host_mut().push_log(line);
        }
    }

    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            rt: None,
            inbox: crysirror_core::input_buffer::InputBuffer::new(client.0, 1, 256)?,
            pass: None,
            last_down: 0,
            last_pointer: [-1, -1],
            pressed: None,
            gc_threshold: 50_000,
            frames: 0,
            last_error: None,
            halted: false,
            events: BTreeMap::new(),
        })
    }

    fn boot(&mut self, h: &mut dyn HostServices, input: InputFrame) -> Result<()> {
        let files = h.storage(self.client).map_err(err)?;
        let storage = Storage::new(files, &self.config.root)?;
        ensure!(
            storage.resolve(&self.config.startup).is_some(),
            "KiriKiri startup script `{}` absent",
            self.config.startup
        );
        let mut host = KrkrHost::new(self.config.canvas, self.config.font_alias.clone());
        host.client = self.client.0;
        host.input = input;
        host.plugin_shells = self
            .config
            .plugin_shells
            .iter()
            .map(|s| crate::plugins::canonical(s).expect("validated plugin name"))
            .collect();
        if let Some(profile) = &self.config.bootstrap_profile {
            let path = storage
                .resolve(profile)
                .with_context(|| format!("bootstrap profile `{profile}` absent"))?;
            let json = storage.read_limited(&path, 64 * 1024)?;
            crate::protect_plugin::configure(&mut host.protect, &json)?;
        }
        host.executable = self
            .config
            .executable
            .clone()
            .or_else(|| host.protect.executable());
        host.storage = Some(storage);
        host.images = Some(h.image_metadata(self.client).map_err(err)?);
        host.text_measurement = Some(h.text_measurement(self.client).map_err(err)?);
        host.now = h.now_ms();
        let mut rt = Runtime::with_host(host);
        natives::install(&mut rt);
        crate::protect_plugin::install_startup(&mut rt);
        // Developer diagnostic: on the first runtime exception, log the paused
        // frame's registers and evaluate `;;`-separated expressions in it.
        if let Ok(exprs) = std::env::var("CRYSIRROR_KRKR_EXCEPTION_EVAL") {
            struct Ui(Vec<String>, bool);
            impl crysirror_tjs2::debug::DebugUi<KrkrHost> for Ui {
                fn on_pause(&mut self, p: &mut crysirror_tjs2::debug::Pause<'_, KrkrHost>) -> crysirror_tjs2::debug::DebugAction {
                    let reason = format!("{:?}", p.reason());
                    if std::env::var_os("CRYSIRROR_KRKR_EXCEPTION_ALL").is_some() {
                        let line = format!("[debug] exception {} at {:?}", reason.chars().take(400).collect::<String>(), p.location().map(|f| (f.object_name.clone(), f.bytecode_offset)));
                        p.runtime().host_mut().push_log(line);
                    }
                    let wanted = std::env::var("CRYSIRROR_KRKR_EXCEPTION_MATCH").map_or(true, |m| reason.contains(&m));
                    if !self.1 && wanted {
                        self.1 = true;
                        let mut lines = vec![format!("[debug] pause {reason} at {:?}", p.location())];
                        for e in self.0.clone() {
                            let v = p.eval(&e).map(|v| format!("{v:?}")).unwrap_or_else(|e| format!("error {e}"));
                            lines.push(format!("[debug] {e} => {}", v.chars().take(1200).collect::<String>()));
                        }
                        for (r, v) in p.registers().into_iter().take(if std::env::var_os("CRYSIRROR_KRKR_EXCEPTION_REGS").is_some() { 48 } else { 0 }) {
                            lines.push(format!("[debug] r{r} = {}", format!("{v:?}").chars().take(160).collect::<String>()));
                            let h = match &v {
                                crysirror_tjs2::runtime::Variant::Object(h) => Some(*h),
                                crysirror_tjs2::runtime::Variant::Closure(c) => Some(c.this_obj.unwrap_or(c.object)),
                                _ => None,
                            };
                            if let Some(h) = h {
                                let rt = p.runtime();
                                let names: Vec<String> = rt.object_members(h).into_iter().map(|(n, _)| n).take(40).collect();
                                lines.push(format!("[debug]   members {}", names.join(",")));
                                if let Some(n) = rt.object_member(h, "names").object_handle() {
                                    let keys: Vec<String> = rt.object_members(n).into_iter().map(|(n, _)| n).collect();
                                    lines.push(format!("[debug]   names {}", keys.join(",")));
                                }
                            }
                        }
                        for l in lines {
                            p.runtime().host_mut().push_log(l);
                        }
                    }
                    crysirror_tjs2::debug::DebugAction::Continue
                }
            }
            rt.enable_debugger().set_break_on_exception(true);
            rt.set_debug_ui(Box::new(Ui(exprs.split(";;").map(str::to_owned).collect(), false)));
        }
        // Developer diagnostic: comma-separated `Class.method` trace patterns.
        if let Ok(patterns) = std::env::var("CRYSIRROR_KRKR_TRACE_NATIVES") {
            rt.set_native_call_traces(patterns.split(','));
        }
        let startup = self.config.startup.clone();
        let result = natives::exec_storage(&mut rt, &startup);
        self.rt = Some(rt);
        result
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("execute {startup}"))?;
        self.flush(h)
    }

    /// Submit queued engine commands in the order TJS issued them.
    fn flush(&mut self, h: &mut dyn HostServices) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        rt.host_mut().sync().map_err(|e| anyhow::anyhow!("{e}"))?;
        let batches = std::mem::take(&mut rt.host_mut().batches);
        for batch in batches {
            let payload = match batch {
                Batch::Layers(ops) => json!({"layers": ops}),
                Batch::Canvases(ops) => json!({"canvases": ops}),
                Batch::Ui(ops) => json!({"ui":ops}),
            };
            h.submit_engine(self.client, 1, &serde_json::to_vec(&payload)?)
                .map_err(err)
                .with_context(|| format!("engine batch {}", truncate(&payload.to_string())))?;
        }
        Ok(())
    }

    fn call(
        &mut self,
        target: ObjectHandle,
        method: &'static str,
        args: Vec<Variant>,
    ) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        if !rt.object_valid(target) || !has_member(rt, target, method) {
            return Ok(());
        }
        *self.events.entry(method).or_default() += 1;
        rt.call_object_method(target, method, args)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("TJS event {method}"))
    }

    fn input(&mut self, input: InputFrame) -> Result<()> {
        self.rt.as_mut().unwrap().host_mut().input = input;
        self.rt.as_mut().unwrap().host_mut().input.buttons_down |= input.buttons_pressed;
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        let window = rt
            .host()
            .windows
            .iter()
            .find(|(_, w)| w.visible)
            .map(|(h, _)| ObjectHandle(*h));
        let Some(window) = window else {
            self.rt.as_mut().unwrap().host_mut().input = input;
            self.last_down = input.buttons_down;
            return Ok(());
        };
        let (x, y) = (i64::from(input.pointer_x), i64::from(input.pointer_y));
        let shift = |bits: u64| -> i64 {
            i64::from(bits & buttons::SHIFT != 0)
                | (i64::from(bits & buttons::CTRL != 0) << 2)
                | (i64::from(bits & buttons::MOUSE_LEFT != 0) << 3)
                | (i64::from(bits & buttons::MOUSE_RIGHT != 0) << 4)
        };
        let flags = shift(input.buttons_down | input.buttons_pressed);
        let mouse_mask = buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT;
        let pointer_active = [input.pointer_x, input.pointer_y] != self.last_pointer
            || ((input.buttons_down ^ self.last_down) | input.buttons_pressed) & mouse_mask != 0;
        let candidates = if pointer_active {
            rt.host().hits(x, y)
        } else {
            vec![]
        };
        let mut hit = None;
        for (id, lx, ly) in candidates {
            let target = ObjectHandle(id);
            // Outside the current modal layer counts as disabled (krkrz
            // GetMostFrontChildAt: `IsDisabledByMode` blocks the hit).
            let mut enabled = !rt.host().disabled_by_mode(id);
            let mut node = Some(id);
            let mut depth = 0;
            while let Some(id) = node {
                if !rt.object_member(ObjectHandle(id), "__lp_enabled").is_truthy() {
                    enabled = false;
                    break;
                }
                node = rt.host().layer(ObjectHandle(id))?.parent;
                depth += 1;
                ensure!(depth <= 64, "native enabled ancestry limit");
            }
            let layer = rt.host().layer(target)?;
            let kind = rt
                .object_member(target, "__lp_hitType")
                .to_integer()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let threshold = rt
                .object_member(target, "__lp_hitThreshold")
                .to_integer()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let yes = if kind == 1 {
                layer
                    .regions
                    .get(self.client.0, [lx - layer.image_left, ly - layer.image_top])?
                    != 0
            } else if kind == 0 {
                let px = lx - layer.image_left;
                let py = ly - layer.image_top;
                if layer.has_image
                    && (px < 0 || py < 0 || px >= layer.image_width || py >= layer.image_height)
                {
                    false
                } else if threshold <= 0 {
                    true
                } else if threshold >= 256 || !layer.has_image {
                    false
                } else if layer.kind
                    == 1
                {
                    true
                } else {
                    layer.full_alpha.context("alpha-mask hit testing requires a shared per-pixel canvas query for this nonuniform image")? as i64>=threshold
                }
            } else {
                bail!("unknown native hitType")
            };
            if yes {
                // Native onHitTest changes a work flag through super.onHitTest;
                // its TJS return value is not the native hit result.
                rt.set_object_member(target, "__hit_test_work", Variant::Integer(1));
                *self.events.entry("onHitTest").or_default() += 1;
                rt.call_object_method(
                    target,
                    "onHitTest",
                    vec![
                        Variant::Integer(lx),
                        Variant::Integer(ly),
                        Variant::Integer(1),
                    ],
                )
                .map_err(|e| anyhow::anyhow!("TJS onHitTest: {e}"))?;
                if rt.object_member(target, "__hit_test_work").is_truthy() {
                    // A hit disabled layer blocks layers behind it; it does
                    // not make the pointer fall through the disabled subtree.
                    if !enabled {
                        break;
                    }
                    hit = Some((id, lx, ly));
                    break;
                }
            }
        }
        if [input.pointer_x, input.pointer_y] != self.last_pointer {
            self.rt.as_mut().unwrap().set_object_member(
                window,
                "__temporary_cursor_hide",
                Variant::Integer(0),
            );
        }
        crate::pointer::apply(
            self.rt.as_mut().unwrap(),
            hit.map_or(window, |h| ObjectHandle(h.0)),
        );
        if [input.pointer_x, input.pointer_y] != self.last_pointer {
            self.last_pointer = [input.pointer_x, input.pointer_y];
            self.call(
                window,
                "onMouseMove",
                vec![
                    Variant::Integer(x),
                    Variant::Integer(y),
                    Variant::Integer(flags),
                ],
            )?;
            if let Some((layer, lx, ly)) = hit {
                self.call(
                    ObjectHandle(layer),
                    "onMouseMove",
                    vec![
                        Variant::Integer(lx),
                        Variant::Integer(ly),
                        Variant::Integer(flags),
                    ],
                )?;
            }
        }
        for (bit, button) in [(buttons::MOUSE_LEFT, 0), (buttons::MOUSE_RIGHT, 1)] {
            let down = input.buttons_down & bit != 0;
            let was = self.last_down & bit != 0;
            let pressed = (down && !was) || (input.buttons_pressed & bit != 0 && !was);
            let released = !down && (was || input.buttons_pressed & bit != 0);
            let args = |px, py| {
                vec![
                    Variant::Integer(px),
                    Variant::Integer(py),
                    Variant::Integer(button),
                    Variant::Integer(flags),
                ]
            };
            if pressed {
                self.call(window, "onMouseDown", args(x, y))?;
                if let Some((layer, lx, ly)) = hit {
                    self.call(ObjectHandle(layer), "onMouseDown", args(lx, ly))?;
                }
                if button == 0 {
                    self.pressed = hit.map(|h| h.0);
                }
            }
            if released {
                self.rt.as_mut().unwrap().host_mut().input.buttons_down &= !bit;
                self.call(window, "onMouseUp", args(x, y))?;
                if let Some((layer, lx, ly)) = hit {
                    self.call(ObjectHandle(layer), "onMouseUp", args(lx, ly))?;
                }
                if button == 0 {
                    self.call(
                        window,
                        "onClick",
                        vec![Variant::Integer(x), Variant::Integer(y)],
                    )?;
                    if let (Some(p), Some((layer, lx, ly))) = (self.pressed.take(), hit) {
                        if p == layer {
                            self.call(
                                ObjectHandle(layer),
                                "onClick",
                                vec![Variant::Integer(lx), Variant::Integer(ly)],
                            )?;
                        }
                    }
                }
            }
        }
        for &(bit, key) in crate::pointer::KEYS {
            if input.buttons_pressed & bit != 0 {
                self.call(
                    window,
                    "onKeyDown",
                    vec![Variant::Integer(key), Variant::Integer(flags)],
                )?;
            }
            if input.buttons_down & bit == 0 && (self.last_down | input.buttons_pressed) & bit != 0
            {
                self.rt.as_mut().unwrap().host_mut().input.buttons_down &= !bit;
                self.call(
                    window,
                    "onKeyUp",
                    vec![Variant::Integer(key), Variant::Integer(flags)],
                )?;
            }
        }
        self.rt.as_mut().unwrap().host_mut().input = input;
        self.last_down = input.buttons_down;
        Ok(())
    }

    fn enqueue_timers(&mut self, now: u64) -> Result<()> {
        use crysirror_core::deferred_dispatch::DispatchPhase;
        let rt = self.rt.as_mut().unwrap();
        let mut due = vec![];
        for (id, t) in &mut rt.host_mut().timers {
            if !t.enabled || t.interval == 0 || now < t.next {
                continue;
            }
            let missed = (now - t.next) / t.interval + 1;
            t.next = t
                .next
                .checked_add(
                    missed
                        .checked_mul(t.interval)
                        .context("timer deadline overflow")?,
                )
                .context("timer deadline overflow")?;
            for _ in 0..missed.min(t.capacity.max(1)) {
                ensure!(due.len() < 4096, "native timer callback budget");
                due.push((*id, t.mode));
            }
        }
        for (id, mode) in due {
            rt.host_mut().deferred.post(
                self.client.0,
                crate::async_trigger::target(id, 1),
                match mode {
                    1 => DispatchPhase::Exclusive,
                    2 => DispatchPhase::Idle,
                    _ => DispatchPhase::Normal,
                },
                false,
            )?;
        }
        Ok(())
    }
    fn drive_sound(&mut self, h: &mut dyn HostServices) -> Result<bool> {
        for _ in 0..128 {
            self.flush(h)?;
            crate::transitions::poll(self.rt.as_mut().unwrap(), h, self.client)?;
            if !crate::motion_plugin::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::image_api::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::psbfile_plugin::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::kirakira::win32_dialog::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::save_struct::poll(self.rt.as_mut().unwrap())? {
                return Ok(false);
            }
            if !crate::sound::drive(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            let rt = self.rt.as_mut().unwrap();
            if rt.is_suspended() {
                rt.resume_suspended().map_err(|e| anyhow::anyhow!("{e}"))?;
            } else {
                self.flush(h)?;
                return Ok(true);
            }
        }
        bail!("native audio continuation budget")
    }
    fn execute(&mut self, h: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        use crysirror_core::deferred_dispatch::DispatchPhase;
        if let Some(e) = &self.last_error {
            bail!("KiriKiri cannot resume after a failed step: {e}")
        }
        if self.halted {
            return Ok(VmYield::Halted);
        }
        let now = h.now_ms();
        self.inbox.push(self.client.0, now, input)?;
        if self.rt.is_none() {
            self.boot(h, input)?;
        }
        self.rt.as_mut().unwrap().host_mut().now = now;
        if !self.drive_sound(h)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if self.pass.is_none() {
            let callbacks = crate::sound::observations(self.rt.as_mut().unwrap(), h, self.client)?;
            for (id, event) in callbacks {
                let kind = if event == "onStatusChanged" { 2 } else { 3 };
                self.rt.as_mut().unwrap().host_mut().deferred.post(
                    self.client.0,
                    crate::async_trigger::target(id, kind),
                    DispatchPhase::Exclusive,
                    false,
                )?;
            }
            // A user close (window close button): krkrz posts onCloseQuery(true)
            // to the main window as an input event; `super.onCloseQuery(true)`
            // closes, `false` cancels (KAG asks first when askOnClose).
            // The native handler consumes `closing_by_user`, also after the
            // script suspended (a confirmation dialog) before answering.
            let rt = self.rt.as_mut().unwrap();
            if std::mem::take(&mut rt.host_mut().user_close_pending) {
                match rt.host().windows.keys().next().copied() {
                    Some(main) => {
                        rt.host_mut().closing_by_user = true;
                        self.call(ObjectHandle(main), "onCloseQuery", vec![Variant::Integer(1)])?;
                    }
                    None => rt.host_mut().exit_requested = true,
                }
            }
            let menus = h
                .query_engine(
                    self.client,
                    crysirror_core::service::EngineQuery::MenuEvents,
                )
                .map_err(err)?;
            let rt = self.rt.as_mut().unwrap();
            rt.host_mut().can_suspend = false;
            let menu = crate::menus::dispatch(rt, &menus);
            rt.host_mut().can_suspend = true;
            menu?;
            self.enqueue_timers(now)?;
            let rt = self.rt.as_mut().unwrap();
            let handlers = natives::continuous_handlers(rt);
            self.pass = Some(FramePass {
                fence: rt.host().deferred.fence(),
                phase: 0,
                handlers,
                handler: 0,
                now,
            });
        }
        for _ in 0..16384 {
            if !self.drive_sound(h)? {
                return Ok(VmYield::BudgetExhausted);
            }
            let p = self.pass.as_mut().unwrap();
            let phase = p.phase;
            let fence = p.fence;
            if matches!(phase, 0 | 2 | 4) {
                let tier = match phase {
                    0 => DispatchPhase::Exclusive,
                    2 => DispatchPhase::Normal,
                    _ => DispatchPhase::Idle,
                };
                if let Some(event) =
                    self.rt
                        .as_mut()
                        .unwrap()
                        .host_mut()
                        .deferred
                        .pop(self.client.0, fence, tier)
                {
                    let id = (event >> 2) as usize;
                    let kind = event & 3;
                    if kind == 1
                        && !self
                            .rt
                            .as_ref()
                            .unwrap()
                            .host()
                            .timers
                            .get(&id)
                            .is_some_and(|t| t.enabled && t.interval != 0)
                    {
                        continue;
                    }
                    let method = match kind {
                        0 => "onFire",
                        1 => "onTimer",
                        2 => "__crysirror_notify_status",
                        _ => "__crysirror_notify_fade",
                    };
                    self.call(ObjectHandle(id), method, vec![])?;
                    continue;
                }
                self.pass.as_mut().unwrap().phase += 1;
                continue;
            }
            if phase == 1 {
                self.pass.as_mut().unwrap().phase = 2;
                let batch = self.inbox.read(self.client.0, 1)?;
                let frame = batch.samples.first().map_or(input, |s| s.frame);
                self.rt.as_mut().unwrap().host_mut().can_suspend = false;
                let result = self.input(frame);
                self.rt.as_mut().unwrap().host_mut().can_suspend = true;
                result?;
                if let Some(receipt) = batch.receipt {
                    self.inbox.acknowledge(self.client.0, receipt)?;
                }
                continue;
            }
            if phase == 3 {
                let p = self.pass.as_mut().unwrap();
                if let Some(f) = p.handlers.get(p.handler).cloned() {
                    p.handler += 1;
                    let time = p.now;
                    self.rt
                        .as_mut()
                        .unwrap()
                        .call_function(f, vec![Variant::Integer(time as i64)])
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    continue;
                }
                p.phase = 4;
                continue;
            }
            // Layer.callOnPaint: before the frame is drawn, each flagged layer
            // gets one onPaint() call and its flag is cleared (krkrz order).
            let flagged: Vec<usize> = {
                let rt = self.rt.as_ref().unwrap();
                rt.host()
                    .layers
                    .keys()
                    .copied()
                    .filter(|id| rt.object_member(ObjectHandle(*id), "__lp_callOnPaint").is_truthy())
                    .collect()
            };
            for id in flagged {
                let rt = self.rt.as_mut().unwrap();
                rt.set_object_member(ObjectHandle(id), "__lp_callOnPaint", Variant::Integer(0));
                if has_member(rt, ObjectHandle(id), "onPaint") {
                    self.call(ObjectHandle(id), "onPaint", vec![])?;
                }
            }
            self.pass = None;
            self.frames += 1;
            if self.rt.as_ref().unwrap().host().exit_requested {
                self.halted = true;
                return Ok(VmYield::Halted);
            }
            return Ok(VmYield::BudgetExhausted);
        }
        bail!("native owner callback budget")
    }

    /// Script log lines (Debug.message, System.inform); for tests and tools.
    pub fn log(&self) -> Vec<String> {
        self.rt
            .as_ref()
            .map(|rt| rt.host().log.clone())
            .unwrap_or_default()
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(240).collect()
}

fn err(e: Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

impl VmAdapter for KrkrVm {
    fn engine_id(&self) -> &'static str {
        "krkr"
    }
    fn request_close(&mut self) -> bool {
        match self.rt.as_mut() {
            Some(rt) if !self.halted => {
                rt.host_mut().user_close_pending = true;
                true
            }
            _ => false,
        }
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
        match self.execute(h, i) {
            Ok(y) => {
                self.collect_garbage();
                Ok(y)
            }
            Err(e) => {
                // Include the VM's latest log lines: script-level errors are
                // usually reported there before the failing native call.
                let tail = self
                    .rt
                    .as_ref()
                    .map(|rt| {
                        let log = &rt.host().log;
                        log[log.len().saturating_sub(160)..]
                            .iter()
                            .map(|l| format!("\n  [vm log] {}", l.chars().take(1500).collect::<String>()))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                let message = format!("{e:#}{tail}");
                self.last_error = Some(message.clone());
                Err(Error::Io(message))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("KiriKiri VM checkpoint not implemented"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("KiriKiri VM checkpoint not implemented"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let host = self.rt.as_ref().map(|rt| rt.host());
        let kag=self.rt.as_ref().and_then(|r|r.global_member("kag").object_handle()).map(|k|{
            let r=self.rt.as_ref().unwrap();let conductor=r.object_member(k,"conductor").object_handle();
            json!({"click_waiting":r.object_member(k,"clickWaiting").is_truthy(),"sleeping":r.object_member(k,"inSleep").is_truthy(),"last_tag":conductor.map(|c|r.object_member(c,"lastTagName").to_tjs_string().unwrap_or_default())})
        });
        serde_json::to_vec(&json!({
            "vm": "kirakira-tjs2",
            "kag":kag,
            "frames": self.frames,
            "queued_input":self.inbox.len(),
            "frame_pass":self.pass.as_ref().map(|p|p.phase),
            "suspended":self.rt.as_ref().is_some_and(|r|r.is_suspended()),
            "layers": host.map(|h| h.layers.len()),
            "timers": host.map(|h| h.timers.len()),
            "plugins": host.map(|h| &h.loaded_plugins),
            // Developer diagnostic (CRYSIRROR_KRKR_DUMP_LAYERS): native layer tree.
            // Developer diagnostic: TJS heap composition.
            "heap_histogram": std::env::var_os("CRYSIRROR_KRKR_HEAP_HISTOGRAM").and_then(|_| {
                self.rt.as_ref().map(|rt| rt.heap_histogram())
            }),
            "layer_dump": std::env::var_os("CRYSIRROR_KRKR_DUMP_LAYERS").and_then(|_| {
                let rt = self.rt.as_ref()?;
                Some(rt.host().layers.iter().map(|(id, l)| {
                    let name = rt.object_member(ObjectHandle(*id), "name").to_tjs_string().unwrap_or_default();
                    json!({"slot": l.slot, "name": name, "parent": l.parent.and_then(|p| rt.host().layers.get(&p)).map(|p| p.slot),
                        "visible": l.visible, "opacity": l.opacity, "pos": [l.left, l.top], "size": [l.width, l.height],
                        "image": [l.image_width, l.image_height], "has_image": l.has_image, "kind": l.kind, "blank": l.blank, "full_alpha": l.full_alpha, "children": l.children.len()})
                }).collect::<Vec<_>>())
            }),
            "plugin_shells": host.map(|h| &h.plugin_shells),
            "engine_api_calls": host.map(|h| h.api_calls),
            "events": self.events,
            "log_tail": host.map(|h| h.log.iter().rev().take(16).cloned().collect::<Vec<_>>()),
            "error": self.last_error,
            "rendering": "engine retained Layer/Canvas",
            "owns_framebuffer": false,
        }))
        .unwrap_or_default()
    }
}

pub struct KrkrFactory;
impl crysirror_runtime::runtime::VmFactory for KrkrFactory {
    fn create(
        &self,
        c: ClientId,
        s: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            s.program.is_empty(),
            "KiriKiri executes original TJS2 scripts"
        );
        Ok(Box::new(KrkrVm::new(
            c,
            serde_json::from_value(s.config.clone())?,
        )?))
    }
}

pub fn register(r: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    r.register("krkr", Arc::new(KrkrFactory))
}
