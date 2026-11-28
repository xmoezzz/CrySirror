// SPDX-License-Identifier: MPL-2.0
//! `renpy.test`, `renpy.test.testast` (`TestSettings`, the store's
//! `_test`) and `renpy.test.testexecution`, after Ren'Py 8.3 (MIT).
//!
//! Test cases run only under the launcher's `test` command. The player
//! has no command line, so `testcase` blocks are registered by name (the
//! parser keeps no test statements) and `execute()` returns at once as
//! upstream does with no active test; starting the `test` command
//! reports that the test runner is not part of this runtime.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const AST: &str = "renpy.test.testast";
const EXEC: &str = "renpy.test.testexecution";

fn ts_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "maximum_framerate", Value::Bool(true))?;
    py::set(vm, this, "timeout", py::f(5.0))?;
    py::set(vm, this, "force", Value::Bool(false))?;
    py::set(vm, this, "transition_timeout", py::f(5.0))?;
    Ok(Value::None)
}

fn init_testast(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, AST, &[]);
    let base = py::import_attr(vm, "renpy.object", "Object")?;
    let ts = py::class(
        vm,
        AST,
        "TestSettings",
        &[base],
        &[("__init__", ts_init)],
        &[],
    )?;
    let t = vm.call(&Value::Obj(ts.clone()), Vec::new(), Vec::new())?;
    crysirror_python::modules::set(vm, &m, "TestSettings", Value::Obj(ts));
    crysirror_python::modules::set(vm, &m, "_test", t);
    Ok(m)
}

fn f_take_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let node = py::import_attr(vm, EXEC, "node")?;
    if py::is_none(&node) {
        return Ok(Value::None);
    }
    if args[0].as_str().is_some() {
        let labels = py::import_attr(vm, EXEC, "labels")?;
        ops::set_add(vm, &labels, args[0].clone())?;
    }
    Ok(Value::None)
}

fn tj_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "node", args[1].clone())?;
    Ok(Value::None)
}

fn f_lookup(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tc = py::import_attr(vm, EXEC, "testcases")?;
    if ops::contains(vm, &tc, &args[0])? {
        return ops::getitem(vm, &tc, args[0].clone());
    }
    let name = ops::str(vm, &args[0])?;
    let f = py::get(vm, &args[1], "filename")?;
    let l = py::get(vm, &args[1], "linenumber")?;
    let msg = format!(
        "Testcase {name} not found at {}:{}.",
        ops::str(vm, &f)?,
        ops::str(vm, &l)?
    );
    Err(py::error(vm, msg))
}

fn not_available(vm: &mut Vm) -> crysirror_python::PyErr {
    py::error(vm, "The Ren'Py test runner is not part of this runtime.")
}

fn f_execute(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let node = py::import_attr(vm, EXEC, "node")?;
    if py::is_none(&node) {
        return Ok(Value::None);
    }
    Err(not_available(vm))
}

fn f_test_command(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(not_available(vm))
}

fn init_testexecution(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        EXEC,
        &[
            ("take_name", f_take_name),
            ("lookup", f_lookup),
            ("execute", f_execute),
            ("execute_node", f_test_command),
            ("test_command", f_test_command),
        ],
    );
    let exc = Value::Obj(vm.types.exc("Exception"));
    let tj = py::class(vm, EXEC, "TestJump", &[exc], &[("__init__", tj_init)], &[])?;
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "TestJump", Value::Obj(tj));
    set(vm, "testcases", vm.types.new_dict());
    for n in [
        "node",
        "node_loc",
        "state",
        "old_state",
        "old_loc",
        "start_time",
        "action",
    ] {
        set(vm, n, Value::None);
    }
    set(vm, "last_state_change", Value::Int(0));
    set(vm, "labels", vm.types.new_set(Default::default()));
    let reg = py::import_attr(vm, "renpy.arguments", "register_command")?;
    let name = py::s(vm, "test");
    let f = py::get_raw(&m, "test_command");
    vm.call(
        &reg,
        vec![name, f],
        vec![("uses_display".into(), Value::Bool(true))],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(AST, init_testast);
    vm.register_native_module(EXEC, init_testexecution);
}
