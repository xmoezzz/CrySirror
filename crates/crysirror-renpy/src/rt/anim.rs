// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.anim`, after Ren'Py 8.3 `anim.py` (MIT): the state-machine
//! animation (State/Edge/SMAnimation), Animation/TransitionAnimation, Blink
//! and Filmstrip.
use super::{
    layout::{blit, new, rargs, size},
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.anim";

fn less_updates(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.game", "less_updates")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false)
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn wrap(vm: &mut Vm, im: &Value) -> PyResult<Value> {
    let (w, h) = size(im);
    let rv = new(vm, w, h);
    blit(vm, &rv, im, 0., 0.)?;
    Ok(rv)
}

// State / Edge ------------------------------------------------------------------------------------

fn state_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = args.get(1).cloned().unwrap_or(Value::None);
    let mut image = args.get(2).cloned().unwrap_or(Value::None);
    if ops::truthy(vm, &image)? && !super::core::is_displayable(vm, &image) {
        image = super::easy::displayable(vm, &image, &Value::None, true)?;
    }
    py::set(vm, this, "name", name)?;
    py::set(vm, this, "image", image)?;
    let atl = vm.types.new_tuple(args.iter().skip(3).cloned().collect());
    py::set(vm, this, "atlist", atl)?;
    let props = py::kwargs_dict(vm, kw)?;
    py::set(vm, this, "properties", props)?;
    Ok(Value::None)
}

fn state_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let states = py::get(vm, &args[1], "states")?;
    let n = py::get(vm, &args[0], "name")?;
    ops::setitem(vm, &states, n, args[0].clone())?;
    Ok(Value::None)
}

fn state_get_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut rv = py::get(vm, this, "image")?;
    for f in py::get_items(vm, this, "atlist")? {
        rv = vm.call(&f, vec![rv], Vec::new())?;
    }
    let props = py::get(vm, this, "properties")?;
    if ops::truthy(vm, &props)? {
        let pos = py::import_attr(vm, "renpy.display.layout", "Position")?;
        let kw = py::str_items(vm, &props)?;
        rv = vm.call(&pos, vec![rv], kw)?;
    }
    Ok(rv)
}

fn state_motion_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let image = py::get_or_none(vm, this, "image")?;
    let child = if py::is_none(&image) {
        args[1].clone()
    } else {
        image
    };
    let mut a = vec![py::get(vm, this, "name")?, child];
    a.extend(py::get_items(vm, this, "atlist")?);
    let c = mget(vm, "State")?;
    vm.call(&c, a, Vec::new())
}

fn edge_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, (n, d)) in [
        ("old", Value::None),
        ("delay", Value::None),
        ("new", Value::None),
        ("trans", Value::None),
        ("prob", Value::Int(1)),
    ]
    .into_iter()
    .enumerate()
    {
        py::set(vm, this, n, arg_or(args, kw, i + 1, n, d))?;
    }
    Ok(Value::None)
}

fn edge_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prob = {
        let v = py::get(vm, this, "prob")?;
        py::int(vm, &v)?
    };
    let edges = py::get(vm, &args[1], "edges")?;
    let old = py::get(vm, this, "old")?;
    for _ in 0..prob {
        let l = vm.call_method(
            &edges,
            "setdefault",
            vec![old.clone(), vm.types.new_list(Vec::new())],
        )?;
        vm.call_method(&l, "append", vec![this.clone()])?;
    }
    Ok(Value::None)
}

// SMAnimation ---------------------------------------------------------------------------------------

fn sma_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut delay = Value::None;
    let mut showold = Value::Bool(false);
    let mut timebase = Value::Bool(true);
    let mut props = Vec::new();
    for (k, v) in kw {
        match &**k {
            "delay" => delay = v.clone(),
            "showold" => showold = v.clone(),
            "anim_timebase" => timebase = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    py::set(vm, this, "delay", delay)?;
    py::set(vm, this, "showold", showold)?;
    py::set(vm, this, "anim_timebase", timebase)?;
    super::core::d_init(vm, &[this.clone()], &props)?;
    let pd = py::kwargs_dict(vm, &props)?;
    py::set(vm, this, "properties", pd)?;
    py::set(
        vm,
        this,
        "initial",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    py::set(vm, this, "states", vm.types.new_dict())?;
    py::set(vm, this, "edges", vm.types.new_dict())?;
    for a in args.iter().skip(2) {
        vm.call_method(a, "add", vec![this.clone()])?;
    }
    for n in ["edge_start", "edge_cache", "edge", "state"] {
        py::set(vm, this, n, Value::None)?;
    }
    Ok(Value::None)
}

fn sma_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let states = py::get(vm, &args[0], "states")?;
    let mut out = Vec::new();
    for (_, s) in ops::mapping_items(vm, &states)? {
        out.push(py::get(vm, &s, "image")?);
    }
    Ok(vm.types.new_list(out))
}

fn pick_edge(vm: &mut Vm, this: &Value, state: &Value) -> PyResult<()> {
    let edges = py::get(vm, this, "edges")?;
    if !ops::contains(vm, &edges, state)? {
        py::set(vm, this, "edge", Value::None)?;
        return Ok(());
    }
    let l = ops::getitem(vm, &edges, state.clone())?;
    let choice = py::import_attr(vm, "random", "choice")?;
    let e = vm.call(&choice, vec![l], Vec::new())?;
    let new = py::get(vm, &e, "new")?;
    py::set(vm, this, "edge", e)?;
    py::set(vm, this, "state", new)?;
    Ok(())
}

fn sma_pick_edge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pick_edge(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn state_image(vm: &mut Vm, this: &Value, key: Value) -> PyResult<Value> {
    let states = py::get(vm, this, "states")?;
    let s = ops::getitem(vm, &states, key)?;
    vm.call_method(&s, "get_image", Vec::new())
}

fn sma_update_cache(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let edge = py::get(vm, this, "edge")?;
    let trans = py::get_or_none(vm, &edge, "trans")?;
    let old = py::get(vm, &edge, "old")?;
    let new = py::get(vm, &edge, "new")?;
    let im = if ops::truthy(vm, &trans)? {
        let o = state_image(vm, this, old)?;
        let n = state_image(vm, this, new)?;
        vm.call(
            &trans,
            Vec::new(),
            vec![("old_widget".into(), o), ("new_widget".into(), n)],
        )?
    } else {
        let so = {
            let v = py::get_or_none(vm, this, "showold")?;
            ops::truthy(vm, &v)?
        };
        state_image(vm, this, if so { old } else { new })?
    };
    py::set(vm, this, "edge_cache", im)?;
    Ok(Value::None)
}

fn sma_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ec = py::get_or_none(vm, this, "edge_cache")?;
    if ops::truthy(vm, &ec)? {
        return vm.call_method(&ec, "get_placement", Vec::new());
    }
    let st = py::get_or_none(vm, this, "state")?;
    if ops::truthy(vm, &st)? {
        let im = state_image(vm, this, st)?;
        return vm.call_method(&im, "get_placement", Vec::new());
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "get_placement")?;
    vm.call(&f, vec![this.clone()], Vec::new())
}

fn fget(vm: &mut Vm, o: &Value, n: &str) -> PyResult<f64> {
    let v = py::get(vm, o, n)?;
    py::f64_of(vm, &v)
}

fn sma_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let tb = {
        let v = py::get_or_none(vm, this, "anim_timebase")?;
        ops::truthy(vm, &v)?
    };
    let t = if tb { at } else { st };
    let es = py::get_or_none(vm, this, "edge_start")?;
    if py::is_none(&es) || t < py::f64_of(vm, &es)? {
        py::set(vm, this, "edge_start", Value::Float(t))?;
        py::set(vm, this, "edge_cache", Value::None)?;
        let initial = py::get(vm, this, "initial")?;
        pick_edge(vm, this, &initial)?;
    }
    loop {
        let edge = py::get_or_none(vm, this, "edge")?;
        if !ops::truthy(vm, &edge)? {
            break;
        }
        let es = fget(vm, this, "edge_start")?;
        let d = fget(vm, &edge, "delay")?;
        if t <= es + d {
            break;
        }
        py::set(vm, this, "edge_start", Value::Float(es + d))?;
        py::set(vm, this, "edge_cache", Value::None)?;
        let new = py::get(vm, &edge, "new")?;
        pick_edge(vm, this, &new)?;
    }
    let edge = py::get_or_none(vm, this, "edge")?;
    let es = fget(vm, this, "edge_start")?;
    let im = if !ops::truthy(vm, &edge)? {
        let s = py::get(vm, this, "state")?;
        let img = state_image(vm, this, s)?;
        render::render(vm, &img, w, h, st - es, at)?
    } else {
        let ec = py::get_or_none(vm, this, "edge_cache")?;
        if !ops::truthy(vm, &ec)? {
            vm.call_method(this, "update_cache", Vec::new())?;
        }
        let ec = py::get(vm, this, "edge_cache")?;
        let im = render::render(vm, &ec, w, h, t - es, at)?;
        if !less_updates(vm) {
            let d = fget(vm, &edge, "delay")?;
            render::redraw(vm, &ec, d - (t - es));
        }
        im
    };
    wrap(vm, &im)
}

fn sma_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut child = arg_or(args, kw, 1, "child", Value::None);
    if py::is_none(&child) {
        child = arg_or(args, kw, 2, "new_widget", Value::None);
    }
    let mut a = vec![py::get(vm, this, "initial")?];
    let states = py::get(vm, this, "states")?;
    for (_, s) in ops::mapping_items(vm, &states)? {
        a.push(vm.call_method(&s, "motion_copy", vec![child.clone()])?);
    }
    let edges = py::get(vm, this, "edges")?;
    for (_, l) in ops::mapping_items(vm, &edges)? {
        a.extend(py::items(vm, &l)?);
    }
    let props = py::get(vm, this, "properties")?;
    let mut k = py::str_items(vm, &props)?;
    k.push(("delay".into(), py::get_or_none(vm, this, "delay")?));
    let c = mget(vm, "SMAnimation")?;
    vm.call(&c, a, k)
}

// Animation / TransitionAnimation ----------------------------------------------------------------

fn f_animation(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut a = Vec::new();
    for (i, v) in args.iter().enumerate() {
        a.push(v.clone());
        if i % 2 == 1 {
            a.push(Value::None);
        }
    }
    let c = mget(vm, "TransitionAnimation")?;
    vm.call(&c, a, kw.to_vec())
}

fn ta_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut props = Vec::new();
    let mut tb = Value::Bool(true);
    for (k, v) in kw {
        if &**k == "anim_timebase" {
            tb = v.clone();
        } else {
            props.push((k.clone(), v.clone()));
        }
    }
    if !props.iter().any(|(k, _)| &**k == "style") {
        props.push(("style".into(), vm.types.new_str("animation")));
    }
    py::set(vm, this, "anim_timebase", tb)?;
    super::core::d_init(vm, &[this.clone()], &props)?;
    let (mut images, mut delays, mut trans) = (Vec::new(), Vec::new(), Vec::new());
    for (i, a) in args.iter().skip(1).enumerate() {
        match i % 3 {
            0 => images.push(super::easy::displayable(vm, a, &Value::None, true)?),
            1 => delays.push(a.clone()),
            _ => trans.push(a.clone()),
        }
    }
    if images.len() > delays.len() {
        delays.push(Value::Float(365.25 * 86400.0));
    }
    if images.len() > trans.len() {
        trans.push(Value::None);
    }
    if images.is_empty() {
        return Err(py::error(vm, "Animation requires at least one image."));
    }
    let mut prev = vec![images[images.len() - 1].clone()];
    prev.extend(images[..images.len() - 1].iter().cloned());
    let mut tr = vec![trans[trans.len() - 1].clone()];
    tr.extend(trans[..trans.len() - 1].iter().cloned());
    py::set(vm, this, "images", vm.types.new_list(images))?;
    py::set(vm, this, "prev_images", vm.types.new_list(prev))?;
    py::set(vm, this, "delays", vm.types.new_list(delays))?;
    py::set(vm, this, "transitions", vm.types.new_list(tr))?;
    Ok(Value::None)
}

fn ta_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let tb = {
        let v = py::get_or_none(vm, this, "anim_timebase")?;
        ops::truthy(vm, &v)?
    };
    let orig_t = if tb { at } else { st };
    let images = py::get_items(vm, this, "images")?;
    let prev = py::get_items(vm, this, "prev_images")?;
    let trans = py::get_items(vm, this, "transitions")?;
    let mut delays = Vec::new();
    for d in py::get_items(vm, this, "delays")? {
        delays.push(py::f64_of(vm, &d)?);
    }
    let total: f64 = delays.iter().sum();
    let mut t = if total > 0. {
        orig_t.rem_euclid(total)
    } else {
        0.
    };
    for i in 0..images.len().min(delays.len()) {
        let delay = delays[i];
        if t < delay {
            if !less_updates(vm) {
                render::redraw(vm, this, delay - t);
            }
            let mut image = images[i].clone();
            let tr = trans.get(i).cloned().unwrap_or(Value::None);
            if ops::truthy(vm, &tr)? && orig_t >= delays[0] {
                image = vm.call(
                    &tr,
                    Vec::new(),
                    vec![
                        ("old_widget".into(), prev[i].clone()),
                        ("new_widget".into(), image),
                    ],
                )?;
            }
            let im = render::render(vm, &image, w, h, t, at)?;
            return wrap(vm, &im);
        }
        t -= delay;
    }
    Ok(Value::None)
}

fn ta_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "images")
}

// Blink -----------------------------------------------------------------------------------------------

const BLINK: &[&str] = &[
    "image",
    "on",
    "off",
    "rise",
    "set",
    "high",
    "low",
    "offset",
    "anim_timebase",
];

fn blink_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut vals = vec![
        Value::None,
        Value::Float(0.5),
        Value::Float(0.5),
        Value::Float(0.5),
        Value::Float(0.5),
        Value::Float(1.),
        Value::Float(0.),
        Value::Float(0.),
        Value::Bool(false),
    ];
    for (i, v) in args.iter().skip(1).enumerate().take(BLINK.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match BLINK.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    vals[0] = super::easy::displayable(vm, &vals[0], &Value::None, true)?;
    let mut cycle = 0.;
    for (i, n) in BLINK.iter().enumerate() {
        if matches!(*n, "on" | "set" | "off" | "rise") {
            cycle += py::f64_of(vm, &vals[i])?;
        }
        py::set(vm, this, n, vals[i].clone())?;
    }
    py::set(vm, this, "cycle", Value::Float(cycle))?;
    Ok(Value::None)
}

fn blink_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::get(vm, &args[0], "image")?;
    Ok(vm.types.new_list(vec![i]))
}

fn blink_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let tb = {
        let v = py::get_or_none(vm, this, "anim_timebase")?;
        ops::truthy(vm, &v)?
    };
    let t = if tb { at } else { st };
    let g = |vm: &mut Vm, n: &str| fget(vm, this, n);
    let (on, set, off, rise) = (g(vm, "on")?, g(vm, "set")?, g(vm, "off")?, g(vm, "rise")?);
    let (high, low, offset, cycle) = (
        g(vm, "high")?,
        g(vm, "low")?,
        g(vm, "offset")?,
        g(vm, "cycle")?,
    );
    let mut time = if cycle > 0. {
        (offset + t).rem_euclid(cycle)
    } else {
        0.
    };
    let mut delay = 0.;
    let mut alpha = high;
    if (0. ..on).contains(&time) {
        delay = on - time;
        alpha = high;
    }
    time -= on;
    if (0. ..set).contains(&time) {
        delay = 0.;
        let frac = time / set;
        alpha = low * frac + high * (1. - frac);
    }
    time -= set;
    if (0. ..off).contains(&time) {
        delay = off - time;
        alpha = low;
    }
    time -= off;
    if (0. ..rise).contains(&time) {
        delay = 0.;
        let frac = time / rise;
        alpha = high * frac + low * (1. - frac);
    }
    let image = py::get(vm, this, "image")?;
    // Upstream passes (height, width) through unchanged; the swap is only in its parameter names.
    let rend = render::render(vm, &image, w, h, st, at)?;
    let rv = wrap(vm, &rend)?;
    py::set(vm, &rv, "alpha", Value::Float(alpha))?;
    vm.call_method(&rv, "add_shader", vec![vm.types.new_str("renpy.alpha")])?;
    vm.call_method(
        &rv,
        "add_uniform",
        vec![vm.types.new_str("u_renpy_alpha"), Value::Float(alpha)],
    )?;
    vm.call_method(
        &rv,
        "add_uniform",
        vec![vm.types.new_str("u_renpy_over"), Value::Float(1.)],
    )?;
    if !less_updates(vm) {
        render::redraw(vm, this, delay);
    }
    Ok(rv)
}

// Filmstrip ---------------------------------------------------------------------------------------

fn f_filmstrip(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let image = arg_or(args, kw, 0, "image", Value::None);
    let fs = py::items(vm, &arg_or(args, kw, 1, "framesize", Value::None))?;
    let gs = py::items(vm, &arg_or(args, kw, 2, "gridsize", Value::None))?;
    let delay = arg_or(args, kw, 3, "delay", Value::None);
    let frames = arg_or(args, kw, 4, "frames", Value::None);
    let lp = arg_or(args, kw, 5, "loop", Value::Bool(true));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| {
            !matches!(
                &**k,
                "image" | "framesize" | "gridsize" | "delay" | "frames" | "loop"
            )
        })
        .cloned()
        .collect();
    let (w, h) = (fs[0].clone(), fs[1].clone());
    let (cols, rows) = (py::int(vm, &gs[0])?, py::int(vm, &gs[1])?);
    let frames = if py::is_none(&frames) {
        cols * rows
    } else {
        py::int(vm, &frames)?
    };
    let tcls = py::import_attr(vm, "renpy.display.transform", "Transform")?;
    let (wi, hi) = (py::f64_of(vm, &w)?, py::f64_of(vm, &h)?);
    let mut a = Vec::new();
    let mut i = 0;
    'outer: for r in 0..rows {
        for c in 0..cols {
            let crop = vm.types.new_tuple(vec![
                Value::Float(c as f64 * wi),
                Value::Float(r as f64 * hi),
                w.clone(),
                h.clone(),
            ]);
            a.push(vm.call(&tcls, vec![image.clone()], vec![("crop".into(), crop)])?);
            a.push(delay.clone());
            i += 1;
            if i == frames {
                break 'outer;
            }
        }
    }
    if !ops::truthy(vm, &lp)? {
        a.pop();
    }
    f_animation(vm, &a, &props)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("Animation", f_animation), ("Filmstrip", f_filmstrip)],
    );
    let d = Value::Obj(super::core::displayable_class(vm));
    let object = vm.types.object.clone();
    let mut c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods| -> PyResult<()> {
        let cls = py::class(vm, MODULE, name, bases, methods, &[])?;
        crysirror_python::modules::set(vm, &m, name, Value::Obj(cls));
        Ok(())
    };
    let o = Value::Obj(object);
    c(
        vm,
        "State",
        &[o.clone()],
        &[
            ("__init__", state_init),
            ("add", state_add),
            ("get_image", state_get_image),
            ("motion_copy", state_motion_copy),
        ],
    )?;
    c(
        vm,
        "Edge",
        &[o],
        &[("__init__", edge_init), ("add", edge_add)],
    )?;
    c(
        vm,
        "SMAnimation",
        &[d.clone()],
        &[
            ("__init__", sma_init),
            ("visit", sma_visit),
            ("pick_edge", sma_pick_edge),
            ("update_cache", sma_update_cache),
            ("get_placement", sma_get_placement),
            ("render", sma_render),
            ("__call__", sma_call),
        ],
    )?;
    c(
        vm,
        "TransitionAnimation",
        &[d.clone()],
        &[
            ("__init__", ta_init),
            ("render", ta_render),
            ("visit", ta_visit),
        ],
    )?;
    c(
        vm,
        "Blink",
        &[d],
        &[
            ("__init__", blink_init),
            ("visit", blink_visit),
            ("render", blink_render),
        ],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
