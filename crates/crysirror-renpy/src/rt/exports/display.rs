// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.displayexports` (Ren'Py 8.3, MIT): images, show/hide/
//! scene, layers, transitions, placement, the mouse and the renderer.
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};

fn sls(vm: &mut Vm) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get(vm, &c, "scene_lists")
}

fn images(vm: &mut Vm) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get(vm, &c, "images")
}

fn name_tuple(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    if matches!(name.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return Ok(name.clone());
    }
    let s = py::string(vm, name)?;
    let parts = s.split_whitespace().map(|p| vm.types.new_str(p)).collect();
    Ok(vm.types.new_tuple(parts))
}

fn init_phase(vm: &mut Vm) -> PyResult<bool> {
    let c = ctx(vm)?;
    let v = py::get_or_none(vm, &c, "init_phase")?;
    py::truthy(vm, &v)
}

fn is_transform(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    isinst(vm, v, "renpy.display.motion", "Transform")
}

fn is_displayable(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    isinst(vm, v, "renpy.display.displayable", "Displayable")
}

fn count_displayables_in_layer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = sls(vm)?;
    let l = py::get(vm, &s, "layers")?;
    let layer = ops::getitem(vm, &l, args[0].clone())?;
    Ok(Value::Int(ops::len(vm, &layer)? as i64))
}

fn image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (name, d) = (&args[0], &args[1]);
    if py::is_none(d) {
        return Err(py::error(vm, "Images may not be declared to be None."));
    }
    if !init_phase(vm)? {
        return Err(py::error(vm, "Images may only be declared inside init blocks."));
    }
    let name = name_tuple(vm, name)?;
    let d = super::super::easy::displayable(vm, d, &Value::None, false)?;
    let ri = py::import_attr(vm, "renpy.display.image", "register_image")?;
    vm.call(&ri, vec![name, d], Vec::new())
}

fn copy_images(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let old = name_tuple(vm, &args[0])?;
    let old = py::items(vm, &old)?;
    let new = name_tuple(vm, &args[1])?;
    let new = py::items(vm, &new)?;
    let imgs = py::import_attr(vm, "renpy.display.image", "images")?;
    let ri = py::import_attr(vm, "renpy.display.image", "register_image")?;
    for (k, v) in ops::mapping_items(vm, &imgs)? {
        let k = py::items(vm, &k)?;
        if k.len() < old.len() {
            continue;
        }
        let mut same = true;
        for (a, b) in k.iter().zip(&old) {
            if !ops::eq(vm, a, b)? {
                same = false;
                break;
            }
        }
        if same {
            let mut n = new.clone();
            n.extend(k[old.len()..].iter().cloned());
            let t = vm.types.new_tuple(n);
            vm.call(&ri, vec![t, v], Vec::new())?;
        }
    }
    Ok(Value::None)
}

pub fn default_layer(vm: &mut Vm, layer: &Value, tag: &Value, expression: bool) -> PyResult<Value> {
    if !py::is_none(layer) {
        return Ok(layer.clone());
    }
    if py::is_none(tag) || expression {
        return py::config(vm, "default_tag_layer");
    }
    let tag = if let Some(s) = tag.as_str() {
        py::s(vm, s.split_whitespace().next().unwrap_or(""))
    } else if let Some(t) = py::seq(tag) {
        t.first().cloned().unwrap_or(Value::None)
    } else {
        tag.clone()
    };
    let s = sls(vm)?;
    let st = py::get(vm, &s, "sticky_tags")?;
    let v = vm.call_method(&st, "get", vec![tag.clone(), Value::None])?;
    if py::truthy(vm, &v)? {
        return Ok(v);
    }
    let tl = py::config(vm, "tag_layer")?;
    let dtl = py::config(vm, "default_tag_layer")?;
    vm.call_method(&tl, "get", vec![tag, dtl])
}

fn f_default_layer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = kwb(vm, arg(args, kw, 2, "expression"), false)?;
    default_layer(vm, &arg_or(args, kw, 0, "layer", Value::None), &arg_or(args, kw, 1, "tag", Value::None), e)
}

fn can_show(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = name_tuple(vm, &args[0])?;
    let mut tag = arg_or(args, kw, 2, "tag", Value::None);
    if py::is_none(&tag) {
        tag = ops::getitem(vm, &name, Value::Int(0))?;
    }
    let layer = default_layer(vm, &arg_or(args, kw, 1, "layer", Value::None), &tag, false)?;
    let im = images(vm)?;
    Ok(vm.call_method(&im, "apply_attributes", vec![layer, tag, name]).unwrap_or(Value::None))
}

fn showing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = name_tuple(vm, &args[0])?;
    let layer = default_layer(vm, &arg_or(args, kw, 1, "layer", Value::None), &name, false)?;
    let im = images(vm)?;
    vm.call_method(&im, "showing", vec![layer, name])
}

fn get_showing_tags(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 0, "layer", py::s(vm, "master"));
    if kwb(vm, arg(args, kw, 1, "sort"), false)? {
        let s = sls(vm)?;
        return vm.call_method(&s, "get_sorted_tags", vec![layer]);
    }
    let im = images(vm)?;
    vm.call_method(&im, "get_showing_tags", vec![layer])
}

fn get_hidden_tags(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 0, "layer", py::s(vm, "master"));
    let im = images(vm)?;
    vm.call_method(&im, "get_hidden_tags", vec![layer])
}

fn get_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = args[0].clone();
    let layer = default_layer(vm, &arg_or(args, kw, 1, "layer", Value::None), &tag, false)?;
    let ih = arg_or(args, kw, 2, "if_hidden", Value::None);
    let im = images(vm)?;
    vm.call_method(&im, "get_attributes", vec![layer, tag, ih])
}

fn clear_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = args[0].clone();
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let current = get_attributes(vm, &[tag.clone(), layer.clone(), Value::None], &[])?;
    if !py::truthy(vm, &current)? {
        return Ok(Value::None);
    }
    let dl = default_layer(vm, &layer, &tag, false)?;
    let shown = showing(vm, &[tag.clone(), dl], &[])?;
    let mut n = vec![tag.clone()];
    for a in py::items(vm, &current)? {
        let s = py::string(vm, &a)?;
        n.push(py::s(vm, &format!("-{s}")));
    }
    let t = vm.types.new_tuple(n);
    set_tag_attributes(vm, &[t, layer.clone()], &[])?;
    if py::truthy(vm, &shown)? {
        show(vm, &[tag], &[("layer".into(), layer)])?;
    }
    Ok(Value::None)
}

/// `_find_image(layer, key, name, what)` -> (name, what).
fn find_image(vm: &mut Vm, layer: &Value, key: &Value, name: Value, what: Value) -> PyResult<(Value, Value)> {
    if !py::is_none(&what) {
        let w = if what.as_str().is_some() { name_tuple(vm, &what)? } else { what };
        return Ok((name, w));
    }
    if py::config_bool(vm, "image_attributes")? {
        let im = images(vm)?;
        let ni = vm.call_method(&im, "apply_attributes", vec![layer.clone(), key.clone(), name.clone()])?;
        if !py::is_none(&ni) {
            let mut n = vec![key.clone()];
            n.extend(py::items(vm, &ni)?.into_iter().skip(1));
            return Ok((vm.types.new_tuple(n), ni));
        }
    }
    let aa = py::config(vm, "adjust_attributes")?;
    let n0 = ops::getitem(vm, &name, Value::Int(0))?;
    let mut f = vm.call_method(&aa, "get", vec![n0, Value::None])?;
    if !py::truthy(vm, &f)? {
        f = vm.call_method(&aa, "get", vec![Value::None, Value::None])?;
    }
    if !py::is_none(&f) {
        let ni = vm.call(&f, vec![name], Vec::new())?;
        let mut n = vec![key.clone()];
        n.extend(py::items(vm, &ni)?.into_iter().skip(1));
        return Ok((vm.types.new_tuple(n), ni));
    }
    Ok((name.clone(), name))
}

fn f_find_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (n, w) = find_image(vm, &args[0], &args[1], args[2].clone(), args[3].clone())?;
    Ok(vm.types.new_tuple(vec![n, w]))
}

fn image_reference(vm: &mut Vm, what: Value) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.image", "ImageReference")?;
    vm.call(&c, vec![what], vec![("style".into(), py::s(vm, "image_placement"))])
}

fn apply_at_list(vm: &mut Vm, mut img: Value, at_list: &[Value]) -> PyResult<Value> {
    for i in at_list {
        img = if is_transform(vm, i)? { vm.call(i, Vec::new(), vec![("child".into(), img)])? } else { vm.call(i, vec![img], Vec::new())? };
        vm.call_method(&img, "_unique", Vec::new())?;
    }
    Ok(img)
}

fn predict_show(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut name = arg_or(args, kw, 0, "name", Value::None);
    let tag = arg_or(args, kw, 2, "tag", Value::None);
    let what = arg_or(args, kw, 3, "what", Value::None);
    let what = if py::is_none(&what) { arg_or(args, kw, 2, "what", Value::None) } else { what };
    let what = py::kw(kw, "what").cloned().unwrap_or(what);
    let at_list = py::kw(kw, "at_list").cloned().or_else(|| args.get(4).cloned()).unwrap_or(Value::None);
    let layer = py::kw(kw, "layer").cloned().or_else(|| args.get(1).cloned()).unwrap_or(Value::None);
    let tag = py::kw(kw, "tag").cloned().unwrap_or(tag);
    let key = if py::truthy(vm, &tag)? { tag } else { ops::getitem(vm, &name, Value::Int(0))? };
    let layer = default_layer(vm, &layer, &key, false)?;
    let img = if is_displayable(vm, &what)? {
        what
    } else {
        let (n, w) = find_image(vm, &layer, &key, name.clone(), what)?;
        name = n;
        let base = image_reference(vm, w)?;
        let ft = vm.call_method(&base, "find_target", Vec::new())?;
        if !py::truthy(vm, &ft)? {
            return Ok(Value::None);
        }
        base
    };
    let al = if py::is_none(&at_list) { Vec::new() } else { py::items(vm, &at_list)? };
    let img = apply_at_list(vm, img, &al)?;
    let im = images(vm)?;
    vm.call_method(&im, "predict_show", vec![layer, name, Value::Bool(true)])?;
    super::super::predict::displayable(vm, &img)?;
    Ok(Value::None)
}

fn set_tag_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = name_tuple(vm, &args[0])?;
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let tag = ops::getitem(vm, &name, Value::Int(0))?;
    let im = images(vm)?;
    let n = vm.call_method(&im, "apply_attributes", vec![layer.clone(), tag, name])?;
    if !py::is_none(&n) {
        vm.call_method(&im, "predict_show", vec![layer, n, Value::Bool(false)])?;
    }
    Ok(Value::None)
}

pub(crate) fn show(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = |i: usize, n: &str, d: Value| arg_or(args, kw, i, n, d);
    let name = a(0, "name", Value::None);
    let mut at_list = a(1, "at_list", Value::None);
    let layer = a(2, "layer", Value::None);
    let what = a(3, "what", Value::None);
    let mut zorder = a(4, "zorder", Value::None);
    let tag = a(5, "tag", Value::None);
    let behind = a(6, "behind", Value::None);
    let mut atl = a(7, "atl", Value::None);
    let transient = a(8, "transient", Value::Bool(false));
    let munge = a(9, "munge_name", Value::Bool(true));
    if isinst(vm, &atl, "renpy.display.transform", "ATLTransform")? {
        atl = py::get(vm, &atl, "atl")?;
    }
    let mut default_transform = py::config(vm, "default_transform")?;
    if init_phase(vm)? {
        return Err(py::error(vm, "Show may not run while in init phase."));
    }
    let mut name = name_tuple(vm, &name)?;
    if py::is_none(&zorder) && !py::config_bool(vm, "preserve_zorder")? {
        zorder = Value::Int(0);
    }
    let s = sls(vm)?;
    let key = if py::truthy(vm, &tag)? { tag.clone() } else { ops::getitem(vm, &name, Value::Int(0))? };
    let layer = default_layer(vm, &layer, &key, false)?;
    if py::config_bool(vm, "sticky_positions")? && !py::truthy(vm, &at_list)? {
        let al = py::get(vm, &s, "at_list")?;
        let ll = ops::getitem(vm, &al, layer.clone())?;
        if ops::contains(vm, &ll, &key)? {
            at_list = ops::getitem(vm, &ll, key.clone())?;
        }
    }
    if !py::truthy(vm, &at_list)? {
        let tt = py::config(vm, "tag_transform")?;
        let t = vm.call_method(&tt, "get", vec![key.clone(), Value::None])?;
        if !py::is_none(&t) {
            let tl = py::import_attr(vm, "renpy.easy", "to_list")?;
            at_list = vm.call(&tl, vec![t], vec![("copy".into(), Value::Bool(true))])?;
        }
    }
    let img = if is_displayable(vm, &what)? {
        if py::config_bool(vm, "wrap_shown_transforms")? && is_transform(vm, &what)? {
            default_transform = Value::None;
            image_reference(vm, what)?
        } else {
            what
        }
    } else {
        let (n, w) = find_image(vm, &layer, &key, name.clone(), what)?;
        name = n;
        let base = image_reference(vm, w.clone())?;
        let ft = vm.call_method(&base, "find_target", Vec::new())?;
        let ms = py::config(vm, "missing_show")?;
        if !py::truthy(vm, &ft)? && py::truthy(vm, &ms)? {
            let r = vm.call(&ms, vec![name.clone(), w, layer.clone()], Vec::new())?;
            if is_displayable(vm, &r)? {
                r
            } else if py::truthy(vm, &r)? {
                return Ok(Value::None);
            } else {
                base
            }
        } else {
            base
        }
    };
    let al = if py::truthy(vm, &at_list)? { py::items(vm, &at_list)? } else { Vec::new() };
    let img = apply_at_list(vm, img, &al)?;
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    if !py::is_none(&p) {
        let si = py::get_or_none(vm, &p, "_seen_images")?;
        if !py::is_none(&si) {
            let mut parts = Vec::new();
            for i in py::items(vm, &name)? {
                let st = crysirror_python::ops::str(vm, &i)?;
                parts.push(py::s(vm, &st));
            }
            let t = vm.types.new_tuple(parts);
            ops::setitem(vm, &si, t, Value::Bool(true))?;
        }
    }
    if py::truthy(vm, &tag)? && py::truthy(vm, &munge)? {
        let mut n = vec![tag.clone()];
        n.extend(py::items(vm, &name)?.into_iter().skip(1));
        name = vm.types.new_tuple(n);
    }
    let mh = py::config(vm, "missing_hide")?;
    if py::truthy(vm, &mh)? {
        vm.call(&mh, vec![name.clone(), layer.clone()], Vec::new())?;
    }
    let behind = if py::is_none(&behind) { vm.types.new_list(Vec::new()) } else { behind };
    let al = vm.types.new_list(al);
    py::method_kw(vm, &s, "add", vec![layer, img, key, zorder, behind], vec![
        ("at_list".into(), al),
        ("name".into(), name),
        ("atl".into(), atl),
        ("default_transform".into(), default_transform),
        ("transient".into(), transient),
    ])?;
    Ok(Value::None)
}

fn hide(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if init_phase(vm)? {
        return Err(py::error(vm, "Hide may not run while in init phase."));
    }
    let name = name_tuple(vm, &args[0])?;
    let s = sls(vm)?;
    let key = ops::getitem(vm, &name, Value::Int(0))?;
    let layer = default_layer(vm, &arg_or(args, kw, 1, "layer", Value::None), &key, false)?;
    vm.call_method(&s, "remove", vec![layer.clone(), key])?;
    let mh = py::config(vm, "missing_hide")?;
    if py::truthy(vm, &mh)? {
        vm.call(&mh, vec![name, layer], Vec::new())?;
    }
    Ok(Value::None)
}

fn scene(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut layer = arg_or(args, kw, 0, "layer", py::s(vm, "master"));
    if py::is_none(&layer) {
        layer = py::s(vm, "master");
    }
    if init_phase(vm)? {
        return Err(py::error(vm, "Scene may not run while in init phase."));
    }
    let s = sls(vm)?;
    vm.call_method(&s, "clear", vec![layer.clone()])?;
    let ms = py::config(vm, "missing_scene")?;
    if py::truthy(vm, &ms)? {
        vm.call(&ms, vec![layer.clone()], Vec::new())?;
    }
    let di = py::import_attr(vm, "renpy.display", "interface")?;
    if !py::is_none(&di) {
        let ot = py::get(vm, &di, "ongoing_transition")?;
        vm.call_method(&ot, "pop", vec![layer.clone(), Value::None])?;
    }
    let cbs = py::config(vm, "scene_callbacks")?;
    for cb in py::items(vm, &cbs)? {
        vm.call(&cb, vec![layer.clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn prefs(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "preferences")
}

fn toggle_fullscreen(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = prefs(vm)?;
    let f = py::get_or_none(vm, &p, "fullscreen")?;
    let nf = !py::truthy(vm, &f)?;
    py::set(vm, &p, "fullscreen", Value::Bool(nf))?;
    Ok(Value::None)
}

fn take_screenshot(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut scale = arg_or(args, kw, 0, "scale", Value::None);
    let bg = arg_or(args, kw, 1, "background", Value::Bool(false));
    if py::is_none(&scale) {
        let w = py::config(vm, "thumbnail_width")?;
        let h = py::config(vm, "thumbnail_height")?;
        scale = vm.types.new_tuple(vec![w, h]);
    }
    let i = iface(vm)?;
    py::method_kw(vm, &i, "take_screenshot", vec![scale], vec![("background".into(), bg)])
}

fn screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = iface(vm)?;
    vm.call_method(&i, "save_screenshot", vec![args[0].clone()])
}

fn screenshot_to_bytes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = iface(vm)?;
    vm.call_method(&i, "screenshot_to_bytes", args.to_vec())
}

fn transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut trans = args[0].clone();
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let always = kwb(vm, arg(args, kw, 2, "always"), false)?;
    let force = arg_or(args, kw, 3, "force", Value::Bool(false));
    let dict = vm.types.dict.clone();
    if ops::isinstance(vm, &trans, &dict) {
        for (ly, t) in ops::mapping_items(vm, &trans)? {
            transition(vm, &[t], &[("layer".into(), ly), ("always".into(), Value::Bool(always)), ("force".into(), force.clone())])?;
        }
        return Ok(Value::None);
    }
    let p = prefs(vm)?;
    let tr = py::get_or_none(vm, &p, "transitions")?;
    if !always && !py::truthy(vm, &tr)? {
        trans = Value::None;
    }
    let sk = py::config(vm, "skipping")?;
    if py::truthy(vm, &sk)? {
        trans = Value::None;
    }
    let i = iface(vm)?;
    py::method_kw(vm, &i, "set_transition", vec![trans, layer], vec![("force".into(), force)])
}

fn get_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 0, "layer", Value::None);
    let i = iface(vm)?;
    let t = py::get(vm, &i, "transition")?;
    vm.call_method(&t, "get", vec![layer, Value::None])
}

fn get_ongoing_transition(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 0, "layer", Value::None);
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    vm.call_method(&i, "get_ongoing_transition", vec![layer])
}

pub(crate) fn restart_interaction(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = iface(vm)?;
    if !py::is_none(&i) {
        let _ = py::set(vm, &i, "restart_interaction", Value::Bool(true));
    }
    Ok(Value::None)
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn image_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let im = super::super::easy::displayable(vm, &args[0], &Value::None, false)?;
    if !isinst(vm, &im, "renpy.display.im", "Image")? {
        return Err(py::error(vm, "renpy.image_size expects it's argument to be an image."));
    }
    let surf = vm.call_method(&im, "load", Vec::new())?;
    vm.call_method(&surf, "get_size", Vec::new())
}

fn get_at_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = name_tuple(vm, &args[0])?;
    let tag = ops::getitem(vm, &name, Value::Int(0))?;
    let layer = default_layer(vm, &arg_or(args, kw, 1, "layer", Value::None), &tag, false)?;
    let s = sls(vm)?;
    let al = py::get(vm, &s, "at_list")?;
    let l = ops::getitem(vm, &al, layer)?;
    let t = vm.call_method(&l, "get", vec![tag, Value::None])?;
    if py::is_none(&t) {
        return Ok(Value::None);
    }
    let items = py::items(vm, &t)?;
    Ok(vm.types.new_list(items))
}

fn show_layer_at(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tl = py::import_attr(vm, "renpy.easy", "to_list")?;
    let at = vm.call(&tl, vec![args[0].clone()], Vec::new())?;
    let layer = arg_or(args, kw, 1, "layer", py::s(vm, "master"));
    let reset = arg_or(args, kw, 2, "reset", Value::Bool(true));
    let camera = arg_or(args, kw, 3, "camera", Value::Bool(false));
    let s = sls(vm)?;
    py::method_kw(vm, &s, "set_layer_at_list", vec![layer, at], vec![("reset".into(), reset), ("camera".into(), camera)])
}

pub(crate) fn free_memory(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    if !py::is_none(&i) {
        vm.call_method(&i, "kill_textures", Vec::new())?;
        vm.call_method(&i, "kill_surfaces", Vec::new())?;
    }
    if let Ok(f) = py::import_attr(vm, "renpy.text.font", "free_memory") {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn easy_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let none = kwb(vm, arg(args, kw, 1, "none"), false)?;
    let name = if none { "displayable" } else { "displayable_or_none" };
    let f = py::import_attr(vm, "renpy.easy", name)?;
    vm.call(&f, vec![args[0].clone()], Vec::new())
}

fn iface_call(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let i = iface(vm)?;
    vm.call_method(&i, name, args)
}

fn quit_event(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    iface_call(vm, "quit_event", Vec::new())
}

fn iconify(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    iface_call(vm, "iconify", Vec::new())
}

fn timeout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    iface_call(vm, "timeout", vec![args[0].clone()])
}

fn end_interaction(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[0]) {
        return Ok(Value::None);
    }
    let c = py::import_attr(vm, "renpy.display.core", "EndInteraction")?;
    let e = vm.call(&c, vec![args[0].clone()], Vec::new())?;
    Err(PyErr::Exc(e.obj().cloned().unwrap()))
}

pub(crate) fn shown_window(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let s = sls(vm)?;
    py::set(vm, &s, "shown_window", Value::Bool(true))?;
    Ok(Value::None)
}

fn placement_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let p = py::items(vm, &args[1])?;
    for (i, n) in ["xpos", "ypos", "xanchor", "yanchor", "xoffset", "yoffset", "subpixel"].iter().enumerate() {
        py::set(vm, this, n, p.get(i).cloned().unwrap_or(Value::None))?;
    }
    Ok(Value::None)
}

fn pair(vm: &mut Vm, this: &Value, a: &str, b: &str) -> PyResult<Value> {
    let x = py::get(vm, this, a)?;
    let y = py::get(vm, this, b)?;
    Ok(vm.types.new_tuple(vec![x, y]))
}

fn placement_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pair(vm, &args[0], "xpos", "ypos")
}

fn placement_anchor(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pair(vm, &args[0], "xanchor", "yanchor")
}

fn placement_offset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pair(vm, &args[0], "xoffset", "yoffset")
}

fn get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = vm.call_method(&args[0], "get_placement", Vec::new())?;
    let c = py::import_attr(vm, "renpy.exports", "placement")?;
    vm.call(&c, vec![p], Vec::new())
}

fn get_image_bounds(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = py::string(vm, &args[0])?;
    let tag = py::s(vm, tag.split_whitespace().next().unwrap_or(""));
    let mut w = arg_or(args, kw, 1, "width", Value::None);
    let mut h = arg_or(args, kw, 2, "height", Value::None);
    let layer = default_layer(vm, &arg_or(args, kw, 3, "layer", Value::None), &tag, false)?;
    if py::is_none(&w) {
        w = py::config(vm, "screen_width")?;
    }
    if py::is_none(&h) {
        h = py::config(vm, "screen_height")?;
    }
    let s = sls(vm)?;
    vm.call_method(&s, "get_image_bounds", vec![layer, tag, w, h])
}

fn is_pixel_opaque(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = super::super::easy::displayable(vm, &args[0], &Value::None, false)?;
    let w = py::f64_of(vm, &args[1])?;
    let h = py::f64_of(vm, &args[2])?;
    let st = py::f64_of(vm, &args[3])?;
    let at = py::f64_of(vm, &args[4])?;
    let r = super::super::render::render(vm, &d, w, h, st, at)?;
    let o = vm.call_method(&r, "is_pixel_opaque", vec![args[5].clone(), args[6].clone()])?;
    Ok(Value::Bool(py::truthy(vm, &o)?))
}

fn draw(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display", "draw")
}

fn get_renderer_info(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = draw(vm)?;
    py::get(vm, &d, "info")
}

fn display_reset(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    py::set(vm, &i, "display_reset", Value::Bool(true))?;
    Ok(Value::None)
}

fn get_physical_size(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = draw(vm)?;
    vm.call_method(&d, "get_physical_size", Vec::new())
}

fn set_physical_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = prefs(vm)?;
    py::set(vm, &p, "fullscreen", Value::Bool(false))?;
    // The host owns the window; the renderer is not resizable.
    let _ = args;
    Ok(Value::None)
}

fn get_image_load_log(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_list(Vec::new()))
}

fn get_mouse_pos(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = draw(vm)?;
    vm.call_method(&d, "get_mouse_pos", Vec::new())
}

fn set_mouse_pos(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let dur = arg_or(args, kw, 2, "duration", Value::Int(0));
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    vm.call_method(&i, "set_mouse_pos", vec![args[0].clone(), args[1].clone(), dur])
}

fn cancel_gesture(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let r = py::import_attr(vm, "renpy.display.gesture", "recognizer")?;
    vm.call_method(&r, "cancel", Vec::new())
}

fn add_layer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = args[0].clone();
    let above = arg_or(args, kw, 1, "above", Value::None);
    let below = arg_or(args, kw, 2, "below", Value::None);
    let menu_clear = kwb(vm, arg(args, kw, 3, "menu_clear"), true)?;
    let sticky = arg_or(args, kw, 4, "sticky", Value::None);
    let layers = py::config(vm, "layers")?;
    if ops::contains(vm, &layers, &layer)? {
        return Ok(Value::None);
    }
    let find = |vm: &mut Vm, l: &Value| -> PyResult<Option<i64>> {
        for (i, x) in py::items(vm, &layers)?.iter().enumerate() {
            if ops::eq(vm, x, l)? {
                return Ok(Some(i as i64));
            }
        }
        Ok(None)
    };
    let index = if !py::is_none(&above) && !py::is_none(&below) {
        return Err(py::error(vm, "The above and below arguments to renpy.add_layer are mutually exclusive."));
    } else if !py::is_none(&above) {
        match find(vm, &above)? {
            Some(i) => i + 1,
            None => {
                let l = py::string(vm, &above)?;
                return Err(py::error(vm, format!("Layer '{l}' does not exist.")));
            }
        }
    } else if !py::is_none(&below) {
        match find(vm, &below)? {
            Some(i) => i,
            None => {
                let l = py::string(vm, &below)?;
                return Err(py::error(vm, format!("Layer '{l}' does not exist.")));
            }
        }
    } else {
        return Err(py::error(vm, "The renpy.add_layer function requires either the above or below argument."));
    };
    vm.call_method(&layers, "insert", vec![Value::Int(index), layer.clone()])?;
    if menu_clear {
        let mcl = py::config(vm, "menu_clear_layers")?;
        ops::list_append(vm, &mcl, layer.clone())?;
    }
    let sl = py::config(vm, "sticky_layers")?;
    if py::truthy(vm, &sticky)? || (py::is_none(&sticky) && py::truthy(vm, &sl)?) {
        ops::list_append(vm, &sl, layer)?;
    }
    Ok(Value::None)
}

fn maximum_framerate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    if !py::is_none(&i) {
        return vm.call_method(&i, "maximum_framerate", vec![args[0].clone()]);
    }
    let core = py::import(vm, "renpy.display.core")?;
    if py::is_none(&args[0]) {
        py::set(vm, &core, "initial_maximum_framerate", Value::Int(0))?;
    } else {
        let cur = py::get(vm, &core, "initial_maximum_framerate")?;
        let v = py::f64_of(vm, &cur)?.max(py::f64_of(vm, &args[0])?);
        py::set(vm, &core, "initial_maximum_framerate", Value::Float(v))?;
    }
    Ok(Value::None)
}

fn is_start_interact(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    py::get_or_none(vm, &i, "start_interact")
}

fn get_refresh_rate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = py::f64_of(vm, &arg_or(args, kw, 0, "precision", Value::Int(5)))?;
    Ok(Value::Float((60. / p).round() * p))
}

fn get_adjustment(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "get_adjustment", Vec::new())
}

fn get_texture_size(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = draw(vm)?;
    vm.call_method(&d, "get_texture_size", Vec::new())
}

fn get_zorder_list(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = sls(vm)?;
    vm.call_method(&s, "get_zorder_list", vec![args[0].clone()])
}

fn change_zorder(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = sls(vm)?;
    vm.call_method(&s, "change_zorder", args.to_vec())
}

fn is_mouse_visible(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    if py::is_none(&i) {
        return Ok(Value::Bool(true));
    }
    let mf = py::get_or_none(vm, &i, "mouse_focused")?;
    if !py::truthy(vm, &mf)? {
        return Ok(Value::Bool(false));
    }
    vm.call_method(&i, "is_mouse_visible", Vec::new())
}

fn get_mouse_name(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    if py::is_none(&i) {
        return Ok(py::s(vm, "default"));
    }
    let inter = arg_or(args, kw, 0, "interaction", Value::Bool(false));
    py::method_kw(vm, &i, "get_mouse_name", Vec::new(), vec![("interaction".into(), inter)])
}

fn set_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 2, "layer", py::s(vm, "screens"));
    let f = py::import(vm, "renpy.display.focus")?;
    let t = vm.types.new_tuple(vec![args[0].clone(), args[1].clone(), layer]);
    py::set(vm, &f, "override", t)?;
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    py::set(vm, &i, "last_event", Value::None)?;
    restart_interaction(vm, &[], &[])
}

fn clear_retain(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 0, "layer", py::s(vm, "screens"));
    let prefix = arg_or(args, kw, 1, "prefix", py::s(vm, "_retain"));
    let prefix = py::string(vm, &prefix)?;
    let tags = get_showing_tags(vm, &[layer], &[])?;
    for t in ops::collect(vm, &tags)? {
        if py::string(vm, &t)?.starts_with(&prefix) {
            ex(vm, "hide_screen", vec![t], Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn can_fullscreen(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display", "can_fullscreen")
}

fn render_to_surface(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut d = args[0].clone();
    let w = arg_or(args, kw, 1, "width", Value::None);
    let h = arg_or(args, kw, 2, "height", Value::None);
    let st = arg_or(args, kw, 3, "st", Value::Float(0.));
    let at = arg_or(args, kw, 4, "at", Value::None);
    if super::super::render::render_data(&d).is_none() {
        let w = if py::is_none(&w) { py::config_f64(vm, "screen_width")? } else { py::f64_of(vm, &w)? };
        let h = if py::is_none(&h) { py::config_f64(vm, "screen_height")? } else { py::f64_of(vm, &h)? };
        let st = py::f64_of(vm, &st)?;
        let at = if py::is_none(&at) { st } else { py::f64_of(vm, &at)? };
        let dd = super::super::easy::displayable(vm, &d, &Value::None, false)?;
        d = super::super::render::render(vm, &dd, w, h, st, at)?;
    }
    let dr = draw(vm)?;
    vm.call_method(&dr, "screenshot", vec![d])
}

fn render_to_file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let _ = render_to_surface(vm, args, kw)?;
    Err(py::error(vm, "renpy.render_to_file is not available: rendered surfaces cannot be written by this runtime."))
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("count_displayables_in_layer", count_displayables_in_layer),
        ("image", image),
        ("copy_images", copy_images),
        ("default_layer", f_default_layer),
        ("can_show", can_show),
        ("showing", showing),
        ("get_showing_tags", get_showing_tags),
        ("get_hidden_tags", get_hidden_tags),
        ("get_attributes", get_attributes),
        ("clear_attributes", clear_attributes),
        ("_find_image", f_find_image),
        ("predict_show", predict_show),
        ("set_tag_attributes", set_tag_attributes),
        ("show", show),
        ("hide", hide),
        ("scene", scene),
        ("toggle_fullscreen", toggle_fullscreen),
        ("take_screenshot", take_screenshot),
        ("screenshot", screenshot),
        ("screenshot_to_bytes", screenshot_to_bytes),
        ("transition", transition),
        ("get_transition", get_transition),
        ("get_ongoing_transition", get_ongoing_transition),
        ("restart_interaction", restart_interaction),
        ("force_full_redraw", noop),
        ("image_size", image_size),
        ("get_at_list", get_at_list),
        ("show_layer_at", show_layer_at),
        ("layer_at_list", show_layer_at),
        ("free_memory", free_memory),
        ("flush_cache_file", noop),
        ("easy_displayable", easy_displayable),
        ("quit_event", quit_event),
        ("iconify", iconify),
        ("timeout", timeout),
        ("end_interaction", end_interaction),
        ("shown_window", shown_window),
        ("get_placement", get_placement),
        ("get_image_bounds", get_image_bounds),
        ("is_pixel_opaque", is_pixel_opaque),
        ("get_renderer_info", get_renderer_info),
        ("display_reset", display_reset),
        ("get_physical_size", get_physical_size),
        ("set_physical_size", set_physical_size),
        ("reset_physical_size", set_physical_size),
        ("get_image_load_log", get_image_load_log),
        ("get_mouse_pos", get_mouse_pos),
        ("set_mouse_pos", set_mouse_pos),
        ("cancel_gesture", cancel_gesture),
        ("add_layer", add_layer),
        ("maximum_framerate", maximum_framerate),
        ("is_start_interact", is_start_interact),
        ("get_refresh_rate", get_refresh_rate),
        ("get_adjustment", get_adjustment),
        ("get_texture_size", get_texture_size),
        ("get_zorder_list", get_zorder_list),
        ("change_zorder", change_zorder),
        ("is_mouse_visible", is_mouse_visible),
        ("get_mouse_name", get_mouse_name),
        ("set_focus", set_focus),
        ("clear_retain", clear_retain),
        ("can_fullscreen", can_fullscreen),
        ("render_to_surface", render_to_surface),
        ("render_to_file", render_to_file),
    ]
}

pub fn values(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let sl = py::import_attr(vm, "renpy.display.scenelists", "scene_lists")?;
    set(vm, "scene_lists", sl);
    let ro = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let mk = |vm: &mut Vm, n: &'static str, f: NativeFn| -> PyResult<Value> {
        let b = vm.types.new_builtin(n, f);
        py::property(vm, b, None)
    };
    let pos = mk(vm, "pos", placement_pos)?;
    let anchor = mk(vm, "anchor", placement_anchor)?;
    let offset = mk(vm, "offset", placement_offset)?;
    let pl = py::class(vm, "renpy.exports", "placement", &[ro.clone()], &[("__init__", placement_init)], &[("pos", pos), ("anchor", anchor), ("offset", offset)])?;
    set(vm, "placement", Value::Obj(pl));
    for (n, module, attr) in [("Render", "renpy.display.render", "Render"), ("render", "renpy.display.render", "render"), ("IgnoreEvent", "renpy.display.core", "IgnoreEvent"), ("redraw", "renpy.display.render", "redraw")] {
        let v = py::import_attr(vm, module, attr)?;
        set(vm, n, v);
    }
    let d = py::import_attr(vm, "renpy.display.displayable", "Displayable")?;
    let dc = py::class(vm, "renpy.exports", "Displayable", &[d, ro.clone()], &[], &[])?;
    set(vm, "Displayable", Value::Obj(dc));
    let c = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let rl = py::import_attr(vm, "renpy.revertable", "RevertableList")?;
    let cc = py::class(vm, "renpy.exports", "Container", &[c, ro], &[], &[("_list_type", rl)])?;
    set(vm, "Container", Value::Obj(cc));
    Ok(())
}
