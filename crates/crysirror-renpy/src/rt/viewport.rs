// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.viewport`, after Ren'Py 8.3 `viewport.py` (MIT):
//! Viewport (drag with inertia, wheel, arrow/page keys, edge scrolling)
//! and VPGrid.
use super::{
    behavior::{ignore_event, map_name},
    core::compute_raw,
    events,
    layout::{rargs, style as sty, style_b, style_f},
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.display.viewport";

fn adjustment(vm: &mut Vm) -> PyResult<Value> {
    let a = py::import_attr(vm, "renpy.display.behavior", "Adjustment")?;
    vm.call(&a, vec![Value::Int(1), Value::Int(0)], Vec::new())
}

fn fget(vm: &mut Vm, o: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, o, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn bget(vm: &mut Vm, o: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, o, n)?;
    ops::truthy(vm, &v)
}

fn vp_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 1 {
        let x = adjustment(vm)?;
        let y = adjustment(vm)?;
        py::set(vm, this, "xadjustment", x)?;
        py::set(vm, this, "yadjustment", y)?;
        for (n, v) in [
            ("set_adjustments", Value::Bool(false)),
            ("mousewheel", Value::Bool(false)),
            ("draggable", Value::Bool(false)),
            ("width", Value::Int(0)),
            ("height", Value::Int(0)),
        ] {
            py::set(vm, this, n, v)?;
        }
    }
    if version < 2 {
        py::set(vm, this, "drag_position", Value::None)?;
    }
    if version < 3 {
        let ef = py::import_attr(vm, MODULE, "edgescroll_proportional")?;
        for (n, v) in [
            ("edge_size", Value::Bool(false)),
            ("edge_speed", Value::Bool(false)),
            ("edge_function", ef),
            ("edge_xspeed", Value::Int(0)),
            ("edge_yspeed", Value::Int(0)),
            ("edge_last_st", Value::None),
        ] {
            py::set(vm, this, n, v)?;
        }
    }
    if version < 5 {
        let d = py::get_or_none(vm, this, "draggable")?;
        py::set(vm, this, "focusable", d)?;
    }
    Ok(Value::None)
}

const VP_NAMES: &[&str] = &[
    "child",
    "child_size",
    "offsets",
    "xadjustment",
    "yadjustment",
    "set_adjustments",
    "mousewheel",
    "draggable",
    "edgescroll",
    "style",
    "xinitial",
    "yinitial",
    "replaces",
    "arrowkeys",
    "pagekeys",
];

fn vp_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pair = vm.types.new_tuple(vec![Value::None, Value::None]);
    let mut v = vec![
        Value::None,
        pair.clone(),
        pair,
        Value::None,
        Value::None,
        Value::Bool(true),
        Value::Bool(false),
        Value::Bool(false),
        Value::None,
        vm.types.new_str("viewport"),
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
    ];
    for (i, a) in args.iter().skip(1).enumerate().take(VP_NAMES.len()) {
        v[i] = a.clone();
    }
    let mut props: Vec<(Rc<str>, Value)> = Vec::new();
    for (k, val) in kw {
        match VP_NAMES.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    if !props.iter().any(|(k, _)| &**k == "keyboard_focus") {
        props.push(("keyboard_focus".into(), v[13].clone()));
    }
    props.push(("style".into(), v[9].clone()));
    super::layout::container_init(vm, &[this.clone()], &props)?;
    if !py::is_none(&v[0]) {
        vm.call_method(this, "add", vec![v[0].clone()])?;
    }
    let xa = if py::is_none(&v[3]) {
        adjustment(vm)?
    } else {
        v[3].clone()
    };
    let ya = if py::is_none(&v[4]) {
        adjustment(vm)?
    } else {
        v[4].clone()
    };
    for a in [&xa, &ya] {
        if py::is_none(&py::get_or_none(vm, a, "adjustable")?) {
            py::set(vm, a, "adjustable", Value::Bool(true))?;
        }
    }
    py::set(vm, this, "xadjustment", xa.clone())?;
    py::set(vm, this, "yadjustment", ya.clone())?;
    py::set(vm, this, "set_adjustments", v[5].clone())?;
    let offsets = py::items(vm, &v[2])?;
    let xo = if py::is_none(&offsets[0]) {
        v[10].clone()
    } else {
        offsets[0].clone()
    };
    let yo = if py::is_none(&offsets[1]) {
        v[11].clone()
    } else {
        offsets[1].clone()
    };
    py::set(vm, this, "xoffset", xo)?;
    py::set(vm, this, "yoffset", yo)?;
    let vpc = py::import_attr(vm, MODULE, "Viewport")?;
    let replaces = v[12].clone();
    let is_vp = vpc.obj().is_some_and(|c| ops::isinstance(vm, &replaces, c));
    let r_offsets = if is_vp {
        py::get_or_none(vm, &replaces, "offsets")?
    } else {
        Value::None
    };
    if is_vp && ops::truthy(vm, &r_offsets)? {
        let rxa = py::get(vm, &replaces, "xadjustment")?;
        let rya = py::get(vm, &replaces, "yadjustment")?;
        vm.call_method(&xa, "viewport_replaces", vec![rxa.clone()])?;
        vm.call_method(&ya, "viewport_replaces", vec![rya.clone()])?;
        for (a, r) in [(&xa, &rxa), (&ya, &rya)] {
            for n in ["range", "value"] {
                let x = py::get(vm, r, n)?;
                py::set(vm, a, n, x)?;
            }
        }
        for n in [
            "xoffset",
            "yoffset",
            "drag_position",
            "drag_position_time",
            "drag_speed",
        ] {
            let x = py::get_or_none(vm, &replaces, n)?;
            py::set(vm, this, n, x)?;
        }
    } else {
        for n in ["drag_position", "drag_position_time", "drag_speed"] {
            py::set(vm, this, n, Value::None)?;
        }
    }
    let cs = py::items(vm, &v[1])?;
    py::set(vm, this, "child_width", cs[0].clone())?;
    py::set(vm, this, "child_height", cs[1].clone())?;
    let draggable = if matches!(v[7], Value::Bool(_)) {
        v[7].clone()
    } else {
        let f = py::import_attr(vm, "renpy.exports", "variant")?;
        vm.call(&f, vec![v[7].clone()], Vec::new())?
    };
    py::set(vm, this, "draggable", draggable)?;
    py::set(vm, this, "mousewheel", v[6].clone())?;
    py::set(vm, this, "arrowkeys", v[13].clone())?;
    py::set(vm, this, "pagekeys", v[14].clone())?;
    let focusable = ops::truthy(vm, &v[7])? || ops::truthy(vm, &v[13])?;
    py::set(vm, this, "focusable", Value::Bool(focusable))?;
    for n in ["width", "height", "edge_xspeed", "edge_yspeed"] {
        py::set(vm, this, n, Value::Int(0))?;
    }
    py::set(vm, this, "edge_last_st", Value::None)?;
    let prop = py::import_attr(vm, MODULE, "edgescroll_proportional")?;
    if !py::is_none(&v[8]) {
        let e = py::items(vm, &v[8])?;
        py::set(vm, this, "edge_size", e[0].clone())?;
        py::set(vm, this, "edge_speed", e[1].clone())?;
        py::set(vm, this, "edge_function", e.get(2).cloned().unwrap_or(prop))?;
    } else {
        py::set(vm, this, "edge_size", Value::Int(0))?;
        py::set(vm, this, "edge_speed", Value::Int(0))?;
        py::set(vm, this, "edge_function", prop)?;
    }
    Ok(Value::None)
}

fn vp_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for n in ["xadjustment", "yadjustment"] {
        let a = py::get(vm, &args[0], n)?;
        vm.call_method(&a, "register", vec![args[0].clone()])?;
    }
    Ok(Value::None)
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn adj_set(vm: &mut Vm, a: &Value, n: &str, v: f64) -> PyResult<()> {
    py::set(vm, a, n, Value::Float(v))
}

fn update_offsets(
    vm: &mut Vm,
    this: &Value,
    cw: f64,
    ch: f64,
    st: f64,
) -> PyResult<(f64, f64, f64, f64)> {
    let cw = cw.ceil();
    let ch = ch.ceil();
    let mut width = fget(vm, this, "width")?;
    let mut height = fget(vm, this, "height")?;
    let (xmin, ymin) = super::layout::xyminimums(vm, this, width, height)?;
    if !style_b(vm, this, "xfill")? {
        width = width.min(cw);
    }
    if !style_b(vm, this, "yfill")? {
        height = height.min(ch);
    }
    width = width.max(xmin);
    height = height.max(ymin);
    let xa = py::get(vm, this, "xadjustment")?;
    let ya = py::get(vm, this, "yadjustment")?;
    let sizing = render::rs(vm).borrow().sizing;
    if !sizing && bget(vm, this, "set_adjustments")? {
        for (a, c, d) in [(&xa, cw, width), (&ya, ch, height)] {
            let range = (c - d).max(0.);
            if fget(vm, a, "range")? != range || fget(vm, a, "page")? != d {
                adj_set(vm, a, "range", range)?;
                adj_set(vm, a, "page", d)?;
                vm.call_method(a, "update", Vec::new())?;
            }
        }
    }
    for (n, a, c, d) in [("xoffset", &xa, cw, width), ("yoffset", &ya, ch, height)] {
        let o = py::get_or_none(vm, this, n)?;
        if !py::is_none(&o) {
            let value = if matches!(o, Value::Int(_)) {
                o.clone()
            } else {
                Value::Float((c - d).max(0.) * py::f64_of(vm, &o)?)
            };
            py::set(vm, a, "value", value)?;
        }
    }
    let es = py::get_or_none(vm, this, "edge_last_st")?;
    let (exs, eys) = (
        fget(vm, this, "edge_xspeed")?,
        fget(vm, this, "edge_yspeed")?,
    );
    if bget(vm, this, "edge_size")? && !py::is_none(&es) && (exs != 0. || eys != 0.) {
        let dur = (st - py::f64_of(vm, &es)?).max(0.);
        for (a, s) in [(&xa, exs), (&ya, eys)] {
            let v = fget(vm, a, "value")?;
            vm.call_method(a, "change", vec![Value::Float(v + dur * s)])?;
        }
        check_edge_redraw(vm, this, st, true)?;
    }
    for a in [&xa, &ya] {
        let r = vm.call_method(a, "periodic", vec![Value::Float(st)])?;
        if !py::is_none(&r) {
            let t = py::f64_of(vm, &r)?;
            render::redraw(vm, this, t);
        }
    }
    let cxo = -fget(vm, &xa, "value")?.trunc();
    let cyo = -fget(vm, &ya, "value")?.trunc();
    py::set(vm, this, "width", Value::Float(width))?;
    py::set(vm, this, "height", Value::Float(height))?;
    Ok((cxo, cyo, width, height))
}

fn vp_update_offsets(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cw = py::f64_of(vm, &args[1])?;
    let ch = py::f64_of(vm, &args[2])?;
    let st = py::f64_of(vm, &args[3])?;
    let (a, b, c, d) = update_offsets(vm, &args[0], cw, ch, st)?;
    Ok(vm.types.new_tuple(vec![
        Value::Int(a as i64),
        Value::Int(b as i64),
        Value::Float(c),
        Value::Float(d),
    ]))
}

fn child_room(vm: &mut Vm, this: &Value, w: f64, h: f64) -> PyResult<(f64, f64)> {
    let cw = py::get_or_none(vm, this, "child_width")?;
    let ch = py::get_or_none(vm, this, "child_height")?;
    let cw = if ops::truthy(vm, &cw)? {
        py::f64_of(vm, &cw)?
    } else {
        w
    };
    let ch = if ops::truthy(vm, &ch)? {
        py::f64_of(vm, &ch)?
    } else {
        h
    };
    Ok((cw, ch))
}

fn finish(vm: &mut Vm, this: &Value, rv: &Value, w: f64, h: f64) -> PyResult<Value> {
    let rv = render::subsurface(vm, rv, [0., 0., w, h], true, false)?;
    if bget(vm, this, "arrowkeys")? || bget(vm, this, "draggable")? {
        let nmf = vm.types.new_str("NO_MOUSE_FOCUS");
        vm.call_method(
            &rv,
            "add_focus",
            vec![
                this.clone(),
                nmf,
                Value::Int(0),
                Value::Int(0),
                Value::Float(w),
                Value::Float(h),
            ],
        )?;
    }
    Ok(rv)
}

fn vp_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    let (cw, ch) = child_room(vm, this, w, h)?;
    let child = py::get(vm, this, "child")?;
    let surf = render::render(vm, &child, cw, ch, st, at)?;
    let (sw, sh) = super::layout::size(&surf);
    let (cxo, cyo, w, h) = update_offsets(vm, this, sw, sh, st)?;
    let off = vm
        .types
        .new_tuple(vec![Value::Int(cxo as i64), Value::Int(cyo as i64)]);
    py::set(vm, this, "offsets", vm.types.new_list(vec![off]))?;
    let rv = super::layout::new(vm, w, h);
    super::layout::blit(vm, &rv, &surf, cxo, cyo)?;
    finish(vm, this, &rv, w, h)
}

fn check_edge_redraw(vm: &mut Vm, this: &Value, st: f64, reset_st: bool) -> PyResult<()> {
    let xa = py::get(vm, this, "xadjustment")?;
    let ya = py::get(vm, this, "yadjustment")?;
    let (exs, eys) = (
        fget(vm, this, "edge_xspeed")?,
        fget(vm, this, "edge_yspeed")?,
    );
    let (xv, xr) = (fget(vm, &xa, "value")?, fget(vm, &xa, "range")?);
    let (yv, yr) = (fget(vm, &ya, "value")?, fget(vm, &ya, "range")?);
    let redraw = (exs > 0. && xv < xr)
        || (exs < 0. && xv > 0.)
        || (eys > 0. && yv < yr)
        || (eys < 0. && yv > 0.);
    if redraw {
        render::redraw(vm, this, 0.);
        let es = py::get_or_none(vm, this, "edge_last_st")?;
        if reset_st || py::is_none(&es) {
            py::set(vm, this, "edge_last_st", Value::Float(st))?;
        }
    } else {
        py::set(vm, this, "edge_last_st", Value::None)?;
    }
    Ok(())
}

fn vp_check_edge_redraw(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let st = py::f64_of(vm, &args[1])?;
    let r = {
        let v = arg_or(args, kw, 2, "reset_st", Value::Bool(true));
        ops::truthy(vm, &v)?
    };
    check_edge_redraw(vm, &args[0], st, r)?;
    Ok(Value::None)
}

fn change(vm: &mut Vm, a: &Value, v: f64) -> PyResult<Value> {
    let rv = vm.call_method(a, "change", vec![Value::Float(v)])?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    Err(ignore_event(vm))
}

fn get_grab(vm: &mut Vm) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.display.focus", "get_grab")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn set_grab(vm: &mut Vm, v: Value) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.display.focus", "set_grab")?;
    vm.call(&f, vec![v], Vec::new())?;
    Ok(())
}

fn pair(vm: &mut Vm, v: &Value) -> PyResult<(f64, f64)> {
    let p = py::items(vm, v)?;
    Ok((py::f64_of(vm, &p[0])?, py::f64_of(vm, &p[1])?))
}

fn release_axis(
    vm: &mut Vm,
    a: &Value,
    old: f64,
    speed: f64,
    screen: &str,
    st: f64,
) -> PyResult<()> {
    let amp = py::config_f64(vm, "viewport_inertia_amplitude").unwrap_or(0.);
    let fs = py::get_or_none(vm, a, "force_step")?;
    if speed != 0. && amp != 0. && !ops::truthy(vm, &fs)? {
        let tc = py::config_f64(vm, "viewport_inertia_time_constant").unwrap_or(0.);
        vm.call_method(
            a,
            "inertia",
            vec![
                Value::Float(amp * speed),
                Value::Float(tc),
                Value::Float(st),
            ],
        )?;
    } else if fs.as_str() == Some("release") {
        let v = py::method_kw(
            vm,
            a,
            "round_value",
            vec![Value::Float(old)],
            vec![("release".into(), Value::Bool(true))],
        )?;
        let v = py::f64_of(vm, &v)?;
        let step = fget(vm, a, "step")?;
        let sw = py::config_f64(vm, screen)?;
        vm.call_method(
            a,
            "inertia",
            vec![
                Value::Float(v - old),
                Value::Float(step / (sw * 2.)),
                Value::Float(st),
            ],
        )?;
    } else {
        let v = py::method_kw(
            vm,
            a,
            "round_value",
            vec![Value::Float(old)],
            vec![("release".into(), Value::Bool(true))],
        )?;
        vm.call_method(a, "change", vec![v])?;
    }
    Ok(())
}

fn edge_speed(n: f64, zero: f64, one: f64) -> f64 {
    let n = (n - zero) / (one - zero);
    if !(0. ..=1.).contains(&n) { 0. } else { n }
}

fn vp_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args[1].clone();
    let x = py::f64_of(vm, &args[2])?;
    let y = py::f64_of(vm, &args[3])?;
    let st = py::f64_of(vm, &args[4])?;
    py::set(vm, this, "xoffset", Value::None)?;
    py::set(vm, this, "yoffset", Value::None)?;
    let (w, h) = (fget(vm, this, "width")?, fget(vm, this, "height")?);
    let inside = (0. ..w).contains(&x) && (0. ..=h).contains(&y);
    if !inside {
        py::set(vm, this, "edge_xspeed", Value::Int(0))?;
        py::set(vm, this, "edge_yspeed", Value::Int(0))?;
        py::set(vm, this, "edge_last_st", Value::None)?;
    }
    let xa = py::get(vm, this, "xadjustment")?;
    let ya = py::get(vm, this, "yadjustment")?;
    let draggable = bget(vm, this, "draggable")?
        && (fget(vm, &xa, "range")? != 0. || fget(vm, &ya, "range")? != 0.);
    let mut grab = get_grab(vm)?;
    let grab_draggable = !py::is_none(&grab) && {
        let v = py::get_opt(vm, &grab, "_draggable")?.unwrap_or(Value::Bool(false));
        ops::truthy(vm, &v)?
    };
    if grab_draggable && !py::same(&grab, this) {
        py::set(vm, this, "drag_position", Value::None)?;
    } else if draggable {
        if py::is_none(&grab) && map_name(vm, &ev, "viewport_drag_end")? {
            py::set(vm, this, "drag_position", Value::None)?;
        }
    } else {
        py::set(vm, this, "drag_position", Value::None)?;
    }
    let dp = py::get_or_none(vm, this, "drag_position")?;
    if inside
        && draggable
        && !py::is_none(&dp)
        && !py::same(&grab, this)
        && events::is_type(vm, &ev, events::MOUSEMOTION)?
    {
        let (ox, oy) = pair(vm, &dp)?;
        let grabbed = grab_draggable && {
            let f = vm.call_method(&grab, "is_focused", Vec::new())?;
            ops::truthy(vm, &f)?
        };
        let radius = py::config_f64(vm, "viewport_drag_radius").unwrap_or(10.);
        if (ox - x).hypot(oy - y) >= radius && !grabbed {
            let ff = py::import_attr(vm, "renpy.display.focus", "force_focus")?;
            let rv = vm.call(&ff, vec![this.clone()], Vec::new())?;
            set_grab(vm, this.clone())?;
            py::set(
                vm,
                this,
                "drag_position",
                vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]),
            )?;
            py::set(vm, this, "drag_position_time", Value::Float(st))?;
            py::set(
                vm,
                this,
                "drag_speed",
                vm.types.new_tuple(vec![Value::Float(0.), Value::Float(0.)]),
            )?;
            grab = this.clone();
            let _ = &grab;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
        }
    }
    if py::same(&get_grab(vm)?, this) {
        let old_x = fget(vm, &xa, "value")?;
        let old_y = fget(vm, &ya, "value")?;
        let dp = py::get(vm, this, "drag_position")?;
        let (ox, oy) = pair(vm, &dp)?;
        let (dx, dy) = (x - ox, y - oy);
        let dt = st - fget(vm, this, "drag_position_time")?;
        if dt > 0. {
            let ds = py::get(vm, this, "drag_speed")?;
            let (osx, osy) = pair(vm, &ds)?;
            let (nsx, nsy) = (-dx / dt / 60., -dy / dt / 60.);
            let done = (dt / (1. / 60.)).min(1.);
            let s = vm.types.new_tuple(vec![
                Value::Float(osx + done * (nsx - osx)),
                Value::Float(osy + done * (nsy - osy)),
            ]);
            py::set(vm, this, "drag_speed", s)?;
        }
        if map_name(vm, &ev, "viewport_drag_end")? {
            set_grab(vm, Value::None)?;
            let ds = py::get(vm, this, "drag_speed")?;
            let (sx, sy) = pair(vm, &ds)?;
            release_axis(vm, &xa, old_x, sx, "screen_width", st)?;
            release_axis(vm, &ya, old_y, sy, "screen_height", st)?;
            py::set(vm, this, "drag_position", Value::None)?;
            py::set(vm, this, "drag_position_time", Value::None)?;
            return Err(ignore_event(vm));
        }
        let nx = py::method_kw(
            vm,
            &xa,
            "round_value",
            vec![Value::Float(old_x - dx)],
            vec![("release".into(), Value::Bool(false))],
        )?;
        let newx = if py::f64_of(vm, &nx)? == old_x {
            ox
        } else {
            vm.call_method(&xa, "change", vec![nx])?;
            x
        };
        let ny = py::method_kw(
            vm,
            &ya,
            "round_value",
            vec![Value::Float(old_y - dy)],
            vec![("release".into(), Value::Bool(false))],
        )?;
        let newy = if py::f64_of(vm, &ny)? == old_y {
            oy
        } else {
            vm.call_method(&ya, "change", vec![ny])?;
            y
        };
        py::set(
            vm,
            this,
            "drag_position",
            vm.types
                .new_tuple(vec![Value::Float(newx), Value::Float(newy)]),
        )?;
        py::set(vm, this, "drag_position_time", Value::Float(st))?;
    }
    let mw = py::get_or_none(vm, this, "mousewheel")?;
    if inside && ops::truthy(vm, &mw)? {
        let (adj, ch) = match mw.as_str() {
            Some("horizontal-change") => (&xa, true),
            Some("change") => (&ya, true),
            Some("horizontal") => (&xa, false),
            _ => (&ya, false),
        };
        let (v, r, step) = (
            fget(vm, adj, "value")?,
            fget(vm, adj, "range")?,
            fget(vm, adj, "step")?,
        );
        if map_name(vm, &ev, "viewport_wheelup")? {
            if ch && v == 0. {
                return Ok(Value::None);
            }
            return change(vm, adj, v - step);
        }
        if map_name(vm, &ev, "viewport_wheeldown")? {
            if ch && v == r {
                return Ok(Value::None);
            }
            return change(vm, adj, v + step);
        }
    }
    if bget(vm, this, "arrowkeys")? {
        for (name, adj, dir) in [
            ("viewport_leftarrow", &xa, -1.),
            ("viewport_rightarrow", &xa, 1.),
            ("viewport_uparrow", &ya, -1.),
            ("viewport_downarrow", &ya, 1.),
        ] {
            if map_name(vm, &ev, name)? {
                let (v, r, step) = (
                    fget(vm, adj, "value")?,
                    fget(vm, adj, "range")?,
                    fget(vm, adj, "step")?,
                );
                if (dir < 0. && v == 0.) || (dir > 0. && v == r) {
                    return Ok(Value::None);
                }
                return change(vm, adj, v + dir * step);
            }
        }
    }
    if bget(vm, this, "pagekeys")? {
        for (name, dir) in [("viewport_pageup", -1.), ("viewport_pagedown", 1.)] {
            if map_name(vm, &ev, name)? {
                let (v, p) = (fget(vm, &ya, "value")?, fget(vm, &ya, "page")?);
                return change(vm, &ya, v + dir * p);
            }
        }
    }
    let edge_size = fget(vm, this, "edge_size")?;
    let ty = {
        let t = py::get_or_none(vm, &ev, "type")?;
        py::int(vm, &t).unwrap_or(0)
    };
    if inside
        && edge_size != 0.
        && matches!(
            ty,
            events::MOUSEMOTION | events::MOUSEBUTTONDOWN | events::MOUSEBUTTONUP
        )
    {
        let speed = fget(vm, this, "edge_speed")?;
        let ef = py::get(vm, this, "edge_function")?;
        let xs = edge_speed(x, w - edge_size, w) - edge_speed(x, edge_size, 0.);
        let ys = edge_speed(y, h - edge_size, h) - edge_speed(y, edge_size, 0.);
        let fx = vm.call(&ef, vec![Value::Float(xs)], Vec::new())?;
        let fy = vm.call(&ef, vec![Value::Float(ys)], Vec::new())?;
        let (fx, fy) = (py::f64_of(vm, &fx)?, py::f64_of(vm, &fy)?);
        py::set(vm, this, "edge_xspeed", Value::Float(speed * fx))?;
        py::set(vm, this, "edge_yspeed", Value::Float(speed * fy))?;
        if xs != 0. || ys != 0. {
            check_edge_redraw(vm, this, st, false)?;
        } else {
            py::set(vm, this, "edge_last_st", Value::None)?;
        }
    }
    let mut ignore = false;
    if inside && draggable && map_name(vm, &ev, "viewport_drag_start")? {
        py::set(
            vm,
            this,
            "drag_position",
            vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]),
        )?;
        py::set(vm, this, "drag_position_time", Value::Float(st))?;
        py::set(
            vm,
            this,
            "drag_speed",
            vm.types.new_tuple(vec![Value::Float(0.), Value::Float(0.)]),
        )?;
        for a in [&xa, &ya] {
            py::method_kw(
                vm,
                a,
                "end_animation",
                Vec::new(),
                vec![("instantly".into(), Value::Bool(true))],
            )?;
        }
        ignore = true;
    }
    let rv = super::layout::container_event(vm, args, kw)?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    if ignore {
        return Err(ignore_event(vm));
    }
    Ok(Value::None)
}

fn vp_set_xoffset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "xoffset", args[1].clone())?;
    render::redraw(vm, &args[0], 0.);
    Ok(Value::None)
}

fn vp_set_yoffset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "yoffset", args[1].clone())?;
    render::redraw(vm, &args[0], 0.);
    Ok(Value::None)
}

// VPGrid -------------------------------------------------------------------------------------------

fn vpg_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["cols", "rows", "transpose", "style", "allow_underfull"];
    let mut v = vec![
        Value::None,
        Value::None,
        Value::None,
        vm.types.new_str("vpgrid"),
        Value::None,
    ];
    for (i, a) in args.iter().skip(1).enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    props.push(("style".into(), v[3].clone()));
    vp_init(vm, &[this.clone()], &props)?;
    if py::is_none(&v[0]) && py::is_none(&v[1]) {
        return Err(py::error(
            vm,
            "A VPGrid must be given the rows or cols property.",
        ));
    }
    let mut transpose = v[2].clone();
    if !py::is_none(&v[1]) && py::is_none(&v[0]) && py::is_none(&transpose) {
        transpose = Value::Bool(true);
    }
    py::set(vm, this, "grid_cols", v[0].clone())?;
    py::set(vm, this, "grid_rows", v[1].clone())?;
    py::set(vm, this, "grid_transpose", transpose)?;
    py::set(vm, this, "allow_underfull", v[4].clone())?;
    Ok(Value::None)
}

fn vpg_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    let (cw0, ch0) = child_room(vm, this, w, h)?;
    let children = py::get_items(vm, this, "children")?;
    if children.is_empty() {
        py::set(vm, this, "offsets", vm.types.new_list(Vec::new()))?;
        return Ok(super::layout::new(vm, 0., 0.));
    }
    let lc = children.len() as i64;
    let gc = py::get_or_none(vm, this, "grid_cols")?;
    let gr = py::get_or_none(vm, this, "grid_rows")?;
    let (mut cols, mut rows) = (
        if py::is_none(&gc) {
            0
        } else {
            py::int(vm, &gc)?
        },
        if py::is_none(&gr) {
            0
        } else {
            py::int(vm, &gr)?
        },
    );
    if py::is_none(&gc) {
        cols = (lc + rows - 1) / rows.max(1);
    }
    if py::is_none(&gr) {
        rows = (lc + cols - 1) / cols.max(1);
    }
    let spacing = sty(vm, this, "spacing")?;
    let mut xs = sty(vm, this, "xspacing")?;
    let mut ys = sty(vm, this, "yspacing")?;
    if py::is_none(&xs) {
        xs = spacing.clone();
    }
    if py::is_none(&ys) {
        ys = spacing;
    }
    let (xs, ys) = if py::config_bool(vm, "relative_spacing").unwrap_or(false) {
        (compute_raw(vm, &xs, w)?, compute_raw(vm, &ys, h)?)
    } else {
        (
            py::f64_of(vm, &xs).unwrap_or(0.),
            py::f64_of(vm, &ys).unwrap_or(0.),
        )
    };
    let lm = style_f(vm, this, "left_margin", w)?;
    let rm = style_f(vm, this, "right_margin", w)?;
    let tm = style_f(vm, this, "top_margin", h)?;
    let bm = style_f(vm, this, "bottom_margin", h)?;
    let rend = render::render(vm, &children[0], cw0, ch0, st, at)?;
    let (mut cw, mut ch) = super::layout::size(&rend);
    let (cf, rf) = (cols as f64, rows as f64);
    let mut tw = (cw + xs) * cf - xs + lm + rm;
    let mut th = (ch + ys) * rf - ys + tm + bm;
    if style_b(vm, this, "xfill")? {
        tw = cw0;
        cw = ((tw - (cf - 1.) * xs - lm - rm) / cf).floor();
    }
    if style_b(vm, this, "yfill")? {
        th = ch0;
        ch = ((th - (rf - 1.) * ys - tm - bm) / rf).floor();
    }
    let (cxo, cyo, w, h) = update_offsets(vm, this, tw, th, st)?;
    let (cxo, cyo) = (cxo + lm, cyo + tm);
    let rv = super::layout::new(vm, w, h);
    let transpose = bget(vm, this, "grid_transpose")?;
    let mut offsets = Vec::new();
    for (index, c) in children.iter().enumerate() {
        let index = index as i64;
        let (gx, gy) = if transpose {
            (index / rows.max(1), index % rows.max(1))
        } else {
            (index % cols.max(1), index / cols.max(1))
        };
        let x = gx as f64 * (cw + xs) + cxo;
        let y = gy as f64 * (ch + ys) + cyo;
        if x + cw < 0. || y + ch < 0. || x >= w || y >= h {
            offsets.push(vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]));
            continue;
        }
        let surf = render::render(vm, c, cw, ch, st, at)?;
        let pos = vm.call_method(
            c,
            "place",
            vec![
                rv.clone(),
                Value::Float(x),
                Value::Float(y),
                Value::Float(cw),
                Value::Float(ch),
                surf,
            ],
        )?;
        offsets.push(pos);
    }
    py::set(vm, this, "offsets", vm.types.new_list(offsets))?;
    finish(vm, this, &rv, w, h)
}

fn vpg_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let f = py::get(vm, &container, "add")?;
    vm.call(&f, args.to_vec(), kw.to_vec())?;
    let gc = py::get_or_none(vm, this, "grid_cols")?;
    let gr = py::get_or_none(vm, this, "grid_rows")?;
    if !py::is_none(&gc) && !py::is_none(&gr) {
        let n = py::get_items(vm, this, "children")?.len() as i64;
        if n > py::int(vm, &gc)? * py::int(vm, &gr)?
            && !py::config_bool(vm, "allow_unfull_vpgrids").unwrap_or(false)
        {
            return Err(py::error(vm, "VPGrid overfull."));
        }
    }
    Ok(Value::None)
}

fn vpg_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    vp_per_interact(vm, args, &[])?;
    let children = py::get_items(vm, this, "children")?.len() as i64;
    let gc = py::get_or_none(vm, this, "grid_cols")?;
    let gr = py::get_or_none(vm, this, "grid_rows")?;
    let given_v = if ops::truthy(vm, &gc)? {
        gc.clone()
    } else {
        gr.clone()
    };
    let given = if py::is_none(&given_v) {
        0
    } else {
        py::int(vm, &given_v)?
    };
    let delta = if py::is_none(&gc) || py::is_none(&gr) {
        if given != 0 {
            let m = children % given;
            given - if m == 0 { given } else { m }
        } else {
            0
        }
    } else {
        py::int(vm, &gc)? * py::int(vm, &gr)? - children
    };
    if delta <= 0 {
        return Ok(Value::None);
    }
    if py::config_bool(vm, "developer").unwrap_or(false) {
        let mut au = py::get_or_none(vm, this, "allow_underfull")?;
        if py::is_none(&au) {
            let a = py::config_bool(vm, "allow_underfull_grids").unwrap_or(false)
                || py::config_bool(vm, "allow_unfull_vpgrids").unwrap_or(false);
            au = Value::Bool(a);
        }
        if !ops::truthy(vm, &au)? {
            let mut msg = "VPGrid not completely full".to_owned();
            if py::is_none(&gc) || py::is_none(&gr) {
                msg += &format!(", needs a multiple of {given} children");
            }
            return Err(py::error(vm, msg + "."));
        }
    }
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let n = vm.call(&null, Vec::new(), Vec::new())?;
    for _ in 0..delta {
        vm.call_method(this, "add", vec![n.clone()])?;
    }
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("edgescroll_proportional", |_, a, _| {
            Ok(a.first().cloned().unwrap_or(Value::None))
        })],
    );
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let vp = py::class(
        vm,
        MODULE,
        "Viewport",
        &[container],
        &[
            ("after_upgrade", vp_after_upgrade),
            ("__init__", vp_init),
            ("per_interact", vp_per_interact),
            ("set_style_prefix", noop),
            ("update_offsets", vp_update_offsets),
            ("render", vp_render),
            ("check_edge_redraw", vp_check_edge_redraw),
            ("event", vp_event),
            ("set_xoffset", vp_set_xoffset),
            ("set_yoffset", vp_set_yoffset),
        ],
        &[
            ("__version__", Value::Int(5)),
            ("arrowkeys", Value::Bool(false)),
            ("pagekeys", Value::Bool(false)),
            ("_draggable", Value::Bool(true)),
            ("drag_position_time", Value::None),
        ],
    )?;
    let vpv = Value::Obj(vp);
    crysirror_python::modules::set(vm, &m, "Viewport", vpv.clone());
    let layout = py::import(vm, "renpy.display.layout")?;
    py::set(vm, &layout, "Viewport", vpv.clone())?;
    let g = py::class(
        vm,
        MODULE,
        "VPGrid",
        &[vpv],
        &[
            ("__init__", vpg_init),
            ("render", vpg_render),
            ("add", vpg_add),
            ("per_interact", vpg_per_interact),
        ],
        &[
            ("__version__", Value::Int(5)),
            ("allow_underfull", Value::None),
        ],
    )?;
    crysirror_python::modules::set(vm, &m, "VPGrid", Value::Obj(g));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
