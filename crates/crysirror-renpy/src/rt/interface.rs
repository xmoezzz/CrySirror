// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.core`, after Ren'Py 8.3 `core.py` (MIT): the host clock,
//! `absolute`, the `Interface` with its interaction loop, and the
//! CrySirror draw backend installed as `renpy.display.draw`.
//!
//! The interaction loop is upstream's: it runs on the script thread and
//! blocks in `pygame.event.wait/poll`, which end the current host tick
//! (`super::link`). A drawn frame becomes retained layer operations
//! (`super::backend`) handed to the owner. Pre-8.0 saves name
//! `renpy.display.core.Displayable`, `SceneLists` and `SceneListEntry`;
//! those names are re-exported here.
use super::{
    events, link,
    py::{self, *},
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.display.core";

pub const PERIODIC_INTERVAL: i64 = 50;

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, n, v)
}

fn call0(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn call(vm: &mut Vm, module: &str, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, Vec::new())
}

fn store(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "store", name)
}

fn store_set(vm: &mut Vm, name: &str, v: Value) -> PyResult<()> {
    let s = py::import(vm, "store")?;
    py::set(vm, &s, name, v)
}

fn context(vm: &mut Vm) -> PyResult<Value> {
    call0(vm, "renpy.game", "context")
}

fn scene_lists(vm: &mut Vm) -> PyResult<Value> {
    let c = context(vm)?;
    py::get(vm, &c, "scene_lists")
}

fn isinst(vm: &mut Vm, v: &Value, module: &str, cls: &str) -> PyResult<bool> {
    let c = py::import_attr(vm, module, cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

/// Whether `e` is an instance of the exception class named `name`.
fn exc_is(e: &PyErr, name: &str) -> bool {
    let PyErr::Exc(o) = e else { return false };
    ops::mro(&o.class()).iter().any(
        |c| matches!(&c.payload, crysirror_python::Payload::Type(t) if &**t.name.borrow() == name),
    )
}

fn truthy_of(vm: &mut Vm, f: impl FnOnce(&mut Vm) -> PyResult<Value>) -> PyResult<bool> {
    let v = f(vm)?;
    py::truthy(vm, &v)
}

fn config_items(vm: &mut Vm, name: &str) -> PyResult<Vec<Value>> {
    let v = py::config(vm, name)?;
    if py::is_none(&v) {
        return Ok(Vec::new());
    }
    py::items(vm, &v)
}

fn call_all(vm: &mut Vm, name: &str) -> PyResult<()> {
    for f in config_items(vm, name)? {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(())
}

// Time -------------------------------------------------------------------------------------------------

/// `get_time()`: the host clock of the current tick, in seconds.
pub fn get_time(vm: &Vm) -> f64 {
    link::now(vm)
}

fn f_get_time(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Float(get_time(vm)))
}

fn f_init_time(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let t = get_time(vm);
    mset(vm, "time_base", Value::Float(t))?;
    mset(vm, "time_mult", Value::Float(1.))?;
    Ok(Value::None)
}

fn f_get_size(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = super::host(vm).borrow().canvas;
    let w = c[0].max(256) as i64;
    let h = c[1].max(256) as i64;
    Ok(vm.types.new_tuple(vec![Value::Int(w), Value::Int(h)]))
}

fn f_displayable_by_tag(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sl = scene_lists(vm)?;
    vm.call_method(&sl, "get_displayable_by_tag", args.to_vec())
}

fn f_get_safe_mode(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn end_interaction_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = args.get(1).cloned().unwrap_or(Value::None);
    py::set(vm, &args[0], "value", v.clone())?;
    py::set(vm, &args[0], "args", vm.types.new_tuple(vec![v]))?;
    Ok(Value::None)
}

// absolute ---------------------------------------------------------------------------------------------

fn float_type(vm: &Vm) -> Value {
    Value::Obj(vm.types.float.clone())
}

fn new_absolute(vm: &mut Vm, v: f64) -> PyResult<Value> {
    let c = mget(vm, "absolute")?;
    vm.call(&c, vec![Value::Float(v)], Vec::new())
}

/// `absolute_wrap(float.<name>)`: float results come back as absolute.
fn abs_wrapped(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = args[0].as_str().unwrap_or("").to_owned();
    let ft = float_type(vm);
    let f = py::get(vm, &ft, &name)?;
    let rv = vm.call(&f, args[1..].to_vec(), Vec::new())?;
    if let Some(o) = rv.obj() {
        if matches!(o.payload, crysirror_python::Payload::Float(_))
            && Rc::ptr_eq(&o.class(), &vm.types.float)
        {
            let x = py::f64_of(vm, &rv)?;
            return new_absolute(vm, x);
        }
    }
    if let Value::Float(x) = rv {
        return new_absolute(vm, x);
    }
    Ok(rv)
}

fn abs_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &args[0])?;
    let ft = float_type(vm);
    let r = py::get(vm, &ft, "__repr__")?;
    let f = vm.call(&r, vec![Value::Float(x)], Vec::new())?;
    let f = py::string(vm, &f)?;
    Ok(py::s(vm, &format!("absolute({f})")))
}

fn abs_divmod(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    let q = ops::binary_op(
        vm,
        a,
        b,
        crysirror_python::ast::BinOp::FloorDiv,
        false,
        false,
    )?;
    let r = ops::binary_op(vm, a, b, crysirror_python::ast::BinOp::Mod, false, false)?;
    Ok(vm.types.new_tuple(vec![q, r]))
}

fn abs_rdivmod(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    abs_divmod(vm, &[args[1].clone(), args[0].clone()], &[])
}

pub fn compute_raw(vm: &mut Vm, value: &Value, room: f64) -> PyResult<Value> {
    if isinst(vm, value, "renpy.atl", "position").unwrap_or(false) {
        let r = py::get(vm, value, "relative")?;
        let a = py::get(vm, value, "absolute")?;
        let r = py::f64_of(vm, &r)?;
        let a = py::f64_of(vm, &a)?;
        return Ok(Value::Float(r * room + a));
    }
    let is_abs = isinst(vm, value, MODULE, "absolute")?;
    match value {
        Value::Int(_) | Value::Bool(_) => return Ok(value.clone()),
        _ if is_abs => return Ok(value.clone()),
        Value::Float(f) => return Ok(Value::Float(f * room)),
        _ => {}
    }
    if value
        .obj()
        .is_some_and(|o| matches!(o.payload, crysirror_python::Payload::Int(_)))
    {
        return Ok(value.clone());
    }
    if value
        .obj()
        .is_some_and(|o| matches!(o.payload, crysirror_python::Payload::Float(_)))
    {
        let f = py::f64_of(vm, value)?;
        return Ok(Value::Float(f * room));
    }
    let r = ops::repr(vm, value)?;
    let t = ops::type_name(vm, value);
    Err(vm.type_error(format!(
        "Value {r} of type {t} not recognized as a position."
    )))
}

fn abs_compute_raw(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let room = py::f64_of(vm, &args[1])?;
    compute_raw(vm, &args[0], room)
}

fn abs_compute(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let room = py::f64_of(vm, &args[1])?;
    let v = compute_raw(vm, &args[0], room)?;
    let f = py::f64_of(vm, &v)?;
    new_absolute(vm, f)
}

const ABS_WRAPPED: &[&str] = &[
    "__abs__",
    "__add__",
    "__ceil__",
    "__floordiv__",
    "__mod__",
    "__mul__",
    "__neg__",
    "__pos__",
    "__pow__",
    "__radd__",
    "__rfloordiv__",
    "__rmod__",
    "__rmul__",
    "__round__",
    "__rpow__",
    "__rsub__",
    "__rtruediv__",
    "__sub__",
    "__truediv__",
    "__div__",
    "__rdiv__",
    "conjugate",
];

fn make_absolute(vm: &mut Vm) -> PyResult<Value> {
    let ft = float_type(vm);
    let cr = vm.types.new_builtin("compute_raw", abs_compute_raw);
    let cr = py::staticmethod(vm, cr);
    let cp = vm.types.new_builtin("compute", abs_compute);
    let cp = py::staticmethod(vm, cp);
    let slots = vm.types.new_tuple(Vec::new());
    let mut attrs: Vec<(&str, Value)> =
        vec![("__slots__", slots), ("compute_raw", cr), ("compute", cp)];
    let mut wrapped = Vec::new();
    for n in ABS_WRAPPED {
        if py::get_opt(vm, &ft, n)?.is_some() {
            let name = vm.types.new_str(n);
            wrapped.push((*n, py::closure(vm, "absolute_wrap", abs_wrapped, name)));
        }
    }
    for (n, v) in &wrapped {
        attrs.push((n, v.clone()));
    }
    let c = py::class(
        vm,
        MODULE,
        "absolute",
        &[ft],
        &[
            ("__repr__", abs_repr),
            ("__divmod__", abs_divmod),
            ("__rdivmod__", abs_rdivmod),
        ],
        &attrs,
    )?;
    Ok(Value::Obj(c))
}

// MouseMove --------------------------------------------------------------------------------------------

fn draw(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display", "draw")
}

fn mm_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let x = arg_or(args, kw, 1, "x", Value::Int(0));
    let y = arg_or(args, kw, 2, "y", Value::Int(0));
    let duration = arg_or(args, kw, 3, "duration", Value::None);
    py::set(vm, this, "start", Value::Float(get_time(vm)))?;
    if !py::is_none(&duration) {
        py::set(vm, this, "duration", duration)?;
        let d = draw(vm)?;
        let p = vm.call_method(&d, "get_mouse_pos", Vec::new())?;
        let p = py::items(vm, &p)?;
        py::set(vm, this, "start_x", p[0].clone())?;
        py::set(vm, this, "start_y", p[1].clone())?;
    } else {
        py::set(vm, this, "duration", Value::Float(4. / 60.))?;
        py::set(vm, this, "start_x", x.clone())?;
        py::set(vm, this, "start_y", y.clone())?;
    }
    py::set(vm, this, "end_x", x)?;
    py::set(vm, this, "end_y", y)?;
    Ok(Value::None)
}

fn mm_perform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let g = |vm: &mut Vm, n: &str| -> PyResult<f64> {
        let v = py::get(vm, this, n)?;
        py::f64_of(vm, &v)
    };
    let elapsed = get_time(vm) - g(vm, "start")?;
    let duration = g(vm, "duration")?;
    let (ex, ey) = (g(vm, "end_x")?, g(vm, "end_y")?);
    let d = draw(vm)?;
    if elapsed >= duration {
        vm.call_method(
            &d,
            "set_mouse_pos",
            vec![Value::Int(ex as i64), Value::Int(ey as i64)],
        )?;
        return Ok(Value::Bool(false));
    }
    let done = elapsed / duration;
    let (sx, sy) = (g(vm, "start_x")?, g(vm, "start_y")?);
    let x = (sx + done * (ex - sx)) as i64;
    let y = (sy + done * (ey - sy)) as i64;
    vm.call_method(&d, "set_mouse_pos", vec![Value::Int(x), Value::Int(y)])?;
    Ok(Value::Bool(true))
}

// The CrySirror draw backend ---------------------------------------------------------------------------

/// Retained-layer ids of the previous frame; a new frame replaces them.
#[derive(Default)]
struct DrawState {
    next_id: u32,
}

fn draw_state(vm: &Vm) -> Rc<std::cell::RefCell<DrawState>> {
    if let Some(s) = vm.ext_get::<Rc<std::cell::RefCell<DrawState>>>() {
        return s.clone();
    }
    let s = Rc::new(std::cell::RefCell::new(DrawState { next_id: 1 }));
    vm.ext_set(s.clone());
    s
}

fn d_true(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn d_false(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn d_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn d_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let info = vm.types.new_dict();
    // The CrySirror renderer draws Ren'Py's model output (matrices, blend
    // modes, meshes); custom shader parts are rejected when drawn. Releases
    // with GL2 (7.4+) see the GL2 model renderer, earlier ones the GL
    // renderer they were written for.
    let gl2 = super::state::rt(vm).profile.at_least(7, 4);
    py::dict_set(
        vm,
        &info,
        "renderer",
        py::s(vm, if gl2 { "gl2" } else { "gl" }),
    )?;
    py::dict_set(vm, &info, "resizable", Value::Bool(false))?;
    py::dict_set(vm, &info, "additive", Value::Bool(true))?;
    py::dict_set(vm, &info, "models", Value::Bool(gl2))?;
    py::dict_set(vm, &info, "crysirror", Value::Bool(true))?;
    if !gl2 {
        // gldraw.pyx with shader support and framebuffer objects.
        py::dict_set(vm, &info, "environ", py::s(vm, "shader"))?;
        py::dict_set(vm, &info, "rtt", py::s(vm, "fbo"))?;
    }
    let c = super::host(vm).borrow().canvas;
    let mws = vm.types.new_tuple(vec![Value::Int(c[0].max(256) as i64), Value::Int(c[1].max(256) as i64)]);
    py::dict_set(vm, &info, "max_window_size", mws)?;
    py::set(vm, this, "info", info)?;
    let sw = py::config_f64(vm, "screen_width")?;
    let sh = py::config_f64(vm, "screen_height")?;
    let size = vm
        .types
        .new_tuple(vec![Value::Int(sw as i64), Value::Int(sh as i64)]);
    py::set(vm, this, "virtual_size", size.clone())?;
    py::set(vm, this, "physical_size", size.clone())?;
    py::set(vm, this, "drawable_size", size)?;
    py::set(vm, this, "full_redraw", Value::Bool(false))?;
    Ok(Value::Bool(true))
}

/// `should_redraw`: at most one frame per host tick, except the first
/// pass of an interaction, which upstream always draws before events.
fn d_should_redraw(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let needs = py::truthy(vm, &args[1])?;
    let first = py::truthy(vm, &args[2])?;
    Ok(Value::Bool(needs && (first || !link::drew_this_tick(vm))))
}

fn d_draw_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tree = &args[1];
    let Some(root) = super::render::render_data(tree) else {
        return Err(vm.type_error("draw_screen expects a Render"));
    };
    let assets = super::host(vm).borrow().assets.clone();
    let lookup = |n: &str| assets.as_ref().and_then(|f| f(n));
    let first = draw_state(vm).borrow().next_id;
    let fit = events::input_scale(vm);
    let (ops, _, errors) = super::backend::build(&root, first, &lookup, fit);
    if let Some(r) = super::backend::text_input_rect(&root.borrow()) {
        let rect = vm.types.new_tuple(r.iter().map(|v| Value::Int(v.round() as i64)).collect());
        if let Ok(i) = py::import_attr(vm, "renpy.display", "interface") {
            py::set(vm, &i, "text_rect", rect)?;
        }
    }
    for e in errors {
        link::diagnostic(vm, e);
    }
    link::emit_frame(vm, ops);
    Ok(Value::None)
}

fn d_mouse_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ev = &args[1];
    let pos = py::get_or_none(vm, ev, "pos")?;
    if !py::is_none(&pos) {
        let p = py::items(vm, &pos)?;
        if p.len() == 2 {
            return Ok(vm.types.new_tuple(p));
        }
    }
    d_get_mouse_pos(vm, &args[..1], &[])
}

fn d_get_mouse_pos(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let (x, y) = events::input(vm).borrow().pos;
    let (vx, vy) = events::input_scale(vm).to_virtual(x as f64, y as f64);
    Ok(vm
        .types
        .new_tuple(vec![Value::Int(vx as i64), Value::Int(vy as i64)]))
}

fn d_set_mouse_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = py::int(vm, &args[1])?;
    let y = py::int(vm, &args[2])?;
    let (cx, cy) = events::input_scale(vm).to_canvas(x as f64, y as f64);
    events::input(vm).borrow_mut().pos = (cx as i32, cy as i32);
    Ok(Value::None)
}

fn d_translate_point(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_tuple(vec![args[1].clone(), args[2].clone()]))
}

fn d_get_texture_size(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]))
}

fn d_get_physical_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "physical_size")
}

/// Screenshots need the composed host frame; the host has no synchronous
/// readback here, so screenshot surfaces are unavailable.
fn d_screenshot(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "Screenshots of the composed frame are not available from the CrySirror host.",
    ))
}

fn d_is_pixel_opaque(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let Some(r) = super::render::render_data(&args[1]) else {
        return Ok(Value::Bool(false));
    };
    let x = py::f64_of(vm, &args[2])?;
    let y = py::f64_of(vm, &args[3])?;
    Ok(Value::Bool(super::backend::pixel_opaque(vm, &r, x, y)?))
}

// Interface ------------------------------------------------------------------------------------------

fn g(vm: &mut Vm, this: &Value, n: &str) -> PyResult<Value> {
    py::get_or_none(vm, this, n)
}

fn gb(vm: &mut Vm, this: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, this, n)?;
    py::truthy(vm, &v)
}

fn st(vm: &mut Vm, this: &Value, n: &str, v: Value) -> PyResult<()> {
    py::set(vm, this, n, v)
}

fn dict(vm: &Vm) -> Value {
    vm.types.new_dict()
}

fn i_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for n in [
        "screenshot",
        "screenshot_surface",
        "pushed_event",
        "timeout_time",
        "last_event",
        "current_context",
        "roll_forward",
        "interact_time",
        "old_root_transform",
        "window_caption",
        "mouse_move",
        "text_editing",
        "text_rect",
        "old_text_rect",
        "cursor_cache",
        "old_mouse",
        "prediction_coroutine",
        "surftree",
        "bgscreenshot_surface",
    ] {
        st(vm, this, n, Value::None)?;
    }
    for n in [
        "old_scene",
        "transition",
        "layer_properties",
        "ongoing_transition",
        "instantiated_transition",
        "transition_time",
        "transition_from",
        "transition_delay",
    ] {
        st(vm, this, n, dict(vm))?;
    }
    for n in [
        "suppress_transition",
        "quick_quit",
        "force_redraw",
        "fullscreen",
        "in_quit_event",
        "shown_window",
        "ignore_touch",
        "clear_screenshot",
        "audio_paused",
        "display_reset",
        "profile_once",
        "safe_mode",
        "bgscreenshot_needed",
        "started",
        "fullscreen_video",
        "force_prediction",
        "trans_pause",
        "suppress_overlay",
        "suppress_underlay",
        "did_autosave",
        "did_persistent",
        "minimized",
    ] {
        st(vm, this, n, Value::Bool(false))?;
    }
    for n in [
        "mouse_focused",
        "keyboard_focused",
        "show_mouse",
        "restart_interaction",
        "start_interact",
        "first_frame",
    ] {
        st(vm, this, n, Value::Bool(true))?;
    }
    for n in [
        "ticks",
        "quit_time",
        "event_time",
        "input_event_time",
        "interaction_counter",
        "mod",
    ] {
        st(vm, this, n, Value::Int(0))?;
    }
    st(vm, this, "mouse", py::s(vm, "default"))?;
    st(vm, this, "preloads", vm.types.new_list(Vec::new()))?;
    st(
        vm,
        this,
        "transition_info_stack",
        vm.types.new_list(Vec::new()),
    )?;
    st(vm, this, "frame_times", vm.types.new_list(Vec::new()))?;
    st(vm, this, "invoke_queue", vm.types.new_list(Vec::new()))?;
    let now = get_time(vm);
    st(vm, this, "init_time", Value::Float(now))?;
    st(vm, this, "frame_time", Value::Float(0.))?;
    let te = events::new_event(vm, events::TIMEEVENT, vec![("modal", Value::Bool(false))])?;
    st(vm, this, "time_event", te)?;
    let re = events::new_event(vm, events::REDRAW, Vec::new())?;
    st(vm, this, "redraw_event", re)?;
    call0(vm, "renpy.display.scenelists", "init_layers")?;
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let n = vm.call(&null, Vec::new(), Vec::new())?;
    mset(vm, "null", n)?;
    let lp = g(vm, this, "layer_properties")?;
    let clip = py::config(vm, "layer_clipping")?;
    let layers = py::import_attr(vm, "renpy.display.scenelists", "layers")?;
    for layer in ops::collect(vm, &layers)? {
        let d = dict(vm);
        if ops::contains(vm, &clip, &layer)? {
            let r = ops::getitem(vm, &clip, layer.clone())?;
            let r = py::items(vm, &r)?;
            for (k, v) in [
                ("xpos", r[0].clone()),
                ("xanchor", Value::Int(0)),
                ("ypos", r[1].clone()),
                ("yanchor", Value::Int(0)),
                ("xmaximum", r[2].clone()),
                ("ymaximum", r[3].clone()),
                ("xminimum", r[2].clone()),
                ("yminimum", r[3].clone()),
                ("clipping", Value::Bool(true)),
            ] {
                py::dict_set(vm, &d, k, v)?;
            }
        }
        ops::setitem(vm, &lp, layer, d)?;
    }
    f_init_time(vm, &[], &[])?;
    st(vm, this, "mouse_event_time", Value::Float(now))?;
    let game = py::import(vm, "renpy.game")?;
    py::set(vm, &game, "interface", this.clone())?;
    let disp = py::import(vm, "renpy.display")?;
    py::set(vm, &disp, "interface", this.clone())?;
    let touch = call(vm, "renpy.exports", "variant", vec![py::s(vm, "touch")])
        .unwrap_or(Value::Bool(false));
    st(vm, this, "touch", touch.clone())?;
    let tk = py::config(vm, "touch_keyboard").unwrap_or(Value::Bool(false));
    st(vm, this, "touch_keyboard", tk)?;
    let pc = py::config(vm, "periodic_callback")?;
    if !py::is_none(&pc) {
        let pcs = py::config(vm, "periodic_callbacks")?;
        ops::list_append(vm, &pcs, pc)?;
    }
    st(vm, this, "dpi_scale", Value::Float(1.))?;
    st(vm, this, "maximum_framerate_time", Value::Float(0.))?;
    st(vm, this, "frame_duration", Value::Float(1. / 60.))?;
    Ok(Value::None)
}

fn i_start(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if gb(vm, this, "started")? {
        return Ok(Value::None);
    }
    call0(vm, "renpy.audio.audio", "init")?;
    vm.call_method(this, "post_init", Vec::new())?;
    call0(vm, "renpy.display.render", "render_ready")?;
    st(vm, this, "started", Value::Bool(true))?;
    vm.call_method(this, "set_mode", Vec::new())?;
    call0(vm, "renpy.text.font", "load_fonts")?;
    call(
        vm,
        "pygame_sdl2.time",
        "set_timer",
        vec![Value::Int(events::PERIODIC), Value::Int(PERIODIC_INTERVAL)],
    )?;
    // Hardware cursors (`config.mouse`) are drawn by the host pointer; the
    // cursor cache stays empty and `mouse_displayable` still renders.
    call_all(vm, "display_start_callbacks")?;
    Ok(Value::None)
}

fn i_post_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::method_kw(
        vm,
        this,
        "set_window_caption",
        Vec::new(),
        vec![("force".into(), Value::Bool(true))],
    )?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    if gb(vm, &prefs, "fullscreen")? {
        py::set(vm, &prefs, "maximized", Value::Bool(false))?;
    }
    Ok(Value::None)
}

fn i_set_window_caption(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let force = match arg(args, kw, 1, "force") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let mut caption = py::config(vm, "window_title")?;
    if py::is_none(&caption) {
        caption = py::s(vm, "A Ren'Py Game");
    }
    let mut caption = py::string(vm, &caption)?;
    let title = store(vm, "_window_subtitle").unwrap_or(Value::None);
    if py::truthy(vm, &title)? {
        caption.push_str(&py::string(vm, &title)?);
    }
    let sub = py::import_attr(vm, "renpy.exports", "substitute").ok();
    if let Some(sub) = sub {
        let c = py::s(vm, &caption);
        let r = vm.call(&sub, vec![c], Vec::new())?;
        caption = py::string(vm, &r)?;
    }
    let old = g(vm, this, "window_caption")?;
    let new = py::s(vm, &caption);
    if !force && ops::eq(vm, &old, &new)? {
        return Ok(Value::None);
    }
    st(vm, this, "window_caption", new)?;
    super::set_window_title(vm, caption.clone());
    link::set_title(vm, caption);
    Ok(Value::None)
}

fn i_set_mode(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = mget(vm, "CrySirrorDraw")?;
    let d = vm.call(&c, Vec::new(), Vec::new())?;
    let sw = py::config(vm, "screen_width")?;
    let sh = py::config(vm, "screen_height")?;
    let size = vm.types.new_tuple(vec![sw, sh]);
    vm.call_method(&d, "init", vec![size])?;
    let disp = py::import(vm, "renpy.display")?;
    py::set(vm, &disp, "draw", d)?;
    let render = py::import(vm, "renpy.display.render")?;
    py::set(vm, &render, "models", Value::Bool(false))?;
    Ok(Value::None)
}

fn i_kill_textures(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    super::render::free_memory(vm);
    if let Ok(f) = py::import_attr(vm, "renpy.text.text", "layout_cache_clear") {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn i_draw_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (root, fsv, do_draw) = (args[1].clone(), args[2].clone(), py::truthy(vm, &args[3])?);
    let render = py::import(vm, "renpy.display.render")?;
    py::set(vm, &render, "per_frame", Value::Bool(true))?;
    let r = call0(vm, "renpy.display.screen", "per_frame");
    py::set(vm, &render, "per_frame", Value::Bool(false))?;
    r?;
    let sw = py::config(vm, "screen_width")?;
    let sh = py::config(vm, "screen_height")?;
    let rs = call(
        vm,
        "renpy.display.render",
        "render_screen",
        vec![root, sw, sh],
    )?;
    if do_draw {
        let d = draw(vm)?;
        vm.call_method(&d, "draw_screen", vec![rs.clone()])?;
    }
    super::focus::take_focuses(vm)?;
    st(vm, this, "surftree", rs)?;
    st(vm, this, "fullscreen_video", fsv)?;
    st(vm, this, "first_frame", Value::Bool(false))?;
    Ok(Value::None)
}

fn i_take_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    st(vm, this, "clear_screenshot", Value::Bool(false))?;
    Ok(Value::None)
}

fn i_get_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    g(vm, &args[0], "screenshot")
}

fn i_lose_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "screenshot", Value::None)?;
    st(vm, &args[0], "screenshot_surface", Value::None)?;
    Ok(Value::None)
}

fn i_save_screenshot(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn i_show_window(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !truthy_of(vm, |vm| store(vm, "_window"))? {
        return Ok(Value::None);
    }
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    if !gb(vm, &prefs, "show_empty_window")? {
        return Ok(Value::None);
    }
    let sl = scene_lists(vm)?;
    if gb(vm, &sl, "shown_window")? {
        return Ok(Value::None);
    }
    let ew = py::config(vm, "empty_window")?;
    if !py::truthy(vm, &ew)? {
        return Ok(Value::None);
    }
    let old_history = store(vm, "_history")?;
    store_set(vm, "_history", Value::Bool(false))?;
    let ctx = context(vm)?;
    let osa = g(vm, &ctx, "say_attributes")?;
    let ota = g(vm, &ctx, "temporary_attributes")?;
    st(vm, &ctx, "say_attributes", Value::None)?;
    st(vm, &ctx, "temporary_attributes", Value::None)?;
    let r = vm.call(&ew, Vec::new(), Vec::new());
    store_set(vm, "_history", old_history)?;
    let ctx = context(vm)?;
    st(vm, &ctx, "say_attributes", osa)?;
    st(vm, &ctx, "temporary_attributes", ota)?;
    r?;
    Ok(Value::None)
}

fn i_do_with(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut trans = arg_or(args, kw, 1, "trans", Value::None);
    let paired = arg_or(args, kw, 2, "paired", Value::None);
    let clear = arg_or(args, kw, 3, "clear", Value::Bool(false));
    let wc = py::config(vm, "with_callback")?;
    if !py::is_none(&wc) {
        trans = vm.call(&wc, vec![trans, paired], Vec::new())?;
    }
    if !py::truthy(vm, &trans)? || gb(vm, this, "suppress_transition")? {
        vm.call_method(this, "with_none", Vec::new())?;
        return Ok(Value::Bool(false));
    }
    vm.call_method(this, "set_transition", vec![trans])?;
    let odw = py::config_bool(vm, "overlay_during_with")?;
    py::method_kw(
        vm,
        this,
        "interact",
        Vec::new(),
        vec![
            ("trans_pause".into(), Value::Bool(true)),
            ("suppress_overlay".into(), Value::Bool(!odw)),
            ("mouse".into(), py::s(vm, "with")),
            ("clear".into(), clear),
        ],
    )
}

fn i_with_none(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let overlay = match arg(args, kw, 1, "overlay") {
        Some(v) => py::truthy(vm, v)?,
        None => true,
    };
    vm.call_method(this, "show_window", Vec::new())?;
    if overlay {
        vm.call_method(this, "compute_overlay", Vec::new())?;
    }
    let sl = scene_lists(vm)?;
    let scene = vm.call_method(this, "compute_scene", vec![sl.clone()])?;
    let old = g(vm, this, "old_scene")?;
    let trans = g(vm, this, "transition")?;
    let has_trans = py::truthy(vm, &trans)?;
    for (layer, d) in ops::mapping_items(vm, &scene)? {
        if py::is_none(&layer) {
            if !has_trans {
                ops::setitem(vm, &old, layer, d)?;
            }
        } else if !ops::contains(vm, &trans, &layer)? {
            ops::setitem(vm, &old, layer, d)?;
        }
    }
    for i in config_items(vm, "overlay_layers")? {
        vm.call_method(&sl, "clear", vec![i])?;
    }
    vm.call_method(&sl, "replace_transient", Vec::new())?;
    st(vm, &sl, "shown_window", Value::Bool(false))?;
    side_image_reset(vm)?;
    Ok(Value::None)
}

fn side_image_reset(vm: &mut Vm) -> PyResult<()> {
    let reset = store(vm, "_side_image_attributes_reset").unwrap_or(Value::Bool(false));
    if py::truthy(vm, &reset)? {
        store_set(vm, "_side_image_attributes", Value::None)?;
        store_set(vm, "_side_image_attributes_reset", Value::Bool(false))?;
    }
    Ok(())
}

fn pop_layer(vm: &mut Vm, this: &Value, layer: &Value) -> PyResult<()> {
    for n in [
        "ongoing_transition",
        "instantiated_transition",
        "transition_time",
        "transition_from",
    ] {
        let d = g(vm, this, n)?;
        vm.call_method(&d, "pop", vec![layer.clone(), Value::None])?;
    }
    Ok(())
}

fn i_end_transitions(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ot = g(vm, this, "ongoing_transition")?;
    let ft = g(vm, this, "frame_time")?;
    let ft = py::f64_of(vm, &ft)?;
    for l in ops::collect(vm, &ot)? {
        if py::is_none(&l) {
            pop_layer(vm, this, &l)?;
            continue;
        }
        let tt = g(vm, this, "transition_time")?;
        let start = vm.call_method(&tt, "get", vec![l.clone(), Value::Float(ft)])?;
        let start = if py::truthy(vm, &start)? {
            py::f64_of(vm, &start)?
        } else {
            0.
        };
        let td = g(vm, this, "transition_delay")?;
        let delay = vm.call_method(&td, "get", vec![l.clone(), Value::None])?;
        let delay = if py::truthy(vm, &delay)? {
            py::f64_of(vm, &delay)?
        } else {
            0.
        };
        if ft - start >= delay {
            pop_layer(vm, this, &l)?;
        }
    }
    Ok(Value::None)
}

fn i_get_ongoing_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let ot = g(vm, this, "ongoing_transition")?;
    let rv = vm.call_method(&ot, "get", vec![layer.clone(), Value::None])?;
    let it = g(vm, this, "instantiated_transition")?;
    let t = vm.call_method(&it, "get", vec![layer.clone(), Value::None])?;
    if py::is_none(&t) {
        return Ok(Value::None);
    }
    let ft = g(vm, this, "frame_time")?;
    let ft = py::f64_of(vm, &ft)?;
    let tt = g(vm, this, "transition_time")?;
    let start = vm.call_method(&tt, "get", vec![layer, Value::Float(ft)])?;
    let start = if py::truthy(vm, &start)? {
        py::f64_of(vm, &start)?
    } else {
        ft
    };
    let delay = py::get_opt(vm, &t, "delay")?.unwrap_or(Value::Int(0));
    let delay = py::f64_of(vm, &delay)?;
    if ft - start < delay {
        return Ok(rv);
    }
    Ok(Value::None)
}

fn i_set_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let transition = arg_or(args, kw, 1, "transition", Value::None);
    let layer = arg_or(args, kw, 2, "layer", Value::None);
    let force = match arg(args, kw, 3, "force") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    if gb(vm, this, "suppress_transition")? && !force {
        return Ok(Value::None);
    }
    let t = g(vm, this, "transition")?;
    if py::is_none(&transition) {
        vm.call_method(&t, "pop", vec![layer, Value::None])?;
    } else {
        ops::setitem(vm, &t, layer, transition)?;
    }
    Ok(Value::None)
}

fn ev_type(vm: &mut Vm, ev: &Value) -> PyResult<i64> {
    match py::get_or_none(vm, ev, "type")? {
        Value::Int(t) => Ok(t),
        _ => Ok(-1),
    }
}

/// `event_peek`: the next event without consuming it, or None. Peeking
/// never ends the host tick.
fn i_event_peek(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pe = g(vm, this, "pushed_event")?;
    if py::truthy(vm, &pe)? {
        return Ok(pe);
    }
    events::due_timers(vm);
    let q = events::input(vm);
    let ev = {
        let mut q = q.borrow_mut();
        if q.queue.is_empty() {
            None
        } else {
            Some(q.queue.remove(0))
        }
    };
    match ev {
        Some(e) => {
            st(vm, this, "pushed_event", e.clone())?;
            Ok(e)
        }
        None => Ok(Value::None),
    }
}

fn i_event_poll(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pe = g(vm, this, "pushed_event")?;
    let rv = if py::truthy(vm, &pe)? {
        st(vm, this, "pushed_event", Value::None)?;
        pe
    } else {
        call0(vm, "pygame_sdl2.event", "poll")?
    };
    st(vm, this, "last_event", rv.clone())?;
    Ok(rv)
}

fn i_event_wait(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pe = g(vm, this, "pushed_event")?;
    let rv = if py::truthy(vm, &pe)? {
        st(vm, this, "pushed_event", Value::None)?;
        pe
    } else {
        call0(vm, "pygame_sdl2.event", "wait")?
    };
    st(vm, this, "last_event", rv.clone())?;
    Ok(rv)
}

fn i_compute_overlay(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if truthy_of(vm, |vm| store(vm, "suppress_overlay"))? {
        return Ok(Value::None);
    }
    call(vm, "renpy.ui", "layer", vec![py::s(vm, "overlay")])?;
    call_all(vm, "overlay_functions")?;
    let sl = scene_lists(vm)?;
    if gb(vm, &sl, "shown_window")? {
        call_all(vm, "window_overlay_functions")?;
    }
    call0(vm, "renpy.ui", "close")?;
    Ok(Value::None)
}

fn i_compute_scene(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sl = args[1].clone();
    let rv = dict(vm);
    let lp = g(vm, this, "layer_properties")?;
    let layers = py::import_attr(vm, "renpy.display.scenelists", "layers")?;
    for layer in ops::collect(vm, &layers)? {
        let props = ops::getitem(vm, &lp, layer.clone())?;
        let d = vm.call_method(&sl, "make_layer", vec![layer.clone(), props])?;
        let t = vm.call_method(&sl, "transform_layer", vec![layer.clone(), d])?;
        ops::setitem(vm, &rv, layer, t)?;
    }
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let root = vm.call(&mb, Vec::new(), vec![("layout".into(), py::s(vm, "fixed"))])?;
    let rl = dict(vm);
    st(vm, &root, "layers", rl.clone())?;
    for layer in config_items(vm, "layers")? {
        let d = ops::getitem(vm, &rv, layer.clone())?;
        ops::setitem(vm, &rl, layer, d.clone())?;
        vm.call_method(&root, "add", vec![d])?;
    }
    ops::setitem(vm, &rv, Value::None, root)?;
    Ok(rv)
}

fn i_quit_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let quit = |vm: &mut Vm| -> PyResult<Value> {
        let f = py::import_attr(vm, "renpy.exports", "quit")?;
        vm.call(&f, Vec::new(), vec![("save".into(), Value::Bool(true))])
    };
    let now = get_time(vm);
    let qt = g(vm, this, "quit_time")?;
    if py::f64_of(vm, &qt)? > now - 0.75 || gb(vm, this, "in_quit_event")? {
        return quit(vm);
    }
    let qa = py::config(vm, "quit_action")?;
    if py::is_none(&qa) {
        return quit(vm);
    }
    st(vm, this, "quit_time", Value::Float(now))?;
    if let Ok(f) = py::import_attr(vm, "renpy.exports", "movie_stop") {
        vm.call(
            &f,
            Vec::new(),
            vec![("only_fullscreen".into(), Value::Bool(true))],
        )?;
    }
    store_set(vm, "mouse_visible", Value::Bool(true))?;
    st(vm, this, "in_quit_event", Value::Bool(true))?;
    let r = call(vm, "renpy.display.behavior", "run", vec![qa]);
    st(vm, this, "in_quit_event", Value::Bool(false))?;
    r?;
    Ok(Value::None)
}

fn i_set_mouse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "old_mouse", args[1].clone())?;
    Ok(Value::None)
}

fn i_hide_mouse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "mouse_event_time", Value::Int(0))?;
    Ok(Value::None)
}

fn i_is_mouse_visible(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mht = py::config(vm, "mouse_hide_time")?;
    let met = g(vm, this, "mouse_event_time")?;
    let met = py::f64_of(vm, &met)?;
    let mut visible = if !py::is_none(&mht) && met + py::f64_of(vm, &mht)? < get_time(vm) {
        false
    } else {
        let mv = store(vm, "mouse_visible")?;
        let less = py::import_attr(vm, "renpy.game", "less_mouse")?;
        py::truthy(vm, &mv)? && !py::truthy(vm, &less)?
    };
    visible = visible && gb(vm, this, "show_mouse")?;
    Ok(Value::Bool(visible))
}

fn i_get_mouse_name(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let interaction = match arg(args, kw, 2, "interaction") {
        Some(v) => py::truthy(vm, v)?,
        None => true,
    };
    let mut kind = call0(vm, "renpy.display.focus", "get_mouse")?;
    if interaction && py::is_none(&kind) {
        kind = g(vm, this, "mouse")?;
    }
    let mut kind = if py::is_none(&kind) {
        "default".to_owned()
    } else {
        py::string(vm, &kind)?
    };
    if events::input(vm).borrow().mouse_buttons[1] {
        kind = format!("pressed_{kind}");
    }
    if kind == "default" {
        let dm = store(vm, "default_mouse").unwrap_or(Value::None);
        if !py::is_none(&dm) {
            return Ok(dm);
        }
    }
    Ok(py::s(vm, &kind))
}

fn i_update_mouse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let vis = vm.call_method(this, "is_mouse_visible", Vec::new())?;
    vm.call_method(this, "set_mouse", vec![vis])?;
    Ok(Value::None)
}

fn i_set_mouse_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = mget(vm, "MouseMove")?;
    let mm = vm.call(&c, args[1..4].to_vec(), Vec::new())?;
    st(vm, this, "mouse_move", mm)?;
    st(vm, this, "force_redraw", Value::Bool(true))?;
    Ok(Value::None)
}

fn i_drawn_since(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ft = g(vm, &args[0], "frame_time")?;
    let ft = py::f64_of(vm, &ft)?;
    let ago = py::f64_of(vm, &args[1])?;
    Ok(Value::Bool(get_time(vm) - ft <= ago))
}

fn i_check_suspend(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    // The host owns application backgrounding and keeps audio/timers;
    // no APP_WILLENTERBACKGROUND reaches the script thread.
    Ok(Value::Bool(false))
}

fn i_enter_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for n in [
        "ongoing_transition",
        "instantiated_transition",
        "transition_from",
        "transition_time",
    ] {
        let d = g(vm, &args[0], n)?;
        vm.call_method(&d, "clear", Vec::new())?;
    }
    Ok(Value::None)
}

fn i_post_time_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let te = g(vm, &args[0], "time_event")?;
    events::input(vm).borrow_mut().queue.push(te);
    Ok(Value::None)
}

fn i_after_longpress(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "ignore_touch", Value::Bool(true))?;
    super::focus::mouse_handler(vm, &Value::None, -1., -1., false)?;
    Ok(Value::None)
}

fn i_text_event_in_queue(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ev = i_event_peek(vm, args, &[])?;
    if py::is_none(&ev) {
        return Ok(Value::Bool(false));
    }
    let t = ev_type(vm, &ev)?;
    Ok(Value::Bool(
        t == events::TEXTINPUT || t == events::TEXTEDITING,
    ))
}

fn i_update_text_rect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let tr = store(vm, "_text_rect").unwrap_or(Value::None);
    if !py::is_none(&tr) {
        st(vm, this, "text_rect", tr)?;
    }
    let tr = g(vm, this, "text_rect")?;
    if py::is_none(&tr) {
        link::set_text_rect(vm, None);
        events::input(vm).borrow_mut().text_input = false;
    } else {
        let r = py::items(vm, &tr)?;
        let mut a = [0f64; 4];
        for (i, v) in r.iter().take(4).enumerate() {
            a[i] = py::f64_of(vm, v)?;
        }
        // `untranslate_point`: virtual screen -> window.
        let fit = events::input_scale(vm);
        let (x0, y0) = fit.to_canvas(a[0], a[1]);
        let (x1, y1) = fit.to_canvas(a[0] + a[2], a[1] + a[3]);
        let rect = [x0, y0, x1 - x0, y1 - y0].map(|v| v.round() as i32);
        link::set_text_rect(vm, Some(rect));
        events::input(vm).borrow_mut().text_input = true;
    }
    st(vm, this, "old_text_rect", tr)?;
    Ok(Value::None)
}

fn i_maximum_framerate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = args.get(1).cloned().unwrap_or(Value::None);
    if py::is_none(&t) {
        st(vm, this, "maximum_framerate_time", Value::Float(0.))?;
    } else {
        let cur = g(vm, this, "maximum_framerate_time")?;
        let cur = py::f64_of(vm, &cur)?;
        let v = cur.max(get_time(vm) + py::f64_of(vm, &t)?);
        st(vm, this, "maximum_framerate_time", Value::Float(v))?;
    }
    Ok(Value::None)
}

fn i_timeout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let offset = py::f64_of(vm, &args[1])?;
    if offset < 0. {
        return Ok(Value::None);
    }
    let et = g(vm, this, "event_time")?;
    let et = py::f64_of(vm, &et)?;
    let tt = g(vm, this, "timeout_time")?;
    let v = if py::truthy(vm, &tt)? {
        (et + offset).min(py::f64_of(vm, &tt)?)
    } else {
        et + offset
    };
    st(vm, this, "timeout_time", Value::Float(v))?;
    Ok(Value::None)
}

fn kwb(vm: &mut Vm, kw: &[(Rc<str>, Value)], n: &str, d: bool) -> PyResult<bool> {
    match py::kw(kw, n) {
        Some(v) => py::truthy(vm, v),
        None => Ok(d),
    }
}

fn kwv(kw: &[(Rc<str>, Value)], n: &str, d: Value) -> Value {
    py::kw(kw, n).cloned().unwrap_or(d)
}

fn i_interact(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut take = |n: &str| {
        kw.iter()
            .position(|(k, _)| &**k == n)
            .map(|i| kw.remove(i).1)
    };
    let clear = take("clear").unwrap_or(Value::Bool(true));
    let suppress_window = take("suppress_window").unwrap_or(Value::Bool(false));
    let trans_pause = take("trans_pause").unwrap_or(Value::Bool(false));
    let pause = take("pause").unwrap_or(Value::None);
    let pause_modal = take("pause_modal").unwrap_or(Value::Bool(false));
    if !gb(vm, this, "started")? {
        vm.call_method(this, "start", Vec::new())?;
    }
    if gb(vm, this, "clear_screenshot")? {
        vm.call_method(this, "lose_screenshot", Vec::new())?;
    }
    st(vm, this, "clear_screenshot", Value::Bool(false))?;
    st(vm, this, "trans_pause", trans_pause.clone())?;
    let ctx = context(vm)?;
    if gb(vm, &ctx, "interacting")? {
        return Err(py::error(
            vm,
            "Cannot start an interaction in the middle of an interaction, without creating a new context.",
        ));
    }
    st(vm, &ctx, "interacting", Value::Bool(true))?;
    if !py::truthy(vm, &suppress_window)? {
        vm.call_method(this, "show_window", Vec::new())?;
    }
    let preloads = g(vm, this, "preloads")?;
    st(vm, this, "preloads", vm.types.new_list(Vec::new()))?;
    let r = (|| -> PyResult<Value> {
        st(vm, this, "start_interact", Value::Bool(true))?;
        call_all(vm, "start_interact_callbacks")?;
        st(vm, this, "interaction_counter", Value::Int(0))?;
        let pause_start = get_time(vm);
        loop {
            let c = g(vm, this, "interaction_counter")?;
            let c = py::int(vm, &c)? + 1;
            st(vm, this, "interaction_counter", Value::Int(c))?;
            if c == 100 && py::config_bool(vm, "developer")? {
                return Err(py::error(
                    vm,
                    "renpy.restart_interaction() was called 100 times without processing any input.",
                ));
            }
            let mut k: Vec<(Rc<str>, Value)> = kw.clone();
            k.push(("preloads".into(), preloads.clone()));
            k.push(("trans_pause".into(), trans_pause.clone()));
            k.push(("pause".into(), pause.clone()));
            k.push(("pause_start".into(), Value::Float(pause_start)));
            k.push(("pause_modal".into(), pause_modal.clone()));
            let (repeat, rv) = interact_core(vm, this, &k)?;
            st(vm, this, "start_interact", Value::Bool(false))?;
            if !repeat {
                return Ok(rv);
            }
        }
    })();
    // finally:
    let ctx = context(vm)?;
    st(vm, &ctx, "deferred_translate_identifier", Value::None)?;
    st(vm, this, "force_prediction", Value::Bool(false))?;
    st(vm, &ctx, "interacting", Value::Bool(false))?;
    if py::truthy(vm, &clear)? {
        let sl = scene_lists(vm)?;
        vm.call_method(&sl, "replace_transient", Vec::new())?;
    }
    vm.call_method(this, "end_transitions", Vec::new())?;
    st(vm, this, "restart_interaction", Value::Bool(true))?;
    let ctx = context(vm)?;
    vm.call_method(&ctx, "mark_seen", Vec::new())?;
    let sl = scene_lists(vm)?;
    st(vm, &sl, "shown_window", Value::Bool(false))?;
    let log = py::import_attr(vm, "renpy.game", "log")?;
    if !py::is_none(&log) {
        st(vm, &log, "did_interaction", Value::Bool(true))?;
    }
    side_image_reset(vm)?;
    r
}

fn i_idle_frame(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let can_block = py::truthy(vm, &args[1])?;
    let expensive = py::truthy(vm, &args[2])?;
    idle_frame(vm, this, can_block, expensive)?;
    Ok(Value::None)
}

fn idle_frame(vm: &mut Vm, this: &Value, can_block: bool, expensive: bool) -> PyResult<()> {
    let start = get_time(vm);
    let mut step = 1;
    loop {
        let pe = vm.call_method(this, "event_peek", Vec::new())?;
        if py::truthy(vm, &pe)? && !gb(vm, this, "force_prediction")? {
            break;
        }
        if !(can_block && expensive) && get_time(vm) > start + 0.0005 {
            break;
        }
        match step {
            1 | 2 | 4 => step += 1,
            3 => {
                let co = g(vm, this, "prediction_coroutine")?;
                if !py::truthy(vm, &co)? {
                    step += 1;
                    continue;
                }
                let r = vm.call_method(&co, "send", vec![Value::Bool(expensive)]);
                let result = match r {
                    Ok(v) => v,
                    Err(e) if exc_is(&e, "ValueError") || exc_is(&e, "StopIteration") => {
                        Value::None
                    }
                    Err(e) => return Err(e),
                };
                if py::is_none(&result) {
                    st(vm, this, "prediction_coroutine", Value::None)?;
                    step += 1;
                } else if matches!(result, Value::Bool(false)) && !expensive {
                    step += 1;
                }
            }
            5 => {
                if !gb(vm, this, "did_autosave")? {
                    call0(vm, "renpy.loadsave", "autosave")?;
                    st(vm, this, "did_autosave", Value::Bool(true))?;
                }
                step += 1;
            }
            6 => {
                if !gb(vm, this, "did_persistent")? {
                    call0(vm, "renpy.persistent", "check_update")?;
                    st(vm, this, "did_persistent", Value::Bool(true))?;
                }
                step += 1;
            }
            _ => {
                st(vm, this, "force_prediction", Value::Bool(false))?;
                break;
            }
        }
    }
    Ok(())
}

/// What `interact_core`'s nested functions share.
struct Core {
    this: Value,
    scene: Value,
    focus_roots: Value,
    suppress_transition: bool,
}

fn instantiate_transition(
    vm: &mut Vm,
    c: &Core,
    layer: &Value,
    old_d: Value,
    new_d: Value,
) -> PyResult<Value> {
    let it = g(vm, &c.this, "instantiated_transition")?;
    let old_trans = vm.call_method(&it, "get", vec![layer.clone(), Value::None])?;
    let ot = g(vm, &c.this, "ongoing_transition")?;
    let t = ops::getitem(vm, &ot, layer.clone())?;
    let trans = vm.call(
        &t,
        Vec::new(),
        vec![("old_widget".into(), old_d), ("new_widget".into(), new_d)],
    )?;
    if !super::core::is_displayable(vm, &trans) {
        let r = ops::repr(vm, &trans)?;
        return Err(py::error(
            vm,
            format!("Expected transition to return a displayable, not a {r}"),
        ));
    }
    if isinst(vm, &trans, "renpy.display.transform", "Transform")?
        && isinst(vm, &old_trans, "renpy.display.transform", "Transform")?
    {
        vm.call_method(&trans, "take_state", vec![old_trans.clone()])?;
        vm.call_method(&trans, "take_execution_state", vec![old_trans])?;
    }
    ops::setitem(vm, &it, layer.clone(), trans.clone())?;
    Ok(trans)
}

fn add_layer(vm: &mut Vm, c: &Core, where_: &Value, layer: &Value) -> PyResult<()> {
    let scene_layer = ops::getitem(vm, &c.scene, layer.clone())?;
    ops::list_append(vm, &c.focus_roots, scene_layer.clone())?;
    let ot = g(vm, &c.this, "ongoing_transition")?;
    let t = vm.call_method(&ot, "get", vec![layer.clone(), Value::None])?;
    let wl = g(vm, where_, "layers")?;
    if py::truthy(vm, &t)? && !c.suppress_transition {
        let tf = g(vm, &c.this, "transition_from")?;
        let from = ops::getitem(vm, &tf, layer.clone())?;
        let trans = instantiate_transition(vm, c, layer, from, scene_layer)?;
        let tt = g(vm, &c.this, "transition_time")?;
        let ttime = vm.call_method(&tt, "get", vec![layer.clone(), Value::None])?;
        vm.call_method(where_, "add", vec![trans.clone(), ttime.clone(), ttime])?;
        ops::setitem(vm, &wl, layer.clone(), trans)?;
    } else {
        vm.call_method(where_, "add", vec![scene_layer.clone()])?;
        ops::setitem(vm, &wl, layer.clone(), scene_layer)?;
    }
    Ok(())
}

/// `take_layer_displayable(ld)` closure data: (interface, scene,
/// layer_transitions, focus_roots, suppress_transition).
fn take_layer_displayable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let data = py::items(vm, &args[0])?;
    let ld = args[1].clone();
    let c = Core {
        this: data[0].clone(),
        scene: data[1].clone(),
        focus_roots: data[3].clone(),
        suppress_transition: py::truthy(vm, &data[4])?,
    };
    let layer = py::get(vm, &ld, "layer")?;
    if ops::contains(vm, &c.scene, &layer)? {
        py::set(vm, &ld, "layers", data[2].clone())?;
        add_layer(vm, &c, &ld, &layer)?;
        let n = vm.types.intern("layers");
        ops::delattr(vm, &ld, &n)?;
    } else {
        let null = mget(vm, "null")?;
        vm.call_method(&ld, "add", vec![null])?;
    }
    Ok(Value::None)
}

fn new_box(vm: &mut Vm) -> PyResult<Value> {
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let b = vm.call(&mb, Vec::new(), vec![("layout".into(), py::s(vm, "fixed"))])?;
    py::set(vm, &b, "layers", vm.types.new_dict())?;
    Ok(b)
}

fn add_layers_to_root(
    vm: &mut Vm,
    c: &Core,
    root: &Value,
    d: Value,
    st_: Value,
    at: Value,
) -> PyResult<()> {
    let lt = py::config(vm, "layer_transforms")?;
    let at_list = vm.call_method(&lt, "get", vec![Value::None, vm.types.new_list(Vec::new())])?;
    let at_list = py::items(vm, &at_list)?;
    if at_list.is_empty() {
        vm.call_method(root, "add", vec![d, st_, at])?;
        return Ok(());
    }
    let mut rv = new_box(vm)?;
    py::set(vm, &rv, "adjust_times", Value::Bool(true))?;
    vm.call_method(&rv, "add", vec![d, st_, at])?;
    let mut new_t = Value::None;
    for a in at_list {
        rv = if isinst(vm, &a, "renpy.display.motion", "Transform")? {
            vm.call(&a, Vec::new(), vec![("child".into(), rv)])?
        } else {
            vm.call(&a, vec![rv], Vec::new())?
        };
        vm.call_method(&rv, "_unique", Vec::new())?;
        if isinst(vm, &rv, "renpy.display.transform", "Transform")? {
            new_t = rv.clone();
        }
    }
    if !py::is_none(&new_t) {
        let old = g(vm, &c.this, "old_root_transform")?;
        super::scenelists::transform_state(vm, &old, &new_t, true)?;
    }
    st(vm, &c.this, "old_root_transform", new_t)?;
    vm.call_method(root, "add", vec![rv, Value::Int(0), Value::Int(0)])?;
    Ok(())
}

fn interact_core(vm: &mut Vm, this: &Value, kw: &[(Rc<str>, Value)]) -> PyResult<(bool, Value)> {
    let show_mouse = kwb(vm, kw, "show_mouse", true)?;
    let trans_pause = kwb(vm, kw, "trans_pause", false)?;
    let mut suppress_overlay = kwb(vm, kw, "suppress_overlay", false)?;
    let suppress_underlay = kwb(vm, kw, "suppress_underlay", false)?;
    let mouse = kwv(kw, "mouse", py::s(vm, "default"));
    let roll_forward = kwv(kw, "roll_forward", Value::None);
    let pause = kwv(kw, "pause", Value::None);
    let pause_start = kwv(kw, "pause_start", Value::Float(0.));
    let pause_modal = kwv(kw, "pause_modal", Value::None);

    call0(vm, "renpy.translation", "check_language")?;
    suppress_overlay = suppress_overlay || truthy_of(vm, |vm| store(vm, "suppress_overlay"))?;
    st(vm, this, "suppress_overlay", Value::Bool(suppress_overlay))?;
    st(
        vm,
        this,
        "suppress_underlay",
        Value::Bool(suppress_underlay),
    )?;
    st(vm, this, "trans_pause", Value::Bool(trans_pause))?;
    call(
        vm,
        "renpy.display.screen",
        "show_overlay_screens",
        vec![Value::Bool(suppress_overlay)],
    )?;
    call0(vm, "renpy.display.screen", "prepare_screens")?;
    st(vm, this, "roll_forward", roll_forward.clone())?;
    st(vm, this, "show_mouse", Value::Bool(show_mouse))?;
    let skipping = py::config(vm, "skipping")?;
    let less_updates = py::import_attr(vm, "renpy.game", "less_updates")?;
    let suppress_transition = py::truthy(vm, &skipping)? || py::truthy(vm, &less_updates)?;
    st(vm, this, "suppress_transition", Value::Bool(false))?;
    let after_rollback = py::import_attr(vm, "renpy.game", "after_rollback")?;
    if suppress_transition || py::truthy(vm, &after_rollback)? {
        i_enter_context(vm, &[this.clone()], &[])?;
    }
    let transition = g(vm, this, "transition")?;
    if !suppress_transition {
        let old_scene = g(vm, this, "old_scene")?;
        let ot = g(vm, this, "ongoing_transition")?;
        let tf = g(vm, this, "transition_from")?;
        let tt = g(vm, this, "transition_time")?;
        for (k, t) in ops::mapping_items(vm, &transition)? {
            if !ops::contains(vm, &old_scene, &k)? {
                continue;
            }
            ops::setitem(vm, &ot, k.clone(), t)?;
            let o = ops::getitem(vm, &old_scene, k.clone())?;
            let o = vm.call_method(&o, "_in_current_store", Vec::new())?;
            ops::setitem(vm, &tf, k.clone(), o)?;
            ops::setitem(vm, &tt, k, Value::None)?;
        }
    }
    vm.call_method(&transition, "clear", Vec::new())?;
    if trans_pause {
        let ot = g(vm, this, "ongoing_transition")?;
        let old_scene = g(vm, this, "old_scene")?;
        if !py::truthy(vm, &ot)?
            || !ops::contains(vm, &ot, &Value::None)?
            || suppress_transition
            || !py::truthy(vm, &old_scene)?
        {
            return Ok((false, Value::None));
        }
    }
    st(vm, this, "restart_interaction", Value::Bool(false))?;
    st(vm, this, "mouse", mouse)?;
    let start_time = get_time(vm);
    let mut end_time = start_time;
    st(vm, this, "frame_time", Value::Float(start_time))?;
    super::render::rs(vm).borrow_mut().frame_time = start_time;
    call_all(vm, "interact_callbacks")?;
    vm.call_method(this, "set_window_caption", Vec::new())?;
    if let Ok(f) = py::import_attr(vm, "renpy.text.text", "text_tick") {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    call0(vm, "renpy.display.predict", "reset")?;
    let sg = py::import_attr(vm, "renpy.display.layout", "size_groups")?;
    vm.call_method(&sg, "clear", Vec::new())?;
    let us = py::import_attr(vm, "renpy.display.screen", "updated_screens")?;
    vm.call_method(&us, "clear", Vec::new())?;
    let types = vm.types.new_tuple(vec![
        Value::Int(events::PERIODIC),
        Value::Int(events::TIMEEVENT),
        Value::Int(events::REDRAW),
    ]);
    call(vm, "pygame_sdl2.event", "clear", vec![types])?;
    vm.call_method(this, "post_time_event", Vec::new())?;
    let scene_lists = scene_lists(vm)?;
    vm.call_method(&scene_lists, "remove_hidden", Vec::new())?;
    if !suppress_overlay {
        vm.call_method(this, "compute_overlay", Vec::new())?;
    }
    let root_widget = new_box(vm)?;
    let focus_roots = vm.types.new_list(Vec::new());
    if !suppress_underlay {
        for i in config_items(vm, "underlay")? {
            vm.call_method(&root_widget, "add", vec![i.clone()])?;
            ops::list_append(vm, &focus_roots, i)?;
        }
        if !py::is_none(&roll_forward) {
            let rf = py::import_attr(vm, "renpy.display.behavior", "RollForward")?;
            let rfw = vm.call(&rf, vec![roll_forward.clone()], Vec::new())?;
            vm.call_method(&root_widget, "add", vec![rfw.clone()])?;
            ops::list_append(vm, &focus_roots, rfw)?;
        }
    }
    let scene = vm.call_method(this, "compute_scene", vec![scene_lists.clone()])?;
    let sroot = ops::getitem(vm, &scene, Value::None)?;
    call(vm, "renpy.display.tts", "set_root", vec![sroot])?;
    let vals = vm.call_method(&scene, "values", Vec::new())?;
    for w in ops::collect(vm, &vals)? {
        let _ = super::predict::displayable(vm, &w);
    }
    if !py::is_none(&pause) {
        let pbc = py::import_attr(vm, "renpy.display.behavior", "PauseBehavior")?;
        let pb = vm.call(
            &pbc,
            vec![pause.clone()],
            vec![("modal".into(), pause_modal.clone())],
        )?;
        vm.call_method(
            &root_widget,
            "add",
            vec![pb.clone(), pause_start.clone(), pause_start.clone()],
        )?;
        ops::list_append(vm, &focus_roots, pb)?;
    }
    let layers_root = new_box(vm)?;
    let core = Core {
        this: this.clone(),
        scene: scene.clone(),
        focus_roots: focus_roots.clone(),
        suppress_transition,
    };
    for layer in config_items(vm, "bottom_layers")? {
        add_layer(vm, &core, &root_widget, &layer)?;
    }
    for layer in config_items(vm, "layers")? {
        add_layer(vm, &core, &layers_root, &layer)?;
    }
    let ot = g(vm, this, "ongoing_transition")?;
    let root_trans = vm.call_method(&ot, "get", vec![Value::None, Value::None])?;
    if py::truthy(vm, &root_trans)? && !suppress_transition {
        let old_root = new_box(vm)?;
        let orl = g(vm, &old_root, "layers")?;
        let tf = g(vm, this, "transition_from")?;
        let from = ops::getitem(vm, &tf, Value::None)?;
        let fl = g(vm, &from, "layers")?;
        for layer in config_items(vm, "layers")? {
            let d = ops::getitem(vm, &fl, layer.clone())?;
            ops::setitem(vm, &orl, layer, d.clone())?;
            vm.call_method(&old_root, "add", vec![d])?;
        }
        let trans = instantiate_transition(vm, &core, &Value::None, old_root, layers_root.clone())?;
        let tt = g(vm, this, "transition_time")?;
        let ttime = vm.call_method(&tt, "get", vec![Value::None, Value::None])?;
        add_layers_to_root(
            vm,
            &core,
            &root_widget,
            trans.clone(),
            ttime.clone(),
            ttime.clone(),
        )?;
        if py::is_none(&ttime) && isinst(vm, &trans, "renpy.display.transform", "Transform")? {
            vm.call_method(&trans, "update_state", Vec::new())?;
        }
        if trans_pause {
            let sbc = py::import_attr(vm, "renpy.display.behavior", "SayBehavior")?;
            let dp = store(vm, "_dismiss_pause")?;
            let du = if py::truthy(vm, &dp)? {
                "dismiss"
            } else {
                "dismiss_hard_pause"
            };
            let sb = vm.call(
                &sbc,
                Vec::new(),
                vec![
                    ("dismiss".into(), vm.types.new_list(Vec::new())),
                    ("dismiss_unfocused".into(), py::s(vm, du)),
                ],
            )?;
            vm.call_method(&root_widget, "add", vec![sb.clone()])?;
            ops::list_append(vm, &focus_roots, sb)?;
            let pbc = py::import_attr(vm, "renpy.display.behavior", "PauseBehavior")?;
            let delay = py::get(vm, &trans, "delay")?;
            let pb = vm.call(&pbc, vec![delay], Vec::new())?;
            vm.call_method(&root_widget, "add", vec![pb.clone(), ttime.clone(), ttime])?;
            ops::list_append(vm, &focus_roots, pb)?;
        }
    } else {
        add_layers_to_root(
            vm,
            &core,
            &root_widget,
            layers_root.clone(),
            Value::None,
            Value::None,
        )?;
    }
    for layer in config_items(vm, "top_layers")? {
        add_layer(vm, &core, &root_widget, &layer)?;
    }
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    let mut mouse_displayable = Value::None;
    if !gb(vm, &prefs, "system_cursor")? {
        mouse_displayable = py::config(vm, "mouse_displayable")?;
        if !py::is_none(&mouse_displayable) {
            if !super::core::is_displayable(vm, &mouse_displayable) {
                mouse_displayable = vm.call(&mouse_displayable, Vec::new(), Vec::new())?;
            }
            if !py::is_none(&mouse_displayable) {
                vm.call_method(
                    &root_widget,
                    "add",
                    vec![mouse_displayable.clone(), Value::Int(0), Value::Int(0)],
                )?;
            }
        }
    }
    let pc = call(
        vm,
        "renpy.display.predict",
        "prediction_coroutine",
        vec![root_widget.clone()],
    )?;
    vm.call_method(&pc, "send", vec![Value::None])?;
    st(vm, this, "prediction_coroutine", pc)?;
    let adj = py::import_attr(vm, "renpy.display.behavior", "adj_registered")?;
    vm.call_method(&adj, "clear", Vec::new())?;
    call0(vm, "renpy.display.video", "early_interact")?;
    let layer_transitions = vm.types.new_dict();

    let r = interact_loop(
        vm,
        this,
        &core,
        &root_widget,
        &layers_root,
        &layer_transitions,
        &scene_lists,
        &mouse_displayable,
        trans_pause,
        suppress_transition,
        &mut end_time,
    );

    // finally:
    let lt_final = py::get_or_none(vm, this, "_layer_transitions")?;
    let lt = if py::is_none(&lt_final) {
        layer_transitions
    } else {
        lt_final
    };
    let delays = vm.types.new_dict();
    for (k, v) in ops::mapping_items(vm, &lt)? {
        let d = py::get_opt(vm, &v, "delay")?.unwrap_or(Value::Int(0));
        ops::setitem(vm, &delays, k, d)?;
    }
    st(vm, this, "transition_delay", delays)?;
    st(vm, this, "_layer_transitions", Value::None)?;
    for i in config_items(vm, "overlay_layers")? {
        vm.call_method(&scene_lists, "clear", vec![i])?;
    }
    for t in [events::TIMEEVENT, events::REDRAW] {
        call(
            vm,
            "pygame_sdl2.time",
            "set_timer",
            vec![Value::Int(t), Value::Int(0)],
        )?;
    }
    let ctx = context(vm)?;
    let rt = g(vm, &ctx, "runtime")?;
    let rt = py::f64_of(vm, &rt).unwrap_or(0.);
    st(
        vm,
        &ctx,
        "runtime",
        Value::Float(rt + end_time - start_time),
    )?;
    st(vm, this, "restart_interaction", Value::Bool(true))?;
    match r {
        Err(e) if exc_is(&e, "EndInteraction") => {
            let PyErr::Exc(o) = &e else { unreachable!() };
            let v = ops::getattr(vm, &Value::Obj(o.clone()), "value")?;
            Ok((false, v))
        }
        r => r,
    }
}

#[allow(clippy::too_many_arguments)]
fn interact_loop(
    vm: &mut Vm,
    this: &Value,
    core: &Core,
    root_widget: &Value,
    layers_root: &Value,
    layer_transitions: &Value,
    scene_lists: &Value,
    mouse_displayable: &Value,
    trans_pause: bool,
    suppress_transition: bool,
    end_time: &mut f64,
) -> PyResult<(bool, Value)> {
    let data = vm.types.new_tuple(vec![
        this.clone(),
        core.scene.clone(),
        layer_transitions.clone(),
        core.focus_roots.clone(),
        Value::Bool(suppress_transition),
    ]);
    let tld = py::closure(vm, "take_layer_displayable", take_layer_displayable, data);
    st(vm, this, "take_layer_displayable", tld)?;
    call0(vm, "renpy.display.behavior", "input_pre_per_interact")?;
    let cb = py::builtin(vm, "per_interact", |vm, a, _| {
        vm.call_method(&a[0], "per_interact", Vec::new())
    });
    vm.call_method(root_widget, "visit_all", vec![cb])?;
    call0(vm, "renpy.display.behavior", "input_post_per_interact")?;
    st(vm, this, "take_layer_displayable", Value::None)?;
    let lrl = g(vm, layers_root, "layers")?;
    let layer_transitions = if py::truthy(vm, layer_transitions)? {
        vm.call_method(layer_transitions, "update", vec![lrl])?;
        layer_transitions.clone()
    } else {
        lrl
    };
    st(vm, this, "_layer_transitions", layer_transitions)?;
    st(vm, this, "old_scene", core.scene.clone())?;
    let roots = py::items(vm, &core.focus_roots)?;
    super::focus::before_interact(vm, &roots)?;
    let game = py::import(vm, "renpy.game")?;
    py::set(vm, &game, "after_rollback", Value::Bool(false))?;
    if gb(vm, this, "restart_interaction")? {
        return Ok((true, Value::None));
    }
    let mut needs_redraw = true;
    let mut first_pass = true;
    st(vm, this, "interact_time", Value::None)?;
    st(vm, this, "did_autosave", Value::Bool(false))?;
    st(vm, this, "did_persistent", Value::Bool(false))?;
    let mut old_timeout_time: Option<f64> = None;
    let mut old_redraw_time: Option<f64> = None;
    call0(vm, "renpy.audio.audio", "interact")?;
    let mut video_frame_drawn = false;
    let mut frame = 0i64;
    let mut can_block = false;
    let draw_v = draw(vm)?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    loop {
        call(
            vm,
            "renpy.execution",
            "not_infinite_loop",
            vec![Value::Int(10)],
        )?;
        for f in config_items(vm, "needs_redraw_callbacks")? {
            let r = vm.call(&f, Vec::new(), Vec::new())?;
            if py::truthy(vm, &r)? {
                needs_redraw = true;
            }
        }
        let fs = gb(vm, &prefs, "fullscreen")?;
        st(vm, this, "fullscreen", Value::Bool(fs))?;
        let peek_any = {
            let all = mget(vm, "ALL_EVENTS")?;
            let items = ops::collect(vm, &all)?;
            let all = vm.types.new_list(items);
            let r = call(vm, "pygame_sdl2.event", "peek", vec![all])?;
            py::truthy(vm, &r)?
        };
        let sr = vm.call_method(
            &draw_v,
            "should_redraw",
            vec![
                Value::Bool(needs_redraw),
                Value::Bool(first_pass),
                Value::Bool(can_block),
            ],
        )?;
        if gb(vm, this, "force_redraw")? || ((first_pass || !peek_any) && py::truthy(vm, &sr)?) {
            st(vm, this, "force_redraw", Value::Bool(false))?;
            call0(vm, "renpy.display.render", "process_redraws")?;
            let fsv = call0(vm, "renpy.display.video", "interact")?;
            st(vm, this, "text_rect", Value::None)?;
            let now = get_time(vm);
            st(vm, this, "frame_time", Value::Float(now))?;
            super::render::rs(vm).borrow_mut().frame_time = now;
            call0(vm, "renpy.audio.audio", "advance_time")?;
            let fsv_t = py::truthy(vm, &fsv)?;
            vm.call_method(
                this,
                "draw_screen",
                vec![
                    root_widget.clone(),
                    fsv.clone(),
                    Value::Bool(!fsv_t || video_frame_drawn),
                ],
            )?;
            if first_pass {
                let it = g(vm, this, "interact_time")?;
                if !py::truthy(vm, &it)? {
                    let fd = g(vm, this, "frame_duration")?;
                    let fd = py::f64_of(vm, &fd)?;
                    st(
                        vm,
                        this,
                        "interact_time",
                        Value::Float(now.max(get_time(vm) - fd)),
                    )?;
                }
                let it = g(vm, this, "interact_time")?;
                vm.call_method(scene_lists, "set_times", vec![it.clone()])?;
                let tt = g(vm, this, "transition_time")?;
                for (k, v) in ops::mapping_items(vm, &tt)? {
                    if py::is_none(&v) {
                        ops::setitem(vm, &tt, k, it.clone())?;
                    }
                }
            }
            frame += 1;
            let frames = py::config(vm, "frames")?;
            let fr = py::int(vm, &frames).unwrap_or(0) + 1;
            let cfg = py::import(vm, "renpy.config")?;
            py::set(vm, &cfg, "frames", Value::Int(fr))?;
            let le = g(vm, this, "last_event")?;
            if first_pass && !py::is_none(&le) && events::is_mouse(vm, &le)? {
                let p = vm.call_method(&draw_v, "get_mouse_pos", Vec::new())?;
                let p = py::items(vm, &p)?;
                let (mut x, mut y) = (py::f64_of(vm, &p[0])?, py::f64_of(vm, &p[1])?);
                if gb(vm, this, "ignore_touch")? {
                    x = -1.;
                    y = -1.;
                }
                super::focus::mouse_handler(vm, &Value::None, x, y, false)?;
            }
            needs_redraw = false;
            first_pass = false;
            call(
                vm,
                "pygame_sdl2.time",
                "set_timer",
                vec![Value::Int(events::REDRAW), Value::Int(0)],
            )?;
            let r = vm.types.new_list(vec![Value::Int(events::REDRAW)]);
            call(vm, "pygame_sdl2.event", "clear", vec![r])?;
            old_redraw_time = None;
            vm.call_method(this, "update_text_rect", Vec::new())?;
        }
        call0(vm, "renpy.display.tts", "tick")?;
        let mm = g(vm, this, "mouse_move")?;
        if !py::is_none(&mm) {
            let more = vm.call_method(&mm, "perform", Vec::new())?;
            if !py::truthy(vm, &more)? {
                st(vm, this, "mouse_move", Value::None)?;
            }
        }
        let das = py::import_attr(vm, "renpy.loadsave", "did_autosave")?;
        if py::truthy(vm, &das)? {
            let ls = py::import(vm, "renpy.loadsave")?;
            py::set(vm, &ls, "did_autosave", Value::Bool(false))?;
            let cb = py::config(vm, "autosave_callback")?;
            call(vm, "renpy.exports", "run", vec![cb])?;
        }
        let ot = g(vm, this, "ongoing_transition")?;
        let rt = vm.call_method(&ot, "get", vec![Value::None, Value::None])?;
        if !trans_pause && py::truthy(vm, &rt)? {
            let og = vm.call_method(this, "get_ongoing_transition", vec![Value::None])?;
            if !py::truthy(vm, &og)? {
                let t = g(vm, this, "transition")?;
                vm.call_method(&t, "pop", vec![Value::None, Value::None])?;
                pop_layer(vm, this, &Value::None)?;
                st(vm, this, "restart_interaction", Value::Bool(true))?;
            }
        }
        if gb(vm, this, "restart_interaction")? && !gb(vm, this, "display_reset")? {
            return Ok((true, Value::None));
        }
        if truthy_of(vm, |vm| call0(vm, "renpy.display.video", "frequent"))? {
            needs_redraw = true;
            video_frame_drawn = true;
        }
        if super::render::check_redraws(vm) {
            needs_redraw = true;
        }
        let mut timeout_in = 3600.;
        let mut redraw_in = 3600.;
        let redraw_time = super::render::redraw_time(vm);
        can_block = true;
        let mft = g(vm, this, "maximum_framerate_time")?;
        if py::f64_of(vm, &mft)? > get_time(vm) {
            can_block = false;
        }
        match redraw_time {
            Some(rt) if !needs_redraw && can_block => {
                if Some(rt) != old_redraw_time {
                    let left = (rt - get_time(vm)).min(3600.);
                    redraw_in = left;
                    if left <= 0. {
                        let re = g(vm, this, "redraw_event")?;
                        events::input(vm).borrow_mut().queue.push(re);
                        call(
                            vm,
                            "pygame_sdl2.time",
                            "set_timer",
                            vec![Value::Int(events::REDRAW), Value::Int(0)],
                        )?;
                    } else {
                        let ms = ((left * 1000.) as i64).max(1);
                        call(
                            vm,
                            "pygame_sdl2.time",
                            "set_timer",
                            vec![Value::Int(events::REDRAW), Value::Int(ms)],
                        )?;
                    }
                    old_redraw_time = Some(rt);
                }
            }
            _ => {
                call(
                    vm,
                    "pygame_sdl2.time",
                    "set_timer",
                    vec![Value::Int(events::REDRAW), Value::Int(0)],
                )?;
            }
        }
        let tt = g(vm, this, "timeout_time")?;
        if !py::truthy(vm, &tt)? {
            call(
                vm,
                "pygame_sdl2.time",
                "set_timer",
                vec![Value::Int(events::TIMEEVENT), Value::Int(0)],
            )?;
        } else {
            let tt = py::f64_of(vm, &tt)?;
            let left = (tt - get_time(vm)).min(3600.);
            timeout_in = left;
            if left <= 0. {
                st(vm, this, "timeout_time", Value::None)?;
                call(
                    vm,
                    "pygame_sdl2.time",
                    "set_timer",
                    vec![Value::Int(events::TIMEEVENT), Value::Int(0)],
                )?;
                vm.call_method(this, "post_time_event", Vec::new())?;
            } else if Some(tt) != old_timeout_time {
                call(
                    vm,
                    "pygame_sdl2.time",
                    "set_timer",
                    vec![
                        Value::Int(events::TIMEEVENT),
                        Value::Int((left * 1000. + 1.) as i64),
                    ],
                )?;
                old_timeout_time = Some(tt);
            }
        }
        let iq = g(vm, this, "invoke_queue")?;
        while ops::len(vm, &iq)? > 0 {
            let t = vm.call_method(&iq, "pop", vec![Value::Int(0)])?;
            let p = py::items(vm, &t)?;
            let a = py::items(vm, &p[1])?;
            let k = py::str_items(vm, &p[2])?;
            let rv = vm.call(&p[0], a, k)?;
            if !py::is_none(&rv) {
                return Ok((false, rv));
            }
        }
        let idle = py::config(vm, "idle_frame")?;
        let idle = py::int(vm, &idle).unwrap_or(0);
        let playing = truthy_of(vm, |vm| call0(vm, "renpy.display.video", "playing"))?;
        if can_block || frame >= idle || gb(vm, this, "force_prediction")? {
            let mut expensive = !(needs_redraw || redraw_in < 0.2 || timeout_in < 0.2 || playing);
            let mut cb = can_block;
            if gb(vm, this, "force_prediction")? {
                expensive = true;
                cb = true;
            }
            idle_frame(vm, this, cb, expensive)?;
        }
        let mm = g(vm, this, "mouse_move")?;
        let ev = if needs_redraw || !can_block || !py::is_none(&mm) || playing {
            vm.call_method(this, "event_poll", Vec::new())?
        } else {
            vm.call_method(this, "event_wait", Vec::new())?
        };
        st(vm, this, "interaction_counter", Value::Int(0))?;
        let ty = ev_type(vm, &ev)?;
        if ty == events::NOEVENT {
            continue;
        }
        if ty == events::KEYDOWN || ty == events::KEYUP {
            let m = py::get_or_none(vm, &ev, "mod")?;
            st(vm, this, "mod", m)?;
        }
        let m = g(vm, this, "mod")?;
        py::set(vm, &ev, "mod", m)?;
        if ty == events::TIMEEVENT {
            old_timeout_time = None;
            let l = vm.types.new_list(vec![Value::Int(events::TIMEEVENT)]);
            call(vm, "pygame_sdl2.event", "clear", vec![l])?;
            py::set(vm, &ev, "modal", Value::Bool(false))?;
        }
        if ty == events::REDRAW {
            let l = vm.types.new_list(vec![Value::Int(events::REDRAW)]);
            call(vm, "pygame_sdl2.event", "clear", vec![l])?;
            old_redraw_time = None;
            continue;
        }
        if ty == events::PERIODIC {
            let l = vm.types.new_list(vec![Value::Int(events::PERIODIC)]);
            let extra = call(vm, "pygame_sdl2.event", "get", vec![l])?;
            let n = 1 + ops::len(vm, &extra)? as i64;
            let t = g(vm, this, "ticks")?;
            let t = py::int(vm, &t)? + n;
            st(vm, this, "ticks", Value::Int(t))?;
            call_all(vm, "periodic_callbacks")?;
            call0(vm, "renpy.audio.audio", "periodic")?;
            call0(vm, "renpy.display.tts", "periodic")?;
            call0(vm, "renpy.display.controller", "periodic")?;
            vm.call_method(this, "update_mouse", vec![mouse_displayable.clone()])?;
            continue;
        }
        if ty == events::QUIT {
            vm.call_method(this, "quit_event", Vec::new())?;
            continue;
        }
        if ty == events::TEXTEDITING {
            let t = py::get_or_none(vm, &ev, "text")?;
            let te = if py::truthy(vm, &t)? {
                ev.clone()
            } else {
                Value::None
            };
            st(vm, this, "text_editing", te)?;
        } else if ty == events::TEXTINPUT {
            st(vm, this, "text_editing", Value::None)?;
        } else if ty == events::KEYMAPCHANGED {
            continue;
        } else if gb(vm, this, "text_editing")? && (ty == events::KEYDOWN || ty == events::KEYUP) {
            continue;
        }
        if ty == events::VIDEOEXPOSE || ty == events::VIDEORESIZE {
            st(vm, this, "force_redraw", Value::Bool(true))?;
            continue;
        }
        if ty == events::WINDOWMOVED {
            continue;
        }
        if gb(vm, this, "ignore_touch")?
            && ty == events::MOUSEBUTTONUP
            && matches!(py::get_or_none(vm, &ev, "button")?, Value::Int(1))
        {
            st(vm, this, "ignore_touch", Value::Bool(false))?;
            continue;
        }
        if events::is_mouse(vm, &ev)? {
            st(vm, this, "mouse_event_time", Value::Float(get_time(vm)))?;
            if gb(vm, this, "ignore_touch")? {
                super::focus::mouse_handler(vm, &Value::None, -1., -1., false)?;
            }
            if !py::is_none(mouse_displayable) {
                super::render::redraw(vm, mouse_displayable, 0.);
            }
        }
        if ty == events::ACTIVEEVENT {
            let state = py::get_or_none(vm, &ev, "state")?;
            let state = py::int(vm, &state).unwrap_or(0);
            let gain = py::get_or_none(vm, &ev, "gain")?;
            let gain = py::truthy(vm, &gain)?;
            if state & 1 != 0 {
                if !gain {
                    call0(vm, "renpy.display.focus", "clear_focus")?;
                }
                st(vm, this, "mouse_focused", Value::Bool(gain))?;
            }
            if state & 2 != 0 {
                st(vm, this, "keyboard_focused", Value::Bool(gain))?;
            }
        }
        let p = vm.call_method(&draw_v, "mouse_event", vec![ev.clone()])?;
        let p = py::items(vm, &p)?;
        let (mut x, mut y) = (p[0].clone(), p[1].clone());
        let er = call(
            vm,
            "renpy.display.emulator",
            "emulator",
            vec![ev.clone(), x.clone(), y.clone()],
        )?;
        let er = py::items(vm, &er)?;
        let ev = er[0].clone();
        if py::is_none(&ev) {
            continue;
        }
        x = er[1].clone();
        y = er[2].clone();
        if !gb(vm, this, "mouse_focused")? || gb(vm, this, "ignore_touch")? {
            x = Value::Int(-1);
            y = Value::Int(-1);
        }
        let now = get_time(vm);
        *end_time = now;
        st(vm, this, "event_time", Value::Float(now))?;
        let ie = mget(vm, "input_events")?;
        if ops::contains(vm, &ie, &Value::Int(ty))? {
            st(vm, this, "input_event_time", Value::Float(now))?;
        }
        let ev = call(vm, "renpy.display.controller", "event", vec![ev])?;
        if !py::truthy(vm, &ev)? {
            continue;
        }
        call(vm, "renpy.display.behavior", "skipping", vec![ev.clone()])?;
        let xf = py::f64_of(vm, &x)?;
        let yf = py::f64_of(vm, &y)?;
        let handled = (|| -> PyResult<Value> {
            if gb(vm, this, "touch")? {
                let rec = py::import_attr(vm, "renpy.display.gesture", "recognizer")?;
                vm.call_method(&rec, "event", vec![ev.clone(), x.clone(), y.clone()])?;
            }
            let mut rv = super::focus::mouse_handler(vm, &ev, xf, yf, true)?;
            if py::is_none(&rv) {
                rv = vm.call_method(
                    root_widget,
                    "event",
                    vec![ev.clone(), x.clone(), y.clone(), Value::Int(0)],
                )?;
            }
            if py::is_none(&rv) {
                rv = call(vm, "renpy.display.focus", "key_handler", vec![ev.clone()])?;
            }
            if !py::is_none(&rv) {
                return Ok(rv);
            }
            let ot = g(vm, this, "ongoing_transition")?;
            let rt = vm.call_method(&ot, "get", vec![Value::None, Value::None])?;
            if py::truthy(vm, &rt)?
                && !suppress_transition
                && !trans_pause
                && py::config_bool(vm, "dismiss_blocking_transitions")?
            {
                let dp = store(vm, "_dismiss_pause")?;
                let dismiss = if py::truthy(vm, &dp)? {
                    "dismiss"
                } else {
                    "dismiss_hard_pause"
                };
                let me = call(
                    vm,
                    "renpy.display.behavior",
                    "map_event",
                    vec![ev.clone(), py::s(vm, dismiss)],
                )?;
                if py::truthy(vm, &me)? {
                    let t = g(vm, this, "transition")?;
                    vm.call_method(&t, "pop", vec![Value::None, Value::None])?;
                    pop_layer(vm, this, &Value::None)?;
                    st(vm, this, "restart_interaction", Value::Bool(true))?;
                    let ie = mget(vm, "IgnoreEvent")?;
                    let e = vm.call(&ie, Vec::new(), Vec::new())?;
                    return Err(PyErr::Exc(e.obj().cloned().unwrap()));
                }
            }
            Ok(Value::None)
        })();
        match handled {
            Ok(rv) if !py::is_none(&rv) => {
                if trans_pause && py::truthy(vm, &rv)? {
                    st(vm, this, "suppress_transition", Value::Bool(true))?;
                }
                return Ok((false, rv));
            }
            Ok(_) => {}
            Err(e) if exc_is(&e, "IgnoreEvent") => {
                if ty != events::TIMEEVENT {
                    vm.call_method(this, "post_time_event", Vec::new())?;
                }
            }
            Err(e) => return Err(e),
        }
        needs_redraw |= super::render::check_redraws(vm);
        if gb(vm, this, "restart_interaction")? {
            return Ok((true, Value::None));
        }
    }
}

fn r_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("init_time", f_init_time),
            ("get_time", f_get_time),
            ("get_size", f_get_size),
            ("displayable_by_tag", f_displayable_by_tag),
            ("get_safe_mode", f_get_safe_mode),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for (n, v) in [
        ("TIMEEVENT", events::TIMEEVENT),
        ("PERIODIC", events::PERIODIC),
        ("REDRAW", events::REDRAW),
        ("EVENTNAME", events::EVENTNAME),
        ("PERIODIC_INTERVAL", PERIODIC_INTERVAL),
    ] {
        set(vm, n, Value::Int(v));
    }
    let all = vm.types.new_set(Default::default());
    for e in events::standard_events()
        .into_iter()
        .chain([events::PERIODIC, events::EVENTNAME])
    {
        ops::set_add(vm, &all, Value::Int(e))?;
    }
    set(vm, "ALL_EVENTS", all.clone());
    let enabled = vm.types.new_set(Default::default());
    for e in events::enabled_events() {
        ops::set_add(vm, &enabled, Value::Int(e))?;
    }
    set(vm, "enabled_events", enabled);
    let input = vm.types.new_set(Default::default());
    for e in events::input_events() {
        ops::set_add(vm, &input, Value::Int(e))?;
    }
    set(vm, "input_events", input);
    set(vm, "null", Value::None);
    set(vm, "time_base", Value::Float(0.));
    set(vm, "time_mult", Value::Float(1.));
    set(vm, "relx", Value::Int(0));
    set(vm, "rely", Value::Int(0));
    set(vm, "initial_maximum_framerate", Value::Float(0.));
    let exc = Value::Obj(vm.types.exc("Exception"));
    let ie = py::class(vm, MODULE, "IgnoreEvent", &[exc.clone()], &[], &[])?;
    set(vm, "IgnoreEvent", Value::Obj(ie));
    let ei = py::class(
        vm,
        MODULE,
        "EndInteraction",
        &[exc],
        &[("__init__", end_interaction_init)],
        &[],
    )?;
    set(vm, "EndInteraction", Value::Obj(ei));
    let a = make_absolute(vm)?;
    set(vm, "absolute", a);
    let obj = Value::Obj(vm.types.object.clone());
    let mm = py::class(
        vm,
        MODULE,
        "MouseMove",
        &[obj.clone()],
        &[("__init__", mm_init), ("perform", mm_perform)],
        &[],
    )?;
    set(vm, "MouseMove", Value::Obj(mm));
    let renderer = py::class(
        vm,
        MODULE,
        "Renderer",
        &[obj.clone()],
        &[
            ("get_texture_size", d_get_texture_size),
            ("update", d_false),
            ("init", d_init),
            ("quit", d_none),
            ("resize", d_none),
            ("can_block", d_true),
            ("should_redraw", d_should_redraw),
            ("mutated_surface", d_none),
            ("load_texture", d_none),
            ("ready_one_texture", d_false),
            ("kill_textures", d_none),
            ("draw_screen", d_draw_screen),
            ("is_pixel_opaque", d_is_pixel_opaque),
            ("translate_point", d_translate_point),
            ("untranslate_point", d_translate_point),
            ("mouse_event", d_mouse_event),
            ("get_mouse_pos", d_get_mouse_pos),
            ("set_mouse_pos", d_set_mouse_pos),
            ("screenshot", d_screenshot),
            ("event_peek_sleep", d_none),
            ("get_physical_size", d_get_physical_size),
        ],
        &[],
    )?;
    let renderer = Value::Obj(renderer);
    set(vm, "Renderer", renderer.clone());
    let cd = py::class(vm, MODULE, "CrySirrorDraw", &[renderer], &[], &[])?;
    set(vm, "CrySirrorDraw", Value::Obj(cd));
    let iface = py::class(
        vm,
        MODULE,
        "Interface",
        &[obj],
        &[
            ("__init__", i_init),
            ("start", i_start),
            ("post_init", i_post_init),
            ("after_first_frame", r_noop),
            ("set_icon", r_noop),
            ("set_window_caption", i_set_window_caption),
            ("iconify", r_noop),
            ("kill_textures", i_kill_textures),
            ("kill_surfaces", r_noop),
            ("before_resize", i_kill_textures),
            ("set_mode", i_set_mode),
            ("draw_screen", i_draw_screen),
            ("take_screenshot", i_take_screenshot),
            ("check_background_screenshot", r_noop),
            ("get_screenshot", i_get_screenshot),
            ("lose_screenshot", i_lose_screenshot),
            ("save_screenshot", i_save_screenshot),
            ("screenshot_to_bytes", i_get_screenshot),
            ("show_window", i_show_window),
            ("do_with", i_do_with),
            ("with_none", i_with_none),
            ("end_transitions", i_end_transitions),
            ("get_ongoing_transition", i_get_ongoing_transition),
            ("set_transition", i_set_transition),
            ("event_peek", i_event_peek),
            ("event_poll", i_event_poll),
            ("event_wait", i_event_wait),
            ("compute_overlay", i_compute_overlay),
            ("compute_scene", i_compute_scene),
            ("quit_event", i_quit_event),
            ("set_mouse", i_set_mouse),
            ("hide_mouse", i_hide_mouse),
            ("is_mouse_visible", i_is_mouse_visible),
            ("get_mouse_name", i_get_mouse_name),
            ("update_mouse", i_update_mouse),
            ("set_mouse_pos", i_set_mouse_pos),
            ("drawn_since", i_drawn_since),
            ("mobile_save", r_noop),
            ("mobile_unlink", r_noop),
            ("check_android_start", r_noop),
            ("check_suspend", i_check_suspend),
            ("enter_context", i_enter_context),
            ("post_time_event", i_post_time_event),
            ("after_longpress", i_after_longpress),
            ("text_event_in_queue", i_text_event_in_queue),
            ("update_text_rect", i_update_text_rect),
            ("maximum_framerate", i_maximum_framerate),
            ("interact", i_interact),
            ("consider_gc", r_noop),
            ("idle_frame", i_idle_frame),
            ("timeout", i_timeout),
            ("finish_pending", r_noop),
            ("exec_js_cmd", r_noop),
        ],
        &[("take_layer_displayable", Value::None)],
    )?;
    set(vm, "Interface", Value::Obj(iface));
    // Pre-8.0 locations of displayable and scene-list classes.
    for (n, module) in [
        ("Displayable", "renpy.display.displayable"),
        ("DisplayableArguments", "renpy.display.displayable"),
        ("place", "renpy.display.displayable"),
        ("SceneLists", "renpy.display.scenelists"),
        ("SceneListEntry", "renpy.display.scenelists"),
    ] {
        if let Ok(v) = py::import_attr(vm, module, n) {
            set(vm, n, v);
        }
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
