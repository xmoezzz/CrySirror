// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.transition`, after Ren'Py 8.3 `transition.py` (MIT).
//! Dissolves become render operations (opacity of the new layer subtree),
//! image dissolves use the shared coverage mask, and crop/push moves are
//! clipped layer offsets.
use super::{
    layout::{rargs, size},
    py::{self, *},
    render::{self, RenderData, new_render, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.transition";

fn less_updates(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.game", "less_updates")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false)
}

fn less_imagedissolve(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.display", "less_imagedissolve")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false)
}

fn f(vm: &mut Vm, this: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, this, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn tr_init_base(vm: &mut Vm, this: &Value, delay: Value, props: Kw) -> PyResult<()> {
    super::core::d_init(vm, &[this.clone()], props)?;
    py::set(vm, this, "delay", delay)?;
    py::set(vm, this, "events", Value::Bool(true))?;
    Ok(())
}

fn tr_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "delay")
        .cloned()
        .collect();
    tr_init_base(vm, &args[0], delay, &props)?;
    Ok(Value::None)
}

fn tr_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let events = py::get_or_none(vm, this, "events")?;
    if ops::truthy(vm, &events)? || super::events::is_type(vm, &ev, super::events::TIMEEVENT)? {
        let nw = py::get(vm, this, "new_widget")?;
        return vm.call_method(&nw, "event", args[1..].to_vec());
    }
    Ok(Value::None)
}

fn tr_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get_or_none(vm, &args[0], "new_widget")?;
    let o = py::get_or_none(vm, &args[0], "old_widget")?;
    Ok(vm.types.new_list(vec![n, o]))
}

fn tr_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::config_bool(vm, "transitions_use_child_placement").unwrap_or(true) {
        let n = py::get(vm, &args[0], "new_widget")?;
        return vm.call_method(&n, "get_placement", Vec::new());
    }
    let s = py::get(vm, &args[0], "style")?;
    vm.call_method(&s, "get_placement", Vec::new())
}

pub fn null_render(vm: &mut Vm, this: &Value, w: f64, h: f64, st: f64, at: f64) -> PyResult<Value> {
    py::set(vm, this, "events", Value::Bool(true))?;
    let nw = py::get(vm, this, "new_widget")?;
    let surf = render::render(vm, &nw, w, h, st, at)?;
    let (sw, sh) = size(&surf);
    let rv = super::layout::new(vm, sw, sh);
    super::layout::blit(vm, &rv, &surf, 0., 0.)?;
    Ok(rv)
}

fn f_null_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    let (w, h, st, at) = rargs(vm, args)?;
    null_render(vm, &d, w, h, st, at)
}

fn widgets(vm: &mut Vm, this: &Value, kw: Kw, args: &[Value], first: usize) -> PyResult<()> {
    let old = arg_or(args, kw, first, "old_widget", Value::None);
    let new = arg_or(args, kw, first + 1, "new_widget", Value::None);
    py::set(vm, this, "old_widget", old)?;
    py::set(vm, this, "new_widget", new)?;
    Ok(())
}

fn strip<'a>(kw: Kw<'a>, names: &[&str]) -> Vec<(std::rc::Rc<str>, Value)> {
    kw.iter()
        .filter(|(k, _)| !names.contains(&&**k))
        .cloned()
        .collect()
}

// NoTransition ------------------------------------------------------------------------------

fn notrans_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    tr_init_base(
        vm,
        this,
        delay,
        &strip(kw, &["delay", "old_widget", "new_widget"]),
    )?;
    widgets(vm, this, kw, args, 2)?;
    Ok(Value::None)
}

fn notrans_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (w, h, st, at) = rargs(vm, args)?;
    null_render(vm, &args[0], w, h, st, at)
}

// MultipleTransition ---------------------------------------------------------------------------

fn multiple_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let items = py::items(vm, &arg_or(args, kw, 1, "args", Value::None))?;
    let old_widget = arg_or(args, kw, 2, "old_widget", Value::None);
    let new_widget = arg_or(args, kw, 3, "new_widget", Value::None);
    if items.len() % 2 != 1 || items.len() < 3 {
        return Err(py::error(
            vm,
            "MultipleTransition requires an odd number of items, and at least 3 items.",
        ));
    }
    let mut screens = Vec::new();
    for i in items.iter().step_by(2) {
        screens.push(super::easy::displayable(vm, i, &Value::None, true)?);
    }
    let oldnew = |w: &Value| match w {
        Value::Bool(false) => old_widget.clone(),
        Value::Bool(true) => new_widget.clone(),
        o => o.clone(),
    };
    let mut transitions = Vec::new();
    let mut total = 0f64;
    for (k, trans) in items.iter().skip(1).step_by(2).enumerate() {
        let old = oldnew(&screens[k]);
        let new = oldnew(&screens[k + 1]);
        let t = vm.call(
            trans,
            Vec::new(),
            vec![("old_widget".into(), old), ("new_widget".into(), new)],
        )?;
        total += f(vm, &t, "delay")?;
        transitions.push(t);
    }
    tr_init_base(
        vm,
        this,
        Value::Float(total),
        &strip(kw, &["args", "old_widget", "new_widget"]),
    )?;
    py::set(vm, this, "screens", vm.types.new_list(screens))?;
    let last = transitions.last().cloned().unwrap_or(Value::None);
    py::set(vm, this, "transitions", vm.types.new_list(transitions))?;
    py::set(vm, this, "new_widget", last)?;
    py::set(vm, this, "events", Value::Bool(false))?;
    Ok(Value::None)
}

fn multiple_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    for s in py::get_items(vm, &args[0], "screens")? {
        if super::core::is_displayable(vm, &s) {
            out.push(s);
        }
    }
    out.extend(py::get_items(vm, &args[0], "transitions")?);
    Ok(vm.types.new_list(out))
}

fn multiple_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let events = py::get_or_none(vm, this, "events")?;
    if ops::truthy(vm, &events)? || super::events::is_type(vm, &ev, super::events::TIMEEVENT)? {
        let t = py::get_items(vm, this, "transitions")?;
        if let Some(last) = t.last() {
            return vm.call_method(last, "event", args[1..].to_vec());
        }
    }
    Ok(Value::None)
}

fn multiple_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, mut st, at) = rargs(vm, args)?;
    if less_updates(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let ts = py::get_items(vm, this, "transitions")?;
    let last = ts.last().cloned().unwrap_or(Value::None);
    let mut trans = last.clone();
    let mut found = false;
    for t in &ts[..ts.len().saturating_sub(1)] {
        let d = f(vm, t, "delay")?;
        if d > st {
            trans = t.clone();
            found = true;
            break;
        }
        st -= d;
    }
    if !found {
        py::set(vm, this, "events", Value::Bool(true))?;
    }
    if !py::same(&trans, &last) {
        render::render(vm, &last, w, h, 0., 0.)?;
    }
    let surf = render::render(vm, &trans, w, h, st, at)?;
    let (sw, sh) = size(&surf);
    let rv = super::layout::new(vm, sw, sh);
    super::layout::blit(vm, &rv, &surf, 0., 0.)?;
    let d = f(vm, &trans, "delay")?;
    if st < d {
        render::redraw(vm, this, d - st);
    }
    Ok(rv)
}

fn f_fade(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let out_time = arg_or(args, kw, 0, "out_time", Value::Int(0));
    let hold_time = arg_or(args, kw, 1, "hold_time", Value::Int(0));
    let in_time = arg_or(args, kw, 2, "in_time", Value::Int(0));
    let old_widget = arg_or(args, kw, 3, "old_widget", Value::None);
    let new_widget = arg_or(args, kw, 4, "new_widget", Value::None);
    let color = arg_or(args, kw, 5, "color", Value::None);
    let widget = arg_or(args, kw, 6, "widget", Value::None);
    let alpha = arg_or(args, kw, 7, "alpha", Value::Bool(false));
    let curry = py::import_attr(vm, "renpy.curry", "curry")?;
    let dc = py::import_attr(vm, MODULE, "Dissolve")?;
    let nc = py::import_attr(vm, MODULE, "NoTransition")?;
    let dissolve = vm.call(&curry, vec![dc], Vec::new())?;
    let notrans = vm.call(&curry, vec![nc], Vec::new())?;
    let mut widget = if py::is_none(&widget) {
        Value::None
    } else {
        super::easy::displayable(vm, &widget, &Value::None, true)?
    };
    let solid = py::import_attr(vm, "renpy.display.imagelike", "Solid")?;
    if ops::truthy(vm, &color)? {
        widget = vm.call(&solid, vec![color], Vec::new())?;
    }
    if !ops::truthy(vm, &widget)? {
        let black = vm.types.new_tuple(vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(255),
        ]);
        widget = vm.call(&solid, vec![black], Vec::new())?;
    }
    let d_out = vm.call(
        &dissolve,
        vec![out_time],
        vec![("alpha".into(), alpha.clone())],
    )?;
    let mut items = vec![Value::Bool(false), d_out, widget.clone()];
    if ops::truthy(vm, &hold_time)? {
        items.push(vm.call(&notrans, vec![hold_time], Vec::new())?);
        items.push(widget);
    }
    items.push(vm.call(&dissolve, vec![in_time], vec![("alpha".into(), alpha)])?);
    items.push(Value::Bool(true));
    let mc = py::import_attr(vm, MODULE, "MultipleTransition")?;
    let l = vm.types.new_list(items);
    vm.call(
        &mc,
        vec![l],
        vec![
            ("old_widget".into(), old_widget),
            ("new_widget".into(), new_widget),
        ],
    )
}

// Pixellate ---------------------------------------------------------------------------------------

fn pixellate_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let time = py::f64_of(vm, &arg_or(args, kw, 1, "time", Value::Int(0)))?;
    let steps = arg_or(args, kw, 2, "steps", Value::Int(1));
    tr_init_base(
        vm,
        this,
        Value::Float(time),
        &strip(kw, &["time", "steps", "old_widget", "new_widget"]),
    )?;
    py::set(vm, this, "time", Value::Float(time))?;
    py::set(vm, this, "steps", steps.clone())?;
    widgets(vm, this, kw, args, 3)?;
    py::set(vm, this, "events", Value::Bool(false))?;
    let s = py::f64_of(vm, &steps)?;
    py::set(vm, this, "quantum", Value::Float(time / (2. * s)))?;
    Ok(Value::None)
}

fn pixellate_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let time = f(vm, this, "time")?;
    if st >= time {
        py::set(vm, this, "events", Value::Bool(true))?;
        let nw = py::get(vm, this, "new_widget")?;
        return render::render(vm, &nw, w, h, st, at);
    }
    let steps = f(vm, this, "steps")?;
    let q = f(vm, this, "quantum")?;
    let mut step = (st / q).floor() + 1.;
    let mut visible = py::get(vm, this, "old_widget")?;
    if step > steps {
        step = steps * 2. - step + 1.;
        visible = py::get(vm, this, "new_widget")?;
        py::set(vm, this, "events", Value::Bool(true))?;
    }
    let rdr = render::render(vm, &visible, w, h, st, at)?;
    let (rw, rh) = size(&rdr);
    let mut rd = RenderData::new(rw, rh);
    let src = render::source_of(vm, &rdr)?;
    rd.children.push(render::Blit {
        source: src,
        x: 0.,
        y: 0.,
        focus: true,
        main: true,
    });
    rd.operation = render::PIXELLATE;
    rd.operation_parameter = 2f64.powf(step);
    render::redraw(vm, this, 0.);
    Ok(new_render(vm, rd))
}

// Dissolve / ImageDissolve / AlphaDissolve ------------------------------------------------------------

fn dissolve_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let time = arg_or(args, kw, 1, "time", Value::Int(0));
    tr_init_base(
        vm,
        this,
        time.clone(),
        &strip(
            kw,
            &["time", "old_widget", "new_widget", "alpha", "time_warp"],
        ),
    )?;
    py::set(vm, this, "time", time)?;
    widgets(vm, this, kw, args, 2)?;
    py::set(vm, this, "events", Value::Bool(false))?;
    py::set(
        vm,
        this,
        "alpha",
        arg_or(args, kw, 4, "alpha", Value::Bool(false)),
    )?;
    py::set(
        vm,
        this,
        "time_warp",
        arg_or(args, kw, 5, "time_warp", Value::None),
    )?;
    Ok(Value::None)
}

fn after_upgrade_alpha(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(1)))?;
    if v < 1 {
        py::set(vm, &args[0], "alpha", Value::Bool(false))?;
    }
    Ok(Value::None)
}

fn warp(vm: &mut Vm, this: &Value, c: f64) -> PyResult<f64> {
    let tw = py::get_or_none(vm, this, "time_warp")?;
    if py::is_none(&tw) {
        return Ok(c);
    }
    let r = vm.call(&tw, vec![Value::Float(c)], Vec::new())?;
    py::f64_of(vm, &r)
}

fn op_size(vm: &mut Vm, renders: &[&Value]) -> (f64, f64) {
    let shrink = py::config_bool(vm, "dissolve_shrinks").unwrap_or(false);
    let sizes: Vec<(f64, f64)> = renders.iter().map(|r| size(r)).collect();
    if shrink {
        (
            sizes.iter().map(|s| s.0).fold(f64::MAX, f64::min),
            sizes.iter().map(|s| s.1).fold(f64::MAX, f64::min),
        )
    } else {
        (
            sizes.iter().map(|s| s.0).fold(0., f64::max),
            sizes.iter().map(|s| s.1).fold(0., f64::max),
        )
    }
}

fn dissolve_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let time = f(vm, this, "time")?;
    let nw = py::get(vm, this, "new_widget")?;
    if st >= time {
        py::set(vm, this, "events", Value::Bool(true))?;
        return render::render(vm, &nw, w, h, st, at);
    }
    let complete = warp(vm, this, (st / time).min(1.))?;
    let ow = py::get(vm, this, "old_widget")?;
    let bottom = render::render(vm, &ow, w, h, st, at)?;
    let top = render::render(vm, &nw, w, h, st, at)?;
    let (rw, rh) = op_size(vm, &[&top, &bottom]);
    let mut rd = RenderData::new(rw, rh);
    rd.operation = render::DISSOLVE;
    let alpha = {
        let a = py::get_or_none(vm, this, "alpha")?;
        ops::truthy(vm, &a)?
    };
    rd.operation_alpha = alpha || py::config_bool(vm, "dissolve_force_alpha").unwrap_or(true);
    rd.operation_complete = complete;
    let b = render::source_of(vm, &bottom)?;
    let t = render::source_of(vm, &top)?;
    rd.children.push(render::Blit {
        source: b,
        x: 0.,
        y: 0.,
        focus: false,
        main: false,
    });
    rd.children.push(render::Blit {
        source: t,
        x: 0.,
        y: 0.,
        focus: true,
        main: true,
    });
    render::redraw(vm, this, 0.);
    Ok(new_render(vm, rd))
}

fn imagedissolve_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "image",
        "time",
        "ramplen",
        "ramptype",
        "ramp",
        "reverse",
        "alpha",
        "old_widget",
        "new_widget",
        "time_warp",
    ];
    let mut vals = vec![
        Value::None,
        Value::Int(0),
        Value::Int(8),
        vm.types.new_str("linear"),
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
        Value::None,
        Value::None,
        Value::None,
    ];
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
    tr_init_base(vm, this, vals[1].clone(), &props)?;
    py::set(vm, this, "old_widget", vals[7].clone())?;
    py::set(vm, this, "new_widget", vals[8].clone())?;
    py::set(vm, this, "events", Value::Bool(false))?;
    py::set(vm, this, "alpha", vals[6].clone())?;
    py::set(vm, this, "time_warp", vals[9].clone())?;
    let image = super::easy::displayable(vm, &vals[0], &Value::None, true)?;
    py::set(vm, this, "image", image)?;
    py::set(vm, this, "reverse", vals[5].clone())?;
    let mut ramplen = py::int(vm, &vals[2])?;
    if !py::is_none(&vals[4]) {
        ramplen = ops::len(vm, &vals[4])? as i64;
    }
    py::set(vm, this, "ramplen", Value::Int(ramplen.max(1)))?;
    Ok(Value::None)
}

fn imagedissolve_visit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let base = tr_visit(vm, args, kw)?;
    let mut v = py::items(vm, &base)?;
    v.push(py::get(vm, &args[0], "image")?);
    Ok(vm.types.new_list(v))
}

fn set_reverse(vm: &mut Vm, rd: &mut RenderData, reverse: bool) -> PyResult<()> {
    if reverse {
        let d = vm.types.new_dict();
        py::dict_set(vm, &d, "imagedissolve_reverse", Value::Bool(true))?;
        rd.properties = d;
    }
    Ok(())
}

fn imagedissolve_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) || less_imagedissolve(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let delay = f(vm, this, "delay")?;
    let nw = py::get(vm, this, "new_widget")?;
    if st >= delay {
        py::set(vm, this, "events", Value::Bool(true))?;
        return render::render(vm, &nw, w, h, st, at);
    }
    let image_d = py::get(vm, this, "image")?;
    let image = render::render(vm, &image_d, w, h, st, at)?;
    let ow = py::get(vm, this, "old_widget")?;
    let bottom = render::render(vm, &ow, w, h, st, at)?;
    let top = render::render(vm, &nw, w, h, st, at)?;
    let (rw, rh) = op_size(vm, &[&bottom, &top, &image]);
    let mut rd = RenderData::new(rw, rh);
    let complete = warp(vm, this, st / delay)?;
    rd.operation = render::IMAGEDISSOLVE;
    let alpha = {
        let a = py::get_or_none(vm, this, "alpha")?;
        ops::truthy(vm, &a)?
    };
    rd.operation_alpha = alpha || py::config_bool(vm, "dissolve_force_alpha").unwrap_or(true);
    rd.operation_complete = complete;
    rd.operation_parameter = f(vm, this, "ramplen")?;
    let reverse = {
        let r = py::get_or_none(vm, this, "reverse")?;
        ops::truthy(vm, &r)?
    };
    set_reverse(vm, &mut rd, reverse)?;
    for (r, focus) in [(&image, false), (&bottom, false), (&top, true)] {
        let s = render::source_of(vm, r)?;
        rd.children.push(render::Blit {
            source: s,
            x: 0.,
            y: 0.,
            focus,
            main: focus,
        });
    }
    render::redraw(vm, this, 0.);
    Ok(new_render(vm, rd))
}

fn alphadissolve_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "control",
        "delay",
        "old_widget",
        "new_widget",
        "alpha",
        "reverse",
    ];
    let mut vals = vec![
        Value::None,
        Value::Float(0.),
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
    ];
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
    tr_init_base(vm, this, vals[1].clone(), &props)?;
    let fixed = py::import_attr(vm, "renpy.display.layout", "Fixed")?;
    let box_ = vm.call(&fixed, Vec::new(), Vec::new())?;
    let mut control = super::easy::displayable(vm, &vals[0], &Value::None, true)?;
    let dup = py::get_or_none(vm, &control, "_duplicatable")?;
    if ops::truthy(vm, &dup)? {
        let a = py::get_or_none(vm, this, "_args")?;
        control = vm.call_method(&control, "_duplicate", vec![a])?;
    }
    vm.call_method(&box_, "add", vec![control])?;
    py::set(vm, this, "control", box_)?;
    let old = super::easy::displayable(vm, &vals[2], &Value::None, true)?;
    let new = super::easy::displayable(vm, &vals[3], &Value::None, true)?;
    py::set(vm, this, "old_widget", old)?;
    py::set(vm, this, "new_widget", new)?;
    py::set(vm, this, "events", Value::Bool(false))?;
    py::set(vm, this, "alpha", vals[4].clone())?;
    py::set(vm, this, "reverse", vals[5].clone())?;
    Ok(Value::None)
}

fn alphadissolve_visit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let base = tr_visit(vm, args, kw)?;
    let mut v = py::items(vm, &base)?;
    v.push(py::get(vm, &args[0], "control")?);
    Ok(vm.types.new_list(v))
}

fn alphadissolve_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) || less_imagedissolve(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    if st >= f(vm, this, "delay")? {
        py::set(vm, this, "events", Value::Bool(true))?;
    }
    let ow = py::get(vm, this, "old_widget")?;
    let nw = py::get(vm, this, "new_widget")?;
    let bottom = render::render(vm, &ow, w, h, st, at)?;
    let top = render::render(vm, &nw, w, h, st, at)?;
    let (rw, rh) = op_size(vm, &[&bottom, &top]);
    let ctl = py::get(vm, this, "control")?;
    let control = render::render(vm, &ctl, rw, rh, st, at)?;
    let mut rd = RenderData::new(rw, rh);
    rd.operation = render::IMAGEDISSOLVE;
    let alpha = {
        let a = py::get_or_none(vm, this, "alpha")?;
        ops::truthy(vm, &a)?
    };
    rd.operation_alpha = alpha || py::config_bool(vm, "dissolve_force_alpha").unwrap_or(true);
    rd.operation_complete = 0.5;
    rd.operation_parameter = 256.;
    let reverse = {
        let r = py::get_or_none(vm, this, "reverse")?;
        ops::truthy(vm, &r)?
    };
    let c = render::source_of(vm, &control)?;
    rd.children.push(render::Blit {
        source: c,
        x: 0.,
        y: 0.,
        focus: false,
        main: false,
    });
    let b = render::source_of(vm, &bottom)?;
    let t = render::source_of(vm, &top)?;
    if !reverse {
        rd.children.push(render::Blit {
            source: b,
            x: 0.,
            y: 0.,
            focus: false,
            main: false,
        });
        rd.children.push(render::Blit {
            source: t,
            x: 0.,
            y: 0.,
            focus: true,
            main: true,
        });
    } else {
        rd.children.push(render::Blit {
            source: t,
            x: 0.,
            y: 0.,
            focus: true,
            main: true,
        });
        rd.children.push(render::Blit {
            source: b,
            x: 0.,
            y: 0.,
            focus: false,
            main: false,
        });
    }
    Ok(new_render(vm, rd))
}

// CropMove / PushMove -----------------------------------------------------------------------------

type Rect4 = [f64; 4];

fn crop_mode(mode: &str) -> Option<(Rect4, [f64; 2], Rect4, [f64; 2], bool)> {
    let full = [0., 0., 1., 1.];
    Some(match mode {
        "wiperight" => ([0., 0., 0., 1.], [0., 0.], full, [0., 0.], true),
        "wipeleft" => ([1., 0., 0., 1.], [1., 0.], full, [0., 0.], true),
        "wipedown" => ([0., 0., 1., 0.], [0., 0.], full, [0., 0.], true),
        "wipeup" => ([0., 1., 1., 0.], [0., 1.], full, [0., 0.], true),
        "slideright" => ([1., 0., 0., 1.], [0., 0.], full, [0., 0.], true),
        "slideleft" => ([0., 0., 0., 1.], [1., 0.], full, [0., 0.], true),
        "slideup" => ([0., 0., 1., 0.], [0., 1.], full, [0., 0.], true),
        "slidedown" => ([0., 1., 1., 0.], [0., 0.], full, [0., 0.], true),
        "slideawayleft" => (full, [0., 0.], [1., 0., 0., 1.], [0., 0.], false),
        "slideawayright" => (full, [0., 0.], [0., 0., 0., 1.], [1., 0.], false),
        "slideawaydown" => (full, [0., 0.], [0., 0., 1., 0.], [0., 1.], false),
        "slideawayup" => (full, [0., 0.], [0., 1., 1., 0.], [0., 0.], false),
        "irisout" => ([0.5, 0.5, 0., 0.], [0.5, 0.5], full, [0., 0.], true),
        "irisin" => (full, [0., 0.], [0.5, 0.5, 0., 0.], [0.5, 0.5], false),
        _ => return None,
    })
}

fn tuple_vals(vm: &mut Vm, v: &[f64]) -> Value {
    vm.types
        .new_tuple(v.iter().map(|x| Value::Float(*x)).collect())
}

fn read4(vm: &mut Vm, this: &Value, n: &str) -> PyResult<Vec<f64>> {
    let v = py::get_items(vm, this, n)?;
    v.iter().map(|x| py::f64_of(vm, x)).collect()
}

fn interp_tuple(a: &[f64], b: &[f64], t: f64, scales: &[f64]) -> Vec<f64> {
    a.iter()
        .zip(b)
        .zip(scales)
        .map(|((a, b), s)| (s * (a * (1. - t) + b * t)).round())
        .collect()
}

fn cropmove_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "time",
        "mode",
        "startcrop",
        "startpos",
        "endcrop",
        "endpos",
        "topnew",
        "old_widget",
        "new_widget",
    ];
    let mut vals = vec![
        Value::Int(0),
        vm.types.new_str("slideright"),
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(true),
        Value::None,
        Value::None,
    ];
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
    tr_init_base(vm, this, vals[0].clone(), &props)?;
    let mode = py::string(vm, &vals[1])?;
    let (sc, sp, ec, ep, topnew) = if mode == "custom" {
        let g4 = |vm: &mut Vm, v: &Value, d: &[f64]| -> PyResult<Vec<f64>> {
            if py::is_none(v) {
                Ok(d.to_vec())
            } else {
                py::items(vm, v)?
                    .iter()
                    .map(|x| py::f64_of(vm, x))
                    .collect()
            }
        };
        let sc = g4(vm, &vals[2], &[0., 0., 0., 1.])?;
        let sp = g4(vm, &vals[3], &[0., 0.])?;
        let ec = g4(vm, &vals[4], &[0., 0., 1., 1.])?;
        let ep = g4(vm, &vals[5], &[0., 0.])?;
        let tn = ops::truthy(vm, &vals[6])?;
        (sc, sp, ec, ep, tn)
    } else {
        let (sc, sp, ec, ep, tn) = crop_mode(&mode)
            .ok_or_else(|| py::error(vm, format!("Invalid mode {mode} passed into CropMove.")))?;
        (sc.to_vec(), sp.to_vec(), ec.to_vec(), ep.to_vec(), tn)
    };
    py::set(vm, this, "time", vals[0].clone())?;
    for (n, v) in [
        ("startcrop", &sc),
        ("startpos", &sp),
        ("endcrop", &ec),
        ("endpos", &ep),
    ] {
        let t = tuple_vals(vm, v);
        py::set(vm, this, n, t)?;
    }
    py::set(vm, this, "topnew", Value::Bool(topnew))?;
    py::set(vm, this, "old_widget", vals[7].clone())?;
    py::set(vm, this, "new_widget", vals[8].clone())?;
    py::set(vm, this, "events", Value::Bool(false))?;
    let (bottom, top) = if topnew {
        (vals[7].clone(), vals[8].clone())
    } else {
        (vals[8].clone(), vals[7].clone())
    };
    py::set(vm, this, "bottom", bottom)?;
    py::set(vm, this, "top", top)?;
    Ok(Value::None)
}

fn cropmove_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let time = st / f(vm, this, "time")?;
    if time >= 1. {
        py::set(vm, this, "events", Value::Bool(true))?;
        let nw = py::get(vm, this, "new_widget")?;
        return render::render(vm, &nw, w, h, st, at);
    }
    let scales = [w, h, w, h];
    let sc = read4(vm, this, "startcrop")?;
    let ec = read4(vm, this, "endcrop")?;
    let sp = read4(vm, this, "startpos")?;
    let ep = read4(vm, this, "endpos")?;
    let crop = interp_tuple(&sc, &ec, time, &scales);
    let pos = interp_tuple(&sp, &ep, time, &scales);
    let topd = py::get(vm, this, "top")?;
    let botd = py::get(vm, this, "bottom")?;
    let top = render::render(vm, &topd, w, h, st, at)?;
    let bottom = render::render(vm, &botd, w, h, st, at)?;
    let (bw, bh) = size(&bottom);
    let topnew = {
        let v = py::get_or_none(vm, this, "topnew")?;
        ops::truthy(vm, &v)?
    };
    let mut rd = RenderData::new(bw.min(w), bh.min(h));
    let b = render::source_of(vm, &bottom)?;
    rd.children.push(render::Blit {
        source: b,
        x: 0.,
        y: 0.,
        focus: !topnew,
        main: true,
    });
    let ss = render::subsurface(
        vm,
        &top,
        [crop[0], crop[1], crop[2], crop[3]],
        topnew,
        false,
    )?;
    let s = render::source_of(vm, &ss)?;
    rd.children.push(render::Blit {
        source: s,
        x: pos[0].trunc(),
        y: pos[1].trunc(),
        focus: topnew,
        main: true,
    });
    render::redraw(vm, this, 0.);
    Ok(new_render(vm, rd))
}

fn push_mode(mode: &str) -> Option<[[f64; 4]; 4]> {
    // new_startcrop, new_endcrop, old_startcrop, old_endcrop; positions below.
    Some(match mode {
        "pushright" => [
            [1., 0., 0., 1.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
            [0., 0., 0., 1.],
        ],
        "pushleft" => [
            [0., 0., 0., 1.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
            [1., 0., 0., 1.],
        ],
        "pushup" => [
            [0., 0., 1., 0.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
            [0., 1., 1., 0.],
        ],
        "pushdown" => [
            [0., 1., 1., 0.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
            [0., 0., 1., 0.],
        ],
        _ => return None,
    })
}

fn push_pos(mode: &str) -> [[f64; 2]; 4] {
    // new_startpos, new_endpos, old_startpos, old_endpos
    match mode {
        "pushright" => [[0., 0.], [0., 0.], [0., 0.], [1., 0.]],
        "pushleft" => [[1., 0.], [0., 0.], [0., 0.], [0., 0.]],
        "pushup" => [[0., 1.], [0., 0.], [0., 0.], [0., 0.]],
        _ => [[0., 0.], [0., 0.], [0., 0.], [0., 1.]],
    }
}

fn pushmove_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let time = arg_or(args, kw, 1, "time", Value::Int(0));
    let mode = arg_or(args, kw, 2, "mode", vm.types.new_str("pushright"));
    tr_init_base(
        vm,
        this,
        time.clone(),
        &strip(kw, &["time", "mode", "old_widget", "new_widget"]),
    )?;
    let mode = py::string(vm, &mode)?;
    let crops = push_mode(&mode)
        .ok_or_else(|| py::error(vm, format!("Invalid mode {mode} passed into PushMove.")))?;
    let pos = push_pos(&mode);
    for (n, v) in [
        ("new_startcrop", &crops[0]),
        ("new_endcrop", &crops[1]),
        ("old_startcrop", &crops[2]),
        ("old_endcrop", &crops[3]),
    ] {
        let t = tuple_vals(vm, v);
        py::set(vm, this, n, t)?;
    }
    for (n, v) in [
        ("new_startpos", &pos[0]),
        ("new_endpos", &pos[1]),
        ("old_startpos", &pos[2]),
        ("old_endpos", &pos[3]),
    ] {
        let t = tuple_vals(vm, v);
        py::set(vm, this, n, t)?;
    }
    py::set(vm, this, "time", time)?;
    widgets(vm, this, kw, args, 3)?;
    py::set(vm, this, "events", Value::Bool(false))?;
    Ok(Value::None)
}

fn pushmove_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    if less_updates(vm) {
        return null_render(vm, this, w, h, st, at);
    }
    let time = st / f(vm, this, "time")?;
    let nw = py::get(vm, this, "new_widget")?;
    if time >= 1. {
        py::set(vm, this, "events", Value::Bool(true))?;
        return render::render(vm, &nw, w, h, st, at);
    }
    let scales = [w, h, w, h];
    let mut v = std::collections::HashMap::new();
    for n in [
        "new_startcrop",
        "new_endcrop",
        "old_startcrop",
        "old_endcrop",
        "new_startpos",
        "new_endpos",
        "old_startpos",
        "old_endpos",
    ] {
        v.insert(n, read4(vm, this, n)?);
    }
    let new_crop = interp_tuple(&v["new_startcrop"], &v["new_endcrop"], time, &scales);
    let new_pos = interp_tuple(&v["new_startpos"], &v["new_endpos"], time, &scales);
    let old_crop = interp_tuple(&v["old_startcrop"], &v["old_endcrop"], time, &scales);
    let old_pos = interp_tuple(&v["old_startpos"], &v["old_endpos"], time, &scales);
    let ow = py::get(vm, this, "old_widget")?;
    let new = render::render(vm, &nw, w, h, st, at)?;
    let old = render::render(vm, &ow, w, h, st, at)?;
    let mut rd = RenderData::new(w, h);
    let oss = render::subsurface(
        vm,
        &old,
        [old_crop[0], old_crop[1], old_crop[2], old_crop[3]],
        true,
        false,
    )?;
    let s = render::source_of(vm, &oss)?;
    rd.children.push(render::Blit {
        source: s,
        x: old_pos[0],
        y: old_pos[1],
        focus: true,
        main: true,
    });
    let nss = render::subsurface(
        vm,
        &new,
        [new_crop[0], new_crop[1], new_crop[2], new_crop[3]],
        true,
        false,
    )?;
    let s = render::source_of(vm, &nss)?;
    rd.children.push(render::Blit {
        source: s,
        x: new_pos[0],
        y: new_pos[1],
        focus: true,
        main: true,
    });
    render::redraw(vm, this, 0.);
    Ok(new_render(vm, rd))
}

// Compose / Sub ------------------------------------------------------------------------------------

fn f_compose(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let trans = arg_or(args, kw, 0, "trans", Value::None);
    let before = arg_or(args, kw, 1, "before", Value::None);
    let after = arg_or(args, kw, 2, "after", Value::None);
    let nw = arg_or(args, kw, 3, "new_widget", Value::None);
    let ow = arg_or(args, kw, 4, "old_widget", Value::None);
    let pair = || -> Vec<(std::rc::Rc<str>, Value)> {
        vec![
            ("new_widget".into(), nw.clone()),
            ("old_widget".into(), ow.clone()),
        ]
    };
    let old = if py::is_none(&before) {
        ow.clone()
    } else {
        vm.call(&before, Vec::new(), pair())?
    };
    let new = if py::is_none(&after) {
        nw.clone()
    } else {
        vm.call(&after, Vec::new(), pair())?
    };
    vm.call(
        &trans,
        Vec::new(),
        vec![("new_widget".into(), new), ("old_widget".into(), old)],
    )
}

fn f_sub(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = arg_or(args, kw, 0, "rect", Value::None);
    let trans = arg_or(args, kw, 1, "trans", Value::None);
    let ow = arg_or(args, kw, 2, "old_widget", Value::None);
    let nw = arg_or(args, kw, 3, "new_widget", Value::None);
    let r = py::items(vm, &rect)?;
    let crop = py::import_attr(vm, "renpy.display.layout", "LiveCrop")?;
    let old = vm.call(&crop, vec![rect.clone(), ow], Vec::new())?;
    let new = vm.call(&crop, vec![rect, nw.clone()], Vec::new())?;
    let inner = vm.call(
        &trans,
        Vec::new(),
        vec![("old_widget".into(), old), ("new_widget".into(), new)],
    )?;
    let delay = py::get_or_none(vm, &inner, "delay")?;
    let pos = py::import_attr(vm, "renpy.display.layout", "Position")?;
    let inner = vm.call(
        &pos,
        vec![inner],
        vec![
            ("xpos".into(), r[0].clone()),
            ("ypos".into(), r[1].clone()),
            ("xanchor".into(), Value::Int(0)),
            ("yanchor".into(), Value::Int(0)),
        ],
    )?;
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let box_ = vm.call(
        &mb,
        Vec::new(),
        vec![("layout".into(), vm.types.new_str("fixed"))],
    )?;
    vm.call_method(&box_, "add", vec![nw])?;
    vm.call_method(&box_, "add", vec![inner])?;
    let nt = py::import_attr(vm, MODULE, "NoTransition")?;
    vm.call(
        &nt,
        vec![delay],
        vec![
            ("old_widget".into(), box_.clone()),
            ("new_widget".into(), box_),
        ],
    )
}

// Swing (`renpy.display.transition.Swing`, 7.x+) ---------------------------------------------------------

fn swing_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "delay",
        "vertical",
        "reverse",
        "background",
        "flatten",
        "old_widget",
        "new_widget",
    ];
    let mut vals = vec![
        Value::Float(1.),
        Value::Bool(false),
        Value::Bool(false),
        vm.types.new_str("#000"),
        Value::Bool(true),
        Value::None,
        Value::None,
    ];
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
    tr_init_base(vm, this, vals[0].clone(), &props)?;
    for (i, n) in names.iter().enumerate().skip(1) {
        py::set(vm, this, n, vals[i].clone())?;
    }
    py::set(vm, this, "events", Value::Bool(false))?;
    Ok(Value::None)
}

fn swing_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let delay = f(vm, this, "delay")?;
    if less_updates(vm) || st >= delay {
        return null_render(vm, this, w, h, st, at);
    }
    let mut t = st / delay;
    let reverse = {
        let v = py::get_or_none(vm, this, "reverse")?;
        ops::truthy(vm, &v)?
    };
    let vertical = {
        let v = py::get_or_none(vm, this, "vertical")?;
        ops::truthy(vm, &v)?
    };
    let (child, angle) = if t < 0.5 {
        (py::get(vm, this, "old_widget")?, t * 180.)
    } else {
        py::set(vm, this, "events", Value::Bool(true))?;
        t = 1. - t;
        (py::get(vm, this, "new_widget")?, t * 180.)
    };
    let angle = if reverse { -angle } else { angle };
    let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
    let key = if vertical { "xrotate" } else { "yrotate" };
    let tr = vm.call(
        &tc,
        vec![child],
        vec![
            (key.into(), Value::Float(angle)),
            ("xalign".into(), Value::Float(0.5)),
            ("yalign".into(), Value::Float(0.5)),
        ],
    )?;
    let bg = py::get_or_none(vm, this, "background")?;
    let bgd = super::easy::displayable(vm, &bg, &Value::None, true)?;
    let back = render::render(vm, &bgd, w, h, st, at)?;
    let front = render::render(vm, &tr, w, h, st, at)?;
    let mut rd = RenderData::new(w, h);
    let b = render::source_of(vm, &back)?;
    let f = render::source_of(vm, &front)?;
    rd.children.push(render::Blit {
        source: b,
        x: 0.,
        y: 0.,
        focus: false,
        main: false,
    });
    rd.children.push(render::Blit {
        source: f,
        x: 0.,
        y: 0.,
        focus: true,
        main: true,
    });
    render::redraw(vm, this, 0.);
    let _ = render_data;
    Ok(new_render(vm, rd))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("null_render", f_null_render),
            ("Fade", f_fade),
            ("ComposeTransition", f_compose),
            ("SubTransition", f_sub),
            ("interpolate_tuple", |vm, a, _| {
                let t0: Vec<f64> = py::items(vm, &a[0])?
                    .iter()
                    .map(|x| py::f64_of(vm, x))
                    .collect::<PyResult<_>>()?;
                let t1: Vec<f64> = py::items(vm, &a[1])?
                    .iter()
                    .map(|x| py::f64_of(vm, x))
                    .collect::<PyResult<_>>()?;
                let t = py::f64_of(vm, &a[2])?;
                let s: Vec<f64> = py::items(vm, &a[3])?
                    .iter()
                    .map(|x| py::f64_of(vm, x))
                    .collect::<PyResult<_>>()?;
                let r = interp_tuple(&t0, &t1, t, &s);
                Ok(vm
                    .types
                    .new_tuple(r.iter().map(|v| Value::Int(*v as i64)).collect()))
            }),
        ],
    );
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm,
             name: &str,
             bases: &[Value],
             methods: Methods,
             attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    let tr = c(
        vm,
        "Transition",
        &[d],
        &[
            ("__init__", tr_init),
            ("event", tr_event),
            ("visit", tr_visit),
            ("get_placement", tr_get_placement),
        ],
        &[("new_widget", Value::None), ("old_widget", Value::None)],
    )?;
    c(
        vm,
        "NoTransition",
        &[tr.clone()],
        &[("__init__", notrans_init), ("render", notrans_render)],
        &[],
    )?;
    c(
        vm,
        "MultipleTransition",
        &[tr.clone()],
        &[
            ("__init__", multiple_init),
            ("visit", multiple_visit),
            ("event", multiple_event),
            ("render", multiple_render),
        ],
        &[],
    )?;
    c(
        vm,
        "Pixellate",
        &[tr.clone()],
        &[("__init__", pixellate_init), ("render", pixellate_render)],
        &[],
    )?;
    c(
        vm,
        "Dissolve",
        &[tr.clone()],
        &[
            ("__init__", dissolve_init),
            ("after_upgrade", after_upgrade_alpha),
            ("render", dissolve_render),
        ],
        &[("__version__", Value::Int(1)), ("time_warp", Value::None)],
    )?;
    c(
        vm,
        "ImageDissolve",
        &[tr.clone()],
        &[
            ("__init__", imagedissolve_init),
            ("after_upgrade", after_upgrade_alpha),
            ("visit", imagedissolve_visit),
            ("render", imagedissolve_render),
        ],
        &[("__version__", Value::Int(1)), ("time_warp", Value::None)],
    )?;
    c(
        vm,
        "AlphaDissolve",
        &[tr.clone()],
        &[
            ("__init__", alphadissolve_init),
            ("visit", alphadissolve_visit),
            ("render", alphadissolve_render),
        ],
        &[("mipmap", Value::None)],
    )?;
    c(
        vm,
        "CropMove",
        &[tr.clone()],
        &[("__init__", cropmove_init), ("render", cropmove_render)],
        &[],
    )?;
    c(
        vm,
        "PushMove",
        &[tr.clone()],
        &[("__init__", pushmove_init), ("render", pushmove_render)],
        &[],
    )?;
    c(
        vm,
        "Swing",
        &[tr],
        &[("__init__", swing_init), ("render", swing_render)],
        &[],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
