// SPDX-License-Identifier: MPL-2.0
//! Pure-Rust Python source interpreter for engine-hosted scripts.
//!
//! Two dialects share one runtime: Python 3 (3.12 grammar) and Python 2.7
//! (statement `print`/`exec`, classic division, list-returning builtins).
//! Source is compiled to a compact bytecode and run on heap frames, so a
//! host function can suspend the running task and resume it later (Ren'Py
//! interactions block inside Python calls).
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::new_without_default)]

pub mod ast;
pub mod builtins;
mod bytes_ext;
pub mod code;
pub mod checkpoint;
pub mod compile;
pub mod dict;
pub mod format;
pub mod gc;
pub mod hash;
pub mod import;
pub mod lexer;
pub mod mangle;
pub mod methods;
pub mod methods_str;
pub mod modules;
pub mod num;
pub mod object;
pub mod ops;
pub mod parser;
pub mod pickle;
pub mod symtable;
pub mod types;
pub mod unicode_names;
pub mod vm;

pub use object::{ObjRef, Payload, Value};
pub use vm::{Host, Kwargs, PyErr, PyResult, Task, TaskOutcome, Vm};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dialect {
    Py2,
    Py3,
}

#[derive(Clone, Debug)]
pub enum Error {
    Syntax { msg: String, filename: String, line: u32 },
    Runtime(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Syntax { msg, filename, line } => write!(f, "{filename}:{line}: SyntaxError: {msg}"),
            Error::Runtime(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

/// Formats an exception with its recorded traceback (like `traceback.format_exc`).
pub fn format_exception(vm: &mut Vm, exc: &ObjRef) -> String {
    let mut out = String::from("Traceback (most recent call last):\n");
    if let Some(Value::Obj(tb)) = ops::get_raw_attr(exc, "__traceback__") {
        if let Payload::List(l) = &tb.payload {
            for e in l.borrow().iter() {
                if let Some(Payload::Tuple(t)) = e.payload() {
                    let f = t[0].as_str().unwrap_or("?");
                    let line = match &t[1] {
                        Value::Int(i) => *i,
                        _ => 0,
                    };
                    let name = t[2].as_str().unwrap_or("?");
                    out.push_str(&format!("  File \"{f}\", line {line}, in {name}\n"));
                }
            }
        }
    }
    let name = exc.class_name();
    let msg = format::exception_str(vm, exc).unwrap_or_default();
    if msg.is_empty() {
        out.push_str(&format!("{name}\n"));
    } else {
        out.push_str(&format!("{name}: {msg}\n"));
    }
    out
}

/// Convenience: compile and run source in a fresh `__main__` namespace.
pub fn run_source(vm: &mut Vm, src: &str, dialect: Dialect) -> Result<ObjRef, String> {
    let code = vm.compile_exec(src, "<string>", dialect).map_err(|e| e.to_string())?;
    let m = vm.new_module("__main__");
    let Payload::Module(_, g) = &m.payload else { unreachable!() };
    let g = g.clone();
    g.dict_set_raw(&vm.types, vm.types.intern("__builtins__"), Value::Obj(vm.builtins_module.clone()));
    match vm.run_code(code, g.clone(), None) {
        Ok(_) => Ok(g),
        Err(PyErr::Exc(e)) => Err(format_exception(vm, &e)),
        Err(PyErr::Suspend(_)) => Err("suspended".into()),
    }
}
