// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.scenelists`, after Ren'Py 8.3 `scenelists.py` (MIT):
//! per-context layer contents, tag/zorder placement, hide/replace
//! transitions, at-lists, camera and layer transforms. Before 8.0 these
//! classes lived in `renpy.display.core`; that module re-exports them so
//! saves of either layout unpickle (`after_upgrade` covers versions 0-9).
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.display.scenelists";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn now(vm: &mut Vm) -> PyResult<f64> {
    let f = py::import_attr(vm, "renpy.display.core", "get_time")?;
    let t = vm.call(&f, Vec::new(), Vec::new())?;
    py::f64_of(vm, &t)
}

fn isinst(vm: &mut Vm, v: &Value, module: &str, cls: &str) -> PyResult<bool> {
    let c = py::import_attr(vm, module, cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

fn is_transform(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    isinst(vm, v, "renpy.display.motion", "Transform")
}

fn tag_str(vm: &mut Vm, sle: &Value) -> PyResult<Option<String>> {
    let t = py::get_or_none(vm, sle, "tag")?;
    if py::is_none(&t) {
        return Ok(None);
    }
    Ok(Some(py::string(vm, &t)?))
}

// Layers -----------------------------------------------------------------------------------------------

fn concat(vm: &mut Vm, names: &[&str]) -> PyResult<Vec<Value>> {
    let mut rv = Vec::new();
    for n in names {
        let v = py::config(vm, n).unwrap_or(Value::None);
        if !py::is_none(&v) {
            rv.extend(py::items(vm, &v)?);
        }
    }
    Ok(rv)
}

fn frozenset(vm: &mut Vm, items: Vec<Value>) -> PyResult<Value> {
    let f = Value::Obj(vm.types.frozenset.clone());
    let l = vm.types.new_list(items);
    vm.call(&f, vec![l], Vec::new())
}

fn f_init_layers(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, MODULE)?;
    let ordered = concat(
        vm,
        &["detached_layers", "bottom_layers", "layers", "top_layers"],
    )?;
    py::set(vm, &m, "ordered_layers", vm.types.new_list(ordered.clone()))?;
    let l = frozenset(vm, ordered)?;
    py::set(vm, &m, "layers", l)?;
    let sticky = concat(vm, &["sticky_layers", "detached_layers"])?;
    let s = frozenset(vm, sticky)?;
    py::set(vm, &m, "sticky_layers", s)?;
    Ok(Value::None)
}

fn layer_names(vm: &mut Vm) -> PyResult<Vec<Value>> {
    let l = mget(vm, "layers")?;
    ops::collect(vm, &l)
}

fn pair_none_list(vm: &Vm) -> Value {
    let l = vm.types.new_list(Vec::new());
    vm.types.new_tuple(vec![Value::None, l])
}

// SceneListEntry ---------------------------------------------------------------------------------------

const SLE_FIELDS: [&str; 6] = [
    "tag",
    "zorder",
    "show_time",
    "animation_time",
    "displayable",
    "name",
];

fn sle_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, n) in SLE_FIELDS.iter().enumerate() {
        let v = arg_or(args, kw, i + 1, n, Value::None);
        py::set(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

fn sle_tuple(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let mut v = Vec::new();
    for n in &SLE_FIELDS[..5] {
        v.push(py::get_or_none(vm, this, n)?);
    }
    Ok(vm.types.new_tuple(v))
}

fn sle_iter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = sle_tuple(vm, &args[0])?;
    ops::iter(vm, &t)
}

fn sle_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = sle_tuple(vm, &args[0])?;
    ops::getitem(vm, &t, args[1].clone())
}

fn sle_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut parts = Vec::new();
    for n in ["tag", "name", "displayable"] {
        let v = py::get_or_none(vm, &args[0], n)?;
        parts.push(ops::repr(vm, &v)?);
    }
    Ok(py::s(
        vm,
        &format!("<SLE: {} {} {}>", parts[0], parts[1], parts[2]),
    ))
}

fn new_sle(vm: &mut Vm, vals: Vec<Value>) -> PyResult<Value> {
    let c = mget(vm, "SceneListEntry")?;
    vm.call(&c, vals, Vec::new())
}

fn sle_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut v = Vec::new();
    for n in SLE_FIELDS {
        v.push(py::get_or_none(vm, &args[0], n)?);
    }
    new_sle(vm, v)
}

fn sle_update_time(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let time = args[1].clone();
    let st = py::get_or_none(vm, this, "show_time")?;
    let at = py::get_or_none(vm, this, "animation_time")?;
    if !py::is_none(&st) && !py::is_none(&at) {
        return Ok(this.clone());
    }
    let rv = sle_copy(vm, &[this.clone()], &[])?;
    let st = if py::truthy(vm, &st)? {
        st
    } else {
        time.clone()
    };
    let at = if py::truthy(vm, &at)? { at } else { time };
    py::set(vm, &rv, "show_time", st)?;
    py::set(vm, &rv, "animation_time", at)?;
    Ok(rv)
}

// SceneLists -------------------------------------------------------------------------------------------

fn layers_of(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    py::get(vm, this, "layers")
}

fn layer_list(vm: &mut Vm, this: &Value, layer: &Value) -> PyResult<Value> {
    let l = layers_of(vm, this)?;
    ops::getitem(vm, &l, layer.clone())
}

fn has_layer(vm: &mut Vm, this: &Value, layer: &Value) -> PyResult<bool> {
    let l = layers_of(vm, this)?;
    ops::contains(vm, &l, layer)
}

fn get_or_default(vm: &mut Vm, this: &Value, n: &str) -> PyResult<Value> {
    match py::get_opt(vm, this, n)? {
        Some(v) => Ok(v),
        None => Ok(vm.types.new_dict()),
    }
}

fn sl_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for n in ["camera_list", "camera_transform", "config_layer_transform"] {
        let v = get_or_default(vm, this, n)?;
        py::set(vm, this, n, v)?;
    }
    let layers = layers_of(vm, this)?;
    let at_list = py::get(vm, this, "at_list")?;
    let lal = py::get(vm, this, "layer_at_list")?;
    let cl = py::get(vm, this, "camera_list")?;
    let clt = py::get(vm, this, "config_layer_transform")?;
    for i in layer_names(vm)? {
        if !ops::contains(vm, &layers, &i)? {
            ops::setitem(vm, &layers, i.clone(), vm.types.new_list(Vec::new()))?;
            ops::setitem(vm, &at_list, i.clone(), vm.types.new_dict())?;
            ops::setitem(vm, &lal, i.clone(), pair_none_list(vm))?;
        }
        if !ops::contains(vm, &cl, &i)? {
            ops::setitem(vm, &cl, i.clone(), pair_none_list(vm))?;
        }
        if !ops::contains(vm, &clt, &i)? {
            ops::setitem(vm, &clt, i, vm.types.new_list(Vec::new()))?;
        }
    }
    Ok(Value::None)
}

fn sl_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 1 {
        let at = vm.types.new_dict();
        let lal = vm.types.new_dict();
        for i in layer_names(vm)? {
            ops::setitem(vm, &at, i.clone(), vm.types.new_dict())?;
            ops::setitem(vm, &lal, i, pair_none_list(vm))?;
        }
        py::set(vm, this, "at_list", at)?;
        py::set(vm, this, "layer_at_list", lal)?;
    }
    if version < 3 {
        py::set(vm, this, "shown_window", Value::Bool(false))?;
    }
    if version < 4 {
        let layers = layers_of(vm, this)?;
        for (k, l) in ops::mapping_items(vm, &layers)? {
            let mut out = Vec::new();
            for i in py::items(vm, &l)? {
                let mut vals = py::items(vm, &i)?;
                vals.push(Value::None);
                out.push(new_sle(vm, vals)?);
            }
            ops::setitem(vm, &layers, k, vm.types.new_list(out))?;
        }
        py::set(
            vm,
            this,
            "additional_transient",
            vm.types.new_list(Vec::new()),
        )?;
    }
    if version < 5 {
        py::set(vm, this, "drag_group", Value::None)?;
    }
    if version < 6 {
        let s = py::get_or_none(vm, this, "image_predict_info")?;
        py::set(vm, this, "shown", s)?;
    }
    if version < 7 {
        py::set(vm, this, "layer_transform", vm.types.new_dict())?;
    }
    if version < 8 {
        py::set(vm, this, "sticky_tags", vm.types.new_dict())?;
    }
    if version < 9 {
        let at = py::get_items(vm, this, "additional_transient")?;
        let mut out = Vec::new();
        for t in at {
            let mut p = py::items(vm, &t)?;
            p.truncate(2);
            p.push(Value::None);
            out.push(vm.types.new_tuple(p));
        }
        py::set(vm, this, "additional_transient", vm.types.new_list(out))?;
    }
    Ok(Value::None)
}

fn sl_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let oldsl = arg_or(args, kw, 1, "oldsl", Value::None);
    let shown = arg_or(args, kw, 2, "shown", Value::None);
    py::set(vm, this, "shown_window", Value::Bool(false))?;
    for n in [
        "layers",
        "at_list",
        "layer_at_list",
        "camera_list",
        "layer_transform",
        "camera_transform",
        "config_layer_transform",
        "sticky_tags",
    ] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    py::set(vm, this, "shown", shown)?;
    py::set(
        vm,
        this,
        "additional_transient",
        vm.types.new_list(Vec::new()),
    )?;
    py::set(vm, this, "drag_group", Value::None)?;
    let layers = layers_of(vm, this)?;
    let at_list = py::get(vm, this, "at_list")?;
    let lal = py::get(vm, this, "layer_at_list")?;
    let cl = py::get(vm, this, "camera_list")?;
    if py::truthy(vm, &oldsl)? {
        let ol = py::get(vm, &oldsl, "layers")?;
        let oat = py::get(vm, &oldsl, "at_list")?;
        let olal = py::get(vm, &oldsl, "layer_at_list")?;
        let ocl = py::get(vm, &oldsl, "camera_list")?;
        for i in layer_names(vm)? {
            let l = if ops::contains(vm, &ol, &i)? {
                let v = ops::getitem(vm, &ol, i.clone())?;
                let items = py::items(vm, &v)?;
                vm.types.new_list(items)
            } else {
                vm.types.new_list(Vec::new())
            };
            ops::setitem(vm, &layers, i.clone(), l)?;
            if ops::contains(vm, &oat, &i)? {
                let a = ops::getitem(vm, &oat, i.clone())?;
                let a = vm.call_method(&a, "copy", Vec::new())?;
                ops::setitem(vm, &at_list, i.clone(), a)?;
                let v = ops::getitem(vm, &olal, i.clone())?;
                ops::setitem(vm, &lal, i.clone(), v)?;
                let v = ops::getitem(vm, &ocl, i.clone())?;
                ops::setitem(vm, &cl, i, v)?;
            } else {
                ops::setitem(vm, &at_list, i.clone(), vm.types.new_dict())?;
                ops::setitem(vm, &lal, i.clone(), pair_none_list(vm))?;
                ops::setitem(vm, &cl, i, pair_none_list(vm))?;
            }
        }
        let overlay = py::config(vm, "overlay_layers")?;
        for i in py::items(vm, &overlay)? {
            clear(vm, this, &i, &Value::Bool(false))?;
        }
        replace_transient(vm, this, &Value::None)?;
        py::set(vm, this, "focused", Value::None)?;
        let dg = py::get_or_none(vm, &oldsl, "drag_group")?;
        py::set(vm, this, "drag_group", dg)?;
        for n in ["layer_transform", "camera_transform", "sticky_tags"] {
            let mine = py::get(vm, this, n)?;
            let theirs = get_or_default(vm, &oldsl, n)?;
            vm.call_method(&mine, "update", vec![theirs])?;
        }
    } else {
        for i in layer_names(vm)? {
            ops::setitem(vm, &layers, i.clone(), vm.types.new_list(Vec::new()))?;
            ops::setitem(vm, &at_list, i.clone(), vm.types.new_dict())?;
            ops::setitem(vm, &lal, i.clone(), pair_none_list(vm))?;
            ops::setitem(vm, &cl, i, pair_none_list(vm))?;
        }
        py::set(vm, this, "music", Value::None)?;
        py::set(vm, this, "focused", Value::None)?;
    }
    Ok(Value::None)
}

fn sl_set_transient_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (layer, tag, prefix) = (&args[1], &args[2], &args[3]);
    let mut out = Vec::new();
    for ltp in py::get_items(vm, this, "additional_transient")? {
        let p = py::items(vm, &ltp)?;
        if ops::eq(vm, &p[0], layer)? && ops::eq(vm, &p[1], tag)? {
            out.push(
                vm.types
                    .new_tuple(vec![p[0].clone(), p[1].clone(), prefix.clone()]),
            );
        } else {
            out.push(ltp);
        }
    }
    py::set(vm, this, "additional_transient", vm.types.new_list(out))?;
    Ok(Value::None)
}

fn replace_transient(vm: &mut Vm, this: &Value, prefix: &Value) -> PyResult<()> {
    let tl = py::config(vm, "transient_layers")?;
    for i in py::items(vm, &tl)? {
        clear(vm, this, &i, &Value::Bool(true))?;
    }
    for ltp in py::get_items(vm, this, "additional_transient")? {
        let p = py::items(vm, &ltp)?;
        let pre = if py::is_none(&p[2]) {
            prefix.clone()
        } else {
            p[2].clone()
        };
        remove(vm, this, &p[0], &p[1], &pre)?;
    }
    py::set(
        vm,
        this,
        "additional_transient",
        vm.types.new_list(Vec::new()),
    )?;
    Ok(())
}

fn sl_replace_transient(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let prefix = match arg(args, kw, 1, "prefix") {
        Some(p) => p.clone(),
        None => py::s(vm, "hide"),
    };
    replace_transient(vm, &args[0], &prefix)?;
    Ok(Value::None)
}

fn sl_transient_is_empty(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tl = py::config(vm, "transient_layers")?;
    for i in py::items(vm, &tl)? {
        let l = layer_list(vm, &args[0], &i)?;
        if py::truthy(vm, &l)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

pub fn transform_state(vm: &mut Vm, old: &Value, new: &Value, execution: bool) -> PyResult<Value> {
    if py::is_none(old) {
        return Ok(new.clone());
    }
    if isinst(vm, new, "renpy.display.screen", "ScreenDisplayable")? {
        return Ok(new.clone());
    }
    let from_target = py::config_bool(vm, "take_state_from_target")?;
    let old_t = if from_target {
        vm.call_method(old, "_target", Vec::new())?
    } else {
        old.clone()
    };
    if !is_transform(vm, &old_t)? {
        return Ok(new.clone());
    }
    let active = py::get_or_none(vm, &old_t, "active")?;
    if !py::truthy(vm, &active)? {
        vm.call_method(&old_t, "update_state", Vec::new())?;
    }
    let mut new_thing = new.clone();
    let mut new_t = if from_target {
        vm.call_method(new, "_target", Vec::new())?
    } else {
        new.clone()
    };
    if !is_transform(vm, &new_t)? {
        let t = py::import_attr(vm, "renpy.display.motion", "Transform")?;
        new_t = vm.call(&t, Vec::new(), vec![("child".into(), new_thing)])?;
        new_thing = new_t.clone();
    }
    vm.call_method(&new_t, "take_state", vec![old_t.clone()])?;
    if execution {
        vm.call_method(&new_t, "take_execution_state", vec![old_t])?;
    }
    Ok(new_thing)
}

fn sl_transform_state(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ex = match arg(args, kw, 3, "execution") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    transform_state(vm, &args[1], &args[2], ex)
}

/// `(add_index, remove_index, zorder)`.
fn find_index(
    vm: &mut Vm,
    this: &Value,
    layer: &Value,
    tag: &Value,
    zorder: &Value,
    behind: &Value,
) -> PyResult<(usize, Option<usize>, Value)> {
    let ll = layer_list(vm, this, layer)?;
    let l = py::items(vm, &ll)?;
    let mut zorder = zorder.clone();
    let mut remove = None;
    for (i, sle) in l.iter().enumerate() {
        let t = py::get_or_none(vm, sle, "tag")?;
        let d = py::get_or_none(vm, sle, "displayable")?;
        if (py::truthy(vm, &t)? && ops::eq(vm, &t, tag)?) || ops::eq(vm, &d, tag)? {
            remove = Some(i);
            if py::is_none(&zorder) {
                zorder = py::get_or_none(vm, sle, "zorder")?;
            }
            break;
        }
    }
    if py::is_none(&zorder) {
        let tz = py::config(vm, "tag_zorder")?;
        zorder = vm.call_method(&tz, "get", vec![tag.clone(), Value::Int(0)])?;
    }
    let mut add = None;
    for (i, sle) in l.iter().enumerate() {
        let z = py::get_or_none(vm, sle, "zorder")?;
        if ops::eq(vm, &z, &zorder)? {
            let t = py::get_or_none(vm, sle, "tag")?;
            if py::truthy(vm, &t)? && (ops::eq(vm, &t, tag)? || ops::contains(vm, behind, &t)?) {
                add = Some(i);
                break;
            }
        } else if py::gt(vm, &z, &zorder)? {
            add = Some(i);
            break;
        }
    }
    Ok((add.unwrap_or(l.len()), remove, zorder))
}

fn sl_find_index(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r, z) = find_index(vm, &args[0], &args[1], &args[2], &args[3], &args[4])?;
    Ok(vm.types.new_tuple(vec![
        Value::Int(a as i64),
        r.map(|r| Value::Int(r as i64)).unwrap_or(Value::None),
        z,
    ]))
}

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter()
        .position(|(k, _)| &**k == n)
        .map(|i| kw.remove(i).1)
}

fn sl_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut pos = args[1..].iter().cloned();
    let mut p = |n: &str, kw: &mut Vec<(Rc<str>, Value)>| pos.next().or_else(|| take(kw, n));
    let layer = p("layer", &mut kw).unwrap_or(Value::None);
    let mut thing = p("thing", &mut kw).unwrap_or(Value::None);
    let key = p("key", &mut kw).unwrap_or(Value::None);
    let zorder = p("zorder", &mut kw).unwrap_or(Value::Int(0));
    let behind = p("behind", &mut kw).unwrap_or_else(|| vm.types.new_list(Vec::new()));
    let at_list = p("at_list", &mut kw).unwrap_or_else(|| vm.types.new_list(Vec::new()));
    let name = p("name", &mut kw).unwrap_or(Value::None);
    let atl = p("atl", &mut kw).unwrap_or(Value::None);
    let default_transform = p("default_transform", &mut kw).unwrap_or(Value::None);
    let transient = p("transient", &mut kw).unwrap_or(Value::Bool(false));
    let keep_st = p("keep_st", &mut kw).unwrap_or(Value::Bool(false));
    if !super::core::is_displayable(vm, &thing) {
        let r = ops::repr(vm, &thing)?;
        return Err(py::error(
            vm,
            format!("Attempting to show something that isn't a displayable:{r}"),
        ));
    }
    if !has_layer(vm, this, &layer)? {
        let l = py::string(vm, &layer)?;
        return Err(py::error(
            vm,
            format!("Trying to add something to non-existent layer '{l}'."),
        ));
    }
    if py::truthy(vm, &key)? {
        remove_hide_replaced(vm, this, &layer, &key)?;
        let at = py::get(vm, this, "at_list")?;
        let la = ops::getitem(vm, &at, layer.clone())?;
        ops::setitem(vm, &la, key.clone(), at_list.clone())?;
        let sticky = mget(vm, "sticky_layers")?;
        if ops::contains(vm, &sticky, &layer)? {
            let st = py::get(vm, this, "sticky_tags")?;
            ops::setitem(vm, &st, key.clone(), layer.clone())?;
        }
    }
    if py::truthy(vm, &key)? && py::truthy(vm, &name)? {
        let mut n = vec![key.clone()];
        n.extend(py::items(vm, &name)?.into_iter().skip(1));
        let n = vm.types.new_tuple(n);
        let shown = py::get(vm, this, "shown")?;
        vm.call_method(&shown, "predict_show", vec![layer.clone(), n])?;
    }
    if py::truthy(vm, &transient)? {
        let at = py::get(vm, this, "additional_transient")?;
        let t = vm
            .types
            .new_tuple(vec![layer.clone(), key.clone(), Value::None]);
        ops::list_append(vm, &at, t)?;
    }
    let l = layer_list(vm, this, &layer)?;
    if py::truthy(vm, &atl)? {
        let c = py::import_attr(vm, "renpy.display.motion", "ATLTransform")?;
        thing = vm.call(&c, vec![atl.clone()], vec![("child".into(), thing)])?;
    }
    let (mut add, remove, zorder) = find_index(vm, this, &layer, &key, &zorder, &behind)?;
    let mut at = Value::None;
    let mut st = Value::None;
    if let Some(ri) = remove {
        let sle = ops::getitem(vm, &l, Value::Int(ri as i64))?;
        let old = py::get(vm, &sle, "displayable")?;
        at = py::get_or_none(vm, &sle, "animation_time")?;
        if py::truthy(vm, &keep_st)? {
            st = py::get_or_none(vm, &sle, "show_time")?;
        }
        let r = py::s(vm, "replaced");
        if !hide_or_replace(vm, this, &layer, Some(ri), &r)? && add > ri {
            add -= 1;
        }
        let keep = !py::truthy(vm, &atl)?
            && !py::truthy(vm, &at_list)?
            && py::config_bool(vm, "keep_running_transform")?
            && is_transform(vm, &old)?;
        thing = if keep {
            vm.call_method(&old, "_change_transform_child", vec![thing])?
        } else {
            transform_state(vm, &old, &thing, false)?
        };
        let e = py::s(vm, "replace");
        vm.call_method(&thing, "set_transform_event", vec![e])?;
    } else {
        if !is_transform(vm, &thing)? {
            thing = transform_state(vm, &default_transform, &thing, false)?;
        }
        let e = py::s(vm, "show");
        vm.call_method(&thing, "set_transform_event", vec![e])?;
    }
    let sle = new_sle(vm, vec![key, zorder, st, at, thing.clone(), name])?;
    vm.call_method(&l, "insert", vec![Value::Int(add as i64), sle])?;
    let cb = py::builtin(vm, "visit", |_, _, _| Ok(Value::None));
    if let Err(e) = vm.call_method(&thing, "visit_all", vec![cb]) {
        if py::config_bool(vm, "developer")? {
            return Err(e);
        }
    }
    Ok(Value::None)
}

/// Returns True when the entry stays as a hiding/replaced entry.
fn hide_or_replace(
    vm: &mut Vm,
    this: &Value,
    layer: &Value,
    index: Option<usize>,
    prefix: &Value,
) -> PyResult<bool> {
    let Some(index) = index else { return Ok(false) };
    let l = layer_list(vm, this, layer)?;
    let old = ops::getitem(vm, &l, Value::Int(index as i64))?;
    let now = now(vm)?;
    let st = py::get_or_none(vm, &old, "show_time")?;
    let st = if py::truthy(vm, &st)? {
        py::f64_of(vm, &st)?
    } else {
        now
    };
    let at = py::get_or_none(vm, &old, "animation_time")?;
    let at = if py::truthy(vm, &at)? {
        py::f64_of(vm, &at)?
    } else {
        now
    };
    let d = py::get(vm, &old, "displayable")?;
    let mut prefix = prefix.clone();
    if py::config_bool(vm, "fast_unhandled_event")? {
        let h = vm.call_method(&d, "_handles_event", vec![prefix.clone()])?;
        if !py::truthy(vm, &h)? {
            prefix = Value::None;
        }
    }
    let tag = tag_str(vm, &old)?;
    if let (false, Some(tag)) = (py::is_none(&prefix), tag.filter(|t| !t.is_empty())) {
        let cur = vm.call_method(&d, "_in_current_store", Vec::new())?;
        let nd = vm.call_method(
            &cur,
            "_hide",
            vec![
                Value::Float(now - st),
                Value::Float(now - at),
                prefix.clone(),
            ],
        )?;
        let idx = vm.call_method(&l, "index", vec![old.clone()])?;
        if !py::is_none(&nd) {
            let p = py::string(vm, &prefix)?;
            let z = py::get_or_none(vm, &old, "zorder")?;
            let t = py::s(vm, &format!("{p}${tag}"));
            let sle = new_sle(
                vm,
                vec![t, z, Value::Float(st), Value::Float(at), nd, Value::None],
            )?;
            ops::setitem(vm, &l, idx, sle)?;
            return Ok(true);
        }
    }
    vm.call_method(&l, "pop", vec![Value::Int(index as i64)])?;
    Ok(false)
}

fn sl_hide_or_replace(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let idx = if py::is_none(&args[2]) {
        None
    } else {
        Some(py::int(vm, &args[2])? as usize)
    };
    Ok(Value::Bool(hide_or_replace(
        vm, &args[0], &args[1], idx, &args[3],
    )?))
}

fn sl_get_all_displayables(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let current = match arg(args, kw, 1, "current") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let layers = layers_of(vm, &args[0])?;
    let vals = vm.call_method(&layers, "values", Vec::new())?;
    let mut rv = Vec::new();
    for l in ops::collect(vm, &vals)? {
        for sle in py::items(vm, &l)? {
            if current && tag_str(vm, &sle)?.is_some_and(|t| t.contains('$')) {
                continue;
            }
            rv.push(py::get(vm, &sle, "displayable")?);
        }
    }
    Ok(vm.types.new_list(rv))
}

fn sl_remove_above(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, layer, thing) = (&args[0], &args[1], &args[2]);
    let l = layer_list(vm, this, layer)?;
    let n = ops::len(vm, &l)?;
    let hide = py::s(vm, "hide");
    for i in (0..n).rev() {
        let sle = ops::getitem(vm, &l, Value::Int(i as i64))?;
        if py::truthy(vm, thing)? {
            let t = py::get_or_none(vm, &sle, "tag")?;
            let d = py::get_or_none(vm, &sle, "displayable")?;
            if ops::eq(vm, &t, thing)? || ops::eq(vm, &d, thing)? {
                break;
            }
        }
        if tag_str(vm, &sle)?.is_some_and(|t| t.contains('$')) {
            continue;
        }
        hide_or_replace(vm, this, layer, Some(i), &hide)?;
    }
    Ok(Value::None)
}

fn remove(vm: &mut Vm, this: &Value, layer: &Value, thing: &Value, prefix: &Value) -> PyResult<()> {
    if !has_layer(vm, this, layer)? {
        let l = py::string(vm, layer)?;
        return Err(py::error(
            vm,
            format!("Trying to remove something from non-existent layer '{l}'."),
        ));
    }
    let behind = vm.types.new_list(Vec::new());
    let (_, ri, _) = find_index(vm, this, layer, thing, &Value::Int(0), &behind)?;
    if let Some(ri) = ri {
        let l = layer_list(vm, this, layer)?;
        let sle = ops::getitem(vm, &l, Value::Int(ri as i64))?;
        let tag = py::get_or_none(vm, &sle, "tag")?;
        if py::truthy(vm, &tag)? {
            let shown = py::get(vm, this, "shown")?;
            let t = vm.types.new_tuple(vec![tag.clone()]);
            vm.call_method(&shown, "predict_hide", vec![layer.clone(), t])?;
            let at = py::get(vm, this, "at_list")?;
            let la = ops::getitem(vm, &at, layer.clone())?;
            vm.call_method(&la, "pop", vec![tag.clone(), Value::None])?;
            let st = py::get(vm, this, "sticky_tags")?;
            let cur = vm.call_method(&st, "get", vec![tag.clone(), Value::None])?;
            if ops::eq(vm, &cur, layer)? {
                ops::delitem(vm, &st, tag)?;
            }
        }
        hide_or_replace(vm, this, layer, Some(ri), prefix)?;
    }
    Ok(())
}

fn sl_remove(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let prefix = match arg(args, kw, 3, "prefix") {
        Some(p) => p.clone(),
        None => py::s(vm, "hide"),
    };
    remove(vm, &args[0], &args[1], &args[2], &prefix)?;
    Ok(Value::None)
}

fn clear(vm: &mut Vm, this: &Value, layer: &Value, hide: &Value) -> PyResult<()> {
    if !has_layer(vm, this, layer)? {
        return Ok(());
    }
    let l = layer_list(vm, this, layer)?;
    if !py::truthy(vm, hide)? {
        vm.call_method(&l, "clear", Vec::new())?;
    } else {
        // `hide` is True or a prefix string; True hides with prefix True,
        // as upstream passes it through unchanged.
        let n = ops::len(vm, &l)?;
        for i in (0..n).rev() {
            hide_or_replace(vm, this, layer, Some(i), hide)?;
        }
    }
    let at = py::get(vm, this, "at_list")?;
    let la = ops::getitem(vm, &at, layer.clone())?;
    vm.call_method(&la, "clear", Vec::new())?;
    let st = py::get(vm, this, "sticky_tags")?;
    let nd = vm.types.new_dict();
    for (k, v) in ops::mapping_items(vm, &st)? {
        if !ops::eq(vm, &v, layer)? {
            ops::setitem(vm, &nd, k, v)?;
        }
    }
    py::set(vm, this, "sticky_tags", nd)?;
    let shown = py::get(vm, this, "shown")?;
    vm.call_method(&shown, "predict_scene", vec![layer.clone()])?;
    if py::config_bool(vm, "scene_clears_layer_at_list")? {
        let lal = py::get(vm, this, "layer_at_list")?;
        ops::setitem(vm, &lal, layer.clone(), pair_none_list(vm))?;
    }
    Ok(())
}

fn sl_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let Some(layer) = arg(args, kw, 1, "layer").cloned() else {
        return Err(vm.type_error("clear() missing required argument: 'layer'"));
    };
    let hide = arg_or(args, kw, 2, "hide", Value::Bool(false));
    clear(vm, &args[0], &layer, &hide)?;
    Ok(Value::None)
}

fn sl_set_layer_at_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = args[1].clone();
    let at = arg_or(args, kw, 2, "at_list", Value::None);
    let reset = match arg(args, kw, 3, "reset") {
        Some(v) => py::truthy(vm, v)?,
        None => true,
    };
    let camera = match arg(args, kw, 4, "camera") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let items = py::items(vm, &at)?;
    let l = vm.types.new_list(items);
    let t = vm.types.new_tuple(vec![Value::None, l]);
    let d = py::get(
        vm,
        this,
        if camera {
            "camera_list"
        } else {
            "layer_at_list"
        },
    )?;
    ops::setitem(vm, &d, layer.clone(), t)?;
    if reset {
        let lt = py::get(vm, this, "layer_transform")?;
        ops::setitem(vm, &lt, layer, Value::None)?;
    }
    Ok(Value::None)
}

fn sl_set_times(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let time = args[1].clone();
    for n in ["camera_list", "layer_at_list"] {
        let d = py::get(vm, this, n)?;
        for (l, v) in ops::mapping_items(vm, &d)? {
            let p = py::items(vm, &v)?;
            let t = if py::truthy(vm, &p[0])? {
                p[0].clone()
            } else {
                time.clone()
            };
            let nv = vm.types.new_tuple(vec![t, p[1].clone()]);
            ops::setitem(vm, &d, l, nv)?;
        }
    }
    let layers = layers_of(vm, this)?;
    let vals = vm.call_method(&layers, "values", Vec::new())?;
    for ll in ops::collect(vm, &vals)? {
        let mut out = Vec::new();
        for i in py::items(vm, &ll)? {
            out.push(vm.call_method(&i, "update_time", vec![time.clone()])?);
        }
        vm.call_method(&ll, "clear", Vec::new())?;
        vm.call_method(&ll, "extend", vec![vm.types.new_list(out)])?;
    }
    Ok(Value::None)
}

fn sl_showing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let shown = py::get(vm, &args[0], "shown")?;
    vm.call_method(&shown, "showing", args[1..].to_vec())
}

fn sl_get_showing_tags(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let shown = py::get(vm, &args[0], "shown")?;
    vm.call_method(&shown, "get_showing_tags", args[1..].to_vec())
}

fn sl_get_sorted_tags(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = layer_list(vm, &args[0], &args[1])?;
    let mut rv = Vec::new();
    for sle in py::items(vm, &l)? {
        if let Some(t) = tag_str(vm, &sle)? {
            if !t.is_empty() && !t.contains('$') {
                rv.push(py::get(vm, &sle, "tag")?);
            }
        }
    }
    Ok(vm.types.new_list(rv))
}

fn sl_make_layer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, layer) = (&args[0], &args[1]);
    let props = args.get(2).cloned().unwrap_or(Value::None);
    let mut kw: Vec<(Rc<str>, Value)> = vec![
        ("layout".into(), py::s(vm, "fixed")),
        ("focus".into(), layer.clone()),
    ];
    if !py::is_none(&props) {
        kw.extend(py::str_items(vm, &props)?);
    }
    let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
    let rv = vm.call(&mb, Vec::new(), kw)?;
    let l = layer_list(vm, this, layer)?;
    vm.call_method(&rv, "append_scene_list", vec![l])?;
    py::set(vm, &rv, "layer_name", layer.clone())?;
    py::set(vm, &rv, "_duplicatable", Value::Bool(false))?;
    let lal = py::get(vm, this, "layer_at_list")?;
    let v = ops::getitem(vm, &lal, layer.clone())?;
    py::set(vm, &rv, "_layer_at_list", v)?;
    let cl = py::get(vm, this, "camera_list")?;
    let v = ops::getitem(vm, &cl, layer.clone())?;
    py::set(vm, &rv, "_camera_list", v)?;
    Ok(rv)
}

/// Wraps `rv` in each transform of `at_list` inside a fixed layer box.
fn wrap_layer(
    vm: &mut Vm,
    this: &Value,
    layer: &Value,
    d: &Value,
    rv: Value,
    at_list: &[Value],
    time: Value,
    store: &str,
    keep_state: bool,
) -> PyResult<Value> {
    let st = py::get(vm, this, store)?;
    let old = vm.call_method(&st, "get", vec![layer.clone(), Value::None])?;
    let mut new_t = Value::None;
    let mut rv = rv;
    if !at_list.is_empty() {
        for a in at_list {
            rv = if is_transform(vm, a)? {
                vm.call(a, Vec::new(), vec![("child".into(), rv)])?
            } else {
                vm.call(a, vec![rv], Vec::new())?
            };
            vm.call_method(&rv, "_unique", Vec::new())?;
            if is_transform(vm, &rv)? {
                new_t = rv.clone();
            }
        }
        if !py::is_none(&new_t) && keep_state {
            transform_state(vm, &old, &new_t, true)?;
        }
        let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
        let f = vm.call(&mb, Vec::new(), vec![("layout".into(), py::s(vm, "fixed"))])?;
        vm.call_method(&f, "add", vec![rv, time.clone(), time])?;
        py::set(vm, &f, "layer_name", layer.clone())?;
        py::set(vm, &f, "untransformed_layer", d.clone())?;
        rv = f;
    }
    ops::setitem(vm, &st, layer.clone(), new_t)?;
    Ok(rv)
}

fn sl_transform_layer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (this, layer, d) = (&args[0], &args[1], &args[2]);
    let mut lal = arg_or(args, kw, 3, "layer_at_list", Value::None);
    let mut cl = arg_or(args, kw, 4, "camera_list", Value::None);
    if py::is_none(&lal) {
        let v = py::get(vm, this, "layer_at_list")?;
        lal = ops::getitem(vm, &v, layer.clone())?;
    }
    if py::is_none(&cl) {
        let v = py::get(vm, this, "camera_list")?;
        cl = ops::getitem(vm, &v, layer.clone())?;
    }
    let p = py::items(vm, &lal)?;
    let at = py::items(vm, &p[1])?;
    let keep = py::config_bool(vm, "keep_show_layer_state")?;
    let rv = wrap_layer(
        vm,
        this,
        layer,
        d,
        d.clone(),
        &at,
        p[0].clone(),
        "layer_transform",
        keep,
    )?;
    let p = py::items(vm, &cl)?;
    let at = py::items(vm, &p[1])?;
    let rv = wrap_layer(
        vm,
        this,
        layer,
        d,
        rv,
        &at,
        p[0].clone(),
        "camera_transform",
        true,
    )?;
    let lt = py::config(vm, "layer_transforms")?;
    let at = vm.call_method(
        &lt,
        "get",
        vec![layer.clone(), vm.types.new_list(Vec::new())],
    )?;
    let at = py::items(vm, &at)?;
    wrap_layer(
        vm,
        this,
        layer,
        d,
        rv,
        &at,
        Value::Int(0),
        "config_layer_transform",
        true,
    )
}

fn remove_hide_replaced(vm: &mut Vm, this: &Value, layer: &Value, tag: &Value) -> PyResult<()> {
    let t = py::string(vm, tag)?;
    let (hide_tag, replaced_tag) = (format!("hide${t}"), format!("replaced${t}"));
    let l = layer_list(vm, this, layer)?;
    let now = now(vm)?;
    let mut out = Vec::new();
    for sle in py::items(vm, &l)? {
        if let Some(st) = tag_str(vm, &sle)? {
            if st == hide_tag || st == replaced_tag {
                let d = py::get(vm, &sle, "displayable")?;
                let s = py::get_or_none(vm, &sle, "show_time")?;
                let a = py::get_or_none(vm, &sle, "animation_time")?;
                let s = py::f64_of(vm, &s)?;
                let a = py::f64_of(vm, &a)?;
                let r = vm.call_method(
                    &d,
                    "_hide",
                    vec![
                        Value::Float(now - s),
                        Value::Float(now - a),
                        py::s(vm, "cancel"),
                    ],
                )?;
                if py::is_none(&r) {
                    continue;
                }
            }
        }
        out.push(sle);
    }
    vm.call_method(&l, "clear", Vec::new())?;
    vm.call_method(&l, "extend", vec![vm.types.new_list(out)])?;
    Ok(())
}

fn sl_remove_hide_replaced(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    remove_hide_replaced(vm, &args[0], &args[1], &args[2])?;
    Ok(Value::None)
}

fn sl_remove_hidden(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let now = now(vm)?;
    let layers = layers_of(vm, &args[0])?;
    let vals = vm.call_method(&layers, "values", Vec::new())?;
    for v in ops::collect(vm, &vals)? {
        let mut out = Vec::new();
        for sle in py::items(vm, &v)? {
            if let Some(t) = tag_str(vm, &sle)? {
                let kind = if t.starts_with("hide$") {
                    Some("hide")
                } else if t.starts_with("replaced$") {
                    Some("replaced")
                } else {
                    None
                };
                if let Some(kind) = kind {
                    let d = py::get(vm, &sle, "displayable")?;
                    let s = py::get_or_none(vm, &sle, "show_time")?;
                    let a = py::get_or_none(vm, &sle, "animation_time")?;
                    let s = py::f64_of(vm, &s)?;
                    let a = py::f64_of(vm, &a)?;
                    let r = vm.call_method(
                        &d,
                        "_hide",
                        vec![
                            Value::Float(now - s),
                            Value::Float(now - a),
                            py::s(vm, kind),
                        ],
                    )?;
                    if !py::truthy(vm, &r)? {
                        continue;
                    }
                }
            }
            out.push(sle);
        }
        vm.call_method(&v, "clear", Vec::new())?;
        vm.call_method(&v, "extend", vec![vm.types.new_list(out)])?;
    }
    Ok(Value::None)
}

fn sl_remove_all_hidden(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let layers = layers_of(vm, &args[0])?;
    let vals = vm.call_method(&layers, "values", Vec::new())?;
    for v in ops::collect(vm, &vals)? {
        let mut out = Vec::new();
        for sle in py::items(vm, &v)? {
            if tag_str(vm, &sle)?.is_some_and(|t| t.contains('$')) {
                continue;
            }
            out.push(sle);
        }
        vm.call_method(&v, "clear", Vec::new())?;
        vm.call_method(&v, "extend", vec![vm.types.new_list(out)])?;
    }
    Ok(Value::None)
}

fn unknown_layer(vm: &mut Vm, layer: &Value) -> crysirror_python::PyErr {
    let r = ops::repr(vm, layer).unwrap_or_default();
    py::error(vm, format!("Unknown layer {r}."))
}

fn sl_get_displayable_by(vm: &mut Vm, args: &[Value], field: &str) -> PyResult<Value> {
    let (this, layer, key) = (&args[0], &args[1], &args[2]);
    if !has_layer(vm, this, layer)? {
        return Err(unknown_layer(vm, layer));
    }
    let l = layer_list(vm, this, layer)?;
    for sle in py::items(vm, &l)? {
        let t = py::get_or_none(vm, &sle, field)?;
        if ops::eq(vm, &t, key)? {
            return py::get(vm, &sle, "displayable");
        }
    }
    Ok(Value::None)
}

fn sl_get_displayable_by_tag(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    sl_get_displayable_by(vm, args, "tag")
}

fn sl_get_displayable_by_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    sl_get_displayable_by(vm, args, "name")
}

fn sl_get_image_bounds(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, layer, tag) = (&args[0], &args[1], &args[2]);
    let w = py::f64_of(vm, &args[3])?;
    let h = py::f64_of(vm, &args[4])?;
    if !has_layer(vm, this, layer)? {
        return Err(unknown_layer(vm, layer));
    }
    let l = layer_list(vm, this, layer)?;
    let mut found = None;
    for sle in py::items(vm, &l)? {
        let t = py::get_or_none(vm, &sle, "tag")?;
        if ops::eq(vm, &t, tag)? {
            found = Some(sle);
            break;
        }
    }
    let Some(sle) = found else {
        return Ok(Value::None);
    };
    let now = now(vm)?;
    let s = py::get_or_none(vm, &sle, "show_time")?;
    let st = if py::is_none(&s) {
        0.
    } else {
        now - py::f64_of(vm, &s)?
    };
    let a = py::get_or_none(vm, &sle, "animation_time")?;
    let at = if py::is_none(&a) {
        0.
    } else {
        now - py::f64_of(vm, &a)?
    };
    let d = py::get(vm, &sle, "displayable")?;
    let rfs = py::import_attr(vm, "renpy.display.render", "render_for_size")?;
    let surf = vm.call(
        &rfs,
        vec![
            d.clone(),
            Value::Float(w),
            Value::Float(h),
            Value::Float(st),
            Value::Float(at),
        ],
        Vec::new(),
    )?;
    let (sw, sh) = super::layout::size(&surf);
    let placement = vm.call_method(&d, "get_placement", Vec::new())?;
    let (x, y) = super::core::place(vm, &placement, w, h, sw, sh)?;
    Ok(vm.types.new_tuple(vec![
        Value::Float(x),
        Value::Float(y),
        Value::Float(sw),
        Value::Float(sh),
    ]))
}

fn sl_get_zorder_list(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let layers = layers_of(vm, &args[0])?;
    let l = vm.call_method(
        &layers,
        "get",
        vec![args[1].clone(), vm.types.new_list(Vec::new())],
    )?;
    let mut rv = Vec::new();
    for sle in py::items(vm, &l)? {
        match tag_str(vm, &sle)? {
            None => continue,
            Some(t) if t.contains('$') => continue,
            _ => {}
        }
        let t = py::get(vm, &sle, "tag")?;
        let z = py::get_or_none(vm, &sle, "zorder")?;
        rv.push(vm.types.new_tuple(vec![t, z]));
    }
    Ok(vm.types.new_list(rv))
}

fn sl_change_zorder(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, layer, tag, z) = (&args[0], &args[1], &args[2], &args[3]);
    let layers = layers_of(vm, this)?;
    let sl = vm.call_method(
        &layers,
        "get",
        vec![layer.clone(), vm.types.new_list(Vec::new())],
    )?;
    let mut entries = Vec::new();
    for sle in py::items(vm, &sl)? {
        let t = py::get_or_none(vm, &sle, "tag")?;
        if ops::eq(vm, &t, tag)? {
            py::set(vm, &sle, "zorder", z.clone())?;
        }
        let zz = py::get_or_none(vm, &sle, "zorder")?;
        entries.push((zz, sle));
    }
    // Stable sort by zorder, as `list.sort(key=...)`.
    let mut err = None;
    entries.sort_by(|a, b| {
        if err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match py::lt(vm, &a.0, &b.0) {
            Ok(true) => std::cmp::Ordering::Less,
            Ok(false) => match py::lt(vm, &b.0, &a.0) {
                Ok(true) => std::cmp::Ordering::Greater,
                Ok(false) => std::cmp::Ordering::Equal,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            },
            Err(e) => {
                err = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    if py::is_none(&sl) {
        return Ok(Value::None);
    }
    vm.call_method(&sl, "clear", Vec::new())?;
    let v = entries.into_iter().map(|(_, s)| s).collect();
    vm.call_method(&sl, "extend", vec![vm.types.new_list(v)])?;
    Ok(Value::None)
}

fn f_scene_lists(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let index = arg_or(args, kw, 0, "index", Value::Int(-1));
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let c = vm.call(&ctx, vec![index], Vec::new())?;
    py::get(vm, &c, "scene_lists")
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("init_layers", f_init_layers),
            ("scene_lists", f_scene_lists),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let base = concat(vm, &["layers"])?;
    set(vm, "ordered_layers", vm.types.new_list(base.clone()));
    let l = frozenset(vm, base)?;
    set(vm, "layers", l);
    let s = frozenset(vm, Vec::new())?;
    set(vm, "sticky_layers", s);
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let sle = py::class(
        vm,
        MODULE,
        "SceneListEntry",
        &[object.clone()],
        &[
            ("__init__", sle_init),
            ("__iter__", sle_iter),
            ("__getitem__", sle_getitem),
            ("__repr__", sle_repr),
            ("copy", sle_copy),
            ("update_time", sle_update_time),
        ],
        &[],
    )?;
    set(vm, "SceneListEntry", Value::Obj(sle));
    let sl = py::class(
        vm,
        MODULE,
        "SceneLists",
        &[object],
        &[
            ("__init__", sl_init),
            ("after_setstate", sl_after_setstate),
            ("after_upgrade", sl_after_upgrade),
            ("set_transient_prefix", sl_set_transient_prefix),
            ("replace_transient", sl_replace_transient),
            ("transient_is_empty", sl_transient_is_empty),
            ("transform_state", sl_transform_state),
            ("find_index", sl_find_index),
            ("add", sl_add),
            ("hide_or_replace", sl_hide_or_replace),
            ("get_all_displayables", sl_get_all_displayables),
            ("remove_above", sl_remove_above),
            ("remove", sl_remove),
            ("clear", sl_clear),
            ("set_layer_at_list", sl_set_layer_at_list),
            ("set_times", sl_set_times),
            ("showing", sl_showing),
            ("get_showing_tags", sl_get_showing_tags),
            ("get_sorted_tags", sl_get_sorted_tags),
            ("make_layer", sl_make_layer),
            ("transform_layer", sl_transform_layer),
            ("remove_hide_replaced", sl_remove_hide_replaced),
            ("remove_hidden", sl_remove_hidden),
            ("remove_all_hidden", sl_remove_all_hidden),
            ("get_displayable_by_tag", sl_get_displayable_by_tag),
            ("get_displayable_by_name", sl_get_displayable_by_name),
            ("get_image_bounds", sl_get_image_bounds),
            ("get_zorder_list", sl_get_zorder_list),
            ("change_zorder", sl_change_zorder),
        ],
        &[("__version__", Value::Int(9))],
    )?;
    set(vm, "SceneLists", Value::Obj(sl));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
