// SPDX-License-Identifier: MPL-2.0
//! `sys`: interpreter state visible to scripts. Version numbers follow the
//! dialect selected by the embedding engine (`set_dialect`).

use super::{module_with, set};
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use crate::Dialect;
use std::cell::RefCell;

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "sys", &[
        ("exc_info", exc_info),
        ("exit", exit),
        ("getrecursionlimit", getrecursionlimit),
        ("setrecursionlimit", setrecursionlimit),
        ("getdefaultencoding", utf8),
        ("getfilesystemencoding", utf8),
        ("_getframe", getframe),
        ("intern", intern),
        ("getsizeof", getsizeof),
        ("getrefcount", getrefcount),
        ("settrace", noop),
        ("setprofile", noop),
        ("gettrace", noop),
        ("exc_clear", noop),
        ("audit", noop),
        ("is_finalizing", falsy),
    ]);
    let out_cls = vm.types.new_builtin_type("TextIOWrapper", &vm.types.object.clone(), BaseKind::Native, false);
    vm.types.add_method(&out_cls, "write", stdout_write);
    vm.types.add_method(&out_cls, "flush", noop);
    vm.types.add_method(&out_cls, "isatty", falsy);
    vm.types.add_method(&out_cls, "fileno", fileno);
    vm.classes.stdout = Some(out_cls.clone());
    for n in ["stdout", "stderr", "__stdout__", "__stderr__"] {
        let o = vm.types.new_native(&out_cls, Box::new(()));
        if let Value::Obj(oo) = &o {
            let e = vm.types.new_str("utf-8");
            ops::set_raw_attr(vm, oo, "encoding", e);
        }
        set(vm, &m, n, o);
    }
    let stdin = vm.types.new_native(&out_cls, Box::new(()));
    set(vm, &m, "stdin", stdin);
    set(vm, &m, "modules", Value::Obj(vm.modules.clone()));
    let path = vm.types.new_list(Vec::new());
    set(vm, &m, "path", path);
    let argv = vm.types.new_list(vec![vm.types.new_str("")]);
    set(vm, &m, "argv", argv);
    set(vm, &m, "maxsize", Value::Int(i64::MAX));
    set(vm, &m, "maxint", Value::Int(i64::MAX));
    set(vm, &m, "maxunicode", Value::Int(0x10ffff));
    let s = vm.types.new_str("little");
    set(vm, &m, "byteorder", s);
    let s = vm.types.new_str("crysirror");
    set(vm, &m, "platform", s);
    let s = vm.types.new_str("");
    set(vm, &m, "executable", s.clone());
    set(vm, &m, "prefix", s.clone());
    set(vm, &m, "exec_prefix", s);
    set(vm, &m, "dont_write_bytecode", Value::Bool(true));
    let e = vm.types.new_list(Vec::new());
    set(vm, &m, "meta_path", e);
    let e = vm.types.new_list(Vec::new());
    set(vm, &m, "path_hooks", e);
    let e = vm.types.new_dict();
    set(vm, &m, "path_importer_cache", e);
    let e = vm.types.new_list(Vec::new());
    set(vm, &m, "warnoptions", e);
    let fi = vm.types.new_tuple(vec![Value::Float(f64::MAX), Value::Int(1024), Value::Int(308), Value::Float(f64::MIN_POSITIVE), Value::Int(-1021), Value::Int(-307), Value::Int(15), Value::Int(53), Value::Float(f64::EPSILON), Value::Int(2), Value::Int(1)]);
    set(vm, &m, "float_info", fi);
    let bm = vm.types.new_tuple(vec![vm.types.new_str("builtins"), vm.types.new_str("sys")]);
    set(vm, &m, "builtin_module_names", bm);
    set_version(vm, &m, Dialect::Py3);
    Ok(m)
}

/// Updates `sys.version*` for the dialect the engine runs scripts in.
pub fn set_dialect(vm: &mut Vm, d: Dialect) {
    vm.dialect_default = d;
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "sys") {
        set_version(vm, &m, d);
    }
}

fn set_version(vm: &mut Vm, m: &ObjRef, d: Dialect) {
    let (maj, min, mic, s, hex) = match d {
        Dialect::Py2 => (2, 7, 18, "2.7.18 (CrySirror)", 0x020712f0),
        Dialect::Py3 => (3, 12, 0, "3.12.0 (CrySirror)", 0x030c00f0),
    };
    let rl = vm.types.new_str("final");
    let vi = vm.types.new_tuple(vec![Value::Int(maj), Value::Int(min), Value::Int(mic), rl, Value::Int(0)]);
    set(vm, m, "version_info", vi.clone());
    let vs = vm.types.new_str(s);
    set(vm, m, "version", vs);
    set(vm, m, "hexversion", Value::Int(hex));
    // `sys.implementation` exists from Python 3.3.
    match d {
        Dialect::Py2 => {
            if let Some(a) = m.attr_table() {
                a.borrow_mut().del_str("implementation");
            }
        }
        Dialect::Py3 => {
            if let Ok(v) = implementation(vm, vi, hex) {
                set(vm, m, "implementation", v);
            }
        }
    }
}

fn implementation(vm: &mut Vm, version: Value, hex: i64) -> PyResult<Value> {
    let ty = Value::Obj(vm.types.type_.clone());
    let name = vm.types.new_str("implementation");
    let bases = vm.types.new_tuple(vec![Value::Obj(vm.types.object.clone())]);
    let ns = vm.types.new_dict();
    let cls = vm.call(&ty, vec![name, bases, ns], Vec::new())?;
    let o = vm.call(&cls, Vec::new(), Vec::new())?;
    for (k, v) in [
        ("name", vm.types.new_str("crysirror")),
        ("cache_tag", vm.types.new_str("crysirror-312")),
        ("version", version),
        ("hexversion", Value::Int(hex)),
    ] {
        let k = vm.types.intern(k);
        ops::setattr(vm, &o, &k, v)?;
    }
    Ok(o)
}

pub fn stdout(vm: &Vm) -> Option<Value> {
    let m = crate::import::get_module(vm, "sys")?;
    m.obj()?.dict_get_str_raw("stdout")
}

fn stdout_write(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = ops::str(vm, a.get(1).unwrap_or(&Value::None))?;
    let n = s.chars().count();
    vm.host.write_stdout(&s);
    Ok(Value::Int(n as i64))
}

fn fileno(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(1))
}

fn noop(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn falsy(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn utf8(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.new_str("utf-8"))
}

fn intern(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = crate::builtins::str_arg(vm, &a[0], "intern")?;
    Ok(vm.types.intern(&s))
}

fn getsizeof(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(64))
}

fn getrefcount(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(match a.first() {
        Some(Value::Obj(o)) => std::rc::Rc::strong_count(o) as i64,
        _ => 2,
    }))
}

fn exc_info(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(match vm.exc_state.clone() {
        Some(e) => {
            let t = Value::Obj(ops::type_of(vm, &e));
            let tb = ops::getattr_opt(vm, &e, "__traceback__")?.unwrap_or(Value::None);
            vm.types.new_tuple(vec![t, e, tb])
        }
        None => vm.types.new_tuple(vec![Value::None, Value::None, Value::None]),
    })
}

fn exit(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = vm.types.exc("SystemExit");
    Err(crate::vm::PyErr::Exc(vm.new_exc(&cls, a.to_vec())))
}

fn getrecursionlimit(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(vm.recursion_limit as i64))
}

fn setrecursionlimit(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[0])?;
    vm.recursion_limit = n.clamp(50, 5000) as usize;
    Ok(Value::None)
}

/// Frame introspection: a snapshot object with f_code/f_globals/f_locals/
/// f_lineno/f_back.
fn getframe(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let depth = match a.first() {
        Some(v) => ops::index_of(vm, v)? as usize,
        None => 0,
    };
    frame_object(vm, depth)
}

pub fn frame_object(vm: &mut Vm, depth: usize) -> PyResult<Value> {
    let n = vm.frames.len();
    if depth >= n {
        return Err(vm.value_error("call stack is not deep enough"));
    }
    let idx = n - 1 - depth;
    let (code, globals, locals, line) = {
        let f = &vm.frames[idx];
        (f.code.clone(), f.globals.clone(), f.locals.clone(), f.code.line_at(f.ip.saturating_sub(1)))
    };
    let cls = frame_class(vm);
    let o = vm.types.new_native(&cls, Box::new(RefCell::new(idx)));
    let Value::Obj(oo) = &o else { unreachable!() };
    let cv = vm.types.new_code(code);
    ops::set_raw_attr(vm, oo, "f_code", cv);
    ops::set_raw_attr(vm, oo, "f_globals", Value::Obj(globals.clone()));
    let l = match locals {
        Some(l) => l,
        None => Value::Obj(globals),
    };
    ops::set_raw_attr(vm, oo, "f_locals", l);
    ops::set_raw_attr(vm, oo, "f_lineno", Value::Int(line as i64));
    let back = if idx > 0 { frame_object(vm, depth + 1)? } else { Value::None };
    ops::set_raw_attr(vm, oo, "f_back", back);
    Ok(o)
}

fn frame_class(vm: &mut Vm) -> ObjRef {
    if let Some(Value::Obj(c)) = vm.builtins.dict_get_str_raw("__frame_type__") {
        return c;
    }
    let c = vm.types.new_builtin_type("frame", &vm.types.object.clone(), BaseKind::Native, false);
    let k = vm.types.intern("__frame_type__");
    vm.builtins.dict_set_raw(&vm.types, k, Value::Obj(c.clone()));
    c
}
