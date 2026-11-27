// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.screen`, after Ren'Py 8.3 `screen.py` (MIT): the screen
//! registry and variants, `ScreenDisplayable` with its update phases, the
//! current-screen stack, and `show_screen`/`hide_screen`/`use_screen`/
//! `predict_screen`.
//!
//! Module state (`screens`, `screens_by_name`, `updated_screens`,
//! `predict_cache`, `_current_screen`, ...) lives in the module namespace,
//! where scripts and the screen language read it. Profiling
//! (`ScreenProfile`) records requests but writes no developer log.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.display.screen";

pub const PREDICT: i64 = 0;
pub const SHOW: i64 = 1;
pub const UPDATE: i64 = 2;
pub const HIDE: i64 = 3;
pub const OLD: i64 = 4;
const PHASE_NAMES: [&str; 5] = ["PREDICT", "SHOW", "UPDATE", "HIDE", "OLD"];

fn m(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, MODULE)
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = m(vm)?;
    py::set(vm, &m, n, v)
}

fn py_eval(vm: &mut Vm, code: &Value, locals: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.python", "py_eval")?;
    vm.call(&f, vec![code.clone()], vec![("locals".into(), locals.clone())])
}

/// Splits a screen name into its tuple form.
fn name_tuple(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    if let Some(s) = name.as_str() {
        let parts: Vec<Value> = s.split_whitespace().map(|p| vm.types.new_str(p)).collect();
        return Ok(vm.types.new_tuple(parts));
    }
    if matches!(name.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return Ok(name.clone());
    }
    let items = py::items(vm, name)?;
    Ok(vm.types.new_tuple(items))
}

fn first(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    if name.as_str().is_some() {
        let t = name_tuple(vm, name)?;
        return ops::getitem(vm, &t, Value::Int(0));
    }
    ops::getitem(vm, name, Value::Int(0))
}

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter().position(|(k, _)| &**k == n).map(|i| kw.remove(i).1)
}

fn container_method(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let f = py::get(vm, &c, name)?;
    vm.call(&f, args, Vec::new())
}

// Profiles and the prediction cache --------------------------------------------------------------------

fn profile_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["predict", "show", "update", "request", "time", "debug", "const"].iter().enumerate() {
        let v = arg_or(args, kw, i + 2, n, Value::Bool(false));
        py::set(vm, this, n, v)?;
    }
    let name = arg_or(args, kw, 1, "name", Value::None);
    if !py::is_none(&name) {
        let name = if name.as_str().is_some() { name_tuple(vm, &name)? } else { name };
        let profile = mget(vm, "profile")?;
        ops::setitem(vm, &profile, name, this.clone())?;
    }
    Ok(Value::None)
}

fn f_get_profile(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = if args[0].as_str().is_some() { name_tuple(vm, &args[0])? } else { args[0].clone() };
    let profile = mget(vm, "profile")?;
    if ops::contains(vm, &profile, &name)? {
        return ops::getitem(vm, &profile, name);
    }
    let c = mget(vm, "ScreenProfile")?;
    vm.call(&c, vec![Value::None], Vec::new())
}

fn predict_list(vm: &mut Vm, screen: &Value) -> PyResult<Value> {
    let pc = mget(vm, "predict_cache")?;
    if !ops::contains(vm, &pc, screen)? {
        ops::setitem(vm, &pc, screen.clone(), vm.types.new_list(Vec::new()))?;
    }
    ops::getitem(vm, &pc, screen.clone())
}

fn has_ast(vm: &mut Vm, screen: &Value) -> PyResult<bool> {
    let a = py::get_or_none(vm, screen, "ast")?;
    Ok(!py::is_none(&a))
}

fn cache_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let screen = arg_or(args, kw, 1, "screen", Value::None);
    if !has_ast(vm, &screen)? {
        return Ok(Value::None);
    }
    py::set(vm, this, "args", arg_or(args, kw, 2, "args", Value::None))?;
    py::set(vm, this, "kwargs", arg_or(args, kw, 3, "kwargs", Value::None))?;
    py::set(vm, this, "cache", arg_or(args, kw, 4, "cache", Value::None))?;
    let pc = predict_list(vm, &screen)?;
    ops::list_append(vm, &pc, this.clone())?;
    let size = py::config(vm, "screen_cache_size")?;
    let size = py::int(vm, &size)?;
    if ops::len(vm, &pc)? as i64 > size {
        vm.call_method(&pc, "pop", vec![Value::Int(0)])?;
    }
    Ok(Value::None)
}

fn f_cache_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    cache_get(vm, &args[0], &args[1], &args[2])
}

fn cache_get(vm: &mut Vm, screen: &Value, a: &Value, k: &Value) -> PyResult<Value> {
    if !has_ast(vm, screen)? {
        return Ok(vm.types.new_dict());
    }
    let pc = predict_list(vm, screen)?;
    if ops::len(vm, &pc)? == 0 {
        return Ok(vm.types.new_dict());
    }
    let mut found = None;
    for sc in py::items(vm, &pc)? {
        let sa = py::get_or_none(vm, &sc, "args")?;
        let sk = py::get_or_none(vm, &sc, "kwargs")?;
        if ops::eq(vm, &sa, a)? && ops::eq(vm, &sk, k)? {
            vm.call_method(&pc, "remove", vec![sc.clone()])?;
            found = Some(sc);
            break;
        }
    }
    let sc = match found {
        Some(sc) => sc,
        None => vm.call_method(&pc, "pop", vec![Value::Int(0)])?,
    };
    py::get(vm, &sc, "cache")
}

// Screen -----------------------------------------------------------------------------------------------

fn screen_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut name = arg_or(args, kw, 1, "name", Value::None);
    let function = arg_or(args, kw, 2, "function", Value::None);
    let modal = arg_or(args, kw, 3, "modal", py::s(vm, "False"));
    let zorder = arg_or(args, kw, 4, "zorder", py::s(vm, "0"));
    let tag = arg_or(args, kw, 5, "tag", Value::None);
    let mut predict = arg_or(args, kw, 6, "predict", Value::None);
    let mut variant = arg_or(args, kw, 7, "variant", Value::None);
    let parameters = arg_or(args, kw, 8, "parameters", Value::Bool(false));
    let location = arg_or(args, kw, 9, "location", Value::None);
    let layer = arg_or(args, kw, 10, "layer", py::s(vm, "screens"));
    let sensitive = arg_or(args, kw, 11, "sensitive", py::s(vm, "True"));
    let roll_forward = arg_or(args, kw, 12, "roll_forward", Value::None);
    if name.as_str().is_some() {
        name = name_tuple(vm, &name)?;
    }
    py::set(vm, this, "name", name.clone())?;
    if py::is_none(&variant) || variant.as_str().is_some() {
        variant = vm.types.new_list(vec![variant]);
    }
    let n0 = ops::getitem(vm, &name, Value::Int(0))?;
    let screens = mget(vm, "screens")?;
    let by_name = mget(vm, "screens_by_name")?;
    for v in py::items(vm, &variant)? {
        let k = vm.types.new_tuple(vec![n0.clone(), v.clone()]);
        ops::setitem(vm, &screens, k, this.clone())?;
        if !ops::contains(vm, &by_name, &n0)? {
            ops::setitem(vm, &by_name, n0.clone(), vm.types.new_dict())?;
        }
        let d = ops::getitem(vm, &by_name, n0.clone())?;
        ops::setitem(vm, &d, v, this.clone())?;
    }
    py::set(vm, this, "function", function.clone())?;
    let ast = if super::slast::ast_of(&function).is_some() { function } else { Value::None };
    py::set(vm, this, "ast", ast)?;
    py::set(vm, this, "modal", modal)?;
    py::set(vm, this, "zorder", zorder)?;
    let tag = if py::truthy(vm, &tag)? { tag } else { n0 };
    py::set(vm, this, "tag", tag)?;
    if py::is_none(&predict) {
        predict = py::config(vm, "predict_screens")?;
    }
    py::set(vm, this, "predict", predict)?;
    py::set(vm, this, "parameters", parameters)?;
    py::set(vm, this, "location", location)?;
    py::set(vm, this, "layer", layer)?;
    py::set(vm, this, "sensitive", sensitive)?;
    py::set(vm, this, "roll_forward", roll_forward)?;
    mset(vm, "prepared", Value::Bool(false))?;
    mset(vm, "analyzed", Value::Bool(false))?;
    Ok(Value::None)
}

// ScreenDisplayable ------------------------------------------------------------------------------------

fn phase(vm: &mut Vm, this: &Value) -> PyResult<i64> {
    let p = py::get(vm, this, "phase")?;
    py::int(vm, &p)
}

fn hiding(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let p = phase(vm, this)?;
    Ok(p == OLD || p == HIDE)
}

fn profile_for(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let profile = mget(vm, "profile")?;
    vm.call_method(&profile, "get", vec![name.clone(), Value::None])
}

fn reset_transient(vm: &mut Vm, this: &Value) -> PyResult<()> {
    py::set(vm, this, "child", Value::None)?;
    py::set(vm, this, "children", vm.types.new_list(Vec::new()))?;
    for n in ["transforms", "widgets", "base_widgets", "hidden_widgets", "cache", "use_cache", "miss_cache"] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    py::set(vm, this, "old_widgets", Value::None)?;
    py::set(vm, this, "old_transforms", Value::None)?;
    py::set(vm, this, "old_transfers", Value::Bool(false))?;
    py::set(vm, this, "copied_from", Value::None)?;
    Ok(())
}

fn sd_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sn = py::get(vm, this, "screen_name")?;
    let n0 = ops::getitem(vm, &sn, Value::Int(0))?;
    let s = get_screen_variant(vm, &n0, None)?;
    py::set(vm, this, "screen", s)?;
    reset_transient(vm, this)?;
    py::set(vm, this, "phase", Value::Int(UPDATE))?;
    let p = profile_for(vm, &sn)?;
    py::set(vm, this, "profile", p)?;
    Ok(Value::None)
}

fn sd_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut props: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut pos = args[1..].iter().cloned();
    let mut p = |n: &str, props: &mut Vec<(Rc<str>, Value)>| pos.next().or_else(|| take(props, n));
    let screen = p("screen", &mut props).unwrap_or(Value::None);
    let tag = p("tag", &mut props).unwrap_or(Value::None);
    let layer = p("layer", &mut props).unwrap_or(Value::None);
    let widget_properties = p("widget_properties", &mut props).unwrap_or_else(|| vm.types.new_dict());
    let scope = p("scope", &mut props).unwrap_or_else(|| vm.types.new_dict());
    let transient = p("transient", &mut props).unwrap_or(Value::Bool(false));
    layout_init(vm, this, &props)?;
    let pd = py::kwargs_dict(vm, &props)?;
    py::set(vm, this, "properties", pd)?;
    py::set(vm, this, "screen", screen.clone())?;
    let sn = py::get(vm, &screen, "name")?;
    py::set(vm, this, "screen_name", sn.clone())?;
    let loc = py::get_or_none(vm, &screen, "location")?;
    py::set(vm, this, "_location", loc)?;
    let prof = profile_for(vm, &sn)?;
    py::set(vm, this, "profile", prof)?;
    py::set(vm, this, "tag", tag.clone())?;
    py::set(vm, this, "layer", layer.clone())?;
    let rd = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
    let scope = vm.call(&rd, vec![scope], Vec::new())?;
    py::set(vm, this, "scope", scope.clone())?;
    py::set(vm, this, "child", Value::None)?;
    py::set(vm, this, "widget_properties", widget_properties)?;
    for n in ["widgets", "base_widgets", "cache", "miss_cache", "hidden_widgets"] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    let old = if py::truthy(vm, &tag)? && py::truthy(vm, &layer)? { get_screen(vm, &tag, &layer, true)? } else { Value::None };
    if py::is_none(&old) {
        py::set(vm, this, "transforms", vm.types.new_dict())?;
        py::set(vm, this, "use_cache", vm.types.new_dict())?;
        py::set(vm, this, "old_transfers", Value::Bool(false))?;
    } else {
        let t = py::get(vm, &old, "transforms")?;
        py::set(vm, this, "transforms", t)?;
        let u = py::get(vm, &old, "use_cache")?;
        py::set(vm, this, "use_cache", u)?;
        let on = py::get(vm, &old, "screen_name")?;
        let same = ops::eq(vm, &on, &sn)?;
        py::set(vm, this, "old_transfers", Value::Bool(same))?;
    }
    py::set(vm, this, "old_widgets", Value::None)?;
    py::set(vm, this, "old_transforms", Value::None)?;
    py::set(vm, this, "current_transform_event", Value::None)?;
    py::set(vm, this, "restarting", Value::Bool(false))?;
    py::set(vm, this, "hiding", Value::Bool(false))?;
    py::set(vm, this, "transient", transient)?;
    let modal = py::get(vm, &screen, "modal")?;
    let modal = py_eval(vm, &modal, &scope)?;
    py::set(vm, this, "modal", modal)?;
    let z = py::get(vm, &screen, "zorder")?;
    let z = py_eval(vm, &z, &scope)?;
    py::set(vm, this, "zorder", z)?;
    py::set(vm, this, "phase", Value::Int(PREDICT))?;
    py::set(vm, this, "copied_from", Value::None)?;
    Ok(Value::None)
}

fn layout_init(vm: &mut Vm, this: &Value, props: &[(Rc<str>, Value)]) -> PyResult<()> {
    super::layout::container_init(vm, &[this.clone()], props)?;
    Ok(())
}

fn sd_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sn = py::get(vm, &args[0], "screen_name")?;
    let parts: Vec<String> = py::items(vm, &sn)?.iter().map(|p| py::string(vm, p)).collect::<PyResult<_>>()?;
    Ok(py::s(vm, &parts.join(" ")))
}

fn sd_repr_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = sd_name(vm, args, &[])?;
    let mut rv = py::string(vm, &n)?;
    let screen = py::get(vm, &args[0], "screen")?;
    let p = py::get_or_none(vm, &screen, "parameters")?;
    if !py::is_none(&p) {
        rv.push_str(&ops::str(vm, &p)?);
    }
    Ok(py::s(vm, &rv))
}

fn sd_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![c]))
}

fn with_current<R>(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut Vm) -> PyResult<R>) -> PyResult<R> {
    push_current_screen(vm, this.clone())?;
    let r = f(vm);
    pop_current_screen(vm)?;
    r
}

fn sd_visit_all(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cb = arg_or(args, kw, 1, "callback", Value::None);
    vm.call(&cb, vec![this.clone()], Vec::new())?;
    with_current(vm, this, |vm| {
        let c = py::get_or_none(vm, this, "child")?;
        if !py::is_none(&c) {
            py::method_kw(vm, &c, "visit_all", vec![cb], vec![("seen".into(), Value::None)])?;
        }
        Ok(Value::None)
    })
}

fn sd_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    super::render::redraw(vm, &args[0], 0.);
    vm.call_method(&args[0], "update", Vec::new())?;
    Ok(Value::None)
}

fn sd_set_transform_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = arg_or(args, kw, 1, "event", Value::None);
    with_current(vm, this, |vm| {
        container_method(vm, "set_transform_event", vec![this.clone(), ev.clone()])?;
        py::set(vm, this, "current_transform_event", ev)?;
        Ok(Value::None)
    })
}

fn sd_find_focusable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cb = arg_or(args, kw, 1, "callback", Value::None);
    let fname = arg_or(args, kw, 2, "focus_name", Value::None);
    let hiding = hiding(vm, this)?;
    let modal = py::get_or_none(vm, this, "modal")?;
    if py::truthy(vm, &modal)? && !crysirror_python::builtins::is_callable(vm, &modal) && !hiding {
        let mm = py::import_attr(vm, "renpy.display.focus", "mark_modal")?;
        vm.call(&mm, Vec::new(), Vec::new())?;
    }
    with_current(vm, this, |vm| {
        let c = py::get_or_none(vm, this, "child")?;
        if py::truthy(vm, &c)? && !hiding {
            vm.call_method(&c, "find_focusable", vec![cb, fname])?;
        }
        Ok(Value::None)
    })
}

fn new_sd(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let c = mget(vm, "ScreenDisplayable")?;
    let mut a = Vec::new();
    for n in ["screen", "tag", "layer", "widget_properties", "scope"] {
        a.push(py::get_or_none(vm, this, n)?);
    }
    let props = py::get_or_none(vm, this, "properties")?;
    let k = if py::is_none(&props) { Vec::new() } else { py::str_items(vm, &props)? };
    vm.call(&c, a, k)
}

fn sd_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let rv = new_sd(vm, this)?;
    let t = py::get(vm, this, "transforms")?;
    let t = vm.call_method(&t, "copy", Vec::new())?;
    py::set(vm, &rv, "transforms", t)?;
    let w = py::get(vm, this, "widgets")?;
    let w1 = vm.call_method(&w, "copy", Vec::new())?;
    py::set(vm, &rv, "widgets", w1)?;
    let w2 = vm.call_method(&w, "copy", Vec::new())?;
    py::set(vm, &rv, "base_widgets", w2)?;
    py::set(vm, &rv, "old_transfers", Value::Bool(true))?;
    let c = py::get_or_none(vm, this, "child")?;
    py::set(vm, &rv, "child", c)?;
    Ok(rv)
}

fn sd_handles_event(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = arg_or(args, kw, 1, "event", Value::None);
    let c = py::get_or_none(vm, this, "child")?;
    if py::is_none(&c) {
        let t = py::get_or_none(vm, this, "transient")?;
        if py::truthy(vm, &t)? {
            return Ok(Value::Bool(false));
        }
        vm.call_method(this, "update", Vec::new())?;
    }
    let c = py::get(vm, this, "child")?;
    vm.call_method(&c, "_handles_event", vec![ev])
}

fn copy_on_change(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let screen = py::get_or_none(vm, this, "screen")?;
    let ast = py::get_or_none(vm, &screen, "ast")?;
    if !py::is_none(&ast) {
        let cache = py::get(vm, this, "cache")?;
        let c0 = vm.call_method(&cache, "get", vec![Value::Int(0), vm.types.new_dict()])?;
        vm.call_method(&ast, "copy_on_change", vec![c0])?;
    }
    Ok(())
}

fn sd_hide(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (st, at, kind) = (args[1].clone(), args[2].clone(), args[3].clone());
    if kind.as_str() == Some("cancel") && py::config_bool(vm, "screens_never_cancel_hide")? {
        return Ok(this.clone());
    }
    let hid = if phase(vm, this)? == HIDE {
        this.clone()
    } else {
        let c = py::get_or_none(vm, this, "child")?;
        if !py::is_none(&c) {
            let h = vm.call_method(&c, "_handles_event", vec![kind.clone()])?;
            if !py::truthy(vm, &h)? {
                return Ok(Value::None);
            }
        }
        let us = mget(vm, "updated_screens")?;
        ops::set_discard(vm, &us, this)?;
        vm.call_method(this, "update", Vec::new())?;
        let screen = py::get_or_none(vm, this, "screen")?;
        if py::is_none(&screen) {
            return Ok(Value::None);
        }
        let c = py::get_or_none(vm, this, "child")?;
        if py::is_none(&c) {
            return Ok(Value::None);
        }
        let h = vm.call_method(&c, "_handles_event", vec![kind.clone()])?;
        if !py::truthy(vm, &h)? {
            return Ok(Value::None);
        }
        copy_on_change(vm, this)?;
        let hid = vm.call_method(this, "copy", Vec::new())?;
        with_current(vm, this, |vm| {
            let cc = py::get_items(vm, &c, "children")?;
            for i in cc {
                vm.call_method(&i, "set_transform_event", vec![kind.clone()])?;
            }
            Ok(())
        })?;
        let modal = py::get_or_none(vm, this, "modal")?;
        if py::truthy(vm, &modal)? {
            super::render::redraw(vm, this, 0.);
        }
        hid
    };
    py::set(vm, &hid, "phase", Value::Int(HIDE))?;
    py::set(vm, &hid, "copied_from", this.clone())?;
    let old_child = py::get_or_none(vm, &hid, "child")?;
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    if !mb.obj().is_some_and(|c| ops::isinstance(vm, &old_child, c)) {
        return Ok(Value::None);
    }
    let fixed = new_screen_root(vm, this)?;
    py::set(vm, &hid, "child", fixed.clone())?;
    py::set(vm, &hid, "children", vm.types.new_list(vec![fixed.clone()]))?;
    let mut rv = Value::None;
    for d in py::get_items(vm, &old_child, "children")? {
        let c = vm.call_method(&d, "_hide", vec![st.clone(), at.clone(), kind.clone()])?;
        if !py::is_none(&c) {
            super::render::redraw(vm, &c, 0.);
            vm.call_method(&fixed, "add", vec![c])?;
            rv = hid.clone();
        }
    }
    super::render::redraw(vm, &hid, 0.);
    Ok(rv)
}

/// `ui.detached(); ui.default_fixed(focus="_screen_..."); ui.close()`.
fn new_screen_root(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let sn = py::get(vm, this, "screen_name")?;
    let parts: Vec<String> = py::items(vm, &sn)?.iter().map(|p| py::string(vm, p)).collect::<PyResult<_>>()?;
    let focus = py::s(vm, &format!("_screen_{}", parts.join("_")));
    let det = py::import_attr(vm, "renpy.ui", "detached")?;
    vm.call(&det, Vec::new(), Vec::new())?;
    let df = py::import_attr(vm, "renpy.ui", "default_fixed")?;
    let f = vm.call(&df, Vec::new(), vec![("focus".into(), focus)])?;
    let close = py::import_attr(vm, "renpy.ui", "close")?;
    vm.call(&close, Vec::new(), Vec::new())?;
    Ok(f)
}

fn sd_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let screen = py::get_or_none(vm, this, "screen")?;
    let child = py::get_or_none(vm, this, "child")?;
    if py::is_none(&screen) || py::is_none(&child) || !py::config_bool(vm, "transition_screens")? {
        return Ok(this.clone());
    }
    copy_on_change(vm, this)?;
    let rv = vm.call_method(this, "copy", Vec::new())?;
    py::set(vm, &rv, "phase", Value::Int(OLD))?;
    let c = vm.call_method(&child, "_in_current_store", Vec::new())?;
    py::set(vm, &rv, "child", c)?;
    py::set(vm, &rv, "copied_from", this.clone())?;
    Ok(rv)
}

fn sd_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let us = mget(vm, "updated_screens")?;
    if ops::set_contains(vm, &us, this)? {
        return Ok(Value::None);
    }
    ops::set_add(vm, &us, this.clone())?;
    let screen = py::get_or_none(vm, this, "screen")?;
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    if py::is_none(&screen) {
        let c = vm.call(&mb, Vec::new(), vec![("layout".into(), py::s(vm, "fixed"))])?;
        py::set(vm, this, "child", c)?;
        return Ok(vm.types.new_dict());
    }
    let restarting = py::get_or_none(vm, this, "restarting")?;
    let p = phase(vm, this)?;
    if py::truthy(vm, &restarting)? || p == HIDE || p == OLD {
        let c = py::get_or_none(vm, this, "child")?;
        if !py::truthy(vm, &c)? {
            let c = vm.call(&mb, Vec::new(), vec![("layout".into(), py::s(vm, "fixed"))])?;
            py::set(vm, this, "child", c)?;
        }
        return py::get(vm, this, "widgets");
    }
    let w = py::get(vm, this, "widgets")?;
    py::set(vm, this, "old_widgets", w)?;
    let t = py::get(vm, this, "transforms")?;
    py::set(vm, this, "old_transforms", t)?;
    for n in ["widgets", "base_widgets", "transforms"] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    push_current_screen(vm, this.clone())?;
    let old_ui_screen = py::import_attr(vm, "renpy.ui", "screen")?;
    let ui = py::import(vm, "renpy.ui")?;
    py::set(vm, &ui, "screen", this.clone())?;
    let scope = py::get(vm, this, "scope")?;
    let r = (|| -> PyResult<()> {
        let child = new_screen_root_open(vm, this)?;
        py::set(vm, this, "child", child.clone())?;
        py::set(vm, this, "children", vm.types.new_list(vec![child]))?;
        py::dict_set(vm, &scope, "_scope", scope.clone())?;
        py::dict_set(vm, &scope, "_name", Value::Int(0))?;
        py::dict_set(vm, &scope, "_debug", Value::Bool(false))?;
        let f = py::get(vm, &screen, "function")?;
        let kw = py::str_items(vm, &scope)?;
        vm.call(&f, Vec::new(), kw)?;
        let close = py::import_attr(vm, "renpy.ui", "close")?;
        vm.call(&close, Vec::new(), Vec::new())?;
        Ok(())
    })();
    vm.call_method(&scope, "pop", vec![py::s(vm, "_scope"), Value::None])?;
    py::set(vm, &ui, "screen", old_ui_screen)?;
    pop_current_screen(vm)?;
    r?;
    py::set(vm, this, "old_widgets", Value::None)?;
    py::set(vm, this, "old_transforms", Value::None)?;
    py::set(vm, this, "old_transfers", Value::Bool(true))?;
    let mc = py::get(vm, this, "miss_cache")?;
    if py::truthy(vm, &mc)? {
        vm.call_method(&mc, "clear", Vec::new())?;
    }
    let cte = py::get_or_none(vm, this, "current_transform_event")?;
    if py::truthy(vm, &cte)? {
        with_current(vm, this, |vm| {
            let c = py::get(vm, this, "child")?;
            for i in py::get_items(vm, &c, "children")? {
                vm.call_method(&i, "set_transform_event", vec![cte.clone()])?;
            }
            Ok(())
        })?;
        py::set(vm, this, "current_transform_event", Value::None)?;
    }
    if phase(vm, this)? == SHOW {
        py::set(vm, this, "phase", Value::Int(UPDATE))?;
    }
    py::get(vm, this, "widgets")
}

/// `ui.detached(); ui.default_fixed(...)`, leaving the fixed open for the
/// screen function.
fn new_screen_root_open(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let sn = py::get(vm, this, "screen_name")?;
    let parts: Vec<String> = py::items(vm, &sn)?.iter().map(|p| py::string(vm, p)).collect::<PyResult<_>>()?;
    let focus = py::s(vm, &format!("_screen_{}", parts.join("_")));
    let det = py::import_attr(vm, "renpy.ui", "detached")?;
    vm.call(&det, Vec::new(), Vec::new())?;
    let df = py::import_attr(vm, "renpy.ui", "default_fixed")?;
    vm.call(&df, Vec::new(), vec![("focus".into(), focus)])
}

fn sensitive(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let screen = py::get_or_none(vm, this, "screen")?;
    if py::is_none(&screen) {
        return Ok(false);
    }
    let s = py::get(vm, &screen, "sensitive")?;
    let scope = py::get(vm, this, "scope")?;
    let v = py_eval(vm, &s, &scope)?;
    py::truthy(vm, &v)
}

fn sd_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = super::layout::rargs(vm, args)?;
    let c = py::get_or_none(vm, this, "child")?;
    if !py::truthy(vm, &c)? {
        vm.call_method(this, "update", Vec::new())?;
    }
    let child = with_current(vm, this, |vm| {
        let c = py::get(vm, this, "child")?;
        super::render::render(vm, &c, w, h, st, at)
    })?;
    let rv = super::layout::new(vm, w, h);
    py::set(vm, &rv, "focus_screen", this.clone())?;
    let hiding = hiding(vm, this)?;
    let sens = sensitive(vm, this)?;
    let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    py::method_kw(vm, &rv, "blit", vec![child, pos], vec![("focus".into(), Value::Bool(sens && !hiding)), ("main".into(), Value::Bool(!hiding))])?;
    let modal = py::get_or_none(vm, this, "modal")?;
    let m = if py::truthy(vm, &modal)? && !hiding { modal } else { Value::Bool(false) };
    py::set(vm, &rv, "modal", m)?;
    Ok(rv)
}

fn sd_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = py::get_or_none(vm, this, "child")?;
    if !py::truthy(vm, &c)? {
        vm.call_method(this, "update", Vec::new())?;
    }
    let c = py::get(vm, this, "child")?;
    vm.call_method(&c, "get_placement", Vec::new())
}

fn sd_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if hiding(vm, this)? {
        return Ok(Value::None);
    }
    let screen = py::get_or_none(vm, this, "screen")?;
    if !py::truthy(vm, &screen)? {
        return Ok(Value::None);
    }
    let mut ev = args[1].clone();
    let (x, y, st) = (args[2].clone(), args[3].clone(), args[4].clone());
    if !sensitive(vm, this)? {
        let interface = py::import_attr(vm, "renpy.display", "interface")?;
        ev = py::get(vm, &interface, "time_event")?;
    }
    let rv = with_current(vm, this, |vm| {
        let c = py::get(vm, this, "child")?;
        vm.call_method(&c, "event", vec![ev.clone(), x.clone(), y.clone(), st])
    })?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let modal = py::get_or_none(vm, this, "modal")?;
    let xf = py::f64_of(vm, &x)?;
    let yf = py::f64_of(vm, &y)?;
    if super::focus::check_modal(vm, &modal, &ev, xf, yf, None, None)? {
        let il = py::import_attr(vm, "renpy.display.layout", "IgnoreLayers")?;
        let e = vm.call(&il, Vec::new(), Vec::new())?;
        return Err(crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()));
    }
    Ok(Value::None)
}

fn sd_get_phase_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = phase(vm, &args[0])?;
    Ok(py::s(vm, PHASE_NAMES.get(p as usize).copied().unwrap_or("UNKNOWN")))
}

fn sd_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if hiding(vm, this)? {
        return Ok(py::s(vm, ""));
    }
    let common = vm.call_method(this, "_tts_common", Vec::new())?;
    let modal = py::get_or_none(vm, this, "modal")?;
    if py::truthy(vm, &modal)? {
        if let Ok(done) = py::import_attr(vm, "renpy.display.tts", "TTSDone") {
            return vm.call(&done, vec![common], Vec::new());
        }
    }
    Ok(common)
}

// Current screen ---------------------------------------------------------------------------------------

pub fn push_current_screen(vm: &mut Vm, screen: Value) -> PyResult<()> {
    let stack = mget(vm, "current_screen_stack")?;
    let cur = mget(vm, "_current_screen")?;
    ops::list_append(vm, &stack, cur)?;
    mset(vm, "_current_screen", screen)
}

pub fn pop_current_screen(vm: &mut Vm) -> PyResult<()> {
    let stack = mget(vm, "current_screen_stack")?;
    let v = vm.call_method(&stack, "pop", Vec::new())?;
    mset(vm, "_current_screen", v)
}

fn f_push_current_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    push_current_screen(vm, args[0].clone())?;
    Ok(Value::None)
}

fn f_pop_current_screen(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    pop_current_screen(vm)?;
    Ok(Value::None)
}

pub fn current_screen(vm: &mut Vm) -> PyResult<Value> {
    let cur = mget(vm, "_current_screen")?;
    if py::truthy(vm, &cur)? {
        let cf = py::get_or_none(vm, &cur, "copied_from")?;
        if py::truthy(vm, &cf)? {
            return Ok(cf);
        }
    }
    Ok(cur)
}

fn f_current_screen(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    current_screen(vm)
}

// Registry ---------------------------------------------------------------------------------------------

pub fn get_screen_variant(vm: &mut Vm, name: &Value, candidates: Option<Value>) -> PyResult<Value> {
    let candidates = match candidates {
        Some(c) if !py::is_none(&c) => c,
        _ => py::config(vm, "variants")?,
    };
    let screens = mget(vm, "screens")?;
    for i in py::items(vm, &candidates)? {
        let k = vm.types.new_tuple(vec![name.clone(), i]);
        let rv = vm.call_method(&screens, "get", vec![k, Value::None])?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    Ok(Value::None)
}

fn f_get_screen_variant(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = arg(args, kw, 1, "candidates").cloned();
    get_screen_variant(vm, &args[0], c)
}

fn f_get_all_screen_variants(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = first(vm, &args[0])?;
    let by_name = mget(vm, "screens_by_name")?;
    if !ops::contains(vm, &by_name, &n)? {
        return Ok(vm.types.new_list(Vec::new()));
    }
    let d = ops::getitem(vm, &by_name, n)?;
    let items = vm.call_method(&d, "items", Vec::new())?;
    let l = ops::collect(vm, &items)?;
    Ok(vm.types.new_list(l))
}

fn f_sort_screens(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let screens = mget(vm, "screens")?;
    let at_sort = mget(vm, "screens_at_sort")?;
    if ops::eq(vm, &at_sort, &screens)? {
        return mget(vm, "sorted_screens");
    }
    let keys = ops::collect(vm, &screens)?;
    let mut names: Vec<String> = Vec::new();
    for k in &keys {
        let n = ops::getitem(vm, k, Value::Int(0))?;
        let n = py::string(vm, &n)?;
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let mut depends: Vec<(String, Vec<String>)> = Vec::new();
    let mut reverse: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for k in &keys {
        let n = ops::getitem(vm, k, Value::Int(0))?;
        let name = py::string(vm, &n)?;
        if !depends.iter().any(|(d, _)| *d == name) {
            depends.push((name.clone(), Vec::new()));
        }
        let v = ops::getitem(vm, &screens, k.clone())?;
        let ast = py::get_or_none(vm, &v, "ast")?;
        let Some(a) = super::slast::ast_of(&ast) else { continue };
        let mut used = Vec::new();
        collect_uses(&a.screen.block, &mut used);
        for u in used {
            if !names.contains(&u) {
                continue;
            }
            let e = depends.iter_mut().find(|(d, _)| *d == name).unwrap();
            if !e.1.contains(&u) {
                e.1.push(u.clone());
            }
            let r = reverse.entry(u).or_default();
            if !r.contains(&name) {
                r.push(name.clone());
            }
        }
    }
    let mut rv = Vec::new();
    let mut work: Vec<String> = depends.iter().filter(|(_, d)| d.is_empty()).map(|(n, _)| n.clone()).collect();
    while let Some(name) = work.pop() {
        rv.push(name.clone());
        for i in reverse.remove(&name).unwrap_or_default() {
            let d = depends.iter_mut().find(|(n, _)| *n == i).unwrap();
            d.1.retain(|x| *x != name);
            if d.1.is_empty() {
                work.push(i);
            }
        }
    }
    let cycle: Vec<Value> = reverse.keys().map(|k| vm.types.new_str(k)).collect();
    mset(vm, "use_cycle", vm.types.new_list(cycle))?;
    let sorted: Vec<Value> = rv.iter().map(|k| vm.types.new_str(k)).collect();
    let sorted = vm.types.new_list(sorted);
    mset(vm, "sorted_screens", sorted.clone())?;
    let dict_t = Value::Obj(vm.types.dict.clone());
    let snap = vm.call(&dict_t, vec![screens], Vec::new())?;
    mset(vm, "screens_at_sort", snap)?;
    Ok(sorted)
}

fn collect_uses(b: &crate::sl2::Block, out: &mut Vec<String>) {
    use crate::sl2::SlNode;
    for n in &b.children {
        match n {
            SlNode::Displayable(d) => collect_uses(&d.block, out),
            SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => entries.iter().for_each(|(_, b)| collect_uses(b, out)),
            SlNode::For { block, .. } => collect_uses(block, out),
            SlNode::Use { target, expression, block, .. } => {
                if !*expression {
                    out.push(target.clone());
                }
                if let Some(b) = block {
                    collect_uses(b, out);
                }
            }
            SlNode::CustomUse { target, block, .. } => {
                out.push(target.clone());
                collect_uses(block, out);
            }
            _ => {}
        }
    }
}

fn sorted_variants(vm: &mut Vm) -> PyResult<Vec<Value>> {
    let f = mget(vm, "sort_screens")?;
    let names = vm.call(&f, Vec::new(), Vec::new())?;
    let by_name = mget(vm, "screens_by_name")?;
    let mut rv = Vec::new();
    for n in py::items(vm, &names)? {
        let d = ops::getitem(vm, &by_name, n)?;
        let vals = vm.call_method(&d, "values", Vec::new())?;
        rv.extend(ops::collect(vm, &vals)?);
    }
    Ok(rv)
}

fn f_sorted_variants(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let v = sorted_variants(vm)?;
    Ok(vm.types.new_list(v))
}

fn f_analyze_screens(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    mset(vm, "analyzed", Value::Bool(true))?;
    Ok(Value::None)
}

/// `prepare_screens`: there is no constant analysis to run; ATL blocks are
/// compiled on first use. A use cycle is an error in developer mode.
fn f_prepare_screens(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = mget(vm, "prepared")?;
    if py::truthy(vm, &p)? {
        return Ok(Value::None);
    }
    let pc = mget(vm, "predict_cache")?;
    vm.call_method(&pc, "clear", Vec::new())?;
    sorted_variants(vm)?;
    mset(vm, "analyzed", Value::Bool(true))?;
    mset(vm, "prepared", Value::Bool(true))?;
    let cycle = mget(vm, "use_cycle")?;
    if py::config_bool(vm, "developer")? && py::truthy(vm, &cycle)? {
        let names: Vec<String> = py::items(vm, &cycle)?.iter().map(|c| py::string(vm, c)).collect::<PyResult<_>>()?;
        return Err(py::error(vm, format!("The following screens use each other in a loop: {}. This is not allowed.", names.join(", "))));
    }
    Ok(Value::None)
}

fn f_define_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = mget(vm, "Screen")?;
    vm.call(&c, args.to_vec(), kw.to_vec())?;
    Ok(Value::None)
}

pub fn get_screen_layer(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let n = if name.as_str().is_some() { name.clone() } else { ops::getitem(vm, name, Value::Int(0))? };
    let s = get_screen_variant(vm, &n, None)?;
    if py::is_none(&s) {
        return Ok(py::s(vm, "screens"));
    }
    py::get(vm, &s, "layer")
}

fn f_get_screen_layer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_screen_layer(vm, &args[0])
}

fn scene_lists(vm: &mut Vm) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.display.scenelists", "scene_lists")?;
    vm.call(&f, Vec::new(), Vec::new())
}

pub fn get_screen(vm: &mut Vm, name: &Value, layer: &Value, tag_only: bool) -> PyResult<Value> {
    let layer = if py::is_none(layer) { get_screen_layer(vm, name)? } else { layer.clone() };
    let names = if name.as_str().is_some() { vec![name.clone()] } else { py::items(vm, name)? };
    let sl = scene_lists(vm)?;
    for tag in &names {
        let sd = vm.call_method(&sl, "get_displayable_by_tag", vec![layer.clone(), tag.clone()])?;
        if !py::is_none(&sd) {
            return Ok(sd);
        }
    }
    if !tag_only {
        for tag in &names {
            let t = vm.types.new_tuple(vec![tag.clone()]);
            let sd = vm.call_method(&sl, "get_displayable_by_name", vec![layer.clone(), t])?;
            if !py::is_none(&sd) {
                return Ok(sd);
            }
        }
    }
    Ok(Value::None)
}

fn f_get_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let tag_only = match arg(args, kw, 2, "tag_only") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    get_screen(vm, &args[0], &layer, tag_only)
}

fn screen_for_variable(vm: &mut Vm, screen: &Value, layer: &Value) -> PyResult<Value> {
    if py::is_none(screen) {
        let s = current_screen(vm)?;
        if py::is_none(&s) {
            return Err(vm.exc_msg("ValueError", "There is no current screen."));
        }
        return Ok(s);
    }
    let s = get_screen(vm, screen, layer, false)?;
    if py::is_none(&s) {
        let n = ops::str(vm, screen)?;
        return Err(vm.exc_msg("ValueError", &format!("Screen {n} is not showing.")));
    }
    Ok(s)
}

fn f_get_screen_variable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = args[0].clone();
    let screen = arg_or(args, kw, 1, "screen", Value::None);
    let layer = arg_or(args, kw, 2, "layer", Value::None);
    let s = screen_for_variable(vm, &screen, &layer)?;
    let scope = py::get(vm, &s, "scope")?;
    if !ops::contains(vm, &scope, &name)? {
        let sn = py::get(vm, &s, "name")?;
        let sn = ops::str(vm, &sn)?;
        let n = ops::str(vm, &name)?;
        return Err(vm.exc_msg("NameError", &format!("Screen {sn} does not have a variable named {n}.")));
    }
    ops::getitem(vm, &scope, name)
}

fn f_set_screen_variable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let screen = arg_or(args, kw, 2, "screen", Value::None);
    let layer = arg_or(args, kw, 3, "layer", Value::None);
    let s = screen_for_variable(vm, &screen, &layer)?;
    let scope = py::get(vm, &s, "scope")?;
    ops::setitem(vm, &scope, args[0].clone(), args[1].clone())?;
    Ok(Value::None)
}

fn f_has_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = name_tuple(vm, &args[0])?;
    if ops::len(vm, &n)? == 0 {
        return Ok(Value::Bool(false));
    }
    let n0 = ops::getitem(vm, &n, Value::Int(0))?;
    let s = get_screen_variant(vm, &n0, None)?;
    Ok(Value::Bool(py::truthy(vm, &s)?))
}

fn known(vm: &mut Vm, name: &Value) -> PyResult<(Value, Value)> {
    let n = name_tuple(vm, name)?;
    let n0 = ops::getitem(vm, &n, Value::Int(0))?;
    let s = get_screen_variant(vm, &n0, None)?;
    if py::is_none(&s) {
        let t = py::string(vm, &n0)?;
        return Err(py::error(vm, format!("Screen {t} is not known.\n")));
    }
    Ok((n, s))
}

fn f_get_screen_roll_forward(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, s) = known(vm, &args[0])?;
    py::get(vm, &s, "roll_forward")
}

/// Splits `_layer`, `_tag`, `_widget_properties`, `_transient`, `_zorder`.
struct ShowArgs {
    layer: Value,
    tag: Value,
    widget_properties: Value,
    transient: Value,
    zorder: Value,
    kwargs: Vec<(Rc<str>, Value)>,
}

fn show_args(vm: &mut Vm, kw: Kw) -> ShowArgs {
    let mut kwargs: Vec<(Rc<str>, Value)> = kw.to_vec();
    ShowArgs {
        layer: take(&mut kwargs, "_layer").unwrap_or(Value::None),
        tag: take(&mut kwargs, "_tag").unwrap_or(Value::None),
        widget_properties: take(&mut kwargs, "_widget_properties").unwrap_or_else(|| vm.types.new_dict()),
        transient: take(&mut kwargs, "_transient").unwrap_or(Value::Bool(false)),
        zorder: take(&mut kwargs, "_zorder").unwrap_or(Value::None),
        kwargs,
    }
}

fn make_scope(vm: &mut Vm, screen: &Value, args: &[Value], kwargs: &Value) -> PyResult<Value> {
    let scope = vm.types.new_dict();
    let params = py::get_or_none(vm, screen, "parameters")?;
    if py::truthy(vm, &params)? {
        py::dict_set(vm, &scope, "_kwargs", kwargs.clone())?;
        let t = vm.types.new_tuple(args.to_vec());
        py::dict_set(vm, &scope, "_args", t)?;
    } else {
        vm.call_method(&scope, "update", vec![kwargs.clone()])?;
    }
    Ok(scope)
}

pub fn show_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let sa = show_args(vm, kw);
    let (name, screen) = known(vm, &args[0])?;
    let layer = if py::is_none(&sa.layer) { get_screen_layer(vm, &name)? } else { sa.layer };
    let tag = if py::is_none(&sa.tag) { py::get(vm, &screen, "tag")? } else { sa.tag };
    let rest = &args[1..];
    let kwargs = py::kwargs_dict(vm, &sa.kwargs)?;
    let scope = make_scope(vm, &screen, rest, &kwargs)?;
    let c = mget(vm, "ScreenDisplayable")?;
    let d = vm.call(&c, vec![screen.clone(), tag.clone(), layer.clone(), sa.widget_properties, scope], vec![("transient".into(), sa.transient.clone())])?;
    let zorder = if py::is_none(&sa.zorder) { py::get(vm, &d, "zorder")? } else { sa.zorder };
    let old = get_screen(vm, &tag, &layer, true)?;
    let rest_t = vm.types.new_tuple(rest.to_vec());
    let old_cache = if py::is_none(&old) { Value::None } else { py::get_or_none(vm, &old, "cache")? };
    if !py::is_none(&old) && py::truthy(vm, &old_cache)? {
        py::set(vm, &d, "cache", old_cache)?;
        let mc = cache_get(vm, &screen, &rest_t, &kwargs)?;
        py::set(vm, &d, "miss_cache", mc)?;
        py::set(vm, &d, "phase", Value::Int(UPDATE))?;
    } else {
        let c = cache_get(vm, &screen, &rest_t, &kwargs)?;
        py::set(vm, &d, "cache", c)?;
        py::set(vm, &d, "phase", Value::Int(SHOW))?;
    }
    let sls = scene_lists(vm)?;
    py::method_kw(vm, &sls, "add", vec![layer.clone(), d, tag], vec![
        ("zorder".into(), zorder),
        ("transient".into(), sa.transient.clone()),
        ("keep_st".into(), Value::Bool(true)),
        ("name".into(), name.clone()),
    ])?;
    if !py::truthy(vm, &sa.transient)? {
        let shown = py::get(vm, &sls, "shown")?;
        vm.call_method(&shown, "predict_show", vec![layer, name, Value::Bool(true)])?;
    }
    Ok(Value::None)
}

fn f_predict_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let sa = show_args(vm, kw);
    let name = name_tuple(vm, &args[0])?;
    let n0 = ops::getitem(vm, &name, Value::Int(0))?;
    let screen = get_screen_variant(vm, &n0, None)?;
    if py::is_none(&screen) {
        return Ok(Value::None);
    }
    let p = py::get_or_none(vm, &screen, "predict")?;
    if !py::truthy(vm, &p)? {
        return Ok(Value::None);
    }
    let rest = &args[1..];
    let kwargs = py::kwargs_dict(vm, &sa.kwargs)?;
    let scope = make_scope(vm, &screen, rest, &kwargs)?;
    py::dict_set(vm, &scope, "_scope", scope.clone())?;
    let rest_t = vm.types.new_tuple(rest.to_vec());
    let r = (|| -> PyResult<()> {
        let c = mget(vm, "ScreenDisplayable")?;
        let d = vm.call(&c, vec![screen.clone(), Value::None, Value::None, sa.widget_properties.clone(), scope.clone()], Vec::new())?;
        let cache = cache_get(vm, &screen, &rest_t, &kwargs)?;
        py::set(vm, &d, "cache", cache)?;
        vm.call_method(&d, "update", Vec::new())?;
        let cp = mget(vm, "ScreenCache")?;
        let c = py::get(vm, &d, "cache")?;
        vm.call(&cp, vec![screen.clone(), rest_t.clone(), kwargs.clone(), c], Vec::new())?;
        let pd = py::import_attr(vm, "renpy.display.predict", "displayable")?;
        vm.call(&pd, vec![d], Vec::new())?;
        Ok(())
    })();
    // Prediction failures are only reported with config.debug_prediction.
    if let Err(e) = r {
        if matches!(e, crysirror_python::PyErr::Suspend(..)) {
            return Err(e);
        }
    }
    vm.call_method(&scope, "pop", vec![py::s(vm, "_scope"), Value::None])?;
    let reset = py::import_attr(vm, "renpy.ui", "reset")?;
    vm.call(&reset, Vec::new(), Vec::new())?;
    Ok(Value::None)
}

pub fn hide_screen(vm: &mut Vm, tag: &Value, layer: &Value, immediately: bool) -> PyResult<()> {
    let layer = if py::is_none(layer) {
        let t = vm.types.new_tuple(vec![tag.clone()]);
        get_screen_layer(vm, &t)?
    } else {
        layer.clone()
    };
    let screen = get_screen(vm, tag, &layer, false)?;
    let sls = scene_lists(vm)?;
    if !py::is_none(&screen) {
        let st = py::get(vm, &screen, "tag")?;
        let prefix = if immediately { Value::None } else { py::s(vm, "hide") };
        py::method_kw(vm, &sls, "remove", vec![layer.clone(), st], vec![("prefix".into(), prefix)])?;
        let shown = py::get(vm, &sls, "shown")?;
        let sn = py::get(vm, &screen, "screen_name")?;
        vm.call_method(&shown, "predict_hide", vec![layer, sn])?;
    }
    Ok(())
}

fn f_hide_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let imm = match arg(args, kw, 2, "immediately") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    hide_screen(vm, &args[0], &layer, imm)?;
    Ok(Value::None)
}

fn f_use_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kwargs: Vec<(Rc<str>, Value)> = kw.to_vec();
    let name_arg = take(&mut kwargs, "_name").unwrap_or_else(|| vm.types.new_tuple(Vec::new()));
    let scope_arg = take(&mut kwargs, "_scope").unwrap_or_else(|| vm.types.new_dict());
    let name = name_tuple(vm, &args[0])?;
    let n0 = ops::getitem(vm, &name, Value::Int(0))?;
    let screen = get_screen_variant(vm, &n0, None)?;
    if py::is_none(&screen) {
        let r = ops::repr(vm, &name)?;
        return Err(py::error(vm, format!("Screen {r} is not known.")));
    }
    let cur = mget(vm, "_current_screen")?;
    let old_transfers = py::get_or_none(vm, &cur, "old_transfers")?;
    py::set(vm, &cur, "old_transfers", Value::Bool(true))?;
    let params = py::get_or_none(vm, &screen, "parameters")?;
    let scope = if py::truthy(vm, &params)? {
        let s = vm.types.new_dict();
        let k = py::kwargs_dict(vm, &kwargs)?;
        py::dict_set(vm, &s, "_kwargs", k)?;
        let t = vm.types.new_tuple(args[1..].to_vec());
        py::dict_set(vm, &s, "_args", t)?;
        s
    } else {
        let s = vm.call_method(&scope_arg, "copy", Vec::new())?;
        for (k, v) in &kwargs {
            py::dict_set(vm, &s, k, v.clone())?;
        }
        s
    };
    py::dict_set(vm, &scope, "_scope", scope.clone())?;
    let nm = vm.types.new_tuple(vec![name_arg, name]);
    py::dict_set(vm, &scope, "_name", nm)?;
    let f = py::get(vm, &screen, "function")?;
    let kw = py::str_items(vm, &scope)?;
    let r = vm.call(&f, Vec::new(), kw);
    vm.call_method(&scope, "pop", vec![py::s(vm, "_scope"), Value::None])?;
    r?;
    py::set(vm, &cur, "old_transfers", old_transfers)?;
    Ok(Value::None)
}

fn f_get_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut screen = arg_or(args, kw, 0, "screen", Value::None);
    let id = arg_or(args, kw, 1, "id", Value::None);
    let mut layer = arg_or(args, kw, 2, "layer", Value::None);
    let base = match arg(args, kw, 3, "base") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let sdc = mget(vm, "ScreenDisplayable")?;
    let is_sd = |vm: &Vm, v: &Value| sdc.obj().is_some_and(|c| ops::isinstance(vm, v, c));
    if is_sd(vm, &screen) {
        screen = py::get(vm, &screen, "screen_name")?;
    }
    let screen = if py::is_none(&screen) {
        current_screen(vm)?
    } else {
        if py::is_none(&layer) {
            layer = get_screen_layer(vm, &screen)?;
        }
        get_screen(vm, &screen, &layer, false)?
    };
    if !is_sd(vm, &screen) {
        return Ok(Value::None);
    }
    let c = py::get_or_none(vm, &screen, "child")?;
    if py::is_none(&c) {
        vm.call_method(&screen, "update", Vec::new())?;
    }
    let w = py::get(vm, &screen, if base { "base_widgets" } else { "widgets" })?;
    vm.call_method(&w, "get", vec![id, Value::None])
}

fn f_get_displayable_properties(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let id = args[0].clone();
    let screen = arg_or(args, kw, 1, "screen", Value::None);
    let mut layer = arg_or(args, kw, 2, "layer", Value::None);
    let s = if py::is_none(&screen) {
        current_screen(vm)?
    } else {
        if py::is_none(&layer) {
            layer = get_screen_layer(vm, &screen)?;
        }
        get_screen(vm, &screen, &layer, false)?
    };
    if py::is_none(&s) {
        return Ok(vm.types.new_dict());
    }
    let wp = py::get(vm, &s, "widget_properties")?;
    let rv = vm.call_method(&wp, "get", vec![id, Value::None])?;
    if py::is_none(&rv) {
        return Ok(vm.types.new_dict());
    }
    Ok(rv)
}

fn f_before_restart(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let interface = py::import_attr(vm, "renpy.display", "interface")?;
    let old = py::get(vm, &interface, "old_scene")?;
    let sdc = mget(vm, "ScreenDisplayable")?;
    for (k, layer) in ops::mapping_items(vm, &old)? {
        if py::is_none(&k) {
            continue;
        }
        for i in py::get_items(vm, &layer, "children")? {
            if sdc.obj().is_some_and(|c| ops::isinstance(vm, &i, c)) {
                py::set(vm, &i, "restarting", Value::Bool(true))?;
            }
        }
    }
    Ok(Value::None)
}

fn f_show_overlay_screens(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let suppress = py::truthy(vm, &args[0])?;
    let mut show = !suppress;
    let os = py::import_attr(vm, "store", "_overlay_screens").unwrap_or(Value::None);
    match os {
        Value::None => {}
        Value::Bool(true) => show = true,
        _ => show = false,
    }
    let screens = py::config(vm, "overlay_screens")?;
    for i in py::items(vm, &screens)? {
        let s = get_screen(vm, &i, &Value::None, false)?;
        if show && py::is_none(&s) {
            show_screen(vm, &[i], &[])?;
        } else if !show && !py::is_none(&s) {
            hide_screen(vm, &i, &Value::None, false)?;
        }
    }
    let always = py::config(vm, "always_shown_screens")?;
    for i in py::items(vm, &always)? {
        let s = get_screen(vm, &i, &Value::None, false)?;
        if py::is_none(&s) {
            show_screen(vm, &[i], &[])?;
        }
    }
    Ok(Value::None)
}

fn f_per_frame(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let pf = py::config(vm, "per_frame_screens")?;
    let us = mget(vm, "updated_screens")?;
    for i in py::items(vm, &pf)? {
        let s = get_screen(vm, &i, &Value::None, false)?;
        if py::is_none(&s) {
            continue;
        }
        ops::set_discard(vm, &us, &s)?;
        super::render::invalidate(vm, &s);
        vm.call_method(&s, "update", Vec::new())?;
    }
    Ok(Value::None)
}

fn f_show_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    show_screen(vm, args, kw)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("get_profile", f_get_profile),
        ("cache_get", f_cache_get),
        ("push_current_screen", f_push_current_screen),
        ("pop_current_screen", f_pop_current_screen),
        ("current_screen", f_current_screen),
        ("get_screen_variant", f_get_screen_variant),
        ("get_all_screen_variants", f_get_all_screen_variants),
        ("sort_screens", f_sort_screens),
        ("sorted_variants", f_sorted_variants),
        ("analyze_screens", f_analyze_screens),
        ("prepare_screens", f_prepare_screens),
        ("define_screen", f_define_screen),
        ("get_screen_layer", f_get_screen_layer),
        ("get_screen", f_get_screen),
        ("get_screen_variable", f_get_screen_variable),
        ("set_screen_variable", f_set_screen_variable),
        ("has_screen", f_has_screen),
        ("get_screen_roll_forward", f_get_screen_roll_forward),
        ("show_screen", f_show_screen),
        ("predict_screen", f_predict_screen),
        ("hide_screen", f_hide_screen),
        ("use_screen", f_use_screen),
        ("get_displayable", f_get_displayable),
        ("get_widget", f_get_displayable),
        ("get_displayable_properties", f_get_displayable_properties),
        ("get_widget_properties", f_get_displayable_properties),
        ("before_restart", f_before_restart),
        ("show_overlay_screens", f_show_overlay_screens),
        ("per_frame", f_per_frame),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for (n, v) in [("PREDICT", PREDICT), ("SHOW", SHOW), ("UPDATE", UPDATE), ("HIDE", HIDE), ("OLD", OLD)] {
        set(vm, n, Value::Int(v));
    }
    let names: Vec<Value> = PHASE_NAMES.iter().map(|n| vm.types.new_str(n)).collect();
    set(vm, "phase_name", vm.types.new_list(names));
    for n in ["profile", "predict_cache", "screens", "screens_by_name", "screens_at_sort"] {
        set(vm, n, vm.types.new_dict());
    }
    set(vm, "updated_screens", vm.types.new_set(Default::default()));
    set(vm, "_current_screen", Value::None);
    set(vm, "current_screen_stack", vm.types.new_list(Vec::new()));
    set(vm, "analyzed", Value::Bool(false));
    set(vm, "prepared", Value::Bool(false));
    set(vm, "sorted_screens", vm.types.new_list(Vec::new()));
    set(vm, "use_cycle", vm.types.new_list(Vec::new()));
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let prof = py::class(vm, MODULE, "ScreenProfile", &[object.clone()], &[("__init__", profile_init)], &[])?;
    set(vm, "ScreenProfile", Value::Obj(prof));
    let obj = Value::Obj(vm.types.object.clone());
    let sc = py::class(vm, MODULE, "ScreenCache", &[obj], &[("__init__", cache_init)], &[])?;
    set(vm, "ScreenCache", Value::Obj(sc.clone()));
    set(vm, "cache_put", Value::Obj(sc));
    let screen = py::class(vm, MODULE, "Screen", &[object], &[("__init__", screen_init)], &[
        ("sensitive", py::s(vm, "True")),
        ("roll_forward", Value::None),
    ])?;
    set(vm, "Screen", Value::Obj(screen));
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let name_p = vm.types.new_builtin("name", sd_name);
    let name_p = py::property(vm, name_p, None)?;
    let strs = |vm: &Vm, l: &[&str]| -> Value {
        let v = l.iter().map(|s| vm.types.new_str(s)).collect();
        vm.types.new_list(v)
    };
    let nosave = strs(vm, &["screen", "child", "children", "transforms", "widgets", "base_widgets", "old_widgets", "hidden_widgets", "old_transforms", "cache", "miss_cache", "profile", "phase", "use_cache", "copied_from"]);
    let noreach = strs(vm, &["screen", "children", "transforms", "widgets", "base_widgets", "old_widgets", "hidden_widgets", "old_transforms", "cache", "miss_cache", "profile", "phase", "use_cache", "copied_from"]);
    let sd = py::class(vm, MODULE, "ScreenDisplayable", &[container], &[
        ("__init__", sd_init),
        ("after_setstate", sd_after_setstate),
        ("_repr_info", sd_repr_info),
        ("visit", sd_visit),
        ("visit_all", sd_visit_all),
        ("per_interact", sd_per_interact),
        ("set_transform_event", sd_set_transform_event),
        ("find_focusable", sd_find_focusable),
        ("copy", sd_copy),
        ("_handles_event", sd_handles_event),
        ("_hide", sd_hide),
        ("_in_current_store", sd_in_current_store),
        ("update", sd_update),
        ("render", sd_render),
        ("get_placement", sd_get_placement),
        ("event", sd_event),
        ("get_phase_name", sd_get_phase_name),
        ("_tts", sd_tts),
    ], &[
        ("nosave", nosave),
        ("noreach", noreach),
        ("restarting", Value::Bool(false)),
        ("hiding", Value::Bool(false)),
        ("transient", Value::Bool(false)),
        ("name", name_p),
    ])?;
    set(vm, "ScreenDisplayable", Value::Obj(sd));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
