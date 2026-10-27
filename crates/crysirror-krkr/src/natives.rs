// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! KiriKiri native classes as thin translations onto engine services.
//!
//! Member names, defaults and argument conventions follow KiriKiri Z (as
//! inspected in Kirakira's `krkr-engine` at the pinned commit). Members whose
//! semantics need an engine capability CrySirror does not offer yet throw a
//! TJS exception naming that capability; nothing is silently stubbed.
use crate::host::{KrkrHost, LayerState, TimerState, WindowState};
use krkr_tjs2::{
    Result, TjsError, compile_source_to_bytecode,
    runtime::{Closure, NativeArgCount, ObjectHandle, Runtime, TjsHost, Variant},
};
use serde_json::json;

pub type Rt = Runtime<KrkrHost>;

pub fn install(rt: &mut Rt) {
    crate::constants::install_tvp_constants(rt);
    install_system(rt);
    crate::save_struct::install(rt);
    install_debug(rt);
    install_storages(rt);
    install_scripts(rt);
    install_plugins(rt);
    install_timer(rt);
    crate::async_trigger::install(rt);
    crate::menus::install(rt);
    install_window(rt);
    install_layer(rt);
    crate::sound::install(rt);
    crate::video::install(rt);
    crate::pointer::install(rt);
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

pub(crate) fn need_int(args: &[Variant], i: usize, what: &str) -> Result<i64> {
    match args.get(i) {
        None | Some(Variant::Void) => {
            Err(TjsError::runtime(format!("{what}: argument {i} required")))
        }
        Some(v) => v.to_integer(),
    }
}

pub(crate) fn string(args: &[Variant], i: usize, what: &str) -> Result<String> {
    match args.get(i) {
        None | Some(Variant::Void) => {
            Err(TjsError::runtime(format!("{what}: argument {i} required")))
        }
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
        rt.register_object_native_with_arg_count(h, *name, NativeArgCount::Any, f);
    }
}

pub(crate) fn properties(rt: &mut Rt, h: ObjectHandle, list: &[(&str, Getter, Setter)]) {
    for (name, g, s) in list {
        let (g, s) = (*g, *s);
        rt.register_object_native_property(h, *name, g, s);
    }
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
                rt.set_object_member(h, name, Variant::Closure(Closure::new(class, Some(h))));
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

pub fn run_source(
    rt: &mut Rt,
    name: &str,
    source: &str,
    this: Option<ObjectHandle>,
) -> Result<Variant> {
    let file = compile_source_to_bytecode(name, source)?;
    rt.execute_file_with_this(&file, this)
}

pub fn exec_storage(rt: &mut Rt, name: &str) -> Result<Variant> {
    let bytes = rt.host_mut().read_binary(name, "")?;
    if bytes.starts_with(&krkr_tjs2::bytecode::BYTECODE_SIGNATURE) {
        return rt.execute_bytecode(&bytes);
    }
    let source = rt.host_mut().read_text(name, "")?;
    run_source(rt, name, &source, None)
}

// ----------------------------------------------------------------- System

fn install_system(rt: &mut Rt) {
    let h = static_object(rt, "System");
    methods(
        rt,
        h,
        &[
            ("getTickCount", |rt, _, _| {
                Ok(Variant::Integer(rt.host().now as i64))
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
                continuous(rt).retain(|g| !g.discern_eq(&f));
                Ok(Variant::Void)
            }),
            ("createAppLock", |_, _, _| Ok(Variant::Integer(1))),
            ("shellExecute", |_, _, _| {
                Err(unsupported("System.shellExecute", "host process launching"))
            }),
            ("readRegValue", |_, _, _| {
                Err(unsupported(
                    "System.readRegValue",
                    "a Windows registry service",
                ))
            }),
        ],
    );
    properties(
        rt,
        h,
        &[
            (
                "screenWidth",
                |rt, _| Ok(Variant::Integer(rt.host().canvas[0] as i64)),
                read_only,
            ),
            (
                "screenHeight",
                |rt, _| Ok(Variant::Integer(rt.host().canvas[1] as i64)),
                read_only,
            ),
            (
                "desktopWidth",
                |rt, _| Ok(Variant::Integer(rt.host().canvas[0] as i64)),
                read_only,
            ),
            (
                "desktopHeight",
                |rt, _| Ok(Variant::Integer(rt.host().canvas[1] as i64)),
                read_only,
            ),
            (
                "exePath",
                |_, _| Ok(Variant::String("file://./".into())),
                read_only,
            ),
            (
                "exeName",
                |_, _| Ok(Variant::String("file://./crysirror".into())),
                read_only,
            ),
            (
                "dataPath",
                |_, _| Ok(Variant::String("file://./savedata/".into())),
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
        ],
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
            ("isExistentStorage", |rt, _, a| {
                let name = string(&a, 0, "Storages.isExistentStorage")?;
                Ok(Variant::Integer(
                    rt.host().storage()?.resolve(&name).is_some() as i64,
                ))
            }),
            ("getPlacedPath", |rt, _, a| {
                let name = string(&a, 0, "Storages.getPlacedPath")?;
                Ok(Variant::String(
                    rt.host()
                        .storage()?
                        .resolve(&name)
                        .map(|p| format!("file://./{p}"))
                        .unwrap_or_default(),
                ))
            }),
            ("getFullPath", |_, _, a| {
                let name = string(&a, 0, "Storages.getFullPath")?;
                Ok(Variant::String(format!(
                    "file://./{}",
                    crate::storage::normalize(&name)
                )))
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
                rt.host_mut()
                    .storage_mut()?
                    .add_auto_path(&path)
                    .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
                Ok(Variant::Void)
            }),
            ("removeAutoPath", |rt, _, a| {
                let path = string(&a, 0, "Storages.removeAutoPath")?;
                rt.host_mut().storage_mut()?.remove_auto_path(&path);
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
            ("searchCD", |_, _, _| {
                Err(unsupported("Storages.searchCD", "optical media"))
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
            ("execStorage", |rt, _, a| {
                let name = string(&a, 0, "Scripts.execStorage")?;
                exec_storage(rt, &name)
            }),
            ("evalStorage", |rt, _, a| {
                let name = string(&a, 0, "Scripts.evalStorage")?;
                let source = rt.host_mut().read_text(&name, "")?;
                if source.trim().is_empty() {
                    return Ok(Variant::Void);
                }
                run_source(rt, &name, &format!("return ({source});"), None)
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
                run_source(rt, "Scripts.eval", &format!("return ({source});"), ctx)
            }),
        ],
    );
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

fn install_window(rt: &mut Rt) {
    let class = native_class(rt, "Window", |rt, this, _| {
        let h = this_of(rt, this, "Window")?;
        if rt.host().windows.contains_key(&h.0) {
            return Ok(Variant::Void);
        }
        if !rt.host().windows.is_empty() {
            return Err(unsupported(
                "a second Window",
                "multiple engine output windows",
            ));
        }
        let [w, height] = rt.host().canvas;
        rt.host_mut().windows.insert(
            h.0,
            WindowState {
                primary: None,
                inner: [w as i64, height as i64],
                visible: false,
                menu: None,
            },
        );
        let root = crate::menus::root(rt, h)?;
        rt.host_mut().windows.get_mut(&h.0).unwrap().menu = Some(root.0);
        rt.set_object_member(h, "__caption", Variant::String(String::new()));
        if let Variant::Object(class) = rt.global_member("Window") {
            rt.set_object_member(class, "mainWindow", Variant::self_bound(h));
        }
        Ok(Variant::Void)
    });
    rt.register_object_native_property(class, "focusedLayer", |_, _| Ok(Variant::Null), read_only);
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
            rt.host_mut().ui_op(json!({"op":"set_title","text":text}));
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
            rt.host_mut()
                .ui_op(json!({"op":"set_fullscreen","enabled":enabled}));
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
            rt.host_mut().ui_op(
                json!({"op":"set_window_policy","decorated":decorated,"resizable":resizable}),
            );
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
            ("hideMouseCursor", |rt, t, _| {
                let h = this_of(rt, t, "Window.hideMouseCursor")?;
                rt.set_object_member(h, "__temporary_cursor_hide", Variant::Integer(1));
                rt.host_mut()
                    .ui_op(json!({"op":"set_cursor_visible","visible":false}));
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
            ("close", |rt, _, _| {
                rt.host_mut().exit_requested = true;
                Ok(Variant::Void)
            }),
            ("showModal", |_, _, _| {
                Err(unsupported("Window.showModal", "a nested modal event loop"))
            }),
            ("setMaskRegion", |_, _, _| {
                Err(unsupported("Window.setMaskRegion", "shaped OS windows"))
            }),
        ],
    );
    let inner_w: Getter = |rt, t| Ok(Variant::Integer(window_of(rt, t)?.inner[0]));
    let inner_h: Getter = |rt, t| Ok(Variant::Integer(window_of(rt, t)?.inner[1]));
    let set_w: Setter = |rt, t, v| {
        let h = this_of(rt, t, "Window")?;
        let height = window_mut(rt, h)?.inner[1];
        check_window_size(rt, v.to_integer()?, height)
    };
    let set_h: Setter = |rt, t, v| {
        let h = this_of(rt, t, "Window")?;
        let w = window_mut(rt, h)?.inner[0];
        check_window_size(rt, w, v.to_integer()?)
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
                    let w = window_mut(rt, h)?;
                    w.visible = boolean(&v);
                    if let Some(p) = w.primary {
                        if let Some(l) = rt.host_mut().layers.get_mut(&p) {
                            l.dirty = true;
                        }
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

fn check_window_size(rt: &Rt, w: i64, h: i64) -> Result<()> {
    let [cw, ch] = rt.host().canvas;
    if [w, h] != [cw as i64, ch as i64] {
        return Err(unsupported(
            &format!("Window inner size {w}x{h}"),
            &format!("a project canvas of the same size (currently {cw}x{ch})"),
        ));
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

fn font_size(rt: &Rt, font: ObjectHandle) -> Result<u64> {
    for property in ["italic", "angle"] {
        if rt.object_member(font, property).to_integer()? != 0 {
            return Err(unsupported(
                &format!("Font.{property}"),
                "matching engine canvas text style and metrics",
            ));
        }
    }
    let size = rt
        .object_member(font, "height")
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
    let size = font_size(rt, font)?;
    let host = rt.host();
    let metrics = host
        .text_measurement
        .as_ref()
        .ok_or_else(|| TjsError::runtime("engine text measurement handle absent"))?
        .measure_simple(&host.font, size as f32, &text)
        .map_err(|e| TjsError::runtime(format!("Font.getTextWidth: {e}")))?;
    Ok(Variant::Integer(metrics.advance.ceil() as i64))
}

fn font_text_height(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let font = this_of(rt, t, "Font.getTextHeight")?;
    // KiriKiri returns abs(Font.height), even for the empty string. It does
    // not return the glyph ink box or the engine's line pitch.
    let _ = string(&a, 0, "Font.getTextHeight")?;
    Ok(Variant::Integer(font_size(rt, font)? as i64))
}

fn install_layer(rt: &mut Rt) {
    let class = native_class(rt, "Layer", |rt, this, a| {
        let h = this_of(rt, this, "Layer")?;
        if a.is_empty() || rt.host().layers.contains_key(&h.0) {
            return Ok(Variant::Void);
        }
        let window = object_arg(&a[0])
            .map(|w| rt.bound_this(w).unwrap_or(w))
            .filter(|w| rt.host().windows.contains_key(&w.0))
            .ok_or_else(|| TjsError::runtime("Layer: first argument must be a Window"))?;
        let parent = match a.get(1) {
            None | Some(Variant::Void | Variant::Null) => None,
            Some(v) => Some(
                object_arg(v)
                    .map(|p| rt.bound_this(p).unwrap_or(p))
                    .filter(|p| rt.host().layers.contains_key(&p.0))
                    .ok_or_else(|| TjsError::runtime("Layer: parent must be a Layer"))?
                    .0,
            ),
        };
        if parent.is_none() && rt.host().windows[&window.0].primary.is_some() {
            return Err(TjsError::runtime("Window already has a primary layer"));
        }
        rt.host_mut().create_layer(h, window.0, parent)?;
        if parent.is_none() {
            let [w, height] = rt.host().windows[&window.0].inner;
            rt.host_mut().windows.get_mut(&window.0).unwrap().primary = Some(h.0);
            let l = rt.host_mut().layer_mut(h)?;
            l.width = w;
            l.height = height;
            rt.host_mut().reset_surface(h, w, height, [0, 0, 0, 255])?;
            rt.set_object_member(h, "type", Variant::Integer(1));
        } else {
            rt.set_object_member(h, "type", Variant::Integer(2));
        }
        let font = rt.alloc_ordinary_object();
        rt.set_object_member(font, "face", Variant::String(String::new()));
        rt.set_object_member(font, "height", Variant::Integer(DEFAULT_FONT_HEIGHT));
        rt.set_object_member(font, "bold", Variant::Integer(0));
        rt.set_object_member(font, "italic", Variant::Integer(0));
        rt.set_object_member(font, "angle", Variant::Integer(0));
        rt.register_object_native_with_arg_count(
            font,
            "getTextWidth",
            NativeArgCount::AtLeast(1),
            font_text_width,
        );
        rt.register_object_native_with_arg_count(
            font,
            "getTextHeight",
            NativeArgCount::AtLeast(1),
            font_text_height,
        );
        rt.set_object_member(h, "font", Variant::Object(font));
        rt.set_object_member(h, "__window", Variant::self_bound(window));
        rt.set_object_member(h, "name", Variant::String(String::new()));
        for (k, v) in [
            ("hitThreshold", 16),
            ("hitType", 0),
            ("showParentHint", 1),
            ("focusable", 0),
            ("enabled", 1),
            ("face", 0),
            ("holdAlpha", 0),
            ("neutralColor", 0),
        ] {
            rt.set_object_member(h, k, Variant::Integer(v));
        }
        Ok(Variant::Void)
    });
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
            // No modal mode is admitted by this profile. Removing the absent
            // mode is valid; entering one remains an explicit capability error.
            ("removeMode", |rt, t, _| {
                let h = this_of(rt, t, "Layer.removeMode")?;
                rt.host().layer(h)?;
                Ok(Variant::Void)
            }),
            ("setMode", |_, _, _| {
                Err(unsupported(
                    "Layer.setMode",
                    "shared native-layer modal input mapping",
                ))
            }),
            ("focus", |rt, t, _| {
                let h = this_of(rt, t, "Layer.focus")?;
                if rt.object_member(h, "focusable").is_truthy() {
                    return Err(unsupported(
                        "Layer.focus",
                        "native layer keyboard focus mapping",
                    ));
                }
                Ok(Variant::Integer(0))
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
            ("adjustGamma", crate::image_api::gamma),
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
            ("update", |_, _, _| Ok(Variant::Void)),
            ("finalize", |_, _, _| Ok(Variant::Void)),
            ("setClip", |_, _, _| {
                Err(unsupported(
                    "Layer.setClip",
                    "draw-time clip rectangles on canvases",
                ))
            }),
            ("piledCopy", |_, _, _| {
                Err(unsupported(
                    "Layer.piledCopy",
                    "canvas snapshots of composed subtrees",
                ))
            }),
            ("stretchCopy", |_, _, _| {
                Err(unsupported("Layer.stretchCopy", "scaled canvas copies"))
            }),
            ("affineCopy", |_, _, _| {
                Err(unsupported("Layer.affineCopy", "affine canvas copies"))
            }),
            ("operateStretch", |_, _, _| {
                Err(unsupported("Layer.operateStretch", "scaled canvas copies"))
            }),
            ("operateAffine", |_, _, _| {
                Err(unsupported("Layer.operateAffine", "affine canvas copies"))
            }),
            ("getMainPixel", |_, _, _| {
                Err(unsupported(
                    "Layer.getMainPixel",
                    "CPU pixel readback of canvases",
                ))
            }),
            ("setMainPixel", |_, _, _| {
                Err(unsupported(
                    "Layer.setMainPixel",
                    "CPU pixel writes to canvases",
                ))
            }),
            ("beginTransition", |_, _, _| {
                Err(unsupported(
                    "Layer.beginTransition",
                    "the KiriKiri transition bridge (not implemented yet)",
                ))
            }),
            ("saveLayerImage", |_, _, _| {
                Err(unsupported("Layer.saveLayerImage", "canvas image export"))
            }),
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
                    let mut node = Some(h.0);
                    let mut depth = 0;
                    while let Some(id) = node {
                        if !rt.object_member(ObjectHandle(id), "enabled").is_truthy() {
                            return Ok(Variant::Integer(0));
                        }
                        node = rt.host().layer(ObjectHandle(id))?.parent;
                        depth += 1;
                        if depth > 64 {
                            return Err(TjsError::runtime("layer enable ancestry limit"));
                        }
                    }
                    Ok(Variant::Integer(1))
                },
                read_only,
            ),
            (
                "left",
                |rt, t| layer_get(rt, t, |l| l.left),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.left = x),
            ),
            (
                "top",
                |rt, t| layer_get(rt, t, |l| l.top),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.top = x),
            ),
            (
                "width",
                |rt, t| layer_get(rt, t, |l| l.width),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.width = x.max(0)),
            ),
            (
                "height",
                |rt, t| layer_get(rt, t, |l| l.height),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.height = x.max(0)),
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
                |_, _| Ok(Variant::String(String::new())),
                |_, _, v| {
                    if !v.to_tjs_string()?.is_empty() {
                        return Err(unsupported("Layer.hint", "shared tooltip mapping"));
                    }
                    Ok(())
                },
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
                            rt.host_mut().reset_surface(
                                h,
                                l.image_width,
                                l.image_height,
                                [0; 4],
                            )?;
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
                    layer_set(rt, t, Variant::Integer(boolean(&v) as i64), |l, x| {
                        l.visible = x != 0
                    })
                },
            ),
            (
                "opacity",
                |rt, t| layer_get(rt, t, |l| l.opacity),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.opacity = x.clamp(0, 255)),
            ),
            (
                "absolute",
                |rt, t| layer_get(rt, t, |l| l.absolute),
                |rt, t, v| layer_set(rt, t, v, |l, x| l.absolute = x),
            ),
            (
                "order",
                |rt, t| {
                    let h = this_of(rt, t, "Layer")?;
                    let parent = rt.host().layer(h)?.parent;
                    Ok(Variant::Integer(parent.map_or(0, |p| {
                        rt.host()
                            .sorted_children(p)
                            .iter()
                            .position(|c| *c == h.0)
                            .unwrap_or(0) as i64
                    })))
                },
                read_only,
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
                    if rt.host().windows.get(&l.window).and_then(|w| w.primary) == Some(h.0) {
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
                    if let Some(old) = l.parent.and_then(|id| rt.host_mut().layers.get_mut(&id)) {
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
                |rt, t| layer_get(rt, t, |l| l.parent.is_none() as i64),
                read_only,
            ),
        ],
    );
}

fn layer_get(rt: &Rt, t: Option<ObjectHandle>, f: fn(&LayerState) -> i64) -> Result<Variant> {
    let h = this_of(rt, t, "Layer")?;
    Ok(Variant::Integer(f(rt.host().layer(h)?)))
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
    if !l.blank {
        return Err(unsupported(
            "resizing a drawn Layer image",
            "canvas resize that keeps existing contents",
        ));
    }
    rt.host_mut().reset_surface(h, w, ht, [255, 255, 255, 0])
}

fn argb(c: i64, alpha: Option<i64>) -> [u8; 4] {
    let c = c as u32;
    let a = alpha.map_or((c >> 24) as u8, |a| a.clamp(0, 255) as u8);
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, a]
}

fn check_face(rt: &Rt, h: ObjectHandle, member: &str) -> Result<()> {
    let face = rt.object_member(h, "face").to_integer().unwrap_or(0);
    let hold = rt.object_member(h, "holdAlpha").to_integer().unwrap_or(0);
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
    let h = this_of(rt, t, "Layer.loadImages")?;
    let name = string(&a, 0, "Layer.loadImages")?;
    if crate::storage::save_path(&name)
        .map_err(|e| TjsError::runtime(e.to_string()))?
        .is_some()
    {
        return Err(unsupported(
            "Layer.loadImages",
            "image preparation from the Save mount",
        ));
    }
    if int(&a, 1, 0x1fffffff)? != 0x1fffffff {
        return Err(unsupported(
            "Layer.loadImages colour key",
            "colour-keyed image decoding",
        ));
    }
    let path = rt
        .host()
        .storage()?
        .resolve_with(&name, &[".tlg", ".png", ".jpg", ".jpeg", ".bmp", ".webp"])
        .ok_or_else(|| TjsError::runtime(format!("image storage `{name}` not found")))?;
    let images = rt
        .host()
        .images
        .clone()
        .ok_or_else(|| TjsError::runtime("engine image metadata unavailable"))?;
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
    Ok(Variant::Void)
}

fn layer_fill_rect(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.fillRect")?;
    if rt.object_member(h, "face").to_integer()? == 3 {
        let owner = rt.host().client;
        let rect = [
            need_int(&a, 0, "province x")?,
            need_int(&a, 1, "province y")?,
            need_int(&a, 2, "province width")?,
            need_int(&a, 3, "province height")?,
        ];
        let region = need_int(&a, 4, "province region")? as u32 & 255;
        rt.host_mut()
            .layer_mut(h)?
            .regions
            .fill(owner, rect, region)
            .map_err(|e| TjsError::runtime(e.to_string()))?;
        return Ok(Variant::Void);
    }
    check_face(rt, h, "Layer.fillRect")?;
    let [x, y, w, ht] = [
        need_int(&a, 0, "Layer.fillRect")?,
        need_int(&a, 1, "Layer.fillRect")?,
        need_int(&a, 2, "Layer.fillRect")?,
        need_int(&a, 3, "Layer.fillRect")?,
    ];
    let color = argb(need_int(&a, 4, "Layer.fillRect")?, None);
    let l = rt.host().layer(h)?.clone();
    if w <= 0 || ht <= 0 {
        return Ok(Variant::Void);
    }
    let covers = x <= 0 && y <= 0 && x + w >= l.image_width && y + ht >= l.image_height;
    let host = rt.host_mut();
    if covers {
        host.canvas_op(json!({"op":"clear","id":l.slot,"color":color}));
        host.layer_mut(h)?.blank = color[3] == 0;
        host.layer_mut(h)?.full_alpha = Some(color[3]);
    } else if color[3] == 255 {
        host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x,y,w,ht],"paint":{"kind":"solid","color":color}}}));
        host.layer_mut(h)?.blank = false;
    } else if !(color[3] == 0 && l.blank) {
        // fillRect replaces pixels; a partial translucent replace cannot be
        // expressed as a source-over draw.
        return Err(unsupported(
            "partial translucent Layer.fillRect",
            "region replace on canvases",
        ));
    }
    host.api_calls += 1;
    Ok(Variant::Void)
}

fn layer_color_rect(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.colorRect")?;
    if rt.object_member(h, "face").to_integer()? == 3 {
        let rect = [
            need_int(&a, 0, "province x")?,
            need_int(&a, 1, "province y")?,
            need_int(&a, 2, "province width")?,
            need_int(&a, 3, "province height")?,
        ];
        let region = need_int(&a, 4, "province region")? as u32 & 255;
        let owner = rt.host().client;
        rt.host_mut()
            .layer_mut(h)?
            .regions
            .fill(owner, rect, region)
            .map_err(|e| TjsError::runtime(e.to_string()))?;
        return Ok(Variant::Void);
    }
    check_face(rt, h, "Layer.colorRect")?;
    let [x, y, w, ht] = [
        need_int(&a, 0, "Layer.colorRect")?,
        need_int(&a, 1, "Layer.colorRect")?,
        need_int(&a, 2, "Layer.colorRect")?,
        need_int(&a, 3, "Layer.colorRect")?,
    ];
    let opa = int(&a, 5, 255)?;
    if opa < 0 {
        return Err(unsupported(
            "Layer.colorRect with negative opacity",
            "subtractive canvas draws",
        ));
    }
    let color = argb(need_int(&a, 4, "Layer.colorRect")?, Some(opa));
    if w <= 0 || ht <= 0 || opa == 0 {
        return Ok(Variant::Void);
    }
    let slot = rt.host().layer(h)?.slot;
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
    let h = this_of(rt, t, member)?;
    check_face(rt, h, member)?;
    let (dx, dy) = (need_int(&a, 0, member)?, need_int(&a, 1, member)?);
    let src = a
        .get(2)
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime(format!("{member}: source must be a Layer")))?;
    let [sx, sy, sw, sh] = [
        need_int(&a, 3, member)?,
        need_int(&a, 4, member)?,
        need_int(&a, 5, member)?,
        need_int(&a, 6, member)?,
    ];
    if !replace {
        let mode = int(&a, 7, 128)?;
        let opa = int(&a, 8, 255)?;
        if !matches!(mode, 2 | 128) {
            return Err(unsupported(
                &format!("{member} mode {mode}"),
                "canvas blend operation modes",
            ));
        }
        if opa != 255 {
            return Err(unsupported(
                &format!("{member} opacity {opa}"),
                "isolated canvas copy opacity",
            ));
        }
    }
    if sw <= 0 || sh <= 0 {
        return Ok(Variant::Void);
    }
    let source = rt.host().layer(src)?.clone();
    let dest = rt.host().layer(h)?.clone();
    // A complete destination replacement can select one cell from a larger
    // source atlas. The shared Canvas copy clips/snapshots that cell itself.
    let whole = [dx, dy] == [0, 0]
        && [sw, sh] == [dest.image_width, dest.image_height]
        && sx >= 0
        && sy >= 0
        && sx.checked_add(sw).is_some_and(|r| r <= source.image_width)
        && sy.checked_add(sh).is_some_and(|b| b <= source.image_height);
    if replace && !whole && !dest.blank {
        return Err(unsupported(
            "partial Layer.copyRect onto drawn pixels",
            "region replace on canvases",
        ));
    }
    let host = rt.host_mut();
    host.canvas_op(
        json!({"op":"copy","source":source.slot,"destination":dest.slot,
        "rect":[sx,sy,sw,sh],"position":[dx,dy],"opacity":255,"replace":replace && whole}),
    );
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

fn layer_assign_images(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.assignImages")?;
    let src = a
        .first()
        .and_then(object_arg)
        .map(|s| rt.bound_this(s).unwrap_or(s))
        .ok_or_else(|| TjsError::runtime("Layer.assignImages: source must be a Layer"))?;
    let source = rt.host().layer(src)?.clone();
    rt.host_mut().reset_surface(
        h,
        source.image_width,
        source.image_height,
        [255, 255, 255, 0],
    )?;
    let host = rt.host_mut();
    let slot = host.layer(h)?.slot;
    host.canvas_op(json!({"op":"copy","source":source.slot,"destination":slot,
        "rect":[0,0,source.image_width,source.image_height],"position":[0,0],"opacity":255,"replace":true}));
    host.layer_mut(h)?.blank = source.blank;
    host.layer_mut(h)?.full_alpha = source.full_alpha;
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
    let color = argb(need_int(&a, 3, "Layer.drawText")?, Some(opa));
    let shadow_level = int(&a, 6, 0)?;
    let blur = int(&a, 8, 0)?;
    if shadow_level != 0 && blur != 0 {
        return Err(unsupported(
            "Layer.drawText blurred shadow/outline",
            "shared retained glyph coverage effects",
        ));
    }
    if text.is_empty() || opa <= 0 {
        return Ok(Variant::Void);
    }
    let font = object_arg(&rt.object_member(h, "font"))
        .ok_or_else(|| TjsError::runtime("Layer.font missing"))?;
    let size = font_size(rt, font)?;
    let l = rt.host().layer(h)?.clone();
    let bold = rt.object_member(font, "bold").is_truthy();
    if !bold && shadow_level == 0 {
        let alias = rt.host().font.clone();
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
        .position_simple(&host.font, size as u16, &text)
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    let glyphs = placements
        .into_iter()
        .map(|g| json!({"character":g.character.to_string(),"baseline":g.baseline}))
        .collect::<Vec<_>>();
    let bold = rt.object_member(font, "bold").is_truthy();
    let alias = rt.host().font.clone();
    let host = rt.host_mut();
    if host.layer(h)?.full_alpha != Some(255) {
        host.layer_mut(h)?.full_alpha = None;
    }
    if shadow_level != 0 {
        let offset = [int(&a, 9, 0)?, int(&a, 10, 0)?];
        let shadow = argb(int(&a, 7, 0)?, Some(opa * shadow_level.clamp(0, 255) / 255));
        host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x+offset[0],y+offset[1],l.image_width.max(1),l.image_height.max(1)],"paint":{"kind":"glyph_run","font":alias,"size":size,"color":shadow,"bold":bold,"glyphs":glyphs}}}));
    }
    host.canvas_op(json!({"op":"draw","id":l.slot,"layer":{"id":0,"rect":[x,y,l.image_width.max(1),l.image_height.max(1)],"paint":{"kind":"glyph_run","font":alias,"size":size,"color":color,"bold":bold,"glyphs":glyphs}}}));
    host.layer_mut(h)?.blank = false;
    host.api_calls += 1;
    Ok(Variant::Void)
}
