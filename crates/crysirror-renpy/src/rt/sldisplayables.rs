// SPDX-License-Identifier: MPL-2.0
//! `renpy.sl2.sldisplayables`, after Ren'Py 8.3 `sldisplayables.py` (MIT):
//! the ShowIf container and the factory functions behind the `bar`,
//! `vbar`, `viewport`, `vpgrid`, `add` and `image` screen statements. The
//! statement table itself is `crate::sl2::builtin_statements`.
use super::{
    layout::{self, rargs},
    py::{self, *},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.sl2.sldisplayables";

// ShowIf ------------------------------------------------------------------------------------------------

fn showif_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    layout::container_init(vm, &[this.clone()], &[])?;
    let condition = arg_or(args, kw, 1, "condition", Value::None);
    let replaces = arg_or(args, kw, 2, "replaces", Value::None);
    let cond = py::truthy(vm, &condition)?;
    py::set(vm, this, "condition", condition.clone())?;
    if py::is_none(&replaces) {
        py::set(
            vm,
            this,
            "pending_event",
            if cond {
                py::s(vm, "appear")
            } else {
                Value::None
            },
        )?;
        py::set(vm, this, "show_child", condition)?;
    } else {
        let rc = py::get(vm, &replaces, "condition")?;
        let rc = py::truthy(vm, &rc)?;
        let ev = if cond && !rc {
            py::s(vm, "show")
        } else if !cond && rc {
            py::s(vm, "hide")
        } else {
            py::get_or_none(vm, &replaces, "pending_event")?
        };
        py::set(vm, this, "pending_event", ev)?;
        let sc = py::get_or_none(vm, &replaces, "show_child")?;
        py::set(vm, this, "show_child", sc)?;
    }
    Ok(Value::None)
}

fn showif_box_skip(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sc = py::get_or_none(vm, &args[0], "show_child")?;
    Ok(Value::Bool(!py::truthy(vm, &sc)?))
}

fn showif_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = py::get_or_none(vm, this, "pending_event")?;
    if py::truthy(vm, &ev)? {
        let c = py::get(vm, this, "child")?;
        vm.call_method(&c, "set_transform_event", vec![ev])?;
        py::set(vm, this, "pending_event", Value::None)?;
    }
    Ok(Value::None)
}

fn showif_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let child = py::get(vm, this, "child")?;
    let cond_v = py::get(vm, this, "condition")?;
    let cond = py::truthy(vm, &cond_v)?;
    let mut cr = None;
    if super::transform::is_transform(vm, &child) {
        let sc = py::get_or_none(vm, this, "show_child")?;
        if cond || py::truthy(vm, &sc)? {
            cr = Some(layout::render(vm, &child, w, h, st, at)?);
            let hr = py::get_or_none(vm, &child, "hide_response")?;
            let show = cond || !py::truthy(vm, &hr)?;
            py::set(vm, this, "show_child", Value::Bool(show))?;
        }
    } else if cond {
        cr = Some(layout::render(vm, &child, w, h, st, at)?);
        py::set(vm, this, "show_child", Value::Bool(true))?;
    } else {
        py::set(vm, this, "show_child", Value::Bool(false))?;
    }
    let sc = py::get_or_none(vm, this, "show_child")?;
    let rv = match (cr, py::truthy(vm, &sc)?) {
        (Some(cr), true) => {
            let (cw, ch) = layout::size(&cr);
            let rv = layout::new(vm, cw, ch);
            let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
            vm.call_method(&rv, "blit", vec![cr, pos, Value::Bool(cond)])?;
            rv
        }
        _ => layout::new(vm, 0., 0.),
    };
    let zero = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    py::set(vm, this, "offsets", vm.types.new_list(vec![zero]))?;
    Ok(rv)
}

fn showif_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cond = py::get(vm, this, "condition")?;
    if !py::truthy(vm, &cond)? {
        return Ok(Value::None);
    }
    let c = py::get(vm, this, "child")?;
    vm.call_method(&c, "event", args[1..].to_vec())
}

fn showif_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "child")?;
    vm.call_method(&c, "get_placement", Vec::new())
}

fn showif_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cond = py::get(vm, &args[0], "condition")?;
    if py::truthy(vm, &cond)? {
        vm.call_method(&args[0], "_tts_common", Vec::new())
    } else {
        Ok(py::s(vm, ""))
    }
}

// Factories ---------------------------------------------------------------------------------------------

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter()
        .position(|(k, _)| &**k == n)
        .map(|i| kw.remove(i).1)
}

fn bar(vm: &mut Vm, kw: Kw, vertical: bool) -> PyResult<Value> {
    let mut props: Vec<(Rc<str>, Value)> = kw.to_vec();
    let context = take(&mut props, "context").unwrap_or(Value::None);
    let width = take(&mut props, "width").unwrap_or(Value::None);
    let height = take(&mut props, "height").unwrap_or(Value::None);
    let range = take(&mut props, "range").unwrap_or(Value::Int(1));
    let value = take(&mut props, "value").unwrap_or(Value::Int(0));
    if !props.iter().any(|(k, _)| &**k == "style") {
        let bv = py::import_attr(vm, "renpy.ui", "BarValue")?;
        if bv.obj().is_some_and(|c| ops::isinstance(vm, &value, c)) {
            let styles = vm.call_method(&value, "get_style", Vec::new())?;
            let st = ops::getitem(vm, &styles, Value::Int(vertical as i64))?;
            let prefix = if py::is_none(&context) {
                Value::None
            } else {
                py::get_or_none(vm, &context, "style_prefix")?
            };
            let combine = py::import_attr(vm, "renpy.ui", "combine_style")?;
            let style = vm.call(&combine, vec![prefix, st], Vec::new())?;
            props.push(("style".into(), style));
        }
    }
    props.push(("vertical".into(), Value::Bool(vertical)));
    let c = py::import_attr(vm, "renpy.display.behavior", "Bar")?;
    vm.call(&c, vec![range, value, width, height], props)
}

fn sl2bar(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    bar(vm, kw, false)
}

fn sl2vbar(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    bar(vm, kw, true)
}

fn viewport_like(vm: &mut Vm, kw: Kw, factory: &str) -> PyResult<Value> {
    let mut kwargs: Vec<(Rc<str>, Value)> = kw.to_vec();
    let context = take(&mut kwargs, "context").unwrap_or(Value::None);
    let detached = py::import_attr(vm, "renpy.ui", "detached")?;
    let d = vm.call(&detached, Vec::new(), Vec::new())?;
    let stack = py::import_attr(vm, "renpy.ui", "stack")?;
    if !py::is_none(&context) {
        let top = ops::getitem(vm, &stack, Value::Int(-1))?;
        let sp = py::get_or_none(vm, &context, "style_prefix")?;
        py::set(vm, &top, "style_prefix", sp)?;
    }
    let f = py::import_attr(vm, "renpy.ui", factory)?;
    let vp = vm.call(&f, Vec::new(), kwargs)?;
    vm.call_method(&stack, "pop", Vec::new())?;
    let rv = py::get(vm, &d, "child")?;
    if !py::same(&vp, &rv) {
        py::set(vm, &rv, "_main", vp)?;
    }
    let children = py::get(vm, &rv, "children")?;
    let parts = py::items(vm, &children)?;
    py::set(vm, &rv, "_composite_parts", vm.types.new_list(parts))?;
    Ok(rv)
}

fn sl2viewport(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    viewport_like(vm, kw, "viewport")
}

fn sl2vpgrid(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    viewport_like(vm, kw, "vpgrid")
}

fn sl2add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kwargs: Vec<(Rc<str>, Value)> = kw.to_vec();
    let d = match args.first() {
        Some(d) => d.clone(),
        None => take(&mut kwargs, "d").unwrap_or(Value::None),
    };
    let replaces = take(&mut kwargs, "replaces").unwrap_or(Value::None);
    let scope = take(&mut kwargs, "scope").unwrap_or(Value::None);
    if py::is_none(&d) && super::state::rt(vm).profile.at_least(6, 99) {
        return py::import_attr(vm, "renpy.sl2.slast", "NO_DISPLAYABLE");
    }
    let mut d = super::easy::displayable(vm, &d, &scope, false)?;
    let dup = py::get_or_none(vm, &d, "_duplicatable")?;
    if py::truthy(vm, &dup)? {
        d = vm.call_method(&d, "_duplicate", vec![Value::None])?;
        vm.call_method(&d, "_unique", Vec::new())?;
    }
    let mut rv = d.clone();
    if !py::is_none(&replaces) && super::transform::is_transform(vm, &rv) {
        vm.call_method(&rv, "take_state", vec![replaces.clone()])?;
        vm.call_method(&rv, "take_execution_state", vec![replaces])?;
    }
    if !kwargs.is_empty() {
        let t = py::import_attr(vm, "renpy.display.motion", "Transform")?;
        kwargs.push(("child".into(), d.clone()));
        rv = vm.call(&t, Vec::new(), kwargs)?;
        py::set(vm, &rv, "_main", d)?;
    }
    Ok(rv)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut funcs: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("sl2bar", sl2bar),
        ("sl2vbar", sl2vbar),
        ("sl2viewport", sl2viewport),
        ("sl2add", sl2add),
    ];
    let rel = super::slparser::release(vm);
    if rel >= (6, 99) {
        funcs.push(("sl2vpgrid", sl2vpgrid));
    }
    let m = crysirror_python::modules::module_with(vm, MODULE, &funcs);
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let skip = vm.types.new_builtin("_box_skip", showif_box_skip);
    let skip = py::property(vm, skip, None)?;
    let cls = py::class(
        vm,
        MODULE,
        "ShowIf",
        &[container],
        &[
            ("__init__", showif_init),
            ("per_interact", showif_per_interact),
            ("render", showif_render),
            ("event", showif_event),
            ("get_placement", showif_get_placement),
            ("_tts", showif_tts),
        ],
        &[("_box_skip", skip)],
    )?;
    crysirror_python::modules::set(vm, &m, "ShowIf", Value::Obj(cls));
    // `from renpy.sl2.slparser import ...` and the property lists this
    // release's sldisplayables.py imports from slproperties (6.18 defines
    // them here; they are shared with slproperties).
    for n in [
        "Positional",
        "Keyword",
        "Style",
        "PrefixStyle",
        "add",
        "DisplayableParser",
        "many",
    ] {
        let v = py::import_attr(vm, "renpy.sl2.slparser", n)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    let tags: Vec<[u32; 2]> = super::sl_tables::SL2.iter().map(|(t, _)| *t).collect();
    let t = super::sl_tables::SL2[super::slparser::ref_tag(vm, &tags)].1;
    let mut lists: Vec<&str> = vec![
        "text_properties",
        "box_properties",
        "window_properties",
        "bar_properties",
        "button_properties",
        "text_position_properties",
        "text_text_properties",
        "side_position_properties",
    ];
    if rel < (6, 99) {
        lists = t.displayable_lists.iter().map(|(n, _)| *n).collect();
    }
    if rel >= (7, 0) {
        lists.extend([
            "grid_properties",
            "scrollbar_bar_properties",
            "scrollbar_position_properties",
            "viewport_position_properties",
            "vscrollbar_bar_properties",
            "vscrollbar_position_properties",
        ]);
    }
    if rel >= (7, 5) {
        lists.push("position_properties");
    }
    for n in lists {
        let v = py::import_attr(vm, "renpy.sl2.slproperties", n)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    // The statement definitions of this release, as sldisplayables.py
    // makes them at import.
    let lookup = |vm: &mut Vm, n: &str| py::import_attr(vm, "renpy.sl2.slproperties", n);
    super::slparser::replay_statements(vm, t.displayables, &lookup)?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
