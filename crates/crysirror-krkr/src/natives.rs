// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! KiriKiri native classes as thin translations onto engine services.
//!
//! Member names, defaults and argument conventions follow KiriKiri Z. Members whose
//! semantics need an engine capability CrySirror does not offer yet throw a
//! TJS exception naming that capability; nothing is silently stubbed.
use crate::host::{KrkrHost, LayerState, TimerState, WindowState};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{Closure, NativeArgCount, ObjectHandle, Runtime, Variant},
};
use serde_json::json;

pub type Rt = Runtime<KrkrHost>;
#[path = "font_class.rs"]
mod font_class;
#[path = "native_extra.rs"]
pub(crate) mod native_extra;

pub fn install(rt: &mut Rt) {
    crate::constants::install_tvp_constants(rt);
    install_system(rt);
    crate::save_struct::install(rt);
    install_debug(rt);
    install_storages(rt);
    install_scripts(rt);
    // PackinOne tjsDataPack + Storages octet I/O.
    crate::plugin_ports::packinone::install(rt);
    install_plugins(rt);
    install_timer(rt);
    crate::async_trigger::install(rt);
    crate::menus::install(rt);
    install_window(rt);
    font_class::install(rt);
    install_layer(rt);
    if let Some(layer)=rt.global_member("Layer").object_handle(){native_extra::install_layer(rt,layer);}
    if let Some(window)=rt.global_member("Window").object_handle(){native_extra::install_window(rt,window);}
    native_extra::bitmap::install(rt);
    crate::native_pad::install(rt);
    // KRKR2 ScriptMgnIntf.cpp registers the real KAGParser as a builtin.
    // Explicit later plugin links can still replace it (including ExtKAG).
    crate::kag::install(rt);
    crate::transitions::install(rt);
    crate::draw_plugin::capture_base_update(rt);
    crate::sound::install(rt);
    crate::video::install(rt);
    crate::pointer::install(rt);
    // Members found missing by the krkrz native-member inventory.
    crate::native_members::install(rt);
    // PackinOne-bundled systemEx members (needs System and Layer).
    crate::system_ex::install(rt);
}

// ---------------------------------------------------------------- helpers

pub fn unsupported(member: &str, capability: &str) -> TjsError {
    TjsError::runtime(format!(
        "{member} is not supported by the CrySirror KiriKiri adapter: requires {capability}"
    ))
}

pub(crate) fn this_of(rt: &Rt, this: Option<ObjectHandle>, what: &str) -> Result<ObjectHandle> {
    let this = this.ok_or_else(|| TjsError::runtime(format!("{what} requires this")))?;
    Ok(rt.bound_this(this).unwrap_or(this))
}

pub(crate) fn object_arg(v: &Variant) -> Option<ObjectHandle> {
    match v {
        Variant::Object(h) => Some(*h),
        Variant::Closure(c) => Some(c.this_obj.unwrap_or(c.object)),
        _ => None,
    }
}

pub(crate) fn int(args: &[Variant], i: usize, default: i64) -> Result<i64> {
    match args.get(i) {
        None | Some(Variant::Void) => Ok(default),
        Some(v) => v.to_integer(),
    }
}

/// A required integer argument. A short argument list is an error; a passed
/// `void` is 0, as krkrz natives read `(tjs_int)*param[i]` (ButtonLayer
/// `_copyButton2` passes a void y to operateRect).
pub(crate) fn need_int(args: &[Variant], i: usize, what: &str) -> Result<i64> {
    match args.get(i) {
        None => Err(TjsError::runtime(format!("{what}: argument {i} required"))),
        Some(Variant::Void) => Ok(0),
        Some(v) => v.to_integer(),
    }
}

pub(crate) fn string(args: &[Variant], i: usize, what: &str) -> Result<String> {
    match args.get(i) {
        None => {
            Err(TjsError::runtime(format!("{what}: argument {i} required")))
        }
        // Native ttstr conversion accepts an explicitly supplied void as "".
        // It is distinct from a missing argument (numparams < required).
        Some(v) => v.to_tjs_string(),
    }
}

pub(crate) fn boolean(v: &Variant) -> bool {
    v.is_truthy()
}

fn static_object(rt: &mut Rt, name: &str) -> ObjectHandle {
    let h = rt.alloc_ordinary_object();
    rt.add_object_class_info(h, name);
    rt.set_global_member(name, Variant::Object(h));
    h
}

pub(crate) type Native = fn(&mut Rt, Option<ObjectHandle>, Vec<Variant>) -> Result<Variant>;
pub(crate) type Getter = fn(&mut Rt, Option<ObjectHandle>) -> Result<Variant>;
pub(crate) type Setter = fn(&mut Rt, Option<ObjectHandle>, Variant) -> Result<()>;

pub(crate) fn methods(rt: &mut Rt, h: ObjectHandle, list: &[(&str, Native)]) {
    for (name, f) in list {
        let f = *f;
        let composite = *name == "operateRect";
        rt.register_object_native_with_arg_count(h, *name, NativeArgCount::Any,
            move |rt: &mut Rt, this, args| {
                if !composite { crate::image_api::before_bitmap_call(rt)?; }
                f(rt, this, args)
            });
    }
}

pub(crate) fn properties(rt: &mut Rt, h: ObjectHandle, list: &[(&str, Getter, Setter)]) {
    for (name, g, s) in list {
        let (g, s) = (*g, *s);
        rt.register_object_native_property(h, *name, g, move |rt: &mut Rt, this, value| {
            crate::image_api::before_bitmap_call(rt)?;
            s(rt, this, value)
        });
    }
}

/// `enabled` along the parent chain (krkrz GetEnabled && ParentEnabled).
fn layer_node_enabled_plain(rt: &Rt, h: ObjectHandle) -> Result<bool> {
    let mut node = Some(h.0);
    let mut depth = 0;
    while let Some(id) = node {
        if !rt.object_member(ObjectHandle(id), "__lp_enabled").is_truthy() {
            return Ok(false);
        }
        node = rt.host().layer(ObjectHandle(id))?.parent;
        depth += 1;
        if depth > 64 {
            return Err(TjsError::runtime("layer enable ancestry limit"));
        }
    }
    Ok(true)
}

/// krkrz GetNodeEnabled: enabled chain and not disabled by a modal layer.
pub(crate) fn layer_node_enabled(rt: &Rt, h: ObjectHandle) -> Result<bool> {
    Ok(layer_node_enabled_plain(rt, h)? && !rt.host().disabled_by_mode(h.0))
}

/// Visible along the parent chain (krkrz GetParentVisible including self).
pub(crate) fn layer_tree_visible(rt: &Rt, h: ObjectHandle) -> Result<bool> {
    let mut node = Some(h.0);
    let mut depth = 0;
    while let Some(id) = node {
        let l = rt.host().layer(ObjectHandle(id))?;
        if !l.visible {
            return Ok(false);
        }
        node = l.parent;
        depth += 1;
        if depth > 64 {
            return Err(TjsError::runtime("layer visibility ancestry limit"));
        }
    }
    Ok(true)
}

/// krkrz SearchFirstFocusable: this layer, then its children in order,
/// depth first: the first visible, node-enabled, focusable one.
fn first_focusable(rt: &Rt, root: ObjectHandle) -> Result<Option<ObjectHandle>> {
    let mut stack = vec![root.0];
    let mut seen = 0;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > 65536 {
            return Err(TjsError::runtime("layer focus search limit"));
        }
        let h = ObjectHandle(id);
        let l = rt.host().layer(h)?;
        if !l.visible {
            continue;
        }
        if rt.object_member(h, "__lp_focusable").is_truthy() && layer_node_enabled(rt, h)? {
            return Ok(Some(h));
        }
        stack.extend(rt.host().sorted_children(id).into_iter().rev());
    }
    Ok(None)
}

/// krkrz RemoveTreeModalState: modal layers inside `root`'s subtree stop
/// being modal (the layer was hidden or disabled).
fn remove_tree_modal_state(rt: &mut Rt, root: ObjectHandle) {
    let host = rt.host();
    let keep: Vec<usize> = host
        .modal_layers
        .iter()
        .copied()
        .filter(|m| !host.is_ancestor_or_self(*m, root.0))
        .collect();
    rt.host_mut().modal_layers = keep;
}

fn node_enabled_states(rt: &Rt) -> Vec<(usize, bool)> {
    let ids: Vec<usize> = rt.host().layers.keys().copied().collect();
    ids.into_iter()
        .map(|id| (id, layer_node_enabled(rt, ObjectHandle(id)).unwrap_or(false)))
        .collect()
}

/// krkrz NotifyNodeEnabledState: onNodeEnabled / onNodeDisabled, delivered
/// immediately, to layers whose node-enabled state changed.
fn notify_node_enabled(rt: &mut Rt, before: Vec<(usize, bool)>) -> Result<()> {
    for (id, was) in before {
        let h = ObjectHandle(id);
        if rt.host().layer(h).is_err() || !rt.object_valid(h) {
            continue;
        }
        let now = layer_node_enabled(rt, h)?;
        if now != was {
            let event = if now { "onNodeEnabled" } else { "onNodeDisabled" };
            if has_member(rt, h, event) {
                rt.call_object_method(h, event, vec![])?;
            }
        }
    }
    Ok(())
}

pub(crate) fn focused_layer(rt: &Rt, window: ObjectHandle) -> Option<ObjectHandle> {
    rt.object_member(window, "__focusedLayer")
        .object_handle()
        .filter(|h| rt.object_valid(*h) && rt.host().layer(*h).is_ok())
}

/// krkrz tTVPLayerManager::SetFocusTo: onBlur(next) on the old layer, then
/// onFocus(prev, direction) on the new one, both delivered immediately.
pub(crate) fn set_focus(rt: &mut Rt, window: ObjectHandle, to: Option<ObjectHandle>, forward: bool) -> Result<()> {
    let from = focused_layer(rt, window);
    if from == to {
        return Ok(());
    }
    let obj = |h: Option<ObjectHandle>| h.map_or(Variant::Null, Variant::Object);
    rt.set_object_member(window, "__focusedLayer", obj(to));
    if let Some(f) = from {
        if has_member(rt, f, "onBlur") {
            rt.call_object_method(f, "onBlur", vec![obj(to)])?;
        }
    }
    if let Some(t) = to {
        if has_member(rt, t, "onFocus") {
            rt.call_object_method(t, "onFocus", vec![obj(from), Variant::Integer(forward as i64)])?;
        }
    }
    Ok(())
}

pub(crate) fn read_only(_: &mut Rt, _: Option<ObjectHandle>, _: Variant) -> Result<()> {
    Err(TjsError::runtime("property is read-only"))
}

/// Install a constructable native class. `init` runs on every constructor
/// call, including the argument-less call TJS makes before a script subclass
/// forwards its arguments with `super.Class(...)`.
pub(crate) fn native_class(rt: &mut Rt, name: &'static str, init: Native) -> ObjectHandle {
    let class = rt.alloc_native_constructor(
        move |rt: &mut Rt, this: Option<ObjectHandle>, args: Vec<Variant>| {
            let existing = this
                .map(|h| rt.bound_this(h).unwrap_or(h))
                .filter(|h| *h != rt.global_handle());
            let h = existing.unwrap_or_else(|| rt.alloc_ordinary_object());
            if let Variant::Object(class) = rt.global_member(name) {
                if rt.object_super_class(h).is_none() && class != h {
                    rt.set_object_super_class(h, class);
                }
                if matches!(rt.object_member(h, name), Variant::Void) {
                    rt.set_object_member(h, name, Variant::Closure(Closure::new(class, Some(h))));
                }
                // Native CreateNew first installs the class, then dispatches
                // its named constructor (tjsNative.cpp:380-415). A script may
                // have replaced that member, e.g. a constructor injection.
                // The saved original class closure re-enters here with h as
                // `this`, so only that original initializes engine resources.
                if existing.is_none() {
                    let constructor = rt.object_member(class, name);
                    if let Some(method) = constructor.object_handle()
                        && method != class
                    {
                        rt.add_object_class_info(h, name);
                        for (member, value) in rt.object_members(class) {
                            let value = match value {
                                Variant::Object(object) => Variant::Closure(Closure::new(object, Some(h))),
                                Variant::Closure(closure) if closure.this_obj.is_none() =>
                                    Variant::Closure(Closure::new(closure.object, Some(h))),
                                value => value,
                            };
                            rt.set_object_member(h, member, value);
                        }
                        let constructor = rt.object_member(h, name);
                        rt.call_function(constructor, args)?;
                        return Ok(Variant::Object(h));
                    }
                }
            }
            rt.add_object_class_info(h, name);
            init(rt, Some(h), args)?;
            Ok(Variant::Object(h))
        },
    );
    rt.add_object_class_info(class, name);
    rt.set_object_member(class, name, Variant::Closure(Closure::new(class, None)));
    rt.set_global_member(name, Variant::Object(class));
    class
}

/// A member defined anywhere in the instance's class chain.
pub fn has_member(rt: &Rt, h: ObjectHandle, name: &str) -> bool {
    let mut current = Some(h);
    let mut depth = 0;
    while let Some(o) = current {
        if !matches!(rt.object_member(o, name), Variant::Void) {
            return true;
        }
        depth += 1;
        if depth > 64 {
            return false;
        }
        current = rt.object_super_class(o);
    }
    false
}

/// TJS2 expression mode (`Scripts.eval`, `evalStorage`, KAG `exp=`):
/// tTJSLexicalAnalyzer prepends `return` and appends `;` without
/// parentheses (`tjsLex.cpp:1061-1066,1753`), so text ending in `;` is valid.
pub(crate) fn expression_source(text: &str) -> String {
    format!("return {text}\n;")
}

// Eval accepts a bare expression at EOF, including one ending in a line comment.
// With no result register the subsequent statements must still execute.
fn eval_source(text: &str, need_result: bool) -> String {
    if need_result { expression_source(text) } else { format!("{text}\n;") }
}

pub fn run_source(
    rt: &mut Rt,
    name: &str,
    source: &str,
    this: Option<ObjectHandle>,
) -> Result<Variant> {
    let file = rt.compile_source(name, source)?;
    rt.execute_file_with_this(&file, this)
}

/// TVPExecuteStorage's first stage (`ScriptMgnIntf.cpp`, `tTJS::LoadByteCode`
/// in `tjs.cpp`): TJS2 bytecode runs, and a `KBAD100` binary struct pack
/// (`saveStruct(.., "b")`) is the result itself. `None`: a text script.
fn storage_non_text(rt: &mut Rt, name: &str, mode:&str, context:Option<ObjectHandle>) -> Result<Option<Variant>> {
    let bytes = rt.read_storage_binary(name, mode)?;
    if bytes.starts_with(&crysirror_tjs2::bytecode::BYTECODE_SIGNATURE) {
        let file=crysirror_tjs2::bytecode::BytecodeFile::parse(&bytes)?;
        return rt.execute_file_with_this(&file,context).map(Some);
    }
    if bytes.starts_with(b"KBAD100\0") {
        return Ok(Some(rt.decode_binary_struct(&bytes)?.unwrap_or_default()));
    }
    Ok(None)
}

pub fn exec_storage(rt: &mut Rt, name: &str) -> Result<Variant> { exec_storage_with_context(rt,name,"",None,None) }

// Original Scripts.execStorage/evalStorage take context as argument THREE,
// unlike the string variants whose name/line arguments put it at FOUR.
fn script_storage_context(args:&[Variant])->Result<(String,Option<ObjectHandle>)> {
    let mode=match args.get(1) {None|Some(Variant::Void)=>String::new(),Some(v)=>v.to_tjs_string()?};
    if !mode.is_empty() {return Err(unsupported("Scripts storage read mode","offset/encoding-specific stream modes"));}
    let context=match args.get(2) {None|Some(Variant::Void|Variant::Null)=>None,Some(v)=>Some(object_arg(v).ok_or_else(||TjsError::runtime("Scripts storage context must be an object or null"))?)};
    Ok((mode,context))
}
fn exec_storage_with_context(rt:&mut Rt,name:&str,mode:&str,context:Option<ObjectHandle>,eval_result:Option<bool>)->Result<Variant> {
    if let Some(value)=storage_non_text(rt,name,mode,context)? {return Ok(value);}
    let source=rt.read_storage_text(name,mode)?;
    if let Some(need_result) = eval_result {
        if source.trim().is_empty() { return Ok(Variant::Void); }
        return run_source(rt,name,&eval_source(&source,need_result),context);
    }
    run_source(rt,name,&source,context)
}

// ----------------------------------------------------------------- System

fn install_system(rt: &mut Rt) {
    let h = static_object(rt, "System");
    // SystemIntf.cpp's native defaults are NULL, not no-op functions.
    for member in ["onActivate","onDeactivate","exceptionHandler"] {
        rt.set_object_member(h,member,Variant::Null);
    }
    methods(
        rt,
        h,
        &[
            ("getTickCount", |rt, _, _| {
                let host=rt.host_mut();
                let now=host.now;
                if host.clock_poll_tick!=now {
                    host.clock_poll_tick=now;host.clock_poll_reads=0;
                }
                host.clock_poll_reads=host.clock_poll_reads.saturating_add(1);
                let yield_read=host.can_suspend && host.clock_poll_reads>=256;
                if yield_read {host.clock_yield_tick=Some(now);}
                if yield_read {rt.request_suspend();}
                Ok(Variant::Integer(now as i64))
            }),
            // krkrz SystemIntf.cpp createUUID: 128 random bits as a version-4
            // UUID string. Bits come from a splitmix64 stream over a process
            // counter and the engine clock (identifiers, not secrets).
            ("createUUID", |rt, _, _| {
                static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mix = |mut z: u64| {
                    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
                    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                    z ^ (z >> 31)
                };
                let seed = (rt.host().now ^ seq.rotate_left(32)).wrapping_add(seq);
                let mut u = [0u8; 16];
                u[..8].copy_from_slice(&mix(seed).to_le_bytes());
                u[8..].copy_from_slice(&mix(seed ^ 0xa5a5_a5a5_a5a5_a5a5).to_le_bytes());
                u[8] = u[8] & 0x3f | 0x80;
                u[6] = u[6] & 0x0f | 0x40;
                let h: Vec<String> = u.iter().map(|b| format!("{b:02x}")).collect();
                Ok(Variant::String(
                    format!(
                        "{}-{}-{}-{}-{}",
                        h[0..4].concat(),
                        h[4..6].concat(),
                        h[6..8].concat(),
                        h[8..10].concat(),
                        h[10..16].concat()
                    )
                    .into(),
                ))
            }),
            ("inform", |rt, _, a| {
                let m = a
                    .first()
                    .map(Variant::to_tjs_string)
                    .transpose()?
                    .unwrap_or_default();
                rt.host_mut().push_log(format!("[inform] {m}"));
                Ok(Variant::Void)
            }),
            ("exit", |rt, _, _| {
                rt.host_mut().exit_requested = true;
                Ok(Variant::Void)
            }),
            ("terminate", |rt, _, _| {
                rt.host_mut().exit_requested = true;
                Ok(Variant::Void)
            }),
            // Cache and compaction requests are hints with no observable result.
            ("touchImages", |_, _, _| Ok(Variant::Void)),
            ("doCompact", |_, _, _| Ok(Variant::Void)),
            ("toActualColor", |_, _, a| {
                Ok(Variant::Integer(int(&a, 0, 0)?))
            }),
            ("getArgument", |rt, _, a| {
                let key = string(&a, 0, "System.getArgument")?;
                let args = rt.global_member("__crysirror_arguments");
                Ok(object_arg(&args)
                    .map(|o| rt.object_member(o, &key))
                    .unwrap_or_default())
            }),
            ("setArgument", |rt, _, a| {
                let key = string(&a, 0, "System.setArgument")?;
                let table = match object_arg(&rt.global_member("__crysirror_arguments")) {
                    Some(t) => t,
                    None => {
                        let t = rt.alloc_dictionary_object();
                        rt.set_global_member("__crysirror_arguments", Variant::Object(t));
                        t
                    }
                };
                rt.set_object_member(table, key, a.get(1).cloned().unwrap_or_default());
                Ok(Variant::Void)
            }),
            ("addContinuousHandler", |rt, _, a| {
                let f = a.first().cloned().unwrap_or_default();
                continuous(rt).push(f);
                Ok(Variant::Void)
            }),
            ("removeContinuousHandler", |rt, _, a| {
                let f = a.first().cloned().unwrap_or_default();
                // EventIntf.cpp:1091-1102 clears the live slot immediately.
                // Erasing it would shift the next callback underneath the
                // current delivery cursor; a frame snapshot would still call
                // a mover whose finalize() has just removed this handler.
                let mut handlers = continuous(rt);
                let mut items = handlers.list();
                if let Some(item) = items.iter_mut().find(|g| g.discern_eq(&f)) {
                    *item = Variant::Void;
                    handlers.store(items);
                }
                Ok(Variant::Void)
            }),
            ("createAppLock", |_, _, _| Ok(Variant::Integer(1))),
            // A modal native dialog has no engine service yet; keep the prompt
            // in the error so the condition that raised it stays visible.
            ("confirm", |_, _, a| {
                let text = a
                    .first()
                    .map(Variant::to_tjs_string)
                    .transpose()?
                    .unwrap_or_default();
                Err(unsupported(
                    &format!("System.confirm({text:?})"),
                    "a modal dialog service",
                ))
            }),
            // A game script does not launch host processes or browsers from this
            // player; the native result for a failed launch is false.
            ("shellExecute", |rt, _, a| {
                let target = a.first().map(Variant::to_tjs_string).transpose()?.unwrap_or_default();
                rt.host_mut().log.push(format!("System.shellExecute({target:?}) declined by the player"));
                Ok(Variant::Integer(0))
            }),
            // The host has no Windows registry: every key is absent, and the
            // native result for an absent key is void.
            ("readRegValue", |_, _, a| {
                string(&a, 0, "System.readRegValue")?;
                Ok(Variant::Void)
            }),
        ],
    );
    properties(
        rt,
        h,
        &[
            // The presenting host's display (canvas when it has none). KAGEX
            // sizes its window and decides on forced fullscreen from these.
            ("screenWidth", |rt, _| Ok(Variant::Integer(rt.host().display_rects().0[0])), read_only),
            ("screenHeight", |rt, _| Ok(Variant::Integer(rt.host().display_rects().0[1])), read_only),
            ("desktopLeft", |rt, _| Ok(Variant::Integer(rt.host().display_rects().1[0])), read_only),
            ("desktopTop", |rt, _| Ok(Variant::Integer(rt.host().display_rects().1[1])), read_only),
            ("desktopWidth", |rt, _| Ok(Variant::Integer(rt.host().display_rects().1[2])), read_only),
            ("desktopHeight", |rt, _| Ok(Variant::Integer(rt.host().display_rects().1[3])), read_only),
            (
                "exePath",
                |_, _| Ok(Variant::String("file://./".into())),
                read_only,
            ),
            (
                "exeName",
                |rt, _| {
                    let exe = rt.host().executable.as_deref().unwrap_or("crysirror");
                    Ok(Variant::String(format!("file://./{exe}")))
                },
                read_only,
            ),
            (
                "dataPath",
                |_, _| Ok(Variant::String("file://./savedata/".into())),
                read_only,
            ),
            // krkrz personalPath (the user's documents) and appDataPath
            // (per-user application data) are host folders; the engine keeps
            // every game write inside the sandboxed save capability root, so
            // both are folders there, the same on every platform.
            (
                "personalPath",
                |_, _| Ok(Variant::String("file://./savedata/personal/".into())),
                read_only,
            ),
            (
                "appDataPath",
                |_, _| Ok(Variant::String("file://./savedata/appdata/".into())),
                read_only,
            ),
            (
                "platformName",
                |_, _| Ok(Variant::String("CrySirror".into())),
                read_only,
            ),
            (
                "osName",
                |_, _| Ok(Variant::String("CrySirror".into())),
                read_only,
            ),
            (
                "versionString",
                |_, _| Ok(Variant::String("CrySirror-krkr".into())),
                read_only,
            ),
            // krkr's free-form "about" text (shown by debug/about dialogs).
            (
                "versionInformation",
                |_, _| Ok(Variant::String("CrySirror KiriKiri adapter".into())),
                read_only,
            ),
        ],
    );
    // krkrz System.drawThreadNum: 0 = automatic (the krkrz default). Engine
    // CPU drawing is already snapshot-safe for self-copies; the value is kept
    // for scripts that read back what they set.
    rt.set_object_member(h, "__drawThreadNum", Variant::Integer(0));
    rt.register_object_native_property(
        h,
        "drawThreadNum",
        |rt, _| {
            let h = rt.global_member("System").object_handle().unwrap();
            Ok(rt.object_member(h, "__drawThreadNum"))
        },
        |rt, _, v| {
            let n = v.to_integer()?;
            if !(0..=64).contains(&n) {
                return Err(TjsError::runtime("System.drawThreadNum must be 0..64"));
            }
            let h = rt.global_member("System").object_handle().unwrap();
            rt.set_object_member(h, "__drawThreadNum", Variant::Integer(n));
            Ok(())
        },
    );
    rt.set_object_member(h, "__title", Variant::String(String::new()));
    rt.register_object_native_property(
        h,
        "title",
        |rt, _| {
            let h = rt.global_member("System").object_handle().unwrap();
            Ok(rt.object_member(h, "__title"))
        },
        |rt, _, v| {
            let text = v.to_tjs_string()?;
            let h = rt.global_member("System").object_handle().unwrap();
            rt.host_mut().ui_op(json!({"op":"set_title","text":text}));
            rt.set_object_member(h, "__title", Variant::String(text));
            Ok(())
        },
    );
    rt.set_object_member(h, "exceptionHandler", Variant::Void);
}

pub const CONTINUOUS: &str = "__crysirror_continuous";

/// Continuous handlers live in a script array on the System object so TJS
/// owns their lifetimes.
fn continuous(rt: &mut Rt) -> ContinuousList<'_> {
    ContinuousList { rt }
}

struct ContinuousList<'a> {
    rt: &'a mut Rt,
}

impl ContinuousList<'_> {
    fn list(&mut self) -> Vec<Variant> {
        let system = object_arg(&self.rt.global_member("System"));
        let arr = system.and_then(|s| object_arg(&self.rt.object_member(s, CONTINUOUS)));
        arr.and_then(|a| self.rt.array_elements(a).map(<[Variant]>::to_vec))
            .unwrap_or_default()
    }
    fn store(&mut self, items: Vec<Variant>) {
        if let Some(system) = object_arg(&self.rt.global_member("System")) {
            let arr = self.rt.alloc_array_object(items);
            self.rt
                .set_object_member(system, CONTINUOUS, Variant::Object(arr));
        }
    }
    fn push(&mut self, f: Variant) {
        let mut items = self.list();
        if !items.iter().any(|g| g.discern_eq(&f)) {
            items.push(f);
        }
        self.store(items);
    }
    fn retain(&mut self, keep: impl Fn(&Variant) -> bool) {
        let items = self.list().into_iter().filter(|v| keep(v)).collect();
        self.store(items);
    }
}

pub fn continuous_handlers(rt: &mut Rt) -> Vec<Variant> {
    continuous(rt).list()
}

pub(crate) fn continuous_handler_at(rt: &Rt, index: usize) -> Option<Variant> {
    let system = object_arg(&rt.global_member("System"))?;
    let array = object_arg(&rt.object_member(system, CONTINUOUS))?;
    rt.array_elements(array)?.get(index).cloned()
}

pub(crate) fn compact_continuous_handlers(rt: &mut Rt) {
    let has_empty = object_arg(&rt.global_member("System"))
        .and_then(|s| object_arg(&rt.object_member(s, CONTINUOUS)))
        .and_then(|a| rt.array_elements(a))
        .is_some_and(|items| items.iter().any(|v| matches!(v, Variant::Void)));
    if has_empty {
        continuous(rt).retain(|v| !matches!(v, Variant::Void));
    }
}

// ------------------------------------------------------------------ Debug

fn install_debug(rt: &mut Rt) {
    let h = static_object(rt, "Debug");
    let message: Native = |rt, _, a| {
        let parts = a
            .iter()
            .map(Variant::to_tjs_string)
            .collect::<Result<Vec<_>>>()?;
        rt.host_mut().push_log(parts.join(" "));
        Ok(Variant::Void)
    };
    methods(
        rt,
        h,
        &[
            ("message", message),
            ("notice", message),
            ("logAsError", |_, _, _| Ok(Variant::Void)),
        ],
    );
    // Log-to-file is a developer diagnostic; the engine log already records messages.
    methods(rt, h, &[("startLogToFile", |_, _, _| Ok(Variant::Void))]);
    let console = rt.alloc_ordinary_object();
    rt.set_object_member(console, "visible", Variant::Integer(0));
    rt.set_object_member(h, "console", Variant::Object(console));
}

// --------------------------------------------------------------- Storages

fn install_storages(rt: &mut Rt) {
    let h = static_object(rt, "Storages");
    methods(
        rt,
        h,
        &[
            ("setDefaultPath", |rt, _, a| {
                let path = string(&a, 0, "Storages.setDefaultPath")?;
                rt.host_mut()
                    .storage_mut()?
                    .set_default_path(&path)
                    .map_err(|e| {
                        TjsError::runtime(format!("Storages.setDefaultPath({path:?}): {e:#}"))
                    })?;
                Ok(Variant::Void)
            }),
            ("isExistentStorage", |rt, _, a| {
                rt.host_mut().before_batch_item()?;
                let name = string(&a, 0, "Storages.isExistentStorage")?;
                crate::wrapper_plugins::sync_proxy(rt)?;
                let found = if crate::varfile_plugin::is_uri(&name) {
                    crate::varfile_plugin::exists(rt,&name)?
                } else if crate::psbfile_plugin::is_uri(&name) {
                    crate::psbfile_plugin::exists(rt, &name)?
                } else { rt.host().storage()?.resolve(&name).is_some() };
                if !found && std::env::var_os("CRYSIRROR_KRKR_TRACE_STORAGE").is_some() {
                    rt.host_mut().push_log(format!("[storage miss] {name}"));
                }
                Ok(Variant::Integer(found as i64))
            }),
            ("getPlacedPath", |rt, _, a| {
                let name = string(&a, 0, "Storages.getPlacedPath")?;
                crate::wrapper_plugins::sync_proxy(rt)?;
                if crate::varfile_plugin::is_uri(&name) {
                    return Ok(Variant::String(if crate::varfile_plugin::exists(rt,&name)? {name} else {String::new()}));
                }
                Ok(Variant::String(rt.host().storage()?.placed_path(&name).unwrap_or_default()))
            }),
            ("getFullPath", |rt, _, a| {
                let name = string(&a, 0, "Storages.getFullPath")?;
                if crate::varfile_plugin::is_uri(&name) {
                    crate::varfile_plugin::validate_uri(&name)?;
                    return Ok(Variant::String(name));
                }
                let full = rt
                    .host()
                    .storage()?
                    .full_uri(&name)
                    .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
                Ok(Variant::String(full))
            }),
            ("getLocalName", |_, _, a| {
                Ok(Variant::String(crate::storage::normalize(&string(
                    &a,
                    0,
                    "Storages.getLocalName",
                )?)))
            }),
            ("addAutoPath", |rt, _, a| {
                let path = string(&a, 0, "Storages.addAutoPath")?;
                crate::wrapper_plugins::sync_proxy(rt)?;
                if crate::protect_plugin::auto_path_hook(rt, &path, true)? == Some(true) {
                    return Ok(Variant::Void);
                }
                rt.host_mut()
                    .storage_mut()?
                    .add_auto_path(&path)
                    .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
                Ok(Variant::Void)
            }),
            ("removeAutoPath", |rt, _, a| {
                let path = string(&a, 0, "Storages.removeAutoPath")?;
                if crate::protect_plugin::auto_path_hook(rt, &path, false)? == Some(true) {
                    return Ok(Variant::Void);
                }
                rt.host_mut()
                    .storage_mut()?
                    .remove_auto_path(&path)
                    .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
                Ok(Variant::Void)
            }),
            ("extractStorageExt", |_, _, a| {
                let n = string(&a, 0, "Storages.extractStorageExt")?;
                let base = n.rsplit(['/', '\\', '>']).next().unwrap_or(&n);
                Ok(Variant::String(
                    base.rfind('.')
                        .map(|i| base[i..].to_owned())
                        .unwrap_or_default(),
                ))
            }),
            ("extractStorageName", |_, _, a| {
                let n = string(&a, 0, "Storages.extractStorageName")?;
                Ok(Variant::String(
                    n.rsplit(['/', '\\', '>']).next().unwrap_or(&n).to_owned(),
                ))
            }),
            ("extractStoragePath", |_, _, a| {
                let n = string(&a, 0, "Storages.extractStoragePath")?;
                let cut = n.rfind(['/', '\\', '>']).map_or(0, |i| i + 1);
                Ok(Variant::String(n[..cut].to_owned()))
            }),
            ("chopStorageExt", |_, _, a| {
                let n = string(&a, 0, "Storages.chopStorageExt")?;
                let start = n.rfind(['/', '\\', '>']).map_or(0, |i| i + 1);
                Ok(Variant::String(match n[start..].rfind('.') {
                    Some(i) => n[..start + i].to_owned(),
                    None => n,
                }))
            }),
            // StorageImpl::TVPSearchCD returns a matching drive letter. The
            // host publishes installed/mounted media explicitly; no ambient
            // optical probing and no unconditional success/failure here.
            ("searchCD", |rt, _, a| {
                let label=string(&a,0,"Storages.searchCD")?;
                let storage=rt.host().storage()?;
                let drive=match &storage.media {
                    Some(media)=>media.find_label(crysirror_core::state::ClientId(rt.host().client),&label)
                        .map_err(|e|TjsError::runtime(e.to_string()))?.map(|v|v.drive.to_string()),
                    None=>None,
                };
                Ok(Variant::String(drive.unwrap_or_default()))
            }),
        ],
    );
}

// ---------------------------------------------------------------- Scripts

fn install_scripts(rt: &mut Rt) {
    let h = static_object(rt, "Scripts");
    methods(
        rt,
        h,
        &[
            // krkrz core: dynamic member dispatch through the object's `missing`.
            ("setCallMissing", |rt, _, a| {
                let h = a.first().and_then(object_arg).ok_or_else(|| {
                    TjsError::runtime("Scripts.setCallMissing requires an object")
                })?;
                rt.set_object_call_missing(h, "missing");
                Ok(Variant::Void)
            }),
            ("getClassNames", |rt, _, a| {
                let Some(h) = a.first().and_then(object_arg) else {
                    return Ok(Variant::Void);
                };
                let names = rt
                    .object_class_infos(h)
                    .iter()
                    .cloned()
                    .map(Variant::String)
                    .collect();
                Ok(Variant::Object(rt.alloc_array_object(names)))
            }),
            // Diagnostic text only: the vendored VM exposes no script call
            // stack to natives, so the trace is empty (documented divergence).
            ("getTraceString", |_, _, _| {
                Ok(Variant::String(String::new()))
            }),
            // Binary data packs (`.pbd`): KBAD100 / TJS ns0 structs.
            ("loadDataPack", |rt, _, a| {
                let name = string(&a, 0, "Scripts.loadDataPack")?;
                // The exact storage first (packs saved under a game-chosen
                // name such as `datavc.ksd`), else the `.pbd` spelling.
                let name = if name.to_ascii_lowercase().ends_with(".pbd")
                    || rt.host().storage()?.resolve(&name).is_some()
                {
                    name
                } else {
                    format!("{name}.pbd")
                };
                if a.get(1).and_then(object_arg).is_some_and(|o| !matches!(rt.object_member(o, "outeriv"), Variant::Void)) {
                    return Err(unsupported(
                        "Scripts.loadDataPack outeriv",
                        "an external data pack IV (the pack header IV is supported)",
                    ));
                }
                let bytes = rt.read_storage_binary(&name, "")?;
                let value = if bytes.starts_with(b"KBAD100\0") {
                    rt.decode_binary_struct(&bytes)?
                } else if crysirror_resource::krkr_pbd::is_pbd(&bytes) {
                    // Shared decoder: TJS/ns0 and (encrypted, LZ4) TJS/4s0.
                    let value = crysirror_resource::krkr_pbd::decode(&bytes)
                        .map_err(|e| TjsError::runtime(format!("Scripts.loadDataPack `{name}`: {e:#}")))?;
                    Some(pbd_value(rt, value))
                } else {
                    None
                };
                value.ok_or_else(|| {
                    TjsError::runtime(format!(
                        "Scripts.loadDataPack: `{name}` is not a binary data pack"
                    ))
                })
            }),
            ("execStorage", |rt, _, a| {
                let name = string(&a, 0, "Scripts.execStorage")?;
                let (mode,context)=script_storage_context(&a)?;
                exec_storage_with_context(rt,&name,&mode,context,None)
            }),
            ("evalStorage", |rt, _, a| {
                let name = string(&a, 0, "Scripts.evalStorage")?;
                let (mode,context)=script_storage_context(&a)?;
                exec_storage_with_context(rt,&name,&mode,context,Some(rt.native_result_needed()))
            }),
            ("exec", |rt, _, a| {
                let source = string(&a, 0, "Scripts.exec")?;
                let ctx = a.get(3).and_then(object_arg);
                run_source(rt, "Scripts.exec", &source, ctx)
            }),
            ("eval", |rt, _, a| {
                let source = string(&a, 0, "Scripts.eval")?;
                if source.trim().is_empty() {
                    return Ok(Variant::Void);
                }
                let ctx = a.get(3).and_then(object_arg);
                let source = eval_source(&source, rt.native_result_needed());
                run_source(rt, "Scripts.eval", &source, ctx)
            }),
        ],
    );
}

pub(crate) fn pbd_value(rt: &mut Rt, v: crysirror_resource::krkr_pbd::PbdValue) -> Variant {
    use crysirror_resource::krkr_pbd::PbdValue as P;
    match v {
        P::Void => Variant::Void,
        P::String(s) => Variant::String(s),
        P::Integer(i) => Variant::Integer(i),
        P::Real(r) => Variant::Real(r),
        P::Array(items) => {
            let items = items.into_iter().map(|v| pbd_value(rt, v)).collect();
            Variant::Object(rt.alloc_array_object(items))
        }
        P::Dictionary(entries) => {
            let h = rt.alloc_dictionary_object();
            for (k, v) in entries {
                let v = pbd_value(rt, v);
                rt.set_object_member(h, k, v);
            }
            Variant::Object(h)
        }
    }
}

fn install_plugins(rt: &mut Rt) {
    let h = static_object(rt, "Plugins");
    crate::plugins::install(rt, h);
}

// ------------------------------------------------------------------ Timer

fn install_timer(rt: &mut Rt) {
    let class = native_class(rt, "Timer", |rt, this, a| {
        let h = this_of(rt, this, "Timer")?;
        if a.is_empty() {
            return Ok(Variant::Void);
        }
        let action = match a.get(1) {
            None | Some(Variant::Void) => "action".to_owned(),
            Some(v) => v.to_tjs_string()?,
        };
        rt.set_object_member(h, "__actionOwner", a[0].clone());
        rt.set_object_member(h, "__actionName", Variant::String(action));
        rt.host_mut().timers.insert(
            h.0,
            TimerState {
                mode: 0,
                interval: 1000,
                enabled: false,
                capacity: 6,
                next: 0,
            },
        );
        rt.set_object_member(h, "mode", Variant::Integer(0));
        Ok(Variant::Void)
    });
    methods(
        rt,
        class,
        &[
            ("onTimer", |rt, this, a| {
                let h = this_of(rt, this, "Timer.onTimer")?;
                let owner = rt.object_member(h, "__actionOwner");
                if matches!(owner, Variant::Void | Variant::Null) {
                    return Ok(Variant::Void);
                }
                let name = rt.object_member(h, "__actionName").to_tjs_string()?;
                if name.is_empty() {
                    rt.call_function(owner, a)
                } else {
                    rt.call_variant_method(owner, &name, a)
                }
            }),
            ("finalize", |_, _, _| Ok(Variant::Void)),
        ],
    );
    properties(
        rt,
        class,
        &[
            (
                "mode",
                |rt, t| Ok(Variant::Integer(timer(rt, t)?.mode as i64)),
                |rt, t, v| {
                    let n = v.to_integer()?;
                    if !(0..=2).contains(&n) {
                        return Err(TjsError::runtime("invalid Timer.mode"));
                    }
                    timer_mut(rt, t)?.mode = n as u8;
                    Ok(())
                },
            ),
            (
                "interval",
                |rt, t| Ok(Variant::Integer(timer(rt, t)?.interval as i64)),
                |rt, t, v| {
                    let now = rt.host().now;
                    let i = v.to_integer()?.max(0) as u64;
                    let s = timer_mut(rt, t)?;
                    s.interval = i;
                    s.next = now
                        .checked_add(i)
                        .ok_or_else(|| TjsError::runtime("timer deadline overflow"))?;
                    let h = this_of(rt, t, "Timer.interval")?;
                    let client = rt.host().client;
                    rt.host_mut()
                        .deferred
                        .cancel(client, crate::async_trigger::target(h.0, 1));
                    Ok(())
                },
            ),
            (
                "enabled",
                |rt, t| Ok(Variant::Integer(timer(rt, t)?.enabled as i64)),
                |rt, t, v| {
                    let now = rt.host().now;
                    let s = timer_mut(rt, t)?;
                    let on = boolean(&v);
                    if on && !s.enabled {
                        s.next = now + s.interval;
                    }
                    s.enabled = on;
                    if !on {
                        let h = this_of(rt, t, "Timer.enabled")?;
                        let client = rt.host().client;
                        rt.host_mut()
                            .deferred
                            .cancel(client, crate::async_trigger::target(h.0, 1));
                    }
                    Ok(())
                },
            ),
            (
                "capacity",
                |rt, t| Ok(Variant::Integer(timer(rt, t)?.capacity as i64)),
                |rt, t, v| {
                    timer_mut(rt, t)?.capacity = v.to_integer()?.max(0) as u64;
                    Ok(())
                },
            ),
        ],
    );
}

fn timer(rt: &Rt, t: Option<ObjectHandle>) -> Result<TimerState> {
    let h = this_of(rt, t, "Timer")?;
    rt.host()
        .timers
        .get(&h.0)
        .cloned()
        .ok_or_else(|| TjsError::runtime("Timer is not constructed"))
}

fn timer_mut(rt: &mut Rt, t: Option<ObjectHandle>) -> Result<&mut TimerState> {
    let h = this_of(rt, t, "Timer")?;
    rt.host_mut()
        .timers
        .get_mut(&h.0)
        .ok_or_else(|| TjsError::runtime("Timer is not constructed"))
}

// ----------------------------------------------------------------- Window

/// BasicDrawDevice members, on the class and on every instance. `recreate`
/// has nothing to rebuild (presentation is the engine's); the native
/// `interface` pointer is never exposed.
pub(crate) fn draw_device_members(rt: &mut Rt, h: ObjectHandle) {
    rt.register_object_native_property(
        h,
        "interface",
        |_, _| {
            Err(unsupported(
                "BasicDrawDevice.interface",
                "native draw-device pointers (never exposed)",
            ))
        },
        read_only,
    );
    methods(rt, h, &[("recreate", |_, _, _| Ok(Variant::Void))]);
}

fn install_window(rt: &mut Rt) {
    let class = native_class(rt, "Window", |rt, this, _| {
        let h = this_of(rt, this, "Window")?;
        if rt.host().windows.contains_key(&h.0) {
            return Ok(Variant::Void);
        }
        // Games also derive invisible controller classes from Window. Those
        // objects retain native state without requiring another OS output.
        // Presentation of a second Window remains an explicit host boundary.
        if rt.host().windows.len()>=1024 {return Err(TjsError::runtime("native Window object limit exceeded"));}
        let output=!rt.host().windows.values().any(|window|window.output);
        let [w, height] = rt.host().canvas;
        rt.host_mut().windows.insert(
            h.0,
            WindowState {
                output,
                primary: None,
                views: Vec::new(),
                device_zoom: None,
                inner: [w as i64, height as i64],
                layer_pos:[0,0],
                visible: false,
                menu: None,
            },
        );
        let root = crate::menus::root(rt, h)?;
        rt.host_mut().windows.get_mut(&h.0).unwrap().menu = Some(root.0);
        rt.host_mut().menus_revision += 1;
        rt.set_object_member(h, "__caption", Variant::String(String::new()));
        // Class objects may be stored bound; act on the object identity.
        if let Some(class) = rt.global_member("Window").object_handle() {
            if output {rt.set_object_member(class, "mainWindow", Variant::self_bound(h));}
            // krkrz: every Window starts with a Window.BasicDrawDevice, which
            // here is the engine's retained-layer presentation path.
            if let Some(device) = rt.object_member(class, "BasicDrawDevice").object_handle() {
                let d = rt.construct(Variant::Object(device), vec![])?;
                rt.set_object_member(h, "__drawDevice", d);
            }
        }
        Ok(Variant::Void)
    });
    // Window.BasicDrawDevice: the engine presentation device. It has no
    // native `interface` pointer; that member fails visibly.
    let device = rt.alloc_native_constructor(|rt: &mut Rt, this: Option<ObjectHandle>, _| {
        let h = this
            .map(|h| rt.bound_this(h).unwrap_or(h))
            .filter(|h| *h != rt.global_handle())
            .unwrap_or_else(|| rt.alloc_ordinary_object());
        rt.add_object_class_info(h, "BasicDrawDevice");
        rt.set_object_member(h, "__crysirror_device", Variant::Integer(1));
        draw_device_members(rt, h);
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(device, "BasicDrawDevice");
    draw_device_members(rt, device);
    rt.set_object_member(class, "BasicDrawDevice", Variant::Object(device));
    // WindowImpl.cpp exposes the read-only GetWindowHandleForPlugin result.
    // A managed Window has no handle for a native plug-in on any host.
    rt.register_object_native_property(
        class, "HWND",
        |rt, t| {
            window_of(rt, t)?;
            Ok(Variant::Integer(0))
        },
        read_only,
    );
    rt.register_object_native_property(
        class,
        "drawDevice",
        |rt, t| {
            let h = this_of(rt, t, "Window.drawDevice")?;
            Ok(rt.object_member(h, "__drawDevice"))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "Window.drawDevice")?;
            let engine = object_arg(&v).is_some_and(|d| {
                matches!(
                    rt.object_member(d, "__crysirror_device"),
                    Variant::Integer(1)
                )
            });
            if !engine {
                return Err(unsupported(
                    "Window.drawDevice = <native device>",
                    "an engine-provided draw device",
                ));
            }
            rt.set_object_member(h, "__drawDevice", v);
            Ok(())
        },
    );
    // krkr2 Win32 frame styles (krkrz dropped them). The engine draws no
    // native frame/scroll bars; the values are kept so scripts read back
    // what they set. krkr2 defaults: innerSunken true, showScrollBars true.
    for (name, slot, default) in [
        ("innerSunken", "__innerSunken", 1),
        ("showScrollBars", "__showScrollBars", 1),
    ] {
        rt.register_object_native_property(
            class,
            name,
            move |rt: &mut Rt, t: Option<ObjectHandle>| {
                let h = this_of(rt, t, "Window")?;
                Ok(match rt.object_member(h, slot) {
                    Variant::Void => Variant::Integer(default),
                    v => v,
                })
            },
            move |rt: &mut Rt, t: Option<ObjectHandle>, v: Variant| {
                let h = this_of(rt, t, "Window")?;
                rt.set_object_member(h, slot, Variant::Integer(v.is_truthy() as i64));
                Ok(())
            },
        );
    }
    // krkrz Window min/max size (WindowIntf.cpp setMinSize/setMaxSize; 0 is
    // unconstrained). The engine window scales the canvas to any size, so the
    // limits only bound user resizing; they are kept for scripts to read.
    for name in ["minWidth", "minHeight", "maxWidth", "maxHeight"] {
        let slot: &'static str = match name {
            "minWidth" => "__minWidth",
            "minHeight" => "__minHeight",
            "maxWidth" => "__maxWidth",
            _ => "__maxHeight",
        };
        rt.register_object_native_property(
            class,
            name,
            move |rt: &mut Rt, t: Option<ObjectHandle>| {
                let h = this_of(rt, t, "Window")?;
                Ok(Variant::Integer(rt.object_member(h, slot).to_integer().unwrap_or(0)))
            },
            move |rt: &mut Rt, t: Option<ObjectHandle>, v: Variant| {
                let h = this_of(rt, t, "Window")?;
                rt.set_object_member(h, slot, Variant::Integer(v.to_integer()?.max(0)));
                Ok(())
            },
        );
    }
    for (name, slots) in [("setMinSize", ["__minWidth", "__minHeight"]), ("setMaxSize", ["__maxWidth", "__maxHeight"])] {
        rt.register_object_native(class, name, move |rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>| {
            let h = this_of(rt, t, "Window")?;
            let (w, ht) = (need_int(&a, 0, "Window.setMinSize/setMaxSize")?, need_int(&a, 1, "Window.setMinSize/setMaxSize")?);
            rt.set_object_member(h, slots[0], Variant::Integer(w.max(0)));
            rt.set_object_member(h, slots[1], Variant::Integer(ht.max(0)));
            Ok(Variant::Void)
        });
    }
    rt.register_object_native_property(
        class,
        "focusedLayer",
        |rt, t| {
            let h = this_of(rt, t, "Window.focusedLayer")?;
            Ok(focused_layer(rt, h).map_or(Variant::Null, Variant::Object))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "Window.focusedLayer")?;
            let layer = v.object_handle().map(|l| rt.bound_this(l).unwrap_or(l));
            if let Some(l) = layer {
                if rt.host().layer(l)?.window != h.0 {
                    return Err(TjsError::runtime("focusedLayer must belong to this Window"));
                }
            }
            set_focus(rt, h, layer, true)
        },
    );
    methods(rt,class,&[("setLayerPos",|rt,t,a|{
        let h=this_of(rt,t,"Window.setLayerPos")?;
        let p=[need_int(&a,0,"Window.setLayerPos")?,need_int(&a,1,"Window.setLayerPos")?];
        rt.host_mut().set_layer_position(h.0,p)?;Ok(Variant::Void)
    })]);
    for (name,axis) in [("layerLeft",0),("layerTop",1)] {
        rt.register_object_native_property(class,name,
            move|rt,t|{let h=this_of(rt,t,"Window layer position")?;let w=rt.host().windows.get(&h.0).ok_or_else(||TjsError::runtime("Window not constructed"))?;Ok(Variant::Integer(w.layer_pos[axis]))},
            move|rt,t,v|{let h=this_of(rt,t,"Window layer position")?;let mut p=rt.host().windows.get(&h.0).ok_or_else(||TjsError::runtime("Window not constructed"))?.layer_pos;p[axis]=v.to_integer()?;rt.host_mut().set_layer_position(h.0,p)});
    }
    // Display zoom of the primary layer. The engine already presents the
    // canvas scaled to the host window; the ratio is recorded for scripts.
    methods(
        rt,
        class,
        &[("setZoom", |rt, t, a| {
            let h = this_of(rt, t, "Window.setZoom")?;
            let (n, d) = (
                need_int(&a, 0, "Window.setZoom")?,
                need_int(&a, 1, "Window.setZoom")?,
            );
            if !(1..=1_000_000).contains(&n) || !(1..=1_000_000).contains(&d) {
                return Err(TjsError::runtime("Window.setZoom ratio out of range"));
            }
            rt.set_object_member(h, "__zoomNumer", Variant::Integer(n));
            rt.set_object_member(h, "__zoomDenom", Variant::Integer(d));
            crate::d2d_view::sync_window_zoom(rt, h);
            Ok(Variant::Void)
        })],
    );
    for (name, key) in [("zoomNumer", "__zoomNumer"), ("zoomDenom", "__zoomDenom")] {
        rt.register_object_native_property(
            class,
            name,
            move |rt, t| {
                let h = this_of(rt, t, "Window.zoom")?;
                Ok(match rt.object_member(h, key) {
                    Variant::Void => Variant::Integer(1),
                    v => v,
                })
            },
            move |rt, t, v| {
                let h = this_of(rt, t, "Window.zoom")?;
                let n = v.to_integer()?;
                if !(1..=1_000_000).contains(&n) {
                    return Err(TjsError::runtime("Window zoom out of range"));
                }
                rt.set_object_member(h, key, Variant::Integer(n));
                crate::d2d_view::sync_window_zoom(rt, h);
                Ok(())
            },
        );
    }
    // Desktop placement belongs to the engine host window (as with setPos):
    // the client canvas is at the origin and writes do not move the window.
    for name in ["left", "top"] {
        rt.register_object_native_property(
            class,
            name,
            |_, _| Ok(Variant::Integer(0)),
            |_, _, v| {
                v.to_integer()?;
                Ok(())
            },
        );
    }
    rt.register_object_native_property(
        class,
        "caption",
        |rt, t| {
            let h = this_of(rt, t, "Window.caption")?;
            Ok(rt.object_member(h, "__caption"))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "Window.caption")?;
            let text = v.to_tjs_string()?;
            if is_output_window(rt,h) {rt.host_mut().ui_op(json!({"op":"set_title","text":text}));}
            rt.set_object_member(h, "__caption", Variant::String(text));
            Ok(())
        },
    );
    rt.register_object_native_property(
        class,
        "fullScreen",
        |rt, t| {
            let h = this_of(rt, t, "Window.fullScreen")?;
            Ok(Variant::Integer(
                rt.object_member(h, "__fullscreen").is_truthy() as i64,
            ))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "Window.fullScreen")?;
            let enabled = v.is_truthy();
            rt.host_mut().push_log(format!("Window.fullScreen = {enabled}"));
            if is_output_window(rt,h) {rt.host_mut()
                .ui_op(json!({"op":"set_fullscreen","enabled":enabled}));}
            rt.set_object_member(h, "__fullscreen", Variant::Integer(enabled as i64));
            Ok(())
        },
    );
    rt.set_object_member(class, "mainWindow", Variant::Null);
    rt.register_object_native_property(
        class,
        "borderStyle",
        |rt, t| {
            let h = this_of(rt, t, "Window.borderStyle")?;
            Ok(rt.object_member(h, "__borderStyle"))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "Window.borderStyle")?;
            let n = v.to_integer()?;
            let (decorated, resizable) = match n {
                0 => (false, false),
                1 | 3 => (true, false),
                2 | 4 | 5 => (true, true),
                _ => return Err(TjsError::runtime("unknown Window.borderStyle")),
            };
            rt.set_object_member(h, "__borderStyle", Variant::Integer(n));
            rt.host_mut().push_log(format!("Window.borderStyle = {n}"));
            if is_output_window(rt,h) {rt.host_mut().ui_op(
                json!({"op":"set_window_policy","decorated":decorated,"resizable":resizable}),
            );}
            Ok(())
        },
    );
    let set_inner: Native = |rt, this, a| {
        let h = this_of(rt, this, "Window.setInnerSize")?;
        let (w, height) = (
            need_int(&a, 0, "Window.setInnerSize")?,
            need_int(&a, 1, "Window.setInnerSize")?,
        );
        check_window_size(rt, w, height)?;
        window_mut(rt, h)?.inner = [w, height];
        Ok(Variant::Void)
    };
    methods(
        rt,
        class,
        &[
            ("onMouseDown", |_, _, _| Ok(Variant::Void)),
            ("onMouseUp", |_, _, _| Ok(Variant::Void)),
            ("onMouseMove", |_, _, _| Ok(Variant::Void)),
            ("onClick", |_, _, _| Ok(Variant::Void)),
            ("onKeyDown", |_, _, _| Ok(Variant::Void)),
            ("onKeyUp", |_, _, _| Ok(Variant::Void)),
            // krkrz Window base event handlers are empty.
            ("onResize", |_, _, _| Ok(Variant::Void)),
            ("onActivate", |_, _, _| Ok(Variant::Void)),
            ("onDeactivate", |_, _, _| Ok(Variant::Void)),
            ("onKeyPress", |_, _, _| Ok(Variant::Void)),
            ("onDoubleClick", |_, _, _| Ok(Variant::Void)),
            ("onMouseWheel", |_, _, _| Ok(Variant::Void)),
            ("onMouseEnter", |_, _, _| Ok(Variant::Void)),
            ("onMouseLeave", |_, _, _| Ok(Variant::Void)),
            ("onPopupHide", |_, _, _| Ok(Variant::Void)),
            ("onFileDrop", |_, _, _| Ok(Variant::Void)),
            ("onDisplayRotate", |_, _, _| Ok(Variant::Void)),
            ("onTouchDown", |_, _, _| Ok(Variant::Void)),
            ("onTouchUp", |_, _, _| Ok(Variant::Void)),
            ("onTouchMove", |_, _, _| Ok(Variant::Void)),
            ("onTouchScaling", |_, _, _| Ok(Variant::Void)),
            ("onTouchRotate", |_, _, _| Ok(Variant::Void)),
            ("onMultiTouch", |_, _, _| Ok(Variant::Void)),
            ("onHintChanged", |_, _, _| Ok(Variant::Void)),
            ("hideMouseCursor", |rt, t, _| {
                let h = this_of(rt, t, "Window.hideMouseCursor")?;
                rt.set_object_member(h, "__temporary_cursor_hide", Variant::Integer(1));
                if is_output_window(rt,h) {rt.host_mut()
                    .ui_op(json!({"op":"set_cursor_visible","visible":false}));}
                Ok(Variant::Void)
            }),
            // Hot keys are recorded per window. The host has no system-wide
            // OS hot keys; dispatch of onHotKeyPressed is not wired yet.
            ("registerHotKey", |rt, t, a| {
                let h = this_of(rt, t, "Window.registerHotKey")?;
                let id = need_int(&a, 0, "Window.registerHotKey")?;
                // `registerHotKey(id)` with no key removes the registration
                // (the game does so before screen-mode changes and on shutdown).
                if a.get(1).is_none_or(|v| matches!(v, Variant::Void)) {
                    if let Some(table) = rt.object_member(h, "__hotkeys").object_handle() {
                        rt.delete_object_member(table, &id.to_string());
                    }
                    return Ok(Variant::Integer(1));
                }
                let table = match rt.object_member(h, "__hotkeys").object_handle() {
                    Some(t) => t,
                    None => {
                        let d = rt.alloc_dictionary_object();
                        rt.set_object_member(h, "__hotkeys", Variant::Object(d));
                        d
                    }
                };
                let spec = rt.alloc_array_object(vec![
                    Variant::Integer(need_int(&a, 1, "Window.registerHotKey")?),
                    Variant::Integer(int(&a, 2, 0)?),
                ]);
                rt.set_object_member(table, id.to_string(), Variant::Object(spec));
                Ok(Variant::Integer(1))
            }),
            ("unregisterHotKey", |rt, t, a| {
                let h = this_of(rt, t, "Window.unregisterHotKey")?;
                let id = need_int(&a, 0, "Window.unregisterHotKey")?;
                if let Some(table) = rt.object_member(h, "__hotkeys").object_handle() {
                    rt.delete_object_member(table, &id.to_string());
                }
                Ok(Variant::Void)
            }),
            ("setInnerSize", set_inner),
            ("setSize", set_inner),
            // The desktop frontend owns the OS window; its position and
            // z-order are not script state.
            ("setPos", |_, _, _| Ok(Variant::Void)),
            ("bringToFront", |_, _, _| Ok(Variant::Void)),
            ("update", |_, _, _| Ok(Variant::Void)),
            ("add", |_, _, _| Ok(Variant::Void)),
            ("remove", |_, _, _| Ok(Variant::Void)),
            // krkrz TTVPWindowForm::Close (program close): `CanCloseWork`
            // starts true, onCloseQuery(true) runs immediately and its
            // `super.onCloseQuery(b)` decides; only an accepted query closes.
            ("close", |rt, t, _| {
                let h = this_of(rt, t, "Window.close")?;
                if crate::adapter::modal_window::contains(rt.host(),h.0) {
                    crate::adapter::modal_window::close(rt,h)?;
                    return Ok(Variant::Void);
                }
                if !is_output_window(rt,h) {
                    return Err(unsupported("secondary Window.close","per-window close-query ownership"));
                }
                // A program close supersedes an unanswered user close.
                rt.host_mut().closing_by_user = false;
                rt.host_mut().close_decision = true;
                rt.host_mut().program_close_pending = true;
                rt.call_object_method(h, "onCloseQuery", vec![Variant::Integer(1)])?;
                if !rt.is_suspended() && rt.host().modal_windows.is_empty() {
                    rt.host_mut().program_close_pending = false;
                    if rt.host().close_decision {
                        rt.host_mut().exit_requested = true;
                    }
                }
                Ok(Variant::Void)
            }),
            // The native default handler (`OnCloseQueryCalled`): a user close
            // closes on `true` and is cancelled on `false`; a program close
            // records the answer for `close()`.
            ("onCloseQuery", |rt, t, a| {
                let h=this_of(rt,t,"Window.onCloseQuery")?;
                let allow = a.first().is_some_and(Variant::is_truthy);
                if !is_output_window(rt,h) {
                    rt.set_object_member(h,"__closeDecision",Variant::Integer(allow as i64));
                    crate::adapter::modal_window::answer_close(rt,h,allow);
                    return Ok(Variant::Void);
                }
                let host = rt.host_mut();
                if host.closing_by_user {
                    host.closing_by_user = false;
                    if allow {
                        host.exit_requested = true;
                    }
                } else {
                    host.close_decision = allow;
                }
                Ok(Variant::Void)
            }),
            ("showModal", crate::adapter::modal_window::begin),
            // WindowIntf.cpp:750-751 declares an empty native finalize.
            // Engine resources are released separately by invalidate_object;
            // KAG's derived finalize invokes this base method after its own
            // layer/plugin cleanup.
            ("finalize", |_, _, _| Ok(Variant::Void)),
            // The host window stays rectangular; a shaped-window request has no state.
            ("setMaskRegion", |_, _, _| Ok(Variant::Void)),
        ],
    );
    let inner_w: Getter = |rt, t| Ok(Variant::Integer(window_of(rt, t)?.inner[0]));
    let inner_h: Getter = |rt, t| Ok(Variant::Integer(window_of(rt, t)?.inner[1]));
    let set_w: Setter = |rt, t, v| {
        let h = this_of(rt, t, "Window")?;
        let height = window_mut(rt, h)?.inner[1];
        let w = v.to_integer()?;
        check_window_size(rt, w, height)?;
        window_mut(rt, h)?.inner[0] = w;
        Ok(())
    };
    let set_h: Setter = |rt, t, v| {
        let h = this_of(rt, t, "Window")?;
        let w = window_mut(rt, h)?.inner[0];
        let height = v.to_integer()?;
        check_window_size(rt, w, height)?;
        window_mut(rt, h)?.inner[1] = height;
        Ok(())
    };
    properties(
        rt,
        class,
        &[
            (
                "menu",
                |rt, t| {
                    let menu = window_of(rt, t)?.menu;
                    Ok(menu
                        .map(|m| Variant::Object(ObjectHandle(m)))
                        .unwrap_or(Variant::Void))
                },
                read_only,
            ),
            ("innerWidth", inner_w, set_w),
            ("innerHeight", inner_h, set_h),
            ("width", inner_w, set_w),
            ("height", inner_h, set_h),
            (
                "visible",
                |rt, t| Ok(Variant::Integer(window_of(rt, t)?.visible as i64)),
                |rt, t, v| {
                    let h = this_of(rt, t, "Window")?;
                    if boolean(&v) && !is_output_window(rt,h) && !crate::adapter::modal_window::contains(rt.host(),h.0) {
                        return Err(unsupported("secondary Window.visible=true","multiple engine output windows"));
                    }
                    let w = window_mut(rt, h)?;
                    w.visible = boolean(&v);
                    let mut roots = Vec::new();
                    w.append_roots(&mut roots);
                    for p in roots {
                        if let Some(l) = rt.host_mut().layers.get_mut(&p) {
                            l.dirty = true;
                        }
                    }
                    if !boolean(&v) {
                        if let Some(modal)=rt.host_mut().modal_windows.iter_mut().find(|m|m.window==h.0){modal.closed=true;}
                    }
                    Ok(())
                },
            ),
            (
                "primaryLayer",
                |rt, t| {
                    Ok(window_of(rt, t)?
                        .primary
                        .map(|p| Variant::self_bound(ObjectHandle(p)))
                        .unwrap_or(Variant::Null))
                },
                read_only,
            ),
        ],
    );
}

fn is_output_window(rt:&Rt,h:ObjectHandle)->bool {
    rt.host().windows.get(&h.0).is_some_and(|window|window.output)
}

/// The inner size is script state for the displayed size. The desktop host owns
/// the OS window and presents the project canvas scaled and letterboxed into it,
/// so a script changing width and height one at a time passes through any ratio.
fn check_window_size(_rt: &Rt, w: i64, h: i64) -> Result<()> {
    if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
        return Err(unsupported(&format!("Window inner size {w}x{h}"), "1..=16384 pixels"));
    }
    Ok(())
}

fn window_of(rt: &Rt, t: Option<ObjectHandle>) -> Result<WindowState> {
    let h = this_of(rt, t, "Window")?;
    rt.host()
        .windows
        .get(&h.0)
        .cloned()
        .ok_or_else(|| TjsError::runtime("Window is not constructed"))
}

fn window_mut(rt: &mut Rt, h: ObjectHandle) -> Result<&mut WindowState> {
    rt.host_mut()
        .windows
        .get_mut(&h.0)
        .ok_or_else(|| TjsError::runtime("Window is not constructed"))
}

// ------------------------------------------------------------------ Layer

/// Default `Font.height` (pixels) of a new layer.
pub const DEFAULT_FONT_HEIGHT: i64 = 24;

/// Font members read through PropGet: scripts may replace them with
/// properties (PreRenderFontEx-style font wrappers).
/// krkr `Font` members are native properties (scripts take `&font.face`
/// and wrap it, e.g. Yuzusoft's objectHookInjection), backed per object.
const FONT_SLOTS: &[(&str, &str)] = &[
    ("face", "__font_face"),
    ("height", "__font_height"),
    ("bold", "__font_bold"),
    ("italic", "__font_italic"),
    ("angle", "__font_angle"),
    ("underline", "__font_underline"),
    ("strikeout", "__font_strikeout"),
    ("faceIsFileName", "__font_faceIsFileName"),
];

fn font_properties(rt: &mut Rt, font: ObjectHandle) {
    for (name, slot) in FONT_SLOTS {
        let initial = match *name {
            "face" => Variant::String(String::new()),
            "height" => Variant::Integer(DEFAULT_FONT_HEIGHT),
            _ => Variant::Integer(0),
        };
        rt.set_object_member(font, *slot, initial);
        let (get_slot, set_slot) = (*slot, *slot);
        rt.register_object_native_property(
            font,
            *name,
            move |rt: &mut Rt, this: Option<ObjectHandle>| {
                let this = this.ok_or_else(|| TjsError::runtime("Font property without object"))?;
                rt.resolve_object_member(this, get_slot)
            },
            move |rt: &mut Rt, this: Option<ObjectHandle>, value: Variant| {
                let this = this.ok_or_else(|| TjsError::runtime("Font property without object"))?;
                rt.set_object_member(this, set_slot, value);
                Ok(())
            },
        );
    }
}

pub(crate) fn font_size(rt: &mut Rt, font: ObjectHandle) -> Result<u64> {
    for property in ["angle"] {
        if rt.resolve_object_member(font, property)?.to_integer()? != 0 {
            return Err(unsupported(
                &format!("Font.{property}"),
                "matching engine canvas text style and metrics",
            ));
        }
    }
    font_metric_size(rt, font)
}

fn font_metric_size(rt: &mut Rt, font: ObjectHandle) -> Result<u64> {
    if rt.resolve_object_member(font, "italic")?.to_integer()? != 0 {
        return Err(unsupported("Font.italic", "matching engine canvas text style and metrics"));
    }
    let size = rt
        .resolve_object_member(font, "height")?
        .to_integer()?
        .unsigned_abs();
    if !(1..=512).contains(&size) {
        return Err(TjsError::runtime("Font.height must have magnitude 1..512"));
    }
    Ok(size)
}

fn font_text_width(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let font = this_of(rt, t, "Font.getTextWidth")?;
    let text = string(&a, 0, "Font.getTextWidth")?;
    let size = font_metric_size(rt, font)?;
    let alias=crate::font_resources::alias(rt,font)?;
    let raster = crate::font_resources::raster_size(rt, font, &alias, size)?;
    let host = rt.host();
    let metrics = host
        .text_measurement
        .as_ref()
        .ok_or_else(|| TjsError::runtime("engine text measurement handle absent"))?
        .measure_simple(&alias, raster.measure, &text)
        .map_err(|e| TjsError::runtime(format!("Font.getTextWidth: {e}")))?;
    Ok(Variant::Integer(metrics.advance.round() as i64))
}

fn font_text_height(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let font = this_of(rt, t, "Font.getTextHeight")?;
    // KiriKiri returns abs(Font.height), even for the empty string. It does
    // not return the glyph ink box or the engine's line pitch.
    let _ = string(&a, 0, "Font.getTextHeight")?;
    let height = rt.resolve_object_member(font, "height")?.to_integer()?.checked_abs()
        .ok_or_else(|| TjsError::runtime("Font.height magnitude overflow"))?;
    Ok(Variant::Integer(height))
}

/// Native Layer default focus handlers forward event dictionaries to the
/// constructor's Window action owner. Missing action is the native default.
fn layer_focus_action(rt:&mut Rt,this:Option<ObjectHandle>,args:Vec<Variant>,focus:bool)->Result<Variant> {
    let layer=this_of(rt,this,"Layer focus event")?;
    let owner=ObjectHandle(rt.host().layer(layer)?.window);
    let fields:&[&str]=if focus {&["blurred","direction"]}else{&["focused"]};
    if args.len()<fields.len() {return Err(TjsError::bad_param_count());}
    if !has_member(rt,owner,"action") {return Ok(Variant::Void);}
    let event=rt.alloc_dictionary_object();
    rt.set_object_member(event,"type",Variant::String(if focus {"onFocus"}else{"onBlur"}.into()));
    rt.set_object_member(event,"target",Variant::self_bound(layer));
    for (name,value) in fields.iter().zip(args) {rt.set_object_member(event,*name,value);}
    rt.call_object_method(owner,"action",vec![Variant::self_bound(event)])
}

// Native Layer instances exist before a derived script constructor runs
// (tjsNative::FuncCall). Some scripts construct their child before their own
// named Layer constructor. Keep a bounded, invisible default instance and bind
// the tree when that constructor arrives. In particular, never dereference a
// not-yet-bound layer manager or invent a primary layer for it.
fn layer_instance_defaults(rt: &mut Rt, h: ObjectHandle, window: ObjectHandle) -> Result<()> {
    for key in ["__preLeft", "__preTop"] {
        if let Variant::Integer(x) = rt.object_member(h, key) {
            let l = rt.host_mut().layer_mut(h)?;
            if key == "__preLeft" { l.left = x; } else { l.top = x; }
            rt.delete_object_member(h, key);
        }
    }
    let font = rt.alloc_ordinary_object();
    font_class::initialize(rt, font);
    // Self-bound like krkr (`tTJSVariant(dsp, dsp)`): writes through
    // `layer.font.x` act on the Font, not the caller (property hooks).
    rt.set_object_member(h, "__crysirror_native_font", Variant::self_bound(font));
    rt.set_object_member(h, "__window", Variant::self_bound(window));
    rt.set_object_member(h, "name", Variant::String(String::new()));
    for (k, v) in [
        ("__lp_hitThreshold", 16),
        ("__lp_hitType", 0),
        ("__lp_showParentHint", 1),
        ("__lp_focusable", 0),
        ("__lp_enabled", 1),
        ("__lp_face", 128),
        ("__lp_holdAlpha", 0),
        ("__lp_neutralColor", 0),
        ("__lp_joinFocusChain", 1),
    ] {
        rt.set_object_member(h, k, Variant::Integer(v));
    }
    Ok(())
}
pub(crate) fn layer_font(rt: &Rt, layer: ObjectHandle) -> Result<ObjectHandle> {
    rt.object_member(layer, "__crysirror_native_font").object_handle()
        .ok_or_else(|| TjsError::runtime("Layer.font requires a constructed Layer"))
}

fn install_layer(rt: &mut Rt) {
    let class = native_class(rt, "Layer", |rt, this, a| {
        let h = this_of(rt, this, "Layer")?;
        if a.is_empty() {
            if !rt.host().layer_instances.contains(&h.0)
                && rt.host().layer_instances.len() >= crate::host::MAX_LAYERS as usize {
                return Err(TjsError::runtime("native Layer instance limit"));
            }
            rt.host_mut().layer_instances.insert(h.0);
            return Ok(Variant::Void);
        }
        if rt.host().layers.get(&h.0).is_some_and(|l| l.constructed) {
            return Ok(Variant::Void);
        }
        let window = object_arg(&a[0])
            .map(|w| rt.bound_this(w).unwrap_or(w))
            .filter(|w| rt.host().windows.contains_key(&w.0))
            .ok_or_else(|| TjsError::runtime("Layer: first argument must be a Window"))?;
        let parent = match a.get(1) {
            None | Some(Variant::Void | Variant::Null) => None,
            Some(v) => {
                let parent=object_arg(v).map(|p|rt.bound_this(p).unwrap_or(p))
                    .ok_or_else(||TjsError::runtime("Layer: parent must be a Layer"))?;
                if !rt.host().layers.contains_key(&parent.0) {
                    if !rt.object_valid(parent) || !rt.host().layer_instances.contains(&parent.0) {
                        return Err(TjsError::runtime("Layer: parent must be a Layer"));
                    }
                    rt.host_mut().reserve_layer(parent,window.0)?;
                    layer_instance_defaults(rt,parent,window)?;
                }
                let state=rt.host().layer(parent)?;
                if state.window!=window.0 || !state.attached || parent==h {
                    return Err(TjsError::runtime("Layer: invalid parent scope or cycle"));
                }
                Some(parent.0)
            }
        };
        let additional = parent.is_none() && rt.host().windows[&window.0].primary.is_some();
        if additional && !crate::d2d_view::supported(rt, window) {
            return Err(TjsError::runtime("Window already has a primary layer"));
        }
        if rt.host().layers.contains_key(&h.0) {
            rt.host_mut().bind_reserved_layer(h,window.0,parent)?;
        } else {
            rt.host_mut().create_layer(h,window.0,parent)?;
            layer_instance_defaults(rt,h,window)?;
        }
        if parent.is_none() {
            let [w,height]=rt.host().windows[&window.0].inner;
            if additional {
                crate::d2d_view::attach(rt, window, h);
            } else {
                rt.host_mut().windows.get_mut(&window.0).unwrap().primary=Some(h.0);
            }
            let l=rt.host_mut().layer_mut(h)?;
            l.width=w;l.height=height;
            if additional { l.visible = true; }
            rt.host_mut().reset_surface(h,w,height,[0,0,0,255])?;
            rt.host_mut().layer_mut(h)?.kind=1;
        } else {
            rt.host_mut().layer_mut(h)?.kind=2;
        }
        Ok(Variant::Void)
    });
    // krkrz LayerIntf.cpp: the base accessor returns this layer's Font
    // instance. A script can replace its own member using &layer.font; the
    // native bitmap and a base-qualified read still use the owned instance.
    rt.register_object_native_property(class, "font",
        |rt, t| Ok(Variant::self_bound(layer_font(rt, this_of(rt, t, "Layer.font")?)?)),
        read_only);
    rt.register_object_native_property(class, "type",
        |rt, t| { let h = this_of(rt,t,"Layer.type")?; Ok(Variant::Integer(rt.host().layers.get(&h.0).map_or(2, |l| l.kind))) },
        |rt, t, value| {
            let h = this_of(rt,t,"Layer.type")?;
            let kind = value.to_integer()?;
            if !(0..=31).contains(&kind) { return Err(TjsError::runtime("invalid Layer.type")); }
            let l = rt.host_mut().layer_mut(h)?;
            l.kind = kind; l.dirty = true; l.presented = None;
            Ok(())
        });
    // Plain-state native properties (krkrz Layer). Values live in hidden
    // per-instance slots so property references (`&Layer.enabled`) resolve to
    // a real property, as scripts that call super setters require.
    for name in [
        "hitThreshold",
        "hitType",
        "showParentHint",
        "focusable",
        "enabled",
        "face",
        "holdAlpha",
        "neutralColor",
        "callOnPaint",
        "joinFocusChain",
        "imeMode",
        "useAttention",
        "attentionLeft",
        "attentionTop",
        "ignoreHintSensing",
    ] {
        let slot: &'static str = Box::leak(format!("__lp_{name}").into_boxed_str());
        rt.register_object_native_property(
            class,
            name,
            move |rt, t| {
                let h = this_of(rt, t, "Layer")?;
                Ok(match rt.object_member(h, slot) {
                    Variant::Void => Variant::Integer(0),
                    v => v,
                })
            },
            move |rt, t, v| {
                let h = this_of(rt, t, "Layer")?;
                let v = Variant::Integer(v.to_integer()?);
                if slot != "__lp_enabled" {
                    rt.set_object_member(h, slot, v);
                    return Ok(());
                }
                // krkrz SetEnabled: a change notifies node-enabled states, and
                // disabling runs BlurTree, which drops modal state in the tree.
                let enable = v.is_truthy();
                if rt.object_member(h, slot).is_truthy() == enable {
                    rt.set_object_member(h, slot, v);
                    return Ok(());
                }
                let before = node_enabled_states(rt);
                rt.set_object_member(h, slot, v);
                if !enable {
                    crate::native_mouse::leave_tree(rt, h.0, false)?;
                    remove_tree_modal_state(rt, h);
                }
                notify_node_enabled(rt, before)
            },
        );
    }
    methods(
        rt,
        class,
        &[
            ("onHitTest", |rt, t, a| {
                let h = this_of(rt, t, "Layer.onHitTest")?;
                let hit = a
                    .get(2)
                    .ok_or_else(|| TjsError::runtime("onHitTest needs a hit flag"))?
                    .is_truthy();
                rt.set_object_member(h, "__hit_test_work", Variant::Integer(hit as i64));
                Ok(Variant::Void)
            }),
            ("onMouseDown", |_, _, _| Ok(Variant::Void)),
            ("onMouseUp", |_, _, _| Ok(Variant::Void)),
            ("onMouseMove", |_, _, _| Ok(Variant::Void)),
            ("onClick", |_, _, _| Ok(Variant::Void)),
            ("onPaint", |_, _, _| Ok(Variant::Void)),
            // krkrz default handlers of the modal-mode notifications.
            ("onMouseEnter", |_, _, _| Ok(Variant::Void)),
            ("onMouseLeave", |_, _, _| Ok(Variant::Void)),
            ("onBlur", |rt,t,a| layer_focus_action(rt,t,a,false)),
            ("onFocus", |rt,t,a| layer_focus_action(rt,t,a,true)),
            ("releaseCapture", |rt, t, _| {
                this_of(rt, t, "Layer.releaseCapture")?;
                crate::native_mouse::release(rt);
                Ok(Variant::Void)
            }),
            ("onNodeEnabled", |_, _, _| Ok(Variant::Void)),
            ("onNodeDisabled", |_, _, _| Ok(Variant::Void)),
            // Pixel representation conversion (from a draw face to this layer's
            // type). The adapter stores every layer canonically as straight
            // alpha composited normally, which renders ltAlpha and ltAddAlpha
            // identically to krkr; converting between dfAlpha (0) / dfAddAlpha
            // (4) and ltAlpha (2) / ltAddAlpha (12), or from opaque data, is
            // therefore the identity here. Conversion to opaque (forcing alpha)
            // needs a shared pixel transform and fails visibly.
            ("convertType", |rt, t, a| {
                let h = this_of(rt, t, "Layer.convertType")?;
                let from = need_int(&a, 0, "Layer.convertType")?;
                let to = rt.host().layer(h)?.kind;
                if matches!(from, 0 | 1 | 4) && matches!(to, 2 | 12) {
                    return Ok(Variant::Void);
                }
                Err(unsupported(
                    &format!("Layer.convertType({from}) on type {to}"),
                    "shared premultiplied/opaque pixel conversion",
                ))
            }),
            ("getLayerAt", layer_get_layer_at),
            ("setPos", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setPos")?;
                let (x, y) = (
                    need_int(&a, 0, "Layer.setPos")?,
                    need_int(&a, 1, "Layer.setPos")?,
                );
                let (w, ht) = (int(&a, 2, -1)?, int(&a, 3, -1)?);
                let l = rt.host_mut().layer_mut(h)?;
                (l.left, l.top) = (x, y);
                if w >= 0 && ht >= 0 {
                    (l.width, l.height) = (w, ht);
                }
                l.dirty = true;
                grow_image(rt, h)?;
                Ok(Variant::Void)
            }),
            ("setSize", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setSize")?;
                let (w, ht) = (
                    need_int(&a, 0, "Layer.setSize")?,
                    need_int(&a, 1, "Layer.setSize")?,
                );
                let l = rt.host_mut().layer_mut(h)?;
                (l.width, l.height) = (w.max(0), ht.max(0));
                l.dirty = true;
                grow_image(rt, h)?;
                Ok(Variant::Void)
            }),
            ("setImagePos", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setImagePos")?;
                let l = rt.host_mut().layer_mut(h)?;
                (l.image_left, l.image_top) = (
                    need_int(&a, 0, "Layer.setImagePos")?,
                    need_int(&a, 1, "Layer.setImagePos")?,
                );
                l.dirty = true;
                Ok(Variant::Void)
            }),
            ("setImageSize", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setImageSize")?;
                let (w, ht) = (
                    need_int(&a, 0, "Layer.setImageSize")?,
                    need_int(&a, 1, "Layer.setImageSize")?,
                );
                set_image_size(rt, h, w, ht)?;
                Ok(Variant::Void)
            }),
            ("setSizeToImageSize", |rt, t, _| {
                let h = this_of(rt, t, "Layer.setSizeToImageSize")?;
                let l = rt.host_mut().layer_mut(h)?;
                (l.width, l.height) = (l.image_width, l.image_height);
                l.dirty = true;
                Ok(Variant::Void)
            }),
            ("getProvincePixel", |rt, t, a| {
                let h = this_of(rt, t, "Layer.getProvincePixel")?;
                let v = rt
                    .host()
                    .layer(h)?
                    .regions
                    .get(
                        rt.host().client,
                        [
                            need_int(&a, 0, "province x")?,
                            need_int(&a, 1, "province y")?,
                        ],
                    )
                    .map_err(|e| TjsError::runtime(e.to_string()))?;
                Ok(Variant::Integer(v as i64))
            }),
            // krkrz tTVPLayerManager::SetModeTo / RemoveModeFrom: input and
            // focus are limited to the current modal layer's subtree; layers
            // outside it hit-test as disabled (blocking, not passing through).
            ("removeMode", |rt, t, _| {
                let h = this_of(rt, t, "Layer.removeMode")?;
                rt.host().layer(h)?;
                if std::env::var_os("CRYSIRROR_KRKR_TRACE_MODE").is_some() {
                    let name = rt.object_member(h, "name").to_tjs_string().unwrap_or_default();
                    rt.host_mut().push_log(format!("[mode] removeMode {name}#{}", h.0));
                }
                if !rt.host().modal_layers.contains(&h.0) {
                    return Ok(Variant::Void);
                }
                let before = node_enabled_states(rt);
                rt.host_mut().modal_layers.retain(|l| *l != h.0);
                notify_node_enabled(rt, before)?;
                Ok(Variant::Void)
            }),
            ("setMode", |rt, t, _| {
                let h = this_of(rt, t, "Layer.setMode")?;
                if std::env::var_os("CRYSIRROR_KRKR_TRACE_MODE").is_some() {
                    let name = rt.object_member(h, "name").to_tjs_string().unwrap_or_default();
                    let stack: Vec<String> = rt.host().modal_layers.iter().map(|l| {
                        rt.object_member(ObjectHandle(*l), "name").to_tjs_string().unwrap_or_default() + "#" + &l.to_string()
                    }).collect();
                    rt.host_mut().push_log(format!("[mode] setMode {name}#{} stack={stack:?}", h.0));
                }
                let refuse = |why: &str| {
                    TjsError::runtime(format!("Layer.setMode: cannot set mode to a disabled or modal layer ({why})"))
                };
                if let Some(current) = rt.host().modal_layers.last().copied() {
                    if rt.host().is_ancestor_or_self(h.0, current) {
                        return Err(refuse("inside the current modal layer"));
                    }
                }
                if !rt.host().layer(h)?.visible {
                    let l = rt.host_mut().layer_mut(h)?;
                    l.visible = true;
                    l.dirty = true;
                }
                if !layer_tree_visible(rt, h)? {
                    return Err(refuse("a parent is invisible"));
                }
                if !layer_node_enabled_plain(rt, h)? {
                    return Err(refuse("disabled"));
                }
                let before = node_enabled_states(rt);
                rt.host_mut().modal_layers.push(h.0);
                let window = ObjectHandle(rt.host().layer(h)?.window);
                let first = first_focusable(rt, h)?;
                set_focus(rt, window, first, true)?;
                notify_node_enabled(rt, before)?;
                Ok(Variant::Void)
            }),
            ("focus", |rt, t, a| {
                let h = this_of(rt, t, "Layer.focus")?;
                let layer = rt.host().layer(h)?;
                if !rt.object_member(h, "__lp_focusable").is_truthy() || !layer.visible {
                    return Ok(Variant::Integer(0));
                }
                let forward = a.first().is_none_or(|v| v.is_truthy());
                set_focus(rt, ObjectHandle(layer.window), Some(h), forward)?;
                Ok(Variant::Integer(1))
            }),
            ("freeImage", |rt, t, _| {
                let h = this_of(rt, t, "Layer.freeImage")?;
                let slot = rt.host().layer(h)?.slot;
                rt.host_mut()
                    .canvas_op(json!({"op":"clear","id":slot,"color":[0,0,0,0]}));
                rt.host_mut().layer_mut(h)?.blank = true;
                rt.host_mut().layer_mut(h)?.has_image = false;
                Ok(Variant::Void)
            }),
            ("loadImages", layer_load_images),
            ("loadProvinceImage", crate::province::load),
            ("adjustGamma", crate::image_api::gamma),
            ("clipAlphaRect",crate::image_api::clip_alpha),
            ("copyRightBlueToLeftAlpha", crate::layer_ex::copy_right_blue_to_left_alpha),
            ("copyBottomBlueToTopAlpha", crate::layer_ex::copy_bottom_blue_to_top_alpha),
            ("fillAlpha", crate::layer_ex::fill_alpha),
            ("overwrapRect", crate::layer_ex::overwrap_rect),
            ("shrinkCopy", crate::layer_ex::shrink_copy),
            ("shrinkCopyFast", crate::layer_ex::shrink_copy_fast),
            ("copyAlphaToProvince", crate::province_ex::copy_alpha_to_province),
            ("fillToProvince", crate::province_ex::fill_to_province),
            ("fillByProvince", crate::province_ex::fill_by_province),
            // krkrz tTJSNI_BaseLayer::DoBoxBlur (LayerIntf.cpp): a box blur
            // of the clip rect over (-xblur,-yblur)..(xblur,yblur); the alpha
            // face (dfAlpha, or dfAuto on an alpha layer) weights colour by
            // alpha (DoBoxBlurForAlpha).
            ("doBoxBlur", |rt, t, a| {
                const M: &str = "Layer.doBoxBlur";
                let h = this_of(rt, t, M)?;
                let arg = |i: usize| -> Result<i64> {
                    match a.get(i) {
                        None | Some(Variant::Void) => Ok(1),
                        Some(v) => v.to_integer(),
                    }
                };
                let (x, y) = (arg(0)?.abs().min(8192) as i32, arg(1)?.abs().min(8192) as i32);
                if x == 0 && y == 0 {
                    return Ok(Variant::Void);
                }
                let face = raw_draw_face(rt, h);
                let l = rt.host().layer(h)?.clone();
                let alpha_weighted = match face {
                    0 => true,
                    128 => l.kind != 1,
                    _ => {
                        return Err(unsupported(&format!("{M} with face={face}"), "per-channel box blur faces"));
                    }
                };
                let [cx, cy, cw, ch] = clip_of(&l);
                if cw <= 0 || ch <= 0 {
                    return Ok(Variant::Void);
                }
                crate::image_api::begin(
                    rt,
                    Some(h),
                    json!({"kind":"box_blur","rect":[cx.max(0),cy.max(0),cw,ch],
                           "area":[-x,-y,x,y],"alpha_weighted":alpha_weighted}),
                )
            }),
            ("doGrayScale", |rt, t, _| {
                crate::image_api::begin(rt, t, json!({"kind":"grayscale"}))
            }),
            ("flipLR", |rt, t, _| {
                crate::image_api::begin(
                    rt,
                    t,
                    json!({"kind":"flip","horizontal":true,"vertical":false}),
                )
            }),
            ("flipUD", |rt, t, _| {
                crate::image_api::begin(
                    rt,
                    t,
                    json!({"kind":"flip","horizontal":false,"vertical":true}),
                )
            }),
            ("fillRect", layer_fill_rect),
            ("colorRect", layer_color_rect),
            ("copyRect", |rt, t, a| layer_copy(rt, t, a, true)),
            ("operateRect", |rt, t, a| layer_copy(rt, t, a, false)),
            ("assignImages", layer_assign_images),
            ("drawText", layer_draw_text),
            ("bringToFront", |rt, t, _| {
                restack(rt, t, |c, i| {
                    let h = c.remove(i);
                    c.push(h);
                })
            }),
            ("sendToBack", |rt, t, _| {
                restack(rt, t, |c, i| {
                    let h = c.remove(i);
                    c.insert(0, h);
                })
            }),
            // Retained engine layers redraw automatically.
            // krkrz `update` (UpdateByScript) raises CallOnPaint, so onPaint
            // fires before the next draw; the region is engine-redrawn anyway.
            ("update", |rt, t, a| {
                let h = this_of(rt, t, "Layer.update")?;
                if !a.is_empty() && a.len() < 4 {
                    return Err(TjsError::bad_param_count());
                }
                if rt.host().layers.contains_key(&h.0) {
                    rt.set_object_member(h, "__lp_callOnPaint", Variant::Integer(1));
                }
                Ok(Variant::Void)
            }),
            ("finalize", |_, _, _| Ok(Variant::Void)),
            // Drawing clip for later draw operations; no arguments resets it.
            ("setClip", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setClip")?;
                let l = rt.host_mut().layer_mut(h)?;
                l.draw_clip = if a.is_empty() {
                    None
                } else {
                    Some([
                        int(&a, 0, 0)?,
                        int(&a, 1, 0)?,
                        int(&a, 2, l.image_width)?,
                        int(&a, 3, l.image_height)?,
                    ])
                };
                Ok(Variant::Void)
            }),
            ("piledCopy", crate::piled_copy::copy),
            ("stretchCopy", layer_stretch_copy),
            ("affineCopy", layer_affine_copy),
            ("operateStretch", layer_operate_stretch),
            ("operateAffine", layer_operate_affine),
            ("getMainPixel", layer_get_main_pixel),
            // krkrz SetMainPixel (LayerIntf.cpp:2594): inside the clip rect,
            // write the colour channels of one pixel and keep its alpha
            // (SetPointMain) -- a 1x1 keep-alpha canvas fill.
            ("setMainPixel", |rt, t, a| {
                const M: &str = "Layer.setMainPixel";
                let h = this_of(rt, t, M)?;
                let (x, y) = (need_int(&a, 0, M)?, need_int(&a, 1, M)?);
                let color = argb(need_int(&a, 2, M)?, Some(255));
                let l = rt.host().layer(h)?.clone();
                if !l.has_image {
                    return Err(TjsError::runtime("Not drawable layer type"));
                }
                if x < 0 || y < 0 || x >= l.image_width || y >= l.image_height {
                    return Err(TjsError::runtime(format!("{M}: point out of the image")));
                }
                if clip_rect(&l, [x, y, 1, 1]).is_none() || l.blank {
                    // A transparent pixel stays invisible; its hidden colour
                    // is not retained by the canvas.
                    return Ok(Variant::Void);
                }
                let host = rt.host_mut();
                host.canvas_op(json!({"op":"fill","id":l.slot,"rect":[x,y,1,1],"color":color,"keep_alpha":true}));
                host.layer_mut(h)?.image_source = None;
                host.api_calls += 1;
                Ok(Variant::Void)
            }),
            ("saveLayerImage", crate::image_api::save_layer_image),
        ],
    );
    properties(
        rt,
        class,
        &[
            (
                "nodeVisible",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.nodeVisible")?;
                    let mut node = Some(h.0);
                    let mut depth = 0;
                    while let Some(id) = node {
                        let l = rt.host().layer(ObjectHandle(id))?;
                        if !l.visible {
                            return Ok(Variant::Integer(0));
                        }
                        node = l.parent;
                        depth += 1;
                        if depth > 64 {
                            return Err(TjsError::runtime("layer visibility ancestry limit"));
                        }
                    }
                    Ok(Variant::Integer(1))
                },
                read_only,
            ),
            (
                "nodeEnabled",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.nodeEnabled")?;
                    Ok(Variant::Integer(layer_node_enabled(rt, h)? as i64))
                },
                read_only,
            ),
            (
                "clipLeft",
                |rt, t| clip_get(rt, t, 0),
                |rt, t, v| clip_set(rt, t, 0, v),
            ),
            (
                "clipTop",
                |rt, t| clip_get(rt, t, 1),
                |rt, t, v| clip_set(rt, t, 1, v),
            ),
            (
                "clipWidth",
                |rt, t| clip_get(rt, t, 2),
                |rt, t, v| clip_set(rt, t, 2, v),
            ),
            (
                "clipHeight",
                |rt, t| clip_get(rt, t, 3),
                |rt, t, v| clip_set(rt, t, 3, v),
            ),
            (
                "focused",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.focused")?;
                    let window = ObjectHandle(rt.host().layer(h)?.window);
                    Ok(Variant::Integer((focused_layer(rt, window) == Some(h)) as i64))
                },
                read_only,
            ),
            (
                "children",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.children")?;
                    let ids = rt.host().layer(h)?.children.clone();
                    let items = ids
                        .into_iter()
                        .map(|c| Variant::self_bound(ObjectHandle(c)))
                        .collect();
                    Ok(Variant::Object(rt.alloc_array_object(items)))
                },
                read_only,
            ),
            (
                "numChildren",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.numChildren")?;
                    Ok(Variant::Integer(rt.host().layer(h)?.children.len() as i64))
                },
                read_only,
            ),
            (
                "left",
                |rt, t| {
                    pre_get(rt, t, "__preLeft").map_or_else(|| layer_get(rt, t, |l| l.left), Ok)
                },
                |rt, t, v| match pre_set(rt, t, &v, "__preLeft")? {
                    true => Ok(()),
                    false => layer_set(rt, t, v, |l, x| l.left = x),
                },
            ),
            (
                "top",
                |rt, t| pre_get(rt, t, "__preTop").map_or_else(|| layer_get(rt, t, |l| l.top), Ok),
                |rt, t, v| match pre_set(rt, t, &v, "__preTop")? {
                    true => Ok(()),
                    false => layer_set(rt, t, v, |l, x| l.top = x),
                },
            ),
            (
                "width",
                |rt, t| layer_get(rt, t, |l| l.width),
                |rt, t, v| {
                    layer_set(rt, t, v, |l, x| l.width = x.max(0))?;
                    grow_image(rt, this_of(rt, t, "Layer.width")?)
                },
            ),
            (
                "height",
                |rt, t| layer_get(rt, t, |l| l.height),
                |rt, t, v| {
                    layer_set(rt, t, v, |l, x| l.height = x.max(0))?;
                    grow_image(rt, this_of(rt, t, "Layer.height")?)
                },
            ),
            (
                "imageLeft",
                |rt, t| layer_get(rt, t, |l| l.image_left),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.image_left = x),
            ),
            (
                "imageTop",
                |rt, t| layer_get(rt, t, |l| l.image_top),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.image_top = x),
            ),
            (
                "hint",
                |rt, t| {
                    let h = this_of(rt,t,"Layer.hint")?;
                    Ok(Variant::String(rt.host().layer(h)?.hint.clone()))
                },
                |rt,t,v| { let h=this_of(rt,t,"Layer.hint")?; crate::tooltip::set(rt,h,v) },
            ),
            (
                "showParentHint",
                |rt,t| { let h=this_of(rt,t,"Layer.showParentHint")?; Ok(Variant::Integer(i64::from(rt.host().layer(h)?.show_parent_hint))) },
                |rt,t,v| { let h=this_of(rt,t,"Layer.showParentHint")?; rt.host_mut().layer_mut(h)?.show_parent_hint=v.is_truthy(); Ok(()) },
            ),
            (
                "ignoreHintSensing",
                |rt,t| { let h=this_of(rt,t,"Layer.ignoreHintSensing")?; Ok(Variant::Integer(i64::from(rt.host().layer(h)?.ignore_hint_sensing))) },
                |rt,t,v| { let h=this_of(rt,t,"Layer.ignoreHintSensing")?; rt.host_mut().layer_mut(h)?.ignore_hint_sensing=v.is_truthy(); Ok(()) },
            ),
            (
                "imageModified",
                |rt, t| layer_get(rt, t, |l| l.image_modified as i64),
                |rt, t, v| {
                    let h = this_of(rt, t, "imageModified")?;
                    rt.host_mut().layer_mut(h)?.image_modified = v.is_truthy();
                    Ok(())
                },
            ),
            (
                "hasImage",
                |rt, t| layer_get(rt, t, |l| l.has_image as i64),
                |rt, t, v| {
                    let h = this_of(rt, t, "hasImage")?;
                    let l = rt.host().layer(h)?.clone();
                    if v.is_truthy() {
                        if !l.has_image {
                            // AllocateImage uses the current Layer rectangle. Its
                            // old bitmap may have been freed before setSize().
                            rt.host_mut().reset_surface(
                                h,
                                l.width.max(1),
                                l.height.max(1),
                                [255, 255, 255, 0],
                            )?;
                            let layer = rt.host_mut().layer_mut(h)?;
                            layer.image_left = 0;
                            layer.image_top = 0;
                        }
                    } else {
                        rt.host_mut()
                            .canvas_op(json!({"op":"clear","id":l.slot,"color":[0,0,0,0]}));
                        rt.host_mut().layer_mut(h)?.has_image = false;
                    }
                    Ok(())
                },
            ),
            (
                "imageWidth",
                |rt, t| layer_get(rt, t, |l| l.image_width),
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer")?;
                    let ht = rt.host().layer(h)?.image_height;
                    set_image_size(rt, h, v.to_integer()?, ht)
                },
            ),
            (
                "imageHeight",
                |rt, t| layer_get(rt, t, |l| l.image_height),
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer")?;
                    let w = rt.host().layer(h)?.image_width;
                    set_image_size(rt, h, w, v.to_integer()?)
                },
            ),
            (
                "visible",
                |rt, t| layer_get(rt, t, |l| l.visible as i64),
                |rt, t, v| {
                    let show = boolean(&v);
                    let h = this_of(rt, t, "Layer.visible")?;
                    let was = rt.host().layer(h)?.visible;
                    layer_set(rt, t, Variant::Integer(show as i64), |l, x| {
                        l.visible = x != 0
                    })?;
                    // krkrz SetVisible(false) runs BlurTree (modal state in
                    // the hidden tree is removed, with node-enabled notices).
                    if was && !show { crate::native_mouse::leave_tree(rt, h.0, false)?; }
                    if !show && rt.host().modal_layers.iter().any(|m| rt.host().is_ancestor_or_self(*m, h.0)) {
                        let before = node_enabled_states(rt);
                        remove_tree_modal_state(rt, h);
                        notify_node_enabled(rt, before)?;
                    }
                    Ok(())
                },
            ),
            (
                "opacity",
                |rt, t| layer_get(rt, t, |l| l.opacity),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.opacity = x.clamp(0, 255)),
            ),
            (
                "absoluteOrderMode",
                |rt, t| layer_get(rt, t, |l| l.absolute_order_mode as i64),
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer.absoluteOrderMode")?;
                    rt.host_mut().set_order_mode(h, v.to_integer()? != 0)
                },
            ),
            (
                "absolute",
                |rt, t| {
                    let h = this_of(rt, t, "Layer.absolute")?;
                    let l = rt.host().layer(h)?;
                    let Some(p) = l.parent else { return Ok(Variant::Integer(0)); };
                    Ok(Variant::Integer(if rt.host().layer(ObjectHandle(p))?.absolute_order_mode {
                        l.absolute
                    } else {
                        rt.host().sorted_children(p).iter().position(|c| *c == h.0).unwrap_or(0) as i64
                    }))
                },
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer.absolute")?;
                    let value = v.to_integer()?;
                    let p = rt.host().layer(h)?.parent.ok_or_else(|| TjsError::runtime("cannot move primary or siblingless layer"))?;
                    rt.host_mut().set_order_mode(ObjectHandle(p), true)?;
                    let mut order = rt.host().sorted_children(p);
                    order.retain(|c| *c != h.0);
                    let index = order.iter().position(|c| rt.host().layers[c].absolute >= value).unwrap_or(order.len());
                    order.insert(index, h.0);
                    rt.host_mut().layer_mut(ObjectHandle(p))?.children = order;
                    let l = rt.host_mut().layer_mut(h)?;
                    l.absolute = value;
                    l.dirty = true;
                    Ok(())
                },
            ),
            (
                "order",
                |rt, t| {
                    let h = this_of(rt, t, "Layer")?;
                    let parent = rt.host().layer(h)?.parent;
                    Ok(Variant::Integer(parent.map_or(0, |p| {
                        rt.host().sorted_children(p).iter().position(|c| *c == h.0).unwrap_or(0) as i64
                    })))
                },
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer.order")?;
                    let n = v.to_integer()?.max(0) as usize;
                    let p = rt.host().layer(h)?.parent.ok_or_else(|| TjsError::runtime("cannot move primary or siblingless layer"))?;
                    rt.host_mut().set_order_mode(ObjectHandle(p), false)?;
                    let mut order = rt.host().sorted_children(p);
                    order.retain(|c| *c != h.0);
                    order.insert(n.min(order.len()), h.0);
                    rt.host_mut().layer_mut(ObjectHandle(p))?.children = order;
                    rt.host_mut().layer_mut(h)?.dirty = true;
                    Ok(())
                },
            ),
            (
                "parent",
                |rt, t| {
                    let h = this_of(rt, t, "Layer")?;
                    Ok(rt
                        .host()
                        .layer(h)?
                        .parent
                        .map(|p| Variant::self_bound(ObjectHandle(p)))
                        .unwrap_or(Variant::Null))
                },
                |rt, t, v| {
                    let h = this_of(rt, t, "Layer.parent")?;
                    let parent = object_arg(&v)
                        .map(|p| rt.bound_this(p).unwrap_or(p))
                        .ok_or_else(|| {
                            unsupported(
                                "Layer.parent = null",
                                "detached native tree lifetime mapping",
                            )
                        })?;
                    let l = rt.host().layer(h)?.clone();
                    let p = rt.host().layer(parent)?;
                    if l.window != p.window {
                        return Err(TjsError::runtime(
                            "native parent belongs to a different window",
                        ));
                    }
                    if !rt.host().same_manager(h.0, parent.0) {
                        return Err(TjsError::runtime("cannot move a layer under another primary layer"));
                    }
                    if rt.host().windows.get(&l.window).is_some_and(|w|
                        w.primary == Some(h.0) || w.views.iter().any(|v| v.layer == h.0)) {
                        return Err(unsupported(
                            "primary layer reparent",
                            "native primary-tree replacement mapping",
                        ));
                    }
                    if l.parent == Some(parent.0) {
                        return Ok(());
                    }
                    let mut cursor = Some(parent.0);
                    let mut depth = 0;
                    while let Some(id) = cursor {
                        if id == h.0 {
                            return Err(TjsError::runtime("native parent cycle"));
                        }
                        depth += 1;
                        if depth > 64 {
                            return Err(TjsError::runtime("native parent depth"));
                        }
                        cursor = rt.host().layers.get(&id).and_then(|l| l.parent);
                    }
                    crate::native_mouse::leave_tree(rt, h.0, true)?;
                    let old_parent = rt.host().layer(h)?.parent;
                    if let Some(old) = old_parent.and_then(|id| rt.host_mut().layers.get_mut(&id)) {
                        old.children.retain(|id| *id != h.0);
                    }
                    rt.host_mut()
                        .layers
                        .get_mut(&parent.0)
                        .unwrap()
                        .children
                        .push(h.0);
                    let l = rt.host_mut().layer_mut(h)?;
                    l.parent = Some(parent.0);
                    l.dirty = true;
                    // Removing the old parent recursively removes its retained
                    // descendants. Publish the move before another script call
                    // can invalidate that subtree in the same VM turn.
                    if l.attached {
                        let id = l.container();
                        let next = rt.host().layer(parent)?.container();
                        rt.host_mut().layer_op(json!({"op":"set_parent","id":id,"parent":next}));
                    }
                    Ok(())
                },
            ),
            (
                "children",
                |rt, t| {
                    let h = this_of(rt, t, "Layer")?;
                    let items = rt
                        .host()
                        .sorted_children(h.0)
                        .into_iter()
                        .map(|c| Variant::self_bound(ObjectHandle(c)))
                        .collect();
                    Ok(Variant::Object(rt.alloc_array_object(items)))
                },
                read_only,
            ),
            (
                "window",
                |rt, t| {
                    let h = this_of(rt, t, "Layer")?;
                    Ok(rt.object_member(h, "__window"))
                },
                read_only,
            ),
            (
                "isPrimary",
                |rt, t| layer_get(rt, t, |l| (l.constructed && l.parent.is_none()) as i64),
                read_only,
            ),
        ],
    );
}

fn layer_get(rt: &Rt, t: Option<ObjectHandle>, f: fn(&LayerState) -> i64) -> Result<Variant> {
    let h = this_of(rt, t, "Layer")?;
    Ok(Variant::Integer(f(rt.host().layer(h)?)))
}

/// TJS2 creates a native instance with the object, before the script
/// constructor runs, so position writes before `Layer(...)` act on the
/// default state. They are kept here and applied when the Layer is built.
fn pre_set(rt: &mut Rt, t: Option<ObjectHandle>, v: &Variant, key: &str) -> Result<bool> {
    let h = this_of(rt, t, "Layer")?;
    if rt.host().layers.contains_key(&h.0) {
        return Ok(false);
    }
    rt.set_object_member(h, key, Variant::Integer(v.to_integer()?));
    Ok(true)
}
fn pre_get(rt: &Rt, t: Option<ObjectHandle>, key: &str) -> Option<Variant> {
    let h = this_of(rt, t, "Layer").ok()?;
    if rt.host().layers.contains_key(&h.0) {
        return None;
    }
    Some(match rt.object_member(h, key) {
        Variant::Void => Variant::Integer(0),
        v => v,
    })
}

fn layer_set(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    v: Variant,
    f: fn(&mut LayerState, i64),
) -> Result<()> {
    let h = this_of(rt, t, "Layer")?;
    let x = v.to_integer()?;
    let l = rt.host_mut().layer_mut(h)?;
    f(l, x);
    l.dirty = true;
    Ok(())
}

fn restack(rt: &mut Rt, t: Option<ObjectHandle>, f: fn(&mut Vec<usize>, usize)) -> Result<Variant> {
    let h = this_of(rt, t, "Layer")?;
    let Some(p) = rt.host().layer(h)?.parent else {
        return Ok(Variant::Void);
    };
    let parent = rt
        .host_mut()
        .layers
        .get_mut(&p)
        .ok_or_else(|| TjsError::runtime("Layer parent missing"))?;
    if let Some(i) = parent.children.iter().position(|c| *c == h.0) {
        f(&mut parent.children, i);
    }
    Ok(Variant::Void)
}

fn set_image_size(rt: &mut Rt, h: ObjectHandle, w: i64, ht: i64) -> Result<()> {
    let l = rt.host().layer(h)?;
    if l.has_image && l.image_width == w && l.image_height == ht {
        return Ok(());
    }
    if l.has_image && (!l.blank || l.province_allocated) {
        return rt.host_mut().resize_surface(h, w, ht);
    }
    rt.host_mut().reset_surface(h, w, ht, [255, 255, 255, 0])
}

fn argb(c: i64, alpha: Option<i64>) -> [u8; 4] {
    let c = c as u32;
    let a = alpha.map_or((c >> 24) as u8, |a| a.clamp(0, 255) as u8);
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, a]
}

/// krkrz Layer.getLayerAt (GetMostFrontChildAt): the front layer at a point in
/// this layer's coordinates, using HitTestNoVisibleCheck without onHitTest.
fn layer_get_layer_at(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.getLayerAt")?;
    let (mut x, mut y) = (need_int(&a, 0, "Layer.getLayerAt")?, need_int(&a, 1, "Layer.getLayerAt")?);
    let exclude_self = a.get(2).is_some_and(Variant::is_truthy);
    let get_disabled = a.get(3).is_some_and(Variant::is_truthy);
    let mut root = h.0;
    let mut node = rt.host().layer(h)?;
    while let Some(parent) = node.parent {
        x += node.left;
        y += node.top;
        root = parent;
        node = rt.host().layer(ObjectHandle(parent))?;
    }
    // Front to back as GetMostFrontChildAt walks: last child first, then the parent.
    fn front_first(host: &crate::host::KrkrHost, id: usize, point: [i64; 2], out: &mut Vec<(usize, i64, i64)>) {
        let Some(l) = host.layers.get(&id) else { return };
        if !l.visible {
            return;
        }
        let local = [point[0] - l.left, point[1] - l.top];
        if local[0] < 0 || local[1] < 0 || local[0] >= l.width || local[1] >= l.height {
            return;
        }
        for child in host.sorted_children(id).into_iter().rev() {
            front_first(host, child, local, out);
        }
        out.push((id, local[0], local[1]));
    }
    let mut candidates = Vec::new();
    front_first(rt.host(), root, [x, y], &mut candidates);
    for (id, lx, ly) in candidates {
        if exclude_self && id == h.0 {
            continue;
        }
        let target = ObjectHandle(id);
        let layer = rt.host().layer(target)?;
        let (px, py) = (lx - layer.image_left, ly - layer.image_top);
        let kind = rt.object_member(target, "__lp_hitType").to_integer().unwrap_or(0);
        let threshold = rt.object_member(target, "__lp_hitThreshold").to_integer().unwrap_or(16);
        let hit = if kind == 1 {
            layer.regions.get(rt.host().client, [px, py]).map_err(|e| TjsError::runtime(e.to_string()))? != 0
        } else if layer.has_image && (px < 0 || py < 0 || px >= layer.image_width || py >= layer.image_height) {
            false
        } else if threshold <= 0 {
            true
        } else if threshold >= 256 || !layer.has_image {
            false
        } else if layer.kind == 1 {
            true
        } else if let Some(alpha) = layer.full_alpha {
            i64::from(alpha) >= threshold
        } else if let Some(crysirror_core::image_processing::ImageInput::Resource(image)) = &layer.image_source {
            image.alpha_at([px as u32, py as u32]).is_some_and(|alpha| i64::from(alpha) >= threshold)
        } else {
            // A composed canvas has no synchronous sample here; only a blank one is known empty.
            !layer.blank
        };
        if !hit {
            continue;
        }
        if !get_disabled {
            let mut enabled = !rt.host().disabled_by_mode(id);
            let mut node = Some(id);
            let mut depth = 0;
            while let (true, Some(n)) = (enabled, node) {
                enabled = rt.object_member(ObjectHandle(n), "__lp_enabled").is_truthy();
                node = rt.host().layer(ObjectHandle(n))?.parent;
                depth += 1;
                if depth > 64 {
                    return Err(TjsError::runtime("native enabled ancestry limit"));
                }
            }
            if !enabled {
                return Ok(Variant::Null);
            }
        }
        // tTJSVariant(owner, owner): the result is bound to the layer itself.
        return Ok(Variant::Closure(Closure::new(target, Some(target))));
    }
    Ok(Variant::Null)
}

/// krkrz GetMainPixel: the colour channels without alpha. Resident images read
/// directly; composed canvases prepare a shared image before retrying this call.
fn layer_get_main_pixel(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.getMainPixel")?;
    let (x, y) = (need_int(&a, 0, "Layer.getMainPixel")?, need_int(&a, 1, "Layer.getMainPixel")?);
    let l = rt.host().layer(h)?;
    if x < 0 || y < 0 || x >= l.image_width || y >= l.image_height {
        return Err(TjsError::runtime("Layer.getMainPixel: pixel out of range"));
    }
    native_extra::prepare(rt,h)?;
    let source=crate::image_api::input(rt,h)?;
    let [r,g,b,_]=source.resident_pixel(crysirror_core::state::ClientId(rt.host().client),[x as u32,y as u32])
        .map_err(|e|TjsError::runtime(e.to_string()))?.ok_or_else(||TjsError::runtime("pixel out of rectangle"))?;
    Ok(Variant::Integer(i64::from(r)<<16|i64::from(g)<<8|i64::from(b)))
}

/// Some legacy KAG classes declare an ordinary `face` field for their font,
/// shadowing the native accessor, then explicitly assign dfAuto/dfProvince to
/// that same field around bitmap operations. Retain the script field (and its
/// font strings), but honour its numeric draw-plane selections at the semantic
/// boundary. Otherwise a province clear paints opaque black into the image.
fn raw_draw_face(rt: &Rt, h: ObjectHandle) -> i64 {
    if let Variant::Integer(face) = rt.object_member(h, "face") {
        return face;
    }
    rt.object_member(h, "__lp_face").to_integer().unwrap_or(128)
}

/// krkrz UpdateDrawFace: dfAuto resolves through the layer's display type.
fn draw_face(rt: &Rt, h: ObjectHandle) -> i64 {
    let face = raw_draw_face(rt, h);
    if face != 128 {
        return face;
    }
    match rt.host().layers.get(&h.0).map_or(2, |l| l.kind) {
        1 | 3 | 4 | 5 | 8 | 9 | 10 | 11 => 1,
        12 => 4,
        _ => 0,
    }
}

fn check_face(rt: &Rt, h: ObjectHandle, member: &str) -> Result<()> {
    let face = raw_draw_face(rt, h);
    let hold = rt
        .object_member(h, "__lp_holdAlpha")
        .to_integer()
        .unwrap_or(0);
    // dfAlpha/dfBoth (0) and dfAuto (128) draw colour and alpha together.
    if !matches!(face, 0 | 128) || hold != 0 {
        return Err(unsupported(
            &format!("{member} with face={face} holdAlpha={hold}"),
            "per-channel canvas draw faces",
        ));
    }
    Ok(())
}

fn layer_load_images(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    rt.host_mut().before_image_work()?;
    let h = this_of(rt, t, "Layer.loadImages")?;
    let name = string(&a, 0, "Layer.loadImages")?;
    if crate::varfile_plugin::is_uri(&name) {
        if int(&a,1,0x1fffffff)?!=0x1fffffff { return Err(unsupported("Layer.loadImages colour key","colour-keyed image decoding")); }
        return crate::varfile_plugin::load_image(rt,h,&name);
    }

    if int(&a, 1, 0x1fffffff)? != 0x1fffffff {
        return Err(unsupported("Layer.loadImages colour key", "colour-keyed image decoding"));
    }
    if crate::storage::save_path(&name)
        .map_err(|e| TjsError::runtime(e.to_string()))?
        .is_some()
    {
        let mut bytes = rt.host().storage()?.read_limited(&name, 64 * 1024 * 1024)
            .map_err(|e| TjsError::runtime(format!("read save image `{name}`: {e}")))?;
        let end = crate::plugin_ports::packinone::image_end(&bytes)
            .ok_or_else(|| TjsError::runtime(format!("no JPEG or PNG thumbnail in `{name}`")))?;
        bytes.truncate(end);
        return crate::psbfile_plugin::load_encoded_image(rt, h, &name, bytes);
    }
    if crate::psbfile_plugin::is_uri(&name) {
        return crate::psbfile_plugin::load_image(rt, h, &name);
    }
    if crate::psd_plugin::psd_uri(&name).is_some() && rt.host().storage()?.wrappers.psd.is_some() {
        return crate::psd_plugin::load_image(rt, h, &name);
    }
    let resolved = rt
        .host()
        .storage()?
        .resolve_with(&name, &[".tlg", ".png", ".jpg", ".jpeg", ".bmp", ".webp"]);
    // Assumption, not verified against the original: a `dummy_*` storage that no
    // archive provides is a placeholder for a script-drawn control (Yuzusoft
    // hsvcpick). It loads as a transparent pixel and the substitution is logged.
    let base = name.rsplit(['/', '\\', '>']).next().unwrap_or(&name);
    if resolved.is_none() && base.to_lowercase().starts_with("dummy_") {
        let host = rt.host_mut();
        host.log.push(format!("Layer.loadImages: placeholder `{name}` absent; using a transparent pixel"));
        host.reset_surface(h, 1, 1, [255, 255, 255, 0])?;
        let l = host.layer_mut(h)?;
        (l.width, l.height) = (1, 1);
        return Ok(Variant::Void);
    }
    let path = resolved.ok_or_else(|| TjsError::runtime(format!("image storage `{name}` not found")))?;
    if let Some((_, error)) = rt.host_mut().image_metadata_failure.take_if(|(failed, _)| failed == &path) {
        return Err(TjsError::runtime(format!("prepare image metadata for `{path}`: {error}")));
    }
    let images = rt
        .host()
        .images
        .clone()
        .ok_or_else(|| TjsError::runtime("engine image metadata unavailable"))?;
    if rt.host().can_suspend && !images.prepare_metadata(&path)
        .map_err(|e| TjsError::runtime(format!("prepare image metadata for `{path}`: {e}")))? {
        rt.host_mut().pending_image_metadata = Some(path);
        return Err(TjsError::resource_pending("waiting for decoded image metadata"));
    }
    let [w, ht] = images
        .size(&path)
        .map_err(|e| TjsError::runtime(format!("decode `{path}`: {e}")))?;
    let (w, ht) = (i64::from(w), i64::from(ht));
    rt.host_mut().reset_surface(h, w, ht, [255, 255, 255, 0])?;
    let host = rt.host_mut();
    let slot = {
        let l = host.layer_mut(h)?;
        (l.width, l.height) = (w, ht);
        l.blank = false;
        l.slot
    };
    host.canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,ht],"paint":{"kind":"image","asset":path}}}));
    host.api_calls += 1;
    let opaque = rt
        .host()
        .images
        .as_ref()
        .ok_or_else(|| TjsError::runtime("image metadata absent"))?
        .opaque(&path)
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    rt.host_mut().layer_mut(h)?.full_alpha = opaque.then_some(255);
    rt.host_mut().layer_mut(h)?.image_source =
        Some(crysirror_core::image_processing::ImageInput::Asset {
            path: path.clone(),
            size: [w as u32, ht as u32],
            fingerprint: Some(
                images
                    .fingerprint(&path)
                    .map_err(|e| TjsError::runtime(e.to_string()))?,
            ),
        });
    // Retain the immutable Asset description. Shared effects decode on demand;
    // prepare_hit_image resolves the current positive-alpha hit candidate before
    // consuming their input receipt. Hidden native caches need no second RGBA
    // copy merely because their image contains transparent pixels.
    // Native LayerIntf returns metainfo as an Object variant. With no tags
    // this is the null object, which is distinct from TJS void.
    Ok(Variant::Null)
}

fn fill_province(rt:&mut Rt,h:ObjectHandle,owner:u32,rect:[i64;4],region:u32)->Result<Variant> {
    let layer=rt.host().layer(h)?;
    let size=[layer.image_width as u32,layer.image_height as u32];
    let Some((rect,_))=crysirror_core::region_map::clip_region_copy(None,size,rect,[rect[0],rect[1]],layer.draw_clip)
        .map_err(|e|TjsError::runtime(e.to_string()))? else {return Ok(Variant::Void);};
    let layer=rt.host_mut().layer_mut(h)?;
    if region!=0 || layer.province_allocated {
        layer.regions.fill(owner,rect,region).map_err(|e|TjsError::runtime(e.to_string()))?;
        layer.image_modified=true;
        if region!=0 {layer.province_allocated=true;}
        else if rect==[0,0,i64::from(size[0]),i64::from(size[1])] {layer.province_allocated=false;}
    }
    rt.host_mut().api_calls+=1;
    Ok(Variant::Void)
}

fn layer_fill_rect(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.fillRect")?;
    if raw_draw_face(rt, h) == 3 {
        let owner = rt.host().client;
        let rect = [
            need_int(&a, 0, "province x")?,
            need_int(&a, 1, "province y")?,
            need_int(&a, 2, "province width")?,
            need_int(&a, 3, "province height")?,
        ];
        let region = need_int(&a, 4, "province region")? as u32 & 255;
        return fill_province(rt,h,owner,rect,region);
    }
    // krkrz LayerIntf FillRect: dfAlpha and dfAddAlpha replace all four
    // channels whatever holdAlpha says. holdAlpha only matters on dfOpaque,
    // where it fills colour alone; otherwise dfOpaque fills with alpha 255.
    let face = draw_face(rt, h);
    let hold = face == 1 && rt.object_member(h, "__lp_holdAlpha").to_integer().unwrap_or(0) != 0;
    if !matches!(face, 0 | 1 | 4) {
        check_face(rt, h, "Layer.fillRect")?;
    }
    let [x, y, w, ht] = [
        need_int(&a, 0, "Layer.fillRect")?,
        need_int(&a, 1, "Layer.fillRect")?,
        need_int(&a, 2, "Layer.fillRect")?,
        need_int(&a, 3, "Layer.fillRect")?,
    ];
    let mut color = argb(need_int(&a, 4, "Layer.fillRect")?, None);
    let l = rt.host().layer(h)?.clone();
    if w <= 0 || ht <= 0 {
        return Ok(Variant::Void);
    }
    let Some([x, y, w, ht]) = clip_rect(&l, [x, y, w, ht]) else {
        return Ok(Variant::Void);
    };
    if hold {
        let input = filled_source(rt, h, [x,y,w,ht], color, true);
        if !l.blank {
            let host = rt.host_mut();
            host.canvas_op(json!({"op":"fill","id":l.slot,"rect":[x,y,w,ht],"color":color,"keep_alpha":true}));
            host.api_calls += 1;
        }
        rt.host_mut().layer_mut(h)?.image_source = input;
        return Ok(Variant::Void);
    }
    if face == 1 {
        color[3] = 255;
    }
    let covers = x <= 0 && y <= 0 && x + w >= l.image_width && y + ht >= l.image_height;
    let input = (!covers).then(|| filled_source(rt, h, [x,y,w,ht], color, false)).flatten();
    let host = rt.host_mut();
    if covers {
        host.canvas_op(json!({"op":"clear","id":l.slot,"color":color}));
        host.layer_mut(h)?.blank = color[3] == 0;
        host.layer_mut(h)?.full_alpha = Some(color[3]);
    } else if !(color[3] == 0 && l.blank) {
        // fillRect replaces the region's pixels (shared canvas replace-fill).
        host.canvas_op(json!({"op":"fill","id":l.slot,"rect":[x,y,w,ht],"color":color,"replace":true}));
        host.layer_mut(h)?.blank = false;
        host.layer_mut(h)?.full_alpha = None;
    }
    if !covers { host.layer_mut(h)?.image_source = input; }
    host.api_calls += 1;
    Ok(Variant::Void)
}

/// Native fill policy expressed as an engine-owned immutable bitmap command.
fn filled_source(rt: &Rt, h: ObjectHandle, rect: [i64;4], color: [u8;4], keep_alpha: bool)
    -> Option<crysirror_core::image_processing::ImageInput>
{
    use crysirror_core::image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput};
    let base=crate::image_api::input(rt,h).ok()?;
    let [x,y,w,height]=[
        i32::try_from(rect[0]).ok()?,i32::try_from(rect[1]).ok()?,
        i32::try_from(rect[2]).ok()?,i32::try_from(rect[3]).ok()?,
    ];
    let layer=ImageCompositeLayer {
        input:Box::new(ImageInput::Solid {size:[w as u32,height as u32],color}),
        source:[0,0,w,height],destination:[x,y],clip:None,opacity:255,
        math:if keep_alpha {ImageCompositeMath::CopyRgb} else {ImageCompositeMath::Copy},
    };
    bounded(base.with_composite_layer(layer))
}

/// A retained CPU history past the shared worker's graph bound can never be
/// evaluated, and self-copies grow it geometrically. Drop it; a later read
/// recovers the layer's CPU source through the canvas capture path.
fn bounded(input: crysirror_core::image_processing::ImageInput) -> Option<crysirror_core::image_processing::ImageInput> {
    if input.within_graph_bound() {
        return Some(input);
    }
    if std::env::var_os("CRYSIRROR_KRKR_TRACE_SOURCE").is_some() {
        eprintln!("[source] composite history exceeds the shared graph bound; released");
    }
    None
}

fn layer_color_rect(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.colorRect")?;
    if raw_draw_face(rt, h) == 3 {
        let rect = [
            need_int(&a, 0, "province x")?,
            need_int(&a, 1, "province y")?,
            need_int(&a, 2, "province width")?,
            need_int(&a, 3, "province height")?,
        ];
        let region = need_int(&a, 4, "province region")? as u32 & 255;
        let owner = rt.host().client;
        return fill_province(rt,h,owner,rect,region);
    }
    let face = draw_face(rt, h);
    if face == 1 {
        // krkrz ColorRect on dfOpaque: tTVPBaseBitmap::FillColor blends the
        // colour at `opa` and always keeps destination alpha (source-atop
        // solid). dfAlpha uses FillColorOnAlpha regardless of holdAlpha.
        let r = [
            need_int(&a, 0, "Layer.colorRect")?,
            need_int(&a, 1, "Layer.colorRect")?,
            need_int(&a, 2, "Layer.colorRect")?,
            need_int(&a, 3, "Layer.colorRect")?,
        ];
        let opa = int(&a, 5, 255)?;
        if !(0..=255).contains(&opa) {
            return Err(unsupported(
                &format!("Layer.colorRect holdAlpha opacity {opa}"),
                "atop colour blend 0..=255",
            ));
        }
        let color = argb(need_int(&a, 4, "Layer.colorRect")?, Some(opa));
        let l = rt.host().layer(h)?.clone();
        let Some([x, y, w, ht]) = clip_rect(&l, r).filter(|_| r[2] > 0 && r[3] > 0 && opa > 0) else {
            return Ok(Variant::Void);
        };
        if !l.blank {
            let host = rt.host_mut();
            host.canvas_op(json!({"op":"draw","id":l.slot,"atop":"normal",
                "layer":{"id":0,"rect":[x,y,w,ht],"paint":{"kind":"solid","color":color}}}));
            host.layer_mut(h)?.image_source = None;
            host.api_calls += 1;
        }
        return Ok(Variant::Void);
    }
    if face != 0 {
        check_face(rt, h, "Layer.colorRect")?;
    }
    let [x, y, w, ht] = [
        need_int(&a, 0, "Layer.colorRect")?,
        need_int(&a, 1, "Layer.colorRect")?,
        need_int(&a, 2, "Layer.colorRect")?,
        need_int(&a, 3, "Layer.colorRect")?,
    ];
    let opa = int(&a, 5, 255)?;
    if opa < 0 {
        // krkrz ColorRect, alpha face: negative opacity removes opacity
        // (RemoveConstOpacity): alpha *= (255 - |opa|) / 255, colour kept.
        let l = rt.host().layer(h)?.clone();
        let Some([x, y, w, ht]) = clip_rect(&l, [x, y, w, ht]).filter(|_| w > 0 && ht > 0) else {
            return Ok(Variant::Void);
        };
        if !l.blank {
            let keep = 255 - opa.unsigned_abs().min(255);
            let host = rt.host_mut();
            host.canvas_op(json!({"op":"fade","id":l.slot,"rect":[x,y,w,ht],"opacity":keep}));
            host.layer_mut(h)?.full_alpha = None;
            host.layer_mut(h)?.image_source = None;
            host.api_calls += 1;
        }
        return Ok(Variant::Void);
    }
    let color = argb(need_int(&a, 4, "Layer.colorRect")?, Some(opa));
    if w <= 0 || ht <= 0 || opa == 0 {
        return Ok(Variant::Void);
    }
    let l = rt.host().layer(h)?.clone();
    let Some([x, y, w, ht]) = clip_rect(&l, [x, y, w, ht]) else {
        return Ok(Variant::Void);
    };
    let slot = l.slot;
    let host = rt.host_mut();
    host.canvas_op(json!({"op":"draw","id":slot,"layer":{"id":0,"rect":[x,y,w,ht],"paint":{"kind":"solid","color":color}}}));
    host.layer_mut(h)?.blank = false;
    host.api_calls += 1;
    Ok(Variant::Void)
}

/// `copyRect(dx, dy, src, sx, sy, sw, sh)` replaces pixels;
/// `operateRect(dx, dy, src, sx, sy, sw, sh, mode = omAuto, opa = 255)`
/// alpha-blends. Both map to engine canvas copies where the result is the
/// same operation.
fn layer_copy(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    a: Vec<Variant>,
    replace: bool,
) -> Result<Variant> {
    let member = if replace {
        "Layer.copyRect"
    } else {
        "Layer.operateRect"
    };
    rt.host_mut().before_image_work()?;
    let h = this_of(rt, t, member)?;
    // holdAlpha with a colour face: operateRect blends onto the colour and
    // keeps the destination alpha (source-atop; PSD clipping layers).
    let face = raw_draw_face(rt, h);
    let hold = rt.object_member(h, "__lp_holdAlpha").to_integer().unwrap_or(0) != 0;
    // copyRect on dfOpaque with holdAlpha copies only the colour plane
    // (krkrz CopyRect: TVP_BB_COPY_MAIN).
    // CopyRect uses DrawFace, not HoldAlpha alone (krkrz LayerIntf.cpp
    // UpdateDrawFace/CopyRect). Alpha/add-alpha faces copy both planes even
    // with holdAlpha set; only the opaque face can request RGB-only copy.
    let copy_face = if face == 128 {
        match rt.host().layer(h)?.kind {
            2 | 13..=28 => 0,
            12 => 4,
            _ => 1,
        }
    } else { face };
    let src = a
        .get(2)
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime(format!("{member}: source must be a Layer")))?;
    let mode = if replace { 128 } else { int(&a, 7, 128)? };
    // krkrz bmAlphaOnAlpha uses TVPAlphaBlend_d/do for both HoldAlpha
    // values. Ordinary alpha layers can therefore paint previously clear
    // pixels when the script leaves holdAlpha enabled between PSD parts.
    let alpha_on_alpha = !replace && copy_face == 0
        && (mode == 2 || (mode == 128 && rt.host().layer(src)?.kind == 2));
    let copy_main = hold && replace && copy_face == 1;
    let atop = copy_main || hold && !replace && !alpha_on_alpha && matches!(face, 0 | 1 | 128);
    if replace {
        if !matches!(copy_face, 0 | 1 | 3 | 4) {
            return Err(unsupported(&format!("{member} with draw face={copy_face}"),
                "per-channel canvas draw faces"));
        }
    } else if !atop && !alpha_on_alpha {
        check_face(rt, h, member)?;
    }
    // krkrz LayerBitmapIntf: Photoshop modes with holdAlpha use the `_HDA`
    // (hold destination alpha) kernels; without it they zero the alpha.
    // holdAlpha Photoshop modes the canvas has no blend state for run on the
    // shared CPU bitmap compositor (krkr's own tvpps kernels), when both
    // layers have CPU sources.
    if atop && !matches!(mode, 2 | 13 | 128 | 16 | 17) {
        if let Some(blend) = ps_blend(mode) {
            return layer_operate_ps_cpu(rt, h, &a, blend, true, member);
        }
    }
    // Without holdAlpha the omPs* modes run the plain (non-HDA) tvpps
    // kernels (LayerBitmapIntf.cpp TVP_BLEND_4), which store the operation
    // result with a cleared alpha byte; same shared CPU compositor.
    if !replace && !atop {
        if let Some(blend) = ps_blend(mode) {
            return layer_operate_ps_cpu(rt, h, &a, blend, false, member);
        }
    }
    crate::image_api::before_bitmap_call(rt)?;
    let atop_mode = match (atop, mode) {
        (false, _) => None,
        _ if copy_main => Some("copy"),
        (true, 2 | 13 | 128) => Some("normal"),
        (true, 16) => Some("multiply"),
        (true, 17) => Some("screen"),
        (true, m) => {
            return Err(unsupported(
                &format!("{member} holdAlpha mode {m}"),
                "keep-alpha canvas blend for this operation mode",
            ))
        }
    };
    let (dx, dy) = (need_int(&a, 0, member)?, need_int(&a, 1, member)?);
    let [sx, sy, sw, sh] = [
        need_int(&a, 3, member)?,
        need_int(&a, 4, member)?,
        need_int(&a, 5, member)?,
        need_int(&a, 6, member)?,
    ];
    if !replace {
        let mode = int(&a, 7, 128)?;
        let opa = int(&a, 8, 255)?;
        if !matches!(mode, 2 | 128) && atop_mode.is_none() {
            return Err(unsupported(
                &format!("{member} mode {mode}"),
                "canvas blend operation modes",
            ));
        }
        if !(0..=255).contains(&opa) {
            return Err(unsupported(
                &format!("{member} opacity {opa}"),
                "canvas copy opacity 0..=255",
            ));
        }
    }
    let opa = if replace { 255 } else { int(&a, 8, 255)? };
    if opa == 0 {
        return Ok(Variant::Void);
    }
    if sw <= 0 || sh <= 0 {
        return Ok(Variant::Void);
    }
    let source = rt.host().layer(src)?.clone();
    let dest = rt.host().layer(h)?.clone();
    if replace && copy_face==3 {
        let owner=rt.host().client;
        let Some((rect,at))=crysirror_core::region_map::clip_region_copy(None,
            [dest.image_width as u32,dest.image_height as u32],
            [sx,sy,sw,sh],[dx,dy],dest.draw_clip).map_err(|e|TjsError::runtime(e.to_string()))? else {return Ok(Variant::Void);};
        let layer=rt.host_mut().layer_mut(h)?;
        if source.province_allocated {
            let changed=layer.regions.copy_from(owner,&source.regions,rect,at,None)
                .map_err(|e|TjsError::runtime(e.to_string()))?;
            layer.province_allocated=true;layer.image_modified|=changed;
        } else {
            // krkrz CopyRect's absent-province branch fills the clipped SOURCE
            // rectangle, not the destination coordinates. Preserve that policy.
            if layer.province_allocated {layer.regions.fill(owner,rect,0)
                .map_err(|e|TjsError::runtime(e.to_string()))?;}
            layer.image_modified=true;
        }
        rt.host_mut().api_calls+=1;
        return Ok(Variant::Void);
    }
    // A complete destination replacement can select one cell from a larger
    // source atlas. The shared Canvas copy clips/snapshots that cell itself.
    let whole = [dx, dy] == [0, 0]
        && [sw, sh] == [dest.image_width, dest.image_height]
        && sx >= 0
        && sy >= 0
        && sx.checked_add(sw).is_some_and(|r| r <= source.image_width)
        && sy.checked_add(sh).is_some_and(|b| b <= source.image_height);
    // Keep the destination's CPU image source as an ordered composite of
    // the two sources, so later pixel effects (doBoxBlur, gamma, ...) see the
    // bytes krkr's blitter produced. Computed before the canvas op, which
    // invalidates the old source. The engine composition graph retains native
    // destination clips; each operation keeps its original source mapping.
    // Modes without a composite form leave the layer canvas-only.
    let composed = match atop_mode {
        None => composite_source(rt, h, src, &dest, &source, [sx, sy, sw, sh], [dx, dy], opa as u8, replace, mode),
        Some("normal") => composite_source(rt, h, src, &dest, &source, [sx, sy, sw, sh], [dx, dy], opa as u8, replace, -1),
        Some("copy") => composite_with(
            rt, h, src, &dest, &source, [sx, sy, sw, sh], [dx, dy], 255,
            crysirror_core::image_processing::ImageCompositeMath::CopyRgb,
        ),
        Some(_) => None,
    };
    // Partial copyRect replaces the destination region (shared canvas
    // region replace); onto a blank destination it is a plain draw.
    let host = rt.host_mut();
    host.canvas_op(
        json!({"op":"copy","source":source.slot,"destination":dest.slot,
        "rect":[sx,sy,sw,sh],"position":[dx,dy],"opacity":opa,"replace":replace && !atop && (whole || !dest.blank),
        "clip":dest.draw_clip,"atop":atop_mode}),
    );
    if atop {
        // Alpha is unchanged; only colour moved.
        host.layer_mut(h)?.image_source = composed;
        host.api_calls += 1;
        return Ok(Variant::Void);
    }
    host.layer_mut(h)?.image_source = composed;
    let blank = (replace && whole && source.blank) || (dest.blank && source.blank);
    host.layer_mut(h)?.blank = blank;
    host.layer_mut(h)?.full_alpha = if replace && whole {
        source.full_alpha
    } else if !replace && dest.full_alpha == Some(255) {
        Some(255)
    } else {
        None
    };
    host.api_calls += 1;
    Ok(Variant::Void)
}

/// krkrz omPs* modes with an exact byte-table form in the shared compositor.
fn ps_blend(mode: i64) -> Option<crysirror_core::image_processing::PsBlend> {
    use crysirror_core::image_processing::PsBlend as P;
    Some(match mode {
        13 => P::Normal,
        14 => P::Additive,
        15 => P::Subtractive,
        16 => P::Multiplicative,
        17 => P::Screen,
        18 => P::Overlay,
        19 => P::HardLight,
        20 => P::SoftLight,
        21 => P::ColorDodge,
        23 => P::ColorBurn,
        24 => P::Lighten,
        25 => P::Darken,
        22 => P::ColorDodge5,
        26 => P::Difference,
        27 => P::Difference5,
        28 => P::Exclusion,
        _ => return None,
    })
}

/// Photoshop-mode bitmap policy over the shared immutable compositor.
/// Small independent writes batch until an ordering boundary; self/large or
/// unresolved inputs retain the single-operation preparation path.
fn layer_operate_ps_cpu(
    rt: &mut Rt,
    h: ObjectHandle,
    a: &[Variant],
    blend: crysirror_core::image_processing::PsBlend,
    hold_alpha: bool,
    member: &str,
) -> Result<Variant> {
    let (dx, dy) = (need_int(a, 0, member)?, need_int(a, 1, member)?);
    let src = a
        .get(2)
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime(format!("{member}: source must be a Layer")))?;
    let rect = [need_int(a, 3, member)?, need_int(a, 4, member)?, need_int(a, 5, member)?, need_int(a, 6, member)?];
    let opa = int(a, 8, 255)?;
    let opacity = u8::try_from(opa)
        .map_err(|_| unsupported(&format!("{member} opacity {opa}"), "canvas copy opacity 0..=255"))?;
    if opacity == 0 || rect[2] <= 0 || rect[3] <= 0 {
        return Ok(Variant::Void);
    }
    let source = rt.host().layer(src)?.clone();
    let dest = rt.host().layer(h)?.clone();
    let math = crysirror_core::image_processing::ImageCompositeMath::Ps { blend, hold_alpha };
    if rt.host().bitmap_batch.as_ref().is_some_and(|b| b.target != h.0 || b.operations >= 64) {
        crate::image_api::before_bitmap_call(rt)?;
    }
    let input = composite_with(rt, h, src, &dest, &source, rect, [dx, dy], opacity, math);
    let suspendable = rt.host().can_suspend;
    // The first operation may need a Canvas capture before it can enter a
    // batch. Its temporary backing is part of this same redraw transaction.
    rt.host_mut().bitmap_redraw_pending = true;
    // Consecutive independent bitmap writes are one bounded shared image DAG.
    // Reads, other native calls and callback completion seal it before their
    // effects. No bitmap algorithm or pixel storage is implemented in the VM.
    let small_source = source.image_width > 0 && source.image_height > 0
        && source.image_width.saturating_mul(source.image_height) <= 65_536
        && matches!(source.image_source, Some(crysirror_core::image_processing::ImageInput::Resource(_)
            | crysirror_core::image_processing::ImageInput::Solid { .. }
            | crysirror_core::image_processing::ImageInput::Asset { .. }));
    if suspendable && src != h && small_source {
        if let Some(input) = input {
            let host = rt.host_mut();
            let layer = host.layer_mut(h)?;
            layer.image_source = Some(input);
            layer.full_alpha = None;
            layer.blank = false;
            let batch = host.bitmap_batch.get_or_insert(crate::image_api::BitmapBatch { target: h.0, operations: 0 });
            batch.operations += 1;
            return Ok(Variant::Void);
        }
    }
    crate::image_api::before_bitmap_call(rt)?;
    let input = composite_with(rt, h, src, &dest, &source, rect, [dx, dy], opacity, math);
    let Some(input) = input.filter(|_| suspendable) else {
        // A composed layer has no CPU source yet, and a native input callback
        // cannot suspend. Queue the operation: captures and composites finish in
        // order before the VM runs again.
        rt.host_mut().composite_pending.push(crate::image_api::PendingComposite::new(
            h, src, rect, [dx, dy], opacity, math,
        ));
        if suspendable {
            rt.request_suspend();
        }
        return Ok(Variant::Void);
    };
    crate::image_api::queue_chain(rt, h, input, Vec::new())
}

/// The composite CPU source of a copyRect/operateRect, mirroring krkrz's
/// tTVPBBBltMethod choice: copy for copyRect; for operateRect omAlpha (or
/// omAuto with a non-opaque source) is bmAlphaOnAlpha onto an alpha layer and
/// bmAlpha onto an opaque one, an opaque source is bmCopyOnAlpha / bmCopy;
/// `mode` -1 is the hold-destination-alpha normal blend.
#[allow(clippy::too_many_arguments)]
fn composite_source(
    rt: &Rt,
    h: ObjectHandle,
    src: ObjectHandle,
    dest: &LayerState,
    source: &LayerState,
    rect: [i64; 4],
    at: [i64; 2],
    opacity: u8,
    replace: bool,
    mode: i64,
) -> Option<crysirror_core::image_processing::ImageInput> {
    use crysirror_core::image_processing::ImageCompositeMath as Math;
    let alpha_source = mode == 2 || source.kind != 1;
    // `mode == -1`: a holdAlpha "normal" blend.
    let math = match (replace, dest.kind == 1, alpha_source) {
        _ if mode == -1 => Math::MaskHoldAlphaScalar256,
        (true, _, _) => Math::Copy,
        (false, false, true) => Math::AlphaOnAlphaScalar256,
        (false, false, false) => Math::OpaqueOnAlphaScalar256,
        (false, true, true) => Math::MaskScalar256,
        (false, true, false) => Math::OpaqueScalar256,
    };
    composite_with(rt, h, src, dest, source, rect, at, opacity, math)
}

/// `dest`'s CPU source with `source`'s rect drawn at `at` by `math`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_with(
    rt: &Rt,
    h: ObjectHandle,
    src: ObjectHandle,
    dest: &LayerState,
    source: &LayerState,
    rect: [i64; 4],
    at: [i64; 2],
    opacity: u8,
    math: crysirror_core::image_processing::ImageCompositeMath,
) -> Option<crysirror_core::image_processing::ImageInput> {
    use crysirror_core::image_processing::ImageCompositeLayer;
    let trace = std::env::var_os("CRYSIRROR_KRKR_TRACE_SOURCE").is_some();
    let base = crate::image_api::input(rt, h).inspect_err(|_| if trace { eprintln!("[source] composite base slot {} missing", dest.slot) }).ok()?;
    let input = crate::image_api::input(rt, src).inspect_err(|_| if trace { eprintln!("[source] composite input slot {} missing: {}", source.slot, source.source_lost) }).ok()?;
    let to_i32 = |v: i64| i32::try_from(v).ok();
    let layer = ImageCompositeLayer {
        input: Box::new(input),
        source: [to_i32(rect[0])?, to_i32(rect[1])?, to_i32(rect[2])?, to_i32(rect[3])?],
        destination: [to_i32(at[0])?, to_i32(at[1])?],
        clip: match dest.draw_clip {
            Some(c) => Some([to_i32(c[0])?, to_i32(c[1])?, to_i32(c[2])?, to_i32(c[3])?]),
            None => None,
        },
        opacity,
        math,
    };
    bounded(base.with_composite_layer(layer))
}

fn layer_assign_images(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.assignImages")?;
    let src = a
        .first()
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime("Layer.assignImages: source must be a Layer"))?;
    let host = rt.host_mut();
    let source = host.layer(src)?;
    let (width, height, source_slot) = (source.image_width, source.image_height, source.slot);
    let (blank, alpha, province) = (source.blank, source.full_alpha, source.province_allocated);
    let regions = source.regions.clone();
    let image_source = source.image_source.clone();
    let lost = image_source.is_none().then(|| format!("assign from slot {source_slot}: {}", source.source_lost));
    host.reset_surface(h, width, height, [255, 255, 255, 0])?;
    let slot = host.layer(h)?.slot;
    host.canvas_op(json!({"op":"copy","source":source_slot,"destination":slot,
        "rect":[0,0,width,height],"position":[0,0],"opacity":255,"replace":true}));
    let destination = host.layer_mut(h)?;
    destination.blank = blank;
    destination.full_alpha = alpha;
    destination.regions = regions;
    destination.province_allocated = province;
    destination.image_source = image_source;
    if let Some(reason) = lost {
        destination.source_lost = reason;
    }
    host.api_calls += 1;
    Ok(Variant::Void)
}

/// `drawText(x, y, text, color, opa = 255, aa = true, shadowlevel = 0, ...)`.
fn layer_draw_text(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.drawText")?;
    check_face(rt, h, "Layer.drawText")?;
    let (x, y) = (
        need_int(&a, 0, "Layer.drawText")?,
        need_int(&a, 1, "Layer.drawText")?,
    );
    let text = string(&a, 2, "Layer.drawText")?;
    if text.contains(['\n', '\r']) {
        return Err(unsupported(
            "multiline Layer.drawText",
            "native multiline text policy",
        ));
    }
    let opa = int(&a, 4, 255)?;
    // krkrz drawText: four parameters are required; a void colour converts to 0.
    if a.len() < 4 {
        return Err(TjsError::bad_param_count());
    }
    let color = argb(a[3].to_integer()?, Some(opa));
    let shadow_level = int(&a, 6, 0)?;
    let blur = int(&a, 8, 0)?;
    if shadow_level != 0 && !(0..=64).contains(&blur.abs()) || !(0..=65535).contains(&shadow_level) {
        return Err(unsupported(
            "Layer.drawText shadow width/level range",
            "shared glyph blur radius 0..=64",
        ));
    }
    if text.is_empty() || opa <= 0 {
        return Ok(Variant::Void);
    }
    let font = layer_font(rt, h)?;
    let size = font_size(rt, font)?;
    let l = rt.host().layer(h)?.clone();
    let alias=crate::font_resources::alias(rt,font)?;
    let raster = crate::font_resources::raster_size(rt, font, &alias, size)?;
    let size = raster.draw;
    let y = y.checked_add(i64::from(raster.offset_y)).ok_or_else(|| TjsError::runtime("text y overflow"))?;
    // A mapped precomputed face already contains the native Font style.
    let bold = rt.resolve_object_member(font, "bold")?.is_truthy()
        && !crate::font_resources::is_mapped(rt,font)?;
    let mapped=crate::font_resources::is_mapped(rt,font)?;
    let underline=!mapped&&rt.resolve_object_member(font,"underline")?.is_truthy();
    let strikeout=!mapped&&rt.resolve_object_member(font,"strikeout")?.is_truthy();
    if !bold && shadow_level == 0 && !underline && !strikeout {
        let host = rt.host_mut();
        if host.layer(h)?.full_alpha != Some(255) {
            host.layer_mut(h)?.full_alpha = None;
        }
        host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x,y,(l.image_width-x).max(1),size as i64*2],"paint":{"kind":"text","text":text,"font":alias,"size":size,"color":color,"layout":crysirror_core::service::UNWRAPPED_TEXT_LAYOUT}}}));
        host.layer_mut(h)?.blank = false;
        host.api_calls += 1;
        return Ok(Variant::Void);
    }
    if text.len() > 4096 {
        return Err(TjsError::runtime(
            "native positioned text exceeds 4096 bytes",
        ));
    }
    let host = rt.host();
    let placements = host
        .text_measurement
        .as_ref()
        .ok_or_else(|| TjsError::runtime("engine text measurement handle absent"))?
        .position_simple(&alias, size as u16, &text)
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    let glyphs = placements
        .into_iter()
        .map(|g| json!({"character":g.character.to_string(),"baseline":g.baseline,"style":{"bold":bold,"underline":underline,"strikeout":strikeout}}))
        .collect::<Vec<_>>();
    let host = rt.host_mut();
    if host.layer(h)?.full_alpha != Some(255) {
        host.layer_mut(h)?.full_alpha = None;
    }
    if shadow_level != 0 {
        let offset = [int(&a, 9, 0)?, int(&a, 10, 0)?];
        // krkrz CharacterData::Blur: coverage blurred by |width| and scaled by
        // level/256 (shared glyph blur), drawn in the shadow colour at `opa`.
        let shadow = argb(int(&a, 7, 0)?, Some(opa));
        let blur = json!({"radius": blur.unsigned_abs(), "level": shadow_level});
        host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x+offset[0],y+offset[1],l.image_width.max(1),l.image_height.max(1)],"paint":{"kind":"glyph_run","font":alias,"size":size,"color":shadow,"bold":bold,"glyphs":glyphs,"blur":blur}}}));
    }
    host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x,y,l.image_width.max(1),l.image_height.max(1)],"paint":{"kind":"glyph_run","font":alias,"size":size,"color":color,"bold":bold,"glyphs":glyphs}}}));
    host.layer_mut(h)?.blank = false;
    host.api_calls += 1;
    Ok(Variant::Void)
}

/// `addFont.dll`: adds `System.addFont`.
pub fn install_add_font(rt: &mut Rt) {
    let system = rt.global_member("System").object_handle().expect("System");
    methods(rt,system,&[("addFont",|rt,_,a| {
        let name=string(&a,0,"System.addFont")?;
        crate::font_resources::add(rt,name)
    })]);
}

/// Shared canvas transform copy (replace semantics of krkr copies). Sampling
/// type is the engine's own filter; `clear` of affineCopy is not mapped.
fn transform_copy(
    rt: &mut Rt,
    h: ObjectHandle,
    src: ObjectHandle,
    rect: [f64; 4],
    matrix: [f64; 6],
    member: &str,
) -> Result<Variant> {
    transform_blit(rt, h, src, rect, matrix, None, member)
}

/// A transformed copy (`opacity: None`, replaces what it covers) or a
/// transformed alpha blend at `opacity` over the destination (operate*).
fn transform_blit(
    rt: &mut Rt,
    h: ObjectHandle,
    src: ObjectHandle,
    rect: [f64; 4],
    matrix: [f64; 6],
    opacity: Option<u8>,
    member: &str,
) -> Result<Variant> {
    check_face(rt, h, member)?;
    if rect[2] <= 0. || rect[3] <= 0. || opacity == Some(0) {
        return Ok(Variant::Void);
    }
    let source = rt.host().layer(src)?.clone();
    let dest = rt.host().layer(h)?.clone();
    let host = rt.host_mut();
    let replace = opacity.is_none() && !dest.blank;
    host.canvas_op(
        json!({"op":"transform_copy","source":source.slot,"destination":dest.slot,
        "rect":rect,"matrix":matrix,"replace":replace,"clip":dest.draw_clip,
        "opacity":opacity.unwrap_or(255)}),
    );
    host.layer_mut(h)?.blank = dest.blank && source.blank;
    host.layer_mut(h)?.full_alpha = None;
    host.api_calls += 1;
    Ok(Variant::Void)
}

fn layer_stretch_copy(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "Layer.stretchCopy";
    let h = this_of(rt, t, M)?;
    let n = |i: usize| -> Result<f64> { Ok(need_int(&a, i, M)? as f64) };
    let (dl, dt, dw, dh) = (n(0)?, n(1)?, n(2)?, n(3)?);
    let src = a
        .get(4)
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime("Layer.stretchCopy: source must be a Layer"))?;
    let (sl, st, sw, sh) = (n(5)?, n(6)?, n(7)?, n(8)?);
    if sw <= 0. || sh <= 0. || dw == 0. || dh == 0. {
        return Ok(Variant::Void);
    }
    let (ax, dy) = (dw / sw, dh / sh);
    transform_copy(
        rt,
        h,
        src,
        [sl, st, sw, sh],
        [ax, 0., 0., dy, dl - ax * sl, dt - dy * st],
        M,
    )
}

/// `Layer.operateStretch(dl, dt, dw, dh, src, sl, st, sw, sh, mode, opacity,
/// type)`: stretchCopy's geometry blended like operateRect. Normal alpha
/// modes (omAuto/omAlpha) without holdAlpha are engine blends; others fail
/// visibly. The stretch filter type selects interpolation quality only; the
/// renderer's filtered sampling stands in for every type, as in stretchCopy.
fn layer_operate_stretch(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "Layer.operateStretch";
    let h = this_of(rt, t, M)?;
    if rt.object_member(h, "__lp_holdAlpha").to_integer().unwrap_or(0) != 0 {
        return Err(unsupported(&format!("{M} with holdAlpha"), "keep-alpha transformed canvas blend"));
    }
    let mode = int(&a, 9, 128)?;
    if !matches!(mode, 2 | 128) {
        return Err(unsupported(&format!("{M} mode {mode}"), "canvas blend operation modes"));
    }
    let opa = int(&a, 10, 255)?;
    let opacity = u8::try_from(opa)
        .map_err(|_| unsupported(&format!("{M} opacity {opa}"), "canvas copy opacity 0..=255"))?;
    let n = |i: usize| -> Result<f64> { Ok(need_int(&a, i, M)? as f64) };
    let (dl, dt, dw, dh) = (n(0)?, n(1)?, n(2)?, n(3)?);
    let src = a
        .get(4)
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime(format!("{M}: source must be a Layer")))?;
    let (sl, st, sw, sh) = (n(5)?, n(6)?, n(7)?, n(8)?);
    if sw <= 0. || sh <= 0. || dw == 0. || dh == 0. {
        return Ok(Variant::Void);
    }
    let (ax, dy) = (dw / sw, dh / sh);
    transform_blit(rt, h, src, [sl, st, sw, sh], [ax, 0., 0., dy, dl - ax * sl, dt - dy * st], Some(opacity), M)
}

fn layer_affine_copy(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "Layer.affineCopy";
    let h = this_of(rt, t, M)?;
    let src = a
        .first()
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime("Layer.affineCopy: source must be a Layer"))?;
    let i = |k: usize| -> Result<f64> { Ok(need_int(&a, k, M)? as f64) };
    let (sl, st, sw, sh) = (i(1)?, i(2)?, i(3)?, i(4)?);
    let affine = a.get(5).is_some_and(Variant::is_truthy);
    let clear = a.get(13).is_some_and(Variant::is_truthy);
    if clear {
        // krkrz AffineBlt(clear): the clip area not covered by the transformed
        // source takes the neutral colour. The copy replaces what it covers,
        // so clearing the whole clip area first is the same result.
        let l = rt.host().layer(h)?.clone();
        if l.has_image {
            let neutral = rt.object_member(h, "__lp_neutralColor").to_integer().unwrap_or(0);
            let color = argb(neutral, None);
            let [x, y, w, ht] = clip_of(&l);
            let host = rt.host_mut();
            host.canvas_op(json!({"op":"fill","id":l.slot,"rect":[x,y,w,ht],"color":color,"replace":true}));
            let covers = x <= 0 && y <= 0 && x + w >= l.image_width && y + ht >= l.image_height;
            let layer = host.layer_mut(h)?;
            layer.blank = covers && color[3] == 0;
            layer.full_alpha = None;
            layer.image_source = None;
        }
    }
    if sw <= 0. || sh <= 0. {
        return Ok(Variant::Void);
    }
    let matrix = affine_matrix(&a, [sl, st, sw, sh], affine)?;
    transform_copy(rt, h, src, [sl, st, sw, sh], matrix, M)
}

/// The canvas matrix of krkr's affine arguments 6..=11: a matrix
/// (`affine`) or the three destination points of the source rect's
/// top-left, top-right and bottom-left corners.
fn affine_matrix(a: &[Variant], [sl, st, sw, sh]: [f64; 4], affine: bool) -> Result<[f64; 6]> {
    let r = |k: usize| -> Result<f64> { a.get(k).ok_or_else(TjsError::bad_param_count)?.to_real() };
    // krkr coordinates are pixel centers; the canvas maps pixel edges.
    Ok(if affine {
        let (ma, mb, mc, md, e, f) = (r(6)?, r(7)?, r(8)?, r(9)?, r(10)?, r(11)?);
        // dest_center = M·(src_center - (sl, st)) + (E, F), with centers at +0.5.
        let (cx, cy) = (sl + 0.5, st + 0.5);
        [
            ma,
            mb,
            mc,
            md,
            e + 0.5 - (ma * cx + mc * cy),
            f + 0.5 - (mb * cx + md * cy),
        ]
    } else {
        let (x0, y0, x1, y1, x2, y2) = (r(6)?, r(7)?, r(8)?, r(9)?, r(10)?, r(11)?);
        let (ma, mb) = ((x1 - x0) / sw, (y1 - y0) / sw);
        let (mc, md) = ((x2 - x0) / sh, (y2 - y0) / sh);
        [
            ma,
            mb,
            mc,
            md,
            x0 + 0.5 - (ma * sl + mc * st),
            y0 + 0.5 - (mb * sl + md * st),
        ]
    })
}

/// `Layer.operateAffine(src, sl, st, sw, sh, affine, a/x0, b/y0, c/x1, d/y1,
/// tx/x2, ty/y2, mode=omAuto, opa=255, type)` (krkrz LayerIntf.cpp:7455):
/// affineCopy's geometry blended like operateStretch -- alpha modes
/// (omAlpha, or omAuto) without holdAlpha are engine blends, other modes fail
/// visibly; the filter type is the renderer's sampling.
fn layer_operate_affine(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "Layer.operateAffine";
    let h = this_of(rt, t, M)?;
    if a.len() < 12 {
        return Err(TjsError::bad_param_count());
    }
    if rt.object_member(h, "__lp_holdAlpha").to_integer().unwrap_or(0) != 0 {
        return Err(unsupported(&format!("{M} with holdAlpha"), "keep-alpha transformed canvas blend"));
    }
    let mode = int(&a, 12, 128)?;
    if !matches!(mode, 2 | 128) {
        return Err(unsupported(&format!("{M} mode {mode}"), "canvas blend operation modes"));
    }
    let opa = int(&a, 13, 255)?;
    let opacity = u8::try_from(opa)
        .map_err(|_| unsupported(&format!("{M} opacity {opa}"), "canvas copy opacity 0..=255"))?;
    let src = a
        .first()
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime(format!("{M}: source must be a Layer")))?;
    let i = |k: usize| -> Result<f64> { Ok(need_int(&a, k, M)? as f64) };
    let rect = [i(1)?, i(2)?, i(3)?, i(4)?];
    if rect[2] <= 0. || rect[3] <= 0. {
        return Ok(Variant::Void);
    }
    let matrix = affine_matrix(&a, rect, a[5].is_truthy())?;
    transform_blit(rt, h, src, rect, matrix, Some(opacity), M)
}

pub(crate) fn clip_of(l: &LayerState) -> [i64; 4] {
    l.draw_clip.unwrap_or([0, 0, l.image_width, l.image_height])
}
fn clip_get(rt: &mut Rt, t: Option<ObjectHandle>, i: usize) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.clip")?;
    Ok(Variant::Integer(clip_of(rt.host().layer(h)?)[i]))
}
fn clip_set(rt: &mut Rt, t: Option<ObjectHandle>, i: usize, v: Variant) -> Result<()> {
    let h = this_of(rt, t, "Layer.clip")?;
    let v = v.to_integer()?;
    let l = rt.host_mut().layer_mut(h)?;
    let mut c = clip_of(l);
    c[i] = v;
    l.draw_clip = Some(c);
    Ok(())
}
/// Intersect a draw rect with the layer's clip; `None` when nothing remains.
fn clip_rect(l: &LayerState, r: [i64; 4]) -> Option<[i64; 4]> {
    let c = clip_of(l);
    let (x0, y0) = (r[0].max(c[0]), r[1].max(c[1]));
    let (x1, y1) = ((r[0] + r[2]).min(c[0] + c[2]), (r[1] + r[3]).min(c[1] + c[3]));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
}

/// krkrz `ImageLayerSizeChanged`: a layer larger than its main image grows the
/// image (contents kept at the top-left).
fn grow_image(rt: &mut Rt, h: ObjectHandle) -> Result<()> {
    let l = rt.host().layer(h)?;
    if !l.has_image || (l.image_width >= l.width && l.image_height >= l.height) {
        return Ok(());
    }
    let (w, ht) = (l.image_width.max(l.width), l.image_height.max(l.height));
    if l.blank {
        rt.host_mut().reset_surface(h, w, ht, [255, 255, 255, 0])
    } else {
        rt.host_mut().resize_surface(h, w, ht)
    }
}
