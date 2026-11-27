// SPDX-License-Identifier: MPL-2.0
//! File operations backed by explicit VM host capabilities.
use super::{io, module_with, set};
use crate::{
    builtins::Kw,
    object::{ObjRef, Value},
    vm::{PyResult, Vm},
};

const LOCK_EX: i64 = 2;
const LOCK_NB: i64 = 4;
const LOCK_UN: i64 = 8;

pub fn fcntl(vm: &mut Vm) -> PyResult<ObjRef> {
    let module = module_with(vm, "fcntl", &[("lockf", lockf)]);
    for (name, value) in [
        ("LOCK_EX", LOCK_EX),
        ("LOCK_NB", LOCK_NB),
        ("LOCK_UN", LOCK_UN),
    ] {
        set(vm, &module, name, Value::Int(value));
    }
    Ok(module)
}

fn lockf(vm: &mut Vm, args: &[Value], kwargs: Kw) -> PyResult<Value> {
    if !kwargs.is_empty() {
        return Err(vm.type_error("lockf does not accept keywords"));
    }
    let [file, Value::Int(op)] = args else {
        return Err(vm.type_error("lockf expects a file object and an operation"));
    };
    match *op {
        LOCK_UN => io::file_lock(vm, file, false),
        bits if bits == LOCK_EX | LOCK_NB => io::file_lock(vm, file, true),
        _ => Err(vm.exc_msg(
            "NotImplementedError",
            "lockf supports nonblocking exclusive whole-file locks",
        )),
    }
}

pub fn tempfile(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(module_with(vm, "tempfile", &[("gettempdir", gettempdir)]))
}

fn gettempdir(vm: &mut Vm, args: &[Value], kwargs: Kw) -> PyResult<Value> {
    if !args.is_empty() || !kwargs.is_empty() {
        return Err(vm.type_error("gettempdir takes no arguments"));
    }
    match vm.host.temporary_directory() {
        Some(path) => Ok(vm.types.new_str(path)),
        None => Err(vm.exc_msg("FileNotFoundError", "no scoped temporary directory")),
    }
}
