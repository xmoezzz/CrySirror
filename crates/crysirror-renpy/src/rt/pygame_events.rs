// SPDX-License-Identifier: MPL-2.0
//! The pygame input/window modules — `event`, `key`, `mouse`, `time`,
//! `scrap`, `display`, `joystick` and `controller` — after pygame_sdl2
//! (Ren'Py 6.18 – 8.4) and the vendored `renpy.pygame` (8.5 SDL2, 8.6
//! SDL3), registered under both package names by `pygame.rs`.
//!
//! The event queue, pressed state and timers live in `events::InputState`;
//! host input arrives through `events::from_input`, and blocking calls
//! (`event.wait`, `time.wait`) end the script thread's tick through the
//! host link until input or the requested host time.
//!
//! Unavailable host capabilities (CrySirror owns the window, its
//! presentation and the devices):
//! - `display`: one window stands for the host canvas. Creating it
//!   (`set_mode`, `Window`) gives a software surface of that size, but
//!   presenting that surface (`flip`, `update`), OpenGL libraries and
//!   contexts, gamma ramps and window placement raise `error` naming that;
//!   CrySirror presents the frames of the Ren'Py renderer.
//! - `joystick` / `controller`: the host maps gamepads to Ren'Py input
//!   itself, so no SDL joystick or gamepad device is enumerable; the count
//!   is 0 and opening a device fails as SDL does for an invalid index.
//! - `scrap`: the host clipboard carries text only; other MIME types raise
//!   `error`.
//! - `mouse.set_cursor`/`ColorCursor`: the host draws its own pointer;
//!   `ColorCursor.activate` raises `error`.
use super::events::{self, input};
use super::pygame::{attr, err, local, modname, new_mod, sdl3, set, vendored};
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const NO_PRESENT: &str =
    "presenting a pygame window surface is unavailable: CrySirror presents the frames of the Ren'Py renderer";
const NO_GL: &str = "OpenGL is unavailable: CrySirror renders through its own pipeline";
const NO_WINDOW_CONTROL: &str = "the host owns the window; its placement and gamma cannot be changed";

/// `d.get(k)` of a dict value.
fn lookup(vm: &mut Vm, d: &Value, k: &Value) -> PyResult<Option<Value>> {
    match d.obj() {
        Some(o) => ops::dict_get(vm, o, k),
        None => Ok(None),
    }
}

fn none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn yes(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn no(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn pair(vm: &Vm, a: i64, b: i64) -> Value {
    vm.types.new_tuple(vec![Value::Int(a), Value::Int(b)])
}

fn class_in(vm: &mut Vm, sub: &str, name: &str, methods: &[(&'static str, NativeFn)], attrs: &[(&str, Value)]) -> PyResult<ObjRef> {
    let mn = modname(vm, sub);
    py::class(vm, &mn, name, &[], methods, attrs)
}

type NativeFn = crysirror_python::object::NativeFn;

// event --------------------------------------------------------------------------------------

/// `POSTEDEVENT` / `USEREVENT_MAX` of the running release.
fn posted_event(vm: &Vm) -> i64 {
    if sdl3(vm) { 0xFFFE } else { 0xFFFA }
}

fn userevent_max(vm: &Vm) -> i64 {
    if sdl3(vm) { 0xFFFD } else { events::USEREVENT_MAX }
}

/// The module's `event_names` dict (type -> name), registered names
/// included.
fn event_names(vm: &mut Vm) -> PyResult<Value> {
    if let Some(d) = input(vm).borrow().event_names.clone() {
        return Ok(d);
    }
    let d = vm.types.new_dict();
    let table = if sdl3(vm) {
        super::pygame_tables::SDL3_EVENT_NAMES
    } else {
        super::pygame_tables::SDL2_EVENT_NAMES
    };
    for (t, n) in table {
        let n = vm.types.new_str(n);
        ops::setitem(vm, &d, Value::Int(*t), n)?;
    }
    input(vm).borrow_mut().event_names = Some(d.clone());
    Ok(d)
}

fn ev_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ty = args.get(1).or_else(|| py::kw(kw, "type")).cloned().unwrap_or(Value::Int(0));
    py::set(vm, this, "_type", ty)?;
    if let Some(d) = args.get(2).or_else(|| py::kw(kw, "dict")) {
        if ops::truthy(vm, d)? {
            for (k, v) in py::str_items(vm, d)? {
                py::set(vm, this, &k, v)?;
            }
        }
    }
    for (k, v) in kw {
        if &**k != "type" && &**k != "dict" {
            py::set(vm, this, k, v.clone())?;
        }
    }
    Ok(Value::None)
}

fn ev_type(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_type")
}

fn ev_dict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "__dict__")
}

fn ev_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get_or_none(vm, &args[0], "_type")?;
    let t = py::int(vm, &t).unwrap_or(0);
    let user_hi = if sdl3(vm) { userevent_max(vm) } else { local(vm, "WINDOWMOVED") - 1 };
    let ename = if (events::USEREVENT..=user_hi).contains(&t) {
        format!("UserEvent{}", t - events::USEREVENT)
    } else {
        let names = event_names(vm)?;
        match lookup(vm, &names, &Value::Int(t))? {
            Some(n) => py::string(vm, &n)?,
            None => "UNKNOWN".into(),
        }
    };
    let d = py::get(vm, &args[0], "__dict__")?;
    let mut rest = Vec::new();
    let mut items = py::str_items(vm, &d)?;
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in items {
        if &*k == "_type" || &*k == "timestamp" {
            continue;
        }
        let r = ops::repr(vm, &v)?;
        rest.push(format!("{k}={r}"));
    }
    Ok(vm.types.new_string(format!("<Event({t}-{ename} {})>", rest.join(", "))))
}

fn ev_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "__dict__")?;
    let b = py::get(vm, &args[1], "__dict__")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn ev_ne(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let eq = ev_eq(vm, args, kw)?;
    Ok(Value::Bool(!ops::truthy(vm, &eq)?))
}

fn ev_bool(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get(vm, &args[0], "_type")?;
    Ok(Value::Bool(!matches!(t, Value::Int(0))))
}

/// The event types named by an optional `t` argument.
fn type_filter(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Option<Vec<i64>>> {
    let v = match py::arg(args, kw, 0, "t").or_else(|| py::kw(kw, "eventtype")) {
        None | Some(Value::None) => return Ok(None),
        Some(v) => v.clone(),
    };
    if let Value::Int(i) = v {
        return Ok(Some(vec![i]));
    }
    let mut rv = Vec::new();
    for x in py::items(vm, &v)? {
        rv.push(py::int(vm, &x)?);
    }
    Ok(Some(rv))
}

fn event_type(vm: &mut Vm, e: &Value) -> PyResult<i64> {
    match py::get_or_none(vm, e, "type")? {
        Value::Int(t) => Ok(t),
        _ => Ok(-1),
    }
}

/// Removes and returns the queued events matching `types` (all when None).
fn take_events(vm: &mut Vm, types: &Option<Vec<i64>>) -> PyResult<Vec<Value>> {
    events::due_timers(vm);
    let q = std::mem::take(&mut input(vm).borrow_mut().queue);
    let mut taken = Vec::new();
    let mut kept = Vec::new();
    for e in q {
        let t = event_type(vm, &e)?;
        if types.as_ref().is_none_or(|ts| ts.contains(&t)) {
            taken.push(e);
        } else {
            kept.push(e);
        }
    }
    let s = input(vm);
    let mut s = s.borrow_mut();
    kept.append(&mut s.queue);
    s.queue = kept;
    Ok(taken)
}

fn pop_event(vm: &mut Vm) -> Option<Value> {
    let s = input(vm);
    let mut s = s.borrow_mut();
    if s.queue.is_empty() { None } else { Some(s.queue.remove(0)) }
}

fn noevent(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "pygame_sdl2.event", "NOEVENT_EVENT")
}

fn f_get(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let types = type_filter(vm, args, kw)?;
    let q = take_events(vm, &types)?;
    Ok(vm.types.new_list(q))
}

/// `poll()`: a queued event or `NOEVENT_EVENT`. Once this tick has drawn
/// a frame and nothing is queued, the tick ends and the next tick's events
/// are returned.
fn f_poll(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    events::due_timers(vm);
    if let Some(e) = pop_event(vm) {
        return Ok(e);
    }
    if super::link::link(vm).is_some() && super::link::drew_this_tick(vm) {
        super::link::end_tick(vm, super::link::Wait::NextFrame)?;
        events::due_timers(vm);
        if let Some(e) = pop_event(vm) {
            return Ok(e);
        }
    }
    noevent(vm)
}

fn next_timer_due(vm: &Vm) -> Option<u64> {
    input(vm).borrow().timers.iter().map(|t| t.2).min()
}

/// `wait()`: blocks the script thread until an event exists.
fn f_wait(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    loop {
        events::due_timers(vm);
        if let Some(e) = pop_event(vm) {
            return Ok(e);
        }
        if super::link::link(vm).is_none() {
            return noevent(vm);
        }
        let wait = match next_timer_due(vm) {
            Some(t) => super::link::Wait::Until(t),
            None => super::link::Wait::Input,
        };
        super::link::end_tick(vm, wait)?;
    }
}

fn f_peek(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    events::due_timers(vm);
    let q = input(vm).borrow().queue.clone();
    let Some(types) = type_filter(vm, args, kw)? else {
        return Ok(Value::Bool(!q.is_empty()));
    };
    for e in &q {
        if types.contains(&event_type(vm, e)?) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn f_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let types = type_filter(vm, args, kw)?;
    take_events(vm, &types)?;
    Ok(Value::None)
}

fn f_copy_event_queue(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let q = input(vm).borrow().queue.clone();
    Ok(vm.types.new_list(q))
}

fn f_get_standard_events(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let names = event_names(vm)?;
    let hi = userevent_max(vm);
    let mut rv = Vec::new();
    for k in py::items(vm, &names)? {
        let i = py::int(vm, &k)?;
        if i < events::USEREVENT || i > hi {
            rv.push(Value::Int(i));
        }
    }
    Ok(vm.types.new_list(rv))
}

fn f_event_name(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "t", Value::Int(0));
    let names = event_names(vm)?;
    match lookup(vm, &names, &t)? {
        Some(n) => Ok(n),
        None => Ok(vm.types.new_str("UNKNOWN")),
    }
}

fn blocked_types(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Vec<i64>> {
    match type_filter(vm, args, kw)? {
        Some(t) => Ok(t),
        None => {
            let names = event_names(vm)?;
            let mut rv = Vec::new();
            for k in py::items(vm, &names)? {
                rv.push(py::int(vm, &k)?);
            }
            Ok(rv)
        }
    }
}

fn f_set_blocked(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ts = blocked_types(vm, args, kw)?;
    input(vm).borrow_mut().blocked.extend(ts);
    Ok(Value::None)
}

fn f_set_allowed(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ts = blocked_types(vm, args, kw)?;
    let s = input(vm);
    let mut s = s.borrow_mut();
    for t in ts {
        s.blocked.remove(&t);
    }
    Ok(Value::None)
}

fn f_get_blocked(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "t", Value::Int(0));
    let t = py::int(vm, &t)?;
    Ok(Value::Bool(input(vm).borrow().blocked.contains(&t)))
}

fn f_set_grab(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let on = arg_or(args, kw, 0, "on", Value::Bool(false));
    let on = ops::truthy(vm, &on)?;
    input(vm).borrow_mut().grab = on;
    Ok(Value::None)
}

fn f_get_grab(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(input(vm).borrow().grab))
}

fn f_set_mousewheel_buttons(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let flag = arg_or(args, kw, 0, "flag", Value::Bool(true));
    let flag = ops::truthy(vm, &flag)?;
    input(vm).borrow_mut().mousewheel_axis = !flag;
    Ok(Value::None)
}

fn f_get_mousewheel_buttons(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(!input(vm).borrow().mousewheel_axis))
}

fn f_post(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = arg_or(args, kw, 0, "e", Value::None);
    let cls = py::import_attr(vm, "pygame_sdl2.event", "EventType")?;
    if !cls.obj().is_some_and(|c| ops::isinstance(vm, &e, c)) {
        return Err(err(vm, "event.post must be called with an Event."));
    }
    let t = event_type(vm, &e)?;
    if input(vm).borrow().blocked.contains(&t) {
        return Ok(Value::None);
    }
    input(vm).borrow_mut().queue.push(e);
    Ok(Value::None)
}

/// `register(name)`: Ren'Py's own event names keep the numbers the Rust
/// event code uses; any other name takes the next free user event.
fn f_register(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", Value::None);
    let s = py::string(vm, &name).unwrap_or_default();
    let n = match s.as_str() {
        "TIMEEVENT" => events::TIMEEVENT,
        "PERIODIC" => events::PERIODIC,
        "REDRAW" => events::REDRAW,
        "EVENTNAME" => events::EVENTNAME,
        _ => {
            let st = input(vm);
            let mut st = st.borrow_mut();
            st.next_user_event += 1;
            events::USEREVENT + 64 + st.next_user_event
        }
    };
    let names = event_names(vm)?;
    ops::setitem(vm, &names, Value::Int(n), name)?;
    Ok(Value::Int(n))
}

/// `_make_keyboard_event(type, scancode, key, mod, repeat=False)` (8.6).
fn f_make_keyboard_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ty = arg_or(args, kw, 0, "type", Value::Int(0));
    let scancode = arg_or(args, kw, 1, "scancode", Value::Int(0));
    let key = arg_or(args, kw, 2, "key", Value::Int(0));
    let m = arg_or(args, kw, 3, "mod", Value::Int(0));
    let repeat = arg_or(args, kw, 4, "repeat", Value::Bool(false));
    let mut unicode = String::new();
    if py::int(vm, &ty)? == events::KEYDOWN {
        let k = py::int(vm, &key)?;
        if input(vm).borrow().text_input {
            if k < 0x20 {
                unicode = char::from_u32(k as u32).map(String::from).unwrap_or_default();
            }
        } else if (0x20..=0x10FFFF).contains(&k) && k != 0x7F {
            let shift = py::int(vm, &m)? & 3 != 0;
            if let Some(c) = char::from_u32(k as u32) {
                unicode = if shift { c.to_uppercase().collect() } else { c.to_string() };
            }
        }
    }
    let d = vm.types.new_dict();
    let u = vm.types.new_string(unicode);
    for (k, v) in [("scancode", scancode), ("key", key), ("mod", m), ("unicode", u), ("repeat", repeat.clone())] {
        py::dict_set(vm, &d, k, v)?;
    }
    let cls = py::import_attr(vm, "pygame_sdl2.event", "EventType")?;
    vm.call(&cls, vec![ty, d], vec![(Rc::from("repeat"), repeat)])
}

pub fn init_event(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "event",
        &[
            ("pump", none),
            ("get", f_get),
            ("poll", f_poll),
            ("wait", f_wait),
            ("peek", f_peek),
            ("clear", f_clear),
            ("get_standard_events", f_get_standard_events),
            ("event_name", f_event_name),
            ("set_blocked", f_set_blocked),
            ("set_allowed", f_set_allowed),
            ("get_blocked", f_get_blocked),
            ("set_grab", f_set_grab),
            ("get_grab", f_get_grab),
            ("set_mousewheel_buttons", f_set_mousewheel_buttons),
            ("get_mousewheel_buttons", f_get_mousewheel_buttons),
            ("post", f_post),
            ("register", f_register),
            ("copy_event_queue", f_copy_event_queue),
            ("init", none),
        ],
    );
    let ec = class_in(
        vm,
        "event",
        "EventType",
        &[
            ("__init__", ev_init),
            ("__repr__", ev_repr),
            ("__eq__", ev_eq),
            ("__ne__", ev_ne),
            ("__bool__", ev_bool),
            ("__nonzero__", ev_bool),
        ],
        &[],
    )?;
    for (n, f) in [("dict", ev_dict as NativeFn), ("type", ev_type)] {
        let g = vm.types.new_builtin(n, f);
        let p = py::property(vm, g, None)?;
        vm.types.set_attr(&ec, n, p);
    }
    set(vm, &m, "EventType", Value::Obj(ec.clone()));
    set(vm, &m, "Event", Value::Obj(ec.clone()));
    let ne = vm.call(&Value::Obj(ec), vec![Value::Int(0)], Vec::new())?;
    set(vm, &m, "NOEVENT_EVENT", ne);
    set(vm, &m, "POSTEDEVENT", Value::Int(posted_event(vm)));
    set(vm, &m, "USEREVENT_MAX", Value::Int(userevent_max(vm)));
    set(vm, &m, "SDL_TOUCH_MOUSEID", Value::Int(u32::MAX as i64));
    let rlock = py::import_attr(vm, "threading", "RLock")?;
    let lock = vm.call(&rlock, Vec::new(), Vec::new())?;
    set(vm, &m, "lock", lock);
    if sdl3(vm) {
        let e = attr(vm, "error", "error")?;
        set(vm, &m, "error", e);
        let names = event_names(vm)?;
        set(vm, &m, "event_names", names);
        let f = vm.types.new_builtin("_make_keyboard_event", f_make_keyboard_event);
        set(vm, &m, "_make_keyboard_event", f);
    } else {
        for n in ["ACTIVEEVENT", "VIDEORESIZE", "VIDEOEXPOSE", "WINDOWMOVED"] {
            set(vm, &m, n, Value::Int(local(vm, n)));
        }
    }
    Ok(m)
}

// key ----------------------------------------------------------------------------------------

fn ks_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let keys: Vec<i64> = input(vm).borrow().pressed_keys.iter().copied().collect();
    let d = vm.types.new_dict();
    for k in keys {
        ops::setitem(vm, &d, Value::Int(k), Value::Int(1))?;
        ops::setitem(vm, &d, Value::Int(k & !events::SC), Value::Int(1))?;
    }
    py::set(vm, &args[0], "_keys", d)?;
    Ok(Value::None)
}

/// `KeyboardState[key]`: 1 while `key` is down. Keycodes between DELETE
/// and CAPSLOCK name no key.
fn ks_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = py::int(vm, &args[1])?;
    if 0x7F < k && k < 0x4000_0039 {
        return Err(vm.exc_msg("IndexError", "Out of range."));
    }
    let d = py::get(vm, &args[0], "_keys")?;
    Ok(Value::Int(ops::contains(vm, &d, &Value::Int(k))? as i64))
}

fn f_get_pressed(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cls = py::import_attr(vm, "pygame_sdl2.key", "KeyboardState")?;
    vm.call(&cls, Vec::new(), Vec::new())
}

fn f_key_name(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let k = arg_or(args, kw, 0, "key", Value::Int(0));
    let k = py::int(vm, &k)?;
    let name = match k {
        8 => "Backspace".to_owned(),
        9 => "Tab".into(),
        13 => "Return".into(),
        27 => "Escape".into(),
        32 => "Space".into(),
        127 => "Delete".into(),
        k if (33..127).contains(&k) => (k as u8 as char).to_ascii_uppercase().to_string(),
        k => events::KEYS
            .iter()
            .find(|(_, v)| *v == k)
            .map(|(n, _)| {
                let n = n.trim_start_matches("K_").replace('_', " ");
                let mut c = n.chars();
                match c.next() {
                    Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
                    None => String::new(),
                }
            })
            .unwrap_or_default(),
    };
    Ok(vm.types.new_string(name))
}

fn set_text_input(vm: &mut Vm, on: bool) -> PyResult<Value> {
    input(vm).borrow_mut().text_input = on;
    if !sdl3(vm) && vendored(vm) {
        let m = py::import(vm, "pygame_sdl2.key")?;
        py::set(vm, &m, "text_input", Value::Bool(on))?;
    }
    Ok(Value::None)
}

pub fn init_key(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "key",
        &[
            ("get_focused", |vm, _, _| Ok(Value::Bool(input(vm).borrow().mouse_focused))),
            ("get_pressed", f_get_pressed),
            ("get_mods", |vm, _, _| Ok(Value::Int(input(vm).borrow().mods))),
            ("set_mods", |vm, a, k| {
                let v = arg_or(a, k, 0, "state", Value::Int(0));
                let v = py::int(vm, &v)?;
                input(vm).borrow_mut().mods = v;
                Ok(Value::None)
            }),
            // "Not possible with SDL2": upstream ignores the repeat rate.
            ("set_repeat", none),
            ("get_repeat", |vm, _, _| Ok(pair(vm, 0, 0))),
            ("name", f_key_name),
            ("start_text_input", |vm, _, _| set_text_input(vm, true)),
            ("stop_text_input", |vm, _, _| set_text_input(vm, false)),
            ("set_text_input_rect", |vm, a, k| {
                let r = arg_or(a, k, 0, "rect", Value::None);
                input(vm).borrow_mut().text_input_rect = Some(r);
                Ok(Value::None)
            }),
            ("has_screen_keyboard_support", no),
            ("is_screen_keyboard_shown", no),
        ],
    );
    let ks = class_in(vm, "key", "KeyboardState", &[("__init__", ks_init), ("__getitem__", ks_getitem)], &[])?;
    set(vm, &m, "KeyboardState", Value::Obj(ks));
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if sdl3(vm) {
        let f = vm.types.new_builtin("text_input_active", |vm, _, _| Ok(Value::Bool(input(vm).borrow().text_input)));
        set(vm, &m, "text_input_active", f);
    } else {
        set(vm, &m, "text_input", Value::Bool(false));
    }
    Ok(m)
}

// mouse --------------------------------------------------------------------------------------

fn set_active_cursor(vm: &mut Vm, v: Value) -> PyResult<Value> {
    let m = py::import(vm, "pygame_sdl2.mouse")?;
    py::set(vm, &m, "active_cursor", v)?;
    Ok(Value::None)
}

fn cc_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, n) in ["surface", "x", "y"].iter().enumerate() {
        let v = arg_or(&args[1..], kw, i, n, Value::None);
        py::set(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

fn cc_activate(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(err(vm, "color cursors are unavailable: the host draws its own pointer"))
}

pub fn init_mouse(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "mouse",
        &[
            ("init", |vm, _, _| set_active_cursor(vm, Value::None)),
            ("quit", |vm, _, _| set_active_cursor(vm, Value::None)),
            ("reset", |vm, _, _| set_active_cursor(vm, Value::None)),
            ("get_pressed", |vm, _, _| {
                let b = input(vm).borrow().mouse_buttons;
                Ok(vm.types.new_tuple(vec![Value::Int(b[1] as i64), Value::Int(b[2] as i64), Value::Int(b[3] as i64)]))
            }),
            ("get_pos", |vm, _, _| {
                let p = input(vm).borrow().pos;
                Ok(pair(vm, p.0 as i64, p.1 as i64))
            }),
            ("get_rel", |vm, _, _| {
                let s = input(vm);
                let mut s = s.borrow_mut();
                let (p, last) = (s.pos, s.rel_origin.unwrap_or(s.pos));
                s.rel_origin = Some(p);
                drop(s);
                Ok(pair(vm, (p.0 - last.0) as i64, (p.1 - last.1) as i64))
            }),
            ("set_pos", |vm, a, k| {
                let p = arg_or(a, k, 0, "pos", Value::None);
                let p = py::items(vm, &p)?;
                if p.len() != 2 {
                    return Err(vm.value_error("too many values to unpack"));
                }
                let (x, y) = (py::int(vm, &p[0])? as i32, py::int(vm, &p[1])? as i32);
                input(vm).borrow_mut().pos = (x, y);
                Ok(Value::None)
            }),
            ("set_visible", |vm, a, k| {
                let v = arg_or(a, k, 0, "visible", Value::Bool(true));
                let v = ops::truthy(vm, &v)?;
                input(vm).borrow_mut().cursor_visible = v;
                Ok(Value::None)
            }),
            ("get_focused", |vm, _, _| Ok(Value::Bool(input(vm).borrow().mouse_focused))),
            // "Does anyone use this?" - upstream ignores bitmap cursors.
            ("set_cursor", none),
            ("get_cursor", none),
        ],
    );
    let cc = class_in(vm, "mouse", "ColorCursor", &[("__init__", cc_init), ("activate", cc_activate)], &[])?;
    set(vm, &m, "ColorCursor", Value::Obj(cc));
    set(vm, &m, "active_cursor", Value::Bool(false));
    if sdl3(vm) {
        let e = attr(vm, "error", "error")?;
        set(vm, &m, "error", e);
    }
    Ok(m)
}

// time ---------------------------------------------------------------------------------------

fn ticks(vm: &Vm) -> i64 {
    super::link::now_ms(vm) as i64
}

/// Blocks the script thread for `ms` of host time; returns the elapsed
/// milliseconds.
fn delay(vm: &mut Vm, ms: i64) -> PyResult<i64> {
    let start = ticks(vm);
    if super::link::link(vm).is_some() {
        let target = (start + ms.max(0)) as u64;
        while (ticks(vm) as u64) < target {
            super::link::end_tick(vm, super::link::Wait::Until(target))?;
        }
    }
    Ok(ticks(vm) - start)
}

fn f_time_wait(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ms = arg_or(args, kw, 0, "milliseconds", Value::Int(0));
    let ms = py::int(vm, &ms)?;
    Ok(Value::Int(delay(vm, ms)?))
}

fn f_set_timer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ty = arg_or(args, kw, 0, "eventid", Value::Int(0));
    let ty = match ty {
        Value::Int(i) => i,
        e => event_type(vm, &e)?,
    };
    let ms = arg_or(args, kw, 1, "milliseconds", Value::Int(0));
    let ms = py::int(vm, &ms)?;
    let once = arg_or(args, kw, 2, "once", Value::Bool(false));
    let once = ops::truthy(vm, &once)?;
    let now = super::link::now_ms(vm);
    let s = input(vm);
    let mut s = s.borrow_mut();
    s.timers.retain(|t| t.0 != ty);
    s.one_shot.remove(&ty);
    if ms > 0 {
        s.timers.push((ty, ms as u64, now + ms as u64));
        if once {
            s.one_shot.insert(ty);
        }
    }
    Ok(Value::None)
}

fn clock_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "last", Value::Int(ticks(vm)))?;
    let l = vm.types.new_list(Vec::new());
    py::set(vm, this, "last_frames", l)?;
    py::set(vm, this, "frametime", Value::Int(0))?;
    py::set(vm, this, "raw_frametime", Value::Int(0))?;
    Ok(Value::None)
}

fn clock_tick(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let framerate = arg_or(&args[1..], kw, 0, "framerate", Value::Int(0));
    let framerate = py::f64_of(vm, &framerate)?;
    let last = py::get(vm, this, "last")?;
    let last = py::int(vm, &last)?;
    let mut now = ticks(vm);
    let raw = now - last;
    py::set(vm, this, "raw_frametime", Value::Int(raw))?;
    let frames = py::get(vm, this, "last_frames")?;
    while ops::len(vm, &frames)? > 9 {
        vm.call_method(&frames, "pop", vec![Value::Int(0)])?;
    }
    if framerate == 0.0 {
        py::set(vm, this, "last", Value::Int(now))?;
        vm.call_method(&frames, "append", vec![Value::Int(raw)])?;
        return Ok(Value::Int(raw));
    }
    let duration = (1.0 / framerate * 1000.0) as i64;
    if raw < duration {
        delay(vm, duration - raw)?;
    }
    now = ticks(vm);
    let ft = now - last;
    py::set(vm, this, "frametime", Value::Int(ft))?;
    py::set(vm, this, "last", Value::Int(now))?;
    vm.call_method(&frames, "append", vec![Value::Int(ft)])?;
    Ok(Value::Int(ft))
}

fn clock_get_fps(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let frames = py::get(vm, &args[0], "last_frames")?;
    let items = py::items(vm, &frames)?;
    let mut total = 0;
    for f in &items {
        total += py::int(vm, f)?;
    }
    if items.is_empty() {
        return Err(vm.exc_msg("ZeroDivisionError", "float division by zero"));
    }
    let avg = total as f64 / 1000.0 / items.len() as f64;
    let fps = 1.0 / avg;
    Ok(if fps.is_nan() { Value::Int(0) } else { Value::Float(fps as f32 as f64) })
}

pub fn init_time(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "time",
        &[
            ("init", none),
            ("quit", none),
            ("get_ticks", |vm, _, _| Ok(Value::Int(ticks(vm)))),
            ("wait", f_time_wait),
            ("delay", f_time_wait),
            ("set_timer", f_set_timer),
            ("_suspend_timers", |vm, a, k| {
                let v = arg_or(a, k, 0, "suspended", Value::Bool(false));
                let v = ops::truthy(vm, &v)?;
                input(vm).borrow_mut().timers_suspended = v;
                Ok(Value::None)
            }),
        ],
    );
    let clock = class_in(
        vm,
        "time",
        "Clock",
        &[
            ("__init__", clock_init),
            ("tick", clock_tick),
            ("tick_busy_loop", clock_tick),
            ("get_time", |vm, a, _| py::get(vm, &a[0], "frametime")),
            ("get_rawtime", |vm, a, _| py::get(vm, &a[0], "raw_frametime")),
            ("get_fps", clock_get_fps),
        ],
        &[],
    )?;
    set(vm, &m, "Clock", Value::Obj(clock));
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    Ok(m)
}

// scrap --------------------------------------------------------------------------------------

const SCRAP_TEXT: &str = "text/plain";

fn scrap_type(vm: &mut Vm, v: &Value) -> PyResult<String> {
    match ops::bytes_of(v) {
        Some(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
        None => py::string(vm, v),
    }
}

fn data_bytes(vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
    match ops::bytes_of(v) {
        Some(b) => Ok(b.to_vec()),
        None => Ok(py::string(vm, v)?.into_bytes()),
    }
}

fn f_scrap_get(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "type", Value::None);
    if scrap_type(vm, &t)? != SCRAP_TEXT {
        return Err(err(vm, "Not implemented."));
    }
    Ok(vm.types.new_bytes(super::clipboard_text(vm).into_bytes()))
}

fn f_scrap_get_data(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "mime_type", Value::None);
    let t = scrap_type(vm, &t)?;
    if t != SCRAP_TEXT {
        return Err(err(vm, format!("clipboard type {t} is unavailable: the host clipboard carries text only")));
    }
    Ok(vm.types.new_bytes(super::clipboard_text(vm).into_bytes()))
}

fn f_scrap_put(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "type", Value::None);
    if scrap_type(vm, &t)? != SCRAP_TEXT {
        return Err(err(vm, "Not implemented."));
    }
    let d = arg_or(args, kw, 1, "data", Value::None);
    let b = data_bytes(vm, &d)?;
    super::set_clipboard_text(vm, String::from_utf8_lossy(&b).into_owned());
    Ok(Value::None)
}

fn f_scrap_put_data(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "data_dict", Value::None);
    let mut text = None;
    for k in py::items(vm, &d)? {
        let mime = scrap_type(vm, &k)?;
        if mime != SCRAP_TEXT {
            return Err(err(vm, format!("clipboard type {mime} is unavailable: the host clipboard carries text only")));
        }
        let v = ops::getitem(vm, &d, k)?;
        text = Some(data_bytes(vm, &v)?);
    }
    super::set_clipboard_text(vm, String::from_utf8_lossy(&text.unwrap_or_default()).into_owned());
    Ok(Value::None)
}

fn f_scrap_contains(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "type", Value::None);
    if scrap_type(vm, &t)? != SCRAP_TEXT {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(!super::clipboard_text(vm).is_empty()))
}

pub fn init_scrap(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "scrap",
        &[
            ("init", none),
            ("get", f_scrap_get),
            ("get_types", |vm, _, _| {
                let t = vm.types.new_str(SCRAP_TEXT);
                Ok(vm.types.new_list(vec![t]))
            }),
            ("put", f_scrap_put),
            ("contains", f_scrap_contains),
            ("lost", no),
            ("set_mode", none),
        ],
    );
    let t = vm.types.new_str(SCRAP_TEXT);
    set(vm, &m, "SCRAP_TEXT", t);
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if sdl3(vm) {
        for (n, f) in [
            ("quit", (|vm: &mut Vm, _: &[Value], _: Kw| {
                super::set_clipboard_text(vm, String::new());
                Ok(Value::None)
            }) as NativeFn),
            ("get_data", f_scrap_get_data),
            ("get_mime_types", |vm, _, _| {
                let rv = if super::clipboard_text(vm).is_empty() { Vec::new() } else { vec![vm.types.new_str(SCRAP_TEXT)] };
                Ok(vm.types.new_list(rv))
            }),
            ("put_data", f_scrap_put_data),
        ] {
            let f = vm.types.new_builtin(n, f);
            set(vm, &m, n, f);
        }
    } else {
        let b = vm.types.new_builtin("bytes_", |vm, a, _| {
            let v = a.first().cloned().unwrap_or(Value::None);
            let b = data_bytes(vm, &v)?;
            Ok(vm.types.new_bytes(b))
        });
        set(vm, &m, "bytes_", b);
        let t = vm.types.new_str("\nSCRAP_TEXT : int\n");
        set(vm, &m, "_types", t);
    }
    Ok(m)
}

// display ------------------------------------------------------------------------------------

fn canvas(vm: &Vm) -> (i64, i64) {
    let [w, h] = super::host(vm).borrow().canvas;
    (w as i64, h as i64)
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else {
        "Linux"
    }
}

fn unavailable(vm: &mut Vm, what: &str) -> crysirror_python::PyErr {
    err(vm, what.to_owned())
}

fn display_module(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, "pygame_sdl2.display")
}

fn main_window(vm: &mut Vm) -> PyResult<Value> {
    let m = display_module(vm)?;
    py::get(vm, &m, "main_window")
}

/// The window surface: a software surface of the window size.
fn make_window_surface(vm: &mut Vm, w: i64, h: i64) -> PyResult<Value> {
    let cls = py::import_attr(vm, "pygame_sdl2.surface", "Surface")?;
    let size = pair(vm, w, h);
    let srcalpha = Value::Int(local(vm, "SRCALPHA"));
    vm.call(&cls, vec![size, srcalpha], Vec::new())
}

fn window_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = &args[1..];
    let title = arg_or(a, kw, 0, "title", Value::None);
    let res = arg_or(a, kw, 1, "resolution", Value::None);
    let flags = arg_or(a, kw, 2, "flags", Value::Int(0));
    let (mut w, mut h) = (0, 0);
    if !py::is_none(&res) {
        let r = py::items(vm, &res)?;
        if r.len() == 2 {
            w = py::int(vm, &r[0])?;
            h = py::int(vm, &r[1])?;
        }
    }
    if w == 0 || h == 0 {
        (w, h) = canvas(vm);
    }
    if !py::is_none(&title) {
        let t = py::string(vm, &title)?;
        super::set_window_title(vm, t);
    }
    py::set(vm, this, "title", title)?;
    py::set(vm, this, "flags", flags)?;
    py::set(vm, this, "_size", pair(vm, w, h))?;
    let s = make_window_surface(vm, w, h)?;
    py::set(vm, this, "surface", s)?;
    py::set(vm, this, "window_surface", Value::None)?;
    py::set(vm, this, "gl_context", Value::None)?;
    Ok(Value::None)
}

fn window_get_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_size")
}

fn window_destroy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mw = main_window(vm)?;
    if py::same(&mw, &args[0]) {
        let m = display_module(vm)?;
        py::set(vm, &m, "main_window", Value::None)?;
    }
    py::set(vm, &args[0], "surface", Value::None)?;
    Ok(Value::None)
}

fn window_resize(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let size = arg_or(&args[1..], kw, 0, "size", Value::None);
    let r = py::items(vm, &size)?;
    let (w, h) = (py::int(vm, &r[0])?, py::int(vm, &r[1])?);
    py::set(vm, &args[0], "_size", pair(vm, w, h))?;
    let s = make_window_surface(vm, w, h)?;
    py::set(vm, &args[0], "surface", s)?;
    Ok(Value::None)
}

fn window_set_caption(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(&args[1..], kw, 0, "title", Value::None);
    let s = py::string(vm, &t)?;
    super::set_window_title(vm, s);
    py::set(vm, &args[0], "title", t)?;
    Ok(Value::None)
}

fn window_flip(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(unavailable(vm, NO_PRESENT))
}

fn window_gl(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(unavailable(vm, NO_GL))
}

fn window_place(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(unavailable(vm, NO_WINDOW_CONTROL))
}

fn window_class(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut methods: Vec<(&'static str, NativeFn)> = vec![
        ("__init__", window_init),
        ("create_surface", |vm, a, _| {
            let s = py::get(vm, &a[0], "_size")?;
            let r = py::items(vm, &s)?;
            let (w, h) = (py::int(vm, &r[0])?, py::int(vm, &r[1])?);
            let s = make_window_surface(vm, w, h)?;
            py::set(vm, &a[0], "surface", s)?;
            Ok(Value::None)
        }),
        ("destroy", window_destroy),
        ("resize", window_resize),
        ("recreate_gl_context", window_gl),
        ("get_window_flags", |vm, a, _| py::get(vm, &a[0], "flags")),
        ("proxy_window_surface", none),
        ("flip", window_flip),
        ("get_surface", |vm, a, _| py::get(vm, &a[0], "surface")),
        ("update", window_flip),
        ("get_wm_info", |vm, _, _| Ok(vm.types.new_dict())),
        ("get_active", |vm, _, _| Ok(Value::Bool(input(vm).borrow().mouse_focused))),
        ("iconify", window_place),
        ("toggle_fullscreen", window_place),
        ("set_icon", window_place),
        ("set_caption", window_set_caption),
        ("get_drawable_size", window_get_size),
        ("get_size", window_get_size),
        ("restore", window_place),
        ("maximize", window_place),
        ("minimize", window_place),
        ("get_sdl_window_pointer", window_place),
        ("get_position", |vm, _, _| Ok(pair(vm, 0, 0))),
        ("set_position", window_place),
    ];
    if sdl3(vm) {
        methods.extend([
            ("get_refresh_rate", (|_: &mut Vm, _: &[Value], _: Kw| Ok(Value::Float(0.0))) as NativeFn),
            ("get_window_display_scale", |_, _, _| Ok(Value::Float(1.0))),
            ("get_window_pixel_density", |_, _, _| Ok(Value::Float(1.0))),
        ]);
    } else {
        methods.push(("set_gamma", window_place));
        methods.push(("set_gamma_ramp", window_place));
    }
    class_in(vm, "display", "Window", &methods, &[])
}

fn f_set_mode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let res = arg_or(args, kw, 0, "resolution", pair(vm, 0, 0));
    let flags = arg_or(args, kw, 1, "flags", Value::Int(0));
    let m = display_module(vm)?;
    let mw = py::get(vm, &m, "main_window")?;
    if !py::is_none(&mw) {
        vm.call_method(&mw, "resize", vec![res])?;
        return py::get(vm, &mw, "surface");
    }
    let title = py::get(vm, &m, "default_title")?;
    let cls = py::get(vm, &m, "Window")?;
    let w = vm.call(&cls, vec![title, res, flags], Vec::new())?;
    py::set(vm, &m, "main_window", w.clone())?;
    py::get(vm, &w, "surface")
}

fn on_main_window(vm: &mut Vm, method: &str, args: &[Value], or: Value) -> PyResult<Value> {
    let mw = main_window(vm)?;
    if py::is_none(&mw) {
        return Ok(or);
    }
    vm.call_method(&mw, method, args.to_vec())
}

fn info_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let (w, h) = canvas(vm);
    let this = &a[0];
    for (n, v) in [
        ("hw", Value::Bool(false)),
        ("wm", Value::Bool(true)),
        ("video_mem", Value::Int(256 * 1024 * 1024)),
        ("bitsize", Value::Int(32)),
        ("bytesize", Value::Int(4)),
        ("masks", Value::None),
        ("shifts", Value::None),
        ("losses", Value::None),
        ("current_w", Value::Int(w)),
        ("current_h", Value::Int(h)),
        ("refresh_rate", Value::Int(60)),
    ] {
        py::set(vm, this, n, v)?;
    }
    let masks = vm.types.new_tuple([0xFF0000, 0xFF00, 0xFF, 0xFF000000u32 as i64].map(Value::Int).to_vec());
    py::set(vm, this, "masks", masks)?;
    let shifts = vm.types.new_tuple([16, 8, 0, 24].map(Value::Int).to_vec());
    py::set(vm, this, "shifts", shifts)?;
    let losses = vm.types.new_tuple([0, 0, 0, 0].map(Value::Int).to_vec());
    py::set(vm, this, "losses", losses)?;
    Ok(Value::None)
}

fn info_repr(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &a[0], "__dict__")?;
    let r = ops::repr(vm, &d)?;
    Ok(vm.types.new_string(format!("<Info({r})>")))
}

fn display_bounds(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let (w, h) = canvas(vm);
    let r = vm.types.new_tuple([0, 0, w, h].map(Value::Int).to_vec());
    Ok(vm.types.new_tuple(vec![r]))
}

fn gl_attrs(vm: &mut Vm) -> PyResult<Value> {
    let m = display_module(vm)?;
    if let Some(d) = py::get_opt(vm, &m, "_crysirror_gl_attributes")? {
        return Ok(d);
    }
    let d = vm.types.new_dict();
    py::set(vm, &m, "_crysirror_gl_attributes", d.clone())?;
    Ok(d)
}

fn f_hint(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let k = arg_or(args, kw, 0, "hint", Value::None);
    let v = arg_or(args, kw, 1, "value", Value::None);
    let hints = py::import_attr(vm, "pygame_sdl2.display", "_pygame_hints")?;
    let k = scrap_type(vm, &k)?;
    let v = py::string(vm, &v)?;
    let (k, v) = (vm.types.new_string(k), vm.types.new_string(v));
    ops::setitem(vm, &hints, k, v)?;
    Ok(Value::None)
}

fn f_get_hint(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let k = arg_or(args, kw, 0, "hint", Value::None);
    let default = arg_or(args, kw, 1, "default", Value::None);
    let hints = py::import_attr(vm, "pygame_sdl2.display", "_pygame_hints")?;
    let k = scrap_type(vm, &k)?;
    let k = vm.types.new_string(k);
    Ok(lookup(vm, &hints, &k)?.unwrap_or(default))
}

pub fn init_display(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "display",
        &[
            ("sdl_main_init", none),
            ("init", none),
            ("quit", none),
            ("get_init", yes),
            ("set_mode", f_set_mode),
            ("destroy", |vm, _, _| on_main_window(vm, "destroy", &[], Value::None)),
            ("get_surface", |vm, _, _| on_main_window(vm, "get_surface", &[], Value::None)),
            ("get_window", |vm, _, _| main_window(vm)),
            ("flip", |vm, _, _| on_main_window(vm, "flip", &[], Value::None)),
            ("update", |vm, a, k| {
                let r = arg_or(a, k, 0, "rectangles", Value::None);
                on_main_window(vm, "update", &[r], Value::None)
            }),
            ("get_driver", |vm, _, _| Ok(vm.types.new_str("crysirror"))),
            ("get_wm_info", |vm, _, _| {
                let d = vm.types.new_dict();
                on_main_window(vm, "get_wm_info", &[], d)
            }),
            ("get_display_bounds", display_bounds),
            ("get_num_video_displays", |_, _, _| Ok(Value::Int(1))),
            ("gl_reset_attributes", |vm, _, _| {
                let d = gl_attrs(vm)?;
                vm.call_method(&d, "clear", Vec::new())?;
                Ok(Value::None)
            }),
            ("gl_set_attribute", |vm, a, k| {
                let f = arg_or(a, k, 0, "flag", Value::None);
                let v = arg_or(a, k, 1, "value", Value::None);
                let d = gl_attrs(vm)?;
                ops::setitem(vm, &d, f, v)?;
                Ok(Value::None)
            }),
            ("gl_get_attribute", |vm, a, k| {
                let f = arg_or(a, k, 0, "flag", Value::None);
                let d = gl_attrs(vm)?;
                match lookup(vm, &d, &f)? {
                    Some(v) => Ok(v),
                    None => Err(unavailable(vm, NO_GL)),
                }
            }),
            ("gl_load_library", window_gl),
            ("gl_unload_library", window_gl),
            ("get_active", |vm, _, _| Ok(Value::Bool(input(vm).borrow().mouse_focused))),
            ("iconify", |vm, _, _| on_main_window(vm, "iconify", &[], Value::Bool(false))),
            ("toggle_fullscreen", |vm, _, _| on_main_window(vm, "toggle_fullscreen", &[], Value::Bool(false))),
            ("set_gamma", window_place),
            ("set_gamma_ramp", window_place),
            ("set_icon", |vm, a, k| {
                let s = arg_or(a, k, 0, "surface", Value::None);
                let m = display_module(vm)?;
                py::set(vm, &m, "default_icon", s)?;
                Ok(Value::None)
            }),
            ("set_caption", |vm, a, k| {
                let t = arg_or(a, k, 0, "title", Value::None);
                let m = display_module(vm)?;
                py::set(vm, &m, "default_title", t.clone())?;
                if !py::is_none(&t) {
                    let s = py::string(vm, &t)?;
                    super::set_window_title(vm, s);
                }
                Ok(Value::None)
            }),
            ("get_caption", |vm, _, _| py::import_attr(vm, "pygame_sdl2.display", "default_title")),
            ("get_drawable_size", |vm, _, _| {
                let (w, h) = canvas(vm);
                on_main_window(vm, "get_drawable_size", &[], pair(vm, w, h))
            }),
            ("get_size", |vm, _, _| on_main_window(vm, "get_size", &[], Value::None)),
            ("get_position", |vm, _, _| on_main_window(vm, "get_position", &[], Value::None)),
            ("set_position", window_place),
            ("set_screensaver", none),
            ("get_platform", |vm, _, _| Ok(vm.types.new_str(platform_name()))),
            ("hint", f_hint),
            ("_get_hint", f_get_hint),
            ("init_done", none),
            ("main_done", none),
        ],
    );
    let win = window_class(vm)?;
    set(vm, &m, "Window", Value::Obj(win));
    let info = class_in(vm, "display", "Info", &[("__init__", info_init), ("__repr__", info_repr)], &[])?;
    set(vm, &m, "Info", Value::Obj(info));
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    set(vm, &m, "main_window", Value::None);
    set(vm, &m, "default_icon", Value::None);
    let t = vm.types.new_str(if vendored(vm) { "renpy.pygame window" } else { "pygame window" });
    set(vm, &m, "default_title", t);
    set(vm, &m, "default_swap_control", Value::Int(1));
    set(vm, &m, "GL_SWAP_CONTROL", Value::Int(local(vm, "GL_SWAP_CONTROL")));
    let hints = vm.types.new_dict();
    set(vm, &m, "_pygame_hints", hints);
    set(vm, &m, "ios", Value::Bool(cfg!(target_os = "ios")));
    if sdl3(vm) {
        for (n, f) in [
            ("get_usable_display_bounds", display_bounds as NativeFn),
            ("get_display_content_scale", |_, _, _| Ok(Value::Float(1.0))),
            ("get_refresh_rate", |_, _, _| Ok(Value::Float(0.0))),
            ("get_window_display_scale", |_, _, _| Ok(Value::Float(1.0))),
            ("get_window_pixel_density", |_, _, _| Ok(Value::Float(1.0))),
            ("set_windows_dpi_awareness", none),
        ] {
            let f = vm.types.new_builtin(n, f);
            set(vm, &m, n, f);
        }
    } else {
        let srcalpha = Value::Int(local(vm, "SRCALPHA"));
        set(vm, &m, "SRCALPHA", srcalpha);
        for (n, f) in [
            ("list_modes", (|vm: &mut Vm, _: &[Value], _: Kw| {
                let (w, h) = canvas(vm);
                let p = pair(vm, w, h);
                Ok(vm.types.new_list(vec![p]))
            }) as NativeFn),
            ("mode_ok", |vm, a, k| {
                let size = arg_or(a, k, 0, "size", Value::None);
                let r = py::items(vm, &size)?;
                let (cw, ch) = canvas(vm);
                let ok = r.len() == 2 && py::int(vm, &r[0])? <= cw && py::int(vm, &r[1])? <= ch;
                Ok(Value::Bool(ok))
            }),
        ] {
            let f = vm.types.new_builtin(n, f);
            set(vm, &m, n, f);
        }
        register_hooks(vm, &m)?;
    }
    Ok(m)
}

/// pygame_sdl2 (SDL2) modules re-export the package's `register_init` and
/// `register_quit`.
fn register_hooks(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    if vendored(vm) {
        for n in ["register_init", "register_quit"] {
            let f = attr(vm, "", n)?;
            set(vm, m, n, f);
        }
    }
    Ok(())
}

// joystick / controller ----------------------------------------------------------------------

fn no_device(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(err(vm, "Invalid joystick device index: the host maps gamepads itself and exposes no SDL devices"))
}

pub fn init_joystick(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "joystick",
        &[
            ("init", none),
            ("quit", none),
            ("get_init", yes),
            ("get_count", |_, _, _| Ok(Value::Int(0))),
        ],
    );
    let j = class_in(vm, "joystick", "Joystick", &[("__init__", no_device)], &[])?;
    set(vm, &m, "Joystick", Value::Obj(j));
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if sdl3(vm) {
        let f = vm.types.new_builtin("get_joystick_ids", |vm, _, _| Ok(vm.types.new_list(Vec::new())));
        set(vm, &m, "get_joystick_ids", f);
    } else {
        register_hooks(vm, &m)?;
    }
    Ok(m)
}

fn name_index(vm: &mut Vm, args: &[Value], kw: Kw, table: &[&str]) -> PyResult<Value> {
    let n = arg_or(args, kw, 0, "name", Value::None);
    let n = scrap_type(vm, &n)?;
    Ok(Value::Int(table.iter().position(|s| *s == n).map_or(-1, |i| i as i64)))
}

fn string_for(vm: &mut Vm, args: &[Value], kw: Kw, table: &[&str], name: &str) -> PyResult<Value> {
    let i = arg_or(args, kw, 0, name, Value::Int(-1));
    let i = py::int(vm, &i)?;
    match usize::try_from(i).ok().and_then(|i| table.get(i)) {
        Some(s) => Ok(vm.types.new_str(s)),
        None => Ok(Value::None),
    }
}

pub fn init_controller(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "controller",
        &[
            ("init", none),
            ("quit", none),
            ("get_init", yes),
            ("get_count", |_, _, _| Ok(Value::Int(0))),
            // Mappings describe SDL gamepads; the host has none to apply
            // them to, and SDL accepts a mapping without a device.
            ("add_mapping", |vm, a, k| {
                let s = arg_or(a, k, 0, "mapping", Value::None);
                let _ = scrap_type(vm, &s)?;
                Ok(Value::Int(1))
            }),
            ("add_mappings", |vm, a, k| {
                let f = arg_or(a, k, 0, "mapping_file", Value::None);
                let data = match ops::bytes_of(&f) {
                    Some(b) => b.to_vec(),
                    None => {
                        let r = vm.call_method(&f, "read", Vec::new())?;
                        data_bytes(vm, &r)?
                    }
                };
                let n = String::from_utf8_lossy(&data)
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .count();
                Ok(Value::Int(n as i64))
            }),
            ("get_axis_from_string", |vm, a, k| name_index(vm, a, k, super::input::AXES)),
            ("get_button_from_string", |vm, a, k| name_index(vm, a, k, super::input::BUTTONS)),
            ("get_string_for_axis", |vm, a, k| string_for(vm, a, k, super::input::AXES, "axis")),
            ("get_string_for_button", |vm, a, k| string_for(vm, a, k, super::input::BUTTONS, "button")),
        ],
    );
    let c = class_in(vm, "controller", "Controller", &[("__init__", no_device)], &[])?;
    set(vm, &m, "Controller", Value::Obj(c));
    let e = attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if !sdl3(vm) {
        register_hooks(vm, &m)?;
    }
    Ok(m)
}
