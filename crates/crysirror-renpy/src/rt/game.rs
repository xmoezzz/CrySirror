// SPDX-License-Identifier: MPL-2.0
//! `renpy.game`, after Ren'Py 8.3 `game.py` (MIT): engine-wide globals,
//! the control-flow exceptions, the context stack and running code or
//! labels in a new context (`invoke_in_new_context`,
//! `call_in_new_context`, `call_replay`).
use super::py::{self, *};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.game";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, n, v)
}

fn contexts(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "contexts")
}

pub fn context(vm: &mut Vm, index: i64) -> PyResult<Value> {
    let c = contexts(vm)?;
    ops::getitem(vm, &c, Value::Int(index))
}

fn f_context(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let i = arg_or(args, kw, 0, "index", Value::Int(-1));
    let c = contexts(vm)?;
    ops::getitem(vm, &c, i)
}

fn exc_of(vm: &mut Vm, e: &PyErr, cls: &str) -> bool {
    super::execution::exc_isinstance(vm, e, MODULE, cls)
}

fn raise(vm: &mut Vm, cls: &str, args: Vec<Value>) -> PyErr {
    match mget(vm, cls).and_then(|c| vm.call(&c, args, Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error(cls),
        Err(e) => e,
    }
}

/// Prepares a new non-rollback context on top of the stack.
fn push_context(vm: &mut Vm, clear: Value) -> PyResult<Value> {
    let log = mget(vm, "log")?;
    if !py::is_none(&log) {
        let cur = py::get_or_none(vm, &log, "current")?;
        if !py::is_none(&cur) {
            vm.call_method(&log, "complete", Vec::new())?;
        }
    }
    let cf = py::import_attr(vm, "renpy.display.focus", "clear_focus")?;
    vm.call(&cf, Vec::new(), Vec::new())?;
    let ctx_c = py::import_attr(vm, "renpy.execution", "Context")?;
    let top = context(vm, -1)?;
    let ctx = vm.call(&ctx_c, vec![Value::Bool(false), top], vec![("clear".into(), clear)])?;
    let cs = contexts(vm)?;
    ops::list_append(vm, &cs, ctx.clone())?;
    let iface = py::import_attr(vm, "renpy.display", "interface").unwrap_or(Value::None);
    if !py::is_none(&iface) {
        vm.call_method(&iface, "enter_context", Vec::new())?;
    }
    Ok(ctx)
}

/// The `finally` of the new-context functions.
fn pop_context(vm: &mut Vm) -> PyResult<()> {
    let cs = contexts(vm)?;
    vm.call_method(&cs, "pop", Vec::new())?;
    let top = context(vm, -1)?;
    vm.call_method(&top, "do_deferred_rollback", Vec::new())?;
    refocus(vm)
}

fn refocus(vm: &mut Vm) -> PyResult<()> {
    let iface = mget(vm, "interface")?;
    if py::truthy(vm, &iface)? {
        let ri = py::get_or_none(vm, &iface, "restart_interaction")?;
        let cs = contexts(vm)?;
        if py::truthy(vm, &ri)? && ops::len(vm, &cs)? > 0 {
            let top = context(vm, -1)?;
            let sl = py::get(vm, &top, "scene_lists")?;
            py::set(vm, &sl, "focused", Value::None)?;
        }
    }
    Ok(())
}

/// `JumpOutException(label)` leaving a nested context becomes a jump in
/// the context below.
fn jump_out(vm: &mut Vm, e: &PyErr) -> PyResult<PyErr> {
    let below = context(vm, -2)?;
    py::set(vm, &below, "force_checkpoint", Value::Bool(true))?;
    py::set(vm, &below, "abnormal", Value::Bool(true))?;
    let PyErr::Exc(o) = e else { unreachable!() };
    let a = ops::getattr(vm, &Value::Obj(o.clone()), "args")?;
    let label = ops::getitem(vm, &a, Value::Int(0))?;
    Ok(raise(vm, "JumpException", vec![label]))
}

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter().position(|(k, _)| &**k == n).map(|i| kw.remove(i).1)
}

fn f_invoke_in_new_context(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let clear = take(&mut kw, "_clear_layers").unwrap_or(Value::Bool(true));
    let callable = args[0].clone();
    let ctx = push_context(vm, clear)?;
    let r = vm.call(&callable, args[1..].to_vec(), kw);
    let mut restart = false;
    let r = match r {
        Err(e) if exc_of(vm, &e, "RestartContext") || exc_of(vm, &e, "RestartTopContext") => {
            restart = true;
            Err(e)
        }
        Err(e) if exc_of(vm, &e, "JumpOutException") => Err(jump_out(vm, &e)?),
        r => r,
    };
    if !restart && !matches!(r, Err(PyErr::Suspend(..))) {
        vm.call_method(&ctx, "pop_all_dynamic", Vec::new())?;
    }
    pop_context(vm)?;
    r
}

fn f_call_in_new_context(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let clear = take(&mut kw, "_clear_layers").unwrap_or(Value::Bool(true));
    let label = args[0].clone();
    let ctx = push_context(vm, clear)?;
    let store = super::python::store_dict(vm, "store")?;
    let a = if args.len() > 1 { vm.types.new_tuple(args[1..].to_vec()) } else { Value::None };
    py::dict_set(vm, &store, "_args", a)?;
    let k = if kw.is_empty() {
        Value::None
    } else {
        let rd = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
        let d = py::kwargs_dict(vm, &kw)?;
        vm.call(&rd, vec![d], Vec::new())?
    };
    py::dict_set(vm, &store, "_kwargs", k)?;
    let r = (|| -> PyResult<Value> {
        vm.call_method(&ctx, "goto_label", vec![label])?;
        super::execution::run_context(vm, false)
    })();
    let r = match r {
        Err(e) if exc_of(vm, &e, "JumpOutException") => Err(jump_out(vm, &e)?),
        r => r,
    };
    pop_context(vm)?;
    r
}

fn f_call_replay(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let label = args[0].clone();
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    let cf = py::import_attr(vm, "renpy.display.focus", "clear_focus")?;
    vm.call(&cf, Vec::new(), Vec::new())?;
    let old_log = mget(vm, "log")?;
    vm.call_method(&old_log, "complete", Vec::new())?;
    let rlc = py::import_attr(vm, "renpy.rollback", "RollbackLog")?;
    let nl = vm.call(&rlc, Vec::new(), Vec::new())?;
    mset(vm, "log", nl)?;
    let sbc = py::import_attr(vm, "renpy.python", "StoreBackup")?;
    let sb = vm.call(&sbc, Vec::new(), Vec::new())?;
    let cs_f = py::import_attr(vm, "renpy.python", "clean_stores")?;
    vm.call(&cs_f, Vec::new(), Vec::new())?;
    let ctx_c = py::import_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&ctx_c, vec![Value::Bool(true)], Vec::new())?;
    let cs = contexts(vm)?;
    ops::list_append(vm, &cs, ctx.clone())?;
    let iface = py::import_attr(vm, "renpy.display", "interface").unwrap_or(Value::None);
    if !py::is_none(&iface) {
        vm.call_method(&iface, "enter_context", Vec::new())?;
    }
    let eds = py::import_attr(vm, "renpy.exports", "execute_default_statement")?;
    vm.call(&eds, Vec::new(), Vec::new())?;
    let store = py::import(vm, "store")?;
    let rs = py::config(vm, "replay_scope")?;
    for (k, v) in py::str_items(vm, &rs)? {
        py::set(vm, &store, &k, v)?;
    }
    if !py::is_none(&scope) {
        for (k, v) in py::str_items(vm, &scope)? {
            py::set(vm, &store, &k, v)?;
        }
    }
    py::set(vm, &store, "_in_replay", label)?;
    let r = (|| -> PyResult<()> {
        vm.call_method(&ctx, "goto_label", vec![py::s(vm, "_start_replay")])?;
        super::execution::run_context(vm, false)?;
        Ok(())
    })();
    let r = match r {
        Err(e) if exc_of(vm, &e, "EndReplay") => Ok(()),
        r => r,
    };
    vm.call_method(&ctx, "pop_all_dynamic", Vec::new())?;
    let cs = contexts(vm)?;
    vm.call_method(&cs, "pop", Vec::new())?;
    mset(vm, "log", old_log)?;
    vm.call_method(&sb, "restore", Vec::new())?;
    refocus(vm)?;
    let cfg = py::import(vm, "renpy.config")?;
    py::set(vm, &cfg, "skipping", Value::None)?;
    r?;
    let arc = py::config(vm, "after_replay_callback")?;
    if py::truthy(vm, &arc)? {
        vm.call(&arc, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

// Exceptions -------------------------------------------------------------------------------------------

fn ei_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "s", args[1].clone())?;
    py::set(vm, &args[0], "args", args[2].clone())?;
    Ok(Value::None)
}

fn ei_enter(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn ei_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !py::is_none(&args[1]) {
        let s = py::get(vm, &args[0], "s")?;
        let a = py::get(vm, &args[0], "args")?;
        let msg = ops::binary_op(vm, &s, &a, crysirror_python::ast::BinOp::Mod, false, false)?;
        mset(vm, "exception_info", msg)?;
    }
    Ok(Value::Bool(false))
}

fn exc_base_init(vm: &mut Vm, this: &Value, args: Vec<Value>) -> PyResult<()> {
    let e = Value::Obj(vm.types.exc("Exception"));
    let f = py::get(vm, &e, "__init__")?;
    let mut a = vec![this.clone()];
    a.extend(args);
    vm.call(&f, a, Vec::new())?;
    Ok(())
}

fn full_restart_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let reason = arg_or(args, kw, 1, "reason", py::s(vm, "end_game"));
    exc_base_init(vm, &args[0], vec![reason.clone()])?;
    py::set(vm, &args[0], "reason", reason)?;
    Ok(Value::None)
}

fn quit_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    exc_base_init(vm, &args[0], Vec::new())?;
    py::set(vm, &args[0], "relaunch", arg_or(args, kw, 1, "relaunch", Value::Bool(false)))?;
    py::set(vm, &args[0], "status", arg_or(args, kw, 2, "status", Value::Int(0)))?;
    Ok(Value::None)
}

fn call_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    exc_base_init(vm, this, Vec::new())?;
    py::set(vm, this, "label", arg_or(args, kw, 1, "label", Value::None))?;
    py::set(vm, this, "args", arg_or(args, kw, 2, "args", Value::None))?;
    py::set(vm, this, "kwargs", arg_or(args, kw, 3, "kwargs", Value::None))?;
    py::set(vm, this, "from_current", arg_or(args, kw, 4, "from_current", Value::Bool(false)))?;
    Ok(Value::None)
}

fn call_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = mget(vm, "CallException")?;
    let mut a = Vec::new();
    for n in ["label", "args", "kwargs", "from_current"] {
        a.push(py::get_or_none(vm, this, n)?);
    }
    let t = vm.types.new_tuple(a);
    Ok(vm.types.new_tuple(vec![c, t]))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("context", f_context),
        ("invoke_in_new_context", f_invoke_in_new_context),
        ("call_in_new_context", f_call_in_new_context),
        ("call_replay", f_call_replay),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in ["basepath", "args", "script", "interface", "log", "style", "persistent", "preferences"] {
        set(vm, n, Value::None);
    }
    set(vm, "searchpath", vm.types.new_list(Vec::new()));
    set(vm, "contexts", vm.types.new_list(Vec::new()));
    set(vm, "post_init", vm.types.new_list(Vec::new()));
    set(vm, "seen_session", vm.types.new_dict());
    set(vm, "exception_info", py::s(vm, ""));
    for n in ["lint", "after_rollback", "less_memory", "less_updates", "less_mouse", "less_imagedissolve"] {
        set(vm, n, Value::Bool(false));
    }
    for n in ["seen_translates_count", "new_translates_count", "initcode_ast_id"] {
        set(vm, n, Value::Int(0));
    }
    let bi = vm.types.new_dict();
    py::dict_set(vm, &bi, "info", vm.types.new_dict())?;
    set(vm, "build_info", bi);
    let obj = Value::Obj(vm.types.object.clone());
    let ei = py::class(vm, MODULE, "ExceptionInfo", &[obj], &[("__init__", ei_init), ("__enter__", ei_enter), ("__exit__", ei_exit)], &[])?;
    set(vm, "ExceptionInfo", Value::Obj(ei));
    let exc = Value::Obj(vm.types.exc("Exception"));
    let mut control = Vec::new();
    let simple = |vm: &mut Vm, name: &str, control: &mut Vec<Value>| -> PyResult<Value> {
        let c = Value::Obj(py::class(vm, MODULE, name, &[exc.clone()], &[], &[])?);
        crysirror_python::modules::set(vm, &m, name, c.clone());
        control.push(c.clone());
        Ok(c)
    };
    simple(vm, "RestartContext", &mut control)?;
    simple(vm, "RestartTopContext", &mut control)?;
    let fr = Value::Obj(py::class(vm, MODULE, "FullRestartException", &[exc.clone()], &[("__init__", full_restart_init)], &[])?);
    set(vm, "FullRestartException", fr.clone());
    control.push(fr);
    simple(vm, "UtterRestartException", &mut control)?;
    let q = Value::Obj(py::class(vm, MODULE, "QuitException", &[exc.clone()], &[("__init__", quit_init)], &[])?);
    set(vm, "QuitException", q.clone());
    control.push(q);
    simple(vm, "JumpException", &mut control)?;
    simple(vm, "JumpOutException", &mut control)?;
    let ce = Value::Obj(py::class(vm, MODULE, "CallException", &[exc.clone()], &[("__init__", call_init), ("__reduce__", call_reduce)], &[("from_current", Value::Bool(false))])?);
    set(vm, "CallException", ce.clone());
    control.push(ce);
    simple(vm, "EndReplay", &mut control)?;
    simple(vm, "ParseErrorException", &mut control)?;
    control.push(Value::Obj(vm.types.exc("KeyboardInterrupt")));
    set(vm, "CONTROL_EXCEPTIONS", vm.types.new_tuple(control));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
