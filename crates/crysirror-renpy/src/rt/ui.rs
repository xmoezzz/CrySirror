// SPDX-License-Identifier: MPL-2.0
//! `renpy.ui`, after Ren'Py 8.3 `ui.py` (MIT): the widget stack (Layer,
//! Many, One, Detached, ChildOrFixed), the Wrapper factory used by every
//! `ui.*` function, Action/BarValue bases, choice actions, imagemaps and
//! viewport helpers.
//!
//! Module state (`stack`, `at_stack`, `add_tag`, `imagemap_stack`,
//! `screen`) lives in the module namespace, where scripts and the screen
//! language read it.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.ui";

fn m(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, MODULE)
}

pub fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = m(vm)?;
    py::set(vm, &m, n, v)
}

pub fn stack(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "stack")
}

fn stack_top(vm: &mut Vm) -> PyResult<Value> {
    let s = stack(vm)?;
    if py::is_none(&s) || ops::len(vm, &s)? == 0 {
        return Err(py::error(vm, "Can't add displayable during init phase."));
    }
    ops::getitem(vm, &s, Value::Int(-1))
}

fn stack_push(vm: &mut Vm, v: Value) -> PyResult<()> {
    let s = stack(vm)?;
    vm.call_method(&s, "append", vec![v])?;
    Ok(())
}

fn stack_pop(vm: &mut Vm) -> PyResult<Value> {
    let s = stack(vm)?;
    vm.call_method(&s, "pop", Vec::new())
}

fn new(vm: &mut Vm, cls: &str, args: Vec<Value>) -> PyResult<Value> {
    let c = mget(vm, cls)?;
    vm.call(&c, args, Vec::new())
}

fn context(vm: &mut Vm, index: Option<i64>) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.game", "context")?;
    let a = index.map(|i| vec![Value::Int(i)]).unwrap_or_default();
    vm.call(&c, a, Vec::new())
}

fn scene_lists(vm: &mut Vm, index: Option<i64>) -> PyResult<Value> {
    let c = context(vm, index)?;
    py::get(vm, &c, "scene_lists")
}

// Action / BarValue ------------------------------------------------------------------------------------

fn ret_true(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn ret_false(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn ret_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn not_implemented(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("NotImplementedError", ""))
}

fn barvalue_get_style(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm
        .types
        .new_tuple(vec![vm.types.new_str("bar"), vm.types.new_str("vbar")]))
}

// Addables ---------------------------------------------------------------------------------------------

fn layer_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "name", args[1].clone())?;
    Ok(Value::None)
}

fn layer_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::get(vm, &args[0], "name")?;
    let key = arg_or(args, kw, 2, "key", Value::None);
    let sl = scene_lists(vm, Some(-1))?;
    py::method_kw(
        vm,
        &sl,
        "add",
        vec![name, args[1].clone()],
        vec![("key".into(), key)],
    )?;
    Ok(Value::None)
}

fn layer_close(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    stack_pop(vm)?;
    let d = arg_or(args, kw, 1, "d", Value::None);
    let name = py::get(vm, &args[0], "name")?;
    if ops::truthy(vm, &d)? && !ops::eq(vm, &d, &name)? {
        let n = ops::repr(vm, &name)?;
        let dr = ops::repr(vm, &d)?;
        return Err(py::error(
            vm,
            format!("ui.close closed layer {n}, not the expected {dr}."),
        ));
    }
    Ok(Value::None)
}

fn layer_get_layer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "name")
}

fn addable_get_layer(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    // Upstream returns (does not raise) this exception object.
    let e = vm
        .types
        .new_str("Operation can only be performed on a layer.");
    let b = Value::Obj(vm.builtins_module.clone());
    let exc = py::get(vm, &b, "Exception")?;
    vm.call(&exc, vec![e], Vec::new())
}

fn many_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for (i, n) in ["displayable", "imagemap", "style_prefix"]
        .iter()
        .enumerate()
    {
        py::set(
            vm,
            &args[0],
            n,
            args.get(i + 1).cloned().unwrap_or(Value::None),
        )?;
    }
    Ok(Value::None)
}

fn add_to_displayable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "displayable")?;
    vm.call_method(&d, "add", vec![args[1].clone()])?;
    Ok(Value::None)
}

fn many_close(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    stack_pop(vm)?;
    let im = py::get_or_none(vm, this, "imagemap")?;
    if ops::truthy(vm, &im)? {
        let ims = mget(vm, "imagemap_stack")?;
        let imagemap = vm.call_method(&ims, "pop", Vec::new())?;
        let cache = py::get(vm, &imagemap, "cache")?;
        vm.call_method(&cache, "finish", Vec::new())?;
    }
    let d = arg_or(args, kw, 1, "d", Value::None);
    let disp = py::get(vm, this, "displayable")?;
    if ops::truthy(vm, &d)? && !ops::eq(vm, &d, &disp)? {
        let a = ops::repr(vm, &disp)?;
        let b = ops::repr(vm, &d)?;
        return Err(py::error(
            vm,
            format!("ui.close closed {a}, not the expected {b}."),
        ));
    }
    Ok(Value::None)
}

fn one_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "displayable", args[1].clone())?;
    py::set(
        vm,
        &args[0],
        "style_prefix",
        args.get(2).cloned().unwrap_or(Value::None),
    )?;
    Ok(Value::None)
}

fn one_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    add_to_displayable(vm, args, kw)?;
    stack_pop(vm)?;
    Ok(Value::None)
}

fn one_close(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "displayable")?;
    let r = ops::repr(vm, &d)?;
    Err(py::error(vm, format!("Widget {r} expects a child.")))
}

fn prefix_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "style_prefix",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    Ok(Value::None)
}

fn detached_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "child", args[1].clone())?;
    stack_pop(vm)?;
    Ok(Value::None)
}

fn detached_close(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Detached expects to be given a child."))
}

fn cof_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "queue", vm.types.new_list(Vec::new()))?;
    py::set(
        vm,
        &args[0],
        "style_prefix",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    Ok(Value::None)
}

fn cof_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let q = py::get(vm, &args[0], "queue")?;
    vm.call_method(&q, "append", vec![args[1].clone()])?;
    Ok(Value::None)
}

fn cof_close(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    stack_pop(vm)?;
    let q = py::get_items(vm, &args[0], "queue")?;
    let ia = mget(vm, "implicit_add")?;
    if q.len() == 1 {
        vm.call(&ia, vec![q[0].clone()], Vec::new())?;
    } else {
        let fixed = mget(vm, "fixed")?;
        vm.call(&fixed, Vec::new(), Vec::new())?;
        for i in q {
            vm.call(&ia, vec![i], Vec::new())?;
        }
        f_close(vm, &[], &[])?;
    }
    let d = arg_or(args, kw, 1, "d", Value::None);
    if !py::is_none(&d) {
        let r = ops::repr(vm, &d)?;
        return Err(py::error(vm, format!("Did not expect to close {r}.")));
    }
    Ok(Value::None)
}

fn repr_of(vm: &mut Vm, this: &Value, kind: &str, attr: &str) -> PyResult<Value> {
    let v = py::get_or_none(vm, this, attr)?;
    let r = ops::repr(vm, &v)?;
    Ok(vm.types.new_string(format!("<{kind}: {r}>")))
}

// Module functions --------------------------------------------------------------------------------------

fn f_reset(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let t = vm.types.new_str("transient");
    let l = new(vm, "Layer", vec![t])?;
    mset(vm, "stack", vm.types.new_list(vec![l]))?;
    mset(vm, "at_stack", vm.types.new_list(Vec::new()))?;
    mset(vm, "imagemap_stack", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn f_interact(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ty = arg_or(args, kw, 0, "type", vm.types.new_str("misc"));
    let roll_forward = py::kw(kw, "roll_forward").cloned().unwrap_or(Value::None);
    let rest: Vec<(Rc<str>, Value)> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "type" | "roll_forward"))
        .cloned()
        .collect();
    let s = stack(vm)?;
    if py::is_none(&s) {
        return Err(py::error(vm, "Interaction not allowed during init phase."));
    }
    let config = py::import(vm, "renpy.config")?;
    let sk = py::get_or_none(vm, &config, "skipping")?;
    if sk.as_str() == Some("fast") {
        py::set(vm, &config, "skipping", Value::None)?;
    }
    if ops::len(vm, &s)? != 1 {
        let mut parts = Vec::new();
        for i in py::items(vm, &s)? {
            let t = ops::str(vm, &i)?;
            parts.push(t.to_string());
        }
        return Err(py::error(
            vm,
            format!(
                "ui.interact called with non-empty widget/layer stack. Did you forget a ui.close() somewhere?\nStack was {}",
                parts.join("\n")
            ),
        ));
    }
    let ats = mget(vm, "at_stack")?;
    if ops::truthy(vm, &ats)? {
        return Err(py::error(vm, "ui.interact called with non-empty at stack."));
    }
    let ctx = context(vm, None)?;
    let info = py::get(vm, &ctx, "info")?;
    py::set(vm, &info, "_current_interact_type", ty.clone())?;
    let iface = py::import_attr(vm, "renpy.game", "interface")?;
    let mut k = rest;
    k.push(("roll_forward".into(), roll_forward.clone()));
    let rv = py::method_kw(vm, &iface, "interact", Vec::new(), k)?;
    let ctx = context(vm, None)?;
    let info = py::get(vm, &ctx, "info")?;
    py::set(vm, &info, "_last_interact_type", ty)?;
    let ifr = py::import_attr(vm, "renpy.exports", "in_fixed_rollback")?;
    let fixed = vm.call(&ifr, Vec::new(), Vec::new())?;
    if ops::truthy(vm, &fixed)? && !py::is_none(&roll_forward) {
        return Ok(roll_forward);
    }
    Ok(rv)
}

fn f_tag(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mset(vm, "add_tag", args[0].clone())?;
    Ok(Value::None)
}

fn f_child_or_fixed(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let top = stack_top(vm)?;
    let sp = py::get_or_none(vm, &top, "style_prefix")?;
    let c = new(vm, "ChildOrFixed", vec![sp])?;
    stack_push(vm, c)?;
    Ok(Value::None)
}

fn layer_op(vm: &mut Vm, op: &str, d: Option<Value>) -> PyResult<Value> {
    let top = stack_top(vm)?;
    let layer = vm.call_method(&top, "get_layer", Vec::new())?;
    let sl = scene_lists(vm, Some(-1))?;
    let mut a = vec![layer];
    a.extend(d);
    vm.call_method(&sl, op, a)?;
    Ok(Value::None)
}

fn f_remove(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    layer_op(vm, "remove", Some(args[0].clone()))
}

fn f_remove_above(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    layer_op(vm, "remove_above", Some(args[0].clone()))
}

fn f_clear(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    layer_op(vm, "clear", None)
}

fn f_at(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mget(vm, "at_stack")?;
    vm.call_method(&a, "append", vec![args[0].clone()])?;
    Ok(Value::None)
}

pub fn f_detached(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let top = stack_top(vm)?;
    let sp = py::get_or_none(vm, &top, "style_prefix")?;
    let d = new(vm, "Detached", vec![sp])?;
    stack_push(vm, d.clone())?;
    Ok(d)
}

fn f_layer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = new(vm, "Layer", vec![args[0].clone()])?;
    stack_push(vm, l)?;
    Ok(Value::None)
}

pub fn f_close(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let top = stack_top(vm)?;
    vm.call_method(&top, "close", vec![d])?;
    let s = stack(vm)?;
    if ops::len(vm, &s)? == 0 {
        return Err(py::error(
            vm,
            "ui.close() called when no layer or widget is open.",
        ));
    }
    Ok(Value::None)
}

fn f_reopen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let w = args[0].clone();
    let many = new(vm, "Many", vec![w.clone(), Value::None, Value::None])?;
    stack_push(vm, many)?;
    if ops::truthy(vm, &args.get(1).cloned().unwrap_or(Value::Bool(false)))? {
        let c = py::get(vm, &w, "children")?;
        if let Some(crysirror_python::Payload::List(l)) = c.payload() {
            l.borrow_mut().clear();
        }
    }
    Ok(Value::None)
}

fn f_context_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let top = stack_top(vm)?;
    let many = mget(vm, "Many")?;
    if many.obj().is_some_and(|c| ops::isinstance(vm, &top, c)) {
        let d = py::get(vm, &top, "displayable")?;
        if py::same(&d, &args[0]) {
            return Ok(Value::None);
        }
    }
    let n = args[0].obj().map(|o| o.class_name()).unwrap_or_default();
    Err(py::error(
        vm,
        format!("{n} cannot be used as a context manager."),
    ))
}

fn f_context_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    f_close(vm, &args[..1], &[])
}

pub fn combine_style(vm: &mut Vm, prefix: &Value, suffix: &Value) -> PyResult<Value> {
    let name = if py::is_none(prefix) {
        suffix.clone()
    } else {
        let p = py::string(vm, prefix)?;
        let s = py::string(vm, suffix)?;
        vm.types.new_string(format!("{p}_{s}"))
    };
    let gs = py::import_attr(vm, "renpy.style", "get_style")?;
    vm.call(&gs, vec![name], Vec::new())
}

fn f_combine_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    combine_style(vm, &args[0], &args[1])
}

pub fn prefixed_style(vm: &mut Vm, suffix: &str) -> PyResult<Value> {
    let top = stack_top(vm)?;
    let sp = py::get_or_none(vm, &top, "style_prefix")?;
    let s = vm.types.new_str(suffix);
    combine_style(vm, &sp, &s)
}

fn f_prefixed_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    prefixed_style(vm, &s)
}

// Wrapper ----------------------------------------------------------------------------------------------

fn wrapper_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "name")
}

fn wrapper_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut one = Value::Bool(false);
    let mut many = Value::Bool(false);
    let mut imagemap = Value::Bool(false);
    let mut replaces = Value::Bool(false);
    let mut style = Value::None;
    let mut rest = Vec::new();
    for (k, v) in kw {
        match &**k {
            "one" => one = v.clone(),
            "many" => many = v.clone(),
            "imagemap" => imagemap = v.clone(),
            "replaces" => replaces = v.clone(),
            "style" => style = v.clone(),
            _ => rest.push((k.clone(), v.clone())),
        }
    }
    let many = Value::Bool(ops::truthy(vm, &many)? || ops::truthy(vm, &imagemap)?);
    py::set(vm, this, "name", vm.types.new_str(""))?;
    py::set(
        vm,
        this,
        "function",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    py::set(vm, this, "one", one)?;
    py::set(vm, this, "many", many)?;
    py::set(vm, this, "imagemap", imagemap)?;
    py::set(vm, this, "replaces", replaces)?;
    let kd = py::kwargs_dict(vm, &rest)?;
    py::set(vm, this, "kwargs", kd)?;
    py::set(vm, this, "style", style)?;
    Ok(Value::None)
}

fn wrapper_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = stack(vm)?;
    if py::is_none(&s) || ops::len(vm, &s)? == 0 {
        return Err(py::error(vm, "Can't add displayable during init phase."));
    }
    let mut widget_id = Value::None;
    let mut at_list = Value::None;
    let top = stack_top(vm)?;
    let mut style_prefix = py::get_or_none(vm, &top, "style_prefix")?;
    let mut kwargs: Vec<(Rc<str>, Value)> = Vec::new();
    let mut style_group = None;
    let mut sp = None;
    for (k, v) in kw {
        match &**k {
            "id" => widget_id = v.clone(),
            "at" => at_list = v.clone(),
            "style_group" => style_group = Some(v.clone()),
            "style_prefix" => sp = Some(v.clone()),
            _ => kwargs.push((k.clone(), v.clone())),
        }
    }
    if let Some(g) = style_group {
        style_prefix = g;
    }
    if let Some(p) = sp {
        style_prefix = p;
    }
    let mut at_list: Vec<Value> = if py::is_none(&at_list) {
        Vec::new()
    } else if matches!(
        at_list.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        py::items(vm, &at_list)?
    } else {
        vec![at_list]
    };
    let wk = py::get(vm, this, "kwargs")?;
    let mut keyword = py::str_items(vm, &wk)?;
    for (k, v) in kwargs {
        keyword.retain(|(kk, _)| *kk != k);
        keyword.push((k, v));
    }
    let set_kw = |keyword: &mut Vec<(Rc<str>, Value)>, k: &str, v: Value| {
        keyword.retain(|(kk, _)| &**kk != k);
        keyword.push((k.into(), v));
    };
    let screen = mget(vm, "screen")?;
    let has_screen = ops::truthy(vm, &screen)?;
    let old_transfers = has_screen && {
        let v = py::get_or_none(vm, &screen, "old_transfers")?;
        ops::truthy(vm, &v)?
    };
    let mut do_add = true;
    if has_screen {
        let wp = py::get(vm, &screen, "widget_properties")?;
        if ops::contains(vm, &wp, &widget_id)? {
            let props = ops::getitem(vm, &wp, widget_id.clone())?;
            for (k, v) in py::str_items(vm, &props)? {
                set_kw(&mut keyword, &k, v);
            }
        }
        let hw = py::get(vm, &screen, "hidden_widgets")?;
        if ops::contains(vm, &hw, &widget_id)? {
            do_add = false;
        }
    }
    let old_main = if old_transfers {
        let ow = py::get(vm, &screen, "old_widgets")?;
        let om = if py::is_none(&ow) {
            Value::None
        } else {
            vm.call_method(&ow, "get", vec![widget_id.clone(), Value::None])?
        };
        let rep = {
            let v = py::get(vm, this, "replaces")?;
            ops::truthy(vm, &v)?
        };
        if rep && !py::is_none(&om) {
            set_kw(&mut keyword, "replaces", om.clone());
        }
        om
    } else {
        Value::None
    };
    let mut suffix = Value::None;
    if let Some(pos) = keyword.iter().position(|(k, _)| &**k == "style_suffix") {
        suffix = keyword.remove(pos).1;
    }
    if !ops::truthy(vm, &suffix)? {
        suffix = py::get_or_none(vm, this, "style")?;
    }
    if ops::truthy(vm, &suffix)? && !keyword.iter().any(|(k, _)| &**k == "style") {
        let st = combine_style(vm, &style_prefix, &suffix)?;
        keyword.push(("style".into(), st));
    }
    let f = py::get(vm, this, "function")?;
    let w = vm.call(&f, args[1..].to_vec(), keyword)?;
    let wm = py::get_or_none(vm, &w, "_main")?;
    let main = if ops::truthy(vm, &wm)? { wm } else { w.clone() };
    if !py::is_none(&old_main) {
        let hiding = {
            let v = py::get_or_none(vm, &screen, "hiding")?;
            ops::truthy(vm, &v)?
        };
        if !hiding {
            let rb = py::import_attr(vm, "renpy.display.focus", "replaced_by")?;
            ops::setitem(vm, &rb, Value::Int(old_main.id()), main.clone())?;
        }
    }
    let ats = mget(vm, "at_stack")?;
    while ops::len(vm, &ats)? > 0 {
        at_list.push(vm.call_method(&ats, "pop", Vec::new())?);
    }
    let tcls = py::import_attr(vm, "renpy.display.motion", "Transform")?;
    let tobj = tcls.obj().cloned();
    let mut atw = w.clone();
    for atf in at_list {
        if tobj.as_ref().is_some_and(|c| ops::isinstance(vm, &atf, c)) {
            atw = vm.call(&atf, Vec::new(), vec![("child".into(), atw)])?;
        } else {
            atw = vm.call(&atf, vec![atw], Vec::new())?;
        }
    }
    if do_add {
        let top = stack_top(vm)?;
        let tag = mget(vm, "add_tag")?;
        vm.call_method(&top, "add", vec![atw.clone(), tag])?;
    }
    if {
        let v = py::get(vm, this, "one")?;
        ops::truthy(vm, &v)?
    } {
        let o = new(vm, "One", vec![w.clone(), style_prefix])?;
        stack_push(vm, o)?;
    } else if {
        let v = py::get(vm, this, "many")?;
        ops::truthy(vm, &v)?
    } {
        let im = py::get(vm, this, "imagemap")?;
        let o = new(vm, "Many", vec![w.clone(), im, style_prefix])?;
        stack_push(vm, o)?;
    }
    if has_screen && !py::is_none(&widget_id) {
        let widgets = py::get(vm, &screen, "widgets")?;
        ops::setitem(vm, &widgets, widget_id.clone(), main.clone())?;
        if tobj.as_ref().is_some_and(|c| ops::isinstance(vm, &atw, c)) {
            let tr = py::get(vm, &screen, "transforms")?;
            ops::setitem(vm, &tr, widget_id.clone(), atw.clone())?;
            let oldt = if old_transfers {
                let ot = py::get_or_none(vm, &screen, "old_transforms")?;
                if py::is_none(&ot) {
                    Value::None
                } else {
                    vm.call_method(&ot, "get", vec![widget_id, Value::None])?
                }
            } else {
                Value::None
            };
            vm.call_method(&atw, "take_state", vec![oldt.clone()])?;
            vm.call_method(&atw, "take_execution_state", vec![oldt])?;
        }
    }
    mset(vm, "add_tag", Value::None)?;
    Ok(main)
}

// Wrapped functions --------------------------------------------------------------------------------------

fn dup(vm: &mut Vm, d: Value) -> PyResult<Value> {
    let dupl = py::get_or_none(vm, &d, "_duplicatable")?;
    if ops::truthy(vm, &dupl)? {
        let c = vm.call_method(&d, "_duplicate", vec![Value::None])?;
        vm.call_method(&c, "_unique", Vec::new())?;
        return Ok(c);
    }
    Ok(d)
}

fn f_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = super::easy::displayable(vm, &args[0], &Value::None, true)?;
    let d = dup(vm, d)?;
    if !kw.is_empty() {
        let t = py::import_attr(vm, "renpy.display.motion", "Transform")?;
        let mut k = kw.to_vec();
        k.push(("child".into(), d));
        return vm.call(&t, Vec::new(), k);
    }
    Ok(d)
}

fn f_implicit_add(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn f_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let im = py::import_attr(vm, "renpy.display.im", "image")?;
    let mut k = kw.to_vec();
    k.push(("loose".into(), Value::Bool(true)));
    let d = vm.call(&im, vec![args[0].clone()], k)?;
    dup(vm, d)
}

fn f_sizer(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let mut props = Vec::new();
    let (mut mw, mut mh) = (Value::None, Value::None);
    for (k, v) in kw {
        match &**k {
            "maxwidth" => mw = v.clone(),
            "maxheight" => mh = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    props.push(("xmaximum".into(), mw));
    props.push(("ymaximum".into(), mh));
    let c = py::import_attr(vm, "renpy.display.layout", "Container")?;
    vm.call(&c, Vec::new(), props)
}

fn f_key(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let key = arg_or(args, kw, 0, "key", Value::None);
    let action = arg_or(args, kw, 1, "action", Value::None);
    let sound = arg_or(args, kw, 2, "activate_sound", Value::None);
    let capture = arg_or(args, kw, 3, "capture", Value::Bool(true));
    let mut k: Vec<(Rc<str>, Value)> = Vec::new();
    let keys = if matches!(
        key.payload(),
        Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))
    ) {
        py::items(vm, &key)?
    } else {
        vec![key]
    };
    for kk in keys {
        let s = py::string(vm, &kk)?;
        k.retain(|(n, _)| **n != *s);
        k.push((Rc::from(s.as_str()), action.clone()));
    }
    k.push(("activate_sound".into(), sound));
    k.push(("capture".into(), capture));
    let km = py::import_attr(vm, "renpy.display.behavior", "Keymap")?;
    vm.call(&km, Vec::new(), k)
}

// Choice actions -----------------------------------------------------------------------------------------

fn cab_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "label",
        "value",
        "location",
        "block_all",
        "sensitive",
        "args",
        "kwargs",
    ];
    let mut v = vec![
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(true),
        Value::None,
        Value::None,
    ];
    for (i, a) in args.iter().skip(1).enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    for (k, val) in kw {
        if let Some(i) = names.iter().position(|n| *n == &**k) {
            v[i] = val.clone();
        }
    }
    if py::is_none(&v[3]) {
        v[3] = py::config(vm, "fix_rollback_without_choice").unwrap_or(Value::Bool(false));
    }
    for (i, n) in names.iter().enumerate() {
        py::set(vm, this, n, v[i].clone())?;
    }
    Ok(Value::None)
}

fn export(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", n)?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn cab_get_sensitive(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = {
        let v = py::get_or_none(vm, this, "sensitive")?;
        ops::truthy(vm, &v)?
    };
    let fixed = {
        let v = export(vm, "in_fixed_rollback")?;
        ops::truthy(vm, &v)?
    };
    if s && !fixed {
        return Ok(Value::Bool(true));
    }
    let block_all = {
        let v = py::get_or_none(vm, this, "block_all")?;
        ops::truthy(vm, &v)?
    };
    if block_all {
        return Ok(Value::Bool(false));
    }
    let sel = vm.call_method(this, "get_selected", Vec::new())?;
    Ok(Value::Bool(ops::truthy(vm, &sel)?))
}

fn cab_get_selected(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rf = export(vm, "roll_forward_info")?;
    let fixed = {
        let v = export(vm, "in_fixed_rollback")?;
        ops::truthy(vm, &v)?
    };
    let value = py::get_or_none(vm, &args[0], "value")?;
    Ok(Value::Bool(fixed && ops::eq(vm, &rf, &value)?))
}

fn cab_chosen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let loc = py::get_or_none(vm, &args[0], "location")?;
    if !ops::truthy(vm, &loc)? {
        return Ok(Value::None);
    }
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    py::get_or_none(vm, &p, "_chosen")
}

fn chosen_key(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let loc = py::get_or_none(vm, this, "location")?;
    let label = py::get_or_none(vm, this, "label")?;
    Ok(vm.types.new_tuple(vec![loc, label]))
}

fn cab_get_chosen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "chosen")?;
    if py::is_none(&c) {
        return Ok(Value::Bool(false));
    }
    let k = chosen_key(vm, &args[0])?;
    Ok(Value::Bool(ops::contains(vm, &c, &k)?))
}

fn mark_chosen(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let c = py::get_or_none(vm, this, "chosen")?;
    if !py::is_none(&c) {
        let k = chosen_key(vm, this)?;
        ops::setitem(vm, &c, k, Value::Bool(true))?;
    }
    Ok(())
}

fn choice_return_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mark_chosen(vm, &args[0])?;
    py::get_or_none(vm, &args[0], "value")
}

fn choice_jump_get_selected(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut rf = export(vm, "roll_forward_info")?;
    let je = py::import_attr(vm, "renpy.game", "JumpException")?;
    if je.obj().is_some_and(|c| ops::isinstance(vm, &rf, c)) {
        let a = py::get(vm, &rf, "args")?;
        rf = ops::getitem(vm, &a, Value::Int(0))?;
    }
    let fixed = {
        let v = export(vm, "in_fixed_rollback")?;
        ops::truthy(vm, &v)?
    };
    let value = py::get_or_none(vm, &args[0], "value")?;
    Ok(Value::Bool(fixed && ops::eq(vm, &rf, &value)?))
}

fn choice_jump_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mark_chosen(vm, &args[0])?;
    let j = py::import_attr(vm, "renpy.exports", "jump")?;
    let v = py::get_or_none(vm, &args[0], "value")?;
    vm.call(&j, vec![v], Vec::new())
}

fn choice_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "value",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    let a = vm.types.new_tuple(args.iter().skip(2).cloned().collect());
    py::set(vm, &args[0], "args", a)?;
    let k = py::kwargs_dict(vm, kw)?;
    py::set(vm, &args[0], "kwargs", k)?;
    Ok(Value::None)
}

fn style_attr(vm: &mut Vm, v: Value) -> PyResult<Value> {
    if let Some(s) = v.as_str() {
        let st = py::import_attr(vm, "renpy.game", "style")?;
        return py::get(vm, &st, s);
    }
    Ok(v)
}

fn f_menu(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = [
        "menuitems",
        "style",
        "caption_style",
        "choice_style",
        "choice_chosen_style",
        "choice_button_style",
        "choice_chosen_button_style",
        "location",
        "focus",
        "default",
    ];
    let defaults = [
        "",
        "menu",
        "menu_caption",
        "menu_choice",
        "menu_choice_chosen",
        "menu_choice_button",
        "menu_choice_chosen_button",
    ];
    let mut v: Vec<Value> = defaults.iter().map(|d| vm.types.new_str(d)).collect();
    v[0] = Value::None;
    v.extend([Value::None, Value::None, Value::Bool(false)]);
    for (i, a) in args.iter().enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    props.push(("style".into(), v[1].clone()));
    let vbox = mget(vm, "vbox")?;
    vm.call(&vbox, Vec::new(), props)?;
    let text = mget(vm, "text")?;
    let tb = mget(vm, "textbutton")?;
    let cr = mget(vm, "ChoiceReturn")?;
    let cro = cr.obj().cloned();
    for item in py::items(vm, &v[0])? {
        let p = py::items(vm, &item)?;
        let (label, val) = (p[0].clone(), p[1].clone());
        if py::is_none(&val) {
            vm.call(&text, vec![label], vec![("style".into(), v[2].clone())])?;
            continue;
        }
        let clicked = if cro.as_ref().is_some_and(|c| ops::isinstance(vm, &val, c)) {
            val
        } else {
            vm.call(&cr, vec![label.clone(), val, v[7].clone()], Vec::new())?
        };
        let chosen = vm.call_method(&clicked, "get_chosen", Vec::new())?;
        let (t, b) = if ops::truthy(vm, &chosen)? {
            (v[4].clone(), v[6].clone())
        } else {
            (v[3].clone(), v[5].clone())
        };
        let b = style_attr(vm, b)?;
        let t = style_attr(vm, t)?;
        let b = ops::getitem(vm, &b, label.clone())?;
        let t = ops::getitem(vm, &t, label.clone())?;
        vm.call(
            &tb,
            vec![label],
            vec![
                ("style".into(), b),
                ("text_style".into(), t),
                ("clicked".into(), clicked),
                ("focus".into(), v[8].clone()),
                ("default".into(), v[9].clone()),
            ],
        )?;
    }
    f_close(vm, &[], &[])
}

fn f_imagemap_compat(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = [
        "ground",
        "selected",
        "hotspots",
        "unselected",
        "style",
        "button_style",
    ];
    let mut v = vec![
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        vm.types.new_str("imagemap"),
        vm.types.new_str("hotspot"),
    ];
    for (i, a) in args.iter().enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    let button_style = style_attr(vm, v[5].clone())?;
    props.push(("style".into(), v[4].clone()));
    let fixed = mget(vm, "fixed")?;
    vm.call(&fixed, Vec::new(), props)?;
    let unselected = if py::is_none(&v[3]) {
        v[0].clone()
    } else {
        v[3].clone()
    };
    let add = mget(vm, "add")?;
    vm.call(&add, vec![v[0].clone()], Vec::new())?;
    let cr = mget(vm, "ChoiceReturn")?;
    let lc = py::import_attr(vm, "renpy.display.layout", "LiveCrop")?;
    let ib = mget(vm, "imagebutton")?;
    for hs in py::items(vm, &v[2])? {
        let p = py::items(vm, &hs)?;
        if py::is_none(&p[4]) {
            continue;
        }
        let (x0, y0, x1, y1) = (
            py::f64_of(vm, &p[0])?,
            py::f64_of(vm, &p[1])?,
            py::f64_of(vm, &p[2])?,
            py::f64_of(vm, &p[3])?,
        );
        let rect = vm.types.new_tuple(vec![
            p[0].clone(),
            p[1].clone(),
            Value::Float(x1 - x0),
            Value::Float(y1 - y0),
        ]);
        let action = vm.call(&cr, vec![p[4].clone(), p[4].clone()], Vec::new())?;
        let sel = vm.call(&lc, vec![rect.clone(), v[1].clone()], Vec::new())?;
        let unsel = vm.call(&lc, vec![rect, unselected.clone()], Vec::new())?;
        let bs = ops::getitem(vm, &button_style, p[4].clone())?;
        vm.call(
            &ib,
            vec![unsel, sel.clone()],
            vec![
                ("selected_idle_image".into(), sel.clone()),
                ("selected_insensitive_image".into(), sel),
                ("clicked".into(), action),
                ("style".into(), bs),
                ("xpos".into(), Value::Float(x0)),
                ("xanchor".into(), Value::Int(0)),
                ("ypos".into(), Value::Float(y0)),
                ("yanchor".into(), Value::Int(0)),
                ("focus_mask".into(), Value::Bool(true)),
            ],
        )?;
    }
    f_close(vm, &[], &[])
}

fn auto_image(vm: &mut Vm, auto: &Value, name: &str) -> PyResult<Value> {
    let f = py::config(vm, "imagemap_auto_function")?;
    vm.call(&f, vec![auto.clone(), vm.types.new_str(name)], Vec::new())
}

fn f_imagebutton(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = [
        "idle_image",
        "hover_image",
        "insensitive_image",
        "activate_image",
        "selected_idle_image",
        "selected_hover_image",
        "selected_insensitive_image",
        "selected_activate_image",
        "idle",
        "hover",
        "insensitive",
        "selected_idle",
        "selected_hover",
        "selected_insensitive",
        "image_style",
        "auto",
    ];
    let mut v = vec![Value::None; names.len()];
    for (i, a) in args.iter().enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    let auto = v[15].clone();
    let mut choice =
        |vm: &mut Vm, a: usize, b: usize, name: &str, required: bool| -> PyResult<Value> {
            if ops::truthy(vm, &v[a])? {
                return Ok(v[a].clone());
            }
            if ops::truthy(vm, &v[b])? {
                return Ok(v[b].clone());
            }
            if !py::is_none(&auto) {
                let r = auto_image(vm, &auto, name)?;
                if !py::is_none(&r) {
                    return Ok(r);
                }
            }
            if required {
                if ops::truthy(vm, &auto)? {
                    let a = ops::repr(vm, &auto)?;
                    return Err(py::error(
                        vm,
                        format!("Imagebutton does not have a {name} image. (auto={a})."),
                    ));
                }
                return Err(py::error(
                    vm,
                    format!("Imagebutton does not have a {name} image."),
                ));
            }
            Ok(Value::None)
        };
    let idle = choice(vm, 8, 0, "idle", true)?;
    let hover = choice(vm, 9, 1, "hover", false)?;
    let insensitive = choice(vm, 10, 2, "insensitive", false)?;
    let s_idle = choice(vm, 11, 4, "selected_idle", false)?;
    let s_hover = choice(vm, 12, 5, "selected_hover", false)?;
    let s_ins = choice(vm, 13, 6, "selected_insensitive", false)?;
    let ib = py::import_attr(vm, "renpy.display.behavior", "ImageButton")?;
    props.extend([
        ("insensitive_image".into(), insensitive),
        ("activate_image".into(), v[3].clone()),
        ("selected_idle_image".into(), s_idle),
        ("selected_hover_image".into(), s_hover),
        ("selected_insensitive_image".into(), s_ins),
        ("selected_activate_image".into(), v[7].clone()),
    ]);
    vm.call(&ib, vec![idle, hover], props)
}

fn split_text(vm: &mut Vm, kw: Kw) -> PyResult<(Vec<(Rc<str>, Value)>, Vec<(Rc<str>, Value)>)> {
    let _ = vm;
    let mut t = Vec::new();
    let mut o = Vec::new();
    for (k, v) in kw {
        match k.strip_prefix("text_") {
            Some(r) => t.push((Rc::from(r), v.clone())),
            None => o.push((k.clone(), v.clone())),
        }
    }
    Ok((t, o))
}

fn text_style_for(vm: &mut Vm, style: &Value, suffix: &str) -> PyResult<Value> {
    let fallback = prefixed_style(vm, suffix)?;
    let gts = py::import_attr(vm, "renpy.style", "get_text_style")?;
    vm.call(&gts, vec![style.clone(), fallback], Vec::new())
}

fn composite_text(
    vm: &mut Vm,
    label: Value,
    kw: Kw,
    outer: &str,
    outer_style: &str,
    text_suffix: &str,
    is_button: bool,
) -> PyResult<Value> {
    let names = if is_button {
        &[
            "label",
            "clicked",
            "style",
            "text_style",
            "substitute",
            "scope",
        ][..]
    } else {
        &["label", "style", "text_style", "substitute", "scope"][..]
    };
    let _ = names;
    let mut clicked = Value::None;
    let mut style = Value::None;
    let mut text_style = Value::None;
    let mut substitute = Value::Bool(true);
    let mut scope = Value::None;
    let mut rest = Vec::new();
    for (k, v) in kw {
        match &**k {
            "clicked" if is_button => clicked = v.clone(),
            "style" => style = v.clone(),
            "text_style" => text_style = v.clone(),
            "substitute" => substitute = v.clone(),
            "scope" => scope = v.clone(),
            _ => rest.push((k.clone(), v.clone())),
        }
    }
    let (mut tk, ok) = split_text(vm, &rest)?;
    if is_button {
        if let Some(pos) = tk.iter().position(|(k, _)| &**k == "align") {
            if matches!(tk[pos].1, Value::Float(_)) {
                tk.remove(pos);
            }
        }
        tk.retain(|(k, _)| &**k != "y_fudge");
    }
    if py::is_none(&style) {
        style = prefixed_style(vm, outer_style)?;
    }
    if py::is_none(&text_style) {
        text_style = text_style_for(vm, &style, text_suffix)?;
    }
    let oc = py::import_attr(
        vm,
        if is_button {
            "renpy.display.behavior"
        } else {
            "renpy.display.layout"
        },
        outer,
    )?;
    let mut okw = ok;
    okw.push(("style".into(), style));
    let rv = if is_button {
        okw.push(("clicked".into(), clicked));
        vm.call(&oc, Vec::new(), okw)?
    } else {
        vm.call(&oc, vec![Value::None], okw)?
    };
    tk.push(("style".into(), text_style));
    tk.push(("substitute".into(), substitute));
    tk.push(("scope".into(), scope));
    let tc = py::import_attr(vm, "renpy.text.text", "Text")?;
    let text = vm.call(&tc, vec![label], tk)?;
    vm.call_method(&rv, "add", vec![text.clone()])?;
    py::set(vm, &rv, "_main", text.clone())?;
    py::set(vm, &rv, "_composite_parts", vm.types.new_list(vec![text]))?;
    Ok(rv)
}

fn f_textbutton(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut k = kw.to_vec();
    if let Some(c) = args.get(1) {
        k.push(("clicked".into(), c.clone()));
    }
    composite_text(
        vm,
        args[0].clone(),
        &k,
        "Button",
        "button",
        "button_text",
        true,
    )
}

fn f_label(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    composite_text(
        vm,
        args[0].clone(),
        kw,
        "Window",
        "label",
        "label_text",
        false,
    )
}

fn f_bar(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (mut width, mut height, mut range, mut value) =
        (Value::None, Value::None, Value::Int(1), Value::Int(0));
    // Upstream's `if len(args) == 4 ... if len(args) == 2 ... else` resets
    // a 4-argument call to the defaults; the behavior is kept.
    if args.len() == 2 {
        range = args[0].clone();
        value = args[1].clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match &**k {
            "width" => width = v.clone(),
            "height" => height = v.clone(),
            "range" => range = v.clone(),
            "value" => value = v.clone(),
            _ => props.push((k.clone(), v.clone())),
        }
    }
    if !props.iter().any(|(k, _)| &**k == "style") {
        let bv = mget(vm, "BarValue")?;
        if bv.obj().is_some_and(|c| ops::isinstance(vm, &value, c)) {
            let vertical = props
                .iter()
                .find(|(k, _)| &**k == "vertical")
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Bool(false));
            let s = vm.call_method(&value, "get_style", Vec::new())?;
            let i = if ops::truthy(vm, &vertical)? { 1 } else { 0 };
            let mut style = ops::getitem(vm, &s, Value::Int(i))?;
            if let Some(st) = style.as_str() {
                let st = st.to_owned();
                style = prefixed_style(vm, &st)?;
            }
            props.push(("style".into(), style));
        }
    }
    let bar = py::import_attr(vm, "renpy.display.behavior", "Bar")?;
    vm.call(&bar, vec![range, value, width, height], props)
}

fn f_autobar_interpolate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (range, start, end, time, st) = (
        args[0].clone(),
        py::f64_of(vm, &args[1])?,
        py::f64_of(vm, &args[2])?,
        py::f64_of(vm, &args[3])?,
        py::f64_of(vm, &args[4])?,
    );
    let (t, redraw) = if st > time {
        (1., Value::None)
    } else {
        (st / time, Value::Int(0))
    };
    let value = start + t * (end - start);
    let bar = py::import_attr(vm, "renpy.display.behavior", "Bar")?;
    let b = vm.call(
        &bar,
        vec![range, Value::Float(value), Value::None, Value::None],
        kw.to_vec(),
    )?;
    Ok(vm.types.new_tuple(vec![b, redraw]))
}

fn f_autobar(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ai = mget(vm, "autobar_interpolate")?;
    let f = vm.call(&ai, args.to_vec(), kw.to_vec())?;
    let dd = py::import_attr(vm, "renpy.display.layout", "DynamicDisplayable")?;
    vm.call(&dd, vec![f], Vec::new())
}

fn viewport_common(vm: &mut Vm, vpf: Value, spacing_to_side: bool, kw: Kw) -> PyResult<Value> {
    let mut scrollbars = Value::None;
    let mut props = Vec::new();
    for (k, v) in kw {
        if &**k == "scrollbars" {
            scrollbars = v.clone();
        } else {
            props.push((k.clone(), v.clone()));
        }
    }
    if py::is_none(&scrollbars) {
        return vm.call(&vpf, Vec::new(), props);
    }
    let (mut vsb, mut sb, mut side, mut vp, mut core) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (k, v) in props {
        if let Some(r) = k.strip_prefix("vscrollbar_") {
            vsb.push((Rc::from(r), v));
        } else if let Some(r) = k.strip_prefix("scrollbar_") {
            sb.push((Rc::from(r), v));
        } else if let Some(r) = k.strip_prefix("side_") {
            side.push((Rc::from(r), v));
        } else if let Some(r) = k.strip_prefix("viewport_") {
            vp.push((Rc::from(r), v));
        } else {
            core.push((k, v));
        }
    }
    if py::config_bool(vm, "position_viewport_side").unwrap_or(false) {
        let compat_min = py::config_bool(vm, "compat_viewport_minimum").unwrap_or(false);
        let pos_names = py::import_attr(vm, "renpy.sl2.slproperties", "position_property_names")?;
        for (k, v) in core {
            let ks = vm.types.new_str(&k);
            if compat_min && matches!(&*k, "minimum" | "xminimum" | "yminimum") {
                vp.push((k, v));
            } else if ops::contains(vm, &pos_names, &ks)? || (spacing_to_side && &*k == "spacing") {
                side.push((k, v));
            } else {
                vp.push((k, v));
            }
        }
    } else {
        vp.extend(core);
    }
    let sbs = scrollbars.as_str().unwrap_or("").to_owned();
    let prefix = py::config_bool(vm, "prefix_viewport_scrollbar_styles").unwrap_or(false);
    let setdefault = |l: &mut Vec<(Rc<str>, Value)>, k: &str, v: Value| {
        if !l.iter().any(|(kk, _)| &**kk == k) {
            l.push((k.into(), v));
        }
    };
    let s = if prefix && sbs != "vertical" {
        prefixed_style(vm, "scrollbar")?
    } else {
        vm.types.new_str("scrollbar")
    };
    setdefault(&mut sb, "style", s);
    let s = if prefix && sbs != "horizontal" {
        prefixed_style(vm, "vscrollbar")?
    } else {
        vm.types.new_str("vscrollbar")
    };
    setdefault(&mut vsb, "style", s);
    let alt = vp
        .iter()
        .find(|(k, _)| &**k == "alt")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| vm.types.new_str("viewport"));
    let tr = py::import_attr(vm, "renpy.minstore", "__")?;
    let a = vm.call(&tr, vec![alt], Vec::new())?;
    let a = py::string(vm, &a)?;
    let h = vm.call(&tr, vec![vm.types.new_str("horizontal scroll")], Vec::new())?;
    let h = py::string(vm, &h)?;
    let vv = vm.call(&tr, vec![vm.types.new_str("vertical scroll")], Vec::new())?;
    let vv = py::string(vm, &vv)?;
    setdefault(&mut sb, "alt", vm.types.new_string(format!("{a} {h}")));
    setdefault(&mut vsb, "alt", vm.types.new_string(format!("{a} {vv}")));
    let child_size = py::config_bool(vm, "scrollbar_child_size").unwrap_or(false);
    const SIZE: i64 = 32767;
    let (layout, cs) = match sbs.as_str() {
        "vertical" => ("c r", (Value::None, Value::Int(SIZE))),
        "horizontal" => ("c b", (Value::Int(SIZE), Value::None)),
        _ => ("c r b", (Value::Int(SIZE), Value::Int(SIZE))),
    };
    if child_size {
        let t = vm.types.new_tuple(vec![cs.0, cs.1]);
        setdefault(&mut vp, "child_size", t);
    }
    let sidef = mget(vm, "side")?;
    vm.call(&sidef, vec![vm.types.new_str(layout)], side)?;
    let rv = vm.call(&vpf, Vec::new(), vp)?;
    let addable = stack_pop(vm)?;
    let vscroll = mget(vm, "vscrollbar")?;
    let hscroll = mget(vm, "scrollbar")?;
    if sbs != "horizontal" {
        let ya = py::get(vm, &rv, "yadjustment")?;
        vsb.push(("adjustment".into(), ya));
        vm.call(&vscroll, Vec::new(), vsb)?;
    }
    if sbs != "vertical" {
        let xa = py::get(vm, &rv, "xadjustment")?;
        sb.push(("adjustment".into(), xa));
        vm.call(&hscroll, Vec::new(), sb)?;
    }
    f_close(vm, &[], &[])?;
    stack_push(vm, addable)?;
    Ok(rv)
}

/// `viewport_common(vpfunc, _spacing_to_side, scrollbars=None, **properties)`.
fn f_viewport_common(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut take = |n: &str| {
        kw.iter()
            .position(|(k, _)| &**k == n)
            .map(|i| kw.remove(i).1)
    };
    let vpf = args.first().cloned().or_else(|| take("vpfunc"));
    let sts = args.get(1).cloned().or_else(|| take("_spacing_to_side"));
    if let Some(sb) = args.get(2) {
        kw.push(("scrollbars".into(), sb.clone()));
    }
    let (Some(vpf), Some(sts)) = (vpf, sts) else {
        return Err(vm.type_error(
            "viewport_common() missing required positional arguments: 'vpfunc' and '_spacing_to_side'",
        ));
    };
    let sts = py::truthy(vm, &sts)?;
    viewport_common(vm, vpf, sts, &kw)
}

/// `style_group_style(s, style_group)` (6.18): `style_group + "_" + s` (or
/// `s`), the style group defaulting to the one of the open container.
fn f_style_group_style(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(args, kw, 0, "s", Value::None);
    let mut group = arg_or(args, kw, 1, "style_group", Value::None);
    let ngg = mget(vm, "NoStyleGroupGiven")?;
    if py::same(&group, &ngg) {
        let top = stack_top(vm)?;
        group = match py::get_opt(vm, &top, "style_group")? {
            Some(g) => g,
            None => py::get_or_none(vm, &top, "style_prefix")?,
        };
    }
    let s = py::string(vm, &s)?;
    let name = if py::is_none(&group) {
        s
    } else {
        format!("{}_{s}", py::string(vm, &group)?)
    };
    let gs = py::import_attr(vm, "renpy.style", "get_style")?;
    vm.call(&gs, vec![py::s(vm, &name)], Vec::new())
}

fn f_viewport(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let f = mget(vm, "_viewport")?;
    viewport_common(vm, f, true, kw)
}

fn f_vpgrid(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let f = mget(vm, "_vpgrid")?;
    viewport_common(vm, f, false, kw)
}

// Imagemaps ---------------------------------------------------------------------------------------------

fn imagemap_obj_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in [
        "insensitive",
        "idle",
        "selected_idle",
        "hover",
        "selected_hover",
        "selected_insensitive",
    ]
    .iter()
    .enumerate()
    {
        let d = super::easy::displayable(vm, &args[i + 1], &Value::None, true)?;
        py::set(vm, this, n, d)?;
    }
    py::set(vm, this, "alpha", args[7].clone())?;
    py::set(vm, this, "cache_param", args[8].clone())?;
    let c = py::import_attr(vm, "renpy.display.imagemap", "ImageMapCache")?;
    let cache = vm.call(&c, vec![args[8].clone()], Vec::new())?;
    py::set(vm, this, "cache", cache)?;
    Ok(Value::None)
}

fn imagemap_obj_reuse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::get(vm, &args[0], "cache_param")?;
    let c = py::import_attr(vm, "renpy.display.imagemap", "ImageMapCache")?;
    let cache = vm.call(&c, vec![p], Vec::new())?;
    py::set(vm, &args[0], "cache", cache)?;
    Ok(Value::None)
}

fn f_imagemap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = [
        "ground",
        "hover",
        "insensitive",
        "idle",
        "selected_hover",
        "selected_idle",
        "selected_insensitive",
        "auto",
        "alpha",
        "cache",
        "style",
    ];
    let mut v = vec![Value::None; names.len()];
    v[8] = Value::Bool(true);
    v[9] = Value::Bool(true);
    v[10] = vm.types.new_str("imagemap");
    for (i, a) in args.iter().enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    let auto = v[7].clone();
    let pick = |vm: &mut Vm, var: &Value, name: &[&str], other: &Value| -> PyResult<Value> {
        if ops::truthy(vm, var)? {
            return Ok(var.clone());
        }
        if ops::truthy(vm, &auto)? {
            for n in name {
                let f = auto_image(vm, &auto, n)?;
                if !py::is_none(&f) {
                    return Ok(f);
                }
            }
        }
        if !py::is_none(other) {
            return Ok(other.clone());
        }
        Err(py::error(
            vm,
            format!("Could not find a {} image for imagemap.", name[0]),
        ))
    };
    let ground = pick(vm, &v[0], &["ground", "idle"], &v[3])?;
    let idle = pick(vm, &v[3], &["idle"], &ground)?;
    let selected_idle = pick(vm, &v[5], &["selected_idle"], &idle)?;
    let hover = pick(vm, &v[1], &["hover"], &ground)?;
    let selected_hover = pick(vm, &v[4], &["selected_hover"], &hover)?;
    let insensitive = pick(vm, &v[2], &["insensitive"], &ground)?;
    let selected_insensitive = pick(vm, &v[6], &["selected_insensitive"], &hover)?;
    let imc = mget(vm, "Imagemap")?;
    let im = vm.call(
        &imc,
        vec![
            insensitive,
            idle,
            selected_idle,
            hover,
            selected_hover,
            selected_insensitive,
            v[8].clone(),
            v[9].clone(),
        ],
        Vec::new(),
    )?;
    let ims = mget(vm, "imagemap_stack")?;
    vm.call_method(&ims, "append", vec![im])?;
    if !props.iter().any(|(k, _)| &**k == "fit_first") {
        props.push(("fit_first".into(), Value::Bool(true)));
    }
    props.push(("style".into(), v[10].clone()));
    props.push(("layout".into(), vm.types.new_str("fixed")));
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let rv = vm.call(&mb, Vec::new(), props)?;
    let mut parts = Vec::new();
    if ops::truthy(vm, &ground)? {
        let g = super::easy::displayable(vm, &ground, &Value::None, true)?;
        vm.call_method(&rv, "add", vec![g])?;
        parts.push(ground);
    }
    let bx = vm.call(
        &mb,
        Vec::new(),
        vec![("layout".into(), vm.types.new_str("fixed"))],
    )?;
    vm.call_method(&rv, "add", vec![bx.clone()])?;
    parts.push(bx.clone());
    py::set(vm, &rv, "_main", bx)?;
    py::set(vm, &rv, "_composite_parts", vm.types.new_list(parts))?;
    Ok(rv)
}

fn imagemap_top(vm: &mut Vm, what: &str) -> PyResult<Value> {
    let ims = mget(vm, "imagemap_stack")?;
    if ops::len(vm, &ims)? == 0 {
        return Err(py::error(
            vm,
            format!("{what} expects an imagemap to be defined."),
        ));
    }
    ops::getitem(vm, &ims, Value::Int(-1))
}

fn crop(vm: &mut Vm, im: &Value, n: &str, spot: &Value) -> PyResult<Value> {
    let cache = py::get(vm, im, "cache")?;
    let d = py::get(vm, im, n)?;
    vm.call_method(&cache, "crop", vec![d, spot.clone()])
}

fn setdefault(props: &mut Vec<(Rc<str>, Value)>, k: &str, v: Value) {
    if !props.iter().any(|(kk, _)| &**kk == k) {
        props.push((k.into(), v));
    }
}

fn f_hotspot_impl(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let spot = arg_or(args, kw, 0, "spot", Value::None);
    let style = arg_or(args, kw, 1, "style", vm.types.new_str("hotspot"));
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "spot" | "style"))
        .cloned()
        .collect();
    let im = imagemap_top(vm, "hotspot")?;
    let s = py::items(vm, &spot)?;
    let mut parts = Vec::new();
    for n in [
        "idle",
        "hover",
        "selected_idle",
        "selected_hover",
        "insensitive",
        "selected_insensitive",
    ] {
        parts.push(crop(vm, &im, n, &spot)?);
    }
    for (k, v) in [
        ("xpos", s[0].clone()),
        ("xanchor", Value::Int(0)),
        ("ypos", s[1].clone()),
        ("yanchor", Value::Int(0)),
        ("xminimum", s[2].clone()),
        ("xmaximum", s[2].clone()),
        ("yminimum", s[3].clone()),
        ("ymaximum", s[3].clone()),
    ] {
        setdefault(&mut props, k, v);
    }
    let alpha = {
        let v = py::get_or_none(vm, &im, "alpha")?;
        ops::truthy(vm, &v)?
    };
    setdefault(
        &mut props,
        "focus_mask",
        if alpha { parts[1].clone() } else { Value::None },
    );
    props.extend([
        ("idle_background".into(), parts[0].clone()),
        ("selected_idle_background".into(), parts[2].clone()),
        ("hover_background".into(), parts[1].clone()),
        ("selected_hover_background".into(), parts[3].clone()),
        ("insensitive_background".into(), parts[4].clone()),
        ("selected_insensitive_background".into(), parts[5].clone()),
        ("style".into(), style),
    ]);
    let b = py::import_attr(vm, "renpy.display.behavior", "Button")?;
    vm.call(&b, vec![Value::None], props)
}

fn f_hotspot(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let h = mget(vm, "hotspot_with_child")?;
    vm.call(&h, args.to_vec(), kw.to_vec())?;
    let n = mget(vm, "null")?;
    vm.call(&n, Vec::new(), Vec::new())?;
    Ok(Value::None)
}

fn f_hotbar(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let spot = arg_or(args, kw, 0, "spot", Value::None);
    let adjustment = arg_or(args, kw, 1, "adjustment", Value::None);
    let range = arg_or(args, kw, 2, "range", Value::None);
    let value = arg_or(args, kw, 3, "value", Value::None);
    let mut props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "spot" | "adjustment" | "range" | "value"))
        .cloned()
        .collect();
    if py::is_none(&adjustment) && py::is_none(&range) && py::is_none(&value) {
        return Err(py::error(
            vm,
            "hotbar requires either an adjustment or a range and value.",
        ));
    }
    let im = imagemap_top(vm, "hotbar")?;
    let s = py::items(vm, &spot)?;
    for (k, v) in [
        ("xpos", s[0].clone()),
        ("ypos", s[1].clone()),
        ("xanchor", Value::Int(0)),
        ("yanchor", Value::Int(0)),
    ] {
        setdefault(&mut props, k, v);
    }
    let mut fore = crop(vm, &im, "selected_idle", &spot)?;
    let mut aft = crop(vm, &im, "idle", &spot)?;
    let mut hfore = crop(vm, &im, "selected_hover", &spot)?;
    let mut haft = crop(vm, &im, "hover", &spot)?;
    if py::f64_of(vm, &s[3])? > py::f64_of(vm, &s[2])? {
        setdefault(&mut props, "bar_vertical", Value::Bool(true));
        setdefault(&mut props, "bar_invert", Value::Bool(true));
        std::mem::swap(&mut fore, &mut aft);
        std::mem::swap(&mut hfore, &mut haft);
    }
    props.extend([
        ("adjustment".into(), adjustment),
        ("range".into(), range),
        ("value".into(), value),
        ("fore_bar".into(), fore),
        ("aft_bar".into(), aft),
        ("hover_fore_bar".into(), hfore),
        ("hover_aft_bar".into(), haft),
        ("fore_gutter".into(), Value::Int(0)),
        ("aft_gutter".into(), Value::Int(0)),
        ("bar_resizing".into(), Value::Bool(false)),
        ("thumb".into(), Value::None),
        ("thumb_shadow".into(), Value::None),
        ("thumb_offset".into(), Value::Int(0)),
        ("xmaximum".into(), s[2].clone()),
        ("ymaximum".into(), s[3].clone()),
    ]);
    let b = py::import_attr(vm, "renpy.display.behavior", "Bar")?;
    vm.call(&b, Vec::new(), props)
}

// Curried helpers ----------------------------------------------------------------------------------------

fn f_returns_impl(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn f_jumps_impl(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let label = arg_or(args, kw, 0, "label", Value::None);
    let mut transition = arg_or(args, kw, 1, "transition", Value::None);
    if let Some(s) = transition.as_str() {
        let s = s.to_owned();
        transition = py::config(vm, &s)?;
    }
    if !py::is_none(&transition) {
        let t = py::import_attr(vm, "renpy.exports", "transition")?;
        vm.call(&t, vec![transition], Vec::new())?;
    }
    let j = py::import_attr(vm, "renpy.exports", "jump")?;
    vm.call(&j, vec![label], Vec::new())
}

fn f_jumpsoutofcontext_impl(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.game", "JumpOutException")?;
    let e = vm.call(&c, vec![args[0].clone()], Vec::new())?;
    Err(crysirror_python::PyErr::Exc(
        e.obj().cloned().expect("exception"),
    ))
}

fn f_callsinnewcontext(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", "curried_call_in_new_context")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

fn f_invokesinnewcontext(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", "curried_invoke_in_new_context")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

fn f_gamemenus(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let gm = vm.types.new_str("_game_menu");
    if let Some(a) = args.first() {
        return f_callsinnewcontext(vm, &[gm], &[("_game_menu_screen".into(), a.clone())]);
    }
    f_callsinnewcontext(vm, &[gm], &[])
}

fn f_screen_id(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let screen = mget(vm, "screen")?;
    if py::is_none(&screen) {
        return Err(py::error(
            vm,
            "ui.screen_id must be called from within a screen.",
        ));
    }
    let d = &args[1];
    let m = py::get_or_none(vm, d, "_main")?;
    let main = if ops::truthy(vm, &m)? { m } else { d.clone() };
    let w = py::get(vm, &screen, "widgets")?;
    ops::setitem(vm, &w, args[0].clone(), main)?;
    let bw = py::get(vm, &screen, "base_widgets")?;
    ops::setitem(vm, &bw, args[0].clone(), d.clone())?;
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let mm = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("reset", f_reset),
            ("interact", f_interact),
            ("tag", f_tag),
            ("child_or_fixed", f_child_or_fixed),
            ("remove", f_remove),
            ("remove_above", f_remove_above),
            ("at", f_at),
            ("clear", f_clear),
            ("detached", f_detached),
            ("layer", f_layer),
            ("close", f_close),
            ("reopen", f_reopen),
            ("context_enter", f_context_enter),
            ("context_exit", f_context_exit),
            ("combine_style", f_combine_style),
            ("prefixed_style", f_prefixed_style),
            ("menu", f_menu),
            ("imagemap_compat", f_imagemap_compat),
            ("viewport", f_viewport),
            ("vpgrid", f_vpgrid),
            ("hotspot", f_hotspot),
            ("callsinnewcontext", f_callsinnewcontext),
            ("invokesinnewcontext", f_invokesinnewcontext),
            ("gamemenus", f_gamemenus),
            ("screen_id", f_screen_id),
            ("_add", f_add),
            ("_implicit_add", f_implicit_add),
            ("_image", f_image),
            ("_sizer", f_sizer),
            ("_key", f_key),
            ("_imagebutton", f_imagebutton),
            ("_textbutton", f_textbutton),
            ("_label", f_label),
            ("_bar", f_bar),
            ("_autobar_interpolate", f_autobar_interpolate),
            ("_autobar", f_autobar),
            ("_imagemap", f_imagemap),
            ("_hotspot", f_hotspot_impl),
            ("_hotbar", f_hotbar),
            ("_returns", f_returns_impl),
            ("_jumps", f_jumps_impl),
            ("_jumpsoutofcontext", f_jumpsoutofcontext_impl),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &mm, n, v);
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let obj = Value::Obj(vm.types.object.clone());
    let cls = |vm: &mut Vm,
               name: &str,
               bases: &[Value],
               methods: Methods,
               attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let c = Value::Obj(py::class(vm, MODULE, name, bases, methods, attrs)?);
        crysirror_python::modules::set(vm, &mm, name, c.clone());
        Ok(c)
    };
    let action = cls(
        vm,
        "Action",
        &[object.clone()],
        &[
            ("get_sensitive", ret_true),
            ("get_selected", ret_false),
            ("get_tooltip", ret_none),
            ("periodic", ret_none),
            ("predict", ret_none),
            ("__call__", not_implemented),
        ],
        &[("alt", Value::None)],
    )?;
    cls(
        vm,
        "BarValue",
        &[object],
        &[
            ("replaces", ret_none),
            ("periodic", ret_none),
            ("get_adjustment", not_implemented),
            ("get_style", barvalue_get_style),
            ("get_tooltip", ret_none),
        ],
        &[
            ("alt", vm.types.new_str("Bar")),
            ("force_step", Value::Bool(false)),
        ],
    )?;
    let addable = cls(
        vm,
        "Addable",
        &[obj.clone()],
        &[
            ("add", not_implemented),
            ("close", not_implemented),
            ("get_layer", addable_get_layer),
        ],
        &[("style_prefix", Value::None)],
    )?;
    cls(
        vm,
        "Layer",
        &[addable.clone()],
        &[
            ("__init__", layer_init),
            ("add", layer_add),
            ("close", layer_close),
            ("get_layer", layer_get_layer),
            ("__repr__", |vm, a, _| repr_of(vm, &a[0], "Layer", "name")),
        ],
        &[],
    )?;
    cls(
        vm,
        "Many",
        &[addable.clone()],
        &[
            ("__init__", many_init),
            ("add", add_to_displayable),
            ("close", many_close),
            ("__repr__", |vm, a, _| {
                repr_of(vm, &a[0], "Many", "displayable")
            }),
        ],
        &[],
    )?;
    cls(
        vm,
        "One",
        &[addable.clone()],
        &[
            ("__init__", one_init),
            ("add", one_add),
            ("close", one_close),
            ("__repr__", |vm, a, _| {
                repr_of(vm, &a[0], "One", "displayable")
            }),
        ],
        &[],
    )?;
    cls(
        vm,
        "Detached",
        &[addable.clone()],
        &[
            ("__init__", prefix_init),
            ("add", detached_add),
            ("close", detached_close),
        ],
        &[],
    )?;
    cls(
        vm,
        "ChildOrFixed",
        &[addable],
        &[
            ("__init__", cof_init),
            ("add", cof_add),
            ("close", cof_close),
        ],
        &[],
    )?;
    let robj = py::import_attr(vm, "renpy.object", "Object")?;
    let wrapper = cls(
        vm,
        "Wrapper",
        &[robj],
        &[
            ("__reduce__", wrapper_reduce),
            ("__init__", wrapper_init),
            ("__call__", wrapper_call),
        ],
        &[],
    )?;
    let cab = cls(
        vm,
        "ChoiceActionBase",
        &[action],
        &[
            ("__init__", cab_init),
            ("get_sensitive", cab_get_sensitive),
            ("get_selected", cab_get_selected),
            ("get_chosen", cab_get_chosen),
        ],
        &[("sensitive", Value::Bool(true))],
    )?;
    let g = vm.types.new_builtin("chosen", cab_chosen);
    let p = py::property(vm, g, None)?;
    py::set(vm, &cab, "chosen", p)?;
    cls(
        vm,
        "ChoiceReturn",
        &[cab.clone()],
        &[("__call__", choice_return_call)],
        &[],
    )?;
    cls(
        vm,
        "ChoiceJump",
        &[cab],
        &[
            ("get_selected", choice_jump_get_selected),
            ("__call__", choice_jump_call),
        ],
        &[],
    )?;
    cls(
        vm,
        "Choice",
        &[obj.clone()],
        &[("__init__", choice_init)],
        &[],
    )?;
    cls(
        vm,
        "Imagemap",
        &[obj],
        &[
            ("__init__", imagemap_obj_init),
            ("reuse", imagemap_obj_reuse),
        ],
        &[
            ("alpha", Value::Bool(true)),
            ("cache_param", Value::Bool(true)),
        ],
    )?;
    let nsp = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let nsp = vm.call(
        &nsp,
        vec![vm.types.new_str("NoStylePrefixGiven")],
        Vec::new(),
    )?;
    set(vm, "NoStylePrefixGiven", nsp);
    for n in ["stack", "at_stack", "imagemap_stack"] {
        let l = vm.types.new_list(Vec::new());
        set(vm, n, l);
    }
    set(vm, "add_tag", Value::None);
    set(vm, "screen", Value::None);
    set(vm, "VIEWPORT_SIZE", Value::Int(32767));
    // Curried helpers.
    let curry = py::import_attr(vm, "renpy.curry", "curry")?;
    for (n, f) in [
        ("returns", "_returns"),
        ("jumps", "_jumps"),
        ("jumpsoutofcontext", "_jumpsoutofcontext"),
        ("autobar_interpolate", "_autobar_interpolate"),
    ] {
        let inner = py::get(vm, &Value::Obj(mm.clone()), f)?;
        let c = vm.call(&curry, vec![inner], Vec::new())?;
        set(vm, n, c);
    }
    let adj = py::import_attr(vm, "renpy.display.behavior", "Adjustment")?;
    set(vm, "adjustment", adj);
    // Wrappers: (name, module, attr or local function, keyword arguments).
    let s = |vm: &Vm, x: &str| vm.types.new_str(x);
    let t = Value::Bool(true);
    let wrappers: Vec<(&str, Value, Vec<(&str, Value)>)> = vec![
        ("add", py::get(vm, &Value::Obj(mm.clone()), "_add")?, vec![]),
        (
            "implicit_add",
            py::get(vm, &Value::Obj(mm.clone()), "_implicit_add")?,
            vec![],
        ),
        (
            "image",
            py::get(vm, &Value::Obj(mm.clone()), "_image")?,
            vec![],
        ),
        (
            "null",
            py::import_attr(vm, "renpy.display.layout", "Null")?,
            vec![],
        ),
        (
            "text",
            py::import_attr(vm, "renpy.text.text", "Text")?,
            vec![("style", s(vm, "text")), ("replaces", t.clone())],
        ),
        (
            "hbox",
            py::import_attr(vm, "renpy.display.layout", "MultiBox")?,
            vec![
                ("layout", s(vm, "horizontal")),
                ("style", s(vm, "hbox")),
                ("many", t.clone()),
            ],
        ),
        (
            "vbox",
            py::import_attr(vm, "renpy.display.layout", "MultiBox")?,
            vec![
                ("layout", s(vm, "vertical")),
                ("style", s(vm, "vbox")),
                ("many", t.clone()),
            ],
        ),
        (
            "fixed",
            py::import_attr(vm, "renpy.display.layout", "MultiBox")?,
            vec![
                ("layout", s(vm, "fixed")),
                ("style", s(vm, "fixed")),
                ("many", t.clone()),
            ],
        ),
        (
            "default_fixed",
            py::import_attr(vm, "renpy.display.layout", "MultiBox")?,
            vec![("layout", s(vm, "fixed")), ("many", t.clone())],
        ),
        (
            "grid",
            py::import_attr(vm, "renpy.display.layout", "Grid")?,
            vec![("style", s(vm, "grid")), ("many", t.clone())],
        ),
        (
            "side",
            py::import_attr(vm, "renpy.display.layout", "Side")?,
            vec![("style", s(vm, "side")), ("many", t.clone())],
        ),
        (
            "sizer",
            py::get(vm, &Value::Obj(mm.clone()), "_sizer")?,
            vec![("one", t.clone())],
        ),
        (
            "window",
            py::import_attr(vm, "renpy.display.layout", "Window")?,
            vec![
                ("style", s(vm, "window")),
                ("one", t.clone()),
                ("child", Value::None),
            ],
        ),
        (
            "frame",
            py::import_attr(vm, "renpy.display.layout", "Window")?,
            vec![
                ("style", s(vm, "frame")),
                ("one", t.clone()),
                ("child", Value::None),
            ],
        ),
        (
            "keymap",
            py::import_attr(vm, "renpy.display.behavior", "Keymap")?,
            vec![],
        ),
        (
            "saybehavior",
            py::import_attr(vm, "renpy.display.behavior", "SayBehavior")?,
            vec![],
        ),
        (
            "pausebehavior",
            py::import_attr(vm, "renpy.display.behavior", "PauseBehavior")?,
            vec![],
        ),
        (
            "soundstopbehavior",
            py::import_attr(vm, "renpy.display.behavior", "SoundStopBehavior")?,
            vec![],
        ),
        ("key", py::get(vm, &Value::Obj(mm.clone()), "_key")?, vec![]),
        (
            "input",
            py::import_attr(vm, "renpy.display.behavior", "Input")?,
            vec![
                ("exclude", s(vm, "{}")),
                ("style", s(vm, "input")),
                ("replaces", t.clone()),
            ],
        ),
        (
            "button",
            py::import_attr(vm, "renpy.display.behavior", "Button")?,
            vec![("style", s(vm, "button")), ("one", t.clone())],
        ),
        (
            "imagebutton",
            py::get(vm, &Value::Obj(mm.clone()), "_imagebutton")?,
            vec![("style", s(vm, "image_button"))],
        ),
        (
            "textbutton",
            py::get(vm, &Value::Obj(mm.clone()), "_textbutton")?,
            vec![],
        ),
        (
            "label",
            py::get(vm, &Value::Obj(mm.clone()), "_label")?,
            vec![],
        ),
        (
            "bar",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("vertical", Value::Bool(false)), ("replaces", t.clone())],
        ),
        (
            "vbar",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("vertical", t.clone()), ("replaces", t.clone())],
        ),
        (
            "slider",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("style", s(vm, "slider")), ("replaces", t.clone())],
        ),
        (
            "vslider",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("style", s(vm, "vslider")), ("replaces", t.clone())],
        ),
        (
            "scrollbar",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("style", s(vm, "scrollbar")), ("replaces", t.clone())],
        ),
        (
            "vscrollbar",
            py::get(vm, &Value::Obj(mm.clone()), "_bar")?,
            vec![("style", s(vm, "vscrollbar")), ("replaces", t.clone())],
        ),
        (
            "autobar",
            py::get(vm, &Value::Obj(mm.clone()), "_autobar")?,
            vec![],
        ),
        (
            "transform",
            py::import_attr(vm, "renpy.display.motion", "Transform")?,
            vec![("one", t.clone()), ("style", s(vm, "transform"))],
        ),
        (
            "_viewport",
            py::import_attr(vm, "renpy.display.viewport", "Viewport")?,
            vec![
                ("one", t.clone()),
                ("replaces", t.clone()),
                ("style", s(vm, "viewport")),
            ],
        ),
        (
            "_vpgrid",
            py::import_attr(vm, "renpy.display.viewport", "VPGrid")?,
            vec![
                ("many", t.clone()),
                ("replaces", t.clone()),
                ("style", s(vm, "vpgrid")),
            ],
        ),
        (
            "conditional",
            py::import_attr(vm, "renpy.display.behavior", "Conditional")?,
            vec![("one", t.clone())],
        ),
        (
            "timer",
            py::import_attr(vm, "renpy.display.behavior", "Timer")?,
            vec![("replaces", t.clone())],
        ),
        (
            "drag",
            py::import_attr(vm, "renpy.display.dragdrop", "Drag")?,
            vec![("replaces", t.clone()), ("one", t.clone())],
        ),
        (
            "draggroup",
            py::import_attr(vm, "renpy.display.dragdrop", "DragGroup")?,
            vec![("replaces", t.clone()), ("many", t.clone())],
        ),
        (
            "mousearea",
            py::import_attr(vm, "renpy.display.behavior", "MouseArea")?,
            vec![("replaces", t.clone())],
        ),
        (
            "imagemap",
            py::get(vm, &Value::Obj(mm.clone()), "_imagemap")?,
            vec![("imagemap", t.clone()), ("style", s(vm, "imagemap"))],
        ),
        (
            "hotspot_with_child",
            py::get(vm, &Value::Obj(mm.clone()), "_hotspot")?,
            vec![("style", s(vm, "hotspot")), ("one", t.clone())],
        ),
        (
            "hotbar",
            py::get(vm, &Value::Obj(mm.clone()), "_hotbar")?,
            vec![("style", s(vm, "hotbar")), ("replaces", t.clone())],
        ),
        (
            "on",
            py::import_attr(vm, "renpy.display.behavior", "OnEvent")?,
            vec![],
        ),
    ];
    for (name, f, kwargs) in wrappers {
        let k: Vec<(Rc<str>, Value)> = kwargs.into_iter().map(|(k, v)| (Rc::from(k), v)).collect();
        let w = vm.call(&wrapper, vec![f], k)?;
        py::set(vm, &w, "name", vm.types.new_str(name))?;
        set(vm, name, w);
    }
    let rel = super::slparser::release(vm);
    // `from renpy.display.behavior import is_selected, is_sensitive`.
    for n in ["is_selected", "is_sensitive"] {
        let f = py::import_attr(vm, "renpy.display.behavior", n)?;
        set(vm, n, f);
    }
    if rel >= (6, 99) {
        let f = vm.types.new_builtin("viewport_common", f_viewport_common);
        set(vm, "viewport_common", f);
    } else {
        // 6.18 names the style group sentinel and helper.
        let ngg = py::import_attr(vm, "renpy.object", "Sentinel")?;
        let ngg = vm.call(&ngg, vec![vm.types.new_str("NoStyleGroupGiven")], Vec::new())?;
        set(vm, "NoStyleGroupGiven", ngg);
        let f = vm.types.new_builtin("style_group_style", f_style_group_style);
        set(vm, "style_group_style", f);
    }
    // The postamble's `for k, v in globals().items()` leaves its loop
    // variables in the module: the final item is ('v', <previous value>),
    // after ('k', 'screen_id') in the 8.x global order.
    let last = if rel >= (6, 99) { "screen_id" } else { "on" };
    set(vm, "k", vm.types.new_str("v"));
    set(vm, "v", vm.types.new_str(last));
    // `renpy.game.post_init.append(reset)` at import, as upstream.
    let pi = py::import_attr(vm, "renpy.game", "post_init")?;
    let reset = py::get(vm, &Value::Obj(mm.clone()), "reset")?;
    ops::list_append(vm, &pi, reset)?;
    Ok(mm)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
