// SPDX-License-Identifier: MPL-2.0
//! Image file access for displayables: sizes and alpha from the shared
//! image service, through the game's asset capability.
use super::render::{TexKind, Texture};
use crysirror_python::Vm;
use std::{cell::RefCell, collections::HashMap};

#[derive(Default)]
pub struct ImageCache {
    pub sizes: HashMap<String, [u32; 2]>,
}

fn cache(vm: &Vm) -> std::rc::Rc<RefCell<ImageCache>> {
    vm.ext_get::<std::rc::Rc<RefCell<ImageCache>>>()
        .expect("image cache")
        .clone()
}

/// Pixel size of a game image file.
pub fn image_size(vm: &Vm, name: &str) -> Result<[u32; 2], String> {
    if let Some(s) = cache(vm).borrow_mut().sizes.get(name) {
        return Ok(*s);
    }
    let asset =
        super::asset_path(vm, name).ok_or_else(|| format!("Couldn't find file '{name}'."))?;
    let images = super::host(vm)
        .borrow_mut()
        .images
        .clone()
        .ok_or("image service unavailable")?;
    let size = images.size(&asset).map_err(|e| format!("{name}: {e}"))?;
    cache(vm).borrow_mut().sizes.insert(name.to_owned(), size);
    Ok(size)
}

/// Alpha of a texture pixel, or None when the decoded pixels are not
/// available from the image service.
pub fn texture_alpha(vm: &Vm, t: &Texture, x: f64, y: f64) -> Option<u8> {
    let images = super::host(vm).borrow_mut().images.clone()?;
    let asset = match &t.kind {
        TexKind::Asset(name) => super::asset_path(vm, name)?,
        TexKind::Published(p) => p.clone(),
        _ => return None,
    };
    if images.opaque(&asset).ok()? {
        return Some(255);
    }
    let _ = (x, y);
    None
}

/// Published image of a `Render.canvas()` drawing.
pub fn canvas_asset(_t: &Texture) -> Option<String> {
    None
}

pub fn install(vm: &mut Vm) {
    vm.ext_set(std::rc::Rc::new(RefCell::new(ImageCache::default())));
}

// `renpy.display.image` -------------------------------------------------------------------
// After Ren'Py 8.3 `display/image.py` (MIT). The registry dictionaries are
// module globals so script code that reads them sees live data.

use super::py::{self, arg_or};
use crysirror_python::{ObjRef, PyResult, Value, builtins::Kw, ops};

const MODULE: &str = "renpy.display.image";

fn module_attr(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, name)
}

fn name_tuple(vm: &mut Vm, name: &Value) -> PyResult<Vec<Value>> {
    Ok(match name.as_str() {
        Some(s) => s.split_whitespace().map(|p| vm.types.new_str(p)).collect(),
        None => py::items(vm, name)?,
    })
}

fn strs(v: &[Value]) -> Vec<String> {
    v.iter()
        .map(|x| x.as_str().unwrap_or("").to_owned())
        .collect()
}

fn tuple_of(vm: &Vm, v: &[String]) -> Value {
    vm.types
        .new_tuple(v.iter().map(|s| vm.types.new_str(s)).collect())
}

fn f_list_images(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    module_attr(vm, "image_names")
}

fn tag_attrs(vm: &mut Vm, tag: &Value) -> PyResult<Vec<(Vec<String>, Value)>> {
    let ia = module_attr(vm, "image_attributes")?;
    let d = ops::getitem(vm, &ia, tag.clone())?;
    let mut out = Vec::new();
    for (k, v) in ops::mapping_items(vm, &d)? {
        let k = py::items(vm, &k)?;
        out.push((strs(&k), v));
    }
    Ok(out)
}

fn f_get_available_image_tags(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ia = module_attr(vm, "image_attributes")?;
    let mut out = Vec::new();
    for (k, v) in ops::mapping_items(vm, &ia)? {
        if ops::truthy(vm, &v)? {
            out.push(k);
        }
    }
    Ok(vm.types.new_list(out))
}

fn f_get_available_image_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = arg_or(args, kw, 0, "tag", Value::None);
    let attrs = arg_or(args, kw, 1, "attributes", vm.types.empty_tuple.clone());
    let attrs = strs(&py::items(vm, &attrs)?);
    let ia = module_attr(vm, "image_attributes")?;
    if !ops::contains(vm, &ia, &tag)? {
        return Ok(vm.types.new_list(Vec::new()));
    }
    let mut out = Vec::new();
    for (at, _) in tag_attrs(vm, &tag)? {
        if attrs.iter().all(|a| at.contains(a)) {
            out.push(tuple_of(vm, &at));
        }
    }
    Ok(vm.types.new_list(out))
}

fn f_get_tag_method(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tag = args.first().cloned().unwrap_or(Value::None);
    let method = args
        .get(1)
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    let images = module_attr(vm, "images")?;
    let key = vm.types.new_tuple(vec![tag]);
    let ti = vm.call_method(&images, "get", vec![key, Value::None])?;
    if py::is_none(&ti) {
        return Ok(Value::None);
    }
    py::get_or_none(vm, &ti, &method)
}

fn choose_attributes(
    vm: &mut Vm,
    d: &Value,
    tag: &Value,
    required: &[String],
    optional: Option<&[String]>,
) -> PyResult<Option<Option<Vec<String>>>> {
    let Some(ca) = py::get_opt(vm, d, "_choose_attributes")? else {
        return Ok(None);
    };
    if py::is_none(&ca) {
        return Ok(None);
    }
    let r = vm
        .types
        .new_list(required.iter().map(|s| vm.types.new_str(s)).collect());
    let o = match optional {
        Some(o) => vm
            .types
            .new_list(o.iter().map(|s| vm.types.new_str(s)).collect()),
        None => Value::None,
    };
    let res = vm.call(&ca, vec![tag.clone(), r, o], Vec::new())?;
    if py::is_none(&res) {
        return Ok(Some(None));
    }
    Ok(Some(Some(strs(&py::items(vm, &res)?))))
}

/// `check_image_attributes(tag, attributes)`.
fn f_check_image_attributes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tag = args.first().cloned().unwrap_or(Value::None);
    let attrs = strs(&py::items(
        vm,
        &args.get(1).cloned().unwrap_or(Value::None),
    )?);
    let negative: Vec<String> = attrs
        .iter()
        .filter(|a| a.starts_with('-'))
        .cloned()
        .collect();
    let positive: Vec<String> = attrs
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();
    let mut l: Vec<Vec<String>> = Vec::new();
    for (at, d) in tag_attrs(vm, &tag)? {
        let remainder: Vec<String> = positive
            .iter()
            .filter(|a| !at.contains(a))
            .cloned()
            .collect();
        match choose_attributes(vm, &d, &tag, &remainder, None)? {
            Some(Some(chosen)) => {
                let mut v = at.clone();
                v.extend(chosen);
                l.push(v);
            }
            Some(None) => {}
            None => {
                if remainder.is_empty() {
                    l.push(at);
                }
            }
        }
    }
    if !negative.is_empty() {
        let negated: Vec<&str> = negative.iter().map(|n| &n[1..]).collect();
        l.retain(|i| !i.iter().any(|a| negated.contains(&a.as_str())));
    }
    for i in &l {
        if i.len() == positive.len() {
            let mut v = i.clone();
            v.extend(negative.iter().cloned());
            return Ok(tuple_of(vm, &v));
        }
    }
    if l.len() != 1 {
        return Ok(Value::None);
    }
    let mut v = l[0].clone();
    v.extend(negative);
    Ok(tuple_of(vm, &v))
}

/// `get_ordered_image_attributes(tag, attributes=(), sort=None)`.
fn f_get_ordered_image_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = arg_or(args, kw, 0, "tag", Value::None);
    let attributes = strs(&py::items(
        vm,
        &arg_or(args, kw, 1, "attributes", vm.types.empty_tuple.clone()),
    )?);
    let sort = arg_or(args, kw, 2, "sort", Value::None);
    let mut entries = tag_attrs(vm, &tag)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut sequences: Vec<Vec<String>> = Vec::new();
    let mut count: Vec<(String, usize, f64)> = Vec::new();
    for (attrs, d) in entries {
        if let Some(la) = py::get_opt(vm, &d, "_list_attributes")?.filter(|v| !py::is_none(v)) {
            let rest: Vec<Value> = attributes
                .iter()
                .filter(|a| !attrs.contains(a))
                .map(|a| vm.types.new_str(a))
                .collect();
            let rest = vm.types.new_list(rest);
            let r = vm.call(&la, vec![tag.clone(), rest], Vec::new())?;
            let mut seq = attrs.clone();
            seq.extend(strs(&py::items(vm, &r)?));
            if !attributes.iter().all(|a| seq.contains(a)) {
                continue;
            }
            sequences.push(seq);
        } else {
            if !attributes.iter().all(|a| attrs.contains(a)) {
                continue;
            }
            for (i, a) in attrs.iter().enumerate() {
                match count.iter_mut().find(|c| &c.0 == a) {
                    Some(c) => {
                        c.1 += 1;
                        c.2 += i as f64;
                    }
                    None => count.push((a.clone(), 1, i as f64)),
                }
            }
        }
    }
    if py::is_none(&sort) {
        let mut all: Vec<String> = count.iter().map(|c| c.0.clone()).collect();
        for s in &sequences {
            for a in s {
                if !all.contains(a) {
                    all.push(a.clone());
                }
            }
        }
        let l = all.iter().map(|s| vm.types.new_str(s)).collect();
        return Ok(vm.types.new_list(l));
    }
    let mut after: Vec<(String, Vec<String>)> = Vec::new();
    for mut s in sequences {
        while !s.is_empty() {
            let j = s.remove(0);
            if !after.iter().any(|a| a.0 == j) {
                after.push((j.clone(), Vec::new()));
            }
            for k in &s {
                match after.iter_mut().find(|a| &a.0 == k) {
                    Some(e) => {
                        if !e.1.contains(&j) {
                            e.1.push(j.clone());
                        }
                    }
                    None => after.push((k.clone(), vec![j.clone()])),
                }
            }
        }
    }
    let mut sort_key = |vm: &mut Vm, a: &str| -> PyResult<Value> {
        let s = vm.types.new_str(a);
        let k = vm.call(&sort, vec![s.clone()], Vec::new())?;
        Ok(vm.types.new_tuple(vec![k, s]))
    };
    let mut rv: Vec<String> = Vec::new();
    while !after.is_empty() {
        let minc = after.iter().map(|a| a.1.len()).min().unwrap_or(0);
        let ready: Vec<String> = after
            .iter()
            .filter(|a| a.1.len() == minc)
            .map(|a| a.0.clone())
            .collect();
        after.retain(|a| !ready.contains(&a.0));
        for a in after.iter_mut() {
            a.1.retain(|x| !ready.contains(x));
        }
        let mut keyed = Vec::new();
        for r in &ready {
            keyed.push((sort_key(vm, r)?, r.clone()));
        }
        let mut err = None;
        keyed.sort_by(|a, b| {
            match ops::compare(vm, &a.0, &b.0, crysirror_python::ast::CmpOp::Lt) {
                Ok(v) if ops::truthy(vm, &v).unwrap_or(false) => std::cmp::Ordering::Less,
                Ok(_) => std::cmp::Ordering::Greater,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        rv.extend(keyed.into_iter().map(|k| k.1));
    }
    let mut tail = Vec::new();
    for (a, n, total) in &count {
        if !rv.contains(a) {
            let s = vm.types.new_str(a);
            let k = vm.call(&sort, vec![s], Vec::new())?;
            tail.push(((total / *n as f64).floor(), k, a.clone()));
        }
    }
    let mut err = None;
    tail.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(
                || match ops::compare(vm, &a.1, &b.1, crysirror_python::ast::CmpOp::Lt) {
                    Ok(v) if ops::truthy(vm, &v).unwrap_or(false) => std::cmp::Ordering::Less,
                    Ok(_) => a.2.cmp(&b.2),
                    Err(e) => {
                        err = Some(e);
                        std::cmp::Ordering::Equal
                    }
                },
            )
    });
    if let Some(e) = err {
        return Err(e);
    }
    rv.extend(tail.into_iter().map(|t| t.2));
    let l = rv.iter().map(|s| vm.types.new_str(s)).collect();
    Ok(vm.types.new_list(l))
}

/// `register_image(name, d)`.
pub fn register_image(vm: &mut Vm, name: &[Value], d: Value) -> PyResult<()> {
    let images = module_attr(vm, "images")?;
    let key = vm.types.new_tuple(name.to_vec());
    ops::setitem(vm, &images, key, d.clone())?;
    let ia = module_attr(vm, "image_attributes")?;
    let tag = name.first().cloned().unwrap_or(Value::None);
    let td = ops::getitem(vm, &ia, tag)?;
    let rest = vm.types.new_tuple(name.get(1..).unwrap_or(&[]).to_vec());
    ops::setitem(vm, &td, rest, d)?;
    let names = module_attr(vm, "image_names")?;
    let joined = strs(name).join(" ");
    let j = vm.types.new_string(joined);
    ops::list_append(vm, &names, j)?;
    Ok(())
}

fn f_register_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = args.first().cloned().unwrap_or(Value::None);
    let name = name_tuple(vm, &name)?;
    register_image(vm, &name, args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

pub fn image_exists(vm: &mut Vm, name: &Value, exact: bool) -> PyResult<bool> {
    let mut n = name_tuple(vm, name)?;
    let images = module_attr(vm, "images")?;
    while !n.is_empty() {
        let key = vm.types.new_tuple(n.clone());
        if ops::contains(vm, &images, &key)? {
            return Ok(true);
        }
        if exact {
            return Ok(false);
        }
        n.pop();
    }
    Ok(false)
}

fn f_image_exists(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", Value::None);
    let exact = match py::arg(args, kw, 1, "exact") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    Ok(Value::Bool(image_exists(vm, &name, exact)?))
}

fn f_get_registered_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = args.first().cloned().unwrap_or(Value::None);
    let n = name_tuple(vm, &name)?;
    let images = module_attr(vm, "images")?;
    let key = vm.types.new_tuple(n);
    vm.call_method(&images, "get", vec![key])
}

fn wrap_render(vm: &mut Vm, child: &Value, args: &[Value]) -> PyResult<Value> {
    let (w, h, st, at) = super::layout::rargs(vm, args)?;
    let rend = super::render::render(vm, child, w, h, st, at)?;
    let (rw, rh) = super::layout::size(&rend);
    let rv = super::layout::new(vm, rw, rh);
    super::layout::blit(vm, &rv, &rend, 0., 0.)?;
    Ok(rv)
}

fn f_wrap_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let child = args.first().cloned().unwrap_or(Value::None);
    wrap_render(vm, &child, args)
}

fn error_text(vm: &mut Vm, msg: String) -> PyResult<Value> {
    let text = py::import_attr(vm, "renpy.text.text", "Text")?;
    let m = vm.types.new_string(msg);
    // Style `_image_error` from 7.8 / 8.3; red text at the origin before.
    let kw = if super::state::rt(vm).profile.since_pair(8, 3) {
        vec![("style".into(), vm.types.new_str("_image_error"))]
    } else {
        let red = vm.types.new_tuple(vec![Value::Int(255), Value::Int(0), Value::Int(0), Value::Int(255)]);
        vec![
            ("color".into(), red),
            ("xanchor".into(), Value::Int(0)),
            ("xpos".into(), Value::Int(0)),
            ("yanchor".into(), Value::Int(0)),
            ("ypos".into(), Value::Int(0)),
        ]
    };
    vm.call(&text, vec![m], kw)
}

// ImageReference -------------------------------------------------------------------------

fn ir_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = arg_or(args, kw, 1, "name", Value::None);
    let props: Vec<_> = kw.iter().filter(|(k, _)| &**k != "name").cloned().collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "name", name)?;
    py::set(vm, this, "target", Value::None)?;
    Ok(Value::None)
}

fn ir_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(1)))?;
    if v < 1 {
        let pt = py::get_or_none(vm, &args[0], "param_target")?;
        let t = py::import_attr(vm, "renpy.display.transform", "Transform")?;
        if t.obj().is_some_and(|c| ops::isinstance(vm, &pt, c)) {
            py::set(vm, &args[0], "old_transform", pt)?;
        }
    }
    Ok(Value::None)
}

fn repr_info_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get(vm, &args[0], "name")?;
    let r = ops::repr(vm, &n)?;
    Ok(vm.types.new_string(r.to_string()))
}

fn ir_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get(vm, &args[0], "name")?;
    Ok(Value::Int(ops::hash(vm, &n)?))
}

fn ir_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = args.get(1).cloned().unwrap_or(Value::None);
    if py::same(&args[0], &o) {
        return Ok(Value::Bool(true));
    }
    let eq = vm.call_method(&args[0], "_equals", vec![o.clone()])?;
    if !ops::truthy(vm, &eq)? {
        return Ok(Value::Bool(false));
    }
    let a = py::get(vm, &args[0], "name")?;
    let b = py::get(vm, &o, "name")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn ensure_target(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let t = py::get_or_none(vm, this, "target")?;
    if py::is_none(&t) {
        vm.call_method(this, "find_target", Vec::new())?;
        return py::get(vm, this, "target");
    }
    Ok(t)
}

fn ir_target(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    vm.call_method(&t, "_target", Vec::new())
}

fn ir_find_target(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = py::get(vm, this, "name")?;
    if super::core::is_displayable(vm, &name) {
        py::set(vm, this, "target", name)?;
        return Ok(Value::Bool(true));
    }
    let full = name_tuple(vm, &name)?;
    let debug = py::config_bool(vm, "debug").unwrap_or(false);
    let err = |vm: &mut Vm, msg: String| -> PyResult<Value> {
        if debug {
            return Err(py::error(vm, msg));
        }
        let t = error_text(vm, msg)?;
        py::set(vm, this, "target", t)?;
        Ok(Value::Bool(false))
    };
    let images = module_attr(vm, "images")?;
    let mut n = full.clone();
    let mut extra: Vec<Value> = Vec::new();
    let mut target = Value::None;
    while !n.is_empty() {
        let key = vm.types.new_tuple(n.clone());
        target = vm.call_method(&images, "get", vec![key, Value::None])?;
        if !py::is_none(&target) {
            break;
        }
        extra.insert(0, n.pop().unwrap());
    }
    if n.is_empty() {
        return err(vm, format!("Image '{}' not found.", strs(&full).join(" ")));
    }
    let da = py::get(vm, this, "_args")?;
    let da_name = py::get_or_none(vm, &da, "name")?;
    let key = vm.types.new_tuple(n.clone());
    if ops::eq(vm, &da_name, &key)? {
        return err(
            vm,
            format!("Image '{}' refers to itself.", strs(&n).join(" ")),
        );
    }
    let da_args = py::get_or_none(vm, &da, "args")?;
    extra.extend(py::items(vm, &da_args).unwrap_or_default());
    let copy = py::get(vm, &da, "copy")?;
    let ea = vm.types.new_tuple(extra);
    let r = vm
        .call(
            &copy,
            Vec::new(),
            vec![("name".into(), key), ("args".into(), ea)],
        )
        .and_then(|a| vm.call_method(&target, "_duplicate", vec![a]));
    match r {
        Ok(t) => py::set(vm, this, "target", t)?,
        Err(e) => {
            if py::config_bool(vm, "raise_image_exceptions").unwrap_or(true) {
                return Err(e);
            }
            let msg = match &e {
                crysirror_python::PyErr::Exc(o) => {
                    ops::str(vm, &Value::Obj(o.clone()))?.to_string()
                }
                _ => "image error".into(),
            };
            return err(vm, msg);
        }
    }
    let t = py::get(vm, this, "target")?;
    let nt = vm.call_method(&t, "_target", Vec::new())?;
    let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
    if tc.obj().is_some_and(|c| ops::isinstance(vm, &nt, c)) {
        let old = py::get_or_none(vm, this, "old_transform")?;
        if !py::is_none(&old) {
            vm.call_method(&nt, "take_state", vec![old])?;
        }
        py::set(vm, this, "old_transform", nt)?;
    } else {
        py::set(vm, this, "old_transform", Value::None)?;
    }
    Ok(Value::Bool(true))
}

fn ir_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = arg_or(args, kw, 1, "args", Value::None);
    if !py::is_none(&a) {
        let x = py::get(vm, &a, "args")?;
        if ops::truthy(vm, &x)? {
            vm.call_method(&a, "extraneous", Vec::new())?;
        }
    }
    let rv = vm.call_method(this, "_copy", vec![a.clone()])?;
    py::set(vm, &rv, "target", Value::None)?;
    let name = py::get(vm, &rv, "name")?;
    if super::core::is_displayable(vm, &name) {
        let d = py::get_or_none(vm, &name, "_duplicatable")?;
        if ops::truthy(vm, &d)? {
            let n = vm.call_method(&name, "_duplicate", vec![a])?;
            py::set(vm, &rv, "name", n)?;
        }
    }
    vm.call_method(&rv, "find_target", Vec::new())?;
    Ok(rv)
}

fn ir_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    vm.call_method(&t, "_unique", Vec::new())?;
    py::set(vm, &args[0], "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn ir_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    let nt = vm.call_method(&t, "_in_current_store", Vec::new())?;
    if py::same(&nt, &t) {
        return Ok(args[0].clone());
    }
    let rv = vm.call_method(&args[0], "_copy", Vec::new())?;
    py::set(vm, &rv, "target", nt)?;
    Ok(rv)
}

macro_rules! forward {
    ($name:ident, $method:literal) => {
        fn $name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
            let t = ensure_target(vm, &args[0])?;
            vm.call_method(&t, $method, args[1..].to_vec())
        }
    };
}
forward!(fwd_handles_event, "_handles_event");
forward!(fwd_hide, "_hide");
forward!(fwd_set_transform_event, "set_transform_event");
forward!(fwd_event, "event");

fn fwd_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    wrap_render(vm, &t, args)
}

fn fwd_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    Ok(vm.types.new_list(vec![t]))
}

fn ir_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = ensure_target(vm, this)?;
    let p = vm.call_method(&t, "get_placement", Vec::new())?;
    if !py::config_bool(vm, "imagereference_respects_position").unwrap_or(false) {
        return Ok(p);
    }
    let mut p = py::items(vm, &p)?;
    let style = py::get(vm, this, "style")?;
    for (i, n) in [(0, "xpos"), (1, "ypos"), (2, "xanchor"), (3, "yanchor")] {
        if py::is_none(&p[i]) {
            p[i] = py::get(vm, &style, n)?;
        }
    }
    Ok(vm.types.new_tuple(p))
}

// DynamicImage ----------------------------------------------------------------------------

fn di_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = arg_or(args, kw, 1, "name", Value::None);
    let scope = arg_or(args, kw, 2, "scope", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "name" | "scope"))
        .cloned()
        .collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "name", name.clone())?;
    let mut uses = false;
    let mut dup = None;
    let parts: Vec<String> = match name.as_str() {
        Some(s) => vec![s.to_owned()],
        None => strs(&py::items(vm, &name).unwrap_or_default()),
    };
    for p in &parts {
        if p.contains("[prefix_") {
            dup = Some(true);
        }
        if p.replace("[prefix_]", "").contains('[') {
            uses = true;
        }
    }
    if let Some(d) = dup {
        py::set(vm, this, "_duplicatable", Value::Bool(d))?;
    }
    if py::is_none(&scope) {
        uses = false;
    }
    py::set(vm, this, "_uses_scope", Value::Bool(uses))?;
    if !py::is_none(&scope) {
        vm.call_method(this, "find_target", vec![scope])?;
    }
    Ok(Value::None)
}

fn di_scope(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "find_target", args[1..].to_vec())
}

fn di_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut hn = py::get_or_none(vm, this, "hash_name")?;
    if py::is_none(&hn) {
        let n = py::get(vm, this, "name")?;
        hn = if let Some(l) =
            py::seq(&n).filter(|_| matches!(n.payload(), Some(crysirror_python::Payload::List(_))))
        {
            vm.types.new_tuple(l)
        } else {
            n
        };
        py::set(vm, this, "hash_name", hn.clone())?;
    }
    Ok(Value::Int(ops::hash(vm, &hn)?))
}

fn di_eq(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = ir_eq(vm, args, kw)?;
    if !ops::truthy(vm, &r)? || py::same(&args[0], &args[1]) {
        return Ok(r);
    }
    let uses = py::get_or_none(vm, &args[0], "_uses_scope")?;
    if ops::truthy(vm, &uses)? {
        let a = py::get_or_none(vm, &args[0], "target")?;
        let b = py::get_or_none(vm, &args[1], "target")?;
        if !ops::eq(vm, &a, &b)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn di_target(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get_or_none(vm, &args[0], "target")?;
    if ops::truthy(vm, &t)? {
        return vm.call_method(&t, "_target", Vec::new());
    }
    Ok(args[0].clone())
}

fn di_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prefix = args.get(1).cloned().unwrap_or(Value::None);
    let style = py::get(vm, this, "style")?;
    let cur = py::get(vm, &style, "prefix")?;
    let dup = py::get_or_none(vm, this, "_duplicatable")?;
    if !ops::eq(vm, &prefix, &cur)? && ops::truthy(vm, &dup)? {
        py::set(vm, this, "target", Value::None)?;
        py::set(vm, this, "raw_target", Value::None)?;
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "set_style_prefix")?;
    vm.call(&f, args.to_vec(), Vec::new())
}

fn di_find_target(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut scope = arg_or(args, kw, 1, "scope", Value::None);
    let update = match py::arg(args, kw, 2, "update") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let locked = py::get_or_none(vm, this, "locked")?;
    let target0 = py::get_or_none(vm, this, "target")?;
    if ops::truthy(vm, &locked)? && !py::is_none(&target0) {
        return Ok(Value::None);
    }
    let uses = {
        let v = py::get_or_none(vm, this, "_uses_scope")?;
        ops::truthy(vm, &v)?
    };
    let dup = {
        let v = py::get_or_none(vm, this, "_duplicatable")?;
        ops::truthy(vm, &v)?
    };
    let da = py::get(vm, this, "_args")?;
    let da_prefix = py::get_or_none(vm, &da, "prefix")?;
    if !py::is_none(&scope) {
        if uses && (dup || !py::is_none(&da_prefix)) {
            let d = vm.call(
                &Value::Obj(vm.types.dict.clone()),
                vec![scope.clone()],
                Vec::new(),
            )?;
            py::set(vm, this, "scope", d)?;
        }
    } else if uses {
        scope = py::get_or_none(vm, this, "scope")?;
    }
    let prefix = if py::is_none(&da_prefix) {
        if dup {
            let s = py::get(vm, this, "style")?;
            py::get(vm, &s, "prefix")?
        } else {
            vm.types.new_str("")
        }
    } else {
        da_prefix
    };
    let name = py::get(vm, this, "name")?;
    let search = vm.types.new_list(Vec::new());
    let di = py::import_attr(vm, "renpy.easy", "dynamic_image")?;
    let target = match vm.call(
        &di,
        vec![name.clone(), scope],
        vec![("prefix".into(), prefix), ("search".into(), search.clone())],
    ) {
        Ok(t) => t,
        Err(e) => {
            let nr = ops::repr(vm, &name)?;
            let msg = match &e {
                crysirror_python::PyErr::Exc(o) => {
                    let cls = o.class_name();
                    let s = ops::repr(vm, &Value::Obj(o.clone()))?;
                    if cls == "KeyError" {
                        format!("In DynamicImage {nr}: Could not find substitution {s}.")
                    } else {
                        format!("In DynamicImage {nr}: {s}")
                    }
                }
                _ => return Err(e),
            };
            return Err(py::error(vm, msg));
        }
    };
    if py::is_none(&target) {
        let nr = ops::repr(vm, &name)?;
        let s = py::items(vm, &search)?;
        let mut msg = format!("DynamicImage {nr}: could not find image.");
        let r = |vm: &mut Vm, v: &Value| ops::repr(vm, v).map(|s| s.to_string());
        match s.len() {
            0 => {}
            1 => msg += &format!(" ({})", r(vm, &s[0])?),
            2 => msg += &format!(" ({}, {})", r(vm, &s[0])?, r(vm, &s[1])?),
            n => {
                msg += &format!(
                    " ({}, {}, and {} more.)",
                    r(vm, &s[0])?,
                    r(vm, &s[1])?,
                    n - 2
                )
            }
        }
        return Err(py::error(vm, msg));
    }
    let raw = py::get_or_none(vm, this, "raw_target")?;
    if ops::eq(vm, &raw, &target)? {
        return Ok(Value::Bool(false));
    }
    if !update {
        return Ok(Value::Bool(true));
    }
    let old = py::get_or_none(vm, this, "target")?;
    let tdup = {
        let v = py::get_or_none(vm, &target, "_duplicatable")?;
        ops::truthy(vm, &v)?
    };
    let mut t = target.clone();
    if tdup {
        t = vm.call_method(&target, "_duplicate", vec![da])?;
        if !dup {
            vm.call_method(&t, "_unique", Vec::new())?;
        }
    }
    py::set(vm, this, "raw_target", target)?;
    py::set(vm, this, "target", t.clone())?;
    super::render::redraw(vm, this, 0.);
    if !ops::truthy(vm, &old)? {
        return Ok(Value::Bool(true));
    }
    let tc = py::import_attr(vm, "renpy.display.motion", "Transform")?;
    let Some(tco) = tc.obj().cloned() else {
        return Ok(Value::Bool(true));
    };
    if !ops::isinstance(vm, &old, &tco) {
        return Ok(Value::Bool(true));
    }
    if !ops::isinstance(vm, &t, &tco) {
        t = vm.call(&tc, Vec::new(), vec![("child".into(), t)])?;
        py::set(vm, this, "target", t.clone())?;
    }
    vm.call_method(&t, "take_state", vec![old])?;
    Ok(Value::Bool(true))
}

fn di_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = arg_or(args, kw, 1, "args", Value::None);
    if !py::is_none(&a) {
        let x = py::get(vm, &a, "args")?;
        if ops::truthy(vm, &x)? {
            vm.call_method(&a, "extraneous", Vec::new())?;
        }
    }
    let rv = vm.call_method(&args[0], "_copy", vec![a])?;
    py::set(vm, &rv, "target", Value::None)?;
    py::set(vm, &rv, "raw_target", Value::None)?;
    Ok(rv)
}

fn di_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get_or_none(vm, &args[0], "target")?;
    if !py::is_none(&t) {
        vm.call_method(&t, "_unique", Vec::new())?;
    }
    py::set(vm, &args[0], "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn di_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rv = vm.call_method(&args[0], "_copy", Vec::new())?;
    let t = py::get_or_none(vm, &rv, "target")?;
    if ops::truthy(vm, &t)? {
        let n = vm.call_method(&t, "_in_current_store", Vec::new())?;
        py::set(vm, &rv, "target", n)?;
    }
    py::set(vm, &rv, "locked", Value::Bool(true))?;
    Ok(rv)
}

fn di_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ensure_target(vm, &args[0])?;
    vm.call_method(&t, "get_placement", Vec::new())
}

fn di_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let old = py::get_or_none(vm, this, "target")?;
    let uses = py::get_or_none(vm, this, "_uses_scope")?;
    if !ops::truthy(vm, &uses)? {
        vm.call_method(this, "find_target", Vec::new())?;
    }
    let t = py::get_or_none(vm, this, "target")?;
    if !py::same(&old, &t) && !py::is_none(&t) {
        let cb = py::builtin(vm, "per_interact_visitor", |vm, a, _| {
            vm.call_method(&a[0], "per_interact", Vec::new())
        });
        vm.call_method(&t, "visit_all", vec![cb])?;
    }
    Ok(Value::None)
}

// ShownImageInfo -----------------------------------------------------------------------------

fn default_layer(vm: &mut Vm, layer: &Value, tag: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", "default_layer")?;
    vm.call(&f, vec![layer.clone(), tag.clone()], Vec::new())
}

fn sii_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let old = arg_or(args, kw, 1, "old", Value::None);
    if py::is_none(&old) {
        py::set(vm, this, "attributes", vm.types.new_dict())?;
        let s = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
        py::set(vm, this, "shown", s)?;
    } else {
        let a = py::get(vm, &old, "attributes")?;
        let a = vm.call_method(&a, "copy", Vec::new())?;
        let s = py::get(vm, &old, "shown")?;
        let s = vm.call_method(&s, "copy", Vec::new())?;
        py::set(vm, this, "attributes", a)?;
        py::set(vm, this, "shown", s)?;
    }
    Ok(Value::None)
}

fn sii_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(2)))?;
    if v >= 2 {
        return Ok(Value::None);
    }
    let attrs = vm.types.new_dict();
    let shown = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    let images = py::get_or_none(vm, this, "images")?;
    if !py::is_none(&images) {
        for (layer, d) in ops::mapping_items(vm, &images)? {
            for (tag, v) in ops::mapping_items(vm, &d)? {
                let items = py::items(vm, &v)?;
                let rest = vm.types.new_tuple(items.get(1..).unwrap_or(&[]).to_vec());
                let key = vm.types.new_tuple(vec![layer.clone(), tag.clone()]);
                ops::setitem(vm, &attrs, key.clone(), rest)?;
                vm.call_method(&shown, "add", vec![key])?;
            }
        }
    }
    py::set(vm, this, "attributes", attrs)?;
    py::set(vm, this, "shown", shown)?;
    Ok(Value::None)
}

fn sii_get_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let tag = arg_or(args, kw, 2, "tag", Value::None);
    let default = arg_or(args, kw, 3, "default", vm.types.empty_tuple.clone());
    let layer = default_layer(vm, &layer, &tag)?;
    let attrs = py::get(vm, &args[0], "attributes")?;
    let key = vm.types.new_tuple(vec![layer, tag]);
    vm.call_method(&attrs, "get", vec![key, default])
}

fn sii_showing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let name = arg_or(args, kw, 2, "name", Value::None);
    let exact = match py::arg(args, kw, 3, "exact") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let n = py::items(vm, &name)?;
    let tag = n.first().cloned().unwrap_or(Value::None);
    let layer = default_layer(vm, &layer, &tag)?;
    let shown = py::get(vm, this, "shown")?;
    let key = vm.types.new_tuple(vec![layer, tag]);
    if !ops::contains(vm, &shown, &key)? {
        return Ok(Value::None);
    }
    let attrs = py::get(vm, this, "attributes")?;
    let sa = ops::getitem(vm, &attrs, key)?;
    let sa = strs(&py::items(vm, &sa)?);
    let rest = strs(&n[1..]);
    if sa.len() < rest.len() || (exact && sa.len() != rest.len()) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(sa.iter().zip(&rest).all(|(a, b)| a == b)))
}

fn sii_get_showing_tags(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let layer = args.get(1).cloned().unwrap_or(Value::None);
    let shown = py::get(vm, &args[0], "shown")?;
    let out = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    for k in py::items(vm, &shown)? {
        let k = py::items(vm, &k)?;
        if ops::eq(vm, &k[0], &layer)? {
            vm.call_method(&out, "add", vec![k[1].clone()])?;
        }
    }
    Ok(out)
}

fn sii_get_hidden_tags(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let layer = args.get(1).cloned().unwrap_or(Value::None);
    let shown = py::get(vm, &args[0], "shown")?;
    let attrs = py::get(vm, &args[0], "attributes")?;
    let out = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    for k in py::items(vm, &attrs)? {
        let kk = py::items(vm, &k)?;
        if ops::eq(vm, &kk[0], &layer)? && !ops::contains(vm, &shown, &k)? {
            vm.call_method(&out, "add", vec![kk[1].clone()])?;
        }
    }
    Ok(out)
}

fn sii_predict_scene(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut layer = args.get(1).cloned().unwrap_or(Value::None);
    if py::is_none(&layer) {
        layer = vm.types.new_str("master");
    }
    let attrs = py::get(vm, this, "attributes")?;
    for k in py::items(vm, &attrs)? {
        let kk = py::items(vm, &k)?;
        if ops::eq(vm, &kk[0], &layer)? {
            ops::delitem(vm, &attrs, k)?;
        }
    }
    let shown = py::get(vm, this, "shown")?;
    let ns = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    for k in py::items(vm, &shown)? {
        let kk = py::items(vm, &k)?;
        if !ops::eq(vm, &kk[0], &layer)? {
            vm.call_method(&ns, "add", vec![k])?;
        }
    }
    py::set(vm, this, "shown", ns)?;
    Ok(Value::None)
}

fn sii_predict_show(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let name = arg_or(args, kw, 2, "name", Value::None);
    let show = match py::arg(args, kw, 3, "show") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let n = py::items(vm, &name)?;
    let tag = n.first().cloned().unwrap_or(Value::None);
    let layer = default_layer(vm, &layer, &tag)?;
    let attrs = py::get(vm, this, "attributes")?;
    let key = vm.types.new_tuple(vec![layer, tag]);
    let rest = vm.types.new_tuple(n.get(1..).unwrap_or(&[]).to_vec());
    ops::setitem(vm, &attrs, key.clone(), rest)?;
    if show {
        let shown = py::get(vm, this, "shown")?;
        vm.call_method(&shown, "add", vec![key])?;
    }
    Ok(Value::None)
}

fn sii_predict_hide(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let name = arg_or(args, kw, 2, "name", Value::None);
    let n = py::items(vm, &name)?;
    let tag = n.first().cloned().unwrap_or(Value::None);
    let layer = default_layer(vm, &layer, &tag)?;
    let attrs = py::get(vm, this, "attributes")?;
    let key = vm.types.new_tuple(vec![layer, tag]);
    if ops::contains(vm, &attrs, &key)? {
        ops::delitem(vm, &attrs, key.clone())?;
    }
    let shown = py::get(vm, this, "shown")?;
    vm.call_method(&shown, "discard", vec![key])?;
    Ok(Value::None)
}

fn sii_apply_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layer = arg_or(args, kw, 1, "layer", Value::None);
    let tag = arg_or(args, kw, 2, "tag", Value::None);
    let mut name = arg_or(args, kw, 3, "name", Value::None);
    let adjust = py::config(vm, "adjust_attributes")?;
    let n0 = py::items(vm, &name)?;
    let first = n0.first().cloned().unwrap_or(Value::None);
    let mut f = vm.call_method(&adjust, "get", vec![first.clone(), Value::None])?;
    if !ops::truthy(vm, &f)? {
        f = vm.call_method(&adjust, "get", vec![Value::None, Value::None])?;
    }
    if !py::is_none(&f) {
        name = vm.call(&f, vec![name], Vec::new())?;
    }
    let layer = default_layer(vm, &layer, &tag)?;
    let images = module_attr(vm, "images")?;
    let nitems = py::items(vm, &name)?;
    let nt = vm.types.new_tuple(nitems);
    if ops::contains(vm, &images, &nt)? {
        let d = ops::getitem(vm, &images, nt.clone())?;
        let ca = py::get_opt(vm, &d, "_choose_attributes")?;
        if ca.is_none_or(|c| py::is_none(&c)) {
            return Ok(nt);
        }
    }
    let n = py::items(vm, &nt)?;
    let nametag = n.first().cloned().unwrap_or(Value::None);
    let attrs = py::get(vm, this, "attributes")?;
    let key = vm.types.new_tuple(vec![layer, tag]);
    let mut defaults = vm.call_method(&attrs, "get", vec![key, Value::None])?;
    if py::is_none(&defaults) {
        let dac = py::config(vm, "default_attribute_callbacks")?;
        let mut f = vm.call_method(&dac, "get", vec![nametag.clone(), Value::None])?;
        if !ops::truthy(vm, &f)? {
            f = vm.call_method(&dac, "get", vec![Value::None, Value::None])?;
        }
        if !py::is_none(&f) {
            defaults = vm.call(&f, vec![nt.clone()], Vec::new())?;
        }
    }
    let mut optional = if ops::truthy(vm, &defaults)? {
        strs(&py::items(vm, &defaults)?)
    } else {
        Vec::new()
    };
    let mut required: Vec<String> = Vec::new();
    for i in strs(&n[1..]) {
        if let Some(x) = i.strip_prefix('-') {
            optional.retain(|o| o != x);
            required.retain(|o| o != x);
        } else {
            required.push(i);
        }
    }
    choose_image(vm, &nametag, &required, &optional, Some(&nt))
}

fn choose_image(
    vm: &mut Vm,
    tag: &Value,
    required: &[String],
    optional: &[String],
    exception_name: Option<&Value>,
) -> PyResult<Value> {
    let mut max_len: i64 = -1;
    let mut matches: Option<Vec<Vec<String>>> = None;
    for (mut attrs, d) in tag_attrs(vm, tag)? {
        if !attrs
            .iter()
            .all(|i| required.contains(i) || optional.contains(i))
        {
            continue;
        }
        let car: Vec<String> = required
            .iter()
            .filter(|i| !attrs.contains(i))
            .cloned()
            .collect();
        let cao: Vec<String> = optional
            .iter()
            .filter(|i| !attrs.contains(i) && !required.contains(i))
            .cloned()
            .collect();
        match choose_attributes(vm, &d, tag, &car, Some(&cao))? {
            Some(None) => continue,
            Some(Some(new)) => attrs.extend(new),
            None => {
                let n = attrs.iter().filter(|i| required.contains(i)).count();
                if n != required.len() {
                    continue;
                }
            }
        }
        let mut uniq = attrs.clone();
        uniq.sort();
        uniq.dedup();
        let l = uniq.len() as i64;
        if l < max_len {
            continue;
        }
        if l > max_len {
            max_len = l;
            matches = Some(Vec::new());
        }
        let mut full = vec![tag.as_str().unwrap_or("").to_owned()];
        full.extend(attrs);
        matches.get_or_insert_with(Vec::new).push(full);
    }
    let Some(m) = matches else {
        return Ok(Value::None);
    };
    if m.len() == 1 {
        return Ok(tuple_of(vm, &m[0]));
    }
    if let Some(n) = exception_name {
        let n = strs(&py::items(vm, n)?).join(" ");
        let opts: Vec<String> = m.iter().map(|i| i.join(" ")).collect();
        return Err(py::error(
            vm,
            format!(
                "Showing '{n}' is ambiguous, possible images include: {}",
                opts.join(", ")
            ),
        ));
    }
    Ok(Value::None)
}

fn sii_choose_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tag = arg_or(args, kw, 1, "tag", Value::None);
    let req = strs(&py::items(
        vm,
        &arg_or(args, kw, 2, "required", Value::None),
    )?);
    let opt = strs(&py::items(
        vm,
        &arg_or(args, kw, 3, "optional", Value::None),
    )?);
    let en = arg_or(args, kw, 4, "exception_name", Value::None);
    let has = ops::truthy(vm, &en)?;
    choose_image(vm, &tag, &req, &opt, if has { Some(&en) } else { None })
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("list_images", f_list_images),
            ("get_available_image_tags", f_get_available_image_tags),
            (
                "get_available_image_attributes",
                f_get_available_image_attributes,
            ),
            ("get_tag_method", f_get_tag_method),
            ("check_image_attributes", f_check_image_attributes),
            (
                "get_ordered_image_attributes",
                f_get_ordered_image_attributes,
            ),
            ("register_image", f_register_image),
            ("image_exists", f_image_exists),
            ("get_registered_image", f_get_registered_image),
            ("wrap_render", f_wrap_render),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "images", vm.types.new_dict());
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let ia = vm.call(&dd, vec![Value::Obj(vm.types.dict.clone())], Vec::new())?;
    set(vm, "image_attributes", ia);
    set(vm, "image_names", vm.types.new_list(Vec::new()));
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm,
             name: &str,
             bases: &[Value],
             methods: py::Methods,
             attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    let nosave_t = vm.types.new_list(vec![vm.types.new_str("target")]);
    c(
        vm,
        "ImageReference",
        &[d.clone()],
        &[
            ("__init__", ir_init),
            ("after_upgrade", ir_after_upgrade),
            ("_repr_info", repr_info_name),
            ("__hash__", ir_hash),
            ("__eq__", ir_eq),
            ("_target", ir_target),
            ("find_target", ir_find_target),
            ("_duplicate", ir_duplicate),
            ("_unique", ir_unique),
            ("_in_current_store", ir_in_current_store),
            ("_handles_event", fwd_handles_event),
            ("_hide", fwd_hide),
            ("set_transform_event", fwd_set_transform_event),
            ("event", fwd_event),
            ("render", fwd_render),
            ("get_placement", ir_get_placement),
            ("visit", fwd_visit),
        ],
        &[
            ("nosave", nosave_t),
            ("target", Value::None),
            ("old_transform", Value::None),
            ("param_target", Value::None),
            ("__version__", Value::Int(1)),
            ("_duplicatable", Value::Bool(true)),
        ],
    )?;
    let nosave_r = vm.types.new_list(vec![vm.types.new_str("raw_target")]);
    c(
        vm,
        "DynamicImage",
        &[d],
        &[
            ("__init__", di_init),
            ("_scope", di_scope),
            ("_repr_info", repr_info_name),
            ("__hash__", di_hash),
            ("__eq__", di_eq),
            ("_target", di_target),
            ("set_style_prefix", di_set_style_prefix),
            ("find_target", di_find_target),
            ("_duplicate", di_duplicate),
            ("_unique", di_unique),
            ("_in_current_store", di_in_current_store),
            ("_handles_event", fwd_handles_event),
            ("_hide", fwd_hide),
            ("set_transform_event", fwd_set_transform_event),
            ("event", fwd_event),
            ("render", fwd_render),
            ("get_placement", di_get_placement),
            ("visit", fwd_visit),
            ("per_interact", di_per_interact),
        ],
        &[
            ("nosave", nosave_r),
            ("target", Value::None),
            ("raw_target", Value::None),
            ("locked", Value::Bool(false)),
            ("hash_name", Value::None),
            ("scope", Value::None),
            ("_duplicatable", Value::Bool(true)),
        ],
    )?;
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    c(
        vm,
        "ShownImageInfo",
        &[object],
        &[
            ("__init__", sii_init),
            ("after_upgrade", sii_after_upgrade),
            ("get_attributes", sii_get_attributes),
            ("showing", sii_showing),
            ("get_showing_tags", sii_get_showing_tags),
            ("get_hidden_tags", sii_get_hidden_tags),
            ("predict_scene", sii_predict_scene),
            ("predict_show", sii_predict_show),
            ("predict_hide", sii_predict_hide),
            ("apply_attributes", sii_apply_attributes),
            ("choose_image", sii_choose_image),
        ],
        &[("__version__", Value::Int(2))],
    )?;
    // Compatibility names at the end of upstream image.py (all releases).
    for n in ["Solid", "Frame"] {
        let v = py::import_attr(vm, "renpy.display.imagelike", n)?;
        set(vm, n, v);
    }
    Ok(m)
}

pub fn install_module(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
