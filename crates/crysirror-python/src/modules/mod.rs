// SPDX-License-Identifier: MPL-2.0
//! Standard library. Modules whose behaviour is defined by native data
//! structures are implemented in Rust; small pure-Python modules are kept as
//! embedded source (`pysrc`) written for this interpreter.

use crate::object::*;
use crate::vm::Vm;

pub mod binascii;
pub mod collections;
pub mod copyreg;
pub mod functools;
pub mod gc;
mod host_files;
pub mod hashlib;
pub mod io;
pub mod itertools;
pub mod json;
pub mod math;
pub mod operator;
pub mod os;
pub mod pysrc;
pub mod pysrc_host;
pub mod random;
pub mod re;
pub mod struct_;
pub mod sys;
pub mod time;
pub mod weakref;
pub mod zlib;

pub fn install(vm: &mut Vm) {
    let natives: &[(&'static str, crate::vm::ModuleInit)] = &[
        ("sys", sys::init),
        ("math", math::init),
        ("cmath", math::init_cmath),
        ("time", time::init),
        ("_functools", functools::init),
        ("itertools", itertools::init),
        ("operator", operator::init),
        ("_operator", operator::init),
        ("copyreg", copyreg::init),
        ("copy_reg", copyreg::init),
        ("_io", io::init),
        ("fcntl", host_files::fcntl),
        ("tempfile", host_files::tempfile),
        ("_collections", collections::init),
        ("_random", random::init),
        ("_sre", re::init),
        ("_json", json::init),
        ("_struct", struct_::init),
        ("zlib", zlib::init),
        ("_hashlib", hashlib::init),
        ("binascii", binascii::init),
        ("posix", os::init),
        ("_weakref", weakref::init),
        ("gc", gc::init),
        ("errno", os::init_errno),
        ("_pickle", crate::pickle::init),
    ];
    for (n, f) in natives {
        vm.register_native_module(n, *f);
    }
    // sys must exist before anything prints.
    let _ = crate::import::import_module(vm, "sys");
}

pub fn source_module(name: &str) -> Option<&'static str> {
    if name == "logging" { Some(include_str!("logging.py")) } else { pysrc::source(name).or_else(|| pysrc_host::source(name)) }
}

pub fn is_package(name: &str) -> bool {
    pysrc::is_package(name) || pysrc_host::is_package(name)
}

/// Creates a module with the given (name, function) members.
pub fn module_with(vm: &mut Vm, name: &str, funcs: &[(&'static str, NativeFn)]) -> ObjRef {
    let m = vm.new_module(name);
    for (n, f) in funcs {
        let v = vm.types.new_builtin(n, *f);
        crate::ops::set_raw_attr(vm, &m, n, v);
    }
    m
}

pub fn set(vm: &Vm, m: &ObjRef, name: &str, v: Value) {
    crate::ops::set_raw_attr(vm, m, name, v);
}
