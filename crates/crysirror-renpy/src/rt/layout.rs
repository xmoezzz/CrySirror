// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.layout`: containers and box layout, after Ren'Py 8.3
//! `layout.py` (MIT). Instance attributes (`children`, `child`, `offsets`,
//! `start_times`, ...) keep upstream names so saves and subclasses see the
//! same object shape.
use super::{
    core::{compute_raw, is_displayable},
    py::{self, *},
    render::{self, RenderData, new_render, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.layout";

pub fn style(vm: &mut Vm, this: &Value, name: &str) -> PyResult<Value> {
    let s = py::get(vm, this, "style")?;
    py::get(vm, &s, name)
}

pub fn style_f(vm: &mut Vm, this: &Value, name: &str, room: f64) -> PyResult<f64> {
    let v = style(vm, this, name)?;
    compute_raw(vm, &v, room)
}

pub fn style_b(vm: &mut Vm, this: &Value, name: &str) -> PyResult<bool> {
    let v = style(vm, this, name)?;
    ops::truthy(vm, &v)
}

pub fn render(vm: &mut Vm, d: &Value, w: f64, h: f64, st: f64, at: f64) -> PyResult<Value> {
    render::render(vm, d, w, h, st, at)
}

pub fn size(r: &Value) -> (f64, f64) {
    render_data(r)
        .map(|r| {
            let r = r.borrow();
            (r.width, r.height)
        })
        .unwrap_or((0., 0.))
}

pub fn new(vm: &Vm, w: f64, h: f64) -> Value {
    new_render(vm, RenderData::new(w, h))
}

pub fn blit(vm: &mut Vm, rv: &Value, src: &Value, x: f64, y: f64) -> PyResult<()> {
    let pos = vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]);
    vm.call_method(rv, "blit", vec![src.clone(), pos])?;
    Ok(())
}

pub fn blit_nofocus(vm: &mut Vm, rv: &Value, src: &Value, x: f64, y: f64) -> PyResult<()> {
    let pos = vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]);
    vm.call_method(rv, "blit", vec![src.clone(), pos, Value::Bool(false)])?;
    Ok(())
}

/// `child.place(rv, x, y, w, h, surf, main)`; returns the offset.
pub fn place(
    vm: &mut Vm,
    child: &Value,
    rv: &Value,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    surf: &Value,
    main: bool,
) -> PyResult<Value> {
    let f = py::get(vm, child, "place")?;
    vm.call(
        &f,
        vec![
            rv.clone(),
            Value::Float(x),
            Value::Float(y),
            Value::Float(w),
            Value::Float(h),
            surf.clone(),
        ],
        vec![("main".into(), Value::Bool(main))],
    )
}

pub fn displayable(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.easy", "displayable")?;
    vm.call(&f, vec![d.clone()], Vec::new())
}

fn cfg_b(vm: &mut Vm, name: &str, default: bool) -> bool {
    match py::config(vm, name) {
        Ok(v) => ops::truthy(vm, &v).unwrap_or(default),
        Err(_) => default,
    }
}

fn interface(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "interface")
}

fn iface_f(vm: &mut Vm, name: &str) -> PyResult<Option<f64>> {
    let i = interface(vm)?;
    if py::is_none(&i) {
        return Ok(None);
    }
    let v = py::get_or_none(vm, &i, name)?;
    if py::is_none(&v) {
        Ok(None)
    } else {
        py::f64_of(vm, &v).map(Some)
    }
}

fn is_float_type(v: &Value) -> bool {
    matches!(v, Value::Float(_))
}

/// `xyminimums(style, width, height)`.
pub fn xyminimums(vm: &mut Vm, this: &Value, width: f64, height: f64) -> PyResult<(f64, f64)> {
    let adjust = cfg_b(vm, "adjust_minimums", false);
    let mut out = [0.; 2];
    for (i, (minn, maxn, room)) in [
        ("xminimum", "xmaximum", width),
        ("yminimum", "ymaximum", height),
    ]
    .iter()
    .enumerate()
    {
        let mut m = style(vm, this, minn)?;
        if let Value::Float(mf) = m {
            let mx = style(vm, this, maxn)?;
            if let Value::Float(xf) = mx {
                if xf != 0. && adjust {
                    m = Value::Float(mf / xf);
                }
            }
        }
        out[i] = compute_raw(vm, &m, *room)?;
    }
    Ok((out[0], out[1]))
}

fn children(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let c = py::get(vm, this, "children")?;
    py::items(vm, &c)
}

fn set_offsets(vm: &mut Vm, this: &Value, offsets: Vec<Value>) -> PyResult<()> {
    let l = vm.types.new_list(offsets);
    py::set(vm, this, "offsets", l)
}

fn offset(vm: &Vm, x: f64, y: f64) -> Value {
    vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)])
}

fn invalidate(vm: &mut Vm, d: &Value) {
    render::invalidate(vm, d);
}

// Null -------------------------------------------------------------------------

fn null_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut width = args.get(1).cloned().unwrap_or(Value::Int(0));
    let mut height = args.get(2).cloned().unwrap_or(Value::Int(0));
    let mut props = Vec::new();
    for (k, v) in kw {
        match &**k {
            "width" => width = v.clone(),
            "height" => height = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "width", width)?;
    py::set(vm, this, "height", height)?;
    Ok(Value::None)
}

fn null_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let w = py::get(vm, this, "width")?;
    let h = py::get(vm, this, "height")?;
    let w = py::f64_of(vm, &w)?;
    let h = py::f64_of(vm, &h)?;
    let rv = new(vm, w, h);
    let focusable = py::get_or_none(vm, this, "focusable")?;
    if ops::truthy(vm, &focusable)? {
        vm.call_method(
            &rv,
            "add_focus",
            vec![
                this.clone(),
                Value::None,
                Value::None,
                Value::None,
                Value::None,
                Value::None,
            ],
        )?;
    }
    Ok(rv)
}

// Container ----------------------------------------------------------------------

pub fn container_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "children", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "child", Value::None)?;
    py::set(vm, this, "offsets", vm.types.new_list(Vec::new()))?;
    for c in &args[1..] {
        vm.call_method(this, "add", vec![c.clone()])?;
    }
    super::core::d_init(vm, &[this.clone()], kw)?;
    Ok(Value::None)
}

fn container_set_transform_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let event = arg_or(args, kw, 1, "event", Value::None);
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "set_transform_event")?;
    vm.call(&f, vec![this.clone(), event.clone()], Vec::new())?;
    let pass = py::config(vm, "containers_pass_transform_events").unwrap_or(Value::None);
    if !py::is_none(&pass) && ops::contains(vm, &pass, &event)? {
        for c in children(vm, this)? {
            vm.call_method(&c, "set_transform_event", vec![event.clone()])?;
        }
    }
    Ok(Value::None)
}

fn container_handles_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let event = arg_or(args, kw, 1, "event", Value::None);
    for c in children(vm, &args[0])? {
        let r = vm.call_method(&c, "_handles_event", vec![event.clone()])?;
        if ops::truthy(vm, &r)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn container_set_style_prefix(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prefix = arg_or(args, kw, 1, "prefix", Value::None);
    let root = arg_or(args, kw, 2, "root", Value::Bool(true));
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "set_style_prefix")?;
    vm.call(&f, vec![this.clone(), prefix.clone(), root], Vec::new())?;
    for c in children(vm, this)? {
        vm.call_method(
            &c,
            "set_style_prefix",
            vec![prefix.clone(), Value::Bool(false)],
        )?;
    }
    Ok(Value::None)
}

fn container_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = arg_or(args, kw, 1, "args", Value::None);
    if !py::is_none(&a) {
        let extra = py::get(vm, &a, "args")?;
        if ops::truthy(vm, &extra)? {
            vm.call_method(&a, "extraneous", Vec::new())?;
        }
    }
    let dup = py::get_or_none(vm, this, "_duplicatable")?;
    if !ops::truthy(vm, &dup)? {
        return Ok(this.clone());
    }
    let rv = vm.call_method(this, "_copy", vec![a.clone()])?;
    let mut new_children = Vec::new();
    let mut any = false;
    for c in children(vm, this)? {
        let n = vm.call_method(&c, "_duplicate", vec![a.clone()])?;
        let d = py::get_or_none(vm, &n, "_duplicatable")?;
        any |= ops::truthy(vm, &d)?;
        new_children.push(n);
    }
    let last = new_children.last().cloned();
    py::set(vm, &rv, "children", vm.types.new_list(new_children))?;
    if let Some(l) = last {
        py::set(vm, &rv, "child", l)?;
    }
    py::set(vm, &rv, "_duplicatable", Value::Bool(any))?;
    Ok(rv)
}

fn container_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for c in children(vm, &args[0])? {
        vm.call_method(&c, "_unique", Vec::new())?;
    }
    py::set(vm, &args[0], "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn container_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut changed = false;
    let mut out = Vec::new();
    for old in children(vm, this)? {
        let n = vm.call_method(&old, "_in_current_store", Vec::new())?;
        changed |= !py::same(&old, &n);
        out.push(n);
    }
    if !changed {
        return Ok(this.clone());
    }
    let rv = vm.call_method(this, "_copy", Vec::new())?;
    let last = out.last().cloned();
    py::set(vm, &rv, "children", vm.types.new_list(out))?;
    if let Some(l) = last {
        py::set(vm, &rv, "child", l)?;
    }
    Ok(rv)
}

fn container_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = arg_or(args, kw, 1, "d", Value::None);
    let child = displayable(vm, &d)?;
    let list = py::get(vm, this, "children")?;
    ops::list_append(vm, &list, child.clone())?;
    py::set(vm, this, "child", child.clone())?;
    py::set(vm, this, "offsets", vm.types.new_list(Vec::new()))?;
    let dup = py::get_or_none(vm, &child, "_duplicatable")?;
    if ops::truthy(vm, &dup)? {
        py::set(vm, this, "_duplicatable", Value::Bool(true))?;
    }
    invalidate(vm, this);
    Ok(Value::None)
}

fn container_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "child", Value::None)?;
    py::set(vm, this, "children", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "offsets", vm.types.new_list(Vec::new()))?;
    render::redraw(vm, this, 0.);
    Ok(Value::None)
}

fn container_remove(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = arg_or(args, kw, 1, "d", Value::None);
    let mut ch = children(vm, this)?;
    let Some(i) = ch.iter().position(|c| py::same(c, &d)) else {
        return Ok(Value::None);
    };
    ch.remove(i);
    let last = ch.last().cloned().unwrap_or(Value::None);
    py::set(vm, this, "children", vm.types.new_list(ch))?;
    py::set(vm, this, "offsets", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "child", last)?;
    Ok(Value::None)
}

fn container_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    invalidate(vm, &args[0]);
    Ok(Value::None)
}

fn container_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let rv = new(vm, w, h);
    let mut offsets = Vec::new();
    for c in children(vm, this)? {
        let cr = render(vm, &c, w, h, st, at)?;
        offsets.push(place(vm, &c, &rv, 0., 0., w, h, &cr, true)?);
    }
    set_offsets(vm, this, offsets)?;
    Ok(rv)
}

pub fn rargs(vm: &mut Vm, args: &[Value]) -> PyResult<(f64, f64, f64, f64)> {
    let g = |vm: &mut Vm, i: usize| -> PyResult<f64> {
        py::f64_of(vm, &args.get(i).cloned().unwrap_or(Value::Int(0)))
    };
    Ok((g(vm, 1)?, g(vm, 2)?, g(vm, 3)?, g(vm, 4)?))
}

/// Dispatches `ev` to children in reverse order using `self.offsets`.
pub fn container_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let st = args.get(4).cloned().unwrap_or(Value::Int(0));
    let ch = children(vm, this)?;
    let offs = py::get(vm, this, "offsets")?;
    let offs = py::items(vm, &offs)?;
    if offs.len() != ch.len() {
        return Ok(Value::None);
    }
    for i in (0..ch.len()).rev() {
        let o = py::items(vm, &offs[i])?;
        let (xo, yo) = (py::f64_of(vm, &o[0])?, py::f64_of(vm, &o[1])?);
        let rv = vm.call_method(
            &ch[i],
            "event",
            vec![
                ev.clone(),
                Value::Float(x - xo),
                Value::Float(y - yo),
                st.clone(),
            ],
        )?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    Ok(Value::None)
}

fn container_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = children(vm, &args[0])?;
    Ok(vm.types.new_list(c))
}

fn container_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.ui", "context_enter")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())?;
    Ok(args[0].clone())
}

fn container_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.ui", "context_exit")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())?;
    Ok(Value::Bool(false))
}

// Composite / Position -------------------------------------------------------------

fn f_composite(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let size = args
        .first()
        .cloned()
        .ok_or_else(|| vm.type_error("Composite() requires a size"))?;
    let sz = py::items(vm, &size)?;
    if sz.len() != 2 {
        return Err(vm.type_error("Composite size must be (width, height)"));
    }
    let mut props: Vec<(std::rc::Rc<str>, Value)> = kw.to_vec();
    if !props.iter().any(|(k, _)| &**k == "style") {
        props.push(("style".into(), vm.types.new_str("image_placement")));
    }
    for (n, v) in [
        ("xmaximum", &sz[0]),
        ("ymaximum", &sz[1]),
        ("xminimum", &sz[0]),
        ("yminimum", &sz[1]),
    ] {
        props.push((n.into(), v.clone()));
    }
    props.push(("layout".into(), vm.types.new_str("fixed")));
    let mb = py::import_attr(vm, MODULE, "MultiBox")?;
    let rv = vm.call(&mb, Vec::new(), props)?;
    let rest = &args[1..];
    if rest.len() % 2 != 0 {
        return Err(py::error(
            vm,
            "LiveComposite requires an odd number of arguments.",
        ));
    }
    let position = py::import_attr(vm, MODULE, "Position")?;
    for pair in rest.chunks(2) {
        let p = py::items(vm, &pair[0])?;
        if p.len() != 2 {
            return Err(vm.type_error("Composite position must be (x, y)"));
        }
        let pos = vm.call(
            &position,
            vec![pair[1].clone()],
            vec![
                ("xpos".into(), p[0].clone()),
                ("xanchor".into(), Value::Int(0)),
                ("ypos".into(), p[1].clone()),
                ("yanchor".into(), Value::Int(0)),
            ],
        )?;
        vm.call_method(&rv, "add", vec![pos])?;
    }
    Ok(rv)
}

fn position_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "child")
        .cloned()
        .collect();
    if !props.iter().any(|(k, _)| &**k == "style") {
        let s = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| vm.types.new_str("image_placement"));
        props.push(("style".into(), s));
    }
    container_init(vm, &[this.clone()], &props)?;
    vm.call_method(this, "add", vec![child])?;
    Ok(Value::None)
}

fn single_child_render(vm: &mut Vm, args: &[Value], focus: bool) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let child = py::get(vm, this, "child")?;
    let surf = render(vm, &child, w, h, st, at)?;
    let (sw, sh) = size(&surf);
    let rv = new(vm, sw, sh);
    if focus {
        blit(vm, &rv, &surf, 0., 0.)?
    } else {
        blit_nofocus(vm, &rv, &surf, 0., 0.)?
    }
    let o = offset(vm, 0., 0.);
    set_offsets(vm, this, vec![o])?;
    Ok(rv)
}

fn position_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    single_child_render(vm, args, true)
}

fn position_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = py::get(vm, this, "child")?;
    let p = vm.call_method(&child, "get_placement", Vec::new())?;
    let mut p = py::items(vm, &p)?;
    while p.len() < 7 {
        p.push(Value::None);
    }
    for i in [4, 5] {
        if py::is_none(&p[i]) {
            p[i] = Value::Int(0);
        }
    }
    for (i, n) in [(0, "xpos"), (1, "ypos"), (2, "xanchor"), (3, "yanchor")] {
        let v = style(vm, this, n)?;
        if !py::is_none(&v) {
            p[i] = v;
        }
    }
    for (i, n) in [(4, "xoffset"), (5, "yoffset")] {
        let v = style(vm, this, n)?;
        if !py::is_none(&v) {
            p[i] = ops::binary_op(
                vm,
                &p[i],
                &v,
                crysirror_python::ast::BinOp::Add,
                false,
                false,
            )?;
        }
    }
    let sp = style(vm, this, "subpixel")?;
    if !ops::truthy(vm, &p[6])? && !py::is_none(&sp) {
        p[6] = sp;
    }
    Ok(vm.types.new_tuple(p))
}

fn child_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let child = py::get(vm, &args[0], "child")?;
    vm.call_method(&child, "get_placement", Vec::new())
}

// Grid -----------------------------------------------------------------------------

fn grid_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut a = [
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::None,
        Value::None,
    ];
    let names = [
        "cols",
        "rows",
        "padding",
        "transpose",
        "style",
        "allow_underfull",
    ];
    for (i, v) in args[1..].iter().enumerate().take(6) {
        a[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => a[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    if !py::is_none(&a[2]) && !props.iter().any(|(k, _)| &**k == "spacing") {
        props.push(("spacing".into(), a[2].clone()));
    }
    let style_v = if py::is_none(&a[4]) {
        vm.types.new_str("grid")
    } else {
        a[4].clone()
    };
    props.push(("style".into(), style_v));
    container_init(vm, &[this.clone()], &props)?;
    let cols = py::int(vm, &a[0])?;
    let rows = py::int(vm, &a[1])?;
    py::set(vm, this, "cols", Value::Int(cols))?;
    py::set(vm, this, "rows", Value::Int(rows))?;
    py::set(vm, this, "transpose", a[3].clone())?;
    py::set(vm, this, "allow_underfull", a[5].clone())?;
    Ok(Value::None)
}

fn spacing(vm: &mut Vm, this: &Value, axis: &str, room: f64) -> PyResult<f64> {
    let mut s = style(vm, this, &format!("{axis}spacing"))?;
    if py::is_none(&s) {
        s = style(vm, this, "spacing")?;
    }
    if cfg_b(vm, "relative_spacing", false) {
        return compute_raw(vm, &s, room);
    }
    if py::is_none(&s) {
        Ok(0.)
    } else {
        py::f64_of(vm, &s)
    }
}

fn grid_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (width, height, st, at) = rargs(vm, args)?;
    let xspacing = spacing(vm, this, "x", width)?;
    let yspacing = spacing(vm, this, "y", height)?;
    let lm = style_f(vm, this, "left_margin", width)?;
    let rm = style_f(vm, this, "right_margin", width)?;
    let tm = style_f(vm, this, "top_margin", height)?;
    let bm = style_f(vm, this, "bottom_margin", height)?;
    let cols = {
        let v = py::get(vm, this, "cols")?;
        py::int(vm, &v)? as usize
    };
    let rows = {
        let v = py::get(vm, this, "rows")?;
        py::int(vm, &v)? as usize
    };
    let transpose = {
        let v = py::get_or_none(vm, this, "transpose")?;
        ops::truthy(vm, &v)?
    };
    let all = children(vm, this)?;
    if all.len() < cols * rows {
        return Err(py::error(vm, "Grid not completely full."));
    }
    let ch: Vec<Value> = if transpose {
        let mut v = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                v.push(all[y + x * rows].clone());
            }
        }
        v
    } else {
        all.clone()
    };
    let xfill = style_b(vm, this, "xfill")?;
    let yfill = style_b(vm, this, "yfill")?;
    let mut renwidth = width;
    let mut renheight = height;
    if xfill && cols > 0 {
        renwidth = ((width - (cols as f64 - 1.) * xspacing - lm - rm) / cols as f64).floor();
    }
    if yfill && rows > 0 {
        renheight = ((height - (rows as f64 - 1.) * yspacing - tm - bm) / rows as f64).floor();
    }
    let mut renders = Vec::new();
    for c in ch.iter().take(cols * rows) {
        renders.push(render(vm, c, renwidth, renheight, st, at)?);
    }
    let (mut cwidth, mut cheight) = (0f64, 0f64);
    for r in &renders {
        let (w, h) = size(r);
        cwidth = cwidth.max(w);
        cheight = cheight.max(h);
    }
    if xfill {
        cwidth = renwidth;
    }
    if yfill {
        cheight = renheight;
    }
    let w = cwidth * cols as f64 + xspacing * (cols as f64 - 1.) + lm + rm;
    let h = cheight * rows as f64 + yspacing * (rows as f64 - 1.) + tm + bm;
    let rv = new(vm, w, h);
    let mut offsets = Vec::new();
    for y in 0..rows {
        for x in 0..cols {
            let i = x + y * cols;
            let xpos = x as f64 * (cwidth + xspacing) + lm;
            let ypos = y as f64 * (cheight + yspacing) + tm;
            offsets.push(place(
                vm,
                &ch[i],
                &rv,
                xpos,
                ypos,
                cwidth,
                cheight,
                &renders[i],
                true,
            )?);
        }
    }
    if transpose {
        let mut o = Vec::new();
        for x in 0..cols {
            for y in 0..rows {
                o.push(offsets[y * cols + x].clone());
            }
        }
        offsets = o;
    }
    set_offsets(vm, this, offsets)?;
    Ok(rv)
}

fn grid_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    container_add(vm, args, kw)?;
    let this = &args[0];
    let n = children(vm, this)?.len() as i64;
    let cols = {
        let v = py::get(vm, this, "cols")?;
        py::int(vm, &v)?
    };
    let rows = {
        let v = py::get(vm, this, "rows")?;
        py::int(vm, &v)?
    };
    if n > cols * rows {
        return Err(py::error(vm, "Grid overfull."));
    }
    Ok(Value::None)
}

fn grid_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let n = children(vm, this)?.len() as i64;
    let cols = {
        let v = py::get(vm, this, "cols")?;
        py::int(vm, &v)?
    };
    let rows = {
        let v = py::get(vm, this, "rows")?;
        py::int(vm, &v)?
    };
    let delta = cols * rows - n;
    if delta <= 0 {
        return Ok(Value::None);
    }
    if cfg_b(vm, "developer", false) {
        let mut allow = py::get_or_none(vm, this, "allow_underfull")?;
        if py::is_none(&allow) {
            allow = py::config(vm, "allow_underfull_grids").unwrap_or(Value::Bool(false));
        }
        if !ops::truthy(vm, &allow)? {
            return Err(py::error(vm, "Grid not completely full."));
        }
    }
    let null_c = py::import_attr(vm, MODULE, "Null")?;
    let null = vm.call(&null_c, Vec::new(), Vec::new())?;
    for _ in 0..delta {
        vm.call_method(this, "add", vec![null.clone()])?;
    }
    Ok(Value::None)
}

// Modal ------------------------------------------------------------------------------

fn f_default_modal_function(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let w = args.get(3).cloned().unwrap_or(Value::None);
    let h = args.get(4).cloned().unwrap_or(Value::None);
    if py::is_none(&w) {
        return Ok(Value::Bool(true));
    }
    let w = py::f64_of(vm, &w)?;
    let h = py::f64_of(vm, &h)?;
    Ok(Value::Bool(0. <= x && x < w && 0. <= y && y < h))
}

fn f_check_modal(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = |i: usize| args.get(i).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &a(2)).unwrap_or(0.);
    let y = py::f64_of(vm, &a(3)).unwrap_or(0.);
    let w = if py::is_none(&a(4)) {
        None
    } else {
        Some(py::f64_of(vm, &a(4))?)
    };
    let h = if py::is_none(&a(5)) {
        None
    } else {
        Some(py::f64_of(vm, &a(5))?)
    };
    Ok(Value::Bool(super::focus::check_modal(
        vm,
        &a(0),
        &a(1),
        x,
        y,
        w,
        h,
    )?))
}

// MultiBox -----------------------------------------------------------------------------

fn multibox_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut spacing = args.get(1).cloned().unwrap_or(Value::None);
    let mut layout = args.get(2).cloned().unwrap_or(Value::None);
    let mut style_v = args.get(3).cloned();
    let mut props = Vec::new();
    for (k, v) in kw {
        match &**k {
            "spacing" => spacing = v.clone(),
            "layout" => layout = v.clone(),
            "style" => style_v = Some(v.clone()),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    if !py::is_none(&spacing) {
        props.push(("spacing".into(), spacing));
    }
    props.push((
        "style".into(),
        style_v.unwrap_or_else(|| vm.types.new_str("default")),
    ));
    // Container.__init__ before start_times exist: add() appends to them.
    py::set(vm, this, "start_times", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "anim_times", vm.types.new_list(Vec::new()))?;
    container_init(vm, &[this.clone()], &props)?;
    let clipping = style(vm, this, "clipping")?;
    py::set(vm, this, "_clipping", clipping)?;
    py::set(vm, this, "default_layout", layout)?;
    py::set(vm, this, "layers", Value::None)?;
    py::set(vm, this, "scene_list", Value::None)?;
    py::set(vm, this, "adjust_times", Value::Bool(false))?;
    Ok(Value::None)
}

fn multibox_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    container_clear(vm, args, kw)?;
    let this = &args[0];
    py::set(vm, this, "start_times", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "anim_times", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "layers", Value::None)?;
    py::set(vm, this, "scene_list", Value::None)?;
    Ok(Value::None)
}

fn multibox_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let w = arg_or(args, kw, 1, "widget", Value::None);
    let s = arg_or(args, kw, 2, "start_time", Value::None);
    let a = arg_or(args, kw, 3, "anim_time", Value::None);
    container_add(vm, &[this.clone(), w], &[])?;
    let st = py::get(vm, this, "start_times")?;
    ops::list_append(vm, &st, s)?;
    let an = py::get(vm, this, "anim_times")?;
    ops::list_append(vm, &an, a)?;
    Ok(Value::None)
}

fn multibox_append_scene_list(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let l = args.get(1).cloned().unwrap_or(Value::None);
    let items = py::items(vm, &l)?;
    for sle in &items {
        let d = py::get(vm, sle, "displayable")?;
        let s = py::get(vm, sle, "show_time")?;
        let a = py::get(vm, sle, "animation_time")?;
        vm.call_method(this, "add", vec![d, s, a])?;
    }
    let sl = py::get_or_none(vm, this, "scene_list")?;
    let sl = if py::is_none(&sl) {
        let n = vm.types.new_list(Vec::new());
        py::set(vm, this, "scene_list", n.clone())?;
        n
    } else {
        sl
    };
    vm.call_method(&sl, "extend", vec![l])?;
    Ok(Value::None)
}

fn multibox_update_times(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let Some(it) = iface_f(vm, "interact_time")? else {
        return Ok(Value::None);
    };
    for name in ["start_times", "anim_times"] {
        let l = py::get(vm, this, name)?;
        let items = py::items(vm, &l)?;
        let out: Vec<Value> = items
            .into_iter()
            .map(|v| if py::is_none(&v) { Value::Float(it) } else { v })
            .collect();
        py::set(vm, this, name, vm.types.new_list(out))?;
    }
    py::set(vm, this, "first", Value::Bool(false))?;
    Ok(Value::None)
}

fn multibox_classname(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cls = this.obj().map(|o| o.class_name()).unwrap_or_default();
    if cls != "MultiBox" {
        return Ok(vm.types.new_string(cls));
    }
    let mut layout = style(vm, this, "box_layout")?;
    if py::is_none(&layout) {
        layout = py::get_or_none(vm, this, "default_layout")?;
    }
    Ok(vm.types.new_str(match layout.as_str() {
        Some("fixed") => "Fixed",
        Some("horizontal") => "HBox",
        Some("vertical") => "VBox",
        _ => "MultiBox",
    }))
}

struct LineItem {
    child: Value,
    pad: (f64, f64),
    x: f64,
    y: f64,
    surf: Value,
}

struct Placement {
    child: Value,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    surf: Value,
}

fn multibox_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (mut width, mut height, st, at) = rargs(vm, args)?;
    let layer_name = py::get_or_none(vm, this, "layer_name")?;
    let layers = py::get_or_none(vm, this, "layers")?;
    let adjust_flag = py::get_or_none(vm, this, "adjust_times")?;
    let adjust_times =
        ops::truthy(vm, &adjust_flag)? || ops::truthy(vm, &layer_name)? || !py::is_none(&layers);
    let minx_v = style(vm, this, "xminimum")?;
    if !py::is_none(&minx_v) {
        width = width.max(compute_raw(vm, &minx_v, width)?);
    }
    let miny_v = style(vm, this, "yminimum")?;
    if !py::is_none(&miny_v) {
        height = height.max(compute_raw(vm, &miny_v, height)?);
    }
    let first = py::get_or_none(vm, this, "first")?;
    if ops::truthy(vm, &first)? && adjust_times {
        multibox_update_times(vm, &[this.clone()], &[])?;
    }
    let mut layout = style(vm, this, "box_layout")?;
    if py::is_none(&layout) {
        layout = py::get_or_none(vm, this, "default_layout")?;
    }
    let ch = children(vm, this)?;
    let n = ch.len();
    let (csts, cats): (Vec<f64>, Vec<f64>) = if adjust_times {
        let ft = iface_f(vm, "frame_time")?.unwrap_or(0.);
        let adj = |vm: &mut Vm, t: &Value, tb: f64| -> PyResult<f64> {
            Ok(match t {
                Value::None => 0.,
                Value::Bool(true) => tb,
                v => ft - py::f64_of(vm, v)?,
            })
        };
        let s = py::get(vm, this, "start_times")?;
        let s = py::items(vm, &s)?;
        let a = py::get(vm, this, "anim_times")?;
        let a = py::items(vm, &a)?;
        let mut cs = Vec::new();
        let mut ca = Vec::new();
        for i in 0..n {
            cs.push(adj(vm, s.get(i).unwrap_or(&Value::None), st)?);
            ca.push(adj(vm, a.get(i).unwrap_or(&Value::None), at)?);
        }
        (cs, ca)
    } else {
        (vec![st; n], vec![at; n])
    };
    let order_reverse = style_b(vm, this, "order_reverse")?;
    if layout.as_str() == Some("fixed") {
        let mut rd = RenderData::new(width, height);
        rd.layer_name = layer_name.clone();
        let rv = new_render(vm, rd);
        let xfit = style_b(vm, this, "xfit")?;
        let yfit = style_b(vm, this, "yfit")?;
        let fit_first = style(vm, this, "fit_first")?;
        let (mut ffw, mut ffh): (Option<bool>, Option<bool>) = match fit_first.as_str() {
            Some("width") => (Some(true), Some(false)),
            Some("height") => (Some(false), Some(true)),
            _ => {
                let t = ops::truthy(vm, &fit_first)?;
                (Some(t), Some(t))
            }
        };
        let mut order: Vec<usize> = (0..n).collect();
        if order_reverse {
            order.reverse();
        }
        let mut sizes = Vec::new();
        let mut offsets = Vec::new();
        for i in order {
            let child = &ch[i];
            let surf = render(vm, child, width, height, csts[i], cats[i])?;
            let sz = size(&surf);
            sizes.push(sz);
            if ffw == Some(true) {
                width = sz.0;
                py::set(vm, &rv, "width", Value::Float(width))?;
                ffw = None;
            }
            if ffh == Some(true) {
                height = sz.1;
                py::set(vm, &rv, "height", Value::Float(height))?;
                ffh = None;
            }
            offsets.push(place(vm, child, &rv, 0., 0., width, height, &surf, true)?);
        }
        let max_fit = py::config_f64(vm, "max_fit_size").unwrap_or(8192.);
        if xfit {
            let mut w = 0f64;
            for (o, s) in offsets.iter().zip(&sizes) {
                let o = py::items(vm, o)?;
                w = w.max(py::f64_of(vm, &o[0])? + s.0);
                if ffw.is_none() {
                    break;
                }
            }
            if w > max_fit {
                return Err(py::error(
                    vm,
                    format!("Fixed fit width ({w}) is too large."),
                ));
            }
            py::set(vm, &rv, "width", Value::Float(w))?;
        }
        if yfit {
            let mut h = 0f64;
            for (o, s) in offsets.iter().zip(&sizes) {
                let o = py::items(vm, o)?;
                h = h.max(py::f64_of(vm, &o[1])? + s.1);
                if ffh.is_none() {
                    break;
                }
            }
            if h > max_fit {
                return Err(py::error(
                    vm,
                    format!("Fixed fit height ({h}) is too large."),
                ));
            }
            py::set(vm, &rv, "height", Value::Float(h))?;
        }
        if order_reverse {
            offsets.reverse();
        }
        set_offsets(vm, this, offsets)?;
        return Ok(rv);
    }
    let sp_v = style(vm, this, "spacing")?;
    let sp = if py::is_none(&sp_v) {
        0.
    } else {
        py::f64_of(vm, &sp_v)?
    };
    let fs_v = style(vm, this, "first_spacing")?;
    let fsp = if py::is_none(&fs_v) {
        sp
    } else {
        py::f64_of(vm, &fs_v)?
    };
    let mut spacings: Vec<f64> = (0..n).map(|i| if i == 0 { fsp } else { sp }).collect();
    let box_wrap = style_b(vm, this, "box_wrap")?;
    let bws_v = style(vm, this, "box_wrap_spacing")?;
    let box_wrap_spacing = if py::is_none(&bws_v) {
        0.
    } else {
        py::f64_of(vm, &bws_v)?
    };
    let xfill = style_b(vm, this, "xfill")?;
    let yfill = style_b(vm, this, "yfill")?;
    let xmin_v = style(vm, this, "xminimum")?;
    let xminimum = compute_raw(vm, &xmin_v, width)?;
    let ymin_v = style(vm, this, "yminimum")?;
    let yminimum = compute_raw(vm, &ymin_v, height)?;
    let box_justify = style(vm, this, "box_justify")?;
    let ba_v = style(vm, this, "box_align")?;
    let (mut box_align, use_box_align) = if py::is_none(&ba_v) {
        (0., false)
    } else {
        (py::f64_of(vm, &ba_v)?, true)
    };
    let box_reverse = style_b(vm, this, "box_reverse")?;
    let simple_box_reverse = cfg_b(vm, "simple_box_reverse", false);
    let box_reverse_align = cfg_b(vm, "box_reverse_align", false);
    let box_skip_cfg = cfg_b(vm, "box_skip", true);
    let mut kids: Vec<usize> = (0..n).collect();
    if box_reverse && simple_box_reverse {
        kids.reverse();
        spacings.reverse();
    } else if box_reverse && !box_reverse_align {
        box_align = 1. - box_align;
    }
    let justify_all =
        matches!(box_justify, Value::Bool(true)) || box_justify.as_str() == Some("all");
    let justify_first = box_justify.as_str() == Some("first");
    let mut line_placements: Vec<Vec<Placement>> = Vec::new();
    let mut line_widths = Vec::new();
    let mut line_heights = Vec::new();
    let mut line_use_justify = Vec::new();
    let (mut maxx, mut maxy) = (0f64, 0f64);
    let (mut minx, mut miny) = (0f64, 0f64);
    let mut line_width = 0f64;
    let mut line_height = 0f64;
    let mut use_justify = false;
    let horizontal = match layout.as_str() {
        Some("horizontal") => true,
        Some("vertical") => false,
        _ => {
            let r = ops::repr(vm, &layout)?;
            return Err(py::error(vm, format!("Unknown box layout: {r}")));
        }
    };
    #[allow(clippy::too_many_arguments)]
    fn layout_line(
        vm: &mut Vm,
        line: Vec<LineItem>,
        xfill: f64,
        yfill: f64,
        use_justify: bool,
        use_box_align: bool,
        box_skip_cfg: bool,
        line_width: f64,
        line_height: f64,
        out: &mut (Vec<Vec<Placement>>, Vec<f64>, Vec<f64>, Vec<bool>),
        maxx: f64,
        maxy: f64,
    ) -> PyResult<(f64, f64)> {
        let xfill = xfill.max(0.);
        let yfill = yfill.max(0.);
        let mut skips = Vec::new();
        for it in &line {
            let b = py::get_or_none(vm, &it.child, "_box_skip")?;
            skips.push(ops::truthy(vm, &b)?);
        }
        let count = if box_skip_cfg {
            skips.iter().filter(|s| !**s).count()
        } else {
            line.len()
        };
        let (xper, yper) = if use_justify || use_box_align || count == 0 {
            (0., 0.)
        } else {
            (xfill / count as f64, yfill / count as f64)
        };
        let mut placements = Vec::new();
        let (mut lmx, mut lmy) = (0f64, 0f64);
        let mut i = 0f64;
        for (it, skip) in line.into_iter().zip(skips) {
            let (sw0, sh0) = size(&it.surf);
            let mut sw = line_width.max(sw0);
            let mut sh = line_height.max(sh0);
            let (mut x, mut y) = (it.x, it.y);
            if !skip || !box_skip_cfg {
                x += (i * xper).trunc();
                y += (i * yper).trunc();
                sw += xper;
                sh += yper;
                x += it.pad.0;
                y += it.pad.1;
                i += 1.;
            }
            lmx = x + sw;
            lmy = y + sh;
            placements.push(Placement {
                child: it.child,
                x,
                y,
                w: sw,
                h: sh,
                surf: it.surf,
            });
        }
        out.0.push(placements);
        out.1.push(lmx);
        out.2.push(lmy);
        out.3.push(use_justify);
        Ok((maxx.max(lmx), maxy.max(lmy)))
    }
    let mut out = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut x, mut y) = (0f64, 0f64);
    let mut line: Vec<LineItem> = Vec::new();
    let mut next_padding = 0f64;
    let mut first_line = true;
    if horizontal {
        miny = if yfill { height } else { yminimum };
        let target_width = if xfill { width } else { xminimum };
        let mut remwidth = width;
        for (k, &i) in kids.iter().enumerate() {
            let d = &ch[i];
            use_justify = justify_all || (justify_first && first_line);
            let skip = {
                let b = py::get_or_none(vm, d, "_box_skip")?;
                ops::truthy(vm, &b)?
            };
            let mut padding = spacings[k];
            if skip && box_skip_cfg {
                padding = 0.;
            }
            let rw = if box_wrap {
                width
            } else {
                remwidth - next_padding
            };
            let mut surf = render(vm, d, rw, height - y, csts[i], cats[i])?;
            let (mut sw, mut sh) = size(&surf);
            if box_wrap && remwidth - sw - next_padding < 0. && !line.is_empty() {
                (maxx, maxy) = layout_line(
                    vm,
                    std::mem::take(&mut line),
                    target_width - x,
                    0.,
                    use_justify,
                    use_box_align,
                    box_skip_cfg,
                    line_width,
                    line_height,
                    &mut out,
                    maxx,
                    maxy,
                )?;
                y += line_height + box_wrap_spacing;
                x = 0.;
                line_height = 0.;
                remwidth = width;
                first_line = false;
                next_padding = 0.;
                surf = render(vm, d, rw, height - y, csts[i], cats[i])?;
                (sw, sh) = size(&surf);
            }
            line.push(LineItem {
                child: d.clone(),
                pad: (next_padding, 0.),
                x,
                y,
                surf,
            });
            line_height = line_height.max(sh);
            x += sw + next_padding;
            remwidth -= sw + next_padding;
            next_padding = padding;
        }
        use_justify = box_justify.as_str() == Some("all");
        (maxx, maxy) = layout_line(
            vm,
            line,
            target_width - x,
            0.,
            use_justify,
            use_box_align,
            box_skip_cfg,
            line_width,
            line_height,
            &mut out,
            maxx,
            maxy,
        )?;
    } else {
        minx = if xfill { width } else { xminimum };
        let target_height = if yfill { height } else { yminimum };
        let mut remheight = height;
        for (k, &i) in kids.iter().enumerate() {
            let d = &ch[i];
            use_justify = justify_all || (justify_first && first_line);
            let skip = {
                let b = py::get_or_none(vm, d, "_box_skip")?;
                ops::truthy(vm, &b)?
            };
            let mut padding = spacings[k];
            if skip && box_skip_cfg {
                padding = 0.;
            }
            let rh = if box_wrap {
                height
            } else {
                remheight - next_padding
            };
            let mut surf = render(vm, d, width - x, rh, csts[i], cats[i])?;
            let (mut sw, mut sh) = size(&surf);
            if box_wrap && remheight - sh - next_padding < 0. && !line.is_empty() {
                (maxx, maxy) = layout_line(
                    vm,
                    std::mem::take(&mut line),
                    0.,
                    target_height - y,
                    use_justify,
                    use_box_align,
                    box_skip_cfg,
                    line_width,
                    line_height,
                    &mut out,
                    maxx,
                    maxy,
                )?;
                x += line_width + box_wrap_spacing;
                y = 0.;
                line_width = 0.;
                remheight = height;
                first_line = false;
                next_padding = 0.;
                surf = render(vm, d, width - x, rh, csts[i], cats[i])?;
                (sw, sh) = size(&surf);
            }
            line.push(LineItem {
                child: d.clone(),
                pad: (0., next_padding),
                x,
                y,
                surf,
            });
            line_width = line_width.max(sw);
            y += sh + next_padding;
            remheight -= sh + next_padding;
            next_padding = padding;
        }
        use_justify = box_justify.as_str() == Some("all");
        (maxx, maxy) = layout_line(
            vm,
            line,
            0.,
            target_height - y,
            use_justify,
            use_box_align,
            box_skip_cfg,
            line_width,
            line_height,
            &mut out,
            maxx,
            maxy,
        )?;
    }
    let (line_placements_v, line_widths_v, line_heights_v, line_use_justify_v) = out;
    line_placements.extend(line_placements_v);
    line_widths.extend(line_widths_v);
    line_heights.extend(line_heights_v);
    line_use_justify.extend(line_use_justify_v);
    if !xfill {
        width = xminimum.max(maxx);
    }
    if !yfill {
        height = yminimum.max(maxy);
    }
    let mut placements: Vec<Placement> = Vec::new();
    if !(use_justify || use_box_align) {
        placements = line_placements.into_iter().flatten().collect();
    } else {
        for (k, line) in line_placements.into_iter().enumerate() {
            let remaining = if horizontal {
                width - line_widths[k]
            } else {
                height - line_heights[k]
            };
            let lj = line_use_justify[k];
            let n = line.len();
            let (jpc, align_offset) = if lj && n > 1 {
                (remaining / (n as f64 - 1.), 0.)
            } else {
                (0., (remaining * box_align).trunc())
            };
            for (i, mut p) in line.into_iter().enumerate() {
                let d = align_offset + (i as f64 * jpc).trunc();
                if horizontal {
                    p.x += d
                } else {
                    p.y += d
                }
                placements.push(p);
            }
        }
    }
    if box_reverse && !simple_box_reverse {
        for p in &mut placements {
            if horizontal {
                p.x = width - p.x - p.w;
            } else {
                p.y = height - p.y - p.h;
            }
        }
    }
    let rv = new(vm, width, height);
    if order_reverse ^ (box_reverse && simple_box_reverse) {
        placements.reverse();
    }
    let mut offsets = Vec::new();
    for p in &placements {
        let w = minx.max(p.w);
        let h = miny.max(p.h);
        offsets.push(place(vm, &p.child, &rv, p.x, p.y, w, h, &p.surf, true)?);
    }
    if order_reverse {
        offsets.reverse();
    }
    set_offsets(vm, this, offsets)?;
    Ok(rv)
}

fn multibox_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let st = py::f64_of(vm, &args.get(4).cloned().unwrap_or(Value::Int(0)))?;
    let first = py::get_or_none(vm, this, "first")?;
    let layer_name = py::get_or_none(vm, this, "layer_name")?;
    let layers = py::get_or_none(vm, this, "layers")?;
    if ops::truthy(vm, &first)? && (ops::truthy(vm, &layer_name)? || !py::is_none(&layers)) {
        multibox_update_times(vm, &[this.clone()], &[])?;
    }
    let ch = children(vm, this)?;
    let offs = py::get(vm, this, "offsets")?;
    let offs = py::items(vm, &offs)?;
    let starts = py::get(vm, this, "start_times")?;
    let starts = py::items(vm, &starts)?;
    let n = ch.len().min(offs.len()).min(starts.len().max(ch.len()));
    let mut idx: Vec<usize> = (0..n.min(offs.len())).collect();
    if !style_b(vm, this, "order_reverse")? {
        idx.reverse();
    }
    let event_time = iface_f(vm, "event_time")?.unwrap_or(0.);
    for i in idx {
        let o = py::items(vm, &offs[i])?;
        let (xo, yo) = (py::f64_of(vm, &o[0])?, py::f64_of(vm, &o[1])?);
        let t = starts.get(i).cloned().unwrap_or(Value::None);
        let cst = match t {
            Value::None => st,
            Value::Bool(true) => st,
            v => event_time - py::f64_of(vm, &v)?,
        };
        let r = vm.call_method(
            &ch[i],
            "event",
            vec![
                ev.clone(),
                Value::Float(x - xo),
                Value::Float(y - yo),
                Value::Float(cst),
            ],
        );
        match r {
            Ok(v) if !py::is_none(&v) => return Ok(v),
            Ok(_) => {}
            Err(crysirror_python::PyErr::Exc(e)) if e.class_name() == "IgnoreLayers" => {
                if !py::is_none(&layers) && ops::truthy(vm, &layers)? {
                    if !super::events::is_type(vm, &ev, super::events::TIMEEVENT)? {
                        let iface = py::import_attr(vm, "renpy.display", "interface")?;
                        if !py::is_none(&iface) {
                            vm.call_method(&iface, "post_time_event", Vec::new())?;
                        }
                    }
                    return Ok(Value::None);
                }
                return Err(crysirror_python::PyErr::Exc(e));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(Value::None)
}

fn multibox_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layers = py::get_or_none(vm, this, "layers")?;
    let sl = py::get_or_none(vm, this, "scene_list")?;
    if ops::truthy(vm, &layers)? || ops::truthy(vm, &sl)? {
        let r = py::config(vm, "tts_front_to_back").unwrap_or(Value::Bool(true));
        let f = py::get(vm, this, "_tts_common")?;
        return vm.call(&f, Vec::new(), vec![("reverse".into(), r)]);
    }
    vm.call_method(this, "_tts_common", Vec::new())
}

fn multibox_in_current_store(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer_name = py::get_or_none(vm, this, "layer_name")?;
    let layers = py::get_or_none(vm, this, "layers")?;
    if !py::is_none(&layer_name) || ops::truthy(vm, &layers)? {
        // Scene-list and layer boxes rebuild from the live scene lists each
        // interaction; their store identity is not copied.
        return Ok(this.clone());
    }
    container_in_current_store(vm, args, kw)
}

fn layout_fn(vm: &mut Vm, kw: Kw, layout: &str) -> PyResult<Value> {
    let mb = py::import_attr(vm, MODULE, "MultiBox")?;
    let mut k: Vec<_> = kw.to_vec();
    k.push(("layout".into(), vm.types.new_str(layout)));
    vm.call(&mb, Vec::new(), k)
}

fn f_fixed(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    layout_fn(vm, kw, "fixed")
}

// SizeGroup / Window -------------------------------------------------------------------------

fn sizegroup_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "members", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "_width", Value::None)?;
    py::set(vm, this, "computing_width", Value::Bool(false))?;
    Ok(Value::None)
}

fn sizegroup_width(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let w0 = py::get_or_none(vm, this, "_width")?;
    if !py::is_none(&w0) {
        return Ok(w0);
    }
    let computing = py::get_or_none(vm, this, "computing_width")?;
    if ops::truthy(vm, &computing)? {
        return Ok(Value::Int(0));
    }
    py::set(vm, this, "computing_width", Value::Bool(true))?;
    let (w, h, st, at) = rargs(vm, args)?;
    let members = py::get(vm, this, "members")?;
    let mut maxw = 0f64;
    for m in py::items(vm, &members)? {
        let old = {
            let s = render::rs(vm);
            let mut s = s.borrow_mut();
            std::mem::replace(&mut s.sizing, true)
        };
        let r = render(vm, &m, w, h, st, at);
        render::rs(vm).borrow_mut().sizing = old;
        maxw = maxw.max(size(&r?).0);
    }
    py::set(vm, this, "_width", Value::Float(maxw))?;
    py::set(vm, this, "computing_width", Value::Bool(false))?;
    Ok(Value::Float(maxw))
}

fn window_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "child")
        .cloned()
        .collect();
    if !props.iter().any(|(k, _)| &**k == "style") {
        let s = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| vm.types.new_str("window"));
        props.push(("style".into(), s));
    }
    container_init(vm, &[this.clone()], &props)?;
    if !py::is_none(&child) {
        vm.call_method(this, "add", vec![child])?;
    }
    Ok(Value::None)
}

fn window_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut out = Vec::new();
    for n in ["background", "foreground", "child"] {
        let v = style(vm, this, n)?;
        if !py::is_none(&v) && is_displayable(vm, &v) {
            out.push(v);
        }
    }
    out.extend(children(vm, this)?);
    Ok(vm.types.new_list(out))
}

fn window_get_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = style(vm, &args[0], "child")?;
    if ops::truthy(vm, &c)? {
        return Ok(c);
    }
    py::get_or_none(vm, &args[0], "child")
}

fn size_groups(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "size_groups")
}

fn window_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sg = style(vm, this, "size_group")?;
    if !ops::truthy(vm, &sg)? {
        return Ok(Value::None);
    }
    let groups = size_groups(vm)?;
    let mut group = vm.call_method(&groups, "get", vec![sg.clone(), Value::None])?;
    if py::is_none(&group) {
        let c = py::import_attr(vm, MODULE, "SizeGroup")?;
        group = vm.call(&c, Vec::new(), Vec::new())?;
        ops::setitem(vm, &groups, sg, group.clone())?;
    }
    let members = py::get(vm, &group, "members")?;
    ops::list_append(vm, &members, this.clone())?;
    Ok(Value::None)
}

fn window_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (mut width, mut height, st, at) = rargs(vm, args)?;
    let (mut xminimum, yminimum) = xyminimums(vm, this, width, height)?;
    let mut xmaximum = style(vm, this, "xmaximum")?;
    let mut ymaximum = style(vm, this, "ymaximum")?;
    if is_float_type(&xmaximum) {
        xmaximum = Value::Float(width);
    }
    if is_float_type(&ymaximum) {
        ymaximum = Value::Float(height);
    }
    let sg = style(vm, this, "size_group")?;
    if ops::truthy(vm, &sg)? {
        let groups = size_groups(vm)?;
        let group = vm.call_method(&groups, "get", vec![sg, Value::None])?;
        if !py::is_none(&group) {
            let w = vm.call_method(
                &group,
                "width",
                vec![
                    Value::Float(width),
                    Value::Float(height),
                    Value::Float(st),
                    Value::Float(at),
                ],
            )?;
            xminimum = xminimum.max(py::f64_of(vm, &w)?);
        }
    }
    width = width.max(xminimum);
    height = height.max(yminimum);
    let lm = style_f(vm, this, "left_margin", width)?;
    let lp = style_f(vm, this, "left_padding", width)?;
    let rm = style_f(vm, this, "right_margin", width)?;
    let rp = style_f(vm, this, "right_padding", width)?;
    let tm = style_f(vm, this, "top_margin", height)?;
    let tp = style_f(vm, this, "top_padding", height)?;
    let bm = style_f(vm, this, "bottom_margin", height)?;
    let bp = style_f(vm, this, "bottom_padding", height)?;
    let (cxm, cym, cxp, cyp) = (lm + rm, tm + bm, lp + rp, tp + bp);
    let child = vm.call_method(this, "get_child", Vec::new())?;
    let current = py::get_or_none(vm, this, "current_child")?;
    if !py::same(&child, &current) {
        if !py::is_none(&current) {
            transfer_transform_state(vm, &current, &child)?;
        }
        py::set(vm, this, "current_child", child.clone())?;
    }
    let surf = render(vm, &child, width - cxm - cxp, height - cym - cyp, st, at)?;
    let (sw, sh) = size(&surf);
    let fill_shrinks = cfg_b(vm, "fill_shrinks_frame", false);
    if !style_b(vm, this, "xfill")? {
        width = (cxm + cxp + sw).max(xminimum);
    } else if !fill_shrinks {
        width = (cxm + cxp + sw).max(width).max(xminimum);
    }
    if !style_b(vm, this, "yfill")? {
        height = (cym + cyp + sh).max(yminimum);
    } else if !fill_shrinks {
        height = (cym + cyp + sh).max(height).max(yminimum);
    }
    if cfg_b(vm, "enforce_window_max_size", false) {
        if !py::is_none(&xmaximum) {
            width = width.min(py::f64_of(vm, &xmaximum)?);
        }
        if !py::is_none(&ymaximum) {
            height = height.min(py::f64_of(vm, &ymaximum)?);
        }
    }
    let rv = new(vm, width, height);
    let mut modal = style(vm, this, "modal")?;
    if ops::truthy(vm, &modal)? && !crysirror_python::builtins::is_callable(vm, &modal) {
        modal = vm.types.new_str("window");
    }
    py::set(vm, &rv, "modal", modal)?;
    let bw = width - cxm;
    let bh = height - cym;
    let bg = style(vm, this, "background")?;
    if ops::truthy(vm, &bg)? {
        let back = render(vm, &bg, bw, bh, st, at)?;
        place(vm, &bg, &rv, lm, tm, bw, bh, &back, false)?;
    }
    let off = place(
        vm,
        &child,
        &rv,
        lm + lp,
        tm + tp,
        width - cxm - cxp,
        height - cym - cyp,
        &surf,
        true,
    )?;
    let fg = style(vm, this, "foreground")?;
    if ops::truthy(vm, &fg)? {
        let fore = render(vm, &fg, bw, bh, st, at)?;
        place(vm, &fg, &rv, lm, tm, bw, bh, &fore, false)?;
    }
    let own_child = py::get_or_none(vm, this, "child")?;
    if ops::truthy(vm, &own_child)? {
        set_offsets(vm, this, vec![off])?;
    }
    let ws = vm
        .types
        .new_tuple(vec![Value::Float(width), Value::Float(height)]);
    py::set(vm, this, "window_size", ws)?;
    Ok(rv)
}

/// Carries Transform state across a child replacement (`take_state`).
pub fn transfer_transform_state(vm: &mut Vm, old: &Value, new: &Value) -> PyResult<()> {
    let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
    let Some(tc) = tc.obj().cloned() else {
        return Ok(());
    };
    let (mut o, mut n) = (old.clone(), new.clone());
    if !ops::isinstance(vm, &o, &tc) && !ops::isinstance(vm, &n, &tc) && ops::eq(vm, &o, &n)? {
        o = vm.call_method(&o, "_target", Vec::new())?;
        n = vm.call_method(&n, "_target", Vec::new())?;
    }
    if ops::isinstance(vm, &o, &tc) && ops::isinstance(vm, &n, &tc) {
        vm.call_method(&n, "take_state", vec![o.clone()])?;
        vm.call_method(&n, "take_execution_state", vec![o])?;
    }
    Ok(())
}

fn window_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let rv = container_event(vm, args, kw)?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    if super::events::is_mouse(vm, &ev)? {
        let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
        let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
        let ws = py::get_or_none(vm, this, "window_size")?;
        let ws = py::items(vm, &ws).unwrap_or_default();
        let (w, h) = if ws.len() == 2 {
            (py::f64_of(vm, &ws[0])?, py::f64_of(vm, &ws[1])?)
        } else {
            (0., 0.)
        };
        let modal = style(vm, this, "modal")?;
        if super::focus::check_modal(vm, &modal, &ev, x, y, Some(w), Some(h))? {
            let ig = py::import_attr(vm, MODULE, "IgnoreLayers")?;
            let e = vm.call(&ig, Vec::new(), Vec::new())?;
            return Err(crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()));
        }
    }
    Ok(Value::None)
}

// DynamicDisplayable ---------------------------------------------------------------------

fn dyn_compat(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let expr = args.get(2).cloned().unwrap_or(Value::None);
    let e = py::import_attr(vm, "renpy.python", "py_eval")?;
    let child = vm.call(&e, vec![expr], Vec::new())?;
    Ok(vm.types.new_tuple(vec![child, Value::None]))
}

fn dyn_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    super::core::d_init(vm, &[this.clone()], &[])?;
    py::set(vm, this, "child", Value::None)?;
    let mut function = args.get(1).cloned().unwrap_or(Value::None);
    let mut fargs = args.get(2..).map(|a| a.to_vec()).unwrap_or_default();
    let mut fkw: Vec<_> = kw.to_vec();
    let predict = fkw
        .iter()
        .position(|(k, _)| &**k == "_predict_function")
        .map(|i| fkw.remove(i).1)
        .unwrap_or(Value::None);
    if function.as_str().is_some() {
        fargs = vec![function.clone()];
        fkw.clear();
        function = py::import_attr(vm, MODULE, "dynamic_displayable_compat")?;
    }
    py::set(vm, this, "predict_function", predict)?;
    py::set(vm, this, "function", function)?;
    py::set(vm, this, "args", vm.types.new_tuple(fargs))?;
    let d = py::kwargs_dict(vm, &fkw)?;
    py::set(vm, this, "kwargs", d)?;
    Ok(Value::None)
}

fn dyn_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "child", Value::None)?;
    py::set(vm, &args[0], "raw_child", Value::None)?;
    Ok(Value::None)
}

fn dyn_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = arg_or(args, kw, 1, "args", Value::None);
    let rv = vm.call_method(&args[0], "_copy", vec![a])?;
    py::set(vm, &rv, "child", Value::None)?;
    py::set(vm, &rv, "raw_child", Value::None)?;
    Ok(rv)
}

fn dyn_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = py::get_or_none(vm, this, "last_st")?;
    let at = py::get_or_none(vm, this, "last_at")?;
    vm.call_method(this, "update", vec![st, at])?;
    let c = py::get_or_none(vm, this, "child")?;
    let items = if ops::truthy(vm, &c)? {
        vec![c]
    } else {
        Vec::new()
    };
    Ok(vm.types.new_list(items))
}

fn call_function(vm: &mut Vm, this: &Value, st: Value, at: Value) -> PyResult<Value> {
    let f = py::get(vm, this, "function")?;
    let a = py::get(vm, this, "args")?;
    let k = py::get(vm, this, "kwargs")?;
    let mut all = vec![st, at];
    all.extend(py::items(vm, &a)?);
    let kw = py::str_items(vm, &k)?;
    vm.call(&f, all, kw)
}

fn dyn_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = args.get(1).cloned().unwrap_or(Value::Int(0));
    let at = args.get(2).cloned().unwrap_or(Value::Int(0));
    py::set(vm, this, "last_st", st.clone())?;
    py::set(vm, this, "last_at", at.clone())?;
    let r = call_function(vm, this, st, at)?;
    let r = py::items(vm, &r)?;
    if r.len() != 2 {
        return Err(vm.type_error("DynamicDisplayable function must return (displayable, redraw)"));
    }
    let (raw, redraw) = (r[0].clone(), r[1].clone());
    let old_raw = py::get_or_none(vm, this, "raw_child")?;
    if !ops::eq(vm, &raw, &old_raw)? {
        py::set(vm, this, "raw_child", raw.clone())?;
        let raw = displayable(vm, &raw)?;
        let dup = py::get_or_none(vm, &raw, "_duplicatable")?;
        let child = if ops::truthy(vm, &dup)? {
            let a = py::get_or_none(vm, this, "_args")?;
            let c = vm.call_method(&raw, "_duplicate", vec![a])?;
            vm.call_method(&c, "_unique", Vec::new())?;
            c
        } else {
            raw
        };
        let old = py::get_or_none(vm, this, "child")?;
        if !py::is_none(&old) {
            let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
            if let Some(tc) = tc.obj() {
                if ops::isinstance(vm, &old, tc) && ops::isinstance(vm, &child, tc) {
                    vm.call_method(&child, "take_state", vec![old.clone()])?;
                    vm.call_method(&child, "take_execution_state", vec![old])?;
                }
            }
        }
        let cb = py::builtin(vm, "per_interact_visitor", |vm, a, _| {
            vm.call_method(&a[0], "per_interact", Vec::new())
        });
        vm.call_method(&child, "visit_all", vec![cb])?;
        py::set(vm, this, "child", child)?;
    }
    if !py::is_none(&redraw) {
        let t = py::f64_of(vm, &redraw)?;
        render::redraw(vm, this, t);
    }
    Ok(Value::None)
}

fn dyn_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    render::redraw(vm, &args[0], 0.);
    Ok(Value::None)
}

fn dyn_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    vm.call_method(this, "update", vec![Value::Float(st), Value::Float(at)])?;
    let child = py::get(vm, this, "child")?;
    let cr = render(vm, &child, w, h, st, at)?;
    let (cw, ch) = size(&cr);
    let rv = new(vm, cw, ch);
    blit(vm, &rv, &cr, 0., 0.)?;
    Ok(rv)
}

fn dyn_predict_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let _ = (vm, args);
    Ok(Value::None)
}

fn dyn_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = py::get_or_none(vm, this, "child")?;
    if !ops::truthy(vm, &c)? {
        vm.call_method(this, "update", vec![Value::Int(0), Value::Int(0)])?;
    }
    let c = py::get(vm, this, "child")?;
    vm.call_method(&c, "get_placement", Vec::new())
}

fn dyn_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    if ops::truthy(vm, &c)? {
        return vm.call_method(&c, "event", args[1..].to_vec());
    }
    Ok(Value::None)
}

// ConditionSwitch / ShowingSwitch ------------------------------------------------------------

fn condition_switch_pick(vm: &mut Vm, switch: &Value) -> PyResult<Value> {
    let cache = py::import_attr(vm, MODULE, "cond_cache")?;
    for pair in py::items(vm, switch)? {
        let p = py::items(vm, &pair)?;
        let (cond, d) = (&p[0], &p[1]);
        if py::is_none(cond) {
            return Ok(d.clone());
        }
        let code = match ops::dict_get(vm, cache.obj().unwrap(), cond)? {
            Some(c) => c,
            None => {
                let pc = py::import_attr(vm, "renpy.python", "py_compile")?;
                let c = vm.call(
                    &pc,
                    vec![cond.clone(), vm.types.new_str("eval")],
                    Vec::new(),
                )?;
                ops::setitem(vm, &cache, cond.clone(), c.clone())?;
                c
            }
        };
        let ev = py::import_attr(vm, "renpy.python", "py_eval_bytecode")?;
        let r = vm.call(&ev, vec![code], Vec::new())?;
        if ops::truthy(vm, &r)? {
            return Ok(d.clone());
        }
    }
    if cfg_b(vm, "developer", false) {
        return Err(py::error(vm, "Switch could not choose a displayable."));
    }
    let null_c = py::import_attr(vm, MODULE, "Null")?;
    vm.call(&null_c, Vec::new(), Vec::new())
}

fn f_condition_switch_show(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let switch = args.get(2).cloned().unwrap_or(Value::None);
    let d = condition_switch_pick(vm, &switch)?;
    Ok(vm.types.new_tuple(vec![d, Value::None]))
}

fn f_condition_switch_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let switch = args.first().cloned().unwrap_or(Value::None);
    let d = condition_switch_pick(vm, &switch)?;
    Ok(vm.types.new_list(vec![d]))
}

fn f_condition_switch(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut props: Vec<_> = kw.to_vec();
    let predict_all = props
        .iter()
        .position(|(k, _)| &**k == "predict_all")
        .map(|i| props.remove(i).1)
        .unwrap_or(Value::None);
    if !props.iter().any(|(k, _)| &**k == "style") {
        props.push(("style".into(), vm.types.new_str("default")));
    }
    if args.len() % 2 != 0 {
        return Err(py::error(
            vm,
            "ConditionSwitch takes an even number of arguments",
        ));
    }
    let cache = py::import_attr(vm, MODULE, "cond_cache")?;
    let pc = py::import_attr(vm, "renpy.python", "py_compile")?;
    let mut switch = Vec::new();
    for pair in args.chunks(2) {
        let cond = pair[0].clone();
        if !py::is_none(&cond) && !ops::contains(vm, &cache, &cond)? {
            let c = vm.call(
                &pc,
                vec![cond.clone(), vm.types.new_str("eval")],
                Vec::new(),
            )?;
            ops::setitem(vm, &cache, cond.clone(), c)?;
        }
        let d = displayable(vm, &pair[1])?;
        switch.push(vm.types.new_tuple(vec![cond, d]));
    }
    let show = py::import_attr(vm, MODULE, "condition_switch_show")?;
    let pred = py::import_attr(vm, MODULE, "condition_switch_predict")?;
    let dd = py::import_attr(vm, MODULE, "DynamicDisplayable")?;
    let sw = vm.types.new_list(switch);
    let rv = vm.call(
        &dd,
        vec![show, sw, predict_all],
        vec![("_predict_function".into(), pred)],
    )?;
    let position = py::import_attr(vm, MODULE, "Position")?;
    vm.call(&position, vec![rv], props)
}

fn f_showing_switch(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut props: Vec<_> = kw.to_vec();
    let layer = props
        .iter()
        .position(|(k, _)| &**k == "layer")
        .map(|i| props.remove(i).1)
        .unwrap_or_else(|| vm.types.new_str("master"));
    if args.len() % 2 != 0 {
        return Err(py::error(
            vm,
            "ShowingSwitch takes an even number of positional arguments",
        ));
    }
    let mut condargs = Vec::new();
    for pair in args.chunks(2) {
        let name = &pair[0];
        let cond = if py::is_none(name) {
            Value::None
        } else {
            let parts: Vec<Value> = match name.as_str() {
                Some(s) => s.split_whitespace().map(|p| vm.types.new_str(p)).collect(),
                None => py::items(vm, name)?,
            };
            let t = vm.types.new_tuple(parts);
            let tr = ops::repr(vm, &t)?;
            let lr = ops::repr(vm, &layer)?;
            vm.types
                .new_string(format!("renpy.showing({tr}, layer={lr})"))
        };
        condargs.push(cond);
        condargs.push(pair[1].clone());
    }
    f_condition_switch(vm, &condargs, &props)
}

// Simple single-child wrappers ------------------------------------------------------------------

fn one_child_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "child")
        .cloned()
        .collect();
    container_init(vm, &[this.clone()], &props)?;
    vm.call_method(this, "add", vec![child])?;
    Ok(Value::None)
}

fn ignores_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    single_child_render(vm, args, false)
}

fn none_event(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn f_crop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = arg_or(args, kw, 0, "rect", Value::None);
    let child = arg_or(args, kw, 1, "child", Value::None);
    let t = py::import_attr(vm, "renpy.display.motion", "Transform")?;
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "rect" && &**k != "child")
        .cloned()
        .collect();
    props.push(("crop".into(), rect));
    vm.call(&t, vec![child], props)
}

// Side -------------------------------------------------------------------------------------------

const SIDE_POSITIONS: [&str; 9] = ["tl", "t", "tr", "r", "br", "b", "bl", "l", "c"];

fn side_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let positions = arg_or(args, kw, 1, "positions", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "positions")
        .cloned()
        .collect();
    if !props.iter().any(|(k, _)| &**k == "style") {
        let s = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| vm.types.new_str("side"));
        props.push(("style".into(), s));
    }
    container_init(vm, &[this.clone()], &props)?;
    let pos: Vec<String> = match positions.as_str() {
        Some(s) => s.split_whitespace().map(str::to_owned).collect(),
        None => py::items(vm, &positions)?
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_owned())
            .collect(),
    };
    let mut seen = std::collections::HashSet::new();
    for p in &pos {
        if !SIDE_POSITIONS.contains(&p.as_str()) {
            return Err(py::error(
                vm,
                format!("Side used with impossible position '{p}'."),
            ));
        }
        if !seen.insert(p.clone()) {
            return Err(py::error(
                vm,
                format!("Side used with duplicate position '{p}'."),
            ));
        }
    }
    let t = vm
        .types
        .new_tuple(pos.iter().map(|p| vm.types.new_str(p)).collect());
    py::set(vm, this, "positions", t)?;
    py::set(vm, this, "sized", Value::Bool(false))?;
    Ok(Value::None)
}

fn side_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let n = children(vm, this)?.len();
    let p = py::get(vm, this, "positions")?;
    if n >= py::items(vm, &p)?.len() {
        return Err(py::error(vm, "Side has been given too many arguments."));
    }
    container_add(vm, args, kw)
}

fn side_unsize(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "sized", Value::Bool(false))?;
    Ok(Value::None)
}

fn side_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    container_clear(vm, args, kw)?;
    side_unsize(vm, args, kw)
}

fn side_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (width, height, st, at) = rargs(vm, args)?;
    let ch = children(vm, this)?;
    let positions: Vec<String> = {
        let p = py::get(vm, this, "positions")?;
        py::items(vm, &p)?
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_owned())
            .collect()
    };
    if cfg_b(vm, "developer", false) && positions.len() != ch.len() {
        return Err(py::error(vm, "A side has the wrong number of children."));
    }
    let mut pos_d = std::collections::HashMap::new();
    let mut pos_i = std::collections::HashMap::new();
    for (i, (p, d)) in positions.iter().zip(&ch).enumerate() {
        pos_d.insert(p.as_str(), d.clone());
        pos_i.insert(p.as_str(), i);
    }
    let sized = {
        let v = py::get_or_none(vm, this, "sized")?;
        ops::truthy(vm, &v)?
    };
    let fget = |vm: &mut Vm, n: &str| -> PyResult<f64> {
        let v = py::get_or_none(vm, this, n)?;
        if py::is_none(&v) {
            Ok(0.)
        } else {
            py::f64_of(vm, &v)
        }
    };
    let (cwidth, cheight, top, bottom, left, right);
    if !sized {
        py::set(vm, this, "sized", Value::Bool(true))?;
        let sp_v = style(vm, this, "spacing")?;
        let sp = if py::is_none(&sp_v) {
            0.
        } else {
            py::f64_of(vm, &sp_v)?
        };
        let spacer = |a: &str, b: &str, c: &str, axis: f64| {
            if pos_d.contains_key(a) || pos_d.contains_key(b) || pos_d.contains_key(c) {
                (sp, axis - sp)
            } else {
                (0., axis)
            }
        };
        let (ls, w1) = spacer("tl", "l", "bl", width);
        let (rs, w2) = spacer("tr", "r", "br", w1);
        let (ts, h1) = spacer("tl", "t", "tr", height);
        let (bs, h2) = spacer("bl", "b", "br", h1);
        for (n, v) in [
            ("left_space", ls),
            ("right_space", rs),
            ("top_space", ts),
            ("bottom_space", bs),
        ] {
            py::set(vm, this, n, Value::Float(v))?;
        }
        let (w, h) = (w2, h2);
        let mut sizeit =
            |vm: &mut Vm, pos: &str, w: f64, h: f64, ow: f64, oh: f64| -> PyResult<(f64, f64)> {
                let Some(d) = pos_d.get(pos) else {
                    return Ok((ow, oh));
                };
                let old = {
                    let s = render::rs(vm);
                    let mut s = s.borrow_mut();
                    std::mem::replace(&mut s.sizing, true)
                };
                let r = render(vm, d, w, h, st, at);
                render::rs(vm).borrow_mut().sizing = old;
                let (rw, rh) = size(&r?);
                Ok((ow.max(rw), oh.max(rh)))
            };
        let (mut cw, mut chh) = sizeit(vm, "c", w, h, 0., 0.)?;
        let (mut l, mut r, mut t, mut b) = (0., 0., 0., 0.);
        (cw, t) = sizeit(vm, "t", cw, h, cw, t)?;
        (cw, b) = sizeit(vm, "b", cw, h, cw, b)?;
        (l, chh) = sizeit(vm, "l", w, chh, l, chh)?;
        (r, chh) = sizeit(vm, "r", w, chh, r, chh)?;
        (l, t) = sizeit(vm, "tl", l, t, l, t)?;
        (l, b) = sizeit(vm, "bl", l, b, l, b)?;
        (r, t) = sizeit(vm, "tr", r, t, r, t)?;
        (r, b) = sizeit(vm, "br", r, b, r, b)?;
        for (n, v) in [
            ("cwidth", cw),
            ("cheight", chh),
            ("top", t),
            ("bottom", b),
            ("left", l),
            ("right", r),
        ] {
            py::set(vm, this, n, Value::Float(v))?;
        }
        (cwidth, cheight, top, bottom, left, right) = (cw, chh, t, b, l, r);
    } else {
        cwidth = fget(vm, "cwidth")?;
        cheight = fget(vm, "cheight")?;
        top = fget(vm, "top")?;
        bottom = fget(vm, "bottom")?;
        left = fget(vm, "left")?;
        right = fget(vm, "right")?;
    }
    let zero = offset(vm, 0., 0.);
    let mut offsets = vec![zero; ch.len()];
    let lefts = fget(vm, "left_space")?;
    let rights = fget(vm, "right_space")?;
    let tops = fget(vm, "top_space")?;
    let bottoms = fget(vm, "bottom_space")?;
    let mut cwidth = if style_b(vm, this, "xfill")? {
        width
    } else {
        cwidth
    };
    let mut cheight = if style_b(vm, this, "yfill")? {
        height
    } else {
        cheight
    };
    cwidth = cwidth.min(width - left - lefts - right - rights);
    cheight = cheight.min(height - top - tops - bottom - bottoms);
    let mut c_rend = None;
    if let Some(d) = pos_d.get("c") {
        let r = render(vm, d, cwidth, cheight, st, at)?;
        let (w, h) = size(&r);
        cwidth = cwidth.max(w);
        cheight = cheight.max(h);
        c_rend = Some(r);
    }
    let rv = new(
        vm,
        left + lefts + cwidth + rights + right,
        top + tops + cheight + bottoms + bottom,
    );
    let (col1, col2, col3) = (0., left + lefts, left + lefts + cwidth + rights);
    let (row1, row2, row3) = (0., top + tops, top + tops + cheight + bottoms);
    let mut order = vec![
        ("c", col2, row2, cwidth, cheight),
        ("t", col2, row1, cwidth, top),
        ("r", col3, row2, right, cheight),
        ("b", col2, row3, cwidth, bottom),
        ("l", col1, row2, left, cheight),
        ("tl", col1, row1, left, top),
        ("tr", col3, row1, right, top),
        ("br", col3, row3, right, bottom),
        ("bl", col1, row3, left, bottom),
    ];
    if cfg_b(vm, "keep_side_render_order", true) {
        order.sort_by_key(|(p, ..)| {
            positions
                .iter()
                .position(|q| q == p)
                .unwrap_or(positions.len())
        });
    }
    for (pos, x, y, w, h) in order {
        let Some(d) = pos_d.get(pos) else { continue };
        let rend = if pos == "c" {
            c_rend.clone().unwrap()
        } else {
            render(vm, d, w, h, st, at)?
        };
        offsets[pos_i[pos]] = place(vm, d, &rv, x, y, w, h, &rend, true)?;
    }
    set_offsets(vm, this, offsets)?;
    Ok(rv)
}

// Alpha ------------------------------------------------------------------------------------------

fn alpha_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "start",
        "end",
        "time",
        "child",
        "repeat",
        "bounce",
        "anim_timebase",
        "time_warp",
    ];
    let mut vals = vec![
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
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
    super::core::d_init(vm, &[this.clone()], &props)?;
    let child = displayable(vm, &vals[3])?;
    for (i, n) in names.iter().enumerate() {
        let v = if *n == "child" {
            child.clone()
        } else {
            vals[i].clone()
        };
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn child_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![c]))
}

fn alpha_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let atb = {
        let v = py::get_or_none(vm, this, "anim_timebase")?;
        ops::truthy(vm, &v)?
    };
    let t = if atb { at } else { st };
    let time = {
        let v = py::get_or_none(vm, this, "time")?;
        if py::is_none(&v) {
            0.
        } else {
            py::f64_of(vm, &v)?
        }
    };
    let mut done = if time != 0. { (t / time).min(1.) } else { 1. };
    let less = py::import_attr(vm, "renpy.game", "less_updates")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false);
    let repeat = {
        let v = py::get_or_none(vm, this, "repeat")?;
        ops::truthy(vm, &v)?
    };
    if less {
        done = 1.;
    } else if repeat {
        done %= 1.;
        render::redraw(vm, this, 0.);
    } else if done != 1. {
        render::redraw(vm, this, 0.);
    }
    let warp = py::get_or_none(vm, this, "time_warp")?;
    if !py::is_none(&warp) {
        let r = vm.call(&warp, vec![Value::Float(done)], Vec::new())?;
        done = py::f64_of(vm, &r)?;
    }
    let s = {
        let v = py::get(vm, this, "start")?;
        py::f64_of(vm, &v)?
    };
    let e = {
        let v = py::get(vm, this, "end")?;
        py::f64_of(vm, &v)?
    };
    let alpha = s + done * (e - s);
    let child = py::get(vm, this, "child")?;
    let rend = render(vm, &child, w, h, st, at)?;
    let (rw, rh) = size(&rend);
    let rv = new(vm, rw, rh);
    blit(vm, &rv, &rend, 0., 0.)?;
    if let Some(r) = render_data(&rv) {
        r.borrow_mut().alpha = alpha;
    }
    Ok(rv)
}

// AdjustTimes / MatchTimes ------------------------------------------------------------------------

fn adjust_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let s = arg_or(args, kw, 2, "start_time", Value::None);
    let a = arg_or(args, kw, 3, "anim_time", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "child" | "start_time" | "anim_time"))
        .cloned()
        .collect();
    container_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "start_time", s)?;
    py::set(vm, this, "anim_time", a)?;
    vm.call_method(this, "add", vec![child])?;
    Ok(Value::None)
}

fn adjusted_times(vm: &mut Vm, this: &Value) -> PyResult<(f64, f64)> {
    let it = iface_f(vm, "interact_time")?;
    let ft = iface_f(vm, "frame_time")?.unwrap_or(0.);
    let mut out = [0.; 2];
    for (i, n) in ["start_time", "anim_time"].iter().enumerate() {
        let mut v = py::get_or_none(vm, this, n)?;
        if py::is_none(&v) {
            if let Some(it) = it {
                v = Value::Float(it);
                py::set(vm, this, n, v.clone())?;
            }
        }
        out[i] = if py::is_none(&v) {
            0.
        } else {
            ft - py::f64_of(vm, &v)?
        };
    }
    Ok((out[0], out[1]))
}

fn f_adjusted_times(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, a) = adjusted_times(vm, &args[0])?;
    Ok(vm.types.new_tuple(vec![Value::Float(s), Value::Float(a)]))
}

fn adjust_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, _, _) = rargs(vm, args)?;
    let (st, at) = adjusted_times(vm, this)?;
    let child = py::get(vm, this, "child")?;
    let cr = render(vm, &child, w, h, st, at)?;
    let (cw, ch) = size(&cr);
    let rv = new(vm, cw, ch);
    blit(vm, &rv, &cr, 0., 0.)?;
    let o = offset(vm, 0., 0.);
    set_offsets(vm, this, vec![o])?;
    Ok(rv)
}

fn adjust_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (st, _) = adjusted_times(vm, &args[0])?;
    let mut a = args.to_vec();
    if a.len() > 4 {
        a[4] = Value::Float(st);
    }
    container_event(vm, &a, kw)
}

fn match_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let target = arg_or(args, kw, 2, "target", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "child" | "target"))
        .cloned()
        .collect();
    container_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "target", target)?;
    vm.call_method(this, "add", vec![child])?;
    Ok(Value::None)
}

fn match_render(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (_, _, st, at) = rargs(vm, args)?;
    let ft = iface_f(vm, "frame_time")?.unwrap_or(0.);
    let target = py::get(vm, this, "target")?;
    py::set(vm, &target, "start_time", Value::Float(ft - st))?;
    py::set(vm, &target, "anim_time", Value::Float(ft - at))?;
    let _ = kw;
    single_child_render(vm, args, true)
}

// Tile / Flatten / AlphaMask ---------------------------------------------------------------------

fn tile_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "child")
        .cloned()
        .collect();
    if !props.iter().any(|(k, _)| &**k == "style") {
        let s = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| vm.types.new_str("tile"));
        props.push(("style".into(), s));
    }
    container_init(vm, &[this.clone()], &props)?;
    vm.call_method(this, "add", vec![child])?;
    Ok(Value::None)
}

fn tile_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (width, height, st, at) = rargs(vm, args)?;
    let child = py::get(vm, this, "child")?;
    let cr = render(vm, &child, width, height, st, at)?;
    let (cw, ch) = size(&cr);
    let rv = new(vm, width, height);
    let (w, h, cw, ch) = (
        width.trunc() as i64,
        height.trunc() as i64,
        cw.trunc() as i64,
        ch.trunc() as i64,
    );
    if cw <= 0 || ch <= 0 {
        return Ok(rv);
    }
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let ccw = cw.min(w - x);
            let cch = ch.min(h - y);
            let ccr = if ccw < cw || cch < ch {
                render::subsurface(vm, &cr, [0., 0., ccw as f64, cch as f64], false, false)?
            } else {
                cr.clone()
            };
            blit_nofocus(vm, &rv, &ccr, x as f64, y as f64)?;
            x += cw;
        }
        y += ch;
    }
    Ok(rv)
}

fn flatten_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let dr = arg_or(args, kw, 2, "drawable_resolution", Value::Bool(true));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "child" | "drawable_resolution"))
        .cloned()
        .collect();
    container_init(vm, &[this.clone()], &props)?;
    vm.call_method(this, "add", vec![child])?;
    py::set(vm, this, "drawable_resolution", dr)?;
    Ok(Value::None)
}

fn flatten_render(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rv = single_child_render(vm, args, true)?;
    let _ = kw;
    if let Some(r) = render_data(&rv) {
        r.borrow_mut().operation = render::FLATTEN;
    }
    Ok(rv)
}

fn alphamask_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let mask = arg_or(args, kw, 2, "mask", Value::None);
    let invert = arg_or(args, kw, 3, "invert", Value::Bool(false));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "child" | "mask" | "invert"))
        .cloned()
        .collect();
    container_init(vm, &[this.clone()], &props)?;
    let mask = displayable(vm, &mask)?;
    py::set(vm, this, "mask", mask.clone())?;
    let dup = py::get_or_none(vm, &mask, "_duplicatable")?;
    py::set(vm, this, "_duplicatable", dup)?;
    vm.call_method(this, "add", vec![mask])?;
    vm.call_method(this, "add", vec![child])?;
    py::set(vm, this, "invert", invert)?;
    Ok(Value::None)
}

fn alphamask_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::get(vm, &args[0], "mask")?;
    let c = py::get(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![m, c]))
}

fn alphamask_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let child = py::get(vm, this, "child")?;
    let mask = py::get(vm, this, "mask")?;
    let cr = render(vm, &child, w, h, st, at)?;
    let (cw, ch) = size(&cr);
    let mr = render(vm, &mask, cw, ch, st, at)?;
    let invert = {
        let v = py::get_or_none(vm, this, "invert")?;
        ops::truthy(vm, &v)?
    };
    let rv = new(vm, cw, ch);
    blit(vm, &rv, &cr, 0., 0.)?;
    let mask = render::source_of(vm, &mr)?;
    if let Some(r) = render_data(&rv) {
        r.borrow_mut().mask = Some((mask, invert));
    }
    let o = offset(vm, 0., 0.);
    set_offsets(vm, this, vec![o.clone(), o])?;
    Ok(rv)
}

// NearRect ----------------------------------------------------------------------------------------

fn nearrect_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "child",
        "rect",
        "focus",
        "preferred_side",
        "invert_offsets",
        "replaces",
    ];
    let mut vals = vec![
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::Bool(false),
        Value::None,
    ];
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    let mut prefer_top = false;
    for (k, v) in kw {
        if &**k == "prefer_top" {
            prefer_top = ops::truthy(vm, v)?;
            continue;
        }
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    container_init(vm, &[this.clone()], &props)?;
    let mut rect = vals[1].clone();
    if !py::is_none(&vals[2]) {
        let g = py::import_attr(vm, "renpy.display.focus", "get_focus_rect")?;
        rect = vm.call(&g, vec![vals[2].clone()], Vec::new())?;
    } else if py::is_none(&rect) {
        return Err(py::error(
            vm,
            "A NearRect requires either a focus or a rect parameter.",
        ));
    }
    py::set(vm, this, "parent_rect", rect)?;
    py::set(vm, this, "focus_rect", vals[2].clone())?;
    let mut side = if ops::truthy(vm, &vals[3])? {
        vals[3].clone()
    } else {
        vm.types.new_str("bottom")
    };
    if prefer_top {
        side = vm.types.new_str("top");
    }
    if !matches!(side.as_str(), Some("left" | "top" | "right" | "bottom")) {
        let r = ops::repr(vm, &side)?;
        return Err(py::error(
            vm,
            format!("preferred_side used with impossible position {r}."),
        ));
    }
    py::set(vm, this, "preferred_side", side)?;
    py::set(vm, this, "invert_offsets", vals[4].clone())?;
    let hide = if py::is_none(&vals[5]) {
        Value::None
    } else {
        py::get_or_none(vm, &vals[5], "hide_parent_rect")?
    };
    py::set(vm, this, "hide_parent_rect", hide)?;
    if !py::is_none(&vals[0]) {
        vm.call_method(this, "add", vec![vals[0].clone()])?;
    }
    Ok(Value::None)
}

fn nearrect_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let fr = py::get_or_none(vm, this, "focus_rect")?;
    if py::is_none(&fr) {
        return Ok(Value::None);
    }
    let g = py::import_attr(vm, "renpy.display.focus", "get_focus_rect")?;
    let rect = vm.call(&g, vec![fr], Vec::new())?;
    let parent = py::get_or_none(vm, this, "parent_rect")?;
    let child = py::get_or_none(vm, this, "child")?;
    if !py::is_none(&rect) && py::is_none(&parent) {
        vm.call_method(
            &child,
            "set_transform_event",
            vec![vm.types.new_str("show")],
        )?;
    } else if py::is_none(&rect) && !py::is_none(&parent) {
        vm.call_method(
            &child,
            "set_transform_event",
            vec![vm.types.new_str("hide")],
        )?;
        py::set(vm, this, "hide_parent_rect", parent.clone())?;
    }
    if !ops::eq(vm, &parent, &rect)? {
        py::set(vm, this, "parent_rect", rect)?;
        render::redraw(vm, this, 0.);
    }
    Ok(Value::None)
}

fn nearrect_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (width, height, st, at) = rargs(vm, args)?;
    let rv = new(vm, width, height);
    let zero = offset(vm, 0., 0.);
    let pick = |vm: &mut Vm| -> PyResult<Value> {
        let p = py::get_or_none(vm, this, "parent_rect")?;
        if ops::truthy(vm, &p)? {
            Ok(p)
        } else {
            py::get_or_none(vm, this, "hide_parent_rect")
        }
    };
    let rect = pick(vm)?;
    if py::is_none(&rect) {
        set_offsets(vm, this, vec![zero])?;
        return Ok(rv);
    }
    let [px, py_, pw, ph] = render::rect4(vm, &rect)?;
    let side = {
        let v = py::get(vm, this, "preferred_side")?;
        py::string(vm, &v)?
    };
    let vertical = side == "top" || side == "bottom";
    let (aw, ah) = if vertical {
        (width, py_.max(height - py_ - ph))
    } else {
        (px.max(width - px - pw), height)
    };
    let child = py::get(vm, this, "child")?;
    let cr = render(vm, &child, aw, ah, st, at)?;
    let (cw, ch) = size(&cr);
    let tc = py::import_attr(vm, "renpy.display.motion", "Transform")?;
    let is_t = tc.obj().is_some_and(|c| ops::isinstance(vm, &child, c));
    let clear_hide = if is_t {
        let hr = py::get_or_none(vm, &child, "hide_response")?;
        ops::truthy(vm, &hr)?
    } else {
        true
    };
    if clear_hide {
        py::set(vm, this, "hide_parent_rect", Value::None)?;
    }
    let rect = pick(vm)?;
    if py::is_none(&rect) {
        set_offsets(vm, this, vec![zero])?;
        return Ok(rv);
    }
    let p = vm.call_method(&child, "get_placement", Vec::new())?;
    let p = py::items(vm, &p)?;
    let num = |vm: &mut Vm, v: &Value| -> PyResult<f64> {
        if py::is_none(v) {
            Ok(0.)
        } else {
            py::f64_of(vm, v)
        }
    };
    let mut xoffset = num(vm, &p[4])?;
    let mut yoffset = num(vm, &p[5])?;
    let (mut lx, mut ly) = (0f64, 0f64);
    let mut on_pref = true;
    if side == "top" && ch < py_ {
        ly = py_ - ch;
    } else if side == "left" && cw < px {
        lx = px - cw;
    } else if vertical && ch <= height - py_ - ph {
        ly = py_ + ph;
        on_pref = side == "bottom";
    } else if !vertical && cw <= width - px - pw {
        lx = px + pw;
        on_pref = side == "right";
    } else if vertical {
        ly = py_ - ch;
        on_pref = side == "top";
    } else {
        lx = px - cw;
        on_pref = side == "left";
    }
    let xpos = compute_raw(
        vm,
        &if py::is_none(&p[0]) {
            Value::Int(0)
        } else {
            p[0].clone()
        },
        pw,
    )?;
    let xanchor = compute_raw(
        vm,
        &if py::is_none(&p[2]) {
            Value::Int(0)
        } else {
            p[2].clone()
        },
        cw,
    )?;
    let ypos = compute_raw(
        vm,
        &if py::is_none(&p[1]) {
            Value::Int(0)
        } else {
            p[1].clone()
        },
        ph,
    )?;
    let yanchor = compute_raw(
        vm,
        &if py::is_none(&p[3]) {
            Value::Int(0)
        } else {
            p[3].clone()
        },
        ch,
    )?;
    if vertical {
        lx = (px + xpos - xanchor).min(width - cw).max(0.);
    } else {
        ly = (py_ + ypos - yanchor).min(height - ch).max(0.);
    }
    let inv = {
        let v = py::get_or_none(vm, this, "invert_offsets")?;
        ops::truthy(vm, &v)?
    };
    if inv && !on_pref {
        xoffset = -xoffset;
        yoffset = -yoffset;
    }
    lx += xoffset;
    ly += yoffset;
    blit(vm, &rv, &cr, lx, ly)?;
    let o = offset(vm, lx, ly);
    set_offsets(vm, this, vec![o])?;
    Ok(rv)
}

fn nearrect_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = py::get_or_none(vm, &args[0], "parent_rect")?;
    if py::is_none(&p) {
        return Ok(Value::None);
    }
    container_event(vm, args, kw)
}

fn nearrect_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::get_or_none(vm, &args[0], "parent_rect")?;
    if py::is_none(&p) {
        return Ok(vm.types.new_str(""));
    }
    vm.call_method(&args[0], "_tts_common", Vec::new())
}

// Layer -------------------------------------------------------------------------------------------

fn layer_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "layer")
        .cloned()
        .collect();
    let clipping = props
        .iter()
        .position(|(k, _)| &**k == "clipping")
        .map(|i| props.remove(i).1)
        .unwrap_or(Value::Bool(true));
    py::set(vm, this, "layer", layer)?;
    py::set(vm, this, "_clipping", clipping)?;
    container_init(vm, &[this.clone()], &props)?;
    Ok(Value::None)
}

fn layer_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = arg_or(args, kw, 1, "d", Value::None);
    let st = arg_or(args, kw, 2, "st", Value::None);
    let at = arg_or(args, kw, 3, "at", Value::None);
    vm.call_method(this, "_clear", Vec::new())?;
    py::set(vm, this, "start_time", st)?;
    py::set(vm, this, "anim_time", at)?;
    container_add(vm, &[this.clone(), d], &[])
}

fn layer_visit_all(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cb = arg_or(args, kw, 1, "callback", Value::None);
    vm.call(&cb, vec![this.clone()], Vec::new())?;
    let d = py::get_or_none(vm, this, "child")?;
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let seen = match arg(args, kw, 2, "seen") {
        Some(s) if !py::is_none(s) => s.clone(),
        _ => vm.types.new_set(Default::default()),
    };
    ops::set_add(vm, &seen, Value::Int(d.id()))?;
    vm.call_method(&d, "visit_all", vec![cb, seen])?;
    Ok(Value::None)
}

fn layer_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = interface(vm)?;
    if !py::is_none(&i) {
        let take = py::get_or_none(vm, &i, "take_layer_displayable")?;
        if ops::truthy(vm, &take)? {
            vm.call(&take, vec![args[0].clone()], Vec::new())?;
        }
    }
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("Composite", f_composite),
            ("LiveComposite", f_composite),
            ("Fixed", f_fixed),
            ("default_modal_function", f_default_modal_function),
            ("check_modal", f_check_modal),
            ("dynamic_displayable_compat", dyn_compat),
            ("condition_switch_show", f_condition_switch_show),
            ("condition_switch_predict", f_condition_switch_predict),
            ("ConditionSwitch", f_condition_switch),
            ("ShowingSwitch", f_showing_switch),
            ("Crop", f_crop),
            ("LiveCrop", f_crop),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
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
    c(
        vm,
        "Null",
        &[d.clone()],
        &[("__init__", null_init), ("render", null_render)],
        &[],
    )?;
    let list_t = Value::Obj(vm.types.list.clone());
    let container = c(
        vm,
        "Container",
        &[d.clone()],
        &[
            ("__init__", container_init),
            ("set_transform_event", container_set_transform_event),
            ("_handles_event", container_handles_event),
            ("set_style_prefix", container_set_style_prefix),
            ("_duplicate", container_duplicate),
            ("_unique", container_unique),
            ("_in_current_store", container_in_current_store),
            ("add", container_add),
            ("_clear", container_clear),
            ("remove", container_remove),
            ("update", container_update),
            ("render", container_render),
            ("event", container_event),
            ("visit", container_visit),
            ("__enter__", container_enter),
            ("__exit__", container_exit),
        ],
        &[("_list_type", list_t)],
    )?;
    c(
        vm,
        "Position",
        &[container.clone()],
        &[
            ("__init__", position_init),
            ("render", position_render),
            ("get_placement", position_get_placement),
        ],
        &[],
    )?;
    c(
        vm,
        "Grid",
        &[container.clone()],
        &[
            ("__init__", grid_init),
            ("render", grid_render),
            ("add", grid_add),
            ("per_interact", grid_per_interact),
        ],
        &[("allow_underfull", Value::None)],
    )?;
    let exc = Value::Obj(vm.types.exc("Exception"));
    c(vm, "IgnoreLayers", &[exc], &[], &[])?;
    c(
        vm,
        "MultiBox",
        &[container.clone()],
        &[
            ("__init__", multibox_init),
            ("_clear", multibox_clear),
            ("_in_current_store", multibox_in_current_store),
            ("_classname", multibox_classname),
            ("add", multibox_add),
            ("append_scene_list", multibox_append_scene_list),
            ("update_times", multibox_update_times),
            ("render", multibox_render),
            ("event", multibox_event),
            ("_tts", multibox_tts),
        ],
        &[
            ("layer_name", Value::None),
            ("first", Value::Bool(true)),
            ("order_reverse", Value::Bool(false)),
            ("layout", Value::None),
            ("_layer_at_list", Value::None),
            ("_camera_list", Value::None),
            ("layers", Value::None),
            ("untransformed_layer", Value::None),
            ("adjust_times", Value::Bool(false)),
            ("scene_list", Value::None),
        ],
    )?;
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    c(
        vm,
        "SizeGroup",
        &[object],
        &[("__init__", sizegroup_init), ("width", sizegroup_width)],
        &[],
    )?;
    set(vm, "size_groups", vm.types.new_dict());
    let wsz = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    c(
        vm,
        "Window",
        &[container.clone()],
        &[
            ("__init__", window_init),
            ("visit", window_visit),
            ("get_child", window_get_child),
            ("per_interact", window_per_interact),
            ("render", window_render),
            ("event", window_event),
        ],
        &[("window_size", wsz), ("current_child", Value::None)],
    )?;
    let nosave = vm.types.new_list(vec![vm.types.new_str("child")]);
    c(
        vm,
        "DynamicDisplayable",
        &[d.clone()],
        &[
            ("__init__", dyn_init),
            ("after_setstate", dyn_after_setstate),
            ("_duplicate", dyn_duplicate),
            ("visit", dyn_visit),
            ("update", dyn_update),
            ("per_interact", dyn_per_interact),
            ("render", dyn_render),
            ("predict_one", dyn_predict_one),
            ("get_placement", dyn_get_placement),
            ("event", dyn_event),
        ],
        &[
            ("nosave", nosave),
            ("_duplicatable", Value::Bool(true)),
            ("raw_child", Value::None),
            ("last_st", Value::Int(0)),
            ("last_at", Value::Int(0)),
        ],
    )?;
    set(vm, "cond_cache", vm.types.new_dict());
    c(
        vm,
        "IgnoresEvents",
        &[container.clone()],
        &[
            ("__init__", one_child_init),
            ("render", ignores_render),
            ("get_placement", child_get_placement),
            ("event", none_event),
        ],
        &[],
    )?;
    let possible = {
        let l = SIDE_POSITIONS.iter().map(|p| vm.types.new_str(p)).collect();
        let l = vm.types.new_list(l);
        vm.call(&Value::Obj(vm.types.set.clone()), vec![l], Vec::new())?
    };
    c(
        vm,
        "Side",
        &[container.clone()],
        &[
            ("__init__", side_init),
            ("after_setstate", side_unsize),
            ("add", side_add),
            ("_clear", side_clear),
            ("per_interact", side_unsize),
            ("render", side_render),
        ],
        &[("possible_positions", possible)],
    )?;
    c(
        vm,
        "Alpha",
        &[d.clone()],
        &[
            ("__init__", alpha_init),
            ("visit", child_visit),
            ("render", alpha_render),
        ],
        &[],
    )?;
    let adjust = c(
        vm,
        "AdjustTimes",
        &[container.clone()],
        &[
            ("__init__", adjust_init),
            ("adjusted_times", f_adjusted_times),
            ("render", adjust_render),
            ("event", adjust_event),
            ("get_placement", child_get_placement),
        ],
        &[],
    )?;
    c(
        vm,
        "MatchTimes",
        &[container.clone()],
        &[
            ("__init__", match_init),
            ("render", match_render),
            ("get_placement", child_get_placement),
        ],
        &[],
    )?;
    let tile = c(
        vm,
        "Tile",
        &[container.clone()],
        &[("__init__", tile_init), ("render", tile_render)],
        &[],
    )?;
    set(vm, "LiveTile", tile);
    c(
        vm,
        "Flatten",
        &[container.clone()],
        &[
            ("__init__", flatten_init),
            ("render", flatten_render),
            ("get_placement", child_get_placement),
        ],
        &[("drawable_resolution", Value::Bool(true))],
    )?;
    c(
        vm,
        "AlphaMask",
        &[container.clone()],
        &[
            ("__init__", alphamask_init),
            ("visit", alphamask_visit),
            ("render", alphamask_render),
        ],
        &[
            ("invert", Value::Bool(false)),
            ("_duplicatable", Value::Bool(false)),
        ],
    )?;
    let nr_pos = {
        let l = ["left", "top", "right", "bottom"]
            .iter()
            .map(|p| vm.types.new_str(p))
            .collect();
        let l = vm.types.new_list(l);
        vm.call(&Value::Obj(vm.types.set.clone()), vec![l], Vec::new())?
    };
    c(
        vm,
        "NearRect",
        &[container.clone()],
        &[
            ("__init__", nearrect_init),
            ("per_interact", nearrect_per_interact),
            ("render", nearrect_render),
            ("event", nearrect_event),
            ("_tts", nearrect_tts),
        ],
        &[
            ("preferred_side", vm.types.new_str("bottom")),
            ("invert_offsets", Value::Bool(false)),
            ("possible_positions", nr_pos),
        ],
    )?;
    c(
        vm,
        "Layer",
        &[adjust],
        &[
            ("__init__", layer_init),
            ("add", layer_add),
            ("visit_all", layer_visit_all),
            ("per_interact", layer_per_interact),
        ],
        &[("layers", vm.types.new_dict())],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
