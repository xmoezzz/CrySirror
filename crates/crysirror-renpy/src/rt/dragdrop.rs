// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.dragdrop`, after Ren'Py 8.3 `dragdrop.py` (MIT): Drag,
//! DragGroup and the drop-selection helpers.
use super::{
    behavior::{ignore_event, map_name, run},
    core::compute_raw,
    events,
    layout::{rargs, style as sty},
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.dragdrop";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
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

fn int(x: f64) -> Value {
    Value::Int(x.trunc() as i64)
}

fn f_default_drag_group(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let c = vm.call(&ctx, Vec::new(), Vec::new())?;
    let sls = py::get(vm, &c, "scene_lists")?;
    let rv = py::get_or_none(vm, &sls, "drag_group")?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let dg = mget(vm, "DragGroup")?;
    let rv = vm.call(&dg, Vec::new(), Vec::new())?;
    py::set(vm, &sls, "drag_group", rv.clone())?;
    Ok(rv)
}

fn f_default_drag_joined(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = vm
        .types
        .new_tuple(vec![args[0].clone(), Value::Int(0), Value::Int(0)]);
    Ok(vm.types.new_list(vec![t]))
}

fn rect_overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let maxleft = a[0].max(b[0]);
    let minright = (a[0] + a[2]).min(b[0] + b[2]);
    let maxtop = a[1].max(b[1]);
    let minbottom = (a[1] + a[3]).min(b[1] + b[3]);
    if minright < maxleft || minbottom < maxtop {
        return 0.;
    }
    (minright - maxleft) * (minbottom - maxtop)
}

fn f_rect_overlap_area(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = render::rect4(vm, &args[0])?;
    let b = render::rect4(vm, &args[1])?;
    Ok(Value::Float(rect_overlap(a, b)))
}

// Drag ------------------------------------------------------------------------------------------------

const DRAG_NAMES: &[&str] = &[
    "d",
    "drag_name",
    "draggable",
    "droppable",
    "drag_raise",
    "dragged",
    "dropped",
    "drop_allowable",
    "drag_handle",
    "drag_joined",
    "clicked",
    "hovered",
    "unhovered",
    "replaces",
    "drag_offscreen",
    "mouse_drop",
    "activated",
    "alternate",
    "style",
    "dragging",
];

const REPLACED: &[&str] = &[
    "x",
    "y",
    "at",
    "target_x",
    "target_y",
    "target_at",
    "target_at_delay",
    "snap_warper",
    "snap_start",
    "snap_start_x",
    "snap_start_y",
    "snapping",
    "grab_x",
    "grab_y",
    "last_x",
    "last_y",
    "old_position",
    "drag_moved",
    "last_drop",
    "mouse_drop",
    "click_time",
    "z",
];

fn drag_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let handle = vm.types.new_tuple(vec![
        Value::Float(0.),
        Value::Float(0.),
        Value::Float(1.),
        Value::Float(1.),
    ]);
    let mut v = vec![
        Value::None,
        Value::None,
        Value::Bool(true),
        Value::Bool(true),
        Value::Bool(true),
        Value::None,
        Value::None,
        mget(vm, "default_drop_allowable")?,
        handle,
        mget(vm, "default_drag_joined")?,
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
        Value::None,
        Value::None,
        vm.types.new_str("drag"),
        Value::None,
    ];
    for (i, a) in args.iter().skip(1).enumerate().take(DRAG_NAMES.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match DRAG_NAMES.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    let snapped = props
        .iter()
        .find(|(k, _)| &**k == "snapped")
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::None);
    props.push(("style".into(), v[18].clone()));
    super::core::d_init(vm, &[this.clone()], &props)?;
    for (i, n) in DRAG_NAMES.iter().enumerate() {
        if matches!(*n, "d" | "replaces" | "style") {
            continue;
        }
        py::set(vm, this, n, v[i].clone())?;
    }
    py::set(vm, this, "focusable", v[2].clone())?;
    py::set(vm, this, "child", Value::None)?;
    if ops::truthy(vm, &v[1])? {
        let g = f_default_drag_group(vm, &[], &[])?;
        py::set(vm, this, "drag_group", g)?;
    }
    for n in [
        "x",
        "y",
        "w",
        "h",
        "old_position",
        "parent_width",
        "parent_height",
        "target_x",
        "target_y",
        "grab_x",
        "grab_y",
        "last_x",
        "last_y",
        "snap_warper",
        "last_drop",
    ] {
        py::set(vm, this, n, Value::None)?;
    }
    for n in [
        "start_x",
        "start_y",
        "at",
        "target_at",
        "target_at_delay",
        "snap_start",
        "snap_start_x",
        "snap_start_y",
        "z",
    ] {
        py::set(vm, this, n, Value::Int(0))?;
    }
    py::set(vm, this, "snapping", Value::Bool(false))?;
    py::set(vm, this, "snapped", snapped)?;
    py::set(vm, this, "drag_moved", Value::Bool(false))?;
    let dc = mget(vm, "Drag")?;
    let replaces = v[13].clone();
    if dc.obj().is_some_and(|c| ops::isinstance(vm, &replaces, c)) {
        for n in REPLACED {
            let x = py::get_or_none(vm, &replaces, n)?;
            py::set(vm, this, n, x)?;
        }
    }
    if !py::is_none(&v[0]) {
        vm.call_method(this, "add", vec![v[0].clone()])?;
    }
    Ok(Value::None)
}

fn drag_draggable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get_or_none(vm, &args[0], "draggable")
}

fn drag_group_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = py::get_or_none(vm, &args[0], "drag_group_weakref")?;
    if py::is_none(&r) {
        return Ok(Value::None);
    }
    vm.call(&r, Vec::new(), Vec::new())
}

fn drag_group_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = &args[1];
    let r = if py::is_none(v) {
        Value::None
    } else {
        let wr = py::import_attr(vm, "weakref", "ref")?;
        vm.call(&wr, vec![v.clone()], Vec::new())?
    };
    py::set(vm, &args[0], "drag_group_weakref", r)?;
    Ok(Value::None)
}

fn drag_group(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    py::get_or_none(vm, this, "drag_group")
}

fn drag_snap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut x = arg_or(args, kw, 1, "x", Value::Int(0));
    let mut y = arg_or(args, kw, 2, "y", Value::Int(0));
    let delay = arg_or(args, kw, 3, "delay", Value::Int(0));
    let warper = arg_or(args, kw, 4, "warper", Value::None);
    let pw = py::get_or_none(vm, this, "parent_width")?;
    if !py::is_none(&pw) {
        let w = py::f64_of(vm, &pw)?;
        x = int(compute_raw(vm, &x, w)?);
    }
    let ph = py::get_or_none(vm, this, "parent_height")?;
    if !py::is_none(&ph) {
        let h = py::f64_of(vm, &ph)?;
        y = int(compute_raw(vm, &y, h)?);
    }
    py::set(vm, this, "target_x", x.clone())?;
    py::set(vm, this, "target_y", y.clone())?;
    let cur_x = py::get_or_none(vm, this, "x")?;
    if !py::is_none(&cur_x) {
        py::set(vm, this, "target_at_delay", delay)?;
        py::set(vm, this, "snap_warper", warper)?;
        py::set(vm, this, "snap_start_x", cur_x)?;
        let cy = py::get_or_none(vm, this, "y")?;
        py::set(vm, this, "snap_start_y", cy)?;
        py::set(vm, this, "snapping", Value::Bool(true))?;
    } else {
        let at = py::get_or_none(vm, this, "at")?;
        py::set(vm, this, "target_at", at)?;
        py::set(vm, this, "x", x.clone())?;
        py::set(vm, this, "y", y.clone())?;
    }
    let g = drag_group(vm, this)?;
    if !py::is_none(&g) {
        let pos = py::get(vm, &g, "positions")?;
        let name = py::get_or_none(vm, this, "drag_name")?;
        let op = py::get_or_none(vm, this, "old_position")?;
        let t = vm.types.new_tuple(vec![x, y, op]);
        ops::setitem(vm, &pos, name, t)?;
    }
    render::redraw(vm, this, 0.);
    Ok(Value::None)
}

fn drag_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let root = args.get(2).cloned().unwrap_or(Value::Bool(true));
    if ops::truthy(vm, &root)? {
        let d = super::core::displayable_class(vm);
        let f = py::get(vm, &Value::Obj(d), "set_style_prefix")?;
        vm.call(&f, vec![this.clone(), args[1].clone(), root], Vec::new())?;
        let c = py::get_or_none(vm, this, "child")?;
        if !py::is_none(&c) {
            vm.call_method(
                &c,
                "set_style_prefix",
                vec![args[1].clone(), Value::Bool(false)],
            )?;
        }
    }
    Ok(Value::None)
}

fn drag_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if !py::is_none(&py::get_or_none(vm, this, "child")?) {
        return Err(py::error(vm, "Drag expects either zero or one children."));
    }
    let c = super::easy::displayable(vm, &args[1], &Value::None, true)?;
    py::set(vm, this, "child", c)?;
    render::invalidate(vm, this);
    Ok(Value::None)
}

fn drag_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "child", Value::None)?;
    render::redraw(vm, &args[0], 0.);
    Ok(Value::None)
}

fn drag_set_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = super::easy::displayable(vm, &args[1], &Value::None, true)?;
    vm.call_method(&c, "per_interact", Vec::new())?;
    py::set(vm, &args[0], "child", c)?;
    render::invalidate(vm, &args[0]);
    Ok(Value::None)
}

fn drag_top(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let g = drag_group(vm, &args[0])?;
    if !py::is_none(&g) {
        let l = vm.types.new_list(vec![args[0].clone()]);
        vm.call_method(&g, "raise_children", vec![l])?;
    }
    Ok(Value::None)
}

fn drag_bottom(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let g = drag_group(vm, &args[0])?;
    if !py::is_none(&g) {
        let l = vm.types.new_list(vec![args[0].clone()]);
        vm.call_method(&g, "lower_children", vec![l])?;
    }
    Ok(Value::None)
}

fn joined_offsets(vm: &mut Vm, this: &Value) -> PyResult<Vec<(Value, f64, f64)>> {
    let dj = py::get(vm, this, "drag_joined")?;
    let l = vm.call(&dj, vec![this.clone()], Vec::new())?;
    let mut out = Vec::new();
    for t in py::items(vm, &l)? {
        let p = py::items(vm, &t)?;
        let d = get_drag_from_name(vm, this, &p[0])?;
        out.push((d, py::f64_of(vm, &p[1])?, py::f64_of(vm, &p[2])?));
    }
    Ok(out)
}

fn get_drag_from_name(vm: &mut Vm, this: &Value, name: &Value) -> PyResult<Value> {
    let dc = mget(vm, "Drag")?;
    if dc.obj().is_some_and(|c| ops::isinstance(vm, name, c)) {
        return Ok(name.clone());
    }
    let g = drag_group(vm, this)?;
    vm.call_method(&g, "get_child_by_name", vec![name.clone()])
}

fn drag_get_drag_from_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_drag_from_name(vm, &args[0], &args[1])
}

fn prefix(vm: &mut Vm, d: &Value, p: &str) -> PyResult<()> {
    let s = vm.types.new_str(p);
    vm.call_method(d, "set_style_prefix", vec![s, Value::Bool(true)])?;
    Ok(())
}

fn update_style_prefix(vm: &mut Vm, this: &Value) -> PyResult<()> {
    prefix(vm, this, "idle_")?;
    for (d, _, _) in joined_offsets(vm, this)? {
        prefix(vm, &d, "selected_hover_")?;
    }
    let ld = py::get_or_none(vm, this, "last_drop")?;
    if !py::is_none(&ld) {
        prefix(vm, &ld, "selected_idle_")?;
    }
    Ok(())
}

fn drag_update_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    update_style_prefix(vm, &args[0])?;
    Ok(Value::None)
}

fn drag_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![c]))
}

fn drag_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "focus")?;
    vm.call(&f, vec![this.clone(), Value::Bool(default)], Vec::new())?;
    if default && bget(vm, this, "drag_moved")? {
        update_style_prefix(vm, this)?;
    }
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        return run(vm, &h, Vec::new(), Vec::new());
    }
    Ok(Value::None)
}

fn drag_unfocus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "unfocus")?;
    vm.call(&f, vec![this.clone(), Value::Bool(default)], Vec::new())?;
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        let ru = py::import_attr(vm, "renpy.display.behavior", "run_unhovered")?;
        vm.call(&ru, vec![h], Vec::new())?;
        let u = py::get_or_none(vm, this, "unhovered")?;
        run(vm, &u, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn drag_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let mut child = sty(vm, this, "child")?;
    if py::is_none(&child) {
        child = py::get(vm, this, "child")?;
    }
    let (pw, ph) = render::rs(vm).borrow().render_size;
    py::set(vm, this, "parent_width", Value::Float(pw))?;
    py::set(vm, this, "parent_height", Value::Float(ph))?;
    let cr = render::render(vm, &child, w, h, st, at)?;
    let (cw, ch) = super::layout::size(&cr);
    let rv = super::layout::new(vm, cw, ch);
    super::layout::blit(vm, &rv, &cr, 0., 0.)?;
    py::set(vm, this, "w", Value::Float(cw))?;
    py::set(vm, this, "h", Value::Float(ch))?;
    let mut pos = Vec::new();
    for n in ["xpos", "ypos", "xanchor", "yanchor", "xoffset", "yoffset"] {
        pos.push(sty(vm, this, n)?);
    }
    let position = vm.types.new_tuple(pos);
    let g = drag_group(vm, this)?;
    let name = py::get_or_none(vm, this, "drag_name")?;
    if py::is_none(&py::get_or_none(vm, this, "x")?) && !py::is_none(&g) && !py::is_none(&name) {
        let positions = py::get(vm, &g, "positions")?;
        if ops::contains(vm, &positions, &name)? {
            let dgp = ops::getitem(vm, &positions, name.clone())?;
            let p = py::items(vm, &dgp)?;
            py::set(vm, this, "x", p[0].clone())?;
            py::set(vm, this, "y", p[1].clone())?;
            let op = if p.len() == 3 {
                p[2].clone()
            } else {
                position.clone()
            };
            py::set(vm, this, "old_position", op)?;
        }
    }
    let op = py::get_or_none(vm, this, "old_position")?;
    let place = !ops::eq(vm, &op, &position)? || py::is_none(&py::get_or_none(vm, this, "x")?);
    if place {
        py::set(vm, this, "x", Value::None)?;
        let r = vm.call_method(
            this,
            "place",
            vec![
                Value::None,
                Value::Int(0),
                Value::Int(0),
                Value::Float(w),
                Value::Float(h),
                rv.clone(),
            ],
        )?;
        let r = py::items(vm, &r)?;
        let (px, pyy) = (py::f64_of(vm, &r[0])?, py::f64_of(vm, &r[1])?);
        py::set(vm, this, "x", int(px))?;
        py::set(vm, this, "y", int(pyy))?;
        py::set(vm, this, "target_x", Value::None)?;
        py::set(vm, this, "old_position", position)?;
    }
    if py::is_none(&py::get_or_none(vm, this, "target_x")?) {
        let x = py::get_or_none(vm, this, "x")?;
        let y = py::get_or_none(vm, this, "y")?;
        py::set(vm, this, "target_x", x)?;
        py::set(vm, this, "target_y", y)?;
        py::set(vm, this, "target_at", Value::Float(at))?;
    }
    let tad = fget(vm, this, "target_at_delay")?;
    let target_at = fget(vm, this, "target_at")?;
    let self_at = fget(vm, this, "at")?;
    if tad != 0. {
        py::set(vm, this, "target_at", Value::Float(at + tad))?;
        py::set(vm, this, "target_at_delay", Value::Int(0))?;
        py::set(vm, this, "snap_start", Value::Float(at))?;
        render::redraw(vm, this, 0.);
        py::set(vm, this, "snapping", Value::Bool(true))?;
    } else if target_at <= at || target_at <= self_at {
        let tx = py::get_or_none(vm, this, "target_x")?;
        let ty = py::get_or_none(vm, this, "target_y")?;
        py::set(vm, this, "x", tx.clone())?;
        py::set(vm, this, "y", ty.clone())?;
        py::set(vm, this, "snap_warper", Value::None)?;
        py::set(vm, this, "snap_start", Value::Int(0))?;
        if bget(vm, this, "snapping")? {
            let s = py::get_or_none(vm, this, "snapped")?;
            run(
                vm,
                &s,
                vec![this.clone(), tx, ty, Value::Bool(true)],
                Vec::new(),
            )?;
        }
        py::set(vm, this, "snapping", Value::Bool(false))?;
    } else if bget(vm, this, "snapping")? {
        let ss = fget(vm, this, "snap_start")?;
        let mut done = (at - ss) / (target_at - ss);
        let w = py::get_or_none(vm, this, "snap_warper")?;
        if !py::is_none(&w) {
            let d = vm.call(&w, vec![Value::Float(done)], Vec::new())?;
            done = py::f64_of(vm, &d)?;
        }
        let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
        for (n, t, s) in [
            ("x", "target_x", "snap_start_x"),
            ("y", "target_y", "snap_start_y"),
        ] {
            let (tv, sv) = (fget(vm, this, t)?, fget(vm, this, s)?);
            let v = vm.call(&abs, vec![Value::Float((tv - sv) * done + sv)], Vec::new())?;
            py::set(vm, this, n, v)?;
        }
        render::redraw(vm, this, 0.);
    }
    let clicked = py::get_or_none(vm, this, "clicked")?;
    if bget(vm, this, "draggable")? || !py::is_none(&clicked) {
        let dh = py::get(vm, this, "drag_handle")?;
        let p = py::items(vm, &dh)?;
        let fx = compute_raw(vm, &p[0], cw)?.trunc();
        let fy = compute_raw(vm, &p[1], ch)?.trunc();
        let fw = compute_raw(vm, &p[2], cw)?.trunc();
        let fh = compute_raw(vm, &p[3], ch)?.trunc();
        let mut mask = sty(vm, this, "focus_mask")?;
        if matches!(mask, Value::Bool(true)) {
            mask = render::subsurface(vm, &cr, [fx, fy, fw, fh], false, false)?;
        } else if !py::is_none(&mask) {
            match render::render(vm, &mask, fw, fh, st, at) {
                Ok(r) => mask = r,
                Err(_) if crysirror_python::builtins::is_callable(vm, &mask) => {}
                Err(_) => {
                    return Err(py::error(
                        vm,
                        "Focus_mask must be None, True, a displayable, or a callable.",
                    ));
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
    let x = py::get_or_none(vm, this, "x")?;
    let y = py::get_or_none(vm, this, "y")?;
    py::set(vm, this, "last_x", x)?;
    py::set(vm, this, "last_y", y)?;
    py::set(vm, this, "at", Value::Float(at))?;
    Ok(rv)
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

fn touch(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.display", "touch")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false)
}

fn run_clicked(vm: &mut Vm, clicked: &Value, this: &Value) -> PyResult<Value> {
    match run(vm, clicked, vec![this.clone()], Vec::new()) {
        Err(e) if vm.is_exc(&e, "TypeError") => run(vm, clicked, Vec::new(), Vec::new()),
        r => r,
    }
}

fn clamp_offscreen(
    vm: &mut Vm,
    off: &Value,
    d: &Value,
    mut nx: f64,
    mut ny: f64,
) -> PyResult<(f64, f64)> {
    let (pw, ph) = (fget(vm, d, "parent_width")?, fget(vm, d, "parent_height")?);
    let (w, h) = (fget(vm, d, "w")?, fget(vm, d, "h")?);
    let truthy = ops::truthy(vm, off)?;
    if !truthy || off.as_str() == Some("vertical") {
        nx = nx.max(0.).min((pw - w).trunc());
    }
    if !truthy || off.as_str() == Some("horizontal") {
        ny = ny.max(0.).min((ph - h).trunc());
    }
    if matches!(off.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        let t = py::items(vm, off)?;
        let mut f = Vec::new();
        for v in &t {
            f.push(py::f64_of(vm, v)?);
        }
        match f.len() {
            4 => {
                nx = nx.max(f[0]).min((pw - w + f[1]).trunc());
                ny = ny.max(f[2]).min((ph - h + f[3]).trunc());
            }
            2 => {
                nx = nx.max((f[0] - w).trunc()).min((pw - f[0]).trunc());
                ny = ny.max((f[1] - h).trunc()).min((ph - f[1]).trunc());
            }
            _ => {
                return Err(py::error(
                    vm,
                    "Invalid number of arguments to drag_offscreen.",
                ));
            }
        }
    } else if truthy && off.as_str().is_none() && crysirror_python::builtins::is_callable(vm, off) {
        let r = vm.call(off, vec![int(nx), int(ny)], Vec::new())?;
        let r = py::items(vm, &r)?;
        nx = py::f64_of(vm, &r[0])?;
        ny = py::f64_of(vm, &r[1])?;
    }
    Ok((nx, ny))
}

fn drag_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args[1].clone();
    let x = py::f64_of(vm, &args[2])?;
    let y = py::f64_of(vm, &args[3])?;
    let st = py::f64_of(vm, &args[4])?;
    let mut grabbed = py::same(&get_grab(vm)?, this);
    let child = py::get(vm, this, "child")?;
    if !grabbed {
        let rv = vm.call_method(&child, "event", args[1..].to_vec())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    let f = vm.call_method(this, "is_focused", Vec::new())?;
    if !ops::truthy(vm, &f)? {
        return Ok(Value::None);
    }
    let par_x = (fget(vm, this, "last_x")? + x).trunc();
    let par_y = (fget(vm, this, "last_y")? + y).trunc();
    if grabbed && bget(vm, this, "snapping")? {
        py::set(vm, this, "snapping", Value::Bool(false))?;
        let s = py::get_or_none(vm, this, "snapped")?;
        let tx = py::get_or_none(vm, this, "target_x")?;
        let ty = py::get_or_none(vm, this, "target_y")?;
        run(
            vm,
            &s,
            vec![this.clone(), tx, ty, Value::Bool(false)],
            Vec::new(),
        )?;
        py::set(vm, this, "snap_warper", Value::None)?;
        py::set(vm, this, "snap_start", Value::Int(0))?;
    }
    let alternate = py::get_or_none(vm, this, "alternate")?;
    let longpress = py::config_f64(vm, "longpress_duration").unwrap_or(0.5);
    if !py::is_none(&alternate) && touch(vm) && map_name(vm, &ev, "drag_activate")? {
        py::set(vm, this, "click_time", Value::Float(st))?;
        let i = py::import_attr(vm, "renpy.game", "interface")?;
        vm.call_method(&i, "timeout", vec![Value::Float(longpress)])?;
    }
    let mut offsets: Vec<(Value, f64, f64)> = Vec::new();
    let mut joined: Vec<Value> = Vec::new();
    if grabbed {
        offsets = joined_offsets(vm, this)?;
        joined = offsets.iter().map(|o| o.0.clone()).collect();
    } else if bget(vm, this, "draggable")? && map_name(vm, &ev, "drag_activate")? {
        offsets = joined_offsets(vm, this)?;
        joined = offsets.iter().map(|o| o.0.clone()).collect();
        if joined.is_empty() {
            return Err(ignore_event(vm));
        }
        set_grab(vm, this.clone())?;
        let act = py::get_or_none(vm, &joined[0], "activated")?;
        let jl = vm.types.new_list(joined.clone());
        run(vm, &act, vec![jl], Vec::new())?;
        let (mut gx, mut gy) = (x, y);
        for (d, xo, yo) in &offsets {
            if py::same(d, this) {
                gx += xo;
                gy += yo;
                break;
            }
        }
        py::set(vm, this, "grab_x", Value::Float(gx))?;
        py::set(vm, this, "grab_y", Value::Float(gy))?;
        py::set(vm, this, "drag_moved", Value::Bool(false))?;
        py::set(vm, this, "start_x", Value::Float(par_x - gx))?;
        py::set(vm, this, "start_y", Value::Float(par_y - gy))?;
        grabbed = true;
        let snd = sty(vm, this, "activate_sound")?;
        if !py::is_none(&snd) {
            let p = py::import_attr(vm, "renpy.exports", "play")?;
            vm.call(&p, vec![snd], Vec::new())?;
        }
    } else if !py::is_none(&alternate) && map_name(vm, &ev, "button_alternate")? {
        let rv = run(vm, &alternate, Vec::new(), Vec::new())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
        return Err(ignore_event(vm));
    }
    let ct = py::get_or_none(vm, this, "click_time")?;
    if !py::is_none(&alternate)
        && touch(vm)
        && !py::is_none(&ct)
        && st - py::f64_of(vm, &ct)? > longpress
    {
        py::set(vm, this, "click_time", Value::None)?;
        let rv = run(vm, &alternate, Vec::new(), Vec::new())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    let clicked = py::get_or_none(vm, this, "clicked")?;
    if !grabbed {
        if !py::is_none(&clicked) && map_name(vm, &ev, "drag_deactivate")? {
            py::set(vm, this, "click_time", Value::None)?;
            let rv = run_clicked(vm, &clicked, this)?;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
            return Err(ignore_event(vm));
        }
        return vm.call_method(&child, "event", args[1..].to_vec());
    }
    let ty = {
        let t = py::get_or_none(vm, &ev, "type")?;
        py::int(vm, &t).unwrap_or(0)
    };
    let handled = matches!(
        ty,
        events::MOUSEMOTION | events::MOUSEBUTTONUP | events::MOUSEBUTTONDOWN
    );
    let (gx, gy) = (fget(vm, this, "grab_x")?, fget(vm, this, "grab_y")?);
    if handled {
        let (sx, sy) = (fget(vm, this, "start_x")?, fget(vm, this, "start_y")?);
        if !bget(vm, this, "drag_moved")? && (sx != par_x - gx || sy != par_y - gy) {
            py::set(vm, this, "drag_moved", Value::Bool(true))?;
            py::set(vm, this, "click_time", Value::None)?;
            let g = drag_group(vm, this)?;
            if bget(vm, this, "drag_raise")? && !py::is_none(&g) {
                let jl = vm.types.new_list(joined.clone());
                vm.call_method(&g, "raise_children", vec![jl])?;
            }
        }
        if bget(vm, this, "drag_moved")? {
            let off = py::get_or_none(vm, this, "drag_offscreen")?;
            let op = py::get_or_none(vm, this, "old_position")?;
            let self_at = py::get_or_none(vm, this, "at")?;
            for (d, xo, yo) in &offsets {
                let (nx, ny) = clamp_offscreen(
                    vm,
                    &off,
                    d,
                    (par_x - gx + xo).trunc(),
                    (par_y - gy + yo).trunc(),
                )?;
                let dg = drag_group(vm, d)?;
                let dn = py::get_or_none(vm, d, "drag_name")?;
                if !py::is_none(&dg) && !py::is_none(&dn) {
                    let pos = py::get(vm, &dg, "positions")?;
                    let t = vm.types.new_tuple(vec![int(nx), int(ny), op.clone()]);
                    ops::setitem(vm, &pos, dn, t)?;
                }
                for (n, v) in [("x", nx), ("y", ny), ("target_x", nx), ("target_y", ny)] {
                    py::set(vm, d, n, int(v))?;
                }
                py::set(vm, d, "target_at", self_at.clone())?;
                let dragging = py::get_or_none(vm, &joined[0], "dragging")?;
                if !py::is_none(&dragging) {
                    let jl = vm.types.new_list(joined.clone());
                    let rv = run(vm, &dragging, vec![jl], Vec::new())?;
                    if !py::is_none(&rv) {
                        return Ok(rv);
                    }
                }
                render::redraw(vm, d, 0.);
            }
        }
    }
    let g = drag_group(vm, this)?;
    let moved = bget(vm, this, "drag_moved")?;
    let drop = if !py::is_none(&g) && moved {
        let jl = vm.types.new_list(joined.clone());
        if bget(vm, this, "mouse_drop")? {
            vm.call_method(
                &g,
                "get_drop_at",
                vec![jl, Value::Float(par_x), Value::Float(par_y)],
            )?
        } else {
            vm.call_method(&g, "get_best_drop", vec![jl])?
        }
    } else {
        Value::None
    };
    let ld = py::get_or_none(vm, this, "last_drop")?;
    if !py::same(&drop, &ld) {
        if !py::is_none(&ld) {
            prefix(vm, &ld, "idle_")?;
        }
        py::set(vm, this, "last_drop", drop.clone())?;
    }
    if moved {
        update_style_prefix(vm, this)?;
    }
    if map_name(vm, &ev, "drag_deactivate")? {
        py::set(vm, this, "click_time", Value::None)?;
        set_grab(vm, Value::None)?;
        if !py::is_none(&drop) {
            prefix(vm, &drop, "idle_")?;
        }
        for d in &joined {
            prefix(vm, d, "idle_")?;
        }
        prefix(vm, this, "hover_")?;
        py::set(vm, this, "last_drop", Value::None)?;
        let clear_grab = |vm: &mut Vm| -> PyResult<()> {
            py::set(vm, this, "grab_x", Value::None)?;
            py::set(vm, this, "grab_y", Value::None)
        };
        if moved {
            let dragged = py::get_or_none(vm, &joined[0], "dragged")?;
            if !py::is_none(&dragged) {
                let jl = vm.types.new_list(joined.clone());
                let rv = run(vm, &dragged, vec![jl, drop.clone()], Vec::new())?;
                if !py::is_none(&rv) {
                    clear_grab(vm)?;
                    return Ok(rv);
                }
            }
            if !py::is_none(&drop) {
                let dropped = py::get_or_none(vm, &drop, "dropped")?;
                if !py::is_none(&dropped) {
                    let jl = vm.types.new_list(joined.clone());
                    let rv = run(vm, &dropped, vec![drop.clone(), jl], Vec::new())?;
                    if !py::is_none(&rv) {
                        clear_grab(vm)?;
                        return Ok(rv);
                    }
                }
            }
        } else if ops::truthy(vm, &clicked)? {
            let rv = match vm.call(&clicked, vec![this.clone()], Vec::new()) {
                Err(e) if vm.is_exc(&e, "TypeError") => run(vm, &clicked, Vec::new(), Vec::new())?,
                r => r?,
            };
            if !py::is_none(&rv) {
                clear_grab(vm)?;
                return Ok(rv);
            }
        }
        clear_grab(vm)?;
        py::set(vm, this, "drag_moved", Value::Bool(false))?;
    }
    if handled {
        return Err(ignore_event(vm));
    }
    Ok(Value::None)
}

fn drag_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let x = py::get_or_none(vm, this, "x")?;
    if !py::is_none(&x) {
        let y = py::get_or_none(vm, this, "y")?;
        return Ok(vm.types.new_tuple(vec![
            x,
            y,
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Bool(true),
        ]));
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "get_placement")?;
    vm.call(&f, vec![this.clone()], Vec::new())
}

fn drag_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    prefix(vm, &args[0], "idle_")?;
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "per_interact")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())
}

// DragGroup -------------------------------------------------------------------------------------------

fn multibox(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    py::get(vm, &mb, n)
}

fn is_drag(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    let dc = mget(vm, "Drag")?;
    Ok(dc.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

fn dg_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut props = Vec::new();
    let mut replaces = Value::None;
    let mut min_overlap = Value::Int(0);
    for (k, v) in kw {
        match &**k {
            "replaces" => replaces = v.clone(),
            "min_overlap" => min_overlap = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    for n in ["style", "layout"] {
        if !props.iter().any(|(k, _)| &**k == n) {
            props.push((n.into(), vm.types.new_str("fixed")));
        }
    }
    py::set(vm, this, "min_overlap", min_overlap)?;
    let init = multibox(vm, "__init__")?;
    vm.call(&init, vec![this.clone()], props)?;
    py::set(vm, this, "sorted", Value::Bool(false))?;
    let rd = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
    let dgc = mget(vm, "DragGroup")?;
    if dgc.obj().is_some_and(|c| ops::isinstance(vm, &replaces, c)) {
        let p = py::get(vm, &replaces, "positions")?;
        let np = vm.call(&rd, vec![p], Vec::new())?;
        py::set(vm, this, "positions", np)?;
        for n in ["sensitive", "z_serial"] {
            let v = py::get_or_none(vm, &replaces, n)?;
            py::set(vm, this, n, v)?;
        }
    } else {
        let np = vm.call(&rd, Vec::new(), Vec::new())?;
        py::set(vm, this, "positions", np)?;
        py::set(vm, this, "sensitive", Value::Bool(true))?;
        py::set(vm, this, "z_serial", Value::Int(0))?;
    }
    for c in &args[1..] {
        vm.call_method(this, "add", vec![c.clone()])?;
    }
    Ok(Value::None)
}

fn dg_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, child) = (&args[0], &args[1]);
    if !is_drag(vm, child)? {
        return Err(py::error(vm, "Only drags can be added to a drag group."));
    }
    let add = multibox(vm, "add")?;
    vm.call(&add, vec![this.clone(), child.clone()], Vec::new())?;
    py::set(vm, this, "sorted", Value::Bool(false))?;
    render::invalidate(vm, this);
    py::set(vm, child, "drag_group", this.clone())?;
    if py::config_bool(vm, "drag_group_add_top").unwrap_or(false) {
        vm.call_method(child, "top", Vec::new())?;
    }
    Ok(Value::None)
}

fn dg_remove(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, child) = (&args[0], &args[1]);
    if !is_drag(vm, child)? {
        return Err(py::error(
            vm,
            "Only drags can be removed from a drag group.",
        ));
    }
    py::set(vm, child, "x", Value::None)?;
    let rm = multibox(vm, "remove")?;
    vm.call(&rm, vec![this.clone(), child.clone()], Vec::new())
}

fn dg_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let children = py::get(vm, this, "children")?;
    let items = py::items(vm, &children)?;
    for c in &items {
        py::set(vm, c, "drag_group", this.clone())?;
    }
    if !bget(vm, this, "sorted")? {
        let mut keyed = Vec::new();
        for c in items {
            keyed.push((fget(vm, &c, "z")?, c));
        }
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let sorted: Vec<Value> = keyed.into_iter().map(|k| k.1).collect();
        match children.payload() {
            Some(crysirror_python::Payload::List(l)) => *l.borrow_mut() = sorted,
            _ => return Err(py::error(vm, "DragGroup.children must be a list.")),
        }
        py::set(vm, this, "sorted", Value::Bool(true))?;
    }
    let r = multibox(vm, "render")?;
    vm.call(&r, args.to_vec(), Vec::new())
}

fn dg_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !bget(vm, &args[0], "sensitive")? {
        return Ok(Value::None);
    }
    let e = multibox(vm, "event")?;
    vm.call(&e, args.to_vec(), Vec::new())
}

fn reorder(vm: &mut Vm, args: &[Value], sign: i64) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "sorted", Value::Bool(false))?;
    let mut serial = {
        let v = py::get_or_none(vm, this, "z_serial")?;
        py::int(vm, &v).unwrap_or(0)
    };
    for i in py::items(vm, &args[1])? {
        serial += 1;
        py::set(vm, &i, "z", Value::Int(sign * serial))?;
    }
    py::set(vm, this, "z_serial", Value::Int(serial))?;
    render::redraw(vm, this, 0.);
    Ok(Value::None)
}

fn dg_raise(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    reorder(vm, args, 1)
}

fn dg_lower(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    reorder(vm, args, -1)
}

fn drop_candidates(vm: &mut Vm, this: &Value, joined: &[Value]) -> PyResult<Vec<Value>> {
    let mut out = Vec::new();
    for c in py::get_items(vm, this, "children")? {
        if joined.iter().any(|j| py::same(j, &c)) {
            continue;
        }
        if !bget(vm, &c, "droppable")? || py::is_none(&py::get_or_none(vm, &c, "x")?) {
            continue;
        }
        out.push(c);
    }
    Ok(out)
}

fn allowable(vm: &mut Vm, c: &Value, joined: &Value) -> PyResult<bool> {
    let f = py::get(vm, c, "drop_allowable")?;
    let r = vm.call(&f, vec![c.clone(), joined.clone()], Vec::new())?;
    ops::truthy(vm, &r)
}

fn rect_of(vm: &mut Vm, d: &Value) -> PyResult<[f64; 4]> {
    Ok([
        fget(vm, d, "x")?,
        fget(vm, d, "y")?,
        fget(vm, d, "w")?,
        fget(vm, d, "h")?,
    ])
}

fn dg_get_best_drop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let jl = args[1].clone();
    let joined = py::items(vm, &jl)?;
    let min_overlap = fget(vm, this, "min_overlap")?;
    let cands = drop_candidates(vm, this, &joined)?;
    let mut max = 0.;
    let mut rv = Value::None;
    for d in &joined {
        let r1 = rect_of(vm, d)?;
        for c in &cands {
            let r2 = rect_of(vm, c)?;
            let o = rect_overlap(r1, r2);
            if o >= max && o >= min_overlap && allowable(vm, c, &jl)? {
                rv = c.clone();
                max = o;
            }
        }
    }
    Ok(if max <= 0. { Value::None } else { rv })
}

fn dg_get_drop_at(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let jl = args[1].clone();
    let joined = py::items(vm, &jl)?;
    let x = py::f64_of(vm, &args[2])?;
    let y = py::f64_of(vm, &args[3])?;
    for c in drop_candidates(vm, this, &joined)? {
        let r = rect_of(vm, &c)?;
        if x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3] && allowable(vm, &c, &jl)? {
            return Ok(c);
        }
    }
    Ok(Value::None)
}

fn dg_get_children(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rl = py::import_attr(vm, "renpy.revertable", "RevertableList")?;
    let c = py::get(vm, &args[0], "children")?;
    vm.call(&rl, vec![c], Vec::new())
}

fn dg_get_child_by_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for c in py::get_items(vm, &args[0], "children")? {
        let n = py::get_or_none(vm, &c, "drag_name")?;
        if ops::eq(vm, &n, &args[1])? {
            return Ok(c);
        }
    }
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("default_drag_group", f_default_drag_group),
            ("default_drag_joined", f_default_drag_joined),
            ("default_drop_allowable", |_, _, _| Ok(Value::Bool(true))),
            ("rect_overlap_area", f_rect_overlap_area),
        ],
    );
    let d = Value::Obj(super::core::displayable_class(vm));
    let ro = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let drag = py::class(
        vm,
        MODULE,
        "Drag",
        &[d, ro],
        &[
            ("__init__", drag_init),
            ("snap", drag_snap),
            ("set_style_prefix", drag_set_style_prefix),
            ("add", drag_add),
            ("_clear", drag_clear),
            ("set_child", drag_set_child),
            ("top", drag_top),
            ("bottom", drag_bottom),
            ("update_style_prefix", drag_update_style_prefix),
            ("visit", drag_visit),
            ("focus", drag_focus),
            ("unfocus", drag_unfocus),
            ("render", drag_render),
            ("get_drag_from_name", drag_get_drag_from_name),
            ("event", drag_event),
            ("get_placement", drag_get_placement),
            ("per_interact", drag_per_interact),
        ],
        &[
            ("z", Value::Int(0)),
            ("focusable", Value::Bool(true)),
            ("old_position", Value::None),
            ("drag_offscreen", Value::Bool(false)),
            ("activated", Value::None),
            ("alternate", Value::None),
            ("dragging", Value::None),
            ("drag_group_weakref", Value::None),
            ("click_time", Value::None),
        ],
    )?;
    let dv = Value::Obj(drag);
    let g = vm.types.new_builtin("_draggable", drag_draggable);
    let p = py::property(vm, g, None)?;
    py::set(vm, &dv, "_draggable", p)?;
    let g = vm.types.new_builtin("drag_group", drag_group_get);
    let s = vm.types.new_builtin("drag_group", drag_group_set);
    let p = py::property(vm, g, Some(s))?;
    py::set(vm, &dv, "drag_group", p)?;
    crysirror_python::modules::set(vm, &m, "Drag", dv);
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let rl = py::import_attr(vm, "renpy.revertable", "RevertableList")?;
    let dg = py::class(
        vm,
        MODULE,
        "DragGroup",
        &[mb],
        &[
            ("__init__", dg_init),
            ("add", dg_add),
            ("remove", dg_remove),
            ("render", dg_render),
            ("event", dg_event),
            ("raise_children", dg_raise),
            ("lower_children", dg_lower),
            ("get_best_drop", dg_get_best_drop),
            ("get_drop_at", dg_get_drop_at),
            ("get_children", dg_get_children),
            ("get_child_by_name", dg_get_child_by_name),
        ],
        &[
            ("z_serial", Value::Int(0)),
            ("sorted", Value::Bool(false)),
            ("_list_type", rl),
        ],
    )?;
    crysirror_python::modules::set(vm, &m, "DragGroup", Value::Obj(dg));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
