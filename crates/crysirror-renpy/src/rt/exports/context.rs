// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.contextexports` (Ren'Py 8.3, MIT): context info,
//! dynamic variables, modes, the call stack, skipping and runtime.
use super::super::py::{self, *};
use crysirror_python::{PyErr, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};

pub(crate) fn ctx(vm: &mut Vm) -> PyResult<Value> {
    super::super::game::context(vm, -1)
}

fn raise(vm: &mut Vm, cls: &str, args: Vec<Value>) -> PyErr {
    match py::import_attr(vm, "renpy.game", cls).and_then(|c| vm.call(&c, args, Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error(cls),
        Err(e) => e,
    }
}

fn context(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get(vm, &c, "info")
}

fn context_nesting_level(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cs = py::import_attr(vm, "renpy.game", "contexts")?;
    Ok(Value::Int(ops::len(vm, &cs)? as i64 - 1))
}

fn jump_out_of_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Err(raise(vm, "JumpOutException", vec![args[0].clone()]))
}

fn info_attr(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let c = ctx(vm)?;
    let info = py::get(vm, &c, "info")?;
    Ok(py::get_opt(vm, &info, n)?.unwrap_or(Value::None))
}

fn current_interact_type(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    info_attr(vm, "_current_interact_type")
}

fn last_interact_type(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    info_attr(vm, "_last_interact_type")
}

fn dynamic(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut vars = args.to_vec();
    for (k, _) in kw {
        vars.push(py::s(vm, k));
    }
    let c = ctx(vm)?;
    let l = vm.types.new_tuple(vars);
    vm.call_method(&c, "make_dynamic", vec![l])?;
    let store = py::import(vm, "store")?;
    for (k, v) in kw {
        py::set(vm, &store, k, v.clone())?;
    }
    Ok(Value::None)
}

fn context_dynamic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let l = vm.types.new_tuple(args.to_vec());
    py::method_kw(
        vm,
        &c,
        "make_dynamic",
        vec![l],
        vec![("context".into(), Value::Bool(true))],
    )?;
    Ok(Value::None)
}

fn scry(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let name = py::get(vm, &c, "current")?;
    if py::is_none(&name) {
        return Ok(Value::None);
    }
    let id = super::super::nodes::lookup(vm, &name)?;
    let h = super::super::nodes::handle(vm, id)?;
    vm.call_method(&h, "scry", Vec::new())
}

fn pop_call(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    vm.call_method(&c, "pop_call", Vec::new())?;
    Ok(Value::None)
}

pub(crate) fn call_stack_depth(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let rs = py::get(vm, &c, "return_stack")?;
    Ok(Value::Int(ops::len(vm, &rs)? as i64))
}

fn game_menu(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let screen = arg_or(args, kw, 0, "screen", Value::None);
    let f = py::import_attr(vm, "renpy.game", "call_in_new_context")?;
    let label = py::s(vm, "_game_menu");
    if py::is_none(&screen) {
        vm.call(&f, vec![label], Vec::new())
    } else {
        vm.call(&f, vec![label], vec![("_game_menu_screen".into(), screen)])
    }
}

fn mode(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = args[0].clone();
    let c = ctx(vm)?;
    let use_modes = py::get_or_none(vm, &c, "use_modes")?;
    if !py::truthy(vm, &use_modes)? {
        return Ok(Value::None);
    }
    let modes = py::get(vm, &c, "modes")?;
    py::set(vm, &c, "use_modes", Value::Bool(false))?;
    let r = (|| -> PyResult<()> {
        let first = ops::getitem(vm, &modes, Value::Int(0))?;
        if !ops::eq(vm, &m, &first)? {
            let cbs = py::config(vm, "mode_callbacks")?;
            for cb in py::items(vm, &cbs)? {
                vm.call(&cb, vec![m.clone(), modes.clone()], Vec::new())?;
            }
        }
        Ok(())
    })();
    py::set(vm, &c, "use_modes", Value::Bool(true))?;
    r?;
    if ops::contains(vm, &modes, &m)? {
        vm.call_method(&modes, "remove", vec![m.clone()])?;
    }
    vm.call_method(&modes, "insert", vec![Value::Int(0), m])?;
    Ok(Value::None)
}

fn get_mode(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let use_modes = py::get_or_none(vm, &c, "use_modes")?;
    if !py::truthy(vm, &use_modes)? {
        return Ok(Value::None);
    }
    let modes = py::get(vm, &c, "modes")?;
    ops::getitem(vm, &modes, Value::Int(0))
}

fn end_replay(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ir = py::import_attr(vm, "store", "_in_replay")?;
    if py::truthy(vm, &ir)? {
        return Err(raise(vm, "EndReplay", Vec::new()));
    }
    Ok(Value::None)
}

fn get_return_stack(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    vm.call_method(&c, "get_return_stack", Vec::new())
}

fn set_return_stack(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    vm.call_method(&c, "set_return_stack", vec![args[0].clone()])
}

fn get_line_log(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let l = py::get_items(vm, &c, "line_log")?;
    Ok(vm.types.new_list(l))
}

fn clear_line_log(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::set(vm, &c, "line_log", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn get_skipping(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::config(vm, "skipping")
}

fn is_skipping(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::config(vm, "skipping")?;
    Ok(Value::Bool(py::truthy(vm, &s)?))
}

fn stop_skipping(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cfg = py::import(vm, "renpy.config")?;
    py::set(vm, &cfg, "skipping", Value::None)?;
    Ok(Value::None)
}

pub(crate) fn is_init_phase(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get_or_none(vm, &c, "init_phase")
}

fn add_to_all_stores(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ip = is_init_phase(vm, &[], &[])?;
    if !py::truthy(vm, &ip)? {
        return Err(py::error(
            vm,
            "add_to_all_stores is only allowed in init code.",
        ));
    }
    let sds = py::import_attr(vm, "renpy.python", "store_dicts")?;
    for (_, ns) in ops::mapping_items(vm, &sds)? {
        if !ops::contains(vm, &ns, &args[0])? {
            ops::setitem(vm, &ns, args[0].clone(), args[1].clone())?;
        }
    }
    Ok(Value::None)
}

fn clear_game_runtime(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = super::super::game::context(vm, 0)?;
    py::set(vm, &c, "runtime", Value::Int(0))?;
    Ok(Value::None)
}

fn get_game_runtime(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = super::super::game::context(vm, 0)?;
    py::get_or_none(vm, &c, "runtime")
}

fn get_statement_name(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.ast", "current_statement_name")
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("context", context),
        ("context_nesting_level", context_nesting_level),
        ("jump_out_of_context", jump_out_of_context),
        ("current_interact_type", current_interact_type),
        ("last_interact_type", last_interact_type),
        ("dynamic", dynamic),
        ("context_dynamic", context_dynamic),
        ("scry", scry),
        ("pop_call", pop_call),
        ("pop_return", pop_call),
        ("call_stack_depth", call_stack_depth),
        ("game_menu", game_menu),
        ("mode", mode),
        ("get_mode", get_mode),
        ("end_replay", end_replay),
        ("get_return_stack", get_return_stack),
        ("set_return_stack", set_return_stack),
        ("get_line_log", get_line_log),
        ("clear_line_log", clear_line_log),
        ("get_skipping", get_skipping),
        ("is_skipping", is_skipping),
        ("stop_skipping", stop_skipping),
        ("is_init_phase", is_init_phase),
        ("add_to_all_stores", add_to_all_stores),
        ("clear_game_runtime", clear_game_runtime),
        ("get_game_runtime", get_game_runtime),
        ("get_statement_name", get_statement_name),
    ]
}

/// `call_in_new_context` and the other names that alias `renpy.game`.
pub fn values(vm: &mut Vm, set: &mut dyn FnMut(&mut Vm, &str, Value)) -> PyResult<()> {
    let curry = py::import_attr(vm, "renpy.curry", "curry")?;
    for n in [
        "call_in_new_context",
        "invoke_in_new_context",
        "call_replay",
    ] {
        let f = py::import_attr(vm, "renpy.game", n)?;
        set(vm, n, f.clone());
        if n != "call_replay" {
            let c = vm.call(&curry, vec![f], Vec::new())?;
            set(vm, &format!("curried_{n}"), c);
        }
    }
    Ok(())
}
