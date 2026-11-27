// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.displayable`: the Displayable base class, placement and
//! position arithmetic (`absolute`, `position`), after Ren'Py 8.3 (MIT).
//!
//! Classes are ordinary Python classes whose methods are natives, so games
//! can subclass them, saves pickle them by their upstream path, and the
//! natives read and write the same instance attributes upstream uses.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

/// `absolute.compute_raw(value, room)`: ints and `absolute` are pixels,
/// floats are fractions of `room`, `position` combines both.
pub fn compute_raw(vm: &mut Vm, value: &Value, room: f64) -> PyResult<f64> {
    match value {
        Value::Int(i) => Ok(*i as f64),
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Float(f) => Ok(f * room),
        Value::None => Ok(0.),
        Value::Obj(o) => {
            let cls = o.class_name();
            if cls == "absolute" {
                return py::f64_of(vm, value);
            }
            if cls == "position" {
                let a = ops::getattr(vm, value, "absolute")?;
                let r = ops::getattr(vm, value, "relative")?;
                return Ok(py::f64_of(vm, &a)? + py::f64_of(vm, &r)? * room);
            }
            if matches!(o.payload, crysirror_python::Payload::Int(_)) {
                return py::f64_of(vm, value);
            }
            if matches!(o.payload, crysirror_python::Payload::Float(_)) {
                // A float subclass other than absolute is relative.
                return Ok(py::f64_of(vm, value)? * room);
            }
            Err(vm.type_error(format!("cannot place with {cls}")))
        }
    }
}

/// `place(width, height, sw, sh, placement)`.
pub fn place(vm: &mut Vm, placement: &Value, width: f64, height: f64, sw: f64, sh: f64) -> PyResult<(f64, f64)> {
    let p = py::items(vm, placement)?;
    if p.len() < 6 {
        return Err(vm.type_error("placement must have 7 components"));
    }
    let xpos = compute_raw(vm, &p[0], width)?;
    let ypos = compute_raw(vm, &p[1], height)?;
    let xanchor = compute_raw(vm, &p[2], sw)?;
    let yanchor = compute_raw(vm, &p[3], sh)?;
    let xoffset = if py::is_none(&p[4]) { 0. } else { py::f64_of(vm, &p[4])? };
    let yoffset = if py::is_none(&p[5]) { 0. } else { py::f64_of(vm, &p[5])? };
    Ok((xpos + xoffset - xanchor, ypos + yoffset - yanchor))
}

fn f_place(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args.len() != 5 {
        return Err(vm.type_error("place(width, height, sw, sh, placement)"));
    }
    let w = py::f64_of(vm, &args[0])?;
    let h = py::f64_of(vm, &args[1])?;
    let sw = py::f64_of(vm, &args[2])?;
    let sh = py::f64_of(vm, &args[3])?;
    let (x, y) = place(vm, &args[4], w, h, sw, sh)?;
    Ok(vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]))
}

fn f_compute_raw(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args.len() != 2 {
        return Err(vm.type_error("compute_raw(value, room)"));
    }
    let room = py::f64_of(vm, &args[1])?;
    Ok(Value::Float(compute_raw(vm, &args[0], room)?))
}

pub fn new_style(vm: &mut Vm, parent: Value, properties: Kw) -> PyResult<Value> {
    let style = py::import_attr(vm, "renpy.style", "Style")?;
    let props = py::kwargs_dict(vm, properties)?;
    vm.call(&style, vec![parent, props], Vec::new())
}

/// `Displayable.__init__(self, focus=None, default=False, style='default',
/// _args=None, tooltip=None, default_focus=False, **properties)`.
pub fn d_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = args.first().cloned().ok_or_else(|| vm.type_error("Displayable.__init__"))?;
    let mut focus = args.get(1).cloned().unwrap_or(Value::None);
    let mut default = args.get(2).cloned().unwrap_or(Value::Bool(false));
    let mut style = args.get(3).cloned();
    let mut a = args.get(4).cloned().unwrap_or(Value::None);
    let mut tooltip = args.get(5).cloned().unwrap_or(Value::None);
    let mut default_focus = args.get(6).cloned().unwrap_or(Value::Bool(false));
    let mut props = Vec::new();
    for (k, v) in kw {
        match &**k {
            "focus" => focus = v.clone(),
            "default" => default = v.clone(),
            "style" => style = Some(v.clone()),
            "_args" => a = v.clone(),
            "tooltip" => tooltip = v.clone(),
            "default_focus" => default_focus = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    let style = style.unwrap_or_else(|| vm.types.new_str("default"));
    let style = new_style(vm, style, &props)?;
    py::set(vm, &this, "style", style)?;
    py::set(vm, &this, "focus_name", focus)?;
    let d = ops::truthy(vm, &default)? || ops::truthy(vm, &default_focus)?;
    py::set(vm, &this, "default", Value::Bool(d))?;
    py::set(vm, &this, "_tooltip", tooltip)?;
    if !py::is_none(&a) {
        py::set(vm, &this, "_args", a)?;
    }
    Ok(Value::None)
}

fn d_copy(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let copy = py::import_attr(vm, "copy", "copy")?;
    let rv = vm.call(&copy, vec![args[0].clone()], Vec::new())?;
    if let Some(a) = arg(args, kw, 1, "args") {
        if !py::is_none(a) {
            py::set(vm, &rv, "_args", a.clone())?;
        }
    }
    Ok(rv)
}

fn d_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if let Some(a) = arg(args, kw, 1, "args") {
        if !py::is_none(a) {
            let extra = ops::getattr(vm, a, "args")?;
            if ops::truthy(vm, &extra)? {
                vm.call_method(a, "extraneous", Vec::new())?;
            }
        }
    }
    Ok(args[0].clone())
}

fn d_self(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn d_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn d_false(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn d_get_tooltip(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get_or_none(vm, &args[0], "_tooltip")
}

fn d_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn d_parameterize(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    let parameters = arg_or(args, kw, 2, "parameters", Value::None);
    let da = ops::getattr(vm, &args[0], "_args")?;
    let copy = ops::getattr(vm, &da, "copy")?;
    let a = vm.call(&copy, Vec::new(), vec![("name".into(), name), ("args".into(), parameters)])?;
    vm.call_method(&args[0], "_duplicate", vec![a])
}

fn d_equals(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], args.get(1).cloned().unwrap_or(Value::None));
    let ta = ops::type_of(vm, a);
    let tb = ops::type_of(vm, &b);
    if !py::same(&Value::Obj(ta), &Value::Obj(tb)) {
        return Ok(Value::Bool(false));
    }
    for n in ["focus_name", "style", "default"] {
        let x = py::get_or_none(vm, a, n)?;
        let y = py::get_or_none(vm, &b, n)?;
        if !ops::eq(vm, &x, &y)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn d_ne(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = ops::eq(vm, &args[0], &args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(Value::Bool(!r))
}

fn d_find_focusable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let callback = arg_or(args, kw, 1, "callback", Value::None);
    let inherited = arg_or(args, kw, 2, "focus_name", Value::None);
    let own = py::get_or_none(vm, this, "focus_name")?;
    let focus_name = if ops::truthy(vm, &own)? { own } else { inherited };
    let focusable = py::get_or_none(vm, this, "focusable")?;
    if ops::truthy(vm, &focusable)? {
        vm.call(&callback, vec![this.clone(), focus_name.clone()], Vec::new())?;
    } else if !py::is_none(&focusable) {
        vm.call(&callback, vec![Value::None, focus_name.clone()], Vec::new())?;
    }
    let children = vm.call_method(this, "visit", Vec::new())?;
    for c in py::items(vm, &children)? {
        if py::is_none(&c) {
            continue;
        }
        let f = ops::getattr(vm, &c, "find_focusable")?;
        vm.call(&f, vec![callback.clone(), focus_name.clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn role(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let r = py::get_or_none(vm, this, "role")?;
    Ok(if py::is_none(&r) { String::new() } else { py::string(vm, &r)? })
}

fn d_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") { Some(v) => ops::truthy(vm, v)?, None => false };
    let prefix = format!("{}hover_", role(vm, this)?);
    let p = vm.types.new_string(prefix);
    vm.call_method(this, "set_style_prefix", vec![p, Value::Bool(true)])?;
    if !default {
        let style = ops::getattr(vm, this, "style")?;
        let sound = ops::getattr(vm, &style, "hover_sound")?;
        if !py::is_none(&sound) {
            let play = py::import_attr(vm, "renpy.exports", "play")?;
            vm.call(&play, vec![sound], Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn d_unfocus(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prefix = format!("{}idle_", role(vm, this)?);
    let p = vm.types.new_string(prefix);
    vm.call_method(this, "set_style_prefix", vec![p, Value::Bool(true)])?;
    Ok(Value::None)
}

fn d_is_focused(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(super::focus::is_focused(vm, &args[0])?))
}

fn d_set_style_prefix(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prefix = arg_or(args, kw, 1, "prefix", Value::None);
    let style = ops::getattr(vm, this, "style")?;
    let current = ops::getattr(vm, &style, "prefix")?;
    if ops::eq(vm, &prefix, &current)? {
        return Ok(Value::None);
    }
    vm.call_method(&style, "set_prefix", vec![prefix])?;
    super::render::redraw(vm, this, 0.);
    Ok(Value::None)
}

fn d_render(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Render not implemented."))
}

fn d_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let style = ops::getattr(vm, &args[0], "style")?;
    vm.call_method(&style, "get_placement", Vec::new())
}

fn d_visit(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_list(Vec::new()))
}

/// `visit_all(callback, seen=None)`: children first (each through its own
/// `visit_all`, which screens override), then `callback(self)`.
fn d_visit_all(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let callback = arg_or(args, kw, 1, "callback", Value::None);
    let seen = match arg(args, kw, 2, "seen") {
        Some(s) if !py::is_none(s) => s.clone(),
        _ => vm.types.new_set(Default::default()),
    };
    visit_all(vm, &args[0], &callback, &seen)?;
    Ok(Value::None)
}

pub fn visit_all(vm: &mut Vm, d: &Value, callback: &Value, seen: &Value) -> PyResult<()> {
    let children = vm.call_method(d, "visit", Vec::new())?;
    for c in py::items(vm, &children)? {
        if py::is_none(&c) {
            continue;
        }
        let id = Value::Int(c.id());
        if ops::contains(vm, seen, &id)? {
            continue;
        }
        ops::set_add(vm, seen, id)?;
        vm.call_method(&c, "visit_all", vec![callback.clone(), seen.clone()])?;
    }
    vm.call(callback, vec![d.clone()], Vec::new())?;
    Ok(())
}

fn d_place(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = |i: usize, n: &str| arg(args, kw, i, n).cloned().unwrap_or(Value::None);
    let dest = a(1, "dest");
    let x = py::f64_of(vm, &a(2, "x"))?;
    let y = py::f64_of(vm, &a(3, "y"))?;
    let width = py::f64_of(vm, &a(4, "width"))?;
    let height = py::f64_of(vm, &a(5, "height"))?;
    let surf = a(6, "surf");
    let main = match arg(args, kw, 7, "main") { Some(v) => ops::truthy(vm, v)?, None => true };
    let placement = vm.call_method(this, "get_placement", Vec::new())?;
    let pl = py::items(vm, &placement)?;
    let subpixel = match pl.get(6) { Some(v) => ops::truthy(vm, v)?, None => false };
    let (sw, sh) = super::render::source_of(vm, &surf)?.size();
    let (px, py_) = place(vm, &placement, width, height, sw, sh)?;
    let pos = vm.types.new_tuple(vec![Value::Float(px + x), Value::Float(py_ + y)]);
    if !py::is_none(&dest) {
        let m = if subpixel { "subpixel_blit" } else { "blit" };
        vm.call_method(&dest, m, vec![surf, pos.clone(), Value::Bool(main), Value::Bool(main), Value::None])?;
    }
    Ok(pos)
}

fn d_set_transform_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let event = arg_or(args, kw, 1, "event", Value::None);
    let responder = py::get_or_none(vm, this, "transform_event_responder")?;
    let store = py::get_or_none(vm, this, "_store_transform_event")?;
    let responder = ops::truthy(vm, &responder)?;
    if responder || ops::truthy(vm, &store)? {
        let current = py::get_or_none(vm, this, "transform_event")?;
        if ops::eq(vm, &event, &current)? {
            return Ok(Value::None);
        }
        py::set(vm, this, "transform_event", event)?;
        if responder {
            super::render::redraw(vm, this, 0.);
        }
    }
    Ok(Value::None)
}

fn d_change_transform_child(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args.get(1).cloned().unwrap_or(Value::None))
}

fn d_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cls = this.obj().map(|o| o.class_name()).unwrap_or_default();
    let info = vm.call_method(this, "_repr_info", Vec::new())?;
    let base = format!("<{} object at {:#x}>", cls, this.id());
    if py::is_none(&info) {
        return Ok(vm.types.new_string(base));
    }
    let mut info = py::string(vm, &info)?;
    if !info.is_empty() && !(info.starts_with('(') && info.ends_with(')')) {
        info = format!("({info})");
    }
    Ok(vm.types.new_string(format!("<{} {} at {:#x}>", cls, info, this.id())))
}

/// `_tts_common(default_alt=None, reverse=False)`: concatenated speech of
/// the children, substituted into `style.alt`.
fn d_tts_common(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default_alt = arg_or(args, kw, 1, "default_alt", Value::None);
    let reverse = match arg(args, kw, 2, "reverse") { Some(v) => ops::truthy(vm, v)?, None => false };
    let children = vm.call_method(this, "visit", Vec::new())?;
    let mut children = py::items(vm, &children)?;
    if reverse {
        children.reverse();
    }
    let mut parts: Vec<String> = Vec::new();
    for c in children {
        if py::is_none(&c) {
            continue;
        }
        let speech = vm.call_method(&c, "_tts", Vec::new())?;
        let done = speech.obj().is_some_and(|o| o.class_name() == "TTSDone");
        let text = py::string(vm, &speech)?;
        if done {
            parts = if text.trim().is_empty() { Vec::new() } else { vec![text] };
            break;
        }
        if !text.trim().is_empty() {
            parts.push(text);
        }
    }
    let rv = parts.join(": ").replace("::", ":").replace(": :", ":");
    let style = ops::getattr(vm, this, "style")?;
    let mut alt = ops::getattr(vm, &style, "alt")?;
    if py::is_none(&alt) {
        alt = default_alt;
    }
    if py::is_none(&alt) {
        return Ok(vm.types.new_string(rv));
    }
    let substitute = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    let scope = py::dict(vm);
    let t = vm.types.new_string(rv);
    py::dict_set(vm, &scope, "text", t)?;
    let r = vm.call(&substitute, vec![alt], vec![("scope".into(), scope)])?;
    let r = py::items(vm, &r)?;
    Ok(r.into_iter().next().unwrap_or(Value::None))
}

fn d_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "_tts_common", Vec::new())
}

fn d_in_current_store(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

// DisplayableArguments --------------------------------------------------------

fn da_copy(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cls = ops::type_of(vm, &args[0]);
    let rv = vm.call(&Value::Obj(cls), Vec::new(), Vec::new())?;
    let src = ops::getattr(vm, &args[0], "__dict__")?;
    let dst = ops::getattr(vm, &rv, "__dict__")?;
    vm.call_method(&dst, "update", vec![src])?;
    for (k, v) in kw {
        py::set(vm, &rv, k, v.clone())?;
    }
    Ok(rv)
}

fn da_extraneous(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let developer = py::config_bool(vm, "developer")?;
    let report = py::config(vm, "report_extraneous_attributes").ok().map(|v| ops::truthy(vm, &v)).transpose()?.unwrap_or(true);
    if developer && report {
        let name = ops::getattr(vm, &args[0], "name")?;
        let a = ops::getattr(vm, &args[0], "args")?;
        let join = |vm: &mut Vm, v: &Value| -> PyResult<String> {
            Ok(py::items(vm, v)?.iter().map(|x| x.as_str().unwrap_or("?").to_owned()).collect::<Vec<_>>().join(" "))
        };
        let n = join(vm, &name)?;
        let a = join(vm, &a)?;
        return Err(py::error(vm, format!("Image '{n}' does not accept attributes '{a}'.")));
    }
    Ok(Value::None)
}

pub struct Classes {
    pub displayable: ObjRef,
    pub displayable_arguments: ObjRef,
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.display.displayable", &[]);
    create(vm, &m)?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module("renpy.display.displayable", init_module);
}

/// Creates the base classes into the module.
fn create(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let empty = vm.types.empty_tuple.clone();
    let da = py::class(vm, "renpy.display.displayable", "DisplayableArguments", &[object.clone()], &[
        ("copy", da_copy),
        ("extraneous", da_extraneous),
    ], &[("name", empty.clone()), ("args", empty.clone()), ("prefix", Value::None), ("lint", Value::Bool(false))])?;
    let da_default = vm.call(&Value::Obj(da.clone()), Vec::new(), Vec::new())?;
    let composite: Value = vm.types.new_list(Vec::new());
    let d = py::class(vm, "renpy.display.displayable", "Displayable", &[object], &[
        ("__init__", d_init),
        ("__ne__", d_ne),
        ("__repr__", d_repr),
        ("_copy", d_copy),
        ("_duplicate", d_duplicate),
        ("_get_tooltip", d_get_tooltip),
        ("_in_current_store", d_in_current_store),
        ("_unique", d_unique),
        ("parameterize", d_parameterize),
        ("_equals", d_equals),
        ("_repr_info", d_none),
        ("find_focusable", d_find_focusable),
        ("focus", d_focus),
        ("unfocus", d_unfocus),
        ("is_focused", d_is_focused),
        ("set_style_prefix", d_set_style_prefix),
        ("render", d_render),
        ("event", d_none),
        ("get_placement", d_get_placement),
        ("visit_all", d_visit_all),
        ("visit", d_visit),
        ("per_interact", d_none),
        ("predict_one", d_none),
        ("predict_one_action", d_none),
        ("place", d_place),
        ("set_transform_event", d_set_transform_event),
        ("_handles_event", d_false),
        ("_hide", d_none),
        ("_show", d_none),
        ("_target", d_self),
        ("_change_transform_child", d_change_transform_child),
        ("_clear", d_none),
        ("_tts_common", d_tts_common),
        ("_tts", d_tts),
        ("_tts_all", d_tts),
    ], &[
        ("focusable", Value::None),
        ("full_focus_name", Value::None),
        ("role", vm.types.new_str("")),
        ("transform_event", Value::None),
        ("transform_event_responder", Value::Bool(false)),
        ("_main", Value::None),
        ("_composite_parts", composite),
        ("_location", Value::None),
        ("_uses_scope", Value::Bool(false)),
        ("_args", da_default),
        ("_duplicatable", Value::Bool(false)),
        ("_clipping", Value::Bool(false)),
        ("_tooltip", Value::None),
        ("_box_skip", Value::Bool(false)),
        ("_offer_size", Value::None),
        ("_draggable", Value::Bool(false)),
        ("_store_transform_event", Value::Bool(false)),
        ("delay", Value::None),
        ("id", Value::None),
        ("default", Value::Bool(false)),
        ("focus_name", Value::None),
    ])?;
    crysirror_python::modules::set(vm, m, "Displayable", Value::Obj(d.clone()));
    crysirror_python::modules::set(vm, m, "DisplayableArguments", Value::Obj(da.clone()));
    let place_f = vm.types.new_builtin("place", f_place);
    crysirror_python::modules::set(vm, m, "place", place_f);
    let cr = vm.types.new_builtin("compute_raw", f_compute_raw);
    crysirror_python::modules::set(vm, m, "compute_raw", cr);
    vm.ext_set(Classes { displayable: d, displayable_arguments: da });
    Ok(())
}

pub fn displayable_class(vm: &mut Vm) -> ObjRef {
    if vm.ext_get::<Classes>().is_none() {
        let _ = py::import(vm, "renpy.display.displayable");
    }
    vm.ext_get::<Classes>().expect("display classes").displayable.clone()
}

pub fn is_displayable(vm: &mut Vm, v: &Value) -> bool {
    let c = displayable_class(vm);
    ops::isinstance(vm, v, &c)
}
