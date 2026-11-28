// SPDX-License-Identifier: MPL-2.0
//! The pygame layer: pygame_sdl2 (every release) and its vendored copy,
//! `renpy.pygame` (8.5+). Both names resolve to the same module objects;
//! module and class names follow the release (`renpy.pygame.*` from 8.5,
//! `pygame_sdl2.*` before), so pickles of `Rect`/`Color` keep upstream's
//! module paths. Ren'Py 8.6 vendors an SDL3 build, so its `locals` values
//! and event names are SDL3's; earlier releases use SDL2's.
//!
//! This file owns the package roots, `error`, `locals`, `rect`, `color`,
//! `power`, `version`, `compat` and `sysfont`; `events.rs` owns the event,
//! key, mouse, time, scrap, display, joystick and controller modules;
//! `pygame_surface.rs` the pixel modules (surface, image, transform, draw,
//! gfxdraw); `pygame_io.rs` the stream modules (rwobject, iostream).
//!
//! Unavailable host capabilities (CrySirror owns the window and devices):
//! `power.get_power_info` reports `POWERSTATE_UNKNOWN` with -1 seconds and
//! percent, which is what SDL reports when it cannot determine the power
//! state. `sysfont` cannot enumerate host fonts (no font directory or
//! `fc-list` is reachable from the script runtime), so the system font
//! dictionaries stay empty and lookups behave as upstream does on a system
//! without fonts.
use super::py::{self, *};
use super::pygame_tables as tables;
use crysirror_python::{ObjRef, Payload, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::cell::RefCell;
use std::rc::Rc;

// Release gates -------------------------------------------------------------------

/// The running release (major, minor); the newest when no runtime is set.
pub fn release(vm: &Vm) -> (u32, u32) {
    super::state::try_rt(vm)
        .map(|r| (r.profile.version[0], r.profile.version[1]))
        .unwrap_or((8, 6))
}

/// Ren'Py 8.6 vendors an SDL3 pygame.
pub fn sdl3(vm: &Vm) -> bool {
    release(vm) >= (8, 6)
}

/// `renpy.pygame` exists from 8.5.
pub fn vendored(vm: &Vm) -> bool {
    release(vm) >= (8, 5)
}

/// The canonical package name of the running release.
pub fn base(vm: &Vm) -> &'static str {
    if vendored(vm) { "renpy.pygame" } else { "pygame_sdl2" }
}

pub fn modname(vm: &Vm, sub: &str) -> String {
    if sub.is_empty() {
        base(vm).to_owned()
    } else {
        format!("{}.{sub}", base(vm))
    }
}

/// Whether submodule `sub` exists in the running release.
pub fn available(vm: &Vm, sub: &str) -> bool {
    match sub {
        "compat" | "pygame_time" | "rwobject" | "sysfont" | "version" => !sdl3(vm),
        "iostream" => sdl3(vm),
        _ => true,
    }
}

/// A module under its canonical name, also entered in `sys.modules` under
/// its other names so either import path finds the same object.
pub fn new_mod(vm: &mut Vm, sub: &str, funcs: &[(&'static str, crysirror_python::object::NativeFn)]) -> ObjRef {
    let name = modname(vm, sub);
    let m = crysirror_python::modules::module_with(vm, &name, funcs);
    let mut names = vec![if sub.is_empty() { "pygame_sdl2".to_owned() } else { format!("pygame_sdl2.{sub}") }];
    if vendored(vm) {
        names.push(if sub.is_empty() { "renpy.pygame".to_owned() } else { format!("renpy.pygame.{sub}") });
    }
    for n in names {
        let k = vm.types.new_string(n);
        vm.modules.dict_set_raw(&vm.types, k, Value::Obj(m.clone()));
    }
    if sub.is_empty() {
        let p = vm.types.new_list(Vec::new());
        crysirror_python::modules::set(vm, &m, "__path__", p);
        // The vendored package is the `pygame` attribute of `renpy`, as an
        // import through either name leaves it.
        if vendored(vm)
            && let Some(Value::Obj(r)) = vm.modules.dict_get_str_raw("renpy")
        {
            crysirror_python::modules::set(vm, &r, "pygame", Value::Obj(m.clone()));
        }
    }
    m
}

pub fn set(vm: &Vm, m: &ObjRef, name: &str, v: Value) {
    crysirror_python::modules::set(vm, m, name, v);
}

/// `name` of submodule `sub` (importing it).
pub fn attr(vm: &mut Vm, sub: &str, name: &str) -> PyResult<Value> {
    if sub.is_empty() {
        return py::import_attr(vm, "pygame_sdl2", name);
    }
    py::import_attr(vm, &format!("pygame_sdl2.{sub}"), name)
}

pub fn class_obj(vm: &mut Vm, sub: &str, name: &str) -> PyResult<ObjRef> {
    let c = attr(vm, sub, name)?;
    c.obj().cloned().ok_or_else(|| vm.type_error(format!("{name} is not a class")))
}

// Shared state ----------------------------------------------------------------------

#[derive(Default)]
pub struct PgState {
    /// `SDL_GetError()`.
    pub last_error: String,
}

pub fn pg(vm: &Vm) -> Rc<RefCell<PgState>> {
    vm.ext_get::<Rc<RefCell<PgState>>>().expect("pygame state").clone()
}

// error -----------------------------------------------------------------------------

/// `pygame_sdl2.error(msg)` as a raisable error.
pub fn err(vm: &mut Vm, msg: impl Into<String>) -> PyErr {
    let msg = msg.into();
    pg(vm).borrow_mut().last_error = msg.clone();
    let s = vm.types.new_string(msg.clone());
    match attr(vm, "error", "error").and_then(|c| vm.call(&c, vec![s], Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        _ => vm.runtime_error(msg),
    }
}

fn error_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let msg = match arg(&a[1..], kw, 0, "message") {
        Some(m) if !py::is_none(m) => m.clone(),
        _ => {
            let e = pg(vm).borrow().last_error.clone();
            vm.types.new_string(e)
        }
    };
    let re = Value::Obj(vm.types.exc("RuntimeError"));
    let init = py::get(vm, &re, "__init__")?;
    vm.call(&init, vec![a[0].clone(), msg], Vec::new())
}

fn init_error(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "error",
        &[
            ("get_error", |vm, _, _| {
                let e = pg(vm).borrow().last_error.clone();
                Ok(vm.types.new_string(e))
            }),
            ("set_error", |vm, a, _| {
                let v = a.first().cloned().unwrap_or(Value::None);
                let s = match ops::bytes_of(&v) {
                    Some(b) => String::from_utf8_lossy(&b).into_owned(),
                    None => py::string(vm, &v)?,
                };
                pg(vm).borrow_mut().last_error = s;
                Ok(Value::None)
            }),
        ],
    );
    let re = Value::Obj(vm.types.exc("RuntimeError"));
    let mn = modname(vm, "error");
    let c = py::class(vm, &mn, "error", &[re], &[("__init__", error_init)], &[])?;
    set(vm, &m, "error", Value::Obj(c));
    Ok(m)
}

// locals ----------------------------------------------------------------------------

/// The integer constants of `locals` in the running release.
pub fn locals_ints(vm: &Vm) -> &'static [(&'static str, i64)] {
    if sdl3(vm) { tables::SDL3_LOCALS } else { tables::SDL2_LOCALS }
}

pub fn local(vm: &Vm, name: &str) -> i64 {
    locals_ints(vm).iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or(0)
}

fn sdl_version_tuple(vm: &Vm) -> Value {
    if sdl3(vm) {
        let (a, b) = tables::SDL3_VERSION;
        vm.types.new_tuple(vec![Value::Int(a), Value::Int(b)])
    } else {
        let (a, b, c) = tables::SDL2_VERSION;
        vm.types.new_tuple(vec![Value::Int(a), Value::Int(b), Value::Int(c)])
    }
}

/// Every `locals` name and value (`from locals import *`).
fn locals_items(vm: &mut Vm) -> PyResult<Vec<(String, Value)>> {
    let mut out: Vec<(String, Value)> = locals_ints(vm).iter().map(|(n, v)| ((*n).to_owned(), Value::Int(*v))).collect();
    if sdl3(vm) {
        for (n, v) in tables::SDL3_STR_LOCALS {
            out.push(((*n).to_owned(), vm.types.new_str(v)));
        }
    } else {
        out.push(("SCRAP_TEXT".into(), vm.types.new_str("text/plain")));
    }
    out.push(("SDL_VERSION_TUPLE".into(), sdl_version_tuple(vm)));
    out.push(("Rect".into(), attr(vm, "rect", "Rect")?));
    out.push(("Color".into(), attr(vm, "color", "Color")?));
    Ok(out)
}

fn init_locals(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "locals", &[]);
    for (n, v) in locals_items(vm)? {
        set(vm, &m, &n, v);
    }
    Ok(m)
}

// Package root ----------------------------------------------------------------------

fn root_register(vm: &mut Vm, a: &[Value], _: Kw, list: &str) -> PyResult<Value> {
    let f = a.first().cloned().unwrap_or(Value::None);
    let l = py::import_attr(vm, "pygame_sdl2", list)?;
    py::method(vm, &l, "append", vec![f.clone()])?;
    Ok(f)
}

fn root_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let fs = py::import_attr(vm, "pygame_sdl2", "init_functions")?;
    let fs = py::items(vm, &fs)?;
    let (mut pass, mut fail) = (0, 0);
    let reraise = sdl3(vm);
    for f in fs {
        match vm.call(&f, Vec::new(), Vec::new()) {
            Ok(_) => pass += 1,
            Err(PyErr::Exc(e)) => {
                fail += 1;
                if reraise {
                    return Err(PyErr::Exc(e));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(vm.types.new_tuple(vec![Value::Int(pass), Value::Int(fail)]))
}

fn root_quit(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let fs = py::import_attr(vm, "pygame_sdl2", "quit_functions")?;
    for f in py::items(vm, &fs)? {
        match vm.call(&f, Vec::new(), Vec::new()) {
            Ok(_) | Err(PyErr::Exc(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Value::None)
}

/// `import_as_pygame()`: enters every loaded `renpy.pygame*` module as
/// `pygame*` and `pygame_sdl2*`.
fn root_import_as_pygame(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let modules = Value::Obj(vm.modules.clone());
    let me = py::import(vm, "pygame_sdl2")?;
    let warn = |vm: &mut Vm, msg: &str| -> PyResult<()> {
        let w = py::import_attr(vm, "warnings", "warn")?;
        let m = vm.types.new_str(msg);
        vm.call(&w, vec![m], vec![(Rc::from("stacklevel"), Value::Int(2))])?;
        Ok(())
    };
    if vm.modules.dict_get_str_raw("pygame").is_some() {
        warn(vm, "Pygame has already been imported, import_as_pygame may not work.")?;
    }
    // CrySirror's runtime enters this package as pygame_sdl2 itself; only a
    // different module there is a conflicting import.
    if vm.modules.dict_get_str_raw("pygame_sdl2").is_some_and(|m| !py::same(&m, &me)) {
        warn(vm, "Pygame SDL2 has already been imported, import_as_pygame may not work.")?;
    }
    for (k, v) in ops::mapping_items(vm, &modules)? {
        let Some(name) = k.as_str().map(str::to_owned) else { continue };
        let Some(suffix) = name.strip_prefix("renpy.pygame") else { continue };
        for p in ["pygame", "pygame_sdl2"] {
            let k = vm.types.new_string(format!("{p}{suffix}"));
            vm.modules.dict_set_raw(&vm.types, k, v.clone());
        }
    }
    let constants = attr(vm, "", "constants").or_else(|_| py::import_attr(vm, "pygame_sdl2", "constants"))?;
    for n in ["pygame.constants", "pygame_sdl2.constants"] {
        let k = vm.types.new_str(n);
        vm.modules.dict_set_raw(&vm.types, k, constants.clone());
    }
    Ok(Value::None)
}

fn init_root(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "",
        &[
            ("init", root_init),
            ("quit", root_quit),
            ("import_as_pygame", root_import_as_pygame),
            ("get_sdl_byteorder", |vm, _, _| Ok(Value::Int(local(vm, "LIL_ENDIAN")))),
            ("get_sdl_version", |vm, _, _| Ok(sdl_version_tuple(vm))),
            ("get_platform", |vm, _, _| py::import_attr(vm, "pygame_sdl2.display", "get_platform").and_then(|f| vm.call(&f, Vec::new(), Vec::new()))),
        ],
    );
    if !sdl3(vm) {
        let ri = vm.types.new_builtin("register_init", |vm, a, k| root_register(vm, a, k, "init_functions"));
        set(vm, &m, "register_init", ri);
        let rq = vm.types.new_builtin("register_quit", |vm, a, k| root_register(vm, a, k, "quit_functions"));
        set(vm, &m, "register_quit", rq);
    }
    set(vm, &m, "HAVE_NEWBUF", Value::Bool(false));
    let l = vm.types.new_list(Vec::new());
    set(vm, &m, "init_functions", l);
    let l = vm.types.new_list(Vec::new());
    set(vm, &m, "quit_functions", l);
    // `from .error import *`
    for n in ["error", "get_error", "set_error"] {
        let v = attr(vm, "error", n)?;
        set(vm, &m, n, v);
    }
    let mut subs = vec![
        "rect", "surface", "color", "display", "event", "key", "locals", "time", "controller", "draw", "image",
        "joystick", "mouse", "power", "transform", "scrap",
    ];
    if !sdl3(vm) {
        subs.extend(["version", "sysfont"]);
    }
    for sub in subs {
        let sm = py::import(vm, &format!("pygame_sdl2.{sub}"))?;
        set(vm, &m, sub, sm.clone());
        if sub == "locals" {
            set(vm, &m, "constants", sm);
        }
    }
    let s = attr(vm, "surface", "Surface")?;
    set(vm, &m, "Surface", s);
    for (n, v) in locals_items(vm)? {
        set(vm, &m, &n, v);
    }
    // The subsystem init/quit functions (8.6 lists them; earlier releases
    // register them as their modules load).
    let inits: &[(&str, &str)] = if sdl3(vm) {
        &[
            ("surface", "_init_surface_deallocation"),
            ("display", "init"),
            ("event", "init"),
            ("time", "init"),
            ("controller", "init"),
            ("joystick", "init"),
            ("mouse", "init"),
            ("scrap", "init"),
        ]
    } else {
        &[("display", "init"), ("event", "init"), ("time", "init"), ("controller", "init"), ("joystick", "init"), ("image", "init")]
    };
    let quits: &[(&str, &str)] = if sdl3(vm) {
        &[
            ("scrap", "quit"),
            ("mouse", "quit"),
            ("joystick", "quit"),
            ("controller", "quit"),
            ("surface", "_quit_surface_deallocation"),
            ("display", "quit"),
        ]
    } else {
        &[("image", "quit"), ("joystick", "quit"), ("controller", "quit"), ("time", "quit"), ("display", "quit")]
    };
    for (list, fs) in [("init_functions", inits), ("quit_functions", quits)] {
        let l = py::get_raw(&m, list);
        for (sub, f) in fs {
            let v = attr(vm, sub, f)?;
            py::method(vm, &l, "append", vec![v])?;
        }
    }
    Ok(m)
}

// rect ------------------------------------------------------------------------------

pub struct RectData(pub [i32; 4]);

pub fn rect_of(v: &Value) -> Option<[i32; 4]> {
    py::native::<RectData, _>(v, |r| r.0)
}

fn rect_store(v: &Value, r: [i32; 4]) {
    py::native::<RectData, _>(v, |d| d.0 = r);
}

fn rect_class(vm: &mut Vm) -> PyResult<ObjRef> {
    class_obj(vm, "rect", "Rect")
}

pub fn new_rect(vm: &mut Vm, r: [i32; 4]) -> PyResult<Value> {
    let c = rect_class(vm)?;
    Ok(vm.types.new_native(&c, Box::new(RectData(r))))
}

pub fn i32_of(vm: &mut Vm, v: &Value) -> PyResult<i32> {
    match v {
        Value::Int(i) => Ok(*i as i32),
        Value::Bool(b) => Ok(*b as i32),
        Value::Float(f) => Ok(f.trunc() as i32),
        _ => Ok(py::int(vm, v)? as i32),
    }
}

fn pair(vm: &mut Vm, v: &Value) -> PyResult<(Value, Value)> {
    let it = py::items(vm, v)?;
    if it.len() != 2 {
        return Err(vm.value_error(format!("expected 2 values to unpack, got {}", it.len())));
    }
    Ok((it[0].clone(), it[1].clone()))
}

/// `Rect(*args)` argument forms.
pub fn rect_from_args(vm: &mut Vm, args: &[Value]) -> PyResult<[i32; 4]> {
    let bad = |vm: &Vm| vm.type_error("Argument must be a rect style object.");
    match args.len() {
        1 => {
            if let Some(r) = rect_of(&args[0]) {
                return Ok(r);
            }
            let it = py::items(vm, &args[0]).map_err(|_| bad(vm))?;
            match it.len() {
                4 => Ok([i32_of(vm, &it[0])?, i32_of(vm, &it[1])?, i32_of(vm, &it[2])?, i32_of(vm, &it[3])?]),
                2 => Ok([i32_of(vm, &it[0])?, i32_of(vm, &it[1])?, 0, 0]),
                _ => Err(bad(vm)),
            }
        }
        2 => {
            let (x, y) = pair(vm, &args[0])?;
            let (w, h) = pair(vm, &args[1])?;
            Ok([i32_of(vm, &x)?, i32_of(vm, &y)?, i32_of(vm, &w)?, i32_of(vm, &h)?])
        }
        4 => Ok([i32_of(vm, &args[0])?, i32_of(vm, &args[1])?, i32_of(vm, &args[2])?, i32_of(vm, &args[3])?]),
        _ => Err(bad(vm)),
    }
}

/// A rect-style value (`Rect`, 4-sequence or 2-sequence position).
pub fn rect_like(vm: &mut Vm, v: &Value) -> PyResult<[i32; 4]> {
    rect_from_args(vm, std::slice::from_ref(v))
}

/// `to_sdl_rect`: a 2-sequence is (x, y) with w, h also taken from it.
pub fn sdl_rect(vm: &mut Vm, v: &Value, argname: Option<&str>) -> PyResult<[i32; 4]> {
    if let Some(r) = rect_of(v) {
        return Ok(r);
    }
    let fail = |vm: &Vm| match argname {
        Some(a) => vm.type_error(format!("Argument {a} must be a rect style object.")),
        None => vm.type_error("Argument must be a rect style object."),
    };
    let Ok(it) = py::items(vm, v) else { return Err(fail(vm)) };
    let conv = |vm: &mut Vm, it: &[Value]| -> PyResult<Vec<i32>> { it.iter().map(|x| i32_of(vm, x)).collect() };
    match it.len() {
        4 => conv(vm, &it).map(|c| [c[0], c[1], c[2], c[3]]).map_err(|_| fail(vm)),
        2 => conv(vm, &it).map(|c| [c[0], c[1], c[0], c[1]]).map_err(|_| fail(vm)),
        _ => Err(fail(vm)),
    }
}

fn this_rect(vm: &Vm, v: &Value) -> PyResult<[i32; 4]> {
    rect_of(v).ok_or_else(|| vm.type_error("descriptor requires a 'Rect' object"))
}

fn fl(x: f64) -> Value {
    Value::Float(x)
}

fn rect_get_attr(vm: &Vm, r: [i32; 4], name: &str) -> Value {
    let [x, y, w, h] = r;
    let cx = x as f64 + w as f64 / 2.;
    let cy = y as f64 + h as f64 / 2.;
    let t = |a: Value, b: Value| vm.types.new_tuple(vec![a, b]);
    let i = |v: i32| Value::Int(v as i64);
    match name {
        "x" | "left" => i(x),
        "y" | "top" => i(y),
        "w" | "width" => i(w),
        "h" | "height" => i(h),
        "right" => i(x + w),
        "bottom" => i(y + h),
        "size" => t(i(w), i(h)),
        "topleft" => t(i(x), i(y)),
        "topright" => t(i(x + w), i(y)),
        "bottomright" => t(i(x + w), i(y + h)),
        "bottomleft" => t(i(x), i(y + h)),
        "centerx" => fl(cx),
        "centery" => fl(cy),
        "center" => t(fl(cx), fl(cy)),
        "midtop" => t(fl(cx), i(y)),
        "midleft" => t(i(x), fl(cy)),
        "midbottom" => t(fl(cx), i(y + h)),
        "midright" => t(i(x + w), fl(cy)),
        _ => Value::None,
    }
}

/// `x += val - cur` with C int truncation.
fn shift(vm: &mut Vm, x: i32, val: &Value, cur: f64) -> PyResult<i32> {
    match val {
        Value::Int(v) if cur.fract() == 0. => Ok((x as i64 + v - cur as i64) as i32),
        _ => {
            let v = py::f64_of(vm, val)?;
            Ok((x as f64 + (v - cur)).trunc() as i32)
        }
    }
}

fn rect_set_attr(vm: &mut Vm, r: &mut [i32; 4], name: &str, v: &Value) -> PyResult<()> {
    match name {
        "x" | "left" => r[0] = i32_of(vm, v)?,
        "y" | "top" => r[1] = i32_of(vm, v)?,
        "w" | "width" => r[2] = i32_of(vm, v)?,
        "h" | "height" => r[3] = i32_of(vm, v)?,
        "right" => r[0] = shift(vm, r[0], v, (r[0] + r[2]) as f64)?,
        "bottom" => r[1] = shift(vm, r[1], v, (r[1] + r[3]) as f64)?,
        "centerx" => r[0] = shift(vm, r[0], v, r[0] as f64 + r[2] as f64 / 2.)?,
        "centery" => r[1] = shift(vm, r[1], v, r[1] as f64 + r[3] as f64 / 2.)?,
        _ => {
            let (a, b) = match name {
                "size" => ("w", "h"),
                "topleft" => ("left", "top"),
                "topright" => ("right", "top"),
                "bottomright" => ("right", "bottom"),
                "bottomleft" => ("left", "bottom"),
                "center" => ("centerx", "centery"),
                "midtop" => ("centerx", "top"),
                "midleft" => ("left", "centery"),
                "midbottom" => ("centerx", "bottom"),
                "midright" => ("right", "centery"),
                _ => return Err(vm.attr_error(format!("'Rect' object has no attribute '{name}'"))),
            };
            let (p, q) = pair(vm, v)?;
            rect_set_attr(vm, r, a, &p)?;
            rect_set_attr(vm, r, b, &q)?;
        }
    }
    Ok(())
}

const RECT_ATTRS: &[&str] = &[
    "x", "y", "w", "h", "left", "top", "width", "height", "right", "bottom", "size", "topleft", "topright",
    "bottomright", "bottomleft", "centerx", "centery", "center", "midtop", "midleft", "midbottom", "midright",
];

fn rect_prop_get(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let name = a[0].as_str().unwrap_or("").to_owned();
    let r = this_rect(vm, &a[1])?;
    Ok(rect_get_attr(vm, r, &name))
}

fn rect_prop_set(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let name = a[0].as_str().unwrap_or("").to_owned();
    let mut r = this_rect(vm, &a[1])?;
    rect_set_attr(vm, &mut r, &name, &a[2])?;
    rect_store(&a[1], r);
    Ok(Value::None)
}

fn rect_new(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = a.first().and_then(|c| c.obj().cloned()).ok_or_else(|| vm.type_error("Rect.__new__(X): X is not a type"))?;
    Ok(vm.types.new_native(&c, Box::new(RectData([0; 4]))))
}

fn rect_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let r = rect_from_args(vm, &a[1..])?;
    rect_store(&a[0], r);
    Ok(Value::None)
}

fn clip(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    if !collide(a, b) {
        return [0; 4];
    }
    let mut r = a;
    if r[0] < b[0] {
        let d = b[0] - r[0];
        r[0] += d;
        r[2] -= d;
    }
    if r[0] + r[2] > b[0] + b[2] {
        r[2] -= r[0] + r[2] - (b[0] + b[2]);
    }
    if r[1] < b[1] {
        let d = b[1] - r[1];
        r[1] += d;
        r[3] -= d;
    }
    if r[1] + r[3] > b[1] + b[3] {
        r[3] -= r[1] + r[3] - (b[1] + b[3]);
    }
    r
}

pub fn clip_rect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    clip(a, b)
}

fn collide(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && a[1] < b[1] + b[3] && a[0] + a[2] > b[0] && a[1] + a[3] > b[1]
}

pub fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [x, y, (a[0] + a[2]).max(b[0] + b[2]) - x, (a[1] + a[3]).max(b[1] + b[3]) - y]
}

/// `flatten(*args)` of rect.pyx, unpacked to two numbers.
fn flat_xy(vm: &mut Vm, a: &[Value]) -> PyResult<(Value, Value)> {
    if a.len() == 1 {
        pair(vm, &a[0])
    } else if a.len() == 2 {
        Ok((a[0].clone(), a[1].clone()))
    } else {
        Err(vm.value_error(format!("expected 2 values to unpack, got {}", a.len())))
    }
}

fn add_i(vm: &mut Vm, x: i32, v: &Value) -> PyResult<i32> {
    match v {
        Value::Int(i) => Ok((x as i64 + i) as i32),
        _ => {
            let f = py::f64_of(vm, v)?;
            Ok((x as f64 + f).trunc() as i32)
        }
    }
}

fn move_ip(vm: &mut Vm, r: &mut [i32; 4], a: &[Value]) -> PyResult<()> {
    let (x, y) = flat_xy(vm, a)?;
    r[0] = add_i(vm, r[0], &x)?;
    r[1] = add_i(vm, r[1], &y)?;
    Ok(())
}

fn inflate_ip(vm: &mut Vm, r: &mut [i32; 4], a: &[Value]) -> PyResult<()> {
    let (x, y) = flat_xy(vm, a)?;
    let c = rect_get_attr(vm, *r, "center");
    r[2] = add_i(vm, r[2], &x)?;
    r[3] = add_i(vm, r[3], &y)?;
    rect_set_attr(vm, r, "center", &c)
}

fn clamp_ip(vm: &mut Vm, r: &mut [i32; 4], other: &Value) -> PyResult<()> {
    let o = rect_like(vm, other)?;
    if r[2] > o[2] || r[3] > o[3] {
        let c = rect_get_attr(vm, o, "center");
        rect_set_attr(vm, r, "center", &c)?;
    } else {
        if r[0] < o[0] {
            r[0] = o[0];
        } else if r[0] + r[2] > o[0] + o[2] {
            r[0] = o[0] + o[2] - r[2];
        }
        if r[1] < o[1] {
            r[1] = o[1];
        } else if r[1] + r[3] > o[1] + o[3] {
            r[1] = o[1] + o[3] - r[3];
        }
    }
    Ok(())
}

fn rect_method(vm: &mut Vm, a: &[Value], kw: Kw, name: &str) -> PyResult<Value> {
    let this = &a[0];
    let mut r = this_rect(vm, this)?;
    let rest = &a[1..];
    let one = |vm: &Vm| rest.first().cloned().ok_or_else(|| vm.type_error(format!("{name}() missing argument")));
    match name {
        "__reduce__" => {
            let c = rect_class(vm)?;
            let args = vm.types.new_tuple(r.iter().map(|v| Value::Int(*v as i64)).collect());
            Ok(vm.types.new_tuple(vec![Value::Obj(c), args]))
        }
        "__repr__" => Ok(vm.types.new_string(format!("<rect({}, {}, {}, {})>", r[0], r[1], r[2], r[3]))),
        "__len__" => Ok(Value::Int(4)),
        "__iter__" => {
            let t = vm.types.new_tuple(r.iter().map(|v| Value::Int(*v as i64)).collect());
            let it = vm.types.new_builtin("iter", |vm, a, _| ops::iter(vm, &a[0]));
            vm.call(&it, vec![t], Vec::new())
        }
        "__getitem__" => {
            let t = vm.types.new_tuple(r.iter().map(|v| Value::Int(*v as i64)).collect());
            ops::getitem(vm, &t, one(vm)?)
        }
        "__setitem__" => {
            let k = py::int(vm, &one(vm)?)?;
            let v = rest.get(1).cloned().unwrap_or(Value::None);
            if !(0..4).contains(&k) {
                return Err(vm.index_error(k.to_string()));
            }
            r[k as usize] = i32_of(vm, &v)?;
            rect_store(this, r);
            Ok(Value::None)
        }
        "__eq__" | "__ne__" => {
            let o = rect_like(vm, &one(vm)?)?;
            Ok(Value::Bool((o == r) == (name == "__eq__")))
        }
        "copy" => new_rect(vm, r),
        "move" => {
            move_ip(vm, &mut r, rest)?;
            new_rect(vm, r)
        }
        "move_ip" => {
            move_ip(vm, &mut r, rest)?;
            rect_store(this, r);
            Ok(Value::None)
        }
        "inflate" => {
            inflate_ip(vm, &mut r, rest)?;
            new_rect(vm, r)
        }
        "inflate_ip" => {
            inflate_ip(vm, &mut r, rest)?;
            rect_store(this, r);
            Ok(Value::None)
        }
        "clamp" => {
            clamp_ip(vm, &mut r, &one(vm)?)?;
            new_rect(vm, r)
        }
        "clamp_ip" => {
            clamp_ip(vm, &mut r, &one(vm)?)?;
            rect_store(this, r);
            Ok(Value::None)
        }
        "clip" => {
            let other = one(vm)?;
            let o = if matches!(other, Value::Int(_)) {
                let mut v = vec![other];
                for (i, n) in ["y", "w", "h"].iter().enumerate() {
                    v.push(py::arg_or(rest, kw, i + 1, n, Value::None));
                }
                rect_from_args(vm, &v)?
            } else {
                rect_like(vm, &other)?
            };
            new_rect(vm, clip(r, o))
        }
        "union" | "union_ip" => {
            let o = rect_like(vm, &one(vm)?)?;
            let u = union(r, o);
            if name == "union" {
                new_rect(vm, u)
            } else {
                rect_store(this, u);
                Ok(Value::None)
            }
        }
        "unionall" | "unionall_ip" => {
            for o in py::items(vm, &one(vm)?)? {
                let o = rect_like(vm, &o)?;
                r = union(r, o);
            }
            if name == "unionall" {
                new_rect(vm, r)
            } else {
                rect_store(this, r);
                Ok(Value::None)
            }
        }
        "fit" => {
            let o = rect_like(vm, &one(vm)?)?;
            let wr = o[2] as f64 / r[2] as f64;
            let hr = o[3] as f64 / r[3] as f64;
            let f = wr.min(hr);
            new_rect(vm, [o[0], o[1], (r[2] as f64 * f).trunc() as i32, (r[3] as f64 * f).trunc() as i32])
        }
        "normalize" => {
            if r[2] < 0 {
                r[0] += r[2];
                r[2] = -r[2];
            }
            if r[3] < 0 {
                r[1] += r[3];
                r[3] = -r[3];
            }
            rect_store(this, r);
            Ok(Value::None)
        }
        "contains" => {
            let o = rect_like(vm, &one(vm)?)?;
            Ok(Value::Bool(
                o[0] >= r[0]
                    && o[0] + o[2] <= r[0] + r[2]
                    && o[1] >= r[1]
                    && o[1] + o[3] <= r[1] + r[3]
                    && o[0] < r[0] + r[2]
                    && o[1] < r[1] + r[3],
            ))
        }
        "collidepoint" => {
            let x = one(vm)?;
            let (x, y) = if matches!(x.payload(), Some(Payload::Tuple(_))) {
                pair(vm, &x)?
            } else {
                (x, py::arg_or(rest, kw, 1, "y", Value::None))
            };
            let (x, y) = (py::f64_of(vm, &x)?, py::f64_of(vm, &y)?);
            Ok(Value::Bool(
                x >= r[0] as f64 && y >= r[1] as f64 && x < (r[0] + r[2]) as f64 && y < (r[1] + r[3]) as f64,
            ))
        }
        "colliderect" => {
            let o = rect_like(vm, &one(vm)?)?;
            Ok(Value::Bool(collide(r, o)))
        }
        "collidelist" | "collidelistall" => {
            let mut hits = Vec::new();
            for (n, o) in py::items(vm, &one(vm)?)?.into_iter().enumerate() {
                let o = rect_like(vm, &o)?;
                if collide(r, o) {
                    if name == "collidelist" {
                        return Ok(Value::Int(n as i64));
                    }
                    hits.push(Value::Int(n as i64));
                }
            }
            Ok(if name == "collidelist" { Value::Int(-1) } else { vm.types.new_list(hits) })
        }
        "collidedict" | "collidedictall" => {
            let mut hits = Vec::new();
            let d = one(vm)?;
            for (k, v) in ops::mapping_items(vm, &d)? {
                let o = rect_like(vm, &v)?;
                if collide(r, o) {
                    let t = vm.types.new_tuple(vec![k, v]);
                    if name == "collidedict" {
                        return Ok(t);
                    }
                    hits.push(t);
                }
            }
            Ok(if name == "collidedict" { Value::None } else { vm.types.new_list(hits) })
        }
        _ => Err(vm.attr_error(name.to_owned())),
    }
}

macro_rules! dispatch {
    ($target:ident; $($name:literal),* $(,)?) => {
        &[$(($name, |vm: &mut Vm, a: &[Value], k: Kw| $target(vm, a, k, $name))),*]
    };
}

const RECT_METHODS: &[(&str, crysirror_python::object::NativeFn)] = dispatch!(rect_method;
    "__reduce__", "__repr__", "__len__", "__iter__", "__getitem__", "__setitem__", "__eq__", "__ne__", "copy",
    "move", "move_ip", "inflate", "inflate_ip", "clamp", "clamp_ip", "clip", "union", "union_ip", "unionall",
    "unionall_ip", "fit", "normalize", "contains", "collidepoint", "colliderect", "collidelist",
    "collidelistall", "collidedict", "collidedictall",
);

fn init_rect(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "rect",
        &[("flatten", |vm, a, _| Ok(if a.len() == 1 { a[0].clone() } else { vm.types.new_tuple(a.to_vec()) }))],
    );
    let mut attrs: Vec<(&str, Value)> = Vec::new();
    for n in RECT_ATTRS {
        let name = vm.types.new_str(n);
        let g = py::closure(vm, "rect_get", rect_prop_get, name.clone());
        let s = py::closure(vm, "rect_set", rect_prop_set, name);
        attrs.push((n, py::property(vm, g, Some(s))?));
    }
    let new = py::builtin(vm, "__new__", rect_new);
    attrs.push(("__new__", py::staticmethod(vm, new)));
    attrs.push(("__hash__", Value::None));
    let mut methods = RECT_METHODS.to_vec();
    methods.push(("__init__", rect_init));
    let mn = modname(vm, "rect");
    let c = py::class(vm, &mn, "Rect", &[], &methods, &attrs)?;
    set(vm, &m, "Rect", Value::Obj(c));
    Ok(m)
}

// color -----------------------------------------------------------------------------

pub struct ColorData {
    pub rgba: [u8; 4],
    pub len: u8,
}

pub fn color_of(v: &Value) -> Option<[u8; 4]> {
    py::native::<ColorData, _>(v, |c| c.rgba)
}

fn color_class(vm: &mut Vm) -> PyResult<ObjRef> {
    class_obj(vm, "color", "Color")
}

pub fn new_color(vm: &mut Vm, c: [u8; 4]) -> PyResult<Value> {
    let cls = color_class(vm)?;
    Ok(vm.types.new_native(&cls, Box::new(ColorData { rgba: c, len: 4 })))
}

/// Cython's `Uint8` conversion.
fn u8_of(vm: &mut Vm, v: &Value) -> PyResult<u8> {
    let i = match v {
        Value::Float(f) => f.trunc() as i64,
        _ => py::int(vm, v)?,
    };
    if !(0..=255).contains(&i) {
        return Err(vm.exc_msg(
            "OverflowError",
            if i < 0 { "can't convert negative value to Uint8" } else { "value too large to convert to Uint8" },
        ));
    }
    Ok(i as u8)
}

fn from_hex(vm: &Vm, c: &str) -> PyResult<[u8; 4]> {
    let c: String = if c.len() == 3 || c.len() == 4 { c.chars().flat_map(|x| [x, x]).collect() } else { c.to_owned() };
    let bad = || vm.value_error(c.clone());
    if !(c.len() == 6 || c.len() == 8) || !c.is_ascii() {
        return Err(bad());
    }
    let mut out = [0u8, 0, 0, 255];
    for (i, ch) in c.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(ch).map_err(|_| bad())?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

pub fn color_from_name(name: &str) -> Option<[u8; 3]> {
    let key: String = name.split_whitespace().collect::<String>().to_lowercase();
    tables::COLORS.iter().find(|(n, _)| *n == key).map(|(_, c)| *c)
}

/// `Color(*args)`.
fn color_from_args(vm: &mut Vm, args: &[Value]) -> PyResult<[u8; 4]> {
    match args.len() {
        0 => Ok([0, 0, 0, 255]),
        1 => {
            let c = &args[0];
            if let Some(s) = c.as_str() {
                if let Some(h) = s.strip_prefix('#') {
                    from_hex(vm, h)
                } else if let Some(h) = s.strip_prefix("0x") {
                    from_hex(vm, h)
                } else {
                    let [r, g, b] = color_from_name(s).ok_or_else(|| vm.value_error(s.to_owned()))?;
                    Ok([r, g, b, 255])
                }
            } else if let Some(rgba) = color_of(c) {
                Ok(rgba)
            } else if matches!(c.payload(), Some(Payload::Tuple(_) | Payload::List(_))) {
                let it = py::items(vm, c)?;
                match it.len() {
                    4 => Ok([u8_of(vm, &it[0])?, u8_of(vm, &it[1])?, u8_of(vm, &it[2])?, u8_of(vm, &it[3])?]),
                    3 => Ok([u8_of(vm, &it[0])?, u8_of(vm, &it[1])?, u8_of(vm, &it[2])?, 255]),
                    _ => {
                        let r = ops::repr(vm, c)?;
                        Err(vm.value_error(r))
                    }
                }
            } else {
                let i = py::int(vm, c)?;
                from_hex(vm, &format!("{i:08x}"))
            }
        }
        3 => Ok([u8_of(vm, &args[0])?, u8_of(vm, &args[1])?, u8_of(vm, &args[2])?, 255]),
        4 => Ok([u8_of(vm, &args[0])?, u8_of(vm, &args[1])?, u8_of(vm, &args[2])?, u8_of(vm, &args[3])?]),
        _ => Ok([0, 0, 0, 255]),
    }
}

/// A color argument (`Color`, name, hex string, 3/4-sequence or packed int).
pub fn color_like(vm: &mut Vm, v: &Value) -> PyResult<[u8; 4]> {
    color_from_args(vm, std::slice::from_ref(v))
}

fn this_color(vm: &Vm, v: &Value) -> PyResult<(([u8; 4]), u8)> {
    py::native::<ColorData, _>(v, |c| (c.rgba, c.len)).ok_or_else(|| vm.type_error("descriptor requires a 'Color' object"))
}

fn color_store(v: &Value, rgba: [u8; 4]) {
    py::native::<ColorData, _>(v, |c| c.rgba = rgba);
}

fn hsva(c: [u8; 4]) -> [f64; 4] {
    let (r, g, b) = (c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255.);
    let cmax = r.max(g).max(b);
    let cmin = r.min(g).min(b);
    let delta = cmax - cmin;
    let (h, s) = if r == g && g == b {
        (0., 0.)
    } else {
        let h = if cmax == r {
            60. * ((g - b) / delta).rem_euclid(6.)
        } else if cmax == g {
            60. * ((b - r) / delta + 2.)
        } else {
            60. * ((r - g) / delta + 4.)
        };
        (h, if cmax == 0. { 0. } else { delta / cmax * 100. })
    };
    [h, s, cmax * 100., c[3] as f64 / 255. * 100.]
}

fn hue_rgb(vm: &Vm, h: f64, c: f64, x: f64) -> PyResult<(f64, f64, f64)> {
    Ok(if (0. ..60.).contains(&h) {
        (c, x, 0.)
    } else if (60. ..120.).contains(&h) {
        (x, c, 0.)
    } else if (120. ..180.).contains(&h) {
        (0., c, x)
    } else if (180. ..240.).contains(&h) {
        (0., x, c)
    } else if (240. ..300.).contains(&h) {
        (x, 0., c)
    } else if (300. ..360.).contains(&h) {
        (c, 0., x)
    } else {
        return Err(vm.value_error(""));
    })
}

/// C double -> Uint8 store (`self.r = int(...)`).
fn store_u8(vm: &mut Vm, x: f64) -> PyResult<u8> {
    u8_of(vm, &Value::Float(x))
}

fn color_prop_get(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let name = a[0].as_str().unwrap_or("").to_owned();
    let (c, _) = this_color(vm, &a[1])?;
    let f = |v: Vec<f64>| vm.types.new_tuple(v.into_iter().map(Value::Float).collect());
    Ok(match name.as_str() {
        "r" => Value::Int(c[0] as i64),
        "g" => Value::Int(c[1] as i64),
        "b" => Value::Int(c[2] as i64),
        "a" => Value::Int(c[3] as i64),
        "cmy" => f(vec![1. - c[0] as f64 / 255., 1. - c[1] as f64 / 255., 1. - c[2] as f64 / 255.]),
        "hsva" => f(hsva(c).to_vec()),
        "hsla" => {
            let h = hsva(c)[0].rem_euclid(360.);
            let (r, g, b) = (c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255.);
            let cmax = r.max(g).max(b);
            let cmin = r.min(g).min(b);
            let delta = cmax - cmin;
            let l = (cmax + cmin) / 2.;
            let s = if delta == 0. { 0. } else { delta / (1. - (2. * l - 1.).abs()) };
            f(vec![h, (s * 100.).min(100.), (l * 100.).min(100.), c[3] as f64 / 255. * 100.])
        }
        "i1i2i3" => {
            let (r, g, b) = (c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255.);
            f(vec![(r + g + b) / 3., (r - b) / 2., (2. * g - r - b) / 4.])
        }
        _ => Value::None,
    })
}

fn four(vm: &mut Vm, v: &Value) -> PyResult<[f64; 4]> {
    let it = py::items(vm, v)?;
    let mut out = [0.; 4];
    match it.len() {
        3 | 4 => {
            for (i, x) in it.iter().enumerate() {
                out[i] = py::f64_of(vm, x)?;
            }
            Ok(out)
        }
        n => Err(vm.value_error(format!("expected 3 or 4 values, got {n}"))),
    }
}

fn color_prop_set(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let name = a[0].as_str().unwrap_or("").to_owned();
    let (mut c, _) = this_color(vm, &a[1])?;
    let v = &a[2];
    match name.as_str() {
        "r" => c[0] = u8_of(vm, v)?,
        "g" => c[1] = u8_of(vm, v)?,
        "b" => c[2] = u8_of(vm, v)?,
        "a" => c[3] = u8_of(vm, v)?,
        "cmy" => {
            let it = py::items(vm, v)?;
            if it.len() != 3 {
                return Err(vm.value_error("expected 3 values"));
            }
            for i in 0..3 {
                let x = py::f64_of(vm, &it[i])?;
                c[i] = store_u8(vm, (1. - x) * 255.)?;
            }
        }
        "hsva" | "hsla" => {
            let [h0, s, l, alpha] = four(vm, v)?;
            let (s, l, alpha) = (s / 100., l / 100., alpha / 100.);
            let (h, cc, m) = if name == "hsva" {
                let h = h0.rem_euclid(360.);
                let cc = l * s;
                (h, cc, l - cc)
            } else {
                let cc = (1. - (2. * l - 1.).abs()) * s;
                (h0, cc, l - cc / 2.)
            };
            let x = cc * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
            let (r, g, b) = hue_rgb(vm, h, cc, x)?;
            c = [
                store_u8(vm, 255. * (r + m))?,
                store_u8(vm, 255. * (g + m))?,
                store_u8(vm, 255. * (b + m))?,
                store_u8(vm, 255. * alpha)?,
            ];
        }
        "i1i2i3" => {
            let it = py::items(vm, v)?;
            if it.len() != 3 {
                return Err(vm.value_error("expected 3 values"));
            }
            let (i1, i2, i3) = (py::f64_of(vm, &it[0])?, py::f64_of(vm, &it[1])?, py::f64_of(vm, &it[2])?);
            let r = i1 + i2 - (2. / 3. * i3);
            let g = i1 + (4. / 3. * i3);
            let b = i1 - i2 - (2. / 3. * i3);
            c[0] = store_u8(vm, r * 255.)?;
            c[1] = store_u8(vm, g * 255.)?;
            c[2] = store_u8(vm, b * 255.)?;
        }
        _ => return Err(vm.attr_error(name)),
    }
    color_store(&a[1], c);
    Ok(Value::None)
}

fn color_new(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = a.first().and_then(|c| c.obj().cloned()).ok_or_else(|| vm.type_error("Color.__new__(X): X is not a type"))?;
    Ok(vm.types.new_native(&c, Box::new(ColorData { rgba: [0, 0, 0, 255], len: 4 })))
}

fn color_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = color_from_args(vm, &a[1..])?;
    py::native::<ColorData, _>(&a[0], |d| {
        d.rgba = c;
        d.len = 4;
    });
    Ok(Value::None)
}

fn packed(c: [u8; 4]) -> i64 {
    u32::from_be_bytes(c) as i64
}

fn color_method(vm: &mut Vm, a: &[Value], _: Kw, name: &str) -> PyResult<Value> {
    let this = &a[0];
    let (c, len) = this_color(vm, this)?;
    let rest = &a[1..];
    let one = |vm: &Vm| rest.first().cloned().ok_or_else(|| vm.type_error(format!("{name}() missing argument")));
    let same_type = |vm: &mut Vm, rgba: [u8; 4]| -> PyResult<Value> {
        let t = Value::Obj(this.obj().unwrap().class().clone());
        let args = rgba.iter().map(|v| Value::Int(*v as i64)).collect();
        vm.call(&t, args, Vec::new())
    };
    let rhs = |vm: &mut Vm| -> PyResult<[u8; 4]> {
        let o = one(vm)?;
        color_of(&o).ok_or_else(|| vm.type_error("Argument 'rhs' has incorrect type (expected Color)"))
    };
    match name {
        "__repr__" => Ok(vm.types.new_string(format!("({}, {}, {}, {})", c[0], c[1], c[2], c[3]))),
        "__int__" | "__index__" => Ok(Value::Int(packed(c))),
        "__float__" => Ok(Value::Float(packed(c) as f64)),
        "__hex__" => Ok(vm.types.new_string(format!("{:#x}", packed(c)))),
        "__oct__" => Ok(vm.types.new_string(format!("0o{:o}", packed(c)))),
        "__reduce__" => {
            let cls = color_class(vm)?;
            let d = vm.types.new_dict();
            let t = vm.types.new_tuple(c.iter().map(|v| Value::Int(*v as i64)).collect());
            py::dict_set(vm, &d, "rgba", t)?;
            let e = vm.types.new_tuple(Vec::new());
            Ok(vm.types.new_tuple(vec![Value::Obj(cls), e, d]))
        }
        "__setstate__" => {
            let d = one(vm)?;
            let k = vm.types.new_str("rgba");
            let t = ops::getitem(vm, &d, k)?;
            let it = py::items(vm, &t)?;
            if it.len() != 4 {
                return Err(vm.value_error("rgba"));
            }
            let mut n = [0u8; 4];
            for i in 0..4 {
                n[i] = u8_of(vm, &it[i])?;
            }
            color_store(this, n);
            Ok(Value::None)
        }
        "__setitem__" => {
            let key = py::int(vm, &one(vm)?)?;
            let val = rest.get(1).cloned().unwrap_or(Value::None);
            let Value::Int(v) = val else {
                let r = ops::repr(vm, &val)?;
                return Err(vm.value_error(r));
            };
            if key >= len as i64 {
                return Err(vm.index_error(key.to_string()));
            }
            if !(0..=255).contains(&v) {
                return Err(vm.value_error(v.to_string()));
            }
            if !(0..4).contains(&key) {
                return Err(vm.index_error(key.to_string()));
            }
            let mut n = c;
            n[key as usize] = v as u8;
            color_store(this, n);
            Ok(Value::None)
        }
        "__getitem__" => {
            let key = one(vm)?;
            if matches!(key.payload(), Some(Payload::Slice(..))) {
                let t = vm.types.new_tuple(c[..len as usize].iter().map(|v| Value::Int(*v as i64)).collect());
                return ops::getitem(vm, &t, key);
            }
            let k = py::int(vm, &key)?;
            if k >= len as i64 || !(0..4).contains(&k) {
                return Err(vm.index_error(k.to_string()));
            }
            Ok(Value::Int(c[k as usize] as i64))
        }
        "__len__" => Ok(Value::Int(len as i64)),
        "__iter__" => {
            let t = vm.types.new_tuple(c[..len as usize].iter().map(|v| Value::Int(*v as i64)).collect());
            ops::iter(vm, &t)
        }
        "__eq__" | "__ne__" => {
            let mut o = one(vm)?;
            if matches!(o.payload(), Some(Payload::Tuple(_))) {
                let t = color_like(vm, &o)?;
                o = new_color(vm, t)?;
            }
            let eq = color_of(&o).is_some_and(|x| x == c);
            Ok(Value::Bool(eq == (name == "__eq__")))
        }
        "__mul__" => {
            let r = rhs(vm)?;
            same_type(vm, std::array::from_fn(|i| (c[i] as u32 * r[i] as u32).min(255) as u8))
        }
        "__add__" => {
            let r = rhs(vm)?;
            same_type(vm, std::array::from_fn(|i| (c[i] as u32 + r[i] as u32).min(255) as u8))
        }
        "__sub__" => {
            let r = rhs(vm)?;
            same_type(vm, std::array::from_fn(|i| c[i].saturating_sub(r[i])))
        }
        "__mod__" => {
            let r = rhs(vm)?;
            same_type(vm, std::array::from_fn(|i| if r[i] == 0 { 0 } else { c[i] % r[i] }))
        }
        "__floordiv__" => {
            let r = rhs(vm)?;
            same_type(vm, std::array::from_fn(|i| if r[i] == 0 { 0 } else { (c[i] / r[i]).min(255) }))
        }
        "normalize" => Ok(vm.types.new_tuple(c.iter().map(|v| Value::Float(*v as f64 / 255.)).collect())),
        "correct_gamma" => {
            let g = py::f64_of(vm, &one(vm)?)?;
            let t: Vec<Value> = c[..len as usize]
                .iter()
                .map(|x| Value::Int(((*x as f64 / 255.).powf(g) * 255.).round_ties_even() as i64))
                .collect();
            let t = vm.types.new_tuple(t);
            let cls = Value::Obj(this.obj().unwrap().class().clone());
            vm.call(&cls, vec![t], Vec::new())
        }
        "set_length" => {
            let n = py::int(vm, &one(vm)?)?;
            if !(1..=4).contains(&n) {
                return Err(vm.value_error(n.to_string()));
            }
            py::native::<ColorData, _>(this, |d| d.len = n as u8);
            Ok(Value::None)
        }
        _ => Err(vm.attr_error(name.to_owned())),
    }
}

const COLOR_METHODS: &[(&str, crysirror_python::object::NativeFn)] = dispatch!(color_method;
    "__repr__", "__int__", "__index__", "__float__", "__hex__", "__oct__", "__reduce__", "__setstate__",
    "__setitem__", "__getitem__", "__len__", "__iter__", "__eq__", "__ne__", "__mul__", "__add__", "__sub__",
    "__mod__", "__floordiv__", "normalize", "correct_gamma", "set_length",
);

fn init_color(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "color", &[]);
    let mut attrs: Vec<(&str, Value)> = Vec::new();
    for n in ["r", "g", "b", "a", "cmy", "hsva", "hsla", "i1i2i3"] {
        let name = vm.types.new_str(n);
        let g = py::closure(vm, "color_get", color_prop_get, name.clone());
        let s = py::closure(vm, "color_set", color_prop_set, name);
        attrs.push((n, py::property(vm, g, Some(s))?));
    }
    let new = py::builtin(vm, "__new__", color_new);
    attrs.push(("__new__", py::staticmethod(vm, new)));
    attrs.push(("__hash__", Value::None));
    let mut methods = COLOR_METHODS.to_vec();
    methods.push(("__init__", color_init));
    let mn = modname(vm, "color");
    let c = py::class(vm, &mn, "Color", &[], &methods, &attrs)?;
    set(vm, &m, "Color", Value::Obj(c));
    Ok(m)
}

// power -----------------------------------------------------------------------------

fn power_info_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let u = local(vm, "POWERSTATE_UNKNOWN");
    py::set(vm, &a[0], "state", Value::Int(u))?;
    py::set(vm, &a[0], "seconds", Value::Int(-1))?;
    py::set(vm, &a[0], "percent", Value::Int(-1))?;
    Ok(Value::None)
}

fn power_info_repr(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let st = py::get(vm, &a[0], "state")?;
    let names = attr(vm, "power", "powerstate_to_name")?;
    let get = py::get(vm, &names, "get")?;
    let inv = vm.types.new_str("INVALID");
    let name = vm.call(&get, vec![st, inv], Vec::new())?;
    let name = py::string(vm, &name)?;
    let s = py::get(vm, &a[0], "seconds")?;
    let s = py::string(vm, &s)?;
    let p = py::get(vm, &a[0], "percent")?;
    let p = py::string(vm, &p)?;
    Ok(vm.types.new_string(format!("<PowerInfo state={name} seconds={s} percent={p}>")))
}

fn init_power(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "power",
        &[("get_power_info", |vm, _, _| {
            // No power service is exposed to the script runtime: SDL's answer
            // when the state cannot be determined.
            let c = attr(vm, "power", "PowerInfo")?;
            vm.call(&c, Vec::new(), Vec::new())
        })],
    );
    let names = vm.types.new_dict();
    for n in ["POWERSTATE_UNKNOWN", "POWERSTATE_ON_BATTERY", "POWERSTATE_NO_BATTERY", "POWERSTATE_CHARGING", "POWERSTATE_CHARGED"] {
        let v = local(vm, n);
        let s = vm.types.new_str(n);
        ops::setitem(vm, &names, Value::Int(v), s)?;
        if !sdl3(vm) {
            set(vm, &m, n, Value::Int(v));
        }
    }
    set(vm, &m, "powerstate_to_name", names);
    let mn = modname(vm, "power");
    let c = py::class(vm, &mn, "PowerInfo", &[], &[("__init__", power_info_init), ("__repr__", power_info_repr)], &[])?;
    set(vm, &m, "PowerInfo", Value::Obj(c));
    Ok(m)
}

// version ---------------------------------------------------------------------------

fn init_version(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "version", &[]);
    let vernum = vm.types.new_tuple(vec![Value::Int(2), Value::Int(1), Value::Int(0)]);
    set(vm, &m, "vernum", vernum);
    let v = vm.types.new_str("2.1.0");
    set(vm, &m, "ver", v);
    let r = vm.types.new_str("not-tracked");
    set(vm, &m, "rev", r);
    Ok(m)
}

// compat ----------------------------------------------------------------------------

fn init_compat(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "compat",
        &[
            ("geterror", |vm, _, _| {
                let i = py::import_attr(vm, "sys", "exc_info")?;
                let t = vm.call(&i, Vec::new(), Vec::new())?;
                ops::getitem(vm, &t, Value::Int(1))
            }),
            ("get_BytesIO", |vm, _, _| py::import_attr(vm, "io", "BytesIO")),
            ("get_StringIO", |vm, _, _| py::import_attr(vm, "io", "StringIO")),
            ("ord_", |vm, a, _| {
                let o = a.first().cloned().unwrap_or(Value::None);
                let f = py::import_attr(vm, "builtins", "ord")?;
                match vm.call(&f, vec![o.clone()], Vec::new()) {
                    Ok(v) => Ok(v),
                    Err(e) if vm.is_exc(&e, "TypeError") => Ok(o),
                    Err(e) => Err(e),
                }
            }),
            ("filesystem_encode", |vm, a, _| {
                let u = a.first().cloned().unwrap_or(Value::None);
                let enc = py::import_attr(vm, "sys", "getfilesystemencoding")?;
                let enc = vm.call(&enc, Vec::new(), Vec::new())?;
                let errors = py::import_attr(vm, "pygame_sdl2.compat", "filesystem_errors")?;
                py::method(vm, &u, "encode", vec![enc, errors])
            }),
            ("as_bytes", |vm, a, _| {
                let s = a.first().cloned().unwrap_or(Value::None);
                let l = vm.types.new_str("latin-1");
                let st = vm.types.new_str("strict");
                py::method(vm, &s, "encode", vec![l, st])
            }),
            ("as_unicode", |vm, a, _| {
                let s = a.first().cloned().unwrap_or(Value::None);
                let (asc, st) = (vm.types.new_str("ascii"), vm.types.new_str("strict"));
                let b = py::method(vm, &s, "encode", vec![asc, st])?;
                let (ue, st) = (vm.types.new_str("unicode_escape"), vm.types.new_str("stict"));
                py::method(vm, &b, "decode", vec![ue, st])
            }),
        ],
    );
    for (n, b) in [
        ("long_", "int"),
        ("xrange_", "range"),
        ("unichr_", "chr"),
        ("unicode_", "str"),
        ("bytes_", "bytes"),
        ("raw_input_", "input"),
        ("next_", "next"),
        ("imap_", "map"),
    ] {
        let v = py::import_attr(vm, "builtins", b)?;
        set(vm, &m, n, v);
    }
    let ft = py::import_attr(vm, "io", "FileIO")?;
    set(vm, &m, "file_type", ft);
    let plat = py::import_attr(vm, "sys", "platform")?;
    let fe = if plat.as_str() == Some("win32") { "replace" } else { "surrogateescape" };
    let fe = vm.types.new_str(fe);
    set(vm, &m, "filesystem_errors", fe);
    let all: Vec<Value> = ["geterror", "long_", "xrange_", "ord_", "unichr_", "unicode_", "raw_input_", "as_bytes", "as_unicode", "file_type"]
        .iter()
        .map(|s| vm.types.new_str(s))
        .collect();
    let all = vm.types.new_list(all);
    set(vm, &m, "__all__", all);
    Ok(m)
}

// sysfont ---------------------------------------------------------------------------

fn simplename(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn sf_get(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "pygame_sdl2.sysfont", name)
}

fn style_key(vm: &Vm, bold: Value, italic: Value) -> Value {
    vm.types.new_tuple(vec![bold, italic])
}

/// `_addfont(name, bold, italic, font, fontdict)`.
fn sf_addfont(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    if a.len() < 5 {
        return Err(vm.type_error("_addfont() takes 5 arguments"));
    }
    let (name, d) = (&a[0], &a[4]);
    if !ops::contains(vm, d, name)? {
        let nd = vm.types.new_dict();
        ops::setitem(vm, d, name.clone(), nd)?;
    }
    let inner = ops::getitem(vm, d, name.clone())?;
    let k = style_key(vm, a[1].clone(), a[2].clone());
    ops::setitem(vm, &inner, k, a[3].clone())?;
    Ok(Value::None)
}

fn sf_search_osx(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let fonts = a.first().cloned().unwrap_or(Value::None);
    let exists = py::import_attr(vm, "os.path", "exists")?;
    for (name, b, i, path) in tables::OSX_FONTS {
        let p = vm.types.new_str(path);
        let e = vm.call(&exists, vec![p.clone()], Vec::new())?;
        if py::truthy(vm, &e)? {
            let n = vm.types.new_str(name);
            sf_addfont(vm, &[n, Value::Bool(*b), Value::Bool(*i), p, fonts.clone()], &[])?;
        }
    }
    Ok(Value::None)
}

fn sf_init_darwin(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let fonts = vm.types.new_dict();
    sf_search_osx(vm, &[fonts.clone()], &[])?;
    Ok(fonts)
}

/// `initsysfonts_unix`: upstream runs `fc-list`; the script runtime cannot
/// spawn processes, which upstream handles by returning no fonts.
fn sf_init_unix(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_dict())
}

/// `initsysfonts_win32`: needs the Windows registry (`winreg`).
fn sf_init_win32(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import(vm, "winreg")?;
    Err(vm.exc_msg("ImportError", "the Windows font registry is not reachable from the CrySirror script runtime"))
}

fn sf_create_aliases(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let sysfonts = sf_get(vm, "Sysfonts")?;
    let sysalias = sf_get(vm, "Sysalias")?;
    let sets: &[&[&str]] = &[
        &[
            "monospace", "misc-fixed", "courier", "couriernew", "console", "fixed", "mono", "freemono",
            "bitstreamverasansmono", "verasansmono", "monotype", "lucidaconsole",
        ],
        &["sans", "arial", "helvetica", "swiss", "freesans", "bitstreamverasans", "verasans", "verdana", "tahoma"],
        &[
            "serif", "times", "freeserif", "bitstreamveraserif", "roman", "timesroman", "timesnewroman", "dutch",
            "veraserif", "georgia",
        ],
        &["wingdings", "wingbats"],
    ];
    for set in sets {
        let mut found = None;
        for name in *set {
            let n = vm.types.new_str(name);
            if ops::contains(vm, &sysfonts, &n)? {
                found = Some(ops::getitem(vm, &sysfonts, n)?);
                break;
            }
        }
        let Some(found) = found else { continue };
        if !py::truthy(vm, &found)? {
            continue;
        }
        for name in *set {
            let n = vm.types.new_str(name);
            if !ops::contains(vm, &sysfonts, &n)? {
                ops::setitem(vm, &sysalias, n, found.clone())?;
            }
        }
    }
    Ok(Value::None)
}

fn sf_initsysfonts(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let plat = py::import_attr(vm, "sys", "platform")?;
    let f = match plat.as_str() {
        Some("win32") => "initsysfonts_win32",
        Some("darwin") => "initsysfonts_darwin",
        _ => "initsysfonts_unix",
    };
    let f = sf_get(vm, f)?;
    let fonts = vm.call(&f, Vec::new(), Vec::new())?;
    let sysfonts = sf_get(vm, "Sysfonts")?;
    py::method(vm, &sysfonts, "update", vec![fonts])?;
    sf_create_aliases(vm, &[], &[])?;
    if !py::truthy(vm, &sysfonts)? {
        ops::setitem(vm, &sysfonts, Value::None, Value::None)?;
    }
    Ok(Value::None)
}

fn sf_ensure(vm: &mut Vm) -> PyResult<Value> {
    let sysfonts = sf_get(vm, "Sysfonts")?;
    if !py::truthy(vm, &sysfonts)? {
        sf_initsysfonts(vm, &[], &[])?;
    }
    Ok(sysfonts)
}

fn sf_styles(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let sysfonts = sf_ensure(vm)?;
    let sysalias = sf_get(vm, "Sysalias")?;
    let n = vm.types.new_string(simplename(name));
    let s = py::method(vm, &sysfonts, "get", vec![n.clone()])?;
    if py::truthy(vm, &s)? { Ok(s) } else { py::method(vm, &sysalias, "get", vec![n]) }
}

fn sf_font_constructor(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let font_mod = py::import(vm, &modname(vm, "font"))?;
    let ctor = py::get(vm, &font_mod, "Font")?;
    let font = vm.call(&ctor, vec![a[0].clone(), a[1].clone()], Vec::new())?;
    if py::truthy(vm, &a[2])? {
        py::method(vm, &font, "set_bold", vec![Value::Int(1)])?;
    }
    if py::truthy(vm, &a[3])? {
        py::method(vm, &font, "set_italic", vec![Value::Int(1)])?;
    }
    Ok(font)
}

fn sf_sysfont(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(a, kw, 0, "name", Value::None);
    let size = py::arg_or(a, kw, 1, "size", Value::None);
    let bold = py::arg_or(a, kw, 2, "bold", Value::Bool(false));
    let italic = py::arg_or(a, kw, 3, "italic", Value::Bool(false));
    let mut ctor = py::arg_or(a, kw, 4, "constructor", Value::None);
    if py::is_none(&ctor) {
        ctor = sf_get(vm, "font_constructor")?;
    }
    sf_ensure(vm)?;
    let (b, i) = (py::truthy(vm, &bold)?, py::truthy(vm, &italic)?);
    let (mut gotbold, mut gotitalic) = (false, false);
    let mut fontname = Value::None;
    if py::truthy(vm, &name)? {
        let all = py::string(vm, &name)?;
        for n in all.split(',') {
            let styles = sf_styles(vm, n)?;
            if py::truthy(vm, &styles)? {
                let plain_k = style_key(vm, Value::Bool(false), Value::Bool(false));
                let plain = py::method(vm, &styles, "get", vec![plain_k])?;
                let want = style_key(vm, bold.clone(), italic.clone());
                fontname = py::method(vm, &styles, "get", vec![want])?;
                let has_font = py::truthy(vm, &fontname)?;
                let has_plain = py::truthy(vm, &plain)?;
                if !has_font && !has_plain {
                    let items = py::method(vm, &styles, "items", Vec::new())?;
                    let first = py::items(vm, &items)?.into_iter().next().ok_or_else(|| vm.index_error("list index out of range"))?;
                    let (style, f) = pair(vm, &first)?;
                    fontname = f;
                    let (s0, s1) = pair(vm, &style)?;
                    if b && py::truthy(vm, &s0)? {
                        gotbold = true;
                    }
                    if i && py::truthy(vm, &s1)? {
                        gotitalic = true;
                    }
                } else if !has_font {
                    fontname = plain;
                } else if !ops::eq(vm, &plain, &fontname)? {
                    gotbold = b;
                    gotitalic = i;
                }
            }
            if py::truthy(vm, &fontname)? {
                break;
            }
        }
    }
    let set_bold = Value::Bool(b && !gotbold);
    let set_italic = Value::Bool(i && !gotitalic);
    vm.call(&ctor, vec![fontname, size, set_bold, set_italic], Vec::new())
}

fn sf_get_fonts(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let s = sf_ensure(vm)?;
    let keys = py::method(vm, &s, "keys", Vec::new())?;
    let k = py::items(vm, &keys)?;
    Ok(vm.types.new_list(k))
}

fn sf_match_font(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(a, kw, 0, "name", Value::None);
    let mut bold = py::arg_or(a, kw, 1, "bold", Value::Int(0));
    let mut italic = py::arg_or(a, kw, 2, "italic", Value::Int(0));
    sf_ensure(vm)?;
    let mut fontname = Value::None;
    let all = py::string(vm, &name)?;
    for n in all.split(',') {
        let styles = sf_styles(vm, n)?;
        if py::truthy(vm, &styles)? {
            while !py::truthy(vm, &fontname)? {
                let k = style_key(vm, bold.clone(), italic.clone());
                fontname = py::method(vm, &styles, "get", vec![k])?;
                if py::truthy(vm, &italic)? {
                    italic = Value::Int(0);
                } else if py::truthy(vm, &bold)? {
                    bold = Value::Int(0);
                } else if !py::truthy(vm, &fontname)? {
                    let vals = py::method(vm, &styles, "values", Vec::new())?;
                    fontname = py::items(vm, &vals)?.into_iter().next().unwrap_or(Value::None);
                    if !py::truthy(vm, &fontname)? {
                        break;
                    }
                }
            }
        }
        if py::truthy(vm, &fontname)? {
            break;
        }
    }
    Ok(fontname)
}

fn init_sysfont(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "sysfont",
        &[
            ("_simplename", |vm, a, _| {
                let s = py::string(vm, &a.first().cloned().unwrap_or(Value::None))?;
                Ok(vm.types.new_string(simplename(&s)))
            }),
            ("_addfont", sf_addfont),
            ("_search_osx_font_paths", sf_search_osx),
            ("initsysfonts_darwin", sf_init_darwin),
            ("initsysfonts_unix", sf_init_unix),
            ("initsysfonts_win32", sf_init_win32),
            ("create_aliases", sf_create_aliases),
            ("initsysfonts", sf_initsysfonts),
            ("font_constructor", sf_font_constructor),
            ("SysFont", sf_sysfont),
            ("get_fonts", sf_get_fonts),
            ("match_font", sf_match_font),
        ],
    );
    let xp: Vec<Value> = tables::XP_FONTS
        .iter()
        .map(|(f, n, b, i)| {
            let f = vm.types.new_str(f);
            let n = vm.types.new_str(n);
            vm.types.new_tuple(vec![f, n, Value::Bool(*b), Value::Bool(*i)])
        })
        .collect();
    let xp = vm.types.new_list(xp);
    set(vm, &m, "_XP_default_font_files", xp);
    let osx = vm.types.new_dict();
    for (name, b, i, path) in tables::OSX_FONTS {
        let n = vm.types.new_str(name);
        let p = vm.types.new_str(path);
        sf_addfont(vm, &[n, Value::Bool(*b), Value::Bool(*i), p, osx.clone()], &[])?;
    }
    set(vm, &m, "_OSX_default_font_files", osx);
    let d = vm.types.new_dict();
    set(vm, &m, "Sysfonts", d);
    let d = vm.types.new_dict();
    set(vm, &m, "Sysalias", d);
    Ok(m)
}

// Registration ----------------------------------------------------------------------

/// Imports one submodule under its canonical name (shared by both import
/// paths; the first import builds it and enters every alias).
fn gate(vm: &mut Vm, name: &str, sub: &str, renpy: bool) -> PyResult<()> {
    if (renpy && !vendored(vm)) || !available(vm, sub) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{name}'")));
    }
    Ok(())
}

macro_rules! modules {
    ($($sub:literal => $init:path;)*) => {
        fn register(vm: &mut Vm) {
            $(
                vm.register_native_module(
                    if $sub.is_empty() { "pygame_sdl2" } else { concat!("pygame_sdl2.", $sub) },
                    |vm| {
                        gate(vm, if $sub.is_empty() { "pygame_sdl2" } else { concat!("pygame_sdl2.", $sub) }, $sub, false)?;
                        $init(vm)
                    },
                );
                vm.register_native_module(
                    if $sub.is_empty() { "renpy.pygame" } else { concat!("renpy.pygame.", $sub) },
                    |vm| {
                        gate(vm, if $sub.is_empty() { "renpy.pygame" } else { concat!("renpy.pygame.", $sub) }, $sub, true)?;
                        $init(vm)
                    },
                );
            )*
        }
    };
}

modules! {
    "" => init_root;
    "error" => init_error;
    "locals" => init_locals;
    "rect" => init_rect;
    "color" => init_color;
    "power" => init_power;
    "version" => init_version;
    "compat" => init_compat;
    "sysfont" => init_sysfont;
    "event" => super::pygame_events::init_event;
    "key" => super::pygame_events::init_key;
    "mouse" => super::pygame_events::init_mouse;
    "time" => super::pygame_events::init_time;
    "pygame_time" => super::pygame_events::init_time;
    "scrap" => super::pygame_events::init_scrap;
    "display" => super::pygame_events::init_display;
    "joystick" => super::pygame_events::init_joystick;
    "controller" => super::pygame_events::init_controller;
    "surface" => super::pygame_surface::init_surface;
    "image" => super::pygame_surface::init_image;
    "transform" => super::pygame_surface::init_transform;
    "draw" => super::pygame_surface::init_draw;
    "gfxdraw" => super::pygame_surface::init_gfxdraw;
    "rwobject" => super::pygame_io::init_rwobject;
    "iostream" => super::pygame_io::init_iostream;
}

pub fn install(vm: &mut Vm) {
    vm.ext_set(Rc::new(RefCell::new(PgState::default())));
    register(vm);
}
