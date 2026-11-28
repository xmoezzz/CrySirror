// SPDX-License-Identifier: MPL-2.0
//! `renpy.log`, after Ren'Py 8.3 (MIT): named log files (`log`,
//! `image_cache`, `text_overflow`, ...) opened on first write, with the
//! release header, `%`-formatted lines and `exception()` tracebacks.
//!
//! The player has no log directory: each log keeps its most recent lines
//! in memory (`lines(vm, name)`), and lines of the main `log` are also
//! reported to the owner as link diagnostics. `config.log_enable`,
//! `config.log_to_stdout` and developer-only logs behave as upstream;
//! stdout/stderr are already captured by the interpreter host, so
//! `post_init` has no streams to restore.
use super::{link, py};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};

const MODULE: &str = "renpy.log";
/// Lines kept per log file.
const KEEP: usize = 2048;

#[derive(Default)]
struct Sinks(HashMap<String, VecDeque<String>>);

fn sinks(vm: &Vm) -> Rc<RefCell<Sinks>> {
    vm.ext_get::<Rc<RefCell<Sinks>>>()
        .expect("renpy.log sinks")
        .clone()
}

/// The retained lines of log `name` (without the trailing newline).
pub fn lines(vm: &Vm, name: &str) -> Vec<String> {
    vm.ext_get::<Rc<RefCell<Sinks>>>()
        .map(|s| {
            s.borrow()
                .0
                .get(name)
                .map(|l| l.iter().cloned().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

pub(crate) fn push(vm: &Vm, name: &str, text: &str) {
    let s = sinks(vm);
    let mut s = s.borrow_mut();
    let buf = s.0.entry(name.to_owned()).or_default();
    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if buf.len() >= KEEP {
            buf.pop_front();
        }
        buf.push_back(line.to_owned());
        if name == "log" {
            link::diagnostic(vm, format!("renpy log: {line}"));
        }
    }
}

/// `renpy.display.log.write(fmt, *args)`, ignoring a missing log.
pub fn write(vm: &mut Vm, msg: &str) {
    if let Ok(d) = py::import(vm, "renpy.display")
        && let Ok(Some(l)) = py::get_opt(vm, &d, "log")
    {
        let s = py::s(vm, "%s");
        let m = py::s(vm, msg);
        let _ = py::method(vm, &l, "write", vec![s, m]);
    }
}

fn this_name(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let n = py::get(vm, this, "name")?;
    py::string(vm, &n)
}

fn lf_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(
        vm,
        this,
        "name",
        py::arg_or(args, kw, 1, "name", Value::None),
    )?;
    py::set(
        vm,
        this,
        "append",
        py::arg_or(args, kw, 2, "append", Value::Bool(false)),
    )?;
    py::set(
        vm,
        this,
        "developer",
        py::arg_or(args, kw, 3, "developer", Value::Bool(false)),
    )?;
    py::set(
        vm,
        this,
        "flush",
        py::arg_or(args, kw, 4, "flush", Value::Bool(true)),
    )?;
    py::set(vm, this, "file", Value::None)?;
    py::set(vm, this, "softspace", Value::Int(0))?;
    py::set(vm, this, "newlines", Value::None)?;
    py::set(vm, this, "raw_write", Value::Bool(false))?;
    Ok(Value::None)
}

fn header(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let mut out = Vec::new();
    if {
        let t__ = py::get(vm, this, "append")?;
        py::truthy(vm, &t__)
    }? {
        out.extend([String::new(), "=".repeat(78), String::new()]);
    }
    let time = py::import(vm, "time")?;
    let gm = py::method(vm, &time, "gmtime", Vec::new())?;
    let f = py::s(vm, "%Y-%m-%d %H:%M:%S");
    let stamp = py::method(vm, &time, "strftime", vec![f, gm])?;
    out.push(format!("{} UTC", py::string(vm, &stamp)?));
    out.push("CrySirror".into());
    let v = py::import_attr(vm, "renpy", "version")?;
    out.push(py::string(vm, &v)?);
    out.push(String::new());
    let bi = py::import_attr(vm, "renpy.game", "build_info")?;
    let key = py::s(vm, "name");
    if ops::contains(vm, &bi, &key)? {
        for k in ["name", "version"] {
            let kv = py::s(vm, k);
            let v = ops::getitem(vm, &bi, kv)?;
            out.push(ops::str(vm, &v)?.to_string());
        }
        out.push(String::new());
    }
    let name = this_name(vm, this)?;
    for l in out {
        push(vm, &name, &l);
    }
    Ok(())
}

/// `LogFile.open()`: `file` is True once the in-memory sink is open,
/// False after a refusal that upstream would not retry.
fn lf_open(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if py::config_bool(vm, "log_to_stdout")? {
        py::set(vm, this, "file", Value::Bool(true))?;
        return Ok(Value::Bool(true));
    }
    match py::get(vm, this, "file")? {
        Value::Bool(true) => return Ok(Value::Bool(true)),
        Value::Bool(false) => return Ok(Value::Bool(false)),
        _ => {}
    }
    if {
        let t__ = py::get(vm, this, "developer")?;
        py::truthy(vm, &t__)
    }? && !py::config_bool(vm, "developer")?
    {
        return Ok(Value::Bool(false));
    }
    if let Some(v) = {
        let t__ = py::import(vm, "renpy.config")?;
        ops::getattr_opt(vm, &t__, "log_enable")
    }? && !py::truthy(vm, &v)?
    {
        return Ok(Value::Bool(false));
    }
    py::set(vm, this, "file", Value::Bool(true))?;
    header(vm, this)?;
    Ok(Value::Bool(true))
}

fn lf_write(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let opened = py::method(vm, this, "open", Vec::new())?;
    if !py::truthy(vm, &opened)? {
        return Ok(Value::None);
    }
    let mut s = args.get(1).cloned().unwrap_or_else(|| py::s(vm, ""));
    if let Some(b) = ops::bytes_of(&s)
        && s.as_str().is_none()
    {
        s = vm
            .types
            .new_str(&b.iter().map(|&c| c as char).collect::<String>());
    }
    let mut text = py::string(vm, &s)?;
    if !{
        let t__ = py::get(vm, this, "raw_write")?;
        py::truthy(vm, &t__)
    }? {
        let rest = vm.types.new_tuple(args[2..].to_vec());
        text = match ops::binary_op(
            vm,
            &s,
            &rest,
            crysirror_python::ast::BinOp::Mod,
            false,
            false,
        ) {
            Ok(v) => py::string(vm, &v)?,
            Err(_) => {
                let mut all = vec![s.clone()];
                all.extend(args[2..].iter().cloned());
                let t = vm.types.new_tuple(all);
                ops::repr(vm, &t)?
            }
        };
        text.push('\n');
    }
    let name = this_name(vm, this)?;
    if py::config_bool(vm, "log_to_stdout")? {
        vm.host.write_stdout(&text);
    }
    push(vm, &name, &text);
    Ok(Value::None)
}

fn lf_exception(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let tb = py::import_attr(vm, "traceback", "format_exc")?;
    let text = vm.call(&tb, Vec::new(), Vec::new())?;
    py::set(vm, this, "raw_write", Value::Bool(true))?;
    let r = py::method(vm, this, "write", vec![text]);
    py::set(vm, this, "raw_write", Value::Bool(false))?;
    r
}

fn f_open(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(args, kw, 0, "name", Value::None);
    let cache = py::import_attr(vm, MODULE, "log_cache")?;
    let d = cache
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("log_cache"))?;
    if let Some(v) = ops::dict_get(vm, &d, &name)? {
        return Ok(v);
    }
    let cls = py::import_attr(vm, MODULE, "LogFile")?;
    let mut kwargs = Vec::new();
    for (i, n, def) in [
        (1, "append", false),
        (2, "developer", false),
        (3, "flush", false),
    ] {
        kwargs.push((Rc::from(n), py::arg_or(args, kw, i, n, Value::Bool(def))));
    }
    let rv = vm.call(&cls, vec![name.clone()], kwargs)?;
    ops::dict_set(vm, &d, name, rv.clone())?;
    Ok(rv)
}

fn tl_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "duration",
        py::arg_or(args, kw, 1, "duration", Value::None),
    )?;
    Ok(Value::None)
}

fn tl_append(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let now = py::f(super::link::now(vm));
    let item = vm.types.new_tuple(vec![now.clone(), args[1].clone()]);
    let list = Value::Obj(vm.types.list.clone());
    py::method(vm, &list, "append", vec![this.clone(), item])?;
    py::method(vm, this, "prune", vec![now])
}

fn tl_prune(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let now = match py::arg(args, kw, 1, "now") {
        Some(v) if !py::is_none(v) => py::f64_of(vm, v)?,
        _ => super::link::now(vm),
    };
    let duration = py::get(vm, this, "duration")?;
    let duration = py::f64_of(vm, &duration)?;
    while ops::len(vm, this)? > 0 {
        let first = ops::getitem(vm, this, Value::Int(0))?;
        let t = ops::getitem(vm, &first, Value::Int(0))?;
        if py::f64_of(vm, &t)? >= now - duration {
            break;
        }
        py::method(vm, this, "pop", vec![Value::Int(0)])?;
    }
    Ok(Value::None)
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    if vm.ext_get::<Rc<RefCell<Sinks>>>().is_none() {
        vm.ext_set(Rc::new(RefCell::new(Sinks::default())));
    }
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("open", f_open), ("post_init", noop)],
    );
    let lf = py::class(
        vm,
        MODULE,
        "LogFile",
        &[],
        &[
            ("__init__", lf_init),
            ("open", lf_open),
            ("write", lf_write),
            ("exception", lf_exception),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "LogFile", Value::Obj(lf));
    let list = Value::Obj(vm.types.list.clone());
    let tl = py::class(
        vm,
        MODULE,
        "TimeLog",
        &[list],
        &[
            ("__init__", tl_init),
            ("append", tl_append),
            ("prune", tl_prune),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "TimeLog", Value::Obj(tl));
    crysirror_python::modules::set(vm, &m, "log_cache", vm.types.new_dict());
    crysirror_python::modules::set(vm, &m, "log_file", Value::None);
    let sys = py::import(vm, "sys")?;
    for (n, s) in [
        ("real_stdout", "stdout"),
        ("real_stderr", "stderr"),
        ("sys_stdout", "stdout"),
        ("sys_stderr", "stderr"),
    ] {
        let v = py::get_or_none(vm, &sys, s)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
