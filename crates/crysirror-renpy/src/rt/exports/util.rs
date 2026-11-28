// SPDX-License-Identifier: MPL-2.0
//! Shared lookups of the `renpy.exports` natives.
use super::super::py;
use crysirror_python::{PyErr, PyResult, Value, Vm, ops};
use std::rc::Rc;

pub fn log(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "log")
}

pub fn ctx(vm: &mut Vm) -> PyResult<Value> {
    super::super::game::context(vm, -1)
}

pub fn iface(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "interface")
}

pub fn store_get(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let s = super::super::python::store_dict(vm, "store")?;
    let k = py::s(vm, n);
    Ok(ops::dict_get(vm, s.obj().unwrap(), &k)?.unwrap_or(Value::None))
}

pub fn store_set(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let s = super::super::python::store_dict(vm, "store")?;
    py::dict_set(vm, &s, n, v)
}

pub fn store_true(vm: &mut Vm, n: &str) -> PyResult<bool> {
    let v = store_get(vm, n)?;
    py::truthy(vm, &v)
}

pub fn config_set(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let c = py::import(vm, "renpy.config")?;
    py::set(vm, &c, n, v)
}

/// `renpy.exports.<name>(*args, **kw)`, so script overrides apply.
pub fn ex(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", name)?;
    vm.call(&f, args, kw)
}

pub fn ui(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.ui", name)?;
    vm.call(&f, args, kw)
}

pub fn raise(vm: &mut Vm, module: &str, cls: &str, args: Vec<Value>) -> PyErr {
    match py::import_attr(vm, module, cls).and_then(|c| vm.call(&c, args, Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error(cls),
        Err(e) => e,
    }
}

/// Whether `v` is an instance of `module.cls`.
pub fn isinst(vm: &mut Vm, v: &Value, module: &str, cls: &str) -> PyResult<bool> {
    let c = py::import_attr(vm, module, cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

pub fn exc_value(e: &PyErr) -> Option<Value> {
    match e {
        PyErr::Exc(o) => Some(Value::Obj(o.clone())),
        _ => None,
    }
}

pub fn is_jump_or_call(vm: &mut Vm, e: &PyErr) -> bool {
    super::super::execution::exc_isinstance(vm, e, "renpy.game", "JumpException")
        || super::super::execution::exc_isinstance(vm, e, "renpy.game", "CallException")
}

pub fn kwb(vm: &mut Vm, v: Option<&Value>, d: bool) -> PyResult<bool> {
    match v {
        Some(v) => py::truthy(vm, v),
        None => Ok(d),
    }
}
