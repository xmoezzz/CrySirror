// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.rollbackexports` (Ren'Py 8.3, MIT).
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};

fn roll_forward_info(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let rb = py::get_or_none(vm, &c, "rollback")?;
    if !py::truthy(vm, &rb)? {
        return Ok(Value::None);
    }
    let l = log(vm)?;
    vm.call_method(&l, "forward_info", Vec::new())
}

fn roll_forward_core(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut value = arg_or(args, kw, 0, "value", Value::None);
    if py::is_none(&value) {
        value = roll_forward_info(vm, &[], &[])?;
    }
    if py::is_none(&value) {
        return Ok(Value::None);
    }
    let i = iface(vm)?;
    py::set(vm, &i, "suppress_transition", Value::Bool(true))?;
    let g = py::import(vm, "renpy.game")?;
    py::set(vm, &g, "after_rollback", Value::Bool(true))?;
    let l = log(vm)?;
    py::set(vm, &l, "rolled_forward", Value::Bool(true))?;
    Ok(value)
}

fn in_rollback(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    let r = vm.call_method(&l, "in_rollback", Vec::new())?;
    if py::truthy(vm, &r)? {
        return Ok(Value::Bool(true));
    }
    let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
    Ok(Value::Bool(py::truthy(vm, &ar)?))
}

fn can_rollback(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "rollback_enabled")? || !store_true(vm, "_rollback")? {
        return Ok(Value::Bool(false));
    }
    let c = ctx(vm)?;
    let rb = py::get_or_none(vm, &c, "rollback")?;
    if !py::truthy(vm, &rb)? {
        return Ok(Value::Bool(false));
    }
    let l = log(vm)?;
    vm.call_method(&l, "can_rollback", Vec::new())
}

fn in_fixed_rollback(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    vm.call_method(&l, "in_fixed_rollback", Vec::new())
}

pub(crate) fn checkpoint(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let data = arg_or(args, kw, 0, "data", Value::None);
    let mut keep = arg_or(args, kw, 1, "keep_rollback", Value::None);
    let hard = arg_or(args, kw, 2, "hard", Value::Bool(true));
    if py::is_none(&keep) {
        keep = py::config(vm, "keep_rollback_data")?;
    }
    let rb = store_true(vm, "_rollback")?;
    let hard = if rb { hard } else { Value::Bool(false) };
    let l = log(vm)?;
    py::method_kw(
        vm,
        &l,
        "checkpoint",
        vec![data],
        vec![("keep_rollback".into(), keep), ("hard".into(), hard)],
    )?;
    if rb && py::config_bool(vm, "auto_clear_screenshot")? {
        let i = iface(vm)?;
        if !py::is_none(&i) {
            py::set(vm, &i, "clear_screenshot", Value::Bool(true))?;
        }
    }
    Ok(Value::None)
}

fn block_rollback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let purge = arg_or(args, kw, 0, "purge", Value::Bool(false));
    let l = log(vm)?;
    py::method_kw(vm, &l, "block", Vec::new(), vec![("purge".into(), purge)])
}

fn suspend_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    vm.call_method(&l, "suspend_checkpointing", vec![args[0].clone()])
}

fn fix_rollback(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    vm.call_method(&l, "fix_rollback", Vec::new())
}

fn retain_after_load(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    vm.call_method(&l, "retain_after_load", Vec::new())
}

fn rollback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let force = arg_or(args, kw, 0, "force", Value::Bool(false));
    let checkpoints = arg_or(args, kw, 1, "checkpoints", Value::Int(1));
    let defer = kwb(vm, arg(args, kw, 2, "defer"), false)?;
    let greedy = arg_or(args, kw, 3, "greedy", Value::Bool(true));
    let label = arg_or(args, kw, 4, "label", Value::None);
    let abnormal = arg_or(args, kw, 5, "abnormal", Value::Bool(true));
    let current_label = arg_or(args, kw, 6, "current_label", Value::None);
    let l = log(vm)?;
    let entries = py::get(vm, &l, "log")?;
    if defer && !py::truthy(vm, &entries)? {
        return Ok(Value::None);
    }
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    if defer && ops::len(vm, &contexts)? > 1 {
        let c0 = ops::getitem(vm, &contexts, Value::Int(0))?;
        let t = vm.types.new_tuple(vec![force, checkpoints]);
        py::set(vm, &c0, "defer_rollback", t)?;
        return Ok(Value::None);
    }
    if !py::truthy(vm, &force)? {
        let c = ctx(vm)?;
        let rb = py::get_or_none(vm, &c, "rollback")?;
        if !store_true(vm, "_rollback")?
            || !py::truthy(vm, &rb)?
            || !py::config_bool(vm, "rollback_enabled")?
        {
            return Ok(Value::None);
        }
    }
    config_set(vm, "skipping", Value::None)?;
    vm.call_method(&l, "complete", Vec::new())?;
    let force_true = matches!(force, Value::Bool(true));
    py::method_kw(
        vm,
        &l,
        "rollback",
        vec![checkpoints],
        vec![
            ("greedy".into(), greedy),
            ("label".into(), label),
            ("force".into(), Value::Bool(force_true)),
            ("abnormal".into(), abnormal),
            ("current_label".into(), current_label),
        ],
    )
}

fn get_roll_forward(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = iface(vm)?;
    py::get_or_none(vm, &i, "shown_window")
}

fn get_identifier_checkpoints(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = log(vm)?;
    vm.call_method(&l, "get_identifier_checkpoints", vec![args[0].clone()])
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("roll_forward_info", roll_forward_info),
        ("roll_forward_core", roll_forward_core),
        ("in_rollback", in_rollback),
        ("can_rollback", can_rollback),
        ("in_fixed_rollback", in_fixed_rollback),
        ("checkpoint", checkpoint),
        ("block_rollback", block_rollback),
        ("suspend_rollback", suspend_rollback),
        ("fix_rollback", fix_rollback),
        ("retain_after_load", retain_after_load),
        ("rollback", rollback),
        ("get_roll_forward", get_roll_forward),
        ("get_identifier_checkpoints", get_identifier_checkpoints),
    ]
}
