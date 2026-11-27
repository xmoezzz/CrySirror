// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.movetransition`, after Ren'Py 8.3 (MIT): MoveTransition
//! (MoveInterpolate per shown image), the pre-6.10 OldMoveTransition with
//! factories, and the factory helpers.
use super::{
    core::compute_raw,
    layout::rargs,
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.movetransition";

fn position4(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    let p = vm.call_method(d, "get_placement", Vec::new())?;
    let p = py::items(vm, &p)?;
    let out: Vec<Value> = p.iter().take(4).map(|v| if py::is_none(v) { Value::Int(0) } else { v.clone() }).collect();
    Ok(vm.types.new_tuple(out))
}

fn offsets(vm: &mut Vm, d: &Value) -> PyResult<Vec<(std::rc::Rc<str>, Value)>> {
    if !py::config_bool(vm, "movetransition_respects_offsets").unwrap_or(true) {
        return Ok(Vec::new());
    }
    let p = vm.call_method(d, "get_placement", Vec::new())?;
    let p = py::items(vm, &p)?;
    Ok(vec![("xoffset".into(), p[4].clone()), ("yoffset".into(), p[5].clone())])
}

fn f_position(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    position4(vm, &args[0])
}

fn f_offsets(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = offsets(vm, &args[0])?;
    py::kwargs_dict(vm, &o)
}

fn motion(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display.motion", name)
}

fn f_move_factory(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (p1, p2, delay, d) = (args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone());
    if ops::eq(vm, &p1, &p2)? {
        return Ok(d);
    }
    let mv = motion(vm, "Move")?;
    vm.call(&mv, vec![p1, p2, delay, d], kw.to_vec())
}

fn aorb(vm: &mut Vm, pos: &Value, pos1: &Value) -> PyResult<Value> {
    let a = py::items(vm, pos)?;
    let b = py::items(vm, pos1)?;
    let out = a.into_iter().zip(b).map(|(a, b)| if py::is_none(&a) { b } else { a }).collect();
    Ok(vm.types.new_tuple(out))
}

fn f_move_in(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let pos = aorb(vm, &args[0], &args[1])?;
    let mv = motion(vm, "Move")?;
    vm.call(&mv, vec![pos, args[1].clone(), args[2].clone(), args[3].clone()], kw.to_vec())
}

fn f_move_out(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let pos = aorb(vm, &args[0], &args[1])?;
    let mv = motion(vm, "Move")?;
    vm.call(&mv, vec![args[1].clone(), pos, args[2].clone(), args[3].clone()], kw.to_vec())
}

fn f_zoom_in_out(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (start, end, pos, delay, d) = (args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone(), args[4].clone());
    let p = py::items(vm, &pos)?;
    let fz = motion(vm, "FactorZoom")?;
    let mut k = kw.to_vec();
    k.push(("opaque".into(), Value::Bool(false)));
    for (n, v) in ["xpos", "ypos", "xanchor", "yanchor"].iter().zip(p) {
        k.push(((*n).into(), v));
    }
    if py::f64_of(vm, &end)? == 1. {
        k.push(("after_child".into(), d.clone()));
    }
    vm.call(&fz, vec![start, end, delay, d], k)
}

fn f_revolve_in_out(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rv = motion(vm, "Revolve")?;
    let mut k = kw.to_vec();
    k.push(("pos".into(), args[2].clone()));
    vm.call(&rv, vec![args[0].clone(), args[1].clone(), args[3].clone(), args[4].clone()], k)
}

// MoveInterpolate --------------------------------------------------------------------------

fn mi_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    super::core::d_init(vm, &[this.clone()], &[])?;
    for (i, n) in ["delay", "old", "new", "use_old", "time_warp"].iter().enumerate() {
        py::set(vm, this, n, args.get(i + 1).cloned().unwrap_or(Value::None))?;
    }
    for n in ["screen_width", "screen_height", "child_width", "child_height", "st"] {
        py::set(vm, this, n, Value::Int(0))?;
    }
    Ok(Value::None)
}

fn mi_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    py::set(vm, this, "screen_width", Value::Float(w))?;
    py::set(vm, this, "screen_height", Value::Float(h))?;
    let old = py::get(vm, this, "old")?;
    let new = py::get(vm, this, "new")?;
    let old_r = render::render(vm, &old, w, h, st, at)?;
    let new_r = render::render(vm, &new, w, h, st, at)?;
    let use_old = { let v = py::get_or_none(vm, this, "use_old")?; ops::truthy(vm, &v)? };
    let cr = if use_old { old_r } else { new_r };
    let (cw, ch) = super::layout::size(&cr);
    py::set(vm, this, "child_width", Value::Float(cw))?;
    py::set(vm, this, "child_height", Value::Float(ch))?;
    py::set(vm, this, "st", Value::Float(st))?;
    let delay = { let v = py::get(vm, this, "delay")?; py::f64_of(vm, &v)? };
    if st < delay {
        render::redraw(vm, this, 0.);
    }
    Ok(cr)
}

fn child_placement(vm: &mut Vm, this: &Value, child: &Value) -> PyResult<[f64; 6]> {
    let g = |vm: &mut Vm, n: &str| -> PyResult<f64> { let v = py::get_or_none(vm, this, n)?; if py::is_none(&v) { Ok(0.) } else { py::f64_of(vm, &v) } };
    let (sw, sh, cw, ch) = (g(vm, "screen_width")?, g(vm, "screen_height")?, g(vm, "child_width")?, g(vm, "child_height")?);
    let p = vm.call_method(child, "get_placement", Vec::new())?;
    let p = py::items(vm, &p)?;
    let based = |vm: &mut Vm, v: &Value, base: f64| -> PyResult<f64> { if py::is_none(v) { Ok(0.) } else { compute_raw(vm, v, base) } };
    let num = |vm: &mut Vm, v: &Value| -> PyResult<f64> { if py::is_none(v) { Ok(0.) } else { py::f64_of(vm, v) } };
    Ok([based(vm, &p[0], sw)?, based(vm, &p[1], sh)?, based(vm, &p[2], cw)?, based(vm, &p[3], ch)?, num(vm, &p[4])?, num(vm, &p[5])?])
}

fn mi_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = { let v = py::get_or_none(vm, this, "st")?; py::f64_of(vm, &v).unwrap_or(0.) };
    let delay = { let v = py::get(vm, this, "delay")?; py::f64_of(vm, &v)? };
    let mut done = if st > delay || delay == 0. { 1. } else { st / delay };
    let tw = py::get_or_none(vm, this, "time_warp")?;
    if !py::is_none(&tw) {
        let r = vm.call(&tw, vec![Value::Float(done)], Vec::new())?;
        done = py::f64_of(vm, &r)?;
    }
    let old = py::get(vm, this, "old")?;
    let new = py::get(vm, this, "new")?;
    let a = child_placement(vm, this, &old)?;
    let b = child_placement(vm, this, &new)?;
    let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
    let mut out = Vec::new();
    for i in 0..6 {
        out.push(vm.call(&abs, vec![Value::Float(a[i] + done * (b[i] - a[i]))], Vec::new())?);
    }
    let op = vm.call_method(&old, "get_placement", Vec::new())?;
    let np = vm.call_method(&new, "get_placement", Vec::new())?;
    let os = py::items(vm, &op)?.get(6).cloned().unwrap_or(Value::Bool(false));
    let ns = py::items(vm, &np)?.get(6).cloned().unwrap_or(Value::Bool(false));
    out.push(Value::Bool(ops::truthy(vm, &os)? || ops::truthy(vm, &ns)?));
    Ok(vm.types.new_tuple(out))
}

fn mi_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = py::get(vm, &args[0], "old")?;
    let n = py::get(vm, &args[0], "new")?;
    Ok(vm.types.new_list(vec![o, n]))
}

// Scene-list merge ---------------------------------------------------------------------------

struct Opts {
    delay: Value,
    enter: Value,
    leave: Value,
    use_old: bool,
    layers: Value,
    time_warp: Value,
    enter_time_warp: Value,
    leave_time_warp: Value,
    old_style: Option<(Value, Value, Value)>,
}

fn layout_attr(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display.layout", n)
}

fn is_multibox(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    let mb = layout_attr(vm, "MultiBox")?;
    Ok(mb.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

fn new_fixed_box(vm: &mut Vm, kw: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<Value> {
    let mb = layout_attr(vm, "MultiBox")?;
    let mut k = kw;
    k.push(("layout".into(), vm.types.new_str("fixed")));
    vm.call(&mb, Vec::new(), k)
}

fn move_interpolate(vm: &mut Vm, o: &Opts, old: Value, new: Value, use_old: bool, tw: &Value) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "MoveInterpolate")?;
    vm.call(&c, vec![o.delay.clone(), old, new, Value::Bool(use_old), tw.clone()], Vec::new())
}

fn wrap(vm: &mut Vm, sle: &Value) -> PyResult<Value> {
    let at = layout_attr(vm, "AdjustTimes")?;
    let d = py::get(vm, sle, "displayable")?;
    let s = py::get_or_none(vm, sle, "show_time")?;
    let a = py::get_or_none(vm, sle, "animation_time")?;
    vm.call(&at, vec![d, s, a], Vec::new())
}

fn tag_of(vm: &mut Vm, sle: &Value) -> PyResult<Value> {
    let t = py::get_or_none(vm, sle, "tag")?;
    if ops::truthy(vm, &t)? { Ok(t) } else { py::get(vm, sle, "displayable") }
}

fn merge(vm: &mut Vm, sle: &Value, d: Value, old_style: bool) -> PyResult<Value> {
    let rv = vm.call_method(sle, "copy", Vec::new())?;
    py::set(vm, &rv, "show_time", if old_style { Value::None } else { Value::Int(0) })?;
    py::set(vm, &rv, "displayable", d)?;
    Ok(rv)
}

fn key_of(vm: &mut Vm, t: &Value) -> PyResult<String> {
    Ok(match t.as_str() { Some(s) => format!("s:{s}"), None => format!("d:{}", t.id()) })
}

fn merge_slide(vm: &mut Vm, o: &Opts, old: &Value, new: &Value) -> PyResult<Value> {
    let new_layers = py::get_or_none(vm, new, "layers")?;
    let new_layer_name = py::get_or_none(vm, new, "layer_name")?;
    if !is_multibox(vm, new)? || (py::is_none(&new_layers) && py::is_none(&new_layer_name)) {
        if let Some((factory, _, _)) = &o.old_style {
            let child = if o.use_old { old.clone() } else { new.clone() };
            let op = position4(vm, old)?;
            let np = position4(vm, new)?;
            if !ops::eq(vm, &op, &np)? {
                let k = offsets(vm, &child)?;
                return vm.call(factory, vec![op, np, o.delay.clone(), child], k);
            }
            return Ok(child);
        }
        if py::same(old, new) {
            return Ok(new.clone());
        }
        return move_interpolate(vm, o, old.clone(), new.clone(), o.use_old, &o.time_warp);
    }
    if ops::truthy(vm, &new_layers)? {
        let rv = new_fixed_box(vm, Vec::new())?;
        let layers = vm.types.new_dict();
        py::set(vm, &rv, "layers", layers.clone())?;
        let old_layers = py::get(vm, old, "layers")?;
        let cfg_layers = py::config(vm, "layers")?;
        for layer in py::items(vm, &cfg_layers)? {
            let nl = ops::getitem(vm, &new_layers, layer.clone())?;
            let ol = ops::getitem(vm, &old_layers, layer.clone())?;
            let d = if o.old_style.is_some() {
                let sl = py::get_or_none(vm, &nl, "scene_list")?;
                if is_multibox(vm, &nl)? && ops::contains(vm, &o.layers, &layer)? && !py::is_none(&sl) { merge_slide(vm, o, &ol, &nl)? } else { nl }
            } else {
                merge_slide(vm, o, &ol, &nl)?
            };
            ops::setitem(vm, &layers, layer, d.clone())?;
            if o.old_style.is_some() {
                vm.call_method(&rv, "add", vec![d])?;
            } else {
                vm.call_method(&rv, "add", vec![d, Value::Bool(true), Value::Bool(true)])?;
            }
        }
        return Ok(rv);
    }
    let mut old = old.clone();
    let mut new = new.clone();
    if o.old_style.is_none() {
        let ou = py::get_or_none(vm, &old, "untransformed_layer")?;
        if ops::truthy(vm, &ou)? {
            old = ou;
        }
        let nu = py::get_or_none(vm, &new, "untransformed_layer")?;
        if ops::truthy(vm, &nu)? {
            let mut rv = new.clone();
            new = nu;
            let layer = py::get_or_none(vm, &new, "layer_name")?;
            let sl = py::get_or_none(vm, &new, "scene_list")?;
            if is_multibox(vm, &new)? && ops::contains(vm, &o.layers, &layer)? && !py::is_none(&sl) {
                let d = merge_slide(vm, o, &old, &new)?;
                let at = layout_attr(vm, "AdjustTimes")?;
                let adjust = vm.call(&at, vec![d.clone(), Value::None, Value::None], Vec::new())?;
                let ctx = py::import_attr(vm, "renpy.game", "context")?;
                let c = vm.call(&ctx, Vec::new(), Vec::new())?;
                let sls = py::get(vm, &c, "scene_lists")?;
                rv = vm.call_method(&sls, "transform_layer", vec![layer, adjust.clone()])?;
                if py::same(&rv, &adjust) {
                    rv = d;
                } else {
                    let mt = layout_attr(vm, "MatchTimes")?;
                    rv = vm.call(&mt, vec![rv, adjust], Vec::new())?;
                }
            }
            return Ok(rv);
        }
    }
    let old_sl = py::get_items(vm, &old, "scene_list")?;
    let new_sl = py::get_items(vm, &new, "scene_list")?;
    let mut old_map = std::collections::HashMap::new();
    for i in &old_sl {
        if !py::is_none(i) {
            let t = tag_of(vm, i)?;
            let k = key_of(vm, &t)?;
            old_map.insert(k, i.clone());
        }
    }
    let mut new_tags = std::collections::HashSet::new();
    for i in &new_sl {
        if !py::is_none(i) {
            let t = tag_of(vm, i)?;
            new_tags.insert(key_of(vm, &t)?);
        }
    }
    let mut rv_sl: Vec<Value> = Vec::new();
    let mut rv_tags = std::collections::HashSet::new();
    let (mut oi, mut ni) = (0, 0);
    let old_style = o.old_style.is_some();
    while oi < old_sl.len() || ni < new_sl.len() {
        if oi < old_sl.len() {
            let ose = old_sl[oi].clone();
            let ot = tag_of(vm, &ose)?;
            let ok = key_of(vm, &ot)?;
            if rv_tags.contains(&ok) {
                oi += 1;
                continue;
            }
            if !new_tags.contains(&ok) {
                // leaving
                let od = wrap(vm, &ose)?;
                let mv = if let Some((_, _, leave_factory)) = &o.old_style {
                    let p = position4(vm, &od)?;
                    let k = offsets(vm, &od)?;
                    vm.call(leave_factory, vec![p, o.delay.clone(), od.clone()], k)?
                } else if ops::truthy(vm, &o.leave)? {
                    let at = py::import_attr(vm, "store", "At")?;
                    let tgt = vm.call(&at, vec![od.clone(), o.leave.clone()], Vec::new())?;
                    move_interpolate(vm, o, od.clone(), tgt, true, &o.leave_time_warp)?
                } else {
                    Value::None
                };
                if !py::is_none(&mv) {
                    let ie = layout_attr(vm, "IgnoresEvents")?;
                    let mv = vm.call(&ie, vec![mv], Vec::new())?;
                    rv_sl.push(merge(vm, &ose, mv, old_style)?);
                }
                rv_tags.insert(ok);
                oi += 1;
                continue;
            }
        }
        if ni >= new_sl.len() {
            break;
        }
        let nse = new_sl[ni].clone();
        ni += 1;
        let nt = tag_of(vm, &nse)?;
        let nk = key_of(vm, &nt)?;
        if let Some(ose) = old_map.get(&nk).cloned() {
            let odisp = py::get(vm, &ose, "displayable")?;
            let ndisp = py::get(vm, &nse, "displayable")?;
            if !old_style && py::same(&odisp, &ndisp) {
                let w = wrap(vm, &nse)?;
                rv_sl.push(merge(vm, &nse, w, old_style)?);
            } else {
                let od = wrap(vm, &ose)?;
                let nd = wrap(vm, &nse)?;
                let mv = if let Some((factory, _, _)) = &o.old_style {
                    let child = if o.use_old { od.clone() } else { nd.clone() };
                    let a = position4(vm, &od)?;
                    let b = position4(vm, &nd)?;
                    let k = offsets(vm, &child)?;
                    vm.call(factory, vec![a, b, o.delay.clone(), child], k)?
                } else {
                    move_interpolate(vm, o, od, nd, o.use_old, &o.time_warp)?
                };
                if !py::is_none(&mv) {
                    rv_sl.push(merge(vm, &nse, mv, old_style)?);
                }
            }
            rv_tags.insert(nk);
        } else {
            let nd = wrap(vm, &nse)?;
            let mv = if let Some((_, enter_factory, _)) = &o.old_style {
                let p = position4(vm, &nd)?;
                let k = offsets(vm, &nd)?;
                vm.call(enter_factory, vec![p, o.delay.clone(), nd.clone()], k)?
            } else if ops::truthy(vm, &o.enter)? {
                let at = py::import_attr(vm, "store", "At")?;
                let src = vm.call(&at, vec![nd.clone(), o.enter.clone()], Vec::new())?;
                move_interpolate(vm, o, src, nd.clone(), false, &o.enter_time_warp)?
            } else {
                Value::None
            };
            if !py::is_none(&mv) {
                rv_sl.push(merge(vm, &nse, mv, old_style)?);
            }
            rv_tags.insert(nk);
        }
    }
    let mut keyed = Vec::new();
    for s in rv_sl {
        let z = py::get_or_none(vm, &s, "zorder")?;
        keyed.push((py::f64_of(vm, &z).unwrap_or(0.), s));
    }
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let layer = py::get_or_none(vm, &new, "layer_name")?;
    let iface = py::import_attr(vm, "renpy.game", "interface")?;
    let lp = py::get_or_none(vm, &iface, "layer_properties")?;
    let mut kw = vec![("focus".into(), layer.clone())];
    if !py::is_none(&lp) {
        let props = vm.call_method(&lp, "get", vec![layer.clone(), vm.types.new_dict()])?;
        kw.extend(py::str_items(vm, &props)?);
    }
    let rv = new_fixed_box(vm, kw)?;
    let l = vm.types.new_list(keyed.into_iter().map(|k| k.1).collect());
    vm.call_method(&rv, "append_scene_list", vec![l])?;
    if old_style {
        py::set(vm, &rv, "layer_name", layer)?;
    }
    Ok(rv)
}

fn default_layers(vm: &mut Vm) -> Value {
    vm.types.new_list(vec![vm.types.new_str("master")])
}

fn f_move_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = |i: usize, n: &str, d: Value| arg_or(args, kw, i, n, d);
    let delay = a(0, "delay", Value::Int(0));
    let old_widget = a(1, "old_widget", Value::None);
    let new_widget = a(2, "new_widget", Value::None);
    let layers = a(6, "layers", default_layers(vm));
    let o = Opts {
        delay: delay.clone(),
        enter: a(3, "enter", Value::None),
        leave: a(4, "leave", Value::None),
        use_old: ops::truthy(vm, &a(5, "old", Value::Bool(false)))?,
        layers,
        time_warp: a(7, "time_warp", Value::None),
        enter_time_warp: a(8, "enter_time_warp", Value::None),
        leave_time_warp: a(9, "leave_time_warp", Value::None),
        old_style: None,
    };
    if py::config_bool(vm, "developer").unwrap_or(false) {
        for w in [&old_widget, &new_widget] {
            if py::get_opt(vm, w, "scene_list")?.is_none() && py::get_opt(vm, w, "layers")?.is_none() {
                let n = w.obj().map(|o| o.class_name()).unwrap_or_default();
                return Err(py::error(vm, format!("MoveTransition can only be applied to one or all layers, not {n}.")));
            }
        }
    }
    let rv = merge_slide(vm, &o, &old_widget, &new_widget)?;
    py::set(vm, &rv, "delay", delay)?;
    Ok(rv)
}

fn f_old_move_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = |i: usize, n: &str, d: Value| arg_or(args, kw, i, n, d);
    let delay = a(0, "delay", Value::Int(0));
    let old_widget = a(1, "old_widget", Value::None);
    let new_widget = a(2, "new_widget", Value::None);
    let mut factory = a(3, "factory", Value::None);
    let mut enter = a(4, "enter_factory", Value::None);
    let mut leave = a(5, "leave_factory", Value::None);
    if py::is_none(&factory) {
        factory = py::import_attr(vm, MODULE, "MoveFactory")?;
    }
    if py::is_none(&enter) {
        enter = py::import_attr(vm, MODULE, "default_enter_factory")?;
    }
    if py::is_none(&leave) {
        leave = py::import_attr(vm, MODULE, "default_leave_factory")?;
    }
    let layers = a(7, "layers", default_layers(vm));
    let o = Opts {
        delay: delay.clone(),
        enter: Value::None,
        leave: Value::None,
        use_old: ops::truthy(vm, &a(6, "old", Value::Bool(false)))?,
        layers,
        time_warp: Value::None,
        enter_time_warp: Value::None,
        leave_time_warp: Value::None,
        old_style: Some((factory, enter, leave)),
    };
    let rv = merge_slide(vm, &o, &old_widget, &new_widget)?;
    py::set(vm, &rv, "delay", delay)?;
    Ok(rv)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("position", f_position),
        ("offsets", f_offsets),
        ("MoveFactory", f_move_factory),
        ("default_enter_factory", |_, a, _| Ok(a.get(2).cloned().unwrap_or(Value::None))),
        ("default_leave_factory", |_, _, _| Ok(Value::None)),
        ("MoveIn", f_move_in),
        ("MoveOut", f_move_out),
        ("ZoomInOut", f_zoom_in_out),
        ("RevolveInOut", f_revolve_in_out),
        ("OldMoveTransition", f_old_move_transition),
        ("MoveTransition", f_move_transition),
    ]);
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = py::class(vm, MODULE, "MoveInterpolate", &[d], &[("__init__", mi_init), ("render", mi_render), ("get_placement", mi_get_placement), ("visit", mi_visit)], &[])?;
    crysirror_python::modules::set(vm, &m, "MoveInterpolate", Value::Obj(c));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
