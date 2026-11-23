// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! `krkr` VmAdapter: runs original TJS2 scripts through `crysirror-tjs2`
//! and translates KiriKiri native objects onto engine Layer/Canvas services.
use crate::{
    host::KrkrHost,
    natives::{self, Rt, has_member},
    storage::{LauncherOverlay, Storage},
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
    /// Ordered, session-local overrides from a selected KiriKiri launcher.
    #[serde(default)]
    pub launcher_overlay: Option<LauncherOverlay>,
    /// Engine-published protected-bootstrap profile (`hxv4_bootstrap_profile`
    /// for `Plugins.linkZ`, or `cxdec_bootstrap_profile` for the startup
    /// `Storages.setupArchiveData` facade) the natives must match.
    #[serde(default)]
    pub bootstrap_profile: Option<String>,
    /// Native command-line values exposed by System.getArgument. Explicit
    /// player values override executable-embedded defaults before startup.
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
    /// User locale reported by getLangName.dll (`ja-JP`, `en-US`). Absent,
    /// the process locale environment is used, then `en-US`.
    #[serde(default)]
    pub locale: Option<String>,
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
        ensure!(self.arguments.len() <= 64, "KiriKiri argument count");
        let mut argument_bytes = 0usize;
        for (name, value) in &self.arguments {
            ensure!(name.starts_with('-') && (2..=128).contains(&name.len())
                && name.chars().all(|c| !c.is_whitespace() && !c.is_control()),
                "KiriKiri argument must be a canonical -name");
            ensure!(value.len() <= 4096 && !value.contains('\0'), "KiriKiri argument value");
            argument_bytes += name.len() + value.len();
        }
        ensure!(argument_bytes <= 64 * 1024, "KiriKiri argument byte budget");
        if let Some(exe) = &self.executable {
            ensure!(
                !exe.is_empty() && exe.len() <= 255 && !exe.contains(['/', '\\', ':', '\0']) && exe != "." && exe != "..",
                "KiriKiri executable must be a basename"
            );
        }
        if let Some(overlay) = &self.launcher_overlay {
            overlay.validate()?;
        }
        if let Some(locale) = &self.locale {
            ensure!(
                (2..=35).contains(&locale.len()) && locale.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@')),
                "KiriKiri locale must be a BCP-47 or POSIX language tag"
            );
        }
        for name in &self.plugin_shells {
            let name = crate::plugins::canonical(name).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                !crate::plugins::is_facade(&name),
                "engine facade cannot be replaced by a plugin shell"
            );
        }
        Ok(())
    }
}

#[path = "input_delivery.rs"]
mod input_delivery;
#[path = "modal_window.rs"]
pub(crate) mod modal_window;

struct FramePass {
    fence: u64,
    phase: u8,
    handlers: Vec<Variant>,
    handler: usize,
    now: u64,
    paints: Option<Vec<usize>>,
    paint: usize,
}

pub struct KrkrVm {
    client: ClientId,
    config: Config,
    rt: Option<Rt>,
    inbox: crysirror_core::input_buffer::InputBuffer,
    pass: Option<FramePass>,
    input_delivery: Option<input_delivery::Delivery>,
    modal_passes: Vec<modal_window::ParkedPass>,
    last_down: u64,
    last_pointer: [i32; 2],
    last_focus: Option<bool>,
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
    /// pending frame handlers are extra roots; native hover/capture references
    /// are reported by KrkrHost.
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
        if let Some(delivery) = &self.input_delivery { delivery.roots(&mut extra); }
        for scope in &self.modal_passes {scope.roots(&mut extra);}
        if let Some(pass) = &self.pass {
            if let Some(paints) = &pass.paints {
                extra.extend(paints.iter().skip(pass.paint).map(|id| Variant::Object(ObjectHandle(*id))));
            }
        }
        let started = web_time::Instant::now();
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
            input_delivery: None,
            modal_passes: Vec::new(),
            last_down: 0,
            last_pointer: [-1, -1],
            last_focus: None,
            gc_threshold: 50_000,
            frames: 0,
            last_error: None,
            halted: false,
            events: BTreeMap::new(),
        })
    }

    fn boot(&mut self, h: &mut dyn HostServices, input: InputFrame) -> Result<()> {
        let files = h.storage(self.client).map_err(err)?;
        let mut storage = Storage::new(files, &self.config.root)?;
        storage.media = Some(h.mounted_media(self.client).map_err(err)?);
        let mut host = KrkrHost::new(self.config.canvas, self.config.font_alias.clone());
        host.client = self.client.0;
        for codec in ["opus","vorbis","ms-adpcm"] {
            if h.audio_codec_supported(self.client,codec).map_err(err)? { host.audio_codecs.insert(codec.into()); }
        }
        host.input = input;
        host.held_buttons = input.buttons_down;
        host.pressed_since_query = input.buttons_down | input.buttons_pressed;
        host.locale = self.config.locale.clone();
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
        host.fonts.library=Some(h.font_library(self.client).map_err(err)?);
        host.now = h.now_ms();
        host.display = h.display_info();
        let mut rt = Runtime::with_host(host);
        natives::install(&mut rt);
        crate::protect_plugin::install_startup(&mut rt);
        // krkr2 reads release options embedded in the executable before any
        // script runs (`System.getArgument("-debugwin")` is "no" in shipped
        // games); they seed the argument table, never overriding a value.
        if let Some(exe) = rt.host().executable.clone()
            && let Some(path) = rt.host().storage.as_ref().and_then(|s| s.resolve(&exe))
        {
            let bytes = rt.host().storage.as_ref().unwrap().read_limited(&path, 64 << 20)?;
            if let Some(bits) = crate::exe_options::pe_bits(&bytes) {
                rt.host_mut().exe_bits = bits;
            }
            let mut options = crate::exe_options::parse(&bytes)
                .with_context(|| format!("{exe}: embedded options"))?;
            // krkrz option stock priority: command line, then the per-user
            // `<datapath>/<exe>.cfu` (written by KAGEX changeUserConf and the
            // kagexopt option set), then `<exe>.cf` beside the executable,
            // then the embedded area. Later entries here override earlier.
            let stem = exe.rsplit_once('.').map_or(exe.as_str(), |(stem, _)| stem).to_owned();
            let leaf = stem.rsplit('/').next().unwrap_or(&stem).to_owned();
            for (name, label) in [(format!("{stem}.cf"), "configuration"), (format!("file://./savedata/{leaf}.cfu"), "user configuration")] {
                let storage = rt.host().storage.as_ref().unwrap();
                let Some(path) = storage.resolve(&name) else { continue };
                let bytes = storage.read_limited(&path, 1 << 20)?;
                options.extend(crate::exe_options::parse_config(&bytes).with_context(|| format!("{name}: {label} options"))?);
            }
            if !options.is_empty() {
                let table = rt.alloc_dictionary_object();
                for (name, value) in options {
                    rt.set_object_member(table, &format!("-{name}"), Variant::String(value.into()));
                }
                rt.set_global_member("__crysirror_arguments", Variant::Object(table));
            }
        }
        if !self.config.arguments.is_empty() {
            let table = match rt.global_member("__crysirror_arguments").object_handle() {
                Some(table) => table,
                None => rt.alloc_dictionary_object(),
            };
            for (name, value) in &self.config.arguments {
                rt.set_object_member(table, name, Variant::String(value.clone()));
            }
            rt.set_global_member("__crysirror_arguments", Variant::Object(table));
        }
        let storage = rt.host_mut().storage.as_mut().context("KiriKiri storage unavailable")?;
        if let Some(overlay) = &self.config.launcher_overlay {
            storage.install_launcher_overlay(overlay)?;
        }
        ensure!(
            storage.resolve(&self.config.startup).is_some(),
            "KiriKiri startup script `{}` absent",
            self.config.startup
        );
        crate::plugins::autoload(&mut rt)?;
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
        // Source-line tracing reads raw VM fields and registers. It never
        // evaluates game expressions or invokes property getters.
        if let Ok(points) = std::env::var("CRYSIRROR_KRKR_TRACE_TJS") {
            anyhow::ensure!(std::env::var_os("CRYSIRROR_KRKR_EXCEPTION_EVAL").is_none(),
                "TRACE_TJS and EXCEPTION_EVAL require separate diagnostic runs");
            struct TraceUi { fields: Vec<String>, remaining: usize }
            impl crysirror_tjs2::debug::DebugUi<KrkrHost> for TraceUi {
                fn on_pause(&mut self, p: &mut crysirror_tjs2::debug::Pause<'_, KrkrHost>) -> crysirror_tjs2::debug::DebugAction {
                    if self.remaining > 0 {
                        self.remaining -= 1;
                        eprintln!("[krkr-tjs] {:?}", p.location());
                        {
                            let host=p.runtime().host();
                            eprintln!("[krkr-tjs-host] now={} layers={} attached={}",host.now,
                                host.layers.len(),host.layers.values().filter(|l|l.attached).count());
                        }
                        let receiver = p.read_register(-1).and_then(|v|v.object_handle());
                        if let Some(receiver) = receiver {
                            let rt = p.runtime();
                            let receiver = rt.bound_this(receiver).unwrap_or(receiver);
                            for field in &self.fields {
                                let mut value = Variant::Object(receiver);
                                for name in field.split('.') {
                                    value = value.object_handle().map(|h|rt.object_member(h,name)).unwrap_or(Variant::Void);
                                }
                                let text = if let Some(h) = value.object_handle() {
                                    if let Some(values) = rt.array_elements(h) {
                                        format!("object#{} array_len={}",h.0,values.len())
                                    } else {
                                        let names=rt.object_members(h).into_iter().take(8).map(|(n,_)|n).collect::<Vec<_>>();
                                        format!("object#{} keys={names:?}",h.0)
                                    }
                                } else {value.to_string().chars().take(256).collect()};
                                eprintln!("[krkr-tjs] this#{receiver:?}.{field}={text}");
                            }
                        }
                        for (reg,value) in p.registers().into_iter().filter(|(r,_)|*r < -2).take(12) {
                            let value=match value {Variant::Object(h)=>format!("object#{}",h.0),Variant::Closure(c)=>format!("closure#{}",c.object.0),v=>v.to_string().chars().take(160).collect()};
                            eprintln!("[krkr-tjs] r{reg}={value}");
                        }
                    }
                    crysirror_tjs2::debug::DebugAction::Continue
                }
            }
            for point in points.split(';').filter(|p|!p.is_empty()) {
                let (file,line)=point.rsplit_once(':').context("TRACE_TJS expects file:line entries separated by semicolons")?;
                let line=line.parse::<usize>().context("TRACE_TJS source line is not an integer")?;
                anyhow::ensure!(!file.is_empty() && line>0,"TRACE_TJS requires a file and positive line");
                rt.enable_debugger().add_tjs_breakpoint(file,line);
            }
            let fields=std::env::var("CRYSIRROR_KRKR_TRACE_TJS_FIELDS").unwrap_or_default()
                .split(',').filter(|s|!s.is_empty()).map(str::to_owned).collect();
            rt.set_debug_ui(Box::new(TraceUi{fields,remaining:2048}));
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
        let _span = crysirror_tasks::profile::span_current("krkr.layers.commit");
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        natives::native_extra::sync_attention(rt).map_err(|e|anyhow::anyhow!("{e}"))?;
        crate::native_pad::sync(rt).map_err(|e|anyhow::anyhow!("{e}"))?;
        rt.host_mut().sync().map_err(|e| anyhow::anyhow!("{e}"))?;
        let batches = std::mem::take(&mut rt.host_mut().batches);
        let mut encoded = Vec::new();
        for batch in batches {
            encoded.clear();
            serde_json::to_writer(&mut encoded, &batch)?;
            h.submit_engine(self.client, 1, &encoded)
                .map_err(err)
                .with_context(|| format!("engine batch {}", truncate(&String::from_utf8_lossy(&encoded))))?;
        }
        let mut retired = std::mem::take(&mut rt.host_mut().retired_images);
        retired.extend(rt.host().motion.take_retired_textures());
        for path in retired {
            h.release_image_resource(self.client,&path).map_err(err)
                .context("retire native image publication after Canvas commit")?;
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
        let _span = crysirror_tasks::profile::span_current("krkr.script.call");
        rt.call_object_method(target, method, args)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("TJS event {method}"))
    }

    fn layer_key(&mut self, window: ObjectHandle, event: &'static str, key: i64, shift: i64) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        if !rt.object_valid(window) {
            return Ok(());
        }
        crate::native_members::primary_key(rt, window, event, key, shift)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("TJS layer event {event}"))
    }

    fn observe_focus(&mut self, host:&dyn HostServices)->Result<()> {
        let focus=host.window_focused(self.client).map_err(err)?;
        let previous=std::mem::replace(&mut self.last_focus,focus);
        let (Some(previous),Some(current))=(previous,focus) else {return Ok(());};
        if previous==current {return Ok(());}
        let event=if current {"onActivate"}else{"onDeactivate"};
        let rt=self.rt.as_mut().context("KiriKiri runtime absent")?;
        if !current {
            // WindowForm::OnDeactive posts ReleaseCapture before subsequent
            // pointer delivery. This does not itself synthesize MouseLeave.
            crate::native_mouse::release(rt);
            rt.host_mut().hint_hover=None;
            rt.host_mut().ui_op(json!({"op":"hide_tooltip"}));
        }
        let system=rt.global_member("System").object_handle().context("System object missing")?;
        let callback=rt.object_member(system,event);
        let old=rt.host().can_suspend;rt.host_mut().can_suspend=false;
        // TVPFireOnApplicationActivateEvent calls the stored closure without a
        // System receiver. Its original ObjThis (or unbound fallback) is kept.
        let result=if callback.object_handle().is_some() {rt.call_function(callback,Vec::new()).map(|_|())}else{Ok(())};
        rt.host_mut().can_suspend=old;
        result.map_err(|e|anyhow::anyhow!("TJS System focus event {event}: {e}"))?;
        let windows=rt.host().windows.iter().filter(|(_,window)|window.visible)
            .map(|(handle,_)|*handle).collect::<Vec<_>>();
        for window in windows {
            self.rt.as_mut().unwrap().host_mut().can_suspend=false;
            let result=self.call(ObjectHandle(window),event,Vec::new());
            self.rt.as_mut().unwrap().host_mut().can_suspend=old;
            result?;
        }
        Ok(())
    }

    /// An accepted disabled hit blocks the layers behind it.
    fn hit_candidate(&mut self, id: usize, lx: i64, ly: i64) -> Result<Option<bool>> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
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
                let alpha=if let Some(alpha)=layer.full_alpha {alpha}
                else if let Some(crysirror_core::image_processing::ImageInput::Resource(image))=&layer.image_source {
                    ensure!(image.client()==self.client,"foreign native hit image resource");
                    image.alpha_at([px as u32,py as u32]).context("native hit point outside shared image resource")?
                } else {
                    let source=match &layer.image_source {
                        Some(crysirror_core::image_processing::ImageInput::Asset{path,..})=>format!("asset {path}"),
                        Some(crysirror_core::image_processing::ImageInput::Encoded{name,..})=>format!("encoded {name}"),
                        Some(crysirror_core::image_processing::ImageInput::Solid{..})=>"solid".into(),
                        Some(crysirror_core::image_processing::ImageInput::Composite{..})=>"composite".into(),
                        Some(crysirror_core::image_processing::ImageInput::Raster{..})=>"raster".into(),
                        Some(crysirror_core::image_processing::ImageInput::Ternary{..})=>"three-input image".into(),
                        Some(crysirror_core::image_processing::ImageInput::Transition{..})=>"transition".into(),
                        Some(crysirror_core::image_processing::ImageInput::Resource(_))=>"resource".into(),
                        Some(_)=>"shared image operation graph".into(),
                        None=>"composed canvas".into(),
                    };
                    bail!("alpha-mask hit for native layer {id} canvas {} at {px},{py} requires a prepared shared image or composed-canvas query; source={source}",layer.slot);
                };
                i64::from(alpha)>=threshold
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
                return Ok(Some(enabled));
            }
        }
        Ok(None)
    }

    fn captured_mouse_target(&mut self, x:i64, y:i64) -> Result<Option<(usize,i64,i64)>> {
        let capture=self.rt.as_ref().unwrap().host().mouse.capture;
        if let Some(id)=capture {
            let rt=self.rt.as_ref().unwrap();
            if rt.host().layers.get(&id).is_some_and(|l|l.attached) {
                let mut point=[x,y];
                if let Some(w)=rt.host().layers.get(&id).and_then(|l|rt.host().windows.get(&l.window)) {point[0]-=w.layer_pos[0];point[1]-=w.layer_pos[1];}
                let mut node=Some(id);let mut depth=0;
                while let Some(h)=node {
                    let layer=rt.host().layer(ObjectHandle(h))?;
                    point[0]-=layer.left;point[1]-=layer.top;node=layer.parent;
                    depth+=1;ensure!(depth<=64,"native captured coordinate ancestry");
                }
                return Ok(Some((id,point[0],point[1])));
            }
            crate::native_mouse::release(self.rt.as_mut().unwrap());
        }
        Ok(None)
    }

    fn enqueue_timers(&mut self, now: u64) -> Result<()> {
        use crysirror_core::deferred_dispatch::DispatchPhase;
        let rt = self.rt.as_mut().unwrap();
        let mut due = vec![];
        let disabled = rt.host().event_disabled;
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
            // Timer events are discardable posts: dropped, not deferred,
            // while System.eventDisabled is set.
            if disabled {
                continue;
            }
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
            let rt = self.rt.as_mut().unwrap();
            if let Some(path) = rt.host().pending_image_metadata.as_ref() {
                match rt.host().images.as_ref().context("image metadata unavailable")?.prepare_metadata(path) {
                    Ok(false) => return Ok(false),
                    Ok(true) => {}
                    Err(error) => {
                        // Let the retried native instruction raise an ordinary,
                        // catchable TJS exception at the original call site.
                        let failure = (path.clone(), error.to_string());
                        rt.host_mut().image_metadata_failure = Some(failure);
                    }
                }
                rt.host_mut().pending_image_metadata = None;
            }
            if self.rt.as_ref().unwrap().host().bitmap_work.as_ref().is_some_and(|s| s.parked) {
                return Ok(false);
            }
            self.flush(h)?;
            if self.modal_boundary()? {return Ok(false);}
            {
                let rt=self.rt.as_mut().unwrap();
                if let Some(tick)=rt.host().clock_yield_tick {
                    let now=rt.host().now;
                    if now==tick {return Ok(false);}
                    rt.host_mut().clock_yield_tick=None;
                    rt.host_mut().clock_poll_reads=0;
                    rt.set_resume_value(Variant::Integer(now as i64));
                }
            }
            crate::transitions::poll(self.rt.as_mut().unwrap(), h, self.client)?;
            if !crate::motion_plugin::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::font_resources::poll(self.rt.as_mut().unwrap())? {return Ok(false);}
            if !crate::province::poll(self.rt.as_mut().unwrap())? {return Ok(false);}
            if !crate::fftgraph::poll(self.rt.as_mut().unwrap(),h,self.client)? {return Ok(false);}
            natives::native_extra::poll(self.rt.as_mut().unwrap(),h,self.client)?;
            crate::get_sample::poll(self.rt.as_mut().unwrap(),h,self.client)?;
            if !crate::wv_decoder::poll(self.rt.as_mut().unwrap(),h,self.client)? {return Ok(false);}
            crate::http_plugin::poll(self.rt.as_mut().unwrap(),h,self.client)?;
            crate::native_pad::poll(self.rt.as_mut().unwrap(),h,self.client)?;
            if !crate::image_api::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::psbfile_plugin::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::message_dialog::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::edit_dialog::poll(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            if !crate::save_struct::poll(self.rt.as_mut().unwrap())? {
                return Ok(false);
            }
            if !crate::sound::drive(self.rt.as_mut().unwrap(), h, self.client)? {
                return Ok(false);
            }
            crate::video::drive(self.rt.as_mut().unwrap(), h, self.client)?;
            crate::alpha_movie::drive(self.rt.as_mut().unwrap(), h, self.client)?;
            if !crate::layer_movie::drive(self.rt.as_mut().unwrap(),h,self.client)? {return Ok(false);}
            let rt = self.rt.as_mut().unwrap();
            if rt.is_suspended() {
                let _span = crysirror_tasks::profile::span_current("krkr.script.resume");
                rt.resume_suspended().map_err(|e| anyhow::anyhow!("{e}"))?;
            } else {
                // A program close may have suspended in onCloseQuery's save
                // or confirmation. Decide only after its entire caller stack
                // resumes, never while a modal event scope is running.
                if rt.host().modal_windows.is_empty()
                    && std::mem::take(&mut rt.host_mut().program_close_pending)
                    && rt.host().close_decision
                {
                    rt.host_mut().exit_requested = true;
                }
                rt.host_mut().bitmap_redraw_pending = false;
                self.flush(h)?;
                return Ok(true);
            }
        }
        // Many continuations completed this frame (e.g. a startup that loads
        // hundreds of images, each suspending on its worker); the VM is still
        // suspended at a valid resume point, so continue next frame.
        Ok(false)
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
        let booting = self.rt.is_none();
        if booting {
            self.boot(h, input)?;
        }
        if let Some(rt) = self.rt.as_mut() {
            crate::wrapper_plugins::sync_proxy(rt).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        self.rt.as_mut().unwrap().host_mut().now = now;
        self.rt.as_mut().unwrap().host_mut().display = h.display_info();
        let input=self.modal_input(h,input)?;
        let mut input=crate::native_pad::filter_input(self.rt.as_ref().unwrap(),input);
        let host = self.rt.as_mut().unwrap().host_mut();
        host.bitmap_work = Some(crate::host::BitmapWorkSlice::default());
        // Startup scripts already observed the first input before boot returned.
        host.prompt_release_mask &= input.buttons_down & !input.buttons_pressed;
        if host.input_prompt.is_none() {
            input.buttons_down &= !host.prompt_release_mask;
            input.buttons_pressed &= !host.prompt_release_mask;
            host.pressed_since_query |= input.buttons_down & !host.held_buttons;
            if !booting { host.pressed_since_query |= input.buttons_pressed; }
            host.held_buttons = input.buttons_down;
            self.inbox.push(self.client.0,now,input)?;
        } else {
            host.prompt_release_mask |= input.buttons_down;
            // The dialog can finish during this same tick. deliver_input
            // also uses this sample directly when the game inbox is empty.
            input.buttons_down = 0;
            input.buttons_pressed = 0;
            host.held_buttons = 0;
            host.pressed_since_query = 0;
        }
        if !self.drive_sound(h)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if self.pass.is_none() {
            self.observe_focus(h)?;
            crate::video::observe(self.rt.as_mut().unwrap(), h, self.client)?;
            crate::alpha_movie::observe(self.rt.as_mut().unwrap(), h, self.client)?;
            crate::layer_movie::observe(self.rt.as_mut().unwrap(),h,self.client)?;
            while let Some((target, method, args)) = self.rt.as_mut().unwrap().host_mut().http.events.pop_front() {
                self.call(target, method, args)?;
            }
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
                let modal=rt.host().modal_windows.last().map(|m|m.window);
                if let Some(modal)=modal {
                    modal_window::close(rt,ObjectHandle(modal))?;
                } else { match rt.host().windows.iter().find(|(_,window)|window.output).map(|(handle,_)|*handle) {
                    Some(main) => {
                        rt.host_mut().closing_by_user = true;
                        self.call(ObjectHandle(main), "onCloseQuery", vec![Variant::Integer(1)])?;
                    }
                    None => rt.host_mut().exit_requested = true,
                }}
            }
            let menus = h
                .query_engine(
                    self.client,
                    crysirror_core::service::EngineQuery::MenuEvents,
                )
                .map_err(err)?;
            let rt = self.rt.as_mut().unwrap();
            rt.host_mut().can_suspend = true;
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
                paints: None,
                paint: 0,
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
                    if kind==3 {
                        if let Some(callback)=crate::layer_movie::take_event(self.rt.as_mut().unwrap(),id) {
                            *self.events.entry("onUpdateMovie").or_default()+=1;
                            self.rt.as_mut().unwrap().call_function(callback,vec![]).map_err(|e|anyhow::anyhow!("layerExMovie callback: {e}"))?;
                            continue;
                        }
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
                if self.input_delivery.is_none(){
                    if let Some((pad,notify))=self.rt.as_mut().unwrap().host_mut().pads.closes.pop_front(){
                        self.input_delivery=Some(input_delivery::Delivery::pad_close(input,pad,notify));
                    }else if let Some((window,method,key,shift))=self.rt.as_mut().unwrap().host_mut().extra.posted.pop_front(){
                        self.input_delivery=Some(input_delivery::Delivery::posted(input,window,method,key,shift));
                    }
                }
                let (frame,receipt)=if self.input_delivery.is_none() {
                    let batch=self.inbox.read(self.client.0,1)?;
                    (batch.samples.first().map_or(input,|s|s.frame),batch.receipt)
                } else {(input,None)};
                let posted=self.input_delivery.as_ref().is_some_and(|d|d.is_posted());
                let Some(receipt)=self.deliver_input(h,frame,receipt)? else {
                    return Ok(VmYield::BudgetExhausted);
                };
                if let Some(receipt)=receipt {self.inbox.acknowledge(self.client.0,receipt)?;}
                if posted{continue;}
                self.pass.as_mut().unwrap().phase=2;
                continue;
            }
            if phase == 3 {
                let p = self.pass.as_mut().unwrap();
                // Read the live slot: an earlier timer, input callback or
                // continuous handler can remove a later mover this same pass.
                // Native EventIntf keeps tombstones until traversal completes;
                // registrations appended during delivery remain observable.
                if let Some(f) = natives::continuous_handler_at(self.rt.as_ref().unwrap(), p.handler) {
                    p.handler += 1;
                    let time = p.now;
                    if matches!(f, Variant::Void) { continue; }
                    self.rt
                        .as_mut()
                        .unwrap()
                        .call_function(f, vec![Variant::Integer(time as i64)])
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    continue;
                }
                natives::compact_continuous_handlers(self.rt.as_mut().unwrap());
                p.phase = 4;
                continue;
            }
            // Layer.callOnPaint: before the frame is drawn, each flagged layer
            // gets one onPaint() call and its flag is cleared (krkrz order).
            if self.pass.as_ref().unwrap().paints.is_none() {
                let rt = self.rt.as_ref().unwrap();
                let flagged = rt.host()
                    .layers
                    .keys()
                    .copied()
                    .filter(|id| rt.object_member(ObjectHandle(*id), "__lp_callOnPaint").is_truthy())
                    .collect();
                self.pass.as_mut().unwrap().paints = Some(flagged);
            }
            let paint = {
                let pass = self.pass.as_mut().unwrap();
                let id = pass.paints.as_ref().unwrap().get(pass.paint).copied();
                if id.is_some() { pass.paint += 1; }
                id
            };
            if let Some(id) = paint {
                let rt = self.rt.as_mut().unwrap();
                if !rt.object_valid(ObjectHandle(id)) || !rt.host().layers.contains_key(&id) { continue; }
                rt.set_object_member(ObjectHandle(id), "__lp_callOnPaint", Variant::Integer(0));
                if has_member(rt, ObjectHandle(id), "onPaint") {
                    self.call(ObjectHandle(id), "onPaint", vec![])?;
                }
                // Complete this callback's resource dependencies before the
                // next paint callback can enter TJS or mutate its Canvases.
                continue;
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
    fn presentation_pending(&self) -> bool {
        self.rt.as_ref().is_some_and(|rt| {
            // Only a batched bitmap redraw retains the previous picture.
            // Ordinary resource waits and the rest of a VM frame do not block
            // startup/animation presentation. Native modals must remain visible.
            rt.host().dialog_pending.is_none() && rt.host().input_prompt.is_none() && rt.host().bitmap_redraw_pending
        })
    }
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
        let executed = {
            let _span = crysirror_tasks::profile::span_current("krkr.callbacks");
            self.execute(h, i)
        };
        match executed {
            Ok(y) => {
                {
                    let _span = crysirror_tasks::profile::span_current("krkr.collect_garbage");
                    self.collect_garbage();
                }
                // Native resource retirement is committed before this frame's
                // renderer snapshot or a freed slot can be reused next step.
                if self.rt.as_ref().is_some_and(|rt|!rt.host().batches.is_empty() || !rt.host().retired_images.is_empty()) {
                    self.flush(h).map_err(|e|Error::Io(format!("native retirement: {e:#}")))?;
                }
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
            "vm": "crysirror-tjs2",
            "parser_positions":host.map(|h|h.parsers.iter().map(|(id,p)|json!({"object":id,"storage":p.parser.cur_storage(),"label":p.parser.cur_label(),"line":p.parser.cur_line(),"call_depth":p.parser.call_stack_depth()})).collect::<Vec<_>>()),
            "script_state": std::env::var_os("CRYSIRROR_KRKR_DUMP_STATE").and_then(|_|self.rt.as_ref().map(|r|{
                let mut out=serde_json::Map::new();
                if let Some(k)=r.global_member("kag").object_handle(){
                    for (label,object) in [("kag",Some(k)),("conductor",r.object_member(k,"conductor").object_handle()),("current",r.object_member(k,"current").object_handle()),("fore",r.object_member(k,"fore").object_handle()),("back",r.object_member(k,"back").object_handle()),("world",r.global_member("world_object").object_handle()),("system_hook",r.global_member("SystemHook").object_handle())] {
                        if let Some(object)=object {
                            let members=r.object_members(object);
                            let mut fields=members.iter().take(256).map(|(name,value)|{
                                let text=match value {Variant::String(s)=>s.chars().take(256).collect(),Variant::Object(h)=>format!("object#{}",h.0),Variant::Closure(c)=>format!("closure#{}",c.object.0),v=>v.to_string()};
                                (name.clone(),serde_json::Value::String(text))
                            }).collect::<serde_json::Map<_,_>>();
                            // Prototype methods must not crowd out scalar state.
                            for (name,value) in members.into_iter().filter(|(_,v)|matches!(v,Variant::Void|Variant::Null|Variant::Integer(_)|Variant::Real(_)|Variant::String(_))).take(512) {
                                let text=match value {Variant::String(s)=>s.chars().take(256).collect(),v=>v.to_string()};
                                fields.insert(name,serde_json::Value::String(text));
                            }
                            out.insert(label.into(),fields.into());
                        }
                    }
                }
                serde_json::Value::Object(out)
            })),
            "kag":kag,
            "frames": self.frames,
            "queued_input":self.inbox.len(),
            "frame_pass":self.pass.as_ref().map(|p|p.phase),
            "suspended":self.rt.as_ref().is_some_and(|r|r.is_suspended()),
            "modal_scopes":self.modal_passes.len(),
            "modal_windows_opened":host.map(|h|h.modal_serial),
            "input_delivery_pending":self.input_delivery.is_some(),
            "clock_waiting":host.and_then(|h|h.clock_yield_tick),
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
            "log_tail": host.map(|h| h.log.iter().rev().take(std::env::var("CRYSIRROR_KRKR_LOG_TAIL").ok().and_then(|n| n.parse().ok()).unwrap_or(16)).cloned().collect::<Vec<_>>()),
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

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod presentation_tests;
