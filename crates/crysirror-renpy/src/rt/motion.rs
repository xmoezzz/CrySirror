// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.motion`: the pre-ATL motion displayables (Motion, Pan,
//! Move, Revolve, Zoom family, RotoZoom) and the Transform re-exports,
//! after Ren'Py 8.3 `motion.py` (MIT).
use super::{
    core::compute_raw,
    layout::{rargs, size, style},
    py::{self, *},
    render::{self, Mat, RenderData, new_render},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.motion";

fn less_updates(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.game", "less_updates").map(|v| ops::truthy(vm, &v).unwrap_or(false)).unwrap_or(false)
}

fn fattr(vm: &mut Vm, this: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, this, n)?;
    if py::is_none(&v) { Ok(0.) } else { py::f64_of(vm, &v) }
}

fn battr(vm: &mut Vm, this: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, this, n)?;
    ops::truthy(vm, &v)
}

// Motion -----------------------------------------------------------------------------------

fn motion_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["function", "period", "child", "new_widget", "old_widget", "repeat", "bounce", "delay", "anim_timebase", "tag_start", "time_warp", "add_sizes", "style"];
    let mut vals: Vec<Value> = vec![Value::None; names.len()];
    for (i, d) in [(5, false), (6, false), (8, false), (11, false)] {
        vals[i] = Value::Bool(d);
    }
    vals[12] = vm.types.new_str("motion");
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
    let child = if py::is_none(&vals[2]) { vals[3].clone() } else { vals[2].clone() };
    let repeat = ops::truthy(vm, &vals[5])?;
    let delay = if py::is_none(&vals[7]) && !repeat { vals[1].clone() } else { vals[7].clone() };
    props.push(("style".into(), vals[12].clone()));
    super::layout::container_init(vm, &[this.clone()], &props)?;
    if !py::is_none(&child) {
        vm.call_method(this, "add", vec![child])?;
    }
    for (n, v) in [("function", &vals[0]), ("period", &vals[1]), ("repeat", &vals[5]), ("bounce", &vals[6]), ("delay", &delay), ("anim_timebase", &vals[8]), ("time_warp", &vals[10]), ("add_sizes", &vals[11])] {
        py::set(vm, this, n, v.clone())?;
    }
    py::set(vm, this, "position", Value::None)?;
    Ok(Value::None)
}

fn update_position(vm: &mut Vm, this: &Value, mut t: f64, sizes: Value) -> PyResult<()> {
    let delay = fattr(vm, this, "delay")?;
    let repeat = battr(vm, this, "repeat")?;
    let period = fattr(vm, this, "period")?;
    if less_updates(vm) {
        if delay != 0. {
            t = delay;
            if repeat {
                t %= period;
            }
        } else {
            t = period;
        }
    } else if delay != 0. && t >= delay {
        t = delay;
        if repeat {
            t %= period;
        }
    } else if repeat {
        t %= period;
        render::redraw(vm, this, 0.);
    } else if t > period {
        t = period;
    } else {
        render::redraw(vm, this, 0.);
    }
    t = if period > 0. { t / period } else { 1. };
    let tw = py::get_or_none(vm, this, "time_warp")?;
    if !py::is_none(&tw) {
        let r = vm.call(&tw, vec![Value::Float(t)], Vec::new())?;
        t = py::f64_of(vm, &r)?;
    }
    if battr(vm, this, "bounce")? {
        t *= 2.;
        if t > 1. {
            t = 2. - t;
        }
    }
    let f = py::get(vm, this, "function")?;
    let res = if battr(vm, this, "add_sizes")? { vm.call(&f, vec![Value::Float(t), sizes], Vec::new())? } else { vm.call(&f, vec![Value::Float(t)], Vec::new())? };
    let mut r = py::items(vm, &res)?;
    if r.len() == 2 {
        let xa = style(vm, this, "xanchor")?;
        let ya = style(vm, this, "yanchor")?;
        r.push(if ops::truthy(vm, &xa)? { xa } else { Value::Int(0) });
        r.push(if ops::truthy(vm, &ya)? { ya } else { Value::Int(0) });
    }
    py::set(vm, this, "position", vm.types.new_tuple(r))?;
    Ok(())
}

fn motion_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pos = py::get_or_none(vm, this, "position")?;
    if py::is_none(&pos) {
        if battr(vm, this, "add_sizes")? {
            let s = py::get(vm, this, "style")?;
            return vm.call_method(&s, "get_placement", Vec::new());
        }
        update_position(vm, this, 0., Value::None)?;
    }
    let mut p = py::get_items(vm, this, "position")?;
    for n in ["xoffset", "yoffset", "subpixel"] {
        p.push(style(vm, this, n)?);
    }
    Ok(vm.types.new_tuple(p))
}

fn motion_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let t = if battr(vm, this, "anim_timebase")? { at } else { st };
    let child = py::get(vm, this, "child")?;
    let cr = render::render(vm, &child, w, h, st, at)?;
    let (cw, ch) = size(&cr);
    let sizes = vm.types.new_tuple(vec![Value::Float(w), Value::Float(h), Value::Float(cw), Value::Float(ch)]);
    update_position(vm, this, t, sizes)?;
    let rv = super::layout::new(vm, cw, ch);
    super::layout::blit(vm, &rv, &cr, 0., 0.)?;
    let o = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    py::set(vm, this, "offsets", vm.types.new_list(vec![o]))?;
    Ok(rv)
}

// Interpolate / Revolver ------------------------------------------------------------------------

fn anchor_name(vm: &mut Vm, v: &Value) -> Value {
    match v.as_str() {
        Some("top") | Some("left") => Value::Float(0.),
        Some("center") => Value::Float(0.5),
        Some("bottom") | Some("right") => Value::Float(1.),
        _ => {
            let _ = vm;
            v.clone()
        }
    }
}

fn interp_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::items(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let e = py::items(vm, &args.get(2).cloned().unwrap_or(Value::None))?;
    if s.len() != e.len() {
        return Err(py::error(vm, "The start and end must have the same number of arguments."));
    }
    let s: Vec<Value> = s.iter().map(|v| anchor_name(vm, v)).collect();
    let e: Vec<Value> = e.iter().map(|v| anchor_name(vm, v)).collect();
    py::set(vm, this, "start", vm.types.new_list(s))?;
    py::set(vm, this, "end", vm.types.new_list(e))?;
    Ok(Value::None)
}

fn interp_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    let s = py::get_items(vm, this, "start")?;
    let e = py::get_items(vm, this, "end")?;
    let st = vm.types.new_tuple(s);
    let et = vm.types.new_tuple(e);
    let pon = py::import_attr(vm, "renpy.atl", "position_or_none")?;
    super::atlrt::interpolate(vm, t, &st, &et, &pon)
}

fn revolver_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let half = || Value::Float(0.5);
    for (i, (n, d)) in [("start", None), ("end", None), ("child", None), ("around", Some(())), ("cor", Some(())), ("pos", None)].iter().enumerate() {
        let v = match arg(args, kw, i + 1, n) {
            Some(v) => v.clone(),
            None if d.is_some() => vm.types.new_tuple(vec![half(), half()]),
            None => Value::None,
        };
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn revolver_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    let rect = py::items(vm, &args.get(2).cloned().unwrap_or(Value::None))?;
    let r: Vec<f64> = rect.iter().map(|v| py::f64_of(vm, v)).collect::<PyResult<_>>()?;
    let (w, h, cw, ch) = (r[0], r[1], r[2], r[3]);
    let fti = |vm: &mut Vm, x: &Value, room: f64| -> PyResult<f64> { if py::is_none(x) { Ok(0.) } else { compute_raw(vm, x, room) } };
    let pos = py::get_or_none(vm, this, "pos")?;
    let pos = if py::is_none(&pos) { let c = py::get(vm, this, "child")?; vm.call_method(&c, "get_placement", Vec::new())? } else { pos };
    let p = py::items(vm, &pos)?;
    let xpos = fti(vm, &p[0], w)?;
    let ypos = fti(vm, &p[1], h)?;
    let xanchor = fti(vm, &p[2], cw)?;
    let yanchor = fti(vm, &p[3], ch)?;
    let around = py::get_items(vm, this, "around")?;
    let xa = fti(vm, &around[0], w)?;
    let ya = fti(vm, &around[1], h)?;
    let cor = py::get_items(vm, this, "cor")?;
    let xc = fti(vm, &cor[0], cw)?;
    let yc = fti(vm, &cor[1], ch)?;
    let s = fattr(vm, this, "start")?;
    let e = fattr(vm, this, "end")?;
    let angle = (s + (e - s) * t).to_radians();
    let x = xpos - xanchor + xc - xa;
    let y = ypos - yanchor + yc - ya;
    let nx = x * angle.cos() - y * angle.sin() - xc + xa;
    let ny = x * angle.sin() + y * angle.cos() - yc + ya;
    let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
    let ax = vm.call(&abs, vec![Value::Float(nx)], Vec::new())?;
    let ay = vm.call(&abs, vec![Value::Float(ny)], Vec::new())?;
    Ok(vm.types.new_tuple(vec![ax, ay, Value::Int(0), Value::Int(0)]))
}

fn motion_call(vm: &mut Vm, f: Value, time: Value, child: Value, mut kw: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<Value> {
    let m = py::import_attr(vm, MODULE, "Motion")?;
    kw.insert(0, ("child".into(), child));
    vm.call(&m, vec![f, time], kw)
}

fn split_kw(kw: Kw, take: &[&str]) -> (Vec<(std::rc::Rc<str>, Value)>, Vec<(std::rc::Rc<str>, Value)>) {
    kw.iter().cloned().partition(|(k, _)| take.contains(&&**k))
}

fn f_pan_move(vm: &mut Vm, args: &[Value], kw: Kw, pan: bool) -> PyResult<Value> {
    let sp = arg_or(args, kw, 0, "startpos", Value::None);
    let ep = arg_or(args, kw, 1, "endpos", Value::None);
    let time = arg_or(args, kw, 2, "time", Value::None);
    let child = arg_or(args, kw, 3, "child", Value::None);
    let (_, rest) = split_kw(kw, &["startpos", "endpos", "time", "child"]);
    let (s, e) = if pan {
        let s = py::items(vm, &sp)?;
        let e = py::items(vm, &ep)?;
        let neg = |vm: &mut Vm, v: &Value| ops::unary_op(vm, v, crysirror_python::ast::UnaryOp::USub);
        let s = vec![neg(vm, &s[0])?, neg(vm, &s[1])?];
        let e = vec![neg(vm, &e[0])?, neg(vm, &e[1])?];
        (vm.types.new_tuple(s), vm.types.new_tuple(e))
    } else {
        (sp, ep)
    };
    let ic = py::import_attr(vm, MODULE, "Interpolate")?;
    let f = vm.call(&ic, vec![s, e], Vec::new())?;
    let mut rest = rest;
    for (i, n) in ["repeat", "bounce", "anim_timebase", "style", "time_warp"].iter().enumerate() {
        if let Some(v) = args.get(4 + i) {
            rest.push(((*n).into(), v.clone()));
        }
    }
    if !rest.iter().any(|(k, _)| &**k == "style") {
        rest.push(("style".into(), vm.types.new_str("motion")));
    }
    motion_call(vm, f, time, child, rest)
}

fn f_pan(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    f_pan_move(vm, args, kw, true)
}

fn f_move(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    f_pan_move(vm, args, kw, false)
}

fn f_revolve(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let start = arg_or(args, kw, 0, "start", Value::None);
    let end = arg_or(args, kw, 1, "end", Value::None);
    let time = arg_or(args, kw, 2, "time", Value::None);
    let child = arg_or(args, kw, 3, "child", Value::None);
    let (rk, mut rest) = split_kw(kw, &["around", "cor", "pos"]);
    rest.retain(|(k, _)| !matches!(&**k, "start" | "end" | "time" | "child"));
    let rc = py::import_attr(vm, MODULE, "Revolver")?;
    let f = vm.call(&rc, vec![start, end, child.clone()], rk)?;
    rest.push(("add_sizes".into(), Value::Bool(true)));
    motion_call(vm, f, time, child, rest)
}

// Zoom family ------------------------------------------------------------------------------------

fn zc_init(vm: &mut Vm, this: &Value, vals: &[(&str, Value)], props: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<()> {
    let get = |n: &str| vals.iter().find(|v| v.0 == n).map(|v| v.1.clone()).unwrap_or(Value::None);
    let mut props = props;
    let style_v = get("style");
    props.push(("style".into(), if py::is_none(&style_v) { vm.types.new_str("motion") } else { style_v }));
    super::core::d_init(vm, &[this.clone()], &props)?;
    let child = super::easy::displayable(vm, &get("child"), &Value::None, true)?;
    py::set(vm, this, "time", get("time"))?;
    py::set(vm, this, "child", child.clone())?;
    py::set(vm, this, "repeat", get("repeat"))?;
    let after = get("after_child");
    let after = if ops::truthy(vm, &after)? {
        super::easy::displayable(vm, &after, &Value::None, true)?
    } else if ops::truthy(vm, &get("end_identity"))? {
        child
    } else {
        Value::None
    };
    py::set(vm, this, "after_child", after)?;
    for n in ["time_warp", "bilinear", "opaque", "anim_timebase"] {
        py::set(vm, this, n, get(n))?;
    }
    Ok(())
}

fn zc_kwargs(vm: &mut Vm, kw: Kw, mut vals: Vec<(&'static str, Value)>) -> (Vec<(&'static str, Value)>, Vec<(std::rc::Rc<str>, Value)>) {
    let known = ["end_identity", "after_child", "time_warp", "bilinear", "opaque", "anim_timebase", "repeat", "style"];
    let _ = vm;
    let mut props = Vec::new();
    for (k, v) in kw {
        match known.iter().find(|n| **n == &**k) {
            Some(n) => vals.push((n, v.clone())),
            None => props.push((k.clone(), v.clone())),
        }
    }
    for (n, d) in [("bilinear", true), ("opaque", true), ("anim_timebase", false), ("repeat", false)] {
        if !vals.iter().any(|v| v.0 == n) {
            vals.push((n, Value::Bool(d)));
        }
    }
    (vals, props)
}

fn zoom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = |i: usize, n: &str| arg_or(args, kw, i, n, Value::None);
    let (size, start, end, time, child) = (a(1, "size"), a(2, "start"), a(3, "end"), a(4, "time"), a(5, "child"));
    let kw2: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "size" | "start" | "end" | "time" | "child")).cloned().collect();
    let sz = py::items(vm, &size)?;
    let mut ident = vec![Value::Float(0.), Value::Float(0.)];
    ident.extend(sz.clone());
    let it = vm.types.new_tuple(ident);
    let end_identity = ops::eq(vm, &end, &it)?;
    let (vals, props) = zc_kwargs(vm, &kw2, vec![("time", time), ("child", child), ("end_identity", Value::Bool(end_identity))]);
    zc_init(vm, this, &vals, props)?;
    py::set(vm, this, "size", size)?;
    py::set(vm, this, "start", start)?;
    py::set(vm, this, "end", end)?;
    Ok(Value::None)
}

fn factorzoom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = |i: usize, n: &str| arg_or(args, kw, i, n, Value::None);
    let (start, end, time, child) = (a(1, "start"), a(2, "end"), a(3, "time"), a(4, "child"));
    let kw2: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "start" | "end" | "time" | "child")).cloned().collect();
    let ei = py::f64_of(vm, &end)? == 1.;
    let (vals, props) = zc_kwargs(vm, &kw2, vec![("time", time), ("child", child), ("end_identity", Value::Bool(ei))]);
    zc_init(vm, this, &vals, props)?;
    py::set(vm, this, "start", start)?;
    py::set(vm, this, "end", end)?;
    Ok(Value::None)
}

fn sizezoom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = |i: usize, n: &str| arg_or(args, kw, i, n, Value::None);
    let (start, end, time, child) = (a(1, "start"), a(2, "end"), a(3, "time"), a(4, "child"));
    let kw2: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "start" | "end" | "time" | "child")).cloned().collect();
    let (vals, props) = zc_kwargs(vm, &kw2, vec![("time", time), ("child", child), ("end_identity", Value::Bool(false))]);
    zc_init(vm, this, &vals, props)?;
    py::set(vm, this, "start", start)?;
    py::set(vm, this, "end", end)?;
    Ok(Value::None)
}

fn zoom_rect(vm: &mut Vm, this: &Value, done: f64, w: f64, h: f64) -> PyResult<[f64; 6]> {
    let cls = this.obj().map(|o| o.class_name()).unwrap_or_default();
    match cls.as_str() {
        "Zoom" => {
            let s = py::get_items(vm, this, "start")?;
            let e = py::get_items(vm, this, "end")?;
            let sz = py::get_items(vm, this, "size")?;
            let mut r = [0.; 6];
            for i in 0..4 {
                let a = py::f64_of(vm, &s[i])?;
                let b = py::f64_of(vm, &e[i])?;
                r[i] = a + (b - a) * done;
            }
            r[4] = py::f64_of(vm, &sz[0])?;
            r[5] = py::f64_of(vm, &sz[1])?;
            Ok(r)
        }
        "FactorZoom" => {
            let s = fattr(vm, this, "start")?;
            let e = fattr(vm, this, "end")?;
            let f = s + (e - s) * done;
            Ok([0., 0., w, h, f * w, f * h])
        }
        "SizeZoom" => {
            let s = py::get_items(vm, this, "start")?;
            let e = py::get_items(vm, this, "end")?;
            let (sw, sh) = (py::f64_of(vm, &s[0])?, py::f64_of(vm, &s[1])?);
            let (ew, eh) = (py::f64_of(vm, &e[0])?, py::f64_of(vm, &e[1])?);
            Ok([0., 0., w, h, sw + (ew - sw) * done, sh + (eh - sh) * done])
        }
        _ => {
            let r = vm.call_method(this, "zoom_rectangle", vec![Value::Float(done), Value::Float(w), Value::Float(h)])?;
            let r = py::items(vm, &r)?;
            let mut out = [0.; 6];
            for (i, v) in r.iter().take(6).enumerate() {
                out[i] = py::f64_of(vm, v)?;
            }
            Ok(out)
        }
    }
}

fn zc_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let t = if battr(vm, this, "anim_timebase")? { at } else { st };
    let time = fattr(vm, this, "time")?;
    let mut done = if time != 0. { (t / time).min(1.) } else { 1. };
    if battr(vm, this, "repeat")? {
        done %= 1.;
    }
    if less_updates(vm) {
        done = 1.;
    }
    py::set(vm, this, "done", Value::Float(done))?;
    let after = py::get_or_none(vm, this, "after_child")?;
    if !py::is_none(&after) && done == 1. {
        return render::render(vm, &after, w, h, st, at);
    }
    let tw = py::get_or_none(vm, this, "time_warp")?;
    let mut d2 = done;
    if !py::is_none(&tw) {
        let r = vm.call(&tw, vec![Value::Float(done)], Vec::new())?;
        d2 = py::f64_of(vm, &r)?;
    }
    let child = py::get(vm, this, "child")?;
    let rend = render::render(vm, &child, w, h, st, at)?;
    let (rw, rh) = size(&rend);
    let [rx, ry, zw0, zh0, zw, zh] = { let r = zoom_rect(vm, this, d2, rw, rh)?; [r[0], r[1], r[2], r[3], r[4], r[5]] };
    if rx < 0. || ry < 0. || rx + zw0 > rw || ry + zh0 > rh {
        return Err(py::error(vm, format!("Zoom rectangle ({rx}, {ry}, {zw0}, {zh0}) falls outside of {rw}x{rh} parent surface.")));
    }
    let mut rd = RenderData::new(zw, zh);
    if zw != 0. && zh != 0. && zw0 != 0. && zh0 != 0. {
        rd.forward = Some(Mat::scale(zw0 / zw, zh0 / zh, 1.));
        let rev = Mat::scale(zw / zw0, zh / zh0, 1.);
        rd.reverse = Some(rev);
        rd.xclipping = true;
        rd.yclipping = true;
        let (bx, by) = rev.transform2(-rx, -ry);
        let src = render::source_of(vm, &rend)?;
        rd.children.push(render::Blit { source: src, x: bx.trunc(), y: by.trunc(), focus: true, main: true });
    }
    if done < 1. {
        render::redraw(vm, this, 0.);
    }
    Ok(new_render(vm, rd))
}

fn zc_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    let a = py::get_or_none(vm, &args[0], "after_child")?;
    Ok(vm.types.new_list(vec![c, a]))
}

fn zc_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = py::f64_of(vm, &args.get(4).cloned().unwrap_or(Value::Int(0)))?;
    let time = fattr(vm, this, "time")?;
    let done = if time == 0. { 1. } else { (st / time).min(1.) };
    let after = py::get_or_none(vm, this, "after_child")?;
    if done == 1. && !py::is_none(&after) {
        return vm.call_method(&after, "event", args[1..].to_vec());
    }
    Ok(Value::None)
}

fn zc_zoom_rectangle(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Zoom rectangle not implemented."))
}

// RotoZoom ----------------------------------------------------------------------------------------

fn rotozoom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["rot_start", "rot_end", "rot_delay", "zoom_start", "zoom_end", "zoom_delay", "child", "rot_repeat", "zoom_repeat", "rot_bounce", "zoom_bounce", "rot_anim_timebase", "zoom_anim_timebase", "rot_time_warp", "zoom_time_warp", "opaque", "style"];
    let mut vals: Vec<Value> = vec![Value::None; names.len()];
    for i in 7..=12 {
        vals[i] = Value::Bool(false);
    }
    vals[15] = Value::Bool(false);
    vals[16] = vm.types.new_str("motion");
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
    props.push(("style".into(), vals[16].clone()));
    super::core::d_init(vm, &[this.clone()], &props)?;
    vals[6] = super::easy::displayable(vm, &vals[6], &Value::None, true)?;
    for (i, n) in names.iter().enumerate().take(16) {
        py::set(vm, this, n, vals[i].clone())?;
    }
    Ok(Value::None)
}

fn rotozoom_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let phase = |vm: &mut Vm, p: &str| -> PyResult<f64> {
        let mut t = if battr(vm, this, &format!("{p}_anim_timebase"))? { at } else { st };
        let delay = fattr(vm, this, &format!("{p}_delay"))?;
        t = if delay == 0. { 1. } else { t / delay };
        if battr(vm, this, &format!("{p}_repeat"))? {
            t %= 1.;
        }
        if battr(vm, this, &format!("{p}_bounce"))? {
            t *= 2.;
            t = t.min(2. - t);
        }
        Ok(t)
    };
    let mut rt = phase(vm, "rot")?;
    let mut zt = phase(vm, "zoom")?;
    if less_updates(vm) {
        rt = 1.;
        zt = 1.;
    }
    rt = rt.min(1.);
    zt = zt.min(1.);
    for (p, t) in [("rot", &mut rt), ("zoom", &mut zt)] {
        let tw = py::get_or_none(vm, this, &format!("{p}_time_warp"))?;
        if !py::is_none(&tw) {
            let r = vm.call(&tw, vec![Value::Float(*t)], Vec::new())?;
            *t = py::f64_of(vm, &r)?;
        }
    }
    let rs = fattr(vm, this, "rot_start")?;
    let re = fattr(vm, this, "rot_end")?;
    let zs = fattr(vm, this, "zoom_start")?;
    let ze = fattr(vm, this, "zoom_end")?;
    let angle = rs + (re - rs) * rt;
    let zoom = (zs + (ze - zs) * zt).max(0.001);
    let mut tr = py::get_or_none(vm, this, "transform")?;
    if py::is_none(&tr) {
        let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
        let child = py::get(vm, this, "child")?;
        tr = vm.call(&tc, vec![child], Vec::new())?;
        py::set(vm, this, "transform", tr.clone())?;
    }
    py::set(vm, &tr, "rotate", Value::Float(angle))?;
    py::set(vm, &tr, "zoom", Value::Float(zoom))?;
    let rv = render::render(vm, &tr, w, h, st, at)?;
    render::redraw(vm, &tr, 0.);
    Ok(rv)
}

fn child_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![c]))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[("Pan", f_pan), ("Move", f_move), ("Revolve", f_revolve)]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in ["Transform", "Proxy", "TransformState", "ATLTransform", "null"] {
        let v = py::import_attr(vm, super::transform::MODULE, n)?;
        set(vm, n, v);
    }
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods, attrs: &[(&str, Value)]| -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    c(vm, "Motion", &[container], &[("__init__", motion_init), ("get_placement", motion_get_placement), ("render", motion_render)], &[])?;
    let anchors = vm.types.new_dict();
    for (k, v) in [("top", 0.), ("center", 0.5), ("bottom", 1.), ("left", 0.), ("right", 1.)] {
        py::dict_set(vm, &anchors, k, Value::Float(v))?;
    }
    c(vm, "Interpolate", &[], &[("__init__", interp_init), ("__call__", interp_call)], &[("anchors", anchors)])?;
    c(vm, "Revolver", &[], &[("__init__", revolver_init), ("__call__", revolver_call)], &[])?;
    let zc = c(vm, "ZoomCommon", &[d.clone()], &[("visit", zc_visit), ("zoom_rectangle", zc_zoom_rectangle), ("render", zc_render), ("event", zc_event)], &[])?;
    c(vm, "Zoom", &[zc.clone()], &[("__init__", zoom_init)], &[])?;
    c(vm, "FactorZoom", &[zc.clone()], &[("__init__", factorzoom_init)], &[])?;
    c(vm, "SizeZoom", &[zc], &[("__init__", sizezoom_init)], &[])?;
    c(vm, "RotoZoom", &[d], &[("__init__", rotozoom_init), ("visit", child_visit), ("render", rotozoom_render)], &[("transform", Value::None)])?;
    // Upstream also publishes these on renpy.display.layout.
    if let Ok(layout) = py::import(vm, "renpy.display.layout") {
        for n in ["Transform", "RotoZoom", "SizeZoom", "FactorZoom", "Zoom", "Revolver", "Motion", "Interpolate", "Revolve", "Move", "Pan"] {
            if let Some(v) = crysirror_python::ops::get_raw_attr(&m, n) {
                py::set(vm, &layout, n, v)?;
            }
        }
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
