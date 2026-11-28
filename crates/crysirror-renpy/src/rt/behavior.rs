// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.behavior`, after Ren'Py 8.3 `behavior.py` (MIT).
//!
//! Keymap specifiers ("K_RETURN", "mousedown_1", "shift_K_TAB", keymap
//! names, ...) compile to native matchers instead of generated Python
//! lambdas; the matching rules (modifier checks, key aliases, repeat
//! handling and EVENTNAME events) follow upstream.
use super::{
    events,
    layout::{rargs, style as sty},
    py::{self, *},
    release, render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MODULE: &str = "renpy.display.behavior";

// Key matching ---------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Target {
    Mouse { up: bool, button: i64 },
    Key(i64),
    Unicode(String),
}

#[derive(Clone, Debug)]
struct Matcher {
    keydown: bool,
    target: Target,
    repeat: Option<bool>,
    checks: Vec<(i64, bool)>,
}

thread_local! {
    static CACHE: RefCell<HashMap<(String, bool), Rc<Vec<Matcher>>>> = RefCell::new(HashMap::new());
}

fn key_code(name: &str) -> Option<i64> {
    events::KEYS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
}

const MODIFIERS: &[&str] = &[
    "keydown",
    "keyup",
    "repeat",
    "alt",
    "meta",
    "shift",
    "noshift",
    "ctrl",
    "osctrl",
    "caps",
    "nocaps",
    "num",
    "nonum",
    "any",
    "anyrepeat",
    "anymod",
];

fn compile_one(
    vm: &mut Vm,
    key: &str,
    keydown: bool,
    out: &mut Vec<Matcher>,
    depth: u32,
) -> PyResult<()> {
    if depth > 16 {
        return Ok(());
    }
    for table in ["keymap", "default_keymap"] {
        let t = py::config(vm, table)?;
        if py::is_none(&t) {
            continue;
        }
        let k = vm.types.new_str(key);
        if ops::contains(vm, &t, &k)? {
            let v = ops::getitem(vm, &t, k)?;
            return compile_value(vm, &v, keydown, out, depth + 1);
        }
    }
    let mut part: Vec<String> = key.split('_').map(str::to_owned).collect();
    let mut mods: Vec<String> = Vec::new();
    while part.len() > 1 && MODIFIERS.contains(&part[0].as_str()) {
        mods.push(part.remove(0));
    }
    let mut key = part.join("_");
    let aliases = py::config(vm, "key_aliases").unwrap_or(Value::None);
    if !py::is_none(&aliases) {
        let k = vm.types.new_str(&key);
        if ops::contains(vm, &aliases, &k)? {
            let a = ops::getitem(vm, &aliases, k)?;
            let a = py::string(vm, &a)?;
            part = a.split('_').map(str::to_owned).collect();
            while part.len() > 1 && MODIFIERS.contains(&part[0].as_str()) {
                mods.push(part.remove(0));
            }
            key = part.join("_");
        }
    }
    let has = |m: &str| mods.iter().any(|x| x == m);
    let mut keydown = keydown;
    if has("keydown") {
        keydown = true;
    } else if has("keyup") {
        keydown = false;
    }
    if part[0] == "joy" || part[0] == "pad" {
        return Ok(());
    }
    let mac = cfg!(target_os = "macos");
    let developer = py::config_bool(vm, "developer").unwrap_or(false);
    let target = if part[0] == "mouseup" {
        if !keydown {
            return Ok(());
        }
        let b = part
            .get(1)
            .and_then(|b| b.parse().ok())
            .ok_or_else(|| py::error(vm, format!("Invalid key specifier {key}")))?;
        Target::Mouse {
            up: true,
            button: b,
        }
    } else if part[0] == "mousedown" {
        let b = part
            .get(1)
            .and_then(|b| b.parse().ok())
            .ok_or_else(|| py::error(vm, format!("Invalid key specifier {key}")))?;
        Target::Mouse {
            up: !keydown,
            button: b,
        }
    } else if part.len() == 1 {
        if part[0].chars().count() != 1 {
            if developer {
                return Err(py::error(vm, format!("Invalid key specifier {key}")));
            }
            return Ok(());
        }
        Target::Unicode(part[0].clone())
    } else {
        if part[0] != "K" {
            if developer {
                return Err(py::error(vm, format!("Invalid key specifier {key}")));
            }
            return Ok(());
        }
        match key_code(&key) {
            Some(c) => Target::Key(c),
            None => return Err(py::error(vm, format!("Invalid key specifier {key}"))),
        }
    };
    let mouse = matches!(target, Target::Mouse { .. });
    let repeat = if mouse {
        None
    } else if has("repeat") {
        Some(true)
    } else if has("any") || has("anyrepeat") {
        None
    } else {
        Some(false)
    };
    let mut checks = Vec::new();
    if !has("anymod") {
        if !matches!(key.as_str(), "K_LALT" | "K_RALT") {
            checks.push((0x300, has("alt") || (mac && has("osctrl"))));
        }
        if !matches!(key.as_str(), "K_LGUI" | "K_RGUI" | "K_LMETA" | "K_RMETA") {
            checks.push((0xC00, has("meta")));
        }
        if !matches!(key.as_str(), "K_LCTRL" | "K_RCTRL") {
            checks.push((0xC0, has("ctrl") || (!mac && has("osctrl"))));
        }
    }
    if !matches!(key.as_str(), "K_LSHIFT" | "K_RSHIFT") {
        if has("shift") {
            checks.push((3, true));
        }
        if has("noshift") {
            checks.push((3, false));
        }
    }
    if key != "K_CAPSLOCK" {
        if has("caps") {
            checks.push((0x2000, true));
        }
        if has("nocaps") {
            checks.push((0x2000, false));
        }
    }
    if key != "K_NUMLOCK" {
        if has("num") {
            checks.push((0x1000, true));
        }
        if has("nonum") {
            checks.push((0x1000, false));
        }
    }
    out.push(Matcher {
        keydown,
        target,
        repeat,
        checks,
    });
    Ok(())
}

fn compile_value(
    vm: &mut Vm,
    key: &Value,
    keydown: bool,
    out: &mut Vec<Matcher>,
    depth: u32,
) -> PyResult<()> {
    if py::is_none(key) {
        return Ok(());
    }
    if let Some(items) = py::seq(key) {
        for i in items {
            compile_value(vm, &i, keydown, out, depth)?;
        }
        return Ok(());
    }
    let k = py::string(vm, key)?;
    compile_one(vm, &k, keydown, out, depth)
}

fn matchers(vm: &mut Vm, keysym: &Value, keydown: bool) -> PyResult<Rc<Vec<Matcher>>> {
    let key = match keysym.as_str() {
        Some(s) => s.to_owned(),
        None => ops::repr(vm, keysym)?.to_string(),
    };
    if let Some(m) = CACHE.with(|c| c.borrow().get(&(key.clone(), keydown)).cloned()) {
        return Ok(m);
    }
    let mut out = Vec::new();
    compile_value(vm, keysym, keydown, &mut out, 0)?;
    let rc = Rc::new(out);
    CACHE.with(|c| c.borrow_mut().insert((key, keydown), rc.clone()));
    Ok(rc)
}

fn ev_int(vm: &mut Vm, ev: &Value, n: &str) -> PyResult<i64> {
    let v = py::get_or_none(vm, ev, n)?;
    if py::is_none(&v) {
        Ok(0)
    } else {
        py::int(vm, &v)
    }
}

fn test(vm: &mut Vm, ev: &Value, ms: &[Matcher]) -> PyResult<bool> {
    let ty = ev_int(vm, ev, "type")?;
    for m in ms {
        let ok_type = match &m.target {
            Target::Mouse { up, .. } => {
                ty == if *up {
                    events::MOUSEBUTTONUP
                } else {
                    events::MOUSEBUTTONDOWN
                }
            }
            _ => {
                ty == if m.keydown {
                    events::KEYDOWN
                } else {
                    events::KEYUP
                }
            }
        };
        if !ok_type {
            continue;
        }
        if let Some(r) = m.repeat {
            let rep = py::get_or_none(vm, ev, "repeat")?;
            if ops::truthy(vm, &rep)? != r {
                continue;
            }
        }
        let mods = ev_int(vm, ev, "mod")?;
        if !m
            .checks
            .iter()
            .all(|(mask, want)| ((mods & mask) != 0) == *want)
        {
            continue;
        }
        let hit = match &m.target {
            Target::Mouse { button, .. } => ev_int(vm, ev, "button")? == *button,
            Target::Key(k) => ev_int(vm, ev, "key")? == *k,
            Target::Unicode(u) => py::get_or_none(vm, ev, "unicode")?.as_str() == Some(u.as_str()),
        };
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

fn eventname_match(vm: &mut Vm, ev: &Value, keysym: &Value, up: bool) -> PyResult<bool> {
    let names = py::get_or_none(vm, ev, "eventnames")?;
    let evup = {
        let v = py::get_or_none(vm, ev, "up")?;
        ops::truthy(vm, &v)?
    };
    if evup != up {
        return Ok(false);
    }
    let keys = match py::seq(keysym) {
        Some(v)
            if !matches!(keysym.payload(), Some(crysirror_python::Payload::Tuple(_))) || true =>
        {
            v
        }
        _ => vec![keysym.clone()],
    };
    let keys = if keysym.as_str().is_some() {
        vec![keysym.clone()]
    } else {
        keys
    };
    for k in keys {
        if ops::contains(vm, &names, &k)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn map_event(vm: &mut Vm, ev: &Value, keysym: &Value) -> PyResult<bool> {
    if events::is_type(vm, ev, events::EVENTNAME)? {
        return eventname_match(vm, ev, keysym, false);
    }
    let m = matchers(vm, keysym, true)?;
    test(vm, ev, &m)
}

pub fn map_keyup(vm: &mut Vm, ev: &Value, keysym: &Value) -> PyResult<bool> {
    if events::is_type(vm, ev, events::EVENTNAME)? && eventname_match(vm, ev, keysym, true)? {
        return Ok(true);
    }
    let m = matchers(vm, keysym, false)?;
    test(vm, ev, &m)
}

/// Engine keymap bindings that later releases added to `config.keymap`
/// (`common/00keymap.rpy`), with the first release that has them. The
/// upstream engine only maps those names in releases that define them, so
/// for an earlier release the engine-side check is simply false.
const KEYMAP_SINCE: &[(&str, &[u32])] = &[
    // Added to config.keymap (6.15+: common/00keymap.rpy) after 6.7.
    ("help", &[6, 8]),
    ("iconify", &[6, 8]),
    ("quit", &[6, 8]),
    ("button_ignore", &[6, 10]),
    ("save_delete", &[6, 10]),
    ("drag_activate", &[6, 12]),
    ("drag_deactivate", &[6, 12]),
    ("choose_renderer", &[6, 13]),
    ("input_delete", &[6, 14]),
    ("input_left", &[6, 14]),
    ("input_right", &[6, 14]),
    ("bar_down", &[6, 15]),
    ("bar_left", &[6, 15]),
    ("bar_right", &[6, 15]),
    ("bar_up", &[6, 15]),
    ("console", &[6, 15]),
    ("console_newer", &[6, 15]),
    ("console_older", &[6, 15]),
    ("button_alternate", &[6, 17]),
    ("button_alternate_ignore", &[6, 17]),
    ("profile_once", &[6, 18]),
    ("self_voicing", &[6, 18]),
    ("input_home", &[6, 99, 7]),
    ("input_end", &[6, 99, 7]),
    ("input_up", &[6, 99, 11]),
    ("input_down", &[6, 99, 11]),
    ("viewport_wheelup", &[6, 99, 11]),
    ("viewport_wheeldown", &[6, 99, 11]),
    ("viewport_leftarrow", &[6, 99, 11]),
    ("viewport_rightarrow", &[6, 99, 11]),
    ("viewport_uparrow", &[6, 99, 11]),
    ("viewport_downarrow", &[6, 99, 11]),
    ("stop_skipping", &[6, 99, 12, 1]),
    ("dismiss_unfocused", &[6, 99, 12]),
    ("toggle_afm", &[6, 99, 12]),
    ("viewport_pageup", &[7, 0]),
    ("viewport_pagedown", &[7, 0]),
    ("input_copy", &[7, 1, 4]),
    ("input_paste", &[7, 1, 4]),
    ("input_jump_word_left", &[7, 4, 7]),
    ("input_jump_word_right", &[7, 4, 7]),
    ("input_delete_word", &[7, 4, 7]),
    ("input_delete_full", &[7, 4, 7]),
];

/// Whether the running release has the engine keymap binding `name`.
pub fn release_has_binding(vm: &Vm, name: &str) -> bool {
    if name == "input_next_line" {
        return release::state_since_pair(vm, 6, 1);
    }
    KEYMAP_SINCE
        .iter()
        .find(|(n, _)| *n == name)
        .is_none_or(|(_, v)| release::at_least(vm, v))
}

pub fn map_name(vm: &mut Vm, ev: &Value, name: &str) -> PyResult<bool> {
    if !release_has_binding(vm, name) {
        return Ok(false);
    }
    let k = vm.types.new_str(name);
    map_event(vm, ev, &k)
}

fn f_map_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(map_event(
        vm,
        &args[0],
        &args.get(1).cloned().unwrap_or(Value::None),
    )?))
}

fn f_map_keyup(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(map_keyup(
        vm,
        &args[0],
        &args.get(1).cloned().unwrap_or(Value::None),
    )?))
}

fn f_clear_keymap_cache(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    CACHE.with(|c| c.borrow_mut().clear());
    Ok(Value::None)
}

fn f_init_keymap(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    CACHE.with(|c| c.borrow_mut().clear());
    let pb = py::config(vm, "pad_bindings").unwrap_or(Value::None);
    let km = py::config(vm, "keymap")?;
    if !py::is_none(&pb) {
        for (_, bl) in ops::mapping_items(vm, &pb)? {
            for b in py::items(vm, &bl)? {
                if !ops::contains(vm, &km, &b)? {
                    ops::setitem(vm, &km, b, vm.types.new_list(Vec::new()))?;
                }
            }
        }
    }
    Ok(Value::None)
}

fn f_compile_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let key = args.first().cloned().unwrap_or(Value::None);
    let kd = ops::truthy(vm, &args.get(1).cloned().unwrap_or(Value::Bool(true)))?;
    let mut out = Vec::new();
    compile_value(vm, &key, kd, &mut out, 0)?;
    Ok(vm.types.new_string(if out.is_empty() {
        "(False)".into()
    } else {
        format!("{out:?}")
    }))
}

fn f_queue_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", Value::None);
    let up = py::kw(kw, "up").cloned().unwrap_or(Value::Bool(false));
    let names = if name.as_str().is_some() {
        vm.types.new_list(vec![name])
    } else {
        name
    };
    let mut fields: Vec<(&str, Value)> = vec![("eventnames", names), ("up", up)];
    let extra: Vec<(String, Value)> = kw
        .iter()
        .filter(|(k, _)| &**k != "up")
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let ev = events::new_event(vm, events::EVENTNAME, std::mem::take(&mut fields))?;
    for (k, v) in extra {
        py::set(vm, &ev, &k, v)?;
    }
    events::input(vm).borrow_mut().queue.push(ev);
    Ok(Value::None)
}

// Actions ------------------------------------------------------------------------------------

fn is_action(vm: &mut Vm, v: &Value) -> bool {
    py::import_attr(vm, "renpy.ui", "Action")
        .ok()
        .and_then(|c| c.obj().cloned())
        .is_some_and(|c| ops::isinstance(vm, v, &c))
}

pub fn run(
    vm: &mut Vm,
    action: &Value,
    args: Vec<Value>,
    kw: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    if py::is_none(action) {
        return Ok(Value::None);
    }
    if matches!(
        action.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        let mut rv = Value::None;
        for i in py::items(vm, action)? {
            let r = run(vm, &i, args.clone(), kw.clone())?;
            if !py::is_none(&r) {
                rv = r;
            }
        }
        return Ok(rv);
    }
    vm.call(action, args, kw)
}

fn f_run(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = args.first().cloned().unwrap_or(Value::None);
    run(
        vm,
        &a,
        args.get(1..).map(|v| v.to_vec()).unwrap_or_default(),
        kw.to_vec(),
    )
}

fn run_unhovered(vm: &mut Vm, v: &Value) -> PyResult<()> {
    if py::is_none(v) {
        return Ok(());
    }
    if matches!(
        v.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        for i in py::items(vm, v)? {
            run_unhovered(vm, &i)?;
        }
        return Ok(());
    }
    if let Some(f) = py::get_opt(vm, v, "unhovered")? {
        if !py::is_none(&f) {
            vm.call(&f, Vec::new(), Vec::new())?;
        }
    }
    Ok(())
}

fn f_run_unhovered(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    run_unhovered(vm, &args.first().cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

pub fn run_periodic(vm: &mut Vm, v: &Value, st: f64) -> PyResult<Option<f64>> {
    if matches!(
        v.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        let mut rv: Option<f64> = None;
        for i in py::items(vm, v)? {
            if let Some(x) = run_periodic(vm, &i, st)? {
                rv = Some(rv.map_or(x, |r| r.min(x)));
            }
        }
        return Ok(rv);
    }
    if is_action(vm, v) {
        let r = vm.call_method(v, "periodic", vec![Value::Float(st)])?;
        if !py::is_none(&r) {
            return Ok(Some(py::f64_of(vm, &r)?));
        }
    }
    Ok(None)
}

fn f_run_periodic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    Ok(
        run_periodic(vm, &args.first().cloned().unwrap_or(Value::None), st)?
            .map_or(Value::None, Value::Float),
    )
}

pub fn get_tooltip(vm: &mut Vm, action: &Value) -> PyResult<Value> {
    if matches!(
        action.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        for i in py::items(vm, action)? {
            let r = get_tooltip(vm, &i)?;
            if !py::is_none(&r) {
                return Ok(r);
            }
        }
        return Ok(Value::None);
    }
    match py::get_opt(vm, action, "get_tooltip")? {
        Some(f) if !py::is_none(&f) => vm.call(&f, Vec::new(), Vec::new()),
        _ => Ok(Value::None),
    }
}

fn f_get_tooltip(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_tooltip(vm, &args.first().cloned().unwrap_or(Value::None))
}

fn store_class(vm: &mut Vm, n: &str) -> Option<ObjRef> {
    py::import_attr(vm, "store", n)
        .ok()
        .and_then(|c| c.obj().cloned())
}

pub fn is_selected(vm: &mut Vm, action: &Value) -> PyResult<bool> {
    if matches!(
        action.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        let items = py::items(vm, action)?;
        if let Some(c) = store_class(vm, "SelectedIf") {
            for i in &items {
                if ops::isinstance(vm, i, &c) {
                    let r = vm.call_method(i, "get_selected", Vec::new())?;
                    return ops::truthy(vm, &r);
                }
            }
        }
        for i in &items {
            if is_selected(vm, i)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if is_action(vm, action) {
        let r = vm.call_method(action, "get_selected", Vec::new())?;
        return ops::truthy(vm, &r);
    }
    Ok(false)
}

pub fn is_sensitive(vm: &mut Vm, action: &Value) -> PyResult<bool> {
    if matches!(
        action.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        let items = py::items(vm, action)?;
        if let Some(c) = store_class(vm, "SensitiveIf") {
            for i in &items {
                if ops::isinstance(vm, i, &c) {
                    let r = vm.call_method(i, "get_sensitive", Vec::new())?;
                    return ops::truthy(vm, &r);
                }
            }
        }
        for i in &items {
            if !is_sensitive(vm, i)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if is_action(vm, action) {
        let r = vm.call_method(action, "get_sensitive", Vec::new())?;
        return ops::truthy(vm, &r);
    }
    Ok(true)
}

fn f_is_selected(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(is_selected(
        vm,
        &args.first().cloned().unwrap_or(Value::None),
    )?))
}

fn f_is_sensitive(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(is_sensitive(
        vm,
        &args.first().cloned().unwrap_or(Value::None),
    )?))
}

fn alt(vm: &mut Vm, clicked: &Value) -> PyResult<Value> {
    if matches!(
        clicked.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        let mut parts = Vec::new();
        for i in py::items(vm, clicked)? {
            let t = alt(vm, &i)?;
            if !py::is_none(&t) {
                parts.push(py::string(vm, &t)?);
            }
        }
        return Ok(if parts.is_empty() {
            Value::None
        } else {
            vm.types.new_string(parts.join(" "))
        });
    }
    if is_action(vm, clicked) {
        return py::get_or_none(vm, clicked, "alt");
    }
    Ok(Value::None)
}

fn f_alt(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    alt(vm, &args.first().cloned().unwrap_or(Value::None))
}

fn f_predict_action(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = args.first().cloned().unwrap_or(Value::None);
    if matches!(
        v.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        for i in py::items(vm, &v)? {
            f_predict_action(vm, &[i], &[])?;
        }
    } else if is_action(vm, &v) {
        vm.call_method(&v, "predict", Vec::new())?;
    }
    Ok(Value::None)
}

/// `skipping(ev)`. Release differences: `store._skipping` gates it from
/// 6.99.5, the interaction restarts from 6.99.5, `stop_skipping` exists
/// from 6.99.12.1, and an open text box suppresses ctrl-skipping from
/// 7.5/8.0 (unless `config.skip_during_text`).
fn f_skipping(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ev = args.first().cloned().unwrap_or(Value::None);
    if !py::config_bool(vm, "allow_skipping").unwrap_or(true) {
        return Ok(Value::None);
    }
    let has_store_flag = release::at_least(vm, &[6, 99, 5]);
    if has_store_flag {
        let sk = py::import_attr(vm, "store", "_skipping").unwrap_or(Value::Bool(true));
        if !ops::truthy(vm, &sk)? {
            return Ok(Value::None);
        }
    }
    let config = py::import(vm, "renpy.config")?;
    let restart = |vm: &mut Vm| -> PyResult<()> {
        if has_store_flag {
            let restart = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
            vm.call(&restart, Vec::new(), Vec::new())?;
        }
        Ok(())
    };
    if map_name(vm, &ev, "skip")? {
        let allowed = if release::state_since_pair(vm, 5, 0) {
            let iface = py::import_attr(vm, "renpy.game", "interface")?;
            let tr = py::get_or_none(vm, &iface, "text_rect")?;
            py::is_none(&tr) || py::config_bool(vm, "skip_during_text").unwrap_or(false)
        } else {
            true
        };
        if allowed {
            py::set(vm, &config, "skipping", vm.types.new_str("slow"))?;
            restart(vm)?;
        }
    }
    let skip = vm.types.new_str("skip");
    let stop = map_keyup(vm, &ev, &skip)?
        || (release::at_least(vm, &[6, 99, 12, 1]) && map_name(vm, &ev, "stop_skipping")?);
    if stop {
        py::set(vm, &config, "skipping", Value::None)?;
        restart(vm)?;
    }
    Ok(Value::None)
}

pub fn ignore_event(vm: &mut Vm) -> crysirror_python::PyErr {
    match py::import_attr(vm, "renpy.display.core", "IgnoreEvent")
        .and_then(|c| vm.call(&c, Vec::new(), Vec::new()))
    {
        Ok(e) => crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()),
        Err(e) => e,
    }
}

fn play(vm: &mut Vm, sound: &Value) -> PyResult<()> {
    if py::is_none(sound) {
        return Ok(());
    }
    let p = py::import_attr(vm, "renpy.exports", "play")?;
    vm.call(&p, vec![sound.clone()], Vec::new())?;
    Ok(())
}

fn interface(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "interface")
}

fn timeout(vm: &mut Vm, t: f64) -> PyResult<()> {
    let i = interface(vm)?;
    if !py::is_none(&i) {
        vm.call_method(&i, "timeout", vec![Value::Float(t.max(0.))])?;
    }
    Ok(())
}

fn null_init(vm: &mut Vm, this: &Value, props: Kw) -> PyResult<()> {
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let f = py::get(vm, &null, "__init__")?;
    vm.call(&f, vec![this.clone()], props.to_vec())?;
    Ok(())
}

fn evargs(vm: &mut Vm, args: &[Value]) -> PyResult<(Value, f64, f64, f64)> {
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let st = py::f64_of(vm, &args.get(4).cloned().unwrap_or(Value::Int(0)))?;
    Ok((ev, x, y, st))
}

// Keymap / RollForward / pauses --------------------------------------------------------------------

fn keymap_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut km = Vec::new();
    let mut sound = Value::None;
    let mut capture = Value::Bool(true);
    for (k, v) in kw {
        match &**k {
            "replaces" => {}
            "activate_sound" => sound = v.clone(),
            "capture" => capture = v.clone(),
            _ => km.push((k.clone(), v.clone())),
        }
    }
    let mut props = vec![("style".into(), vm.types.new_str("default"))];
    if !py::is_none(&sound) {
        props.push(("activate_sound".into(), sound));
    }
    null_init(vm, this, &props)?;
    let d = py::kwargs_dict(vm, &km)?;
    py::set(vm, this, "keymap", d)?;
    py::set(vm, this, "capture", capture)?;
    Ok(Value::None)
}

fn keymap_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, ..) = evargs(vm, args)?;
    let sup = py::get_opt(vm, &ev, "_suppressed")?.unwrap_or(Value::Bool(false));
    if ops::truthy(vm, &sup)? {
        return Ok(Value::None);
    }
    let km = py::get(vm, this, "keymap")?;
    for (name, action) in ops::mapping_items(vm, &km)? {
        if map_event(vm, &ev, &name)? {
            let snd = sty(vm, this, "activate_sound")?;
            play(vm, &snd)?;
            let rv = run(vm, &action, Vec::new(), Vec::new())?;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
            let cap = py::get_or_none(vm, this, "capture")?;
            if ops::truthy(vm, &cap)? {
                return Err(ignore_event(vm));
            }
        }
    }
    Ok(Value::None)
}

fn keymap_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let km = py::get(vm, &args[0], "keymap")?;
    for (_, a) in ops::mapping_items(vm, &km)? {
        f_predict_action(vm, &[a], &[])?;
    }
    Ok(Value::None)
}

fn rollforward_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let value = arg_or(args, kw, 1, "value", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "value")
        .cloned()
        .collect();
    null_init(vm, this, &props)?;
    py::set(vm, this, "value", value)?;
    Ok(Value::None)
}

fn rollforward_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (ev, ..) = evargs(vm, args)?;
    if map_name(vm, &ev, "rollforward")? {
        let v = py::get_or_none(vm, &args[0], "value")?;
        let f = py::import_attr(vm, "renpy.exports", "roll_forward_core")?;
        return vm.call(&f, vec![v], Vec::new());
    }
    Ok(Value::None)
}

fn pause_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["delay", "result", "voice", "self_voicing", "modal"];
    let mut vals = vec![
        Value::Int(0),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::None,
    ];
    for (i, v) in args[1..].iter().enumerate().take(5) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    null_init(vm, this, &props)?;
    for (i, n) in names.iter().enumerate().take(4) {
        py::set(vm, this, n, vals[i].clone())?;
    }
    let modal = if py::is_none(&vals[4]) {
        py::config(vm, "modal_blocks_pause").unwrap_or(Value::Bool(true))
    } else {
        vals[4].clone()
    };
    py::set(vm, this, "modal", modal)?;
    Ok(Value::None)
}

fn afm_callback_ok(vm: &mut Vm) -> PyResult<bool> {
    let cb = py::config(vm, "afm_callback").unwrap_or(Value::None);
    if py::is_none(&cb) {
        return Ok(true);
    }
    let r = vm.call(&cb, Vec::new(), Vec::new())?;
    ops::truthy(vm, &r)
}

fn tts_active(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.display.tts", "is_active")
        .and_then(|f| vm.call(&f, Vec::new(), Vec::new()))
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false)
}

fn pause_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, _, _, st) = evargs(vm, args)?;
    let delay = {
        let v = py::get(vm, this, "delay")?;
        py::f64_of(vm, &v)?
    };
    if events::is_type(vm, &ev, events::TIMEEVENT)? {
        let em = py::get_or_none(vm, &ev, "modal")?;
        let modal = py::get_or_none(vm, this, "modal")?;
        if ops::truthy(vm, &em)? && ops::truthy(vm, &modal)? {
            timeout(vm, delay - st)?;
            return Ok(Value::None);
        }
        if st >= delay {
            let nw_voice = py::config_bool(vm, "nw_voice").unwrap_or(true);
            let sv = {
                let v = py::get_or_none(vm, this, "self_voicing")?;
                ops::truthy(vm, &v)?
            };
            if sv && nw_voice && (tts_active(vm) || !afm_callback_ok(vm)?) {
                timeout(vm, 0.05)?;
                return Ok(Value::None);
            }
            let voice = {
                let v = py::get_or_none(vm, this, "voice")?;
                ops::truthy(vm, &v)?
            };
            if voice && nw_voice && !afm_callback_ok(vm)? {
                timeout(vm, 0.05)?;
                return Ok(Value::None);
            }
            let iface = interface(vm)?;
            let drawn = vm.call_method(&iface, "drawn_since", vec![Value::Float(st - delay)])?;
            if ops::truthy(vm, &drawn)? {
                return py::get_or_none(vm, this, "result");
            }
            py::set(vm, &iface, "force_redraw", Value::Bool(true))?;
        }
    }
    timeout(vm, delay - st)?;
    Ok(Value::None)
}

fn predictpause_event(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let iface = py::import_attr(vm, "renpy.display", "interface")?;
    let fp = py::get_or_none(vm, &iface, "force_prediction")?;
    if !ops::truthy(vm, &fp)? {
        return Ok(Value::Bool(false));
    }
    timeout(vm, 0.)?;
    Ok(Value::None)
}

fn soundstop_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let channel = arg_or(args, kw, 1, "channel", Value::None);
    let result = arg_or(args, kw, 2, "result", Value::Bool(false));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "channel" | "result"))
        .cloned()
        .collect();
    null_init(vm, this, &props)?;
    py::set(vm, this, "channel", channel)?;
    py::set(vm, this, "result", result)?;
    Ok(Value::None)
}

fn soundstop_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ch = py::get(vm, this, "channel")?;
    let gp = py::import_attr(vm, "renpy.audio.music", "get_playing")?;
    let p = vm.call(&gp, vec![ch], Vec::new())?;
    if py::is_none(&p) {
        return py::get_or_none(vm, this, "result");
    }
    timeout(vm, 0.025)?;
    Ok(Value::None)
}

// SayBehavior -------------------------------------------------------------------------------------

fn say_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    // `dismiss_unfocused` arrives in 6.99.12.1, `dialogue_pause` in 7.6/8.1.
    let nargs = if release::state_since_pair(vm, 6, 1) {
        6
    } else if release::at_least(vm, &[6, 99, 12, 1]) {
        5
    } else {
        4
    };
    let names = &[
        "default",
        "afm",
        "dismiss",
        "allow_dismiss",
        "dismiss_unfocused",
        "dialogue_pause",
    ][..nargs];
    let dismiss_d = vm.types.new_list(vec![vm.types.new_str("dismiss")]);
    let unf_d = if nargs >= 5 {
        vm.types
            .new_list(vec![vm.types.new_str("dismiss_unfocused")])
    } else {
        vm.types.new_list(Vec::new())
    };
    let mut vals = vec![
        Value::Bool(true),
        Value::None,
        dismiss_d,
        Value::None,
        unf_d,
        Value::None,
    ];
    for (i, v) in args[1..].iter().enumerate().take(nargs) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    props.push(("default".into(), vals[0].clone()));
    null_init(vm, this, &props)?;
    let listify = |vm: &mut Vm, v: &Value| -> PyResult<Value> {
        if matches!(
            v.payload(),
            Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
        ) {
            Ok(v.clone())
        } else {
            Ok(vm.types.new_list(vec![v.clone()]))
        }
    };
    let dismiss = listify(vm, &vals[2])?;
    let unf = listify(vm, &vals[4])?;
    let afm_len = if py::is_none(&vals[1]) {
        Value::None
    } else {
        Value::Int(ops::len(vm, &vals[1])? as i64)
    };
    py::set(vm, this, "afm_length", afm_len)?;
    py::set(vm, this, "dismiss", dismiss)?;
    py::set(vm, this, "dismiss_unfocused", unf)?;
    py::set(vm, this, "allow_dismiss", vals[3].clone())?;
    py::set(vm, this, "dialogue_pause", vals[5].clone())?;
    Ok(Value::None)
}

fn say_tts_all(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.tts", "TTSRoot")?;
    let e = vm.call(&c, Vec::new(), Vec::new())?;
    Err(crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()))
}

fn say_set_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let texts = args[1..].to_vec();
    py::set(vm, this, "text_tuple", vm.types.new_tuple(texts.clone()))?;
    let mut len = 1i64;
    let filt = py::import_attr(vm, "renpy.text.extras", "filter_text_tags")?;
    for t in texts {
        let r: PyResult<i64> = (|| {
            let tx = py::get(vm, &t, "text")?;
            let first = ops::getitem(vm, &tx, Value::Int(0))?;
            let s = py::get_or_none(vm, &t, "start")?;
            let e = py::get_or_none(vm, &t, "end")?;
            let sl = vm.call_method(
                &first,
                "__getitem__",
                vec![Value::Obj(crysirror_python::object::new_obj(
                    vm.types.slice.clone(),
                    None,
                    crysirror_python::Payload::Slice(s, e, Value::None),
                ))],
            )?;
            let f = vm.call(
                &filt,
                vec![sl],
                vec![("allow".into(), vm.types.new_list(Vec::new()))],
            )?;
            Ok((ops::len(vm, &f)? as i64).max(1))
        })();
        len += match r {
            Ok(n) => n,
            Err(_) => {
                let s = py::get_or_none(vm, &t, "start")?;
                let e = py::get_or_none(vm, &t, "end")?;
                let (s, e) = (py::int(vm, &s).unwrap_or(0), py::int(vm, &e).unwrap_or(0));
                (e - s).max(1)
            }
        };
    }
    py::set(vm, this, "afm_length", Value::Int(len))?;
    Ok(Value::None)
}

fn prefs(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "preferences")
}

fn pref_f(vm: &mut Vm, n: &str) -> PyResult<f64> {
    let p = prefs(vm)?;
    let v = py::get_or_none(vm, &p, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn pref_b(vm: &mut Vm, n: &str) -> PyResult<bool> {
    let p = prefs(vm)?;
    let v = py::get_or_none(vm, &p, n)?;
    ops::truthy(vm, &v)
}

fn say_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, x, _, st) = evargs(vm, args)?;
    let afm_len = py::get_or_none(vm, this, "afm_length")?;
    let afm_time = pref_f(vm, "afm_time")?;
    if !py::is_none(&afm_len)
        && py::int(vm, &afm_len)? != 0
        && afm_time != 0.
        && pref_b(vm, "afm_enable")?
    {
        let bonus = py::config_f64(vm, "afm_bonus").unwrap_or(25.);
        let chars = py::config_f64(vm, "afm_characters").unwrap_or(250.);
        let mut delay = (bonus + py::f64_of(vm, &afm_len)?) / chars * afm_time;
        let dp = py::get_or_none(vm, this, "dialogue_pause")?;
        if !py::is_none(&dp) {
            delay += py::f64_of(vm, &dp)?;
        }
        let tt = py::get_or_none(vm, this, "text_tuple")?;
        if !py::is_none(&tt) {
            let mut mt = 0f64;
            for t in py::items(vm, &tt)? {
                let v = vm.call_method(&t, "get_time", Vec::new())?;
                mt = mt.max(py::f64_of(vm, &v)?);
            }
            delay += mt;
        }
        // Modal screens pause auto-forward from 7.5.2 / 8.0.2.
        let modal_pause = release::at_least(vm, &[8, 0, 2])
            || (release::version(vm)[0] == 7 && release::at_least(vm, &[7, 5, 2]));
        if modal_pause && events::is_type(vm, &ev, events::TIMEEVENT)? {
            let m = py::get_or_none(vm, &ev, "modal")?;
            if ops::truthy(vm, &m)? {
                return Ok(Value::None);
            }
        }
        if st > delay {
            let sv = pref_b(vm, "self_voicing")?;
            let focused = vm.call_method(this, "is_focused", Vec::new())?;
            if !tts_active(vm) && (!sv || ops::truthy(vm, &focused)?) && afm_callback_ok(vm)? {
                return Ok(Value::Bool(true));
            }
            timeout(vm, 0.1)?;
        } else {
            timeout(vm, delay - st)?;
        }
    }
    let mut dismiss: Vec<(Value, bool)> = Vec::new();
    for d in py::get_items(vm, this, "dismiss")? {
        dismiss.push((d, true));
    }
    for d in py::get_items(vm, this, "dismiss_unfocused")? {
        dismiss.push((d, false));
    }
    let config = py::import(vm, "renpy.config")?;
    let restart = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
    for (de, check_focus) in dismiss {
        if !map_event(vm, &ev, &de)? {
            continue;
        }
        if check_focus {
            let f = vm.call_method(this, "is_focused", Vec::new())?;
            if !ops::truthy(vm, &f)? {
                continue;
            }
        }
        let skipping = py::get_or_none(vm, &config, "skipping")?;
        if ops::truthy(vm, &skipping)? {
            py::set(vm, &config, "skipping", Value::None)?;
            vm.call(&restart, Vec::new(), Vec::new())?;
            return Err(ignore_event(vm));
        }
        let p = prefs(vm)?;
        let variant = py::import_attr(vm, "renpy.exports", "variant")?;
        let mob = vm.call(&variant, vec![vm.types.new_str("mobile")], Vec::new())?;
        // Rollback-side clicks arrive in 6.99.8.
        let side = if release::before(vm, &[6, 99, 8])
            || !py::config_bool(vm, "enable_rollback_side").unwrap_or(true)
        {
            "disable".to_owned()
        } else {
            let n = if ops::truthy(vm, &mob)? {
                "mobile_rollback_side"
            } else {
                "desktop_rollback_side"
            };
            let v = py::get_or_none(vm, &p, n)?;
            py::string(vm, &v).unwrap_or_else(|_| "disable".into())
        };
        if events::is_type(vm, &ev, events::MOUSEBUTTONUP)? {
            let sw = py::config_f64(vm, "screen_width")?;
            let size = py::config_f64(vm, "rollback_side_size").unwrap_or(0.2);
            let pct = x / sw;
            let rb = py::import_attr(vm, "renpy.exports", "rollback")?;
            if (side == "left" && pct < size) || (side == "right" && 1. - pct < size) {
                vm.call(&rb, Vec::new(), Vec::new())?;
                return Err(ignore_event(vm));
            }
        }
        if pref_b(vm, "using_afm_enable")?
            && pref_b(vm, "afm_enable")?
            && !pref_b(vm, "afm_after_click")?
        {
            py::set(vm, &p, "afm_enable", Value::Bool(false))?;
            vm.call(&restart, Vec::new(), Vec::new())?;
            return Err(ignore_event(vm));
        }
        let ad = py::get_or_none(vm, this, "allow_dismiss")?;
        if ops::truthy(vm, &ad)? {
            let r = vm.call(&ad, Vec::new(), Vec::new())?;
            if !ops::truthy(vm, &r)? {
                return Err(ignore_event(vm));
            }
        }
        return Ok(Value::Bool(true));
    }
    say_skip(vm, &ev, st, &config, &restart)
}

/// The skipping tail of `SayBehavior.event`, per release:
/// - 6.18: any event once `st >= skip_delay` (no `store._skipping`);
/// - 6.99.0: only `TIMEEVENT`s advance; 6.99.5 adds `store._skipping`;
/// - 6.99.12.1: unseen dialogue stops skipping (`False`, `None` from 7.3.3);
/// - 7.4.5: a modal `TIMEEVENT` stops skipping; 7.5 / 8.0 just waits;
/// - 8.4: dialogue whose translation was seen also skips.
fn say_skip(
    vm: &mut Vm,
    ev: &Value,
    st: f64,
    config: &Value,
    restart: &Value,
) -> PyResult<Value> {
    let skip_delay = py::config_f64(vm, "skip_delay").unwrap_or(75.) / 1000.;
    let skipping = py::get_or_none(vm, config, "skipping")?;
    let sk = if release::at_least(vm, &[6, 99, 5]) {
        py::import_attr(vm, "store", "_skipping").unwrap_or(Value::Bool(true))
    } else {
        Value::Bool(true)
    };
    if !(ops::truthy(vm, &skipping)?
        && py::config_bool(vm, "allow_skipping").unwrap_or(true)
        && ops::truthy(vm, &sk)?)
    {
        return Ok(Value::None);
    }
    let advance = if release::at_least(vm, &[6, 99]) {
        events::is_type(vm, ev, events::TIMEEVENT)? && st >= skip_delay
    } else {
        st >= skip_delay
    };
    if !advance {
        timeout(vm, skip_delay - st)?;
        return Ok(Value::None);
    }
    if release::at_least(vm, &[7, 4, 5]) {
        let m = py::get_or_none(vm, ev, "modal")?;
        if ops::truthy(vm, &m)? {
            if release::state_since_pair(vm, 5, 0) {
                return Ok(Value::None);
            }
            py::set(vm, config, "skipping", Value::None)?;
            vm.call(restart, Vec::new(), Vec::new())?;
            return Ok(Value::None);
        }
    }
    if pref_b(vm, "skip_unseen")? || skipping.as_str() == Some("fast") {
        return Ok(Value::Bool(true));
    }
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let c = vm.call(&ctx, Vec::new(), Vec::new())?;
    let seen = vm.call_method(&c, "seen_current", vec![Value::Bool(true)])?;
    if ops::truthy(vm, &seen)? {
        return Ok(Value::Bool(true));
    }
    if release::at_least(vm, &[8, 4]) {
        let tlid = py::get_or_none(vm, &c, "translate_identifier")?;
        if ops::truthy(vm, &tlid)? {
            let f = py::import_attr(vm, "renpy.exports", "seen_translation")?;
            let r = vm.call(&f, vec![tlid], Vec::new())?;
            if ops::truthy(vm, &r)? {
                return Ok(Value::Bool(true));
            }
        }
    }
    if release::at_least(vm, &[6, 99, 12, 1]) {
        let stop = if release::at_least(vm, &[7, 3, 3]) {
            Value::None
        } else {
            Value::Bool(false)
        };
        py::set(vm, config, "skipping", stop)?;
        vm.call(restart, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

// DismissBehavior ----------------------------------------------------------------------------------

fn dismiss_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let action = arg_or(args, kw, 1, "action", Value::None);
    let modal = arg_or(args, kw, 2, "modal", Value::Bool(true));
    let keysym = arg_or(args, kw, 3, "keysym", vm.types.new_str("dismiss"));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "action" | "modal" | "keysym"))
        .cloned()
        .collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    if py::is_none(&action) {
        return Err(py::error(vm, "Dismiss requires an action."));
    }
    py::set(vm, this, "action", action)?;
    py::set(vm, this, "modal", modal)?;
    py::set(vm, this, "keysym", keysym)?;
    Ok(Value::None)
}

fn empty_str(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_str(""))
}

fn action_tts_all(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get_or_none(vm, &args[0], "action")?;
    let alt_v = alt(vm, &a)?;
    vm.call_method(&args[0], "_tts_common", vec![alt_v])
}

fn modal_flag(vm: &mut Vm, this: &Value) -> PyResult<(Value, bool)> {
    let m = py::get_or_none(vm, this, "modal")?;
    let callable = crysirror_python::builtins::is_callable(vm, &m);
    let t = ops::truthy(vm, &m)?;
    Ok((m, t && !callable))
}

fn dismiss_find_focusable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, mark) = modal_flag(vm, &args[0])?;
    if mark {
        let f = py::import_attr(vm, "renpy.display.focus", "mark_modal")?;
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "find_focusable")?;
    vm.call(&f, args.to_vec(), Vec::new())
}

fn dismiss_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rv = super::layout::new(vm, 0., 0.);
    vm.call_method(
        &rv,
        "add_focus",
        vec![
            args[0].clone(),
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
        ],
    )?;
    let (_, mark) = modal_flag(vm, &args[0])?;
    if mark {
        py::set(vm, &rv, "modal", vm.types.new_str("default"))?;
    }
    Ok(rv)
}

fn dismiss_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, x, y, _) = evargs(vm, args)?;
    let focused = vm.call_method(this, "is_focused", Vec::new())?;
    let keysym = py::get(vm, this, "keysym")?;
    if ops::truthy(vm, &focused)? && map_event(vm, &ev, &keysym)? {
        let snd = sty(vm, this, "activate_sound")?;
        play(vm, &snd)?;
        let a = py::get(vm, this, "action")?;
        let rv = run(vm, &a, Vec::new(), Vec::new())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
        return Err(ignore_event(vm));
    }
    let (m, _) = modal_flag(vm, this)?;
    if super::focus::check_modal(vm, &m, &ev, x, y, None, None)? {
        let ig = py::import_attr(vm, "renpy.display.layout", "IgnoreLayers")?;
        let e = vm.call(&ig, Vec::new(), Vec::new())?;
        return Err(crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()));
    }
    Ok(Value::None)
}

// Button --------------------------------------------------------------------------------------------

fn button_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "child",
        "style",
        "clicked",
        "hovered",
        "unhovered",
        "action",
        "role",
        "time_policy",
        "keymap",
        "alternate",
        "selected",
        "sensitive",
        "keysym",
        "alternate_keysym",
    ];
    let mut vals: Vec<Value> = vec![Value::None; names.len()];
    vals[1] = vm.types.new_str("button");
    vals[8] = vm.types.new_dict();
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    if is_action(vm, &vals[2]) {
        vals[5] = vals[2].clone();
    }
    let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
    let wi = py::get(vm, &window, "__init__")?;
    props.push(("style".into(), vals[1].clone()));
    vm.call(&wi, vec![this.clone(), vals[0].clone()], props)?;
    for (n, i) in [
        ("action", 5),
        ("selected", 10),
        ("sensitive", 11),
        ("clicked", 2),
        ("hovered", 3),
        ("unhovered", 4),
        ("alternate", 9),
        ("role_parameter", 6),
        ("keymap", 8),
        ("keysym", 12),
        ("alternate_keysym", 13),
    ] {
        py::set(vm, this, n, vals[i].clone())?;
    }
    py::set(vm, this, "focusable", Value::Bool(true))?;
    py::set(vm, this, "time_policy_data", Value::None)?;
    py::set(vm, this, "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn button_get_tooltip(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get_or_none(vm, &args[0], "_tooltip")?;
    if !py::is_none(&t) {
        return Ok(t);
    }
    let a = py::get_or_none(vm, &args[0], "action")?;
    get_tooltip(vm, &a)
}

fn button_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let rv = vm.call_method(this, "_copy", Vec::new())?;
    let s = py::get(vm, this, "style")?;
    let sc = vm.call_method(&s, "copy", Vec::new())?;
    py::set(vm, &rv, "style", sc)?;
    let p = py::get(vm, &s, "prefix")?;
    vm.call_method(&rv, "set_style_prefix", vec![p, Value::Bool(true)])?;
    py::set(vm, &rv, "focusable", Value::Bool(false))?;
    py::set(vm, &rv, "locked", Value::Bool(true))?;
    Ok(rv)
}

fn button_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for n in ["clicked", "hovered", "unhovered", "alternate"] {
        let a = py::get_or_none(vm, &args[0], n)?;
        f_predict_action(vm, &[a], &[])?;
    }
    Ok(Value::None)
}

fn button_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, mut st, at) = rargs(vm, args)?;
    let tp = sty(vm, this, "time_policy")?;
    if ops::truthy(vm, &tp)? {
        let data = py::get_or_none(vm, this, "time_policy_data")?;
        let s = py::get(vm, this, "style")?;
        let r = vm.call(&tp, vec![Value::Float(st), data, s], Vec::new())?;
        let r = py::items(vm, &r)?;
        st = py::f64_of(vm, &r[0])?;
        py::set(vm, this, "time_policy_data", r[1].clone())?;
    }
    let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
    let wr = py::get(vm, &window, "render")?;
    let rv = vm.call(
        &wr,
        vec![
            this.clone(),
            Value::Float(w),
            Value::Float(h),
            Value::Float(st),
            Value::Float(at),
        ],
        Vec::new(),
    )?;
    let clicked = py::get_or_none(vm, this, "clicked")?;
    if ops::truthy(vm, &clicked)? {
        let (rw, rh) = super::layout::size(&rv);
        let rect = sty(vm, this, "focus_rect")?;
        let (fx, fy, fw, fh) = if !py::is_none(&rect) {
            let r = render::rect4(vm, &rect)?;
            (r[0], r[1], r[2], r[3])
        } else {
            let lm = super::layout::style_f(vm, this, "left_margin", rw)?;
            let tm = super::layout::style_f(vm, this, "top_margin", rh)?;
            let rm = super::layout::style_f(vm, this, "right_margin", rw)?;
            let bm = super::layout::style_f(vm, this, "bottom_margin", rh)?;
            (lm, tm, rw - rm, rh - bm)
        };
        let mut mask = sty(vm, this, "focus_mask")?;
        if matches!(mask, Value::Bool(true)) {
            mask = rv.clone();
        } else if !py::is_none(&mask) {
            match render::render(vm, &mask, rw, rh, st, at) {
                Ok(r) => mask = r,
                Err(e) => {
                    if !crysirror_python::builtins::is_callable(vm, &mask) {
                        let _ = e;
                        return Err(py::error(
                            vm,
                            "Focus_mask must be None, True, a displayable, or a callable.",
                        ));
                    }
                }
            }
        }
        let (fmx, fmy) = if py::is_none(&mask) {
            (Value::None, Value::None)
        } else {
            (Value::Int(0), Value::Int(0))
        };
        vm.call_method(
            &rv,
            "add_focus",
            vec![
                this.clone(),
                Value::None,
                Value::Float(fx),
                Value::Float(fy),
                Value::Float(fw),
                Value::Float(fh),
                fmx,
                fmy,
                mask,
            ],
        )?;
    }
    Ok(rv)
}

fn role(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let r = py::get_or_none(vm, this, "role")?;
    Ok(if py::is_none(&r) {
        String::new()
    } else {
        py::string(vm, &r)?
    })
}

fn set_tevent(vm: &mut Vm, this: &Value, ev: String) -> PyResult<()> {
    let e = vm.types.new_string(ev);
    vm.call_method(this, "set_transform_event", vec![e.clone()])?;
    let c = py::get_or_none(vm, this, "child")?;
    if !py::is_none(&c) {
        vm.call_method(&c, "set_transform_event", vec![e])?;
    }
    Ok(())
}

fn button_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "focus")?;
    vm.call(
        &f,
        vec![this.clone()],
        vec![("default".into(), Value::Bool(default))],
    )?;
    let mut rv = Value::None;
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        rv = run(vm, &h, Vec::new(), Vec::new())?;
    }
    let r = role(vm, this)?;
    set_tevent(vm, this, format!("{r}hover"))?;
    Ok(rv)
}

fn button_unfocus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "unfocus")?;
    vm.call(
        &f,
        vec![this.clone()],
        vec![("default".into(), Value::Bool(default))],
    )?;
    py::set(vm, this, "longpress_start", Value::None)?;
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        run_unhovered(vm, &h)?;
        let u = py::get_or_none(vm, this, "unhovered")?;
        run(vm, &u, Vec::new(), Vec::new())?;
    }
    let r = role(vm, this)?;
    set_tevent(vm, this, format!("{r}idle"))?;
    Ok(Value::None)
}

fn button_is_selected(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::get_or_none(vm, &args[0], "selected")?;
    if !py::is_none(&s) {
        return Ok(s);
    }
    let a = py::get_or_none(vm, &args[0], "action")?;
    Ok(Value::Bool(is_selected(vm, &a)?))
}

fn button_is_sensitive(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::get_or_none(vm, &args[0], "sensitive")?;
    if !py::is_none(&s) {
        return Ok(s);
    }
    let a = py::get_or_none(vm, &args[0], "action")?;
    Ok(Value::Bool(is_sensitive(vm, &a)?))
}

fn button_per_interact(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let locked = py::get_or_none(vm, this, "locked")?;
    if !ops::truthy(vm, &locked)? {
        let action = py::get_or_none(vm, this, "action")?;
        let (mut role_s, clicked) = if !py::is_none(&action) {
            let sel = vm.call_method(this, "is_selected", Vec::new())?;
            let r = if ops::truthy(vm, &sel)? {
                "selected_"
            } else {
                ""
            };
            let sens = vm.call_method(this, "is_sensitive", Vec::new())?;
            (
                r.to_owned(),
                if ops::truthy(vm, &sens)? {
                    action.clone()
                } else {
                    Value::None
                },
            )
        } else {
            (String::new(), py::get_or_none(vm, this, "clicked")?)
        };
        let rp = py::get_or_none(vm, this, "role_parameter")?;
        if !py::is_none(&rp) {
            role_s = py::string(vm, &rp)?;
        }
        let cur_role = role(vm, this)?;
        let cur_clicked = py::get_or_none(vm, this, "clicked")?;
        if role_s != cur_role || !py::same(&clicked, &cur_clicked) {
            render::invalidate(vm, this);
            py::set(vm, this, "role", vm.types.new_string(role_s.clone()))?;
            py::set(vm, this, "clicked", clicked.clone())?;
        }
        if !py::is_none(&clicked) {
            let p = vm.types.new_string(format!("{role_s}idle_"));
            vm.call_method(this, "set_style_prefix", vec![p, Value::Bool(true)])?;
            py::set(vm, this, "focusable", Value::Bool(true))?;
        } else {
            set_tevent(vm, this, format!("{role_s}insensitive"))?;
            let p = vm.types.new_string(format!("{role_s}insensitive_"));
            vm.call_method(this, "set_style_prefix", vec![p, Value::Bool(true)])?;
            py::set(vm, this, "focusable", Value::Bool(false))?;
        }
    }
    let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
    let f = py::get(vm, &window, "per_interact")?;
    let _ = kw;
    vm.call(&f, vec![this.clone()], Vec::new())
}

fn handle_click(vm: &mut Vm, this: &Value, action: &Value) -> PyResult<Value> {
    let snd = sty(vm, this, "activate_sound")?;
    play(vm, &snd)?;
    let rv = run(vm, action, Vec::new(), Vec::new())?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    Err(ignore_event(vm))
}

fn button_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let locked = py::get_or_none(vm, this, "locked")?;
    if ops::truthy(vm, &locked)? {
        return Ok(Value::None);
    }
    let (ev, x, y, st) = evargs(vm, args)?;
    let action = py::get_or_none(vm, this, "action")?;
    if let Some(t) = run_periodic(vm, &action, st)? {
        timeout(vm, t)?;
    }
    let ty = ev_int(vm, &ev, "type")?;
    let key_ev = matches!(
        ty,
        events::KEYDOWN | events::KEYUP | events::TEXTEDITING | events::TEXTINPUT
    );
    let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
    let we = py::get(vm, &window, "event")?;
    if !key_ev || super::layout::style_b(vm, this, "key_events")? {
        let rv = vm.call(&we, args.to_vec(), kw.to_vec())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    } else {
        py::set(vm, &ev, "_suppressed", Value::Bool(true))?;
        let rv = vm.call(&we, args.to_vec(), kw.to_vec());
        py::set(vm, &ev, "_suppressed", Value::Bool(false))?;
        let rv = rv?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    let clicked = py::get_or_none(vm, this, "clicked")?;
    let alternate = py::get_or_none(vm, this, "alternate")?;
    let keysym = py::get_or_none(vm, this, "keysym")?;
    if !py::is_none(&keysym) && !py::is_none(&clicked) && map_event(vm, &ev, &keysym)? {
        return handle_click(vm, this, &clicked);
    }
    let aks = py::get_or_none(vm, this, "alternate_keysym")?;
    if !py::is_none(&aks) && !py::is_none(&alternate) && map_event(vm, &ev, &aks)? {
        return handle_click(vm, this, &alternate);
    }
    let focused = vm.call_method(this, "is_focused", Vec::new())?;
    if !ops::truthy(vm, &focused)? {
        return Ok(Value::None);
    }
    let km = py::get_or_none(vm, this, "keymap")?;
    if !py::is_none(&km) {
        for (name, a) in ops::mapping_items(vm, &km)? {
            if map_event(vm, &ev, &name)? {
                return run(vm, &a, Vec::new(), Vec::new());
            }
        }
    }
    let touch = py::import_attr(vm, "renpy.display", "touch")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false);
    if !py::is_none(&alternate) && touch {
        let dur = py::config_f64(vm, "longpress_duration").unwrap_or(0.5);
        if ty == events::MOUSEBUTTONDOWN && ev_int(vm, &ev, "button")? == 1 {
            py::set(vm, this, "longpress_start", Value::Float(st))?;
            py::set(vm, this, "longpress_x", Value::Float(x))?;
            py::set(vm, this, "longpress_y", Value::Float(y))?;
            timeout(vm, dur)?;
        }
        let ls = py::get_or_none(vm, this, "longpress_start")?;
        if !py::is_none(&ls) {
            let lx = {
                let v = py::get_or_none(vm, this, "longpress_x")?;
                py::f64_of(vm, &v)?
            };
            let ly = {
                let v = py::get_or_none(vm, this, "longpress_y")?;
                py::f64_of(vm, &v)?
            };
            let radius = py::config_f64(vm, "longpress_radius").unwrap_or(15.);
            if ty == events::MOUSEBUTTONUP && ev_int(vm, &ev, "button")? == 1 {
                py::set(vm, this, "longpress_start", Value::None)?;
            } else if (x - lx).hypot(y - ly) > radius {
                py::set(vm, this, "longpress_start", Value::None)?;
            } else if st >= py::f64_of(vm, &ls)? + dur {
                return handle_click(vm, this, &alternate);
            }
        }
    }
    if !py::is_none(&clicked) && map_name(vm, &ev, "button_ignore")? {
        return Err(ignore_event(vm));
    }
    if !py::is_none(&alternate) && map_name(vm, &ev, "button_alternate_ignore")? {
        return Err(ignore_event(vm));
    }
    if !py::is_none(&clicked) && map_name(vm, &ev, "button_select")? {
        return handle_click(vm, this, &clicked);
    }
    if !py::is_none(&alternate) && map_name(vm, &ev, "button_alternate")? {
        return handle_click(vm, this, &alternate);
    }
    Ok(Value::None)
}

fn button_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let root = args.get(2).cloned().unwrap_or(Value::Bool(true));
    if ops::truthy(vm, &root)? {
        let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
        let f = py::get(vm, &window, "set_style_prefix")?;
        vm.call(&f, args.to_vec(), Vec::new())?;
    }
    Ok(Value::None)
}

fn button_tts_all(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = py::get_or_none(vm, this, "action")?;
    let alt_v = alt(vm, &a)?;
    let rv = vm.call_method(this, "_tts_common", vec![alt_v])?;
    let sel = vm.call_method(this, "is_selected", Vec::new())?;
    if ops::truthy(vm, &sel)? {
        let s = py::get(vm, this, "style")?;
        let a = py::get_or_none(vm, &s, "alt")?;
        let ha = vm
            .call_method(&s, "_hover_alt", Vec::new())
            .unwrap_or(Value::None);
        if ops::eq(vm, &a, &ha)? {
            let tr = py::import_attr(vm, "renpy.minstore", "__")?;
            let sel_s = vm.call(&tr, vec![vm.types.new_str("selected")], Vec::new())?;
            let s1 = py::string(vm, &rv)?;
            let s2 = py::string(vm, &sel_s)?;
            return Ok(vm.types.new_string(format!("{s1} {s2}")));
        }
    }
    Ok(rv)
}

fn f_text_button(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let text = arg_or(args, kw, 0, "text", Value::None);
    let style = arg_or(args, kw, 1, "style", vm.types.new_str("button"));
    let text_style = arg_or(args, kw, 2, "text_style", vm.types.new_str("button_text"));
    let clicked = arg_or(args, kw, 3, "clicked", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "text" | "style" | "text_style" | "clicked"))
        .cloned()
        .collect();
    let mut tp = Vec::new();
    let mut bp = Vec::new();
    for (k, v) in props {
        match k.strip_prefix("text_") {
            Some(r) => tp.push((Rc::from(r), v)),
            None => bp.push((k, v)),
        }
    }
    tp.push(("style".into(), text_style));
    let tc = py::import_attr(vm, "renpy.text.text", "Text")?;
    let t = vm.call(&tc, vec![text], tp)?;
    bp.push(("style".into(), style));
    bp.push(("clicked".into(), clicked));
    let bc = py::import_attr(vm, MODULE, "Button")?;
    vm.call(&bc, vec![t], bp)
}

fn imagebutton_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "idle_image",
        "hover_image",
        "insensitive_image",
        "activate_image",
        "selected_idle_image",
        "selected_hover_image",
        "selected_insensitive_image",
        "selected_activate_image",
        "style",
        "clicked",
        "hovered",
    ];
    let mut vals: Vec<Value> = vec![Value::None; names.len()];
    vals[8] = vm.types.new_str("image_button");
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    let or = |vm: &mut Vm, a: &Value, b: &Value| -> PyResult<Value> {
        if ops::truthy(vm, a)? {
            Ok(a.clone())
        } else {
            Ok(b.clone())
        }
    };
    let idle = vals[0].clone();
    let hover = or(vm, &vals[1], &idle)?;
    let insensitive = or(vm, &vals[2], &idle)?;
    let activate = or(vm, &vals[3], &hover)?;
    let s_idle = or(vm, &vals[4], &idle)?;
    let s_hover = or(vm, &vals[5], &hover)?;
    let s_ins = or(vm, &vals[6], &insensitive)?;
    let s_act = or(vm, &vals[7], &activate)?;
    let sc = vm.types.new_dict();
    for (k, v) in [
        ("idle_", idle),
        ("hover_", hover),
        ("insensitive_", insensitive),
        ("activate_", activate),
        ("selected_idle_", s_idle),
        ("selected_hover_", s_hover),
        ("selected_insensitive_", s_ins),
        ("selected_activate_", s_act),
    ] {
        let d = super::easy::displayable(vm, &v, &Value::None, true)?;
        py::dict_set(vm, &sc, k, d)?;
    }
    py::set(vm, this, "state_children", sc)?;
    props.push(("style".into(), vals[8].clone()));
    props.push(("clicked".into(), vals[9].clone()));
    props.push(("hovered".into(), vals[10].clone()));
    button_init(vm, &[this.clone(), Value::None], &props)
}

fn imagebutton_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sc = py::get(vm, &args[0], "state_children")?;
    let mut out: Vec<Value> = ops::mapping_items(vm, &sc)?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    let c = py::get_or_none(vm, &args[0], "imagebutton_child")?;
    if !py::is_none(&c) {
        out.push(c);
    }
    Ok(vm.types.new_list(out))
}

fn imagebutton_get_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sc = sty(vm, this, "child")?;
    let raw = if ops::truthy(vm, &sc)? {
        sc
    } else {
        let s = py::get(vm, this, "style")?;
        let p = py::get(vm, &s, "prefix")?;
        let d = py::get(vm, this, "state_children")?;
        ops::getitem(vm, &d, p)?
    };
    let cur = py::get_or_none(vm, this, "imagebutton_raw_child")?;
    if !py::same(&raw, &cur) {
        py::set(vm, this, "imagebutton_raw_child", raw.clone())?;
        let dup = py::get_or_none(vm, &raw, "_duplicatable")?;
        let child = if ops::truthy(vm, &dup)? {
            let c = vm.call_method(&raw, "_duplicate", vec![Value::None])?;
            vm.call_method(&c, "_unique", Vec::new())?;
            c
        } else {
            raw
        };
        vm.call_method(&child, "per_interact", Vec::new())?;
        py::set(vm, this, "imagebutton_child", child)?;
    }
    py::get(vm, this, "imagebutton_child")
}

fn hp_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "a",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    py::set(
        vm,
        &args[0],
        "b",
        args.get(2).cloned().unwrap_or(Value::None),
    )?;
    Ok(Value::None)
}

fn hp_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "a")?;
    vm.call(&a, Vec::new(), Vec::new())?;
    let b = py::get_or_none(vm, &args[0], "b")?;
    if ops::truthy(vm, &b)? {
        return vm.call(&b, Vec::new(), Vec::new());
    }
    Ok(Value::None)
}

pub fn create_part1(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let window = py::import_attr(vm, "renpy.display.layout", "Window")?;
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm,
             name: &str,
             bases: &[Value],
             methods: Methods,
             attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, m, name, v.clone());
        Ok(v)
    };
    c(
        vm,
        "Keymap",
        &[null.clone()],
        &[
            ("__init__", keymap_init),
            ("event", keymap_event),
            ("predict_one_action", keymap_predict),
        ],
        &[
            ("capture", Value::Bool(true)),
            ("_box_skip", Value::Bool(true)),
        ],
    )?;
    c(
        vm,
        "RollForward",
        &[null.clone()],
        &[("__init__", rollforward_init), ("event", rollforward_event)],
        &[],
    )?;
    c(
        vm,
        "PauseBehavior",
        &[null.clone()],
        &[("__init__", pause_init), ("event", pause_event)],
        &[
            ("voice", Value::Bool(false)),
            ("modal", Value::Bool(false)),
            ("self_voice", Value::Bool(false)),
        ],
    )?;
    c(
        vm,
        "PredictPauseBehavior",
        &[null.clone()],
        &[("event", predictpause_event)],
        &[],
    )?;
    c(
        vm,
        "SoundStopBehavior",
        &[null.clone()],
        &[("__init__", soundstop_init), ("event", soundstop_event)],
        &[],
    )?;
    let unf = vm
        .types
        .new_list(vec![vm.types.new_str("dismiss_unfocused")]);
    c(
        vm,
        "SayBehavior",
        &[null],
        &[
            ("__init__", say_init),
            ("_tts_all", say_tts_all),
            ("set_text", say_set_text),
            ("event", say_event),
        ],
        &[
            ("focusable", Value::Bool(true)),
            ("text_tuple", Value::None),
            ("dismiss_unfocused", unf),
            ("dialogue_pause", Value::None),
        ],
    )?;
    c(
        vm,
        "DismissBehavior",
        &[d],
        &[
            ("__init__", dismiss_init),
            ("_tts", empty_str),
            ("_tts_all", action_tts_all),
            ("find_focusable", dismiss_find_focusable),
            ("render", dismiss_render),
            ("event", dismiss_event),
        ],
        &[
            ("focusable", Value::Bool(true)),
            ("keysym", vm.types.new_str("dismiss")),
        ],
    )?;
    let button = c(
        vm,
        "Button",
        &[window],
        &[
            ("__init__", button_init),
            ("_get_tooltip", button_get_tooltip),
            ("_in_current_store", button_in_current_store),
            ("predict_one_action", button_predict),
            ("render", button_render),
            ("focus", button_focus),
            ("unfocus", button_unfocus),
            ("is_selected", button_is_selected),
            ("is_sensitive", button_is_sensitive),
            ("per_interact", button_per_interact),
            ("event", button_event),
            ("set_style_prefix", button_set_style_prefix),
            ("_tts", empty_str),
            ("_tts_all", button_tts_all),
        ],
        &[
            ("_store_transform_event", Value::Bool(true)),
            ("keymap", vm.types.new_dict()),
            ("action", Value::None),
            ("alternate", Value::None),
            ("longpress_start", Value::None),
            ("longpress_x", Value::None),
            ("longpress_y", Value::None),
            ("role_parameter", Value::None),
            ("keysym", Value::None),
            ("alternate_keysym", Value::None),
            ("locked", Value::Bool(false)),
        ],
    )?;
    c(
        vm,
        "ImageButton",
        &[button],
        &[
            ("__init__", imagebutton_init),
            ("visit", imagebutton_visit),
            ("get_child", imagebutton_get_child),
        ],
        &[
            ("imagebutton_child", Value::None),
            ("imagebutton_raw_child", Value::None),
        ],
    )?;
    c(
        vm,
        "HoveredProxy",
        &[],
        &[("__init__", hp_init), ("__call__", hp_call)],
        &[],
    )?;
    let tb = vm.types.new_builtin("TextButton", f_text_button);
    set(vm, "TextButton", tb);
    let ke = vm.types.new_tuple(vec![
        Value::Int(events::KEYDOWN),
        Value::Int(events::KEYUP),
        Value::Int(events::TEXTEDITING),
        Value::Int(events::TEXTINPUT),
    ]);
    set(vm, "KEY_EVENTS", ke);
    set(vm, "event_cache", vm.types.new_dict());
    set(vm, "keyup_cache", vm.types.new_dict());
    Ok(())
}

pub fn functions() -> &'static [(&'static str, crysirror_python::object::NativeFn)] {
    &[
        ("compile_event", f_compile_event),
        ("init_keymap", f_init_keymap),
        ("clear_keymap_cache", f_clear_keymap_cache),
        ("queue_event", f_queue_event),
        ("map_event", f_map_event),
        ("map_keyup", f_map_keyup),
        ("skipping", f_skipping),
        ("inspector", |vm, a, _| {
            Ok(Value::Bool(map_name(vm, &a[0], "inspector")?))
        }),
        ("predict_action", f_predict_action),
        ("run", f_run),
        ("run_unhovered", f_run_unhovered),
        ("run_periodic", f_run_periodic),
        ("get_tooltip", f_get_tooltip),
        ("is_selected", f_is_selected),
        ("is_sensitive", f_is_sensitive),
        ("alt", f_alt),
    ]
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, functions());
    create_part1(vm, &m)?;
    super::behavior2::create_part2(vm, &m)?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
