// SPDX-License-Identifier: MPL-2.0
//! Input helpers of the interaction loop, after Ren'Py 8.3 (MIT):
//! `renpy.display.controller` (game-controller events to `pad_*` event
//! names with repeat), `renpy.display.gesture` (touch stroke gestures),
//! `renpy.display.emulator` (touch/TV input emulation) and
//! `renpy.display.joystick`.
//!
//! The host's input frames carry keyboard and pointer state only; it
//! maps physical pads to keys itself, so no controller device is ever
//! opened (`exists()` is False) and controller events reach
//! `event()` only when posted by game code. The emulators are chosen by
//! `RENPY_EMULATOR`, which the player never sets, so `init_emulator`
//! installs the null emulator; the touch and TV filters are kept for
//! games that install them directly.
use super::{events, py};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const CONTROLLER: &str = "renpy.display.controller";
const GESTURE: &str = "renpy.display.gesture";
const EMULATOR: &str = "renpy.display.emulator";
const JOYSTICK: &str = "renpy.display.joystick";

/// SDL_GameControllerGetStringForAxis / ...ForButton.
pub const AXES: &[&str] = &["leftx", "lefty", "rightx", "righty", "lefttrigger", "righttrigger"];
pub const BUTTONS: &[&str] = &[
    "a", "b", "x", "y", "back", "guide", "start", "leftstick", "rightstick", "leftshoulder", "rightshoulder", "dpup", "dpdown", "dpleft",
    "dpright", "misc1", "paddle1", "paddle2", "paddle3", "paddle4", "touchpad",
];

const THRESHOLD: i64 = 8192 + 4096;
const ZERO_THRESHOLD: i64 = 8192;

fn get_time(vm: &mut Vm) -> PyResult<f64> {
    let f = py::import_attr(vm, "renpy.display.core", "get_time")?;
    let t = vm.call(&f, Vec::new(), Vec::new())?;
    py::f64_of(vm, &t)
}

fn interface(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display", "interface")
}

fn ev_type(vm: &mut Vm, ev: &Value) -> PyResult<i64> {
    let t = py::get(vm, ev, "type")?;
    py::int(vm, &t)
}

// renpy.display.controller -----------------------------------------------------------

fn post_event(vm: &mut Vm, control: &str, state: &str, repeat: bool) -> PyResult<()> {
    let i = interface(vm)?;
    if py::is_none(&i) || !{ let t__ = py::get(vm, &i, "keyboard_focused")?; py::truthy(vm, &t__) }? {
        return Ok(());
    }
    if { let t__ = py::import_attr(vm, CONTROLLER, "ignore")?; py::truthy(vm, &t__) }? {
        return Ok(());
    }
    let mut name = format!("pad_{control}_{state}");
    if repeat {
        name = format!("repeat_{name}");
    }
    let nv = py::s(vm, &name);
    let mut names = vec![nv.clone()];
    let map = py::config(vm, "map_pad_event")?;
    let extra = if py::truthy(vm, &map)? {
        vm.call(&map, vec![nv.clone()], Vec::new())?
    } else {
        let b = py::config(vm, "pad_bindings")?;
        let empty = vm.types.new_tuple(Vec::new());
        py::method(vm, &b, "get", vec![nv.clone(), empty])?
    };
    names.extend(ops::collect(vm, &extra)?);
    let d = py::dict(vm);
    let l = vm.types.new_list(names);
    py::dict_set(vm, &d, "eventnames", l)?;
    py::dict_set(vm, &d, "controller", nv)?;
    py::dict_set(vm, &d, "up", Value::Bool(false))?;
    let en = py::import_attr(vm, "renpy.display.core", "EVENTNAME")?;
    let cls = py::import_attr(vm, "pygame_sdl2.event", "Event")?;
    let ev = vm.call(&cls, vec![en, d], Vec::new())?;
    let post = py::import_attr(vm, "pygame_sdl2.event", "post")?;
    vm.call(&post, vec![ev], Vec::new())?;
    Ok(())
}

fn pe_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "control", args[1].clone())?;
    py::set(vm, &args[0], "state", Value::None)?;
    py::set(vm, &args[0], "repeat_time", Value::Int(0))?;
    Ok(Value::None)
}

fn pe_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "state", args[1].clone())?;
    let first = py::config_f64(vm, "controller_first_repeat")?;
    let t = get_time(vm)? + first;
    py::set(vm, this, "repeat_time", py::f(t))?;
    let c = py::get(vm, this, "control")?;
    let (c, s) = (py::string(vm, &c)?, py::string(vm, &args[1])?);
    post_event(vm, &c, &s, false)?;
    let i = interface(vm)?;
    if !py::is_none(&i) {
        py::method(vm, &i, "hide_mouse", Vec::new())?;
    }
    Ok(Value::None)
}

fn pe_repeat(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let state = py::get(vm, this, "state")?;
    let states = py::config(vm, "controller_repeat_states")?;
    if !ops::contains(vm, &states, &state)? {
        return Ok(Value::None);
    }
    let now = get_time(vm)?;
    let rt = py::get(vm, this, "repeat_time")?;
    let mut rt = py::f64_of(vm, &rt)?;
    if now < rt {
        return Ok(Value::None);
    }
    let rep = py::config_f64(vm, "controller_repeat")?;
    rt += rep;
    if rt < now {
        rt = now + rep;
    }
    py::set(vm, this, "repeat_time", py::f(rt))?;
    let c = py::get(vm, this, "control")?;
    let (c, s) = (py::string(vm, &c)?, py::string(vm, &state)?);
    post_event(vm, &c, &s, true)?;
    Ok(Value::None)
}

fn controller_event(vm: &mut Vm, control: &str, state: &str) -> PyResult<()> {
    let pes = py::import_attr(vm, CONTROLLER, "pad_events")?;
    let key = py::s(vm, control);
    let pe = match ops::getitem(vm, &pes, key.clone()) {
        Ok(v) => v,
        Err(_) => {
            let cls = py::import_attr(vm, CONTROLLER, "PadEvent")?;
            let pe = vm.call(&cls, vec![key.clone()], Vec::new())?;
            ops::setitem(vm, &pes, key, pe.clone())?;
            pe
        }
    };
    let s = py::s(vm, state);
    py::method(vm, &pe, "event", vec![s])?;
    Ok(())
}

fn f_controller_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::string(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    controller_event(vm, &c, &s)?;
    Ok(Value::None)
}

fn f_periodic(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let pes = py::import_attr(vm, CONTROLLER, "pad_events")?;
    let vals = py::method(vm, &pes, "values", Vec::new())?;
    for pe in ops::collect(vm, &vals)? {
        py::method(vm, &pe, "repeat", Vec::new())?;
    }
    Ok(Value::None)
}

fn f_exists(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, CONTROLLER, "controllers")?;
    Ok(Value::Bool(py::truthy(vm, &c)?))
}

fn f_quit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, CONTROLLER, "controllers")?;
    if ops::contains(vm, &c, &args[0])? {
        let d = c.obj().cloned().ok_or_else(|| vm.type_error("controllers"))?;
        if let Some(ctl) = ops::dict_pop(vm, &d, &args[0])? {
            py::method(vm, &ctl, "quit", Vec::new())?;
        }
        let r = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
        vm.call(&r, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

/// `start(index)`: the host owns pads, so device indices never open.
fn f_start(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    f_quit(vm, args, kw)
}

fn none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn f_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ev = args[0].clone();
    let rv = if py::config_bool(vm, "pass_controller_events")? { ev.clone() } else { Value::None };
    let ty = ev_type(vm, &ev)?;
    match ty {
        events::CONTROLLERDEVICEADDED | events::CONTROLLERDEVICEREMOVED => Ok(rv),
        events::CONTROLLERAXISMOTION => {
            let get = py::import_attr(vm, "pygame_sdl2.event", "get")?;
            let more = vm.call(&get, vec![Value::Int(events::CONTROLLERAXISMOTION)], Vec::new())?;
            let mut all = vec![ev];
            all.extend(ops::collect(vm, &more)?);
            let positions = py::import_attr(vm, CONTROLLER, "axis_positions")?;
            for e in all {
                let which = py::get_or_none(vm, &e, "which")?;
                let axis = py::get(vm, &e, "axis")?;
                let value = py::get(vm, &e, "value")?;
                let value = py::int(vm, &value)?;
                let key = vm.types.new_tuple(vec![which, axis.clone()]);
                let old = py::method(vm, &positions, "get", vec![key.clone(), Value::None])?;
                let pos = if value > THRESHOLD {
                    py::s(vm, "pos")
                } else if value < -THRESHOLD {
                    py::s(vm, "neg")
                } else if value.abs() < ZERO_THRESHOLD {
                    py::s(vm, "zero")
                } else {
                    old.clone()
                };
                if ops::eq(vm, &pos, &old)? {
                    continue;
                }
                ops::setitem(vm, &positions, key, pos.clone())?;
                let a = py::int(vm, &axis)?;
                let name = AXES.get(a as usize).copied().unwrap_or("invalid");
                let p = py::string(vm, &pos)?;
                controller_event(vm, name, &p)?;
            }
            Ok(rv)
        }
        events::CONTROLLERBUTTONDOWN | events::CONTROLLERBUTTONUP => {
            let pr = if ty == events::CONTROLLERBUTTONDOWN { "press" } else { "release" };
            let b = py::get(vm, &ev, "button")?;
            let b = py::int(vm, &b)?;
            let name = BUTTONS.get(b as usize).copied().unwrap_or("invalid");
            controller_event(vm, name, pr)?;
            Ok(rv)
        }
        events::JOYAXISMOTION | events::JOYHATMOTION | events::JOYBALLMOTION | events::JOYBUTTONDOWN | events::JOYBUTTONUP
        | events::JOYDEVICEADDED | events::JOYDEVICEREMOVED => {
            if py::config_bool(vm, "pass_joystick_events")? { Ok(ev) } else { Ok(Value::None) }
        }
        _ => Ok(ev),
    }
}

fn init_controller(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, CONTROLLER, &[
        ("load_mappings", none),
        ("init", none),
        ("post_event", |vm, a, _| {
            let c = py::string(vm, &a[0])?;
            let s = py::string(vm, &a[1])?;
            let r = a.get(2).map(|v| py::truthy(vm, v)).transpose()?.unwrap_or(false);
            post_event(vm, &c, &s, r)?;
            Ok(Value::None)
        }),
        ("exists", f_exists),
        ("quit", f_quit),
        ("start", f_start),
        ("controller_event", f_controller_event),
        ("periodic", f_periodic),
        ("event", f_event),
    ]);
    let pe = py::class(vm, CONTROLLER, "PadEvent", &[], &[("__init__", pe_init), ("event", pe_event), ("repeat", pe_repeat)], &[])?;
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "PadEvent", Value::Obj(pe));
    for n in ["controllers", "axis_positions", "pad_events"] {
        set(vm, n, vm.types.new_dict());
    }
    set(vm, "THRESHOLD", Value::Int(THRESHOLD));
    set(vm, "ZERO_THRESHOLD", Value::Int(ZERO_THRESHOLD));
    set(vm, "ignore", Value::Bool(false));
    Ok(m)
}

// renpy.display.gesture ----------------------------------------------------------------

const DIRECTIONS: &[&str] = &["n", "ne", "e", "se", "s", "sw", "w", "nw"];

fn f_dispatch_gesture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let g = py::config(vm, "gestures")?;
    let e = py::method(vm, &g, "get", vec![args[0].clone(), Value::None])?;
    if !py::is_none(&e) {
        let q = py::import_attr(vm, "renpy.exports", "queue_event")?;
        vm.call(&q, vec![e], Vec::new())?;
        let ie = py::import_attr(vm, "renpy.display.core", "IgnoreEvent")?;
        let ex = vm.call(&ie, Vec::new(), Vec::new())?;
        return Err(crysirror_python::PyErr::Exc(ex.obj().cloned().ok_or_else(|| vm.type_error("IgnoreEvent"))?));
    }
    Ok(Value::None)
}

fn gr_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "x", Value::None)?;
    py::set(vm, &args[0], "y", Value::None)?;
    Ok(Value::None)
}

fn gr_start(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "x", args[1].clone())?;
    py::set(vm, this, "y", args[2].clone())?;
    let w = py::config_f64(vm, "screen_width")?;
    let comp = py::config_f64(vm, "gesture_component_size")?;
    let stroke = py::config_f64(vm, "gesture_stroke_size")?;
    py::set(vm, this, "min_component", py::f(w * comp))?;
    py::set(vm, this, "min_stroke", py::f(w * stroke))?;
    py::set(vm, this, "current_stroke", Value::None)?;
    py::set(vm, this, "stroke_length", Value::Int(0))?;
    py::set(vm, this, "strokes", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn gr_take_point(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sx = py::get(vm, this, "x")?;
    if py::is_none(&sx) {
        return Ok(Value::None);
    }
    let sy = py::get(vm, this, "y")?;
    let (x, y) = (py::f64_of(vm, &args[1])?, py::f64_of(vm, &args[2])?);
    let dx = x - py::f64_of(vm, &sx)?;
    let dy = y - py::f64_of(vm, &sy)?;
    let length = dx.hypot(dy);
    let mc = py::get(vm, this, "min_component")?;
    if length < py::f64_of(vm, &mc)? {
        return Ok(Value::None);
    }
    py::set(vm, this, "x", args[1].clone())?;
    py::set(vm, this, "y", args[2].clone())?;
    let mut angle = dx.atan2(-dy) * 180.0 / std::f64::consts::PI + 22.5;
    if angle < 0.0 {
        angle += 360.0;
    }
    let stroke = DIRECTIONS[((angle / 45.0) as usize).min(7)];
    let cur = py::get(vm, this, "current_stroke")?;
    let total = if cur.as_str() == Some(stroke) {
        let sl = py::get(vm, this, "stroke_length")?;
        py::f64_of(vm, &sl)? + length
    } else {
        py::set(vm, this, "current_stroke", py::s(vm, stroke))?;
        length
    };
    py::set(vm, this, "stroke_length", py::f(total))?;
    let ms = py::get(vm, this, "min_stroke")?;
    if total > py::f64_of(vm, &ms)? {
        let strokes = py::get(vm, this, "strokes")?;
        let items = py::items(vm, &strokes)?;
        if items.last().and_then(|s| s.as_str()) != Some(stroke) {
            ops::list_append(vm, &strokes, py::s(vm, stroke))?;
        }
    }
    Ok(Value::None)
}

fn gr_finish(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if py::is_none(&py::get(vm, this, "x")?) {
        return Ok(Value::None);
    }
    let strokes = py::get(vm, this, "strokes")?;
    let mut rv = Value::None;
    if py::truthy(vm, &strokes)? {
        let mut names = Vec::new();
        for s in py::items(vm, &strokes)? {
            names.push(py::string(vm, &s)?);
        }
        let mut func = py::config(vm, "dispatch_gesture")?;
        if py::is_none(&func) {
            func = py::import_attr(vm, GESTURE, "dispatch_gesture")?;
        }
        let g = py::s(vm, &names.join("_"));
        // The recognizer is reset even when dispatch raises IgnoreEvent.
        let r = vm.call(&func, vec![g], Vec::new());
        py::set(vm, this, "x", Value::None)?;
        py::set(vm, this, "y", Value::None)?;
        rv = r?;
    }
    py::set(vm, this, "x", Value::None)?;
    py::set(vm, this, "y", Value::None)?;
    Ok(rv)
}

fn gr_cancel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "x", Value::None)?;
    py::set(vm, &args[0], "y", Value::None)?;
    Ok(Value::None)
}

fn gr_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, ev, x, y) = (&args[0], &args[1], args[2].clone(), args[3].clone());
    let ty = ev_type(vm, ev)?;
    if ty == events::MOUSEBUTTONDOWN {
        py::method(vm, this, "start", vec![x, y])?;
    } else if ty == events::MOUSEMOTION {
        let b = py::get(vm, ev, "buttons")?;
        let b0 = ops::getitem(vm, &b, Value::Int(0))?;
        if py::truthy(vm, &b0)? {
            py::method(vm, this, "take_point", vec![x, y])?;
        }
    } else if ty == events::MOUSEBUTTONUP {
        py::method(vm, this, "take_point", vec![x, y])?;
        let b = py::get(vm, ev, "button")?;
        if py::int(vm, &b)? == 1 {
            return py::method(vm, this, "finish", Vec::new());
        }
    }
    Ok(Value::None)
}

fn init_gesture(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, GESTURE, &[("dispatch_gesture", f_dispatch_gesture)]);
    let gr = py::class(vm, GESTURE, "GestureRecognizer", &[], &[
        ("__init__", gr_init),
        ("start", gr_start),
        ("take_point", gr_take_point),
        ("finish", gr_finish),
        ("cancel", gr_cancel),
        ("event", gr_event),
    ], &[])?;
    let r = vm.call(&Value::Obj(gr.clone()), Vec::new(), Vec::new())?;
    let dirs = vm.types.new_list(DIRECTIONS.iter().map(|d| vm.types.new_str(d)).collect());
    crysirror_python::modules::set(vm, &m, "DIRECTIONS", dirs);
    crysirror_python::modules::set(vm, &m, "GestureRecognizer", Value::Obj(gr));
    crysirror_python::modules::set(vm, &m, "recognizer", r);
    Ok(m)
}

// renpy.display.emulator ------------------------------------------------------------------

fn triple(vm: &Vm, ev: Value, x: Value, y: Value) -> Value {
    vm.types.new_tuple(vec![ev, x, y])
}

fn f_null_emulator(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(triple(vm, args[0].clone(), args[1].clone(), args[2].clone()))
}

fn key_of(vm: &mut Vm, ev: &Value) -> PyResult<(i64, i64)> {
    let k = py::get(vm, ev, "key")?;
    let m = py::get_or_none(vm, ev, "mod")?;
    Ok((py::int(vm, &k)?, if py::is_none(&m) { 0 } else { py::int(vm, &m)? }))
}

fn keysym(vm: &mut Vm, name: &str) -> PyResult<i64> {
    let v = py::import_attr(vm, "pygame_sdl2", name)?;
    py::int(vm, &v)
}

fn f_touch_emulator(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (ev, mut x, mut y) = (args[0].clone(), args[1].clone(), args[2].clone());
    let i = interface(vm)?;
    if { let t__ = py::get(vm, &i, "text_rect")?; py::truthy(vm, &t__) }? {
        return Ok(triple(vm, ev, x, y));
    }
    let ios = { let t__ = py::import_attr(vm, EMULATOR, "ios")?; py::truthy(vm, &t__) }?;
    let ty = ev_type(vm, &ev)?;
    let touch_keys = [keysym(vm, "K_ESCAPE")?, keysym(vm, "K_PAGEUP")?];
    let shift = keysym(vm, "KMOD_SHIFT")?;
    if ty == events::MOUSEBUTTONDOWN || ty == events::MOUSEBUTTONUP {
        let b = py::get(vm, &ev, "button")?;
        if py::int(vm, &b)? != 1 {
            return Ok(triple(vm, Value::None, x, y));
        }
        if ty == events::MOUSEBUTTONUP {
            let d = py::dict(vm);
            let z = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
            py::dict_set(vm, &d, "pos", z.clone())?;
            py::dict_set(vm, &d, "rel", z)?;
            let bs = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0), Value::Int(0)]);
            py::dict_set(vm, &d, "buttons", bs)?;
            let cls = py::import_attr(vm, "pygame_sdl2.event", "Event")?;
            let mv = vm.call(&cls, vec![Value::Int(events::MOUSEMOTION), d], Vec::new())?;
            py::set(vm, &i, "pushed_event", mv)?;
        }
    } else if ty == events::MOUSEMOTION {
        let b = py::get(vm, &ev, "buttons")?;
        let b0 = ops::getitem(vm, &b, Value::Int(0))?;
        if !py::truthy(vm, &b0)? {
            x = Value::Int(0);
            y = Value::Int(0);
        }
    } else if (ty == events::KEYDOWN || ty == events::KEYUP) && !ios {
        let (key, m) = key_of(vm, &ev)?;
        let shifted = m & shift != 0;
        if ty == events::KEYDOWN {
            if !shifted && !touch_keys.contains(&key) {
                return Ok(triple(vm, Value::None, x, y));
            }
        } else if !touch_keys.contains(&key) {
            return Ok(triple(vm, Value::None, x, y));
        }
    }
    Ok(triple(vm, ev, x, y))
}

fn f_tv_emulator(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (ev, x, y) = (args[0].clone(), args[1].clone(), args[2].clone());
    let ty = ev_type(vm, &ev)?;
    if ty == events::MOUSEBUTTONDOWN || ty == events::MOUSEBUTTONUP || ty == events::MOUSEMOTION {
        return Ok(triple(vm, Value::None, x, y));
    }
    if ty == events::KEYDOWN {
        let mut keys = Vec::new();
        for k in ["K_LEFT", "K_RIGHT", "K_UP", "K_DOWN", "K_RETURN", "K_ESCAPE", "K_PAGEUP"] {
            keys.push(keysym(vm, k)?);
        }
        let (key, _) = key_of(vm, &ev)?;
        if !keys.contains(&key) {
            return Ok(triple(vm, Value::None, x, y));
        }
    }
    Ok(triple(vm, ev, x, y))
}

fn f_dynamic_keyboard(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, EMULATOR)?;
    if py::is_none(&py::get(vm, &m, "keyboard")?) {
        let store = py::import(vm, "store")?;
        let k = |s: &str, v: Value| (Rc::<str>::from(s), v);
        let solid = py::get(vm, &store, "Solid")?;
        let shade = py::s(vm, "#0008");
        let s = vm.call(&solid, vec![shade], vec![k("yalign", py::f(1.0)), k("ymaximum", py::f(0.625))])?;
        let text = py::get(vm, &store, "Text")?;
        let label = py::s(vm, "On-Screen Keyboard");
        let t = vm.call(&text, vec![label], vec![k("xalign", py::f(0.5)), k("yalign", py::f(0.75))])?;
        let fixed = py::get(vm, &store, "Fixed")?;
        let kb = vm.call(&fixed, vec![s, t], Vec::new())?;
        py::set(vm, &m, "keyboard", kb)?;
        let null = py::get(vm, &store, "Null")?;
        let n = vm.call(&null, Vec::new(), Vec::new())?;
        py::set(vm, &m, "null", n)?;
    }
    let i = interface(vm)?;
    let old = py::get(vm, &i, "old_text_rect")?;
    let which = if py::truthy(vm, &old)? { "keyboard" } else { "null" };
    let rv = py::get(vm, &m, which)?;
    Ok(vm.types.new_tuple(vec![rv, py::f(0.33)]))
}

/// `init_emulator()`: `RENPY_EMULATOR` is unset in the player.
fn f_init_emulator(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, EMULATOR)?;
    let ne = py::get(vm, &m, "null_emulator")?;
    py::set(vm, &m, "emulator", ne)?;
    py::set(vm, &m, "overlay", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn init_emulator(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, EMULATOR, &[
        ("null_emulator", f_null_emulator),
        ("touch_emulator", f_touch_emulator),
        ("tv_emulator", f_tv_emulator),
        ("dynamic_keyboard", f_dynamic_keyboard),
        ("init_emulator", f_init_emulator),
        ("early_init_emulator", none),
    ]);
    let ne = py::get_raw(&m, "null_emulator");
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "emulator", ne);
    set(vm, "overlay", vm.types.new_list(Vec::new()));
    set(vm, "ios", Value::Bool(false));
    set(vm, "keyboard", Value::None);
    set(vm, "null", Value::None);
    Ok(m)
}

// renpy.display.joystick ------------------------------------------------------------------

fn f_count(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "pygame_sdl2.joystick", "get_count")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn f_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let j = py::import_attr(vm, JOYSTICK, "joysticks")?;
    if ops::contains(vm, &j, &args[0])? {
        return ops::getitem(vm, &j, args[0].clone());
    }
    Ok(Value::None)
}

fn init_joystick(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, JOYSTICK, &[("count", f_count), ("get", f_get)]);
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let jb = py::class(vm, JOYSTICK, "JoyBehavior", &[null], &[], &[])?;
    crysirror_python::modules::set(vm, &m, "JoyBehavior", Value::Obj(jb));
    crysirror_python::modules::set(vm, &m, "joysticks", vm.types.new_dict());
    crysirror_python::modules::set(vm, &m, "enabled", Value::Bool(false));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(CONTROLLER, init_controller);
    vm.register_native_module(GESTURE, init_gesture);
    vm.register_native_module(EMULATOR, init_emulator);
    vm.register_native_module(JOYSTICK, init_joystick);
}
