// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.imagemap`, after Ren'Py 8.3 `imagemap.py` (MIT).
//!
//! Upstream forces `ImageMapCache.enable` to False, so `crop` always yields
//! an `ImageMapCrop` and no hotspot cache file is ever written or read; the
//! cache object keeps its fields for pickle compatibility.
use super::{
    layout::rargs,
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};

const MODULE: &str = "renpy.display.imagemap";

fn crop_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    super::core::d_init(vm, &[args[0].clone()], &[])?;
    py::set(vm, &args[0], "child", args[1].clone())?;
    py::set(vm, &args[0], "rect", args[2].clone())?;
    Ok(Value::None)
}

fn crop_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "child")?;
    Ok(vm.types.new_list(vec![c]))
}

fn crop_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, _, st, at) = rargs(vm, args)?;
    let sw = py::config_f64(vm, "screen_width")?;
    let sh = py::config_f64(vm, "screen_height")?;
    let child = py::get(vm, &args[0], "child")?;
    let cr = render::render(vm, &child, sw, sh, st, at)?;
    let rect = py::get(vm, &args[0], "rect")?;
    let r = render::rect4(vm, &rect)?;
    render::subsurface(vm, &cr, r, false, false)
}

fn ccrop_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    super::core::d_init(vm, &[args[0].clone()], &[])?;
    py::set(vm, &args[0], "cache", args[1].clone())?;
    py::set(vm, &args[0], "index", args[2].clone())?;
    Ok(Value::None)
}

fn ccrop_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "cache")?;
    let i = py::get(vm, &args[0], "index")?;
    vm.call_method(&c, "visit", vec![i])
}

fn ccrop_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "cache")?;
    let i = py::get(vm, &args[0], "index")?;
    let mut a = vec![i];
    a.extend(args[1..].iter().cloned());
    vm.call_method(&c, "render", a)
}

fn cache_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "md5", Value::None)?;
    py::set(vm, this, "imagerect", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "hotspots", vm.types.new_dict())?;
    py::set(vm, this, "areas", vm.types.new_list(Vec::new()))?;
    for n in ["cache", "cache_rect", "cache_width", "cache_height"] {
        py::set(vm, this, n, Value::None)?;
    }
    py::set(vm, this, "enable", Value::Bool(false))?;
    Ok(Value::None)
}

fn cache_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "cache")?;
    if !py::is_none(&c) {
        return Ok(vm.types.new_list(vec![c]));
    }
    let ir = py::get(vm, &args[0], "imagerect")?;
    let e = crysirror_python::ops::getitem(vm, &ir, args[1].clone())?;
    let d = crysirror_python::ops::getitem(vm, &e, Value::Int(0))?;
    Ok(vm.types.new_list(vec![d]))
}

fn cache_crop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "ImageMapCrop")?;
    vm.call(&c, vec![args[1].clone(), args[2].clone()], Vec::new())
}

fn cache_finish(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn cache_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let index = args[1].clone();
    let w = py::f64_of(vm, &args[2])?;
    let h = py::f64_of(vm, &args[3])?;
    let st = py::f64_of(vm, &args[4])?;
    let at = py::f64_of(vm, &args[5])?;
    let ir = py::get(vm, this, "imagerect")?;
    let e = crysirror_python::ops::getitem(vm, &ir, index)?;
    let p = py::items(vm, &e)?;
    let r = render::render(vm, &p[0], w, h, st, at)?;
    let rect = render::rect4(vm, &p[1])?;
    render::subsurface(vm, &r, rect, false, false)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[]);
    let set = vm.types.new_set(crysirror_python::dict::Dict::default());
    crysirror_python::modules::set(vm, &m, "cached", set);
    let d = Value::Obj(super::core::displayable_class(vm));
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let mut c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods| -> PyResult<()> {
        let cls = py::class(vm, MODULE, name, bases, methods, &[])?;
        crysirror_python::modules::set(vm, &m, name, Value::Obj(cls));
        Ok(())
    };
    c(
        vm,
        "ImageMapCrop",
        &[d.clone()],
        &[
            ("__init__", crop_init),
            ("visit", crop_visit),
            ("render", crop_render),
        ],
    )?;
    c(
        vm,
        "ImageCacheCrop",
        &[d],
        &[
            ("__init__", ccrop_init),
            ("visit", ccrop_visit),
            ("render", ccrop_render),
        ],
    )?;
    c(
        vm,
        "ImageMapCache",
        &[object],
        &[
            ("__init__", cache_init),
            ("visit", cache_visit),
            ("crop", cache_crop),
            ("finish", cache_finish),
            ("render", cache_render),
        ],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
