// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.focus` and the focus walk of the screen render, after
//! Ren'Py 8.3 `focus.py` / `render.pyx` (MIT).
//!
//! The focus state (`grab`, `argument`, `tooltip`, `focus_list`, ...) lives
//! in the module namespace, as upstream module globals do, so script code
//! that reads `renpy.display.focus.grab` sees the live value.
use super::{
    py::{self, *},
    render::{Mat, RenderData, RenderRef, Source, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.focus";

fn module(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, MODULE)
}

fn g(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let m = module(vm)?;
    py::get_or_none(vm, &m, name)
}

fn sg(vm: &mut Vm, name: &str, v: Value) -> PyResult<()> {
    let m = module(vm)?;
    py::set(vm, &m, name, v)
}

fn scene_lists(vm: &mut Vm) -> PyResult<Option<Value>> {
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let c = match vm.call(&ctx, Vec::new(), Vec::new()) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    if py::is_none(&c) {
        return Ok(None);
    }
    let sl = py::get_or_none(vm, &c, "scene_lists")?;
    Ok((!py::is_none(&sl)).then_some(sl))
}

pub fn get_focused(vm: &mut Vm) -> PyResult<Value> {
    match scene_lists(vm)? {
        Some(sl) => py::get_or_none(vm, &sl, "focused"),
        None => g(vm, "_focused"),
    }
}

fn set_focused_widget(vm: &mut Vm, w: Value) -> PyResult<()> {
    match scene_lists(vm)? {
        Some(sl) => py::set(vm, &sl, "focused", w),
        None => sg(vm, "_focused", w),
    }
}

pub fn is_focused(vm: &mut Vm, d: &Value) -> PyResult<bool> {
    let grab = g(vm, "grab")?;
    if !py::is_none(&grab) && !py::same(&grab, d) {
        return Ok(false);
    }
    Ok(py::same(&get_focused(vm)?, d))
}

fn focus_class(vm: &mut Vm) -> PyResult<Value> {
    g(vm, "Focus")
}

fn new_focus(
    vm: &mut Vm,
    widget: Value,
    arg: Value,
    x: Value,
    y: Value,
    w: Value,
    h: Value,
    screen: Value,
) -> PyResult<Value> {
    let c = focus_class(vm)?;
    vm.call(&c, vec![widget, arg, x, y, w, h, screen], Vec::new())
}

// Focus objects --------------------------------------------------------------

fn fo_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["widget", "arg", "x", "y", "w", "h", "screen"]
        .iter()
        .enumerate()
    {
        py::set(vm, this, n, args.get(i + 1).cloned().unwrap_or(Value::None))?;
    }
    Ok(Value::None)
}

fn fo_fields(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    ["widget", "arg", "x", "y", "w", "h", "screen"]
        .iter()
        .map(|n| py::get_or_none(vm, this, n))
        .collect()
}

fn fo_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = fo_fields(vm, &args[0])?;
    let c = focus_class(vm)?;
    vm.call(&c, f, Vec::new())
}

fn fo_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = fo_fields(vm, &args[0])?;
    let mut parts = Vec::new();
    for v in &f {
        parts.push(ops::repr(vm, v)?.to_string());
    }
    Ok(vm.types.new_string(format!(
        "<Focus: {} {} ({}, {}, {}, {}) {}>",
        parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6]
    )))
}

fn rect_of(vm: &mut Vm, f: &Value) -> PyResult<Option<[f64; 4]>> {
    let x = py::get_or_none(vm, f, "x")?;
    if matches!(x, Value::None | Value::Bool(false)) {
        return Ok(None);
    }
    let mut r = [0.; 4];
    for (i, n) in ["x", "y", "w", "h"].iter().enumerate() {
        let v = py::get_or_none(vm, f, n)?;
        r[i] = py::f64_of(vm, &v)?;
    }
    Ok(Some(r))
}

fn fo_inside(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = super::render::rect4(vm, &arg_or(args, kw, 1, "rect", Value::None))?;
    let Some([x, y, w, h]) = rect_of(vm, &args[0])? else {
        return Ok(Value::Bool(false));
    };
    let (minx, miny, maxx, maxy) = (rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
    Ok(Value::Bool(
        minx <= x
            && x < maxx
            && miny <= y
            && y < maxy
            && minx <= x + w
            && x + w < maxx
            && miny <= y + h
            && y + h < maxy,
    ))
}

fn inset_rect(vm: &mut Vm, f: &Value) -> PyResult<[f64; 4]> {
    let [mut x, mut y, mut w, mut h] = rect_of(vm, f)?.unwrap_or([0.; 4]);
    let widget = py::get_or_none(vm, f, "widget")?;
    let style = py::get(vm, &widget, "style")?;
    let insets = py::get_or_none(vm, &style, "keyboard_focus_insets")?;
    if !py::is_none(&insets) {
        let i = super::render::rect4(vm, &insets)?;
        x += i[0];
        y += i[1];
        w -= i[0] + i[2];
        h -= i[1] + i[3];
    }
    Ok([x, y, w.max(1.), h.max(1.)])
}

fn fo_inset_rect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = inset_rect(vm, &args[0])?;
    Ok(vm
        .types
        .new_tuple(r.iter().map(|v| Value::Float(*v)).collect()))
}

// Render walks ---------------------------------------------------------------

fn screen_render(vm: &Vm) -> Option<RenderRef> {
    super::render::rs(vm)
        .borrow_mut()
        .screen_render
        .clone()
        .and_then(|v| render_data(&v))
}

struct Collected {
    widget: Value,
    arg: Value,
    rect: Option<[f64; 4]>,
    screen: Value,
}

fn transform_point(m: &Mat, x: f64, y: f64) -> (f64, f64) {
    m.transform2(x, y)
}

fn take(
    vm: &mut Vm,
    r: &RenderData,
    clip: [f64; 4],
    transform: Mat,
    screen: Value,
    out: &mut Vec<Collected>,
) -> PyResult<()> {
    let screen = if py::is_none(&r.focus_screen) {
        screen
    } else {
        r.focus_screen.clone()
    };
    let [mut cminx, mut cminy, mut cmaxx, mut cmaxy] = clip;
    let modal = r.modal.clone();
    if ops::truthy(vm, &modal)? {
        if modal.as_str() == Some("window") {
            let (x1, y1) = transform_point(&transform, 0., 0.);
            let (x2, y2) = transform_point(&transform, r.width, r.height);
            let (minx, maxx, miny, maxy) = (x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2));
            out.retain(|f| match f.rect {
                Some([x, y, w, h]) => {
                    !(minx <= x
                        && x < maxx
                        && miny <= y
                        && y < maxy
                        && minx <= x + w
                        && x + w < maxx
                        && miny <= y + h
                        && y + h < maxy)
                }
                None => true,
            });
        } else if !crysirror_python::builtins::is_callable(vm, &modal) {
            out.clear();
        }
    }
    for f in &r.focuses {
        let Some([xo, yo, w, h]) = f.rect else {
            out.push(Collected {
                widget: f.d.clone(),
                arg: f.arg.clone(),
                rect: None,
                screen: screen.clone(),
            });
            continue;
        };
        let (x1, y1) = transform_point(&transform, xo, yo);
        let (x2, y2) = transform_point(&transform, xo + w, yo + h);
        let minx = x1.min(x2).max(cminx);
        let maxx = x1.max(x2).min(cmaxx);
        let miny = y1.min(y2).max(cminy);
        let maxy = y1.max(y2).min(cmaxy);
        if maxx <= minx || maxy <= miny {
            continue;
        }
        out.push(Collected {
            widget: f.d.clone(),
            arg: f.arg.clone(),
            rect: Some([minx, miny, maxx - minx, maxy - miny]),
            screen: screen.clone(),
        });
    }
    if r.xclipping || r.yclipping {
        let (x1, y1) = transform_point(&transform, 0., 0.);
        let (x2, y2) = transform_point(&transform, r.width, r.height);
        if r.xclipping {
            cminx = cminx.max(x1.min(x2));
            cmaxx = cmaxx.min(x1.max(x2));
        }
        if r.yclipping {
            cminy = cminy.max(y1.min(y2));
            cmaxy = cmaxy.min(y1.max(y2));
        }
    }
    for c in &r.children {
        if !c.focus {
            continue;
        }
        let Source::Render(child, _) = &c.source else {
            continue;
        };
        let mut t = transform;
        if c.x != 0. || c.y != 0. {
            t = t.mul(&Mat::offset(c.x, c.y, 0.));
        }
        if let Some(rev) = &r.reverse {
            t = t.mul(rev);
        }
        let child = child.borrow();
        take(
            vm,
            &child,
            [cminx, cminy, cmaxx, cmaxy],
            t,
            screen.clone(),
            out,
        )?;
    }
    for p in &r.pass_focuses {
        let p = p.borrow();
        take(
            vm,
            &p,
            [cminx, cminy, cmaxx, cmaxy],
            transform,
            screen.clone(),
            out,
        )?;
    }
    Ok(())
}

/// `renpy.display.focus.take_focuses()`.
pub fn take_focuses(vm: &mut Vm) -> PyResult<()> {
    let mut collected = Vec::new();
    if let Some(root) = screen_render(vm) {
        let r = root.borrow();
        take(
            vm,
            &r,
            [0., 0., r.width, r.height],
            Mat::IDENTITY,
            Value::None,
            &mut collected,
        )?;
    }
    let mut list = Vec::new();
    for c in collected {
        let (x, y, w, h) = match c.rect {
            Some([x, y, w, h]) => (
                Value::Float(x),
                Value::Float(y),
                Value::Float(w),
                Value::Float(h),
            ),
            None => (Value::None, Value::None, Value::None, Value::None),
        };
        list.push(new_focus(vm, c.widget, c.arg, x, y, w, h, c.screen)?);
    }
    if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some() {
        fn count(r: &RenderData, depth: usize, unfocused: &mut usize) -> usize {
            let mut n = r.focuses.len();
            for c in &r.children {
                if let Source::Render(cr, _) = &c.source {
                    let k = count(&cr.borrow(), depth + 1, unfocused);
                    if !c.focus {
                        *unfocused += k;
                    }
                    n += k;
                }
            }
            n
        }
        let mut unfocused = 0;
        let total = screen_render(vm)
            .map(|r| count(&r.borrow(), 0, &mut unfocused))
            .unwrap_or(0);
        super::link::diagnostic(
            vm,
            format!(
                "trace: take_focuses focuses={} in_tree={total} under_unfocused_blits={unfocused}",
                list.len()
            ),
        );
    }
    let l = vm.types.new_list(list.clone());
    sg(vm, "focus_list", l)?;
    sg(vm, "global_focus", Value::None)?;
    let grab = g(vm, "grab")?;
    let mut grab_found = false;
    let mut global_focus = Value::None;
    for f in &list {
        let x = py::get_or_none(vm, f, "x")?;
        if py::is_none(&x) {
            global_focus = f.clone();
        }
        let w = py::get_or_none(vm, f, "widget")?;
        if py::same(&w, &grab) {
            grab_found = true;
        }
    }
    sg(vm, "global_focus", global_focus.clone())?;
    if !grab_found {
        sg(vm, "grab", Value::None)?;
    }
    if !py::is_none(&global_focus) && py::is_none(&get_focused(vm)?) {
        change_focus(vm, global_focus, true)?;
    }
    Ok(())
}

pub fn check_modal(
    vm: &mut Vm,
    modal: &Value,
    ev: &Value,
    x: f64,
    y: f64,
    w: Option<f64>,
    h: Option<f64>,
) -> PyResult<bool> {
    if !ops::truthy(vm, modal)? {
        return Ok(false);
    }
    let is_time = !py::is_none(ev) && super::events::is_type(vm, ev, super::events::TIMEEVENT)?;
    if is_time {
        let m = py::get_or_none(vm, ev, "modal")?;
        if ops::truthy(vm, &m)? {
            return Ok(false);
        }
    }
    let hit = if crysirror_python::builtins::is_callable(vm, modal) {
        let wv = w.map_or(Value::None, Value::Float);
        let hv = h.map_or(Value::None, Value::Float);
        let r = vm.call(
            modal,
            vec![ev.clone(), Value::Float(x), Value::Float(y), wv, hv],
            Vec::new(),
        )?;
        ops::truthy(vm, &r)?
    } else {
        match (w, h) {
            (Some(w), Some(h)) => !(x < 0. || y < 0. || x >= w || y >= h),
            _ => true,
        }
    };
    if hit {
        if is_time {
            py::set(vm, ev, "modal", Value::Bool(true))?;
            return Ok(false);
        }
        return Ok(true);
    }
    Ok(false)
}

enum Hit {
    None,
    Modal,
    Focus(Value, Value, Value),
}

fn at_point(vm: &mut Vm, r: &RenderData, x: f64, y: f64, screen: Value) -> PyResult<Hit> {
    let screen = if py::is_none(&r.focus_screen) {
        screen
    } else {
        r.focus_screen.clone()
    };
    if (r.xclipping && (x < 0. || x >= r.width)) || (r.yclipping && (y < 0. || y >= r.height)) {
        return Ok(Hit::None);
    }
    if let Some(f) = &r.forward {
        if f.0[0] == 0. && f.0[1] == 0. && f.0[4] == 0. && f.0[5] == 0. {
            return Ok(Hit::None);
        }
    }
    let mut rv = Hit::None;
    for p in r.pass_focuses.iter().rev() {
        let p = p.borrow();
        rv = at_point(vm, &p, x, y, screen.clone())?;
        if !matches!(rv, Hit::None) {
            break;
        }
    }
    if matches!(rv, Hit::None) {
        for c in r.children.iter().rev() {
            if !c.focus {
                continue;
            }
            let Source::Render(child, _) = &c.source else {
                continue;
            };
            let (mut cx, mut cy) = (x - c.x, y - c.y);
            if let Some(f) = &r.forward {
                (cx, cy) = f.transform2(cx, cy);
            }
            let child = child.borrow();
            rv = at_point(vm, &child, cx, cy, screen.clone())?;
            if !matches!(rv, Hit::None) {
                break;
            }
        }
    }
    if matches!(rv, Hit::None) {
        for f in r.focuses.iter().rev() {
            let Some([xo, yo, w, h]) = f.rect else {
                continue;
            };
            if f.arg.as_str() == Some("NO_MOUSE_FOCUS")
                || f.arg.obj().is_some_and(|o| {
                    o.class_name() == "Sentinel"
                        && ops::repr(vm, &f.arg)
                            .map(|s| s.contains("NO_MOUSE_FOCUS"))
                            .unwrap_or(false)
                })
            {
                continue;
            }
            if let Some((mx, my, mask)) = &f.mask {
                let (mut cx, mut cy) = (x - mx, y - my);
                if let Some(fw) = &r.forward {
                    (cx, cy) = fw.transform2(cx, cy);
                }
                if let Some(m) = render_data(mask) {
                    let (mw, mh) = {
                        let m = m.borrow();
                        (m.width, m.height)
                    };
                    if cx < 0. || cx >= mw || cy < 0. || cy >= mh {
                        continue;
                    }
                    if super::backend::pixel_opaque(vm, &m, cx, cy)? {
                        rv = Hit::Focus(f.d.clone(), f.arg.clone(), screen.clone());
                    }
                } else {
                    let mut res =
                        vm.call(mask, vec![Value::Float(cx), Value::Float(cy)], Vec::new())?;
                    if crysirror_python::builtins::is_callable(vm, &res) {
                        res = vm.call(
                            &res,
                            vec![
                                Value::Float(cx),
                                Value::Float(cy),
                                Value::Float(w - mx),
                                Value::Float(w - my),
                            ],
                            Vec::new(),
                        )?;
                    }
                    if ops::truthy(vm, &res)? {
                        rv = Hit::Focus(f.d.clone(), f.arg.clone(), screen.clone());
                    }
                }
            } else if xo <= x && x < xo + w && yo <= y && y < yo + h {
                rv = Hit::Focus(f.d.clone(), f.arg.clone(), screen.clone());
            }
            if !matches!(rv, Hit::None) {
                break;
            }
        }
    }
    if matches!(rv, Hit::Focus(..)) && r.operation == super::render::IMAGEDISSOLVE {
        if let Some(Source::Render(m, _)) = r.children.first().map(|c| &c.source) {
            if !super::backend::pixel_opaque(vm, m, x, y)? {
                rv = Hit::None;
            }
        }
    }
    if matches!(rv, Hit::None) && ops::truthy(vm, &r.modal)? {
        let (w, h) = if r.modal.as_str() == Some("default") {
            (None, None)
        } else {
            (Some(r.width), Some(r.height))
        };
        if check_modal(vm, &r.modal, &Value::None, x, y, w, h)? {
            return Ok(Hit::Modal);
        }
    }
    Ok(rv)
}

/// `renpy.display.render.focus_at_point(x, y)`.
pub fn focus_at_point(vm: &mut Vm, x: f64, y: f64) -> PyResult<Value> {
    let Some(root) = screen_render(vm) else {
        return Ok(Value::None);
    };
    let hit = {
        let r = root.borrow();
        at_point(vm, &r, x, y, Value::None)?
    };
    match hit {
        Hit::Focus(d, a, screen) => new_focus(
            vm,
            d,
            a,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            screen,
        ),
        _ => Ok(Value::None),
    }
}

// Focus changes ----------------------------------------------------------------

fn push_screen(vm: &mut Vm, screen: &Value) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.display.screen", "push_current_screen")?;
    vm.call(&f, vec![screen.clone()], Vec::new())?;
    Ok(())
}

fn pop_screen(vm: &mut Vm) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.display.screen", "pop_current_screen")?;
    vm.call(&f, Vec::new(), Vec::new())?;
    Ok(())
}

fn with_screen<T>(
    vm: &mut Vm,
    screen: &Value,
    f: impl FnOnce(&mut Vm) -> PyResult<T>,
) -> PyResult<T> {
    push_screen(vm, screen)?;
    let r = f(vm);
    pop_screen(vm)?;
    r
}

fn screen_names(vm: &mut Vm, screen: &Value) -> PyResult<Value> {
    let set = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    if !py::is_none(screen) {
        let name = py::get(vm, screen, "screen_name")?;
        let first = ops::getitem(vm, &name, Value::Int(0))?;
        vm.call_method(&set, "add", vec![first])?;
        let tag = py::get_or_none(vm, screen, "tag")?;
        vm.call_method(&set, "add", vec![tag])?;
    }
    Ok(set)
}

pub fn set_focused(vm: &mut Vm, widget: Value, arg: Value, screen: Value) -> PyResult<()> {
    sg(vm, "argument", arg)?;
    sg(vm, "screen_of_focused", screen.clone())?;
    let names = screen_names(vm, &screen)?;
    sg(vm, "screen_of_focused_names", names.clone())?;
    set_focused_widget(vm, widget.clone())?;
    if let Ok(tts) = py::import_attr(vm, "renpy.display.tts", "displayable") {
        vm.call(&tts, vec![widget.clone()], Vec::new())?;
    }
    let new_tooltip = if py::is_none(&widget) {
        Value::None
    } else {
        vm.call_method(&widget, "_get_tooltip", Vec::new())?
    };
    let tooltip = g(vm, "tooltip")?;
    if !ops::eq(vm, &tooltip, &new_tooltip)? {
        sg(vm, "tooltip", new_tooltip.clone())?;
        capture_focus(vm, "tooltip")?;
        let restart = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
        vm.call(&restart, Vec::new(), Vec::new())?;
        if !py::is_none(&new_tooltip) {
            sg(vm, "last_tooltip", new_tooltip)?;
            sg(vm, "screen_of_last_focused_names", names)?;
        }
    }
    Ok(())
}

pub fn change_focus(vm: &mut Vm, newfocus: Value, default: bool) -> PyResult<Value> {
    let grab = g(vm, "grab")?;
    if ops::truthy(vm, &grab)? {
        return Ok(Value::None);
    }
    let widget = if py::is_none(&newfocus) {
        Value::None
    } else {
        py::get(vm, &newfocus, "widget")?
    };
    let current = get_focused(vm)?;
    let argument = g(vm, "argument")?;
    if py::same(&current, &widget) {
        let same_arg = if py::is_none(&newfocus) {
            true
        } else {
            let a = py::get(vm, &newfocus, "arg")?;
            ops::eq(vm, &a, &argument)?
        };
        if same_arg {
            return Ok(Value::None);
        }
    }
    let pending = g(vm, "pending_focus_type")?;
    sg(vm, "focus_type", pending)?;
    if !py::is_none(&current) {
        let screen = g(vm, "screen_of_focused")?;
        with_screen(vm, &screen, |vm| {
            let f = py::get(vm, &current, "unfocus")?;
            vm.call(
                &f,
                Vec::new(),
                vec![("default".into(), Value::Bool(default))],
            )
        })?;
    }
    if py::is_none(&newfocus) {
        set_focused(vm, Value::None, Value::None, Value::None)?;
    } else {
        let a = py::get(vm, &newfocus, "arg")?;
        let s = py::get(vm, &newfocus, "screen")?;
        set_focused(vm, widget.clone(), a, s)?;
    }
    let mut rv = Value::None;
    if !py::is_none(&widget) {
        let screen = g(vm, "screen_of_focused")?;
        rv = with_screen(vm, &screen, |vm| {
            let f = py::get(vm, &widget, "focus")?;
            vm.call(
                &f,
                Vec::new(),
                vec![("default".into(), Value::Bool(default))],
            )
        })?;
    }
    Ok(rv)
}

fn focus_coordinates(vm: &mut Vm) -> PyResult<Option<[f64; 4]>> {
    let current = get_focused(vm)?;
    let argument = g(vm, "argument")?;
    let list = g(vm, "focus_list")?;
    for f in py::items(vm, &list)? {
        let w = py::get(vm, &f, "widget")?;
        let a = py::get(vm, &f, "arg")?;
        if ops::eq(vm, &w, &current)? && ops::eq(vm, &a, &argument)? {
            return rect_of(vm, &f);
        }
    }
    Ok(None)
}

fn capture_focus(vm: &mut Vm, name: &str) -> PyResult<()> {
    let storage = g(vm, "focus_storage")?;
    let key = vm.types.new_str(name);
    match focus_coordinates(vm)? {
        Some(r) => {
            let t = vm
                .types
                .new_tuple(r.iter().map(|v| Value::Float(*v)).collect());
            ops::setitem(vm, &storage, key, t)?;
        }
        None => {
            vm.call_method(&storage, "pop", vec![key, Value::None])?;
        }
    }
    Ok(())
}

fn f_capture_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", vm.types.new_str("default"));
    let name = py::string(vm, &name)?;
    capture_focus(vm, &name)?;
    Ok(Value::None)
}

fn f_clear_capture_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", vm.types.new_str("default"));
    let storage = g(vm, "focus_storage")?;
    if py::is_none(&name) {
        vm.call_method(&storage, "clear", Vec::new())?;
    } else {
        vm.call_method(&storage, "pop", vec![name, Value::None])?;
    }
    Ok(Value::None)
}

fn f_get_focus_rect(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", vm.types.new_str("default"));
    let storage = g(vm, "focus_storage")?;
    vm.call_method(&storage, "get", vec![name, Value::None])
}

fn f_get_focused(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    get_focused(vm)
}

fn f_set_focused(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = |i: usize| args.get(i).cloned().unwrap_or(Value::None);
    set_focused(vm, a(0), a(1), a(2))?;
    Ok(Value::None)
}

fn f_get_mouse(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let focused = get_focused(vm)?;
    if py::is_none(&focused) {
        return Ok(Value::None);
    }
    let style = py::get(vm, &focused, "style")?;
    let mouse = py::get_or_none(vm, &style, "mouse")?;
    let button = py::import_attr(vm, "renpy.display.behavior", "Button")?;
    if let Some(b) = button.obj() {
        if ops::isinstance(vm, &focused, b) && !ops::truthy(vm, &mouse)? {
            return Ok(vm.types.new_str("button"));
        }
    }
    Ok(mouse)
}

fn f_get_tooltip(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let screen = arg_or(args, kw, 0, "screen", Value::None);
    let last = match arg(args, kw, 1, "last") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    if py::is_none(&screen) {
        return g(vm, if last { "last_tooltip" } else { "tooltip" });
    }
    let names = g(
        vm,
        if last {
            "screen_of_last_focused_names"
        } else {
            "screen_of_focused_names"
        },
    )?;
    if ops::contains(vm, &names, &screen)? {
        return g(vm, if last { "last_tooltip" } else { "tooltip" });
    }
    Ok(Value::None)
}

fn f_set_grab(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    sg(vm, "grab", args.first().cloned().unwrap_or(Value::None))?;
    if let Ok(cancel) = py::import_attr(vm, "renpy.exports", "cancel_gesture") {
        vm.call(&cancel, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn f_get_grab(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    g(vm, "grab")
}

fn f_take_focuses(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    take_focuses(vm)?;
    Ok(Value::None)
}

fn f_focus_coordinates(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(match focus_coordinates(vm)? {
        Some(r) => vm
            .types
            .new_tuple(r.iter().map(|v| Value::Float(*v)).collect()),
        None => vm.types.new_tuple(vec![Value::None; 4]),
    })
}

fn f_mark_modal(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let g0 = g(vm, "modal_generation")?;
    let n = py::int(vm, &g0)? + 1;
    sg(vm, "modal_generation", Value::Int(n))?;
    Ok(Value::None)
}

/// `before_interact(roots)`: names focusables, restores the focus across
/// interactions and applies default focus.
pub fn before_interact(vm: &mut Vm, roots: &[Value]) -> PyResult<()> {
    sg(vm, "modal_generation", Value::Int(0))?;
    let fwn = vm.types.new_list(Vec::new());
    let cb = py::closure(vm, "focus_callback", focus_callback, fwn.clone());
    let ignore = py::import_attr(vm, "renpy.display.layout", "IgnoreLayers").ok();
    for root in roots {
        let f = py::get(vm, root, "find_focusable")?;
        match vm.call(&f, vec![cb.clone(), Value::None], Vec::new()) {
            Ok(_) => {}
            Err(crysirror_python::PyErr::Exc(e))
                if ignore
                    .as_ref()
                    .and_then(|c| c.obj())
                    .is_some_and(|c| ops::isinstance(vm, &Value::Obj(e.clone()), c)) => {}
            Err(e) => return Err(e),
        }
    }
    let mut namecount: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let replaced_by = g(vm, "replaced_by")?;
    let mut entries = Vec::new();
    for t in py::items(vm, &fwn)? {
        let t = py::items(vm, &t)?;
        let (f, n, screen, generation) = (t[0].clone(), t[1].clone(), t[2].clone(), t[3].clone());
        let key = ops::repr(vm, &n)?.to_string();
        let serial = *namecount.get(&key).unwrap_or(&0);
        namecount.insert(key, serial + 1);
        if py::is_none(&f) {
            continue;
        }
        let ffn = vm.types.new_tuple(vec![n.clone(), Value::Int(serial)]);
        py::set(vm, &f, "full_focus_name", ffn)?;
        ops::setitem(vm, &replaced_by, Value::Int(f.id()), f.clone())?;
        let generation = py::int(vm, &generation)?;
        entries.push((f, screen, generation));
    }
    let modal_generation = {
        let v = g(vm, "modal_generation")?;
        py::int(vm, &v)?
    };
    let mut defaults: Vec<(f64, Value, Value)> = Vec::new();
    for (f, screen, generation) in &entries {
        if *generation != modal_generation {
            continue;
        }
        let d = py::get_or_none(vm, f, "default")?;
        if ops::truthy(vm, &d)? {
            let dv = py::f64_of(vm, &d)?;
            defaults.push((dv, f.clone(), screen.clone()));
        }
    }
    defaults.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (max_default, max_default_focus, max_default_screen) = defaults
        .last()
        .cloned()
        .unwrap_or((0., Value::None, Value::None));
    let max_default_focus_name = if py::is_none(&max_default_focus) {
        Value::None
    } else {
        py::get(vm, &max_default_focus, "full_focus_name")?
    };
    let interface = py::import_attr(vm, "renpy.display", "interface").unwrap_or(Value::None);
    let (input_t, mouse_t, start_interact) = if py::is_none(&interface) {
        (0., 0., true)
    } else {
        let a = py::get_or_none(vm, &interface, "input_event_time")?;
        let b = py::get_or_none(vm, &interface, "mouse_event_time")?;
        let s = py::get_or_none(vm, &interface, "start_interact")?;
        (
            if py::is_none(&a) {
                0.
            } else {
                py::f64_of(vm, &a)?
            },
            if py::is_none(&b) {
                0.
            } else {
                py::f64_of(vm, &b)?
            },
            ops::truthy(vm, &s)?,
        )
    };
    let should_max_default = input_t > mouse_t + 0.1;
    let mut explicit = false;
    vm.call_method(&replaced_by, "pop", vec![Value::None, Value::None])?;
    let mut current = get_focused(vm)?;
    if !py::is_none(&current) {
        current = vm.call_method(
            &replaced_by,
            "get",
            vec![Value::Int(current.id()), current.clone()],
        )?;
    }
    let old_current = current.clone();
    let grab = g(vm, "grab")?;
    let grab = if py::is_none(&grab) {
        Value::None
    } else {
        vm.call_method(
            &replaced_by,
            "get",
            vec![Value::Int(grab.id()), Value::None],
        )?
    };
    sg(vm, "grab", grab.clone())?;
    let override_ = g(vm, "override")?;
    if !py::is_none(&override_) {
        let gd = py::import_attr(vm, "renpy.exports", "get_displayable")?;
        let a = py::items(vm, &override_)?;
        let d = vm.call(&gd, a, vec![("base".into(), Value::Bool(true))])?;
        if !py::is_none(&d) && !py::same(&current, &d) && !ops::truthy(vm, &grab)? {
            current = d;
            explicit = true;
        }
    }
    sg(vm, "override", Value::None)?;
    let old_max_default = {
        let v = g(vm, "old_max_default")?;
        if py::is_none(&v) {
            0.
        } else {
            py::f64_of(vm, &v)?
        }
    };
    if !explicit && should_max_default && max_default > old_max_default {
        current = max_default_focus.clone();
        sg(vm, "grab", Value::None)?;
        set_focused(
            vm,
            max_default_focus.clone(),
            Value::None,
            max_default_screen.clone(),
        )?;
        explicit = true;
    }
    sg(vm, "old_max_default", Value::Float(max_default))?;
    if !py::is_none(&current) {
        let current_name = py::get_or_none(vm, &current, "full_focus_name")?;
        let mut found = false;
        for (f, screen, generation) in &entries {
            if *generation != modal_generation {
                continue;
            }
            let n = py::get_or_none(vm, f, "full_focus_name")?;
            if ops::eq(vm, &n, &current_name)? {
                current = f.clone();
                let argument = g(vm, "argument")?;
                set_focused(vm, f.clone(), argument, screen.clone())?;
                found = true;
                break;
            }
        }
        if !found {
            current = Value::None;
        }
    }
    let grab = g(vm, "grab")?;
    if !py::is_none(&grab) {
        current = grab;
    }
    let old_name = g(vm, "old_max_default_focus_name")?;
    if should_max_default
        && max_default > 0.
        && py::is_none(&current)
        && (start_interact || !ops::eq(vm, &max_default_focus_name, &old_name)?)
    {
        explicit = true;
        current = max_default_focus.clone();
        set_focused(
            vm,
            max_default_focus.clone(),
            Value::None,
            max_default_screen.clone(),
        )?;
    }
    sg(vm, "old_max_default_focus_name", max_default_focus_name)?;
    if py::is_none(&current) {
        set_focused(vm, Value::None, Value::None, Value::None)?;
    }
    let always_unfocus = py::config(vm, "always_unfocus")
        .map(|v| ops::truthy(vm, &v))
        .unwrap_or(Ok(true))?;
    for (f, screen, _) in &entries {
        if py::same(f, &current) {
            continue;
        }
        let default = if py::same(f, &old_current) && always_unfocus {
            false
        } else {
            !explicit
        };
        with_screen(vm, screen, |vm| {
            let u = py::get(vm, f, "unfocus")?;
            vm.call(
                &u,
                Vec::new(),
                vec![("default".into(), Value::Bool(default))],
            )
        })?;
    }
    if !py::is_none(&current) && ops::truthy(vm, &current)? {
        let screen = g(vm, "screen_of_focused")?;
        with_screen(vm, &screen, |vm| {
            let fo = py::get(vm, &current, "focus")?;
            vm.call(
                &fo,
                Vec::new(),
                vec![("default".into(), Value::Bool(!explicit))],
            )
        })?;
    }
    vm.call_method(&replaced_by, "clear", Vec::new())?;
    Ok(())
}

fn focus_callback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    // args: (fwn list, f, n)
    let list = args[0].clone();
    let f = args.get(1).cloned().unwrap_or(Value::None);
    let n = args.get(2).cloned().unwrap_or(Value::None);
    let screen =
        py::import_attr(vm, "renpy.display.screen", "_current_screen").unwrap_or(Value::None);
    let generation = g(vm, "modal_generation")?;
    let t = vm.types.new_tuple(vec![f, n, screen, generation]);
    ops::list_append(vm, &list, t)?;
    Ok(Value::None)
}

fn f_before_interact(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let roots = arg_or(args, kw, 0, "roots", Value::None);
    let roots = py::items(vm, &roots)?;
    before_interact(vm, &roots)?;
    Ok(Value::None)
}

fn f_change_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let nf = arg_or(args, kw, 0, "newfocus", Value::None);
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    change_focus(vm, nf, default)
}

fn f_clear_focus(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    sg(vm, "grab", Value::None)?;
    change_focus(vm, Value::None, false)?;
    Ok(Value::None)
}

fn f_force_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let a = arg_or(args, kw, 1, "arg", Value::None);
    let current = get_focused(vm)?;
    let argument = g(vm, "argument")?;
    if py::same(&current, &d) && ops::eq(vm, &a, &argument)? {
        return Ok(Value::None);
    }
    sg(vm, "grab", Value::None)?;
    if !py::is_none(&current) {
        let screen = g(vm, "screen_of_focused")?;
        with_screen(vm, &screen, |vm| {
            let u = py::get(vm, &current, "unfocus")?;
            vm.call(&u, Vec::new(), vec![("default".into(), Value::Bool(false))])
        })?;
    }
    let cs = py::import_attr(vm, "renpy.display.screen", "current_screen")?;
    let screen = vm.call(&cs, Vec::new(), Vec::new())?;
    set_focused(vm, d.clone(), a, screen)?;
    let fo = py::get(vm, &d, "focus")?;
    vm.call(
        &fo,
        Vec::new(),
        vec![("default".into(), Value::Bool(false))],
    )
}

/// `mouse_handler(ev, x, y, default=False)`.
pub fn mouse_handler(vm: &mut Vm, ev: &Value, x: f64, y: f64, default: bool) -> PyResult<Value> {
    let grab = g(vm, "grab")?;
    if ops::truthy(vm, &grab)? {
        return Ok(Value::None);
    }
    if !py::is_none(ev) {
        let ok = super::events::is_mouse(vm, ev)?;
        if !ok {
            return Ok(Value::None);
        }
        let m = vm.types.new_str("mouse");
        sg(vm, "pending_focus_type", m)?;
    }
    let mut nf = focus_at_point(vm, x, y)?;
    if py::is_none(&nf) {
        nf = g(vm, "global_focus")?;
    }
    change_focus(vm, nf, default)
}

fn f_mouse_handler(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ev = arg_or(args, kw, 0, "ev", Value::None);
    let x = py::f64_of(vm, &arg_or(args, kw, 1, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 2, "y", Value::Int(0)))?;
    let default = match arg(args, kw, 3, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    mouse_handler(vm, &ev, x, y, default)
}

fn keyboard_focus(vm: &mut Vm, f: &Value) -> PyResult<bool> {
    let w = py::get(vm, f, "widget")?;
    let s = py::get(vm, &w, "style")?;
    let k = py::get_or_none(vm, &s, "keyboard_focus")?;
    ops::truthy(vm, &k)
}

fn focus_extreme(vm: &mut Vm, mul: [f64; 4]) -> PyResult<Value> {
    let list = g(vm, "focus_list")?;
    let mut best: Option<(f64, Value)> = None;
    for f in py::items(vm, &list)? {
        if !keyboard_focus(vm, &f)? {
            continue;
        }
        let Some([x, y, w, h]) = rect_of(vm, &f)? else {
            continue;
        };
        let score = x * mul[0] + y * mul[1] + w * mul[2] + h * mul[3];
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, f));
        }
    }
    match best {
        Some((_, f)) => change_focus(vm, f, false),
        None => Ok(Value::None),
    }
}

fn check_keyboard_focus(vm: &mut Vm) -> PyResult<()> {
    let current = get_focused(vm)?;
    if ops::truthy(vm, &current)? {
        let s = py::get(vm, &current, "style")?;
        let k = py::get_or_none(vm, &s, "keyboard_focus")?;
        if !ops::truthy(vm, &k)? {
            change_focus(vm, Value::None, false)?;
        }
    }
    Ok(())
}

fn points_dist(x0: f64, y0: f64, x1: f64, y1: f64, xf: f64, yf: f64) -> f64 {
    ((x0 - x1) * xf).powi(2) + ((y0 - y1) * yf).powi(2)
}

fn horiz_line_dist(p: f64, a: [f64; 4], b: [f64; 4]) -> f64 {
    let [ax0, ay0, ax1, ay1] = a;
    let [bx0, by0, bx1, by1] = b;
    if (bx0 <= ax0 && ax0 <= ax1 && ax1 <= bx1)
        || (ax0 <= bx0 && bx0 <= bx1 && bx1 <= ax1)
        || (ax0 <= bx0 && bx0 <= ax1 && ax1 <= bx1)
        || (bx0 <= ax0 && ax0 <= bx1 && bx1 <= ax1)
    {
        return (ay0 - by0).powi(2);
    }
    if ax0 <= ax1 && ax1 <= bx0 && bx0 <= bx1 {
        points_dist(ax1, ay1, bx0, by0, p, 1.)
    } else {
        points_dist(ax0, ay0, bx1, by1, p, 1.)
    }
}

fn verti_line_dist(p: f64, a: [f64; 4], b: [f64; 4]) -> f64 {
    let [ax0, ay0, ax1, ay1] = a;
    let [bx0, by0, bx1, by1] = b;
    if (by0 <= ay0 && ay0 <= ay1 && ay1 <= by1)
        || (ay0 <= by0 && by0 <= by1 && by1 <= ay1)
        || (ay0 <= by0 && by0 <= ay1 && ay1 <= by1)
        || (by0 <= ay0 && ay0 <= by1 && by1 <= ay1)
    {
        return (ax0 - bx0).powi(2);
    }
    if ay0 <= ay1 && ay1 <= by0 && by0 <= by1 {
        points_dist(ax1, ay1, bx0, by0, 1., p)
    } else {
        points_dist(ax0, ay0, bx1, by1, 1., p)
    }
}

#[derive(Clone, Copy)]
enum Dir {
    Right,
    Left,
    Up,
    Down,
}

fn focus_nearest(vm: &mut Vm, dir: Dir) -> PyResult<Value> {
    let (from, to, vertical, mul): ([f64; 4], [f64; 4], bool, [f64; 4]) = match dir {
        Dir::Right => (
            [0.9, 0.1, 0.9, 0.9],
            [0.1, 0.1, 0.1, 0.9],
            true,
            [-1., 0., 0., 0.],
        ),
        Dir::Left => (
            [0.1, 0.1, 0.1, 0.9],
            [0.9, 0.1, 0.9, 0.9],
            true,
            [1., 0., 1., 0.],
        ),
        Dir::Up => (
            [0.1, 0.1, 0.9, 0.1],
            [0.1, 0.9, 0.9, 0.9],
            false,
            [0., 1., 0., 1.],
        ),
        Dir::Down => (
            [0.1, 0.9, 0.9, 0.9],
            [0.1, 0.1, 0.9, 0.1],
            false,
            [0., -1., 0., 0.],
        ),
    };
    let k = vm.types.new_str("keyboard");
    sg(vm, "pending_focus_type", k)?;
    let list = g(vm, "focus_list")?;
    let list = py::items(vm, &list)?;
    if list.is_empty() {
        return Ok(Value::None);
    }
    check_keyboard_focus(vm)?;
    let current = get_focused(vm)?;
    if !ops::truthy(vm, &current)? {
        focus_extreme(vm, mul)?;
        if !py::is_none(&get_focused(vm)?) {
            return Ok(Value::None);
        }
        for f in &list {
            let x = py::get_or_none(vm, f, "x")?;
            if matches!(x, Value::Bool(false)) || !keyboard_focus(vm, f)? {
                continue;
            }
            change_focus(vm, f.clone(), false)?;
            return Ok(Value::None);
        }
        return Ok(Value::None);
    }
    let argument = g(vm, "argument")?;
    let mut from_focus = None;
    for f in &list {
        let w = py::get(vm, f, "widget")?;
        let a = py::get(vm, f, "arg")?;
        if py::same(&w, &current) && ops::eq(vm, &a, &argument)? {
            from_focus = Some(f.clone());
            break;
        }
    }
    let Some(from_focus) = from_focus else {
        return change_focus(vm, list[0].clone(), false);
    };
    if rect_of(vm, &from_focus)?.is_none() {
        return focus_extreme(vm, mul);
    }
    let fr = inset_rect(vm, &from_focus)?;
    let fl = [
        fr[0] + fr[2] * from[0],
        fr[1] + fr[3] * from[1],
        fr[0] + fr[2] * from[2],
        fr[1] + fr[3] * from[3],
    ];
    let penalty = py::config_f64(vm, "focus_crossrange_penalty").unwrap_or(1024.);
    let mut placeless = None;
    let mut best: Option<Value> = None;
    let mut best_dist = (65536.0 * penalty).powi(2);
    for f in &list {
        if py::same(f, &from_focus) || !keyboard_focus(vm, f)? {
            continue;
        }
        let x = py::get_or_none(vm, f, "x")?;
        if py::is_none(&x) {
            placeless = Some(f.clone());
            continue;
        }
        if matches!(x, Value::Bool(false)) {
            continue;
        }
        let tr = inset_rect(vm, f)?;
        let ok = match dir {
            Dir::Right => fr[0] + fr[2] <= tr[0],
            Dir::Left => tr[0] + tr[2] <= fr[0],
            Dir::Up => tr[1] + tr[3] <= fr[1],
            Dir::Down => fr[1] + fr[3] <= tr[1],
        };
        if !ok {
            continue;
        }
        let tl = [
            tr[0] + tr[2] * to[0],
            tr[1] + tr[3] * to[1],
            tr[0] + tr[2] * to[2],
            tr[1] + tr[3] * to[3],
        ];
        let d = if vertical {
            verti_line_dist(penalty, fl, tl)
        } else {
            horiz_line_dist(penalty, fl, tl)
        };
        if d < best_dist {
            best = Some(f.clone());
            best_dist = d;
        }
    }
    match best.or(placeless) {
        Some(f) => change_focus(vm, f, false),
        None => Ok(Value::None),
    }
}

fn focus_ordered(vm: &mut Vm, delta: i64) -> PyResult<Value> {
    let k = vm.types.new_str("keyboard");
    sg(vm, "pending_focus_type", k)?;
    check_keyboard_focus(vm)?;
    let current = get_focused(vm)?;
    let list = g(vm, "focus_list")?;
    let mut placeless = None;
    let mut candidates = Vec::new();
    let mut current_index = None;
    for f in py::items(vm, &list)? {
        let x = py::get_or_none(vm, &f, "x")?;
        if py::is_none(&x) {
            placeless = Some(f);
            continue;
        }
        if matches!(x, Value::Bool(false)) {
            continue;
        }
        let a = py::get_or_none(vm, &f, "arg")?;
        if !py::is_none(&a) || !keyboard_focus(vm, &f)? {
            continue;
        }
        let w = py::get(vm, &f, "widget")?;
        if py::same(&w, &current) {
            current_index = Some(candidates.len() as i64);
        }
        candidates.push(f);
    }
    let n = candidates.len() as i64;
    let pick = match current_index {
        None if n > 0 => {
            let i = if delta > 0 { delta - 1 } else { n + delta };
            (0..n).contains(&i).then(|| candidates[i as usize].clone())
        }
        None => None,
        Some(c) => {
            let i = c + delta;
            (0..n).contains(&i).then(|| candidates[i as usize].clone())
        }
    };
    change_focus(vm, pick.or(placeless).unwrap_or(Value::None), false)
}

fn f_key_handler(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ev = arg_or(args, kw, 0, "ev", Value::None);
    let map_event = py::import_attr(vm, "renpy.display.behavior", "map_event")?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    let sv = py::get_or_none(vm, &prefs, "self_voicing")?;
    let mut m = |vm: &mut Vm, name: &str| -> PyResult<bool> {
        let n = vm.types.new_str(name);
        let r = vm.call(&map_event, vec![ev.clone(), n], Vec::new())?;
        ops::truthy(vm, &r)
    };
    if ops::truthy(vm, &sv)? {
        if m(vm, "focus_right")? || m(vm, "focus_down")? {
            return focus_ordered(vm, 1);
        }
        if m(vm, "focus_left")? || m(vm, "focus_up")? {
            return focus_ordered(vm, -1);
        }
    } else {
        for (name, dir) in [
            ("focus_right", Dir::Right),
            ("focus_left", Dir::Left),
            ("focus_up", Dir::Up),
            ("focus_down", Dir::Down),
        ] {
            if m(vm, name)? {
                return focus_nearest(vm, dir);
            }
        }
    }
    Ok(Value::None)
}

fn f_focus_ordered(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::int(vm, &args.first().cloned().unwrap_or(Value::Int(1)))?;
    focus_ordered(vm, d)
}

fn f_focus_extreme(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut m = [0.; 4];
    for (i, v) in m.iter_mut().enumerate() {
        *v = py::f64_of(vm, &args.get(i).cloned().unwrap_or(Value::Int(0)))?;
    }
    focus_extreme(vm, m)
}

fn f_check_keyboard_focus(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    check_keyboard_focus(vm)?;
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("capture_focus", f_capture_focus),
            ("clear_capture_focus", f_clear_capture_focus),
            ("get_focus_rect", f_get_focus_rect),
            ("set_focused", f_set_focused),
            ("get_focused", f_get_focused),
            ("get_mouse", f_get_mouse),
            ("get_tooltip", f_get_tooltip),
            ("set_grab", f_set_grab),
            ("get_grab", f_get_grab),
            ("take_focuses", f_take_focuses),
            ("focus_coordinates", f_focus_coordinates),
            ("mark_modal", f_mark_modal),
            ("before_interact", f_before_interact),
            ("change_focus", f_change_focus),
            ("clear_focus", f_clear_focus),
            ("force_focus", f_force_focus),
            ("mouse_handler", f_mouse_handler),
            ("focus_extreme", f_focus_extreme),
            ("check_keyboard_focus", f_check_keyboard_focus),
            ("focus_ordered", f_focus_ordered),
            ("key_handler", f_key_handler),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let fc = py::class(
        vm,
        MODULE,
        "Focus",
        &[],
        &[
            ("__init__", fo_init),
            ("copy", fo_copy),
            ("__repr__", fo_repr),
            ("inside", fo_inside),
            ("inset_rect", fo_inset_rect),
        ],
        &[],
    )?;
    set(vm, "Focus", Value::Obj(fc));
    set(vm, "focus_storage", vm.types.new_dict());
    for n in [
        "argument",
        "screen_of_focused",
        "grab",
        "global_focus",
        "tooltip",
        "last_tooltip",
        "override",
        "old_max_default_focus_name",
        "_focused",
    ] {
        set(vm, n, Value::None);
    }
    let empty_set = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    set(vm, "screen_of_focused_names", empty_set.clone());
    set(vm, "screen_of_last_focused_names", empty_set);
    let mouse = vm.types.new_str("mouse");
    set(vm, "focus_type", mouse.clone());
    set(vm, "pending_focus_type", mouse);
    set(vm, "focus_list", vm.types.new_list(Vec::new()));
    set(vm, "replaced_by", vm.types.new_dict());
    set(vm, "modal_generation", Value::Int(0));
    set(vm, "old_max_default", Value::Int(0));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
