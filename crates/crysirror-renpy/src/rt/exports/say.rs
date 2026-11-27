// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.sayexports` (Ren'Py 8.3, MIT): say, prediction and
//! scrying of dialogue, reshowing, side images and dialogue counts.
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, object::NativeFn};
use std::rc::Rc;

fn store(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, "store")
}

fn tqd_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let key = &args[1];
    let d = super::super::python::store_dict(vm, "store")?;
    let k = py::string(vm, key)?;
    let kv = py::s(vm, &k);
    if let Some(rv) = crysirror_python::ops::dict_get(vm, d.obj().unwrap(), &kv)? {
        if let Some(s) = rv.as_str() {
            return Ok(py::s(vm, &s.replace('{', "{{")));
        }
        return Ok(rv);
    }
    if py::config_bool(vm, "debug")? {
        return Err(py::error(vm, format!("During an interpolation, '{k}' was not found as a variable.")));
    }
    Ok(py::s(vm, &format!("<{k} unbound>")))
}

fn narrator(vm: &mut Vm) -> PyResult<Value> {
    let s = store(vm)?;
    py::get(vm, &s, "narrator")
}

fn predict_say(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (mut who, what) = (args[0].clone(), args[1].clone());
    if py::is_none(&who) {
        who = narrator(vm)?;
    }
    if who.as_str().is_some() {
        let s = store(vm)?;
        let f = py::get(vm, &s, "predict_say")?;
        return vm.call(&f, vec![who, what], Vec::new());
    }
    if let Some(p) = py::get_opt(vm, &who, "predict")? {
        if py::truthy(vm, &p)? {
            vm.call(&p, vec![what], Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn scry_say(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (who, what, scry) = (&args[0], &args[1], &args[2]);
    let interacts = vm.call_method(who, "will_interact", Vec::new()).unwrap_or(Value::Bool(true));
    py::set(vm, scry, "interacts", interacts)?;
    let et = match vm.call_method(who, "get_extend_text", vec![what.clone()]) {
        Ok(v) => v,
        Err(_) => py::import_attr(vm, "renpy.ast", "DoesNotExtend")?,
    };
    py::set(vm, scry, "extend_text", et)?;
    Ok(Value::None)
}

pub(crate) fn say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut who = args[0].clone();
    let mut what = args[1].clone();
    let mut a = args[2..].to_vec();
    let mut k: Vec<(Rc<str>, Value)> = kw.to_vec();
    if py::config_bool(vm, "old_substitutions")? {
        let tqd = py::import_attr(vm, "renpy.exports", "tag_quoting_dict")?;
        what = crysirror_python::ops::binary_op(vm, &what, &tqd, crysirror_python::ast::BinOp::Mod, false, false)?;
    }
    if py::is_none(&who) {
        who = narrator(vm)?;
    }
    let sac = py::config(vm, "say_arguments_callback")?;
    if !py::is_none(&sac) {
        let mut ca = vec![who.clone()];
        ca.extend(a.clone());
        let r = vm.call(&sac, ca, k.clone())?;
        let p = py::items(vm, &r)?;
        a = py::items(vm, &p[0])?;
        k = py::str_items(vm, &p[1])?;
    }
    let mut ca = vec![what];
    ca.extend(a);
    if who.as_str().is_some() {
        let s = store(vm)?;
        let f = py::get(vm, &s, "say")?;
        let mut full = vec![who];
        full.extend(ca);
        return vm.call(&f, full, k);
    }
    vm.call(&who, ca, k)
}

fn do_reshow_say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut who = args[0].clone();
    let what = args[1].clone();
    let interact = match args.get(2) {
        Some(v) => v.clone(),
        None => kw.iter().position(|(k, _)| &**k == "interact").map(|i| kw.remove(i).1).unwrap_or(Value::Bool(false)),
    };
    if !py::is_none(&who) {
        who = super::super::python::eval(vm, &who, &Value::None, &Value::None)?;
    }
    let mut a = vec![who, what];
    a.extend(args.iter().skip(3).cloned());
    kw.push(("interact".into(), interact));
    say(vm, &a, &kw)
}

fn get_reshow_say(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let lk = store_get(vm, "_last_say_kwargs")?;
    let mut k = if py::is_none(&lk) { Vec::new() } else { py::str_items(vm, &lk)? };
    for (n, v) in kw {
        k.retain(|(x, _)| x != n);
        k.push((n.clone(), v.clone()));
    }
    let crs = py::import_attr(vm, "renpy.exports", "curried_do_reshow_say")?;
    let who = store_get(vm, "_last_say_who")?;
    let what = store_get(vm, "_last_say_what")?;
    let args = store_get(vm, "_last_say_args")?;
    vm.call(&crs, vec![who, what, args], k)
}

fn reshow_say(vm: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let f = get_reshow_say(vm, &[], &[])?;
    vm.call(&f, Vec::new(), kw.to_vec())
}

fn get_say_attributes(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get_or_none(vm, &c, "say_attributes")
}

fn get_side_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let prefix = arg_or(args, kw, 0, "prefix_tag", Value::None);
    let image_tag = arg_or(args, kw, 1, "image_tag", Value::None);
    let mut not_showing = arg_or(args, kw, 2, "not_showing", Value::None);
    let layer = arg_or(args, kw, 3, "layer", Value::None);
    if py::is_none(&not_showing) {
        not_showing = py::config(vm, "side_image_only_not_showing")?;
    }
    let c = ctx(vm)?;
    let images = py::get(vm, &c, "images")?;
    let attrs = if !py::is_none(&image_tag) {
        let il = ex(vm, "default_layer", vec![layer.clone(), image_tag.clone()], Vec::new())?;
        let a = vm.call_method(&images, "get_attributes", vec![il, image_tag.clone()])?;
        let mut v = vec![image_tag];
        v.extend(py::items(vm, &a)?);
        if py::config_bool(vm, "side_image_requires_attributes")? && v.len() < 2 {
            return Ok(Value::None);
        }
        vm.types.new_tuple(v)
    } else {
        store_get(vm, "_side_image_attributes")?
    };
    if !py::truthy(vm, &attrs)? {
        return Ok(Value::None);
    }
    let al = ex(vm, "default_layer", vec![layer, attrs.clone()], Vec::new())?;
    let items = py::items(vm, &attrs)?;
    let first = vm.types.new_tuple(vec![items[0].clone()]);
    if py::truthy(vm, &not_showing)? {
        let s = vm.call_method(&images, "showing", vec![al, first])?;
        if py::truthy(vm, &s)? {
            return Ok(Value::None);
        }
    }
    let required = vm.types.new_list(vec![items[0].clone()]);
    let optional = vm.types.new_list(items[1..].to_vec());
    vm.call_method(&images, "choose_image", vec![prefix, required, optional, Value::None])
}

fn count_dialogue_blocks(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::import_attr(vm, "renpy.game", "script")?;
    let t = py::get(vm, &s, "translator")?;
    vm.call_method(&t, "count_translates", Vec::new())
}

fn count_seen_dialogue_blocks(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "seen_translates_count")
}

fn count_newly_seen_dialogue_blocks(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "new_translates_count")
}

fn substitute(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = args[0].clone();
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    let translate = arg_or(args, kw, 2, "translate", Value::Bool(true));
    let f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    let r = vm.call(&f, vec![s], vec![("scope".into(), scope), ("translate".into(), translate)])?;
    crysirror_python::ops::getitem(vm, &r, Value::Int(0))
}

fn get_say_image_tag(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let a = store_get(vm, "_side_image_attributes")?;
    if py::is_none(&a) {
        return Ok(Value::None);
    }
    crysirror_python::ops::getitem(vm, &a, Value::Int(0))
}

fn ls_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "_who", arg_or(args, kw, 1, "who", Value::None))?;
    py::set(vm, this, "what", arg_or(args, kw, 2, "what", Value::None))?;
    py::set(vm, this, "args", arg_or(args, kw, 3, "args", Value::None))?;
    py::set(vm, this, "kwargs", arg_or(args, kw, 4, "kwargs", Value::None))?;
    Ok(Value::None)
}

fn ls_who(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let w = py::get_or_none(vm, &args[0], "_who")?;
    ex(vm, "eval_who", vec![w], Vec::new())
}

fn last_say(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.exports", "LastSay")?;
    let who = store_get(vm, "_last_say_who")?;
    let what = store_get(vm, "_last_say_what")?;
    let a = store_get(vm, "_last_say_args")?;
    let k = store_get(vm, "_last_say_kwargs")?;
    vm.call(&c, Vec::new(), vec![("who".into(), who), ("what".into(), what), ("args".into(), a), ("kwargs".into(), k)])
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("predict_say", predict_say),
        ("scry_say", scry_say),
        ("say", say),
        ("do_reshow_say", do_reshow_say),
        ("get_reshow_say", get_reshow_say),
        ("reshow_say", reshow_say),
        ("get_say_attributes", get_say_attributes),
        ("get_side_image", get_side_image),
        ("count_dialogue_blocks", count_dialogue_blocks),
        ("count_seen_dialogue_blocks", count_seen_dialogue_blocks),
        ("count_newly_seen_dialogue_blocks", count_newly_seen_dialogue_blocks),
        ("substitute", substitute),
        ("get_say_image_tag", get_say_image_tag),
        ("last_say", last_say),
    ]
}

pub fn values(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let obj = Value::Obj(vm.types.object.clone());
    let tqd = py::class(vm, "renpy.exports", "TagQuotingDict", &[obj.clone()], &[("__getitem__", tqd_getitem)], &[])?;
    let inst = vm.call(&Value::Obj(tqd.clone()), Vec::new(), Vec::new())?;
    set(vm, "TagQuotingDict", Value::Obj(tqd));
    set(vm, "tag_quoting_dict", inst);
    let who = vm.types.new_builtin("who", ls_who);
    let who = py::property(vm, who, None)?;
    let ls = py::class(vm, "renpy.exports", "LastSay", &[obj], &[("__init__", ls_init)], &[("who", who)])?;
    set(vm, "LastSay", Value::Obj(ls));
    let curry = py::import_attr(vm, "renpy.curry", "curry")?;
    let drs = py::get_raw(m, "do_reshow_say");
    let crs = vm.call(&curry, vec![drs], Vec::new())?;
    set(vm, "curried_do_reshow_say", crs);
    Ok(())
}
