// SPDX-License-Identifier: MPL-2.0
//! `renpy.python`, after Ren'Py 8.3 `python.py` (MIT): named stores
//! (`StoreDict` namespaces with change tracking for rollback, exposed as
//! store modules), store backups for restart, and compiling/evaluating
//! script Python in the store. Compilation is `crate::pycompile` (the
//! wrap_node transform and the per-file Python 2/3 dialect).
//!
//! Rollback classes are re-exported from `renpy.rollback`, where they
//! moved in 8.0, so pre-8.0 pickles naming `renpy.python.RollbackLog`
//! load.
use super::py::{self, *};
use crate::lexer::PyExpr;
use crysirror_python::{Dialect, ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MODULE: &str = "renpy.python";

thread_local! {
    static CACHE: RefCell<HashMap<(u32, Rc<str>, Rc<str>, &'static str, u8), Rc<crysirror_python::code::Code>>> = RefCell::new(HashMap::new());
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

// StoreDict --------------------------------------------------------------------------------------------

fn copy_dict(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    let rv = vm.types.new_dict();
    for (k, v) in ops::mapping_items(vm, d)? {
        ops::setitem(vm, &rv, k, v)?;
    }
    Ok(rv)
}

fn sd_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let old = vm.types.new_dict();
    py::set(vm, this, "old", old)?;
    py::set(
        vm,
        this,
        "ever_been_changed",
        vm.types.new_set(Default::default()),
    )?;
    Ok(Value::None)
}

fn sd_reduce(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "Cannot pickle a reference to a store dictionary.",
    ))
}

fn dict_method(vm: &mut Vm, this: &Value, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let d = Value::Obj(vm.types.dict.clone());
    let f = py::get(vm, &d, name)?;
    let mut a = vec![this.clone()];
    a.extend(args);
    vm.call(&f, a, Vec::new())
}

fn sd_reset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(
        vm,
        this,
        "ever_been_changed",
        vm.types.new_set(Default::default()),
    )?;
    dict_method(vm, this, "clear", Vec::new())?;
    py::set(vm, this, "old", vm.types.new_dict())?;
    Ok(Value::None)
}

fn is_constant(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let k = py::s(vm, "_constant");
    match this
        .obj()
        .filter(|o| matches!(o.payload, crysirror_python::Payload::Dict(_)))
    {
        Some(o) => match ops::dict_get(vm, o, &k)? {
            Some(v) => py::truthy(vm, &v),
            None => Ok(false),
        },
        None => Ok(false),
    }
}

fn sd_begin(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if is_constant(vm, this)? {
        return Ok(Value::None);
    }
    let old = copy_dict(vm, this)?;
    py::set(vm, this, "old", old)?;
    Ok(Value::None)
}

/// `find_changes(old, new, deleted)`: the previous value of every key
/// whose value object changed, `deleted` for new keys; None when equal.
fn find_changes(vm: &mut Vm, old: &Value, new: &Value, deleted: &Value) -> PyResult<Option<Value>> {
    let rv = vm.types.new_dict();
    let mut any = false;
    for (k, v) in ops::mapping_items(vm, new)? {
        let o = match old.obj() {
            Some(od) => ops::dict_get(vm, od, &k)?,
            None => None,
        };
        match o {
            None => {
                ops::setitem(vm, &rv, k, deleted.clone())?;
                any = true;
            }
            Some(o) if !o.is(&v) => {
                ops::setitem(vm, &rv, k, o)?;
                any = true;
            }
            _ => {}
        }
    }
    for (k, v) in ops::mapping_items(vm, old)? {
        if !ops::contains(vm, new, &k)? {
            ops::setitem(vm, &rv, k, v)?;
            any = true;
        }
    }
    Ok(any.then_some(rv))
}

fn sd_get_changes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cycle = py::truthy(vm, &arg_or(args, kw, 1, "cycle", Value::Bool(false)))?;
    let previous = arg_or(args, kw, 2, "previous", Value::None);
    if is_constant(vm, this)? {
        return Ok(Value::None);
    }
    let new = copy_dict(vm, this)?;
    let old = py::get(vm, this, "old")?;
    let deleted = py::import_attr(vm, "renpy.rollback", "deleted")?;
    let Some(rv) = find_changes(vm, &old, &new, &deleted)? else {
        return Ok(Value::None);
    };
    if cycle {
        py::set(vm, this, "old", new)?;
    }
    if !py::is_none(&previous) {
        vm.call_method(&rv, "update", vec![previous])?;
    }
    let delta = vm.types.new_set(Default::default());
    if cycle {
        let ebc = py::get(vm, this, "ever_been_changed")?;
        for k in ops::collect(vm, &rv)? {
            if !ops::set_contains(vm, &ebc, &k)? {
                ops::set_add(vm, &ebc, k.clone())?;
                ops::set_add(vm, &delta, k)?;
            }
        }
    }
    Ok(vm.types.new_tuple(vec![rv, delta]))
}

fn f_begin_stores(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let sds = mget(vm, "store_dicts")?;
    let vals = vm.call_method(&sds, "values", Vec::new())?;
    for sd in ops::collect(vm, &vals)? {
        vm.call_method(&sd, "begin", Vec::new())?;
    }
    Ok(Value::None)
}

// StoreModule ------------------------------------------------------------------------------------------

fn sm_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = mget(vm, "get_store_module")?;
    let n = py::get(vm, &args[0], "__name__")?;
    let t = vm.types.new_tuple(vec![n]);
    Ok(vm.types.new_tuple(vec![f, t]))
}

fn f_get_store_module(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    crysirror_python::import::get_module(vm, &name).ok_or_else(|| vm.exc_msg("KeyError", &name))
}

pub fn create_store(vm: &mut Vm, name: &str) -> PyResult<()> {
    if name == "store.store" {
        return Err(vm.exc_msg("NameError", "Namespaces may not begin with \"store\"."));
    }
    let (parent, var) = match name.rsplit_once('.') {
        Some((p, v)) => (Some(p.to_owned()), v.to_owned()),
        None => (None, name.to_owned()),
    };
    if let Some(p) = &parent {
        create_store(vm, p)?;
    }
    let inited = mget(vm, "initialized_store_dicts")?;
    let key = py::s(vm, name);
    if ops::set_contains(vm, &inited, &key)? {
        return Ok(());
    }
    ops::set_add(vm, &inited, key.clone())?;
    let sds = mget(vm, "store_dicts")?;
    let d = match ops::dict_get(vm, sds.obj().unwrap(), &key)? {
        Some(d) => d,
        None => {
            let c = mget(vm, "StoreDict")?;
            let d = vm.call(&c, Vec::new(), Vec::new())?;
            ops::setitem(vm, &sds, key.clone(), d.clone())?;
            d
        }
    };
    vm.call_method(&d, "reset", Vec::new())?;
    py::dict_set(vm, &d, "__name__", key.clone())?;
    py::dict_set(vm, &d, "__package__", key.clone())?;
    let b = crysirror_python::import::import_module(vm, "builtins")?;
    py::dict_set(vm, &d, "__builtins__", b)?;
    let minstore = py::import(vm, "renpy.minstore")?;
    let skip = [
        "__all__",
        "__name__",
        "__doc__",
        "__package__",
        "__loader__",
        "__spec__",
        "__file__",
        "__cached__",
        "__builtins__",
    ];
    let ms_dict = py::namespace_of(vm, &minstore)?;
    for (k, v) in ops::mapping_items(vm, &ms_dict)? {
        if k.as_str().is_some_and(|k| skip.contains(&k)) || ops::contains(vm, &d, &k)? {
            continue;
        }
        ops::setitem(vm, &d, k, v)?;
    }
    let sms = mget(vm, "store_modules")?;
    let module = match ops::dict_get(vm, sms.obj().unwrap(), &key)? {
        Some(m) => {
            let modules = Value::Obj(vm.modules.clone());
            ops::setitem(vm, &modules, key.clone(), m.clone())?;
            m
        }
        None => {
            let m = Value::Obj(vm.new_module_with_namespace(name, d.obj().cloned().unwrap()));
            ops::setitem(vm, &sms, key.clone(), m.clone())?;
            m
        }
    };
    if let Some(p) = parent {
        let pk = py::s(vm, &p);
        let pd = ops::getitem(vm, &sds, pk)?;
        py::dict_set(vm, &pd, &var, module)?;
    }
    Ok(())
}

fn f_create_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    create_store(vm, &n)?;
    Ok(Value::None)
}

/// The dict of store `name` (`store_dicts[name]`).
pub fn store_dict(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let sds = mget(vm, "store_dicts")?;
    let k = py::s(vm, name);
    match ops::dict_get(vm, sds.obj().unwrap(), &k)? {
        Some(d) => Ok(d),
        None => Err(vm.exc_msg("KeyError", name)),
    }
}

// Store backups ----------------------------------------------------------------------------------------

fn sb_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for n in ["store", "old", "ever_been_changed"] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    let sds = mget(vm, "store_dicts")?;
    for (k, v) in ops::mapping_items(vm, &sds)? {
        if !is_constant(vm, &v)? {
            vm.call_method(this, "backup_one", vec![k])?;
        }
    }
    Ok(Value::None)
}

fn sb_backup_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = args[1].clone();
    let sds = mget(vm, "store_dicts")?;
    let d = ops::getitem(vm, &sds, name.clone())?;
    let s = py::get(vm, this, "store")?;
    let c = copy_dict(vm, &d)?;
    ops::setitem(vm, &s, name.clone(), c)?;
    let o = py::get(vm, this, "old")?;
    let old = py::get(vm, &d, "old")?;
    let oc = copy_dict(vm, &old)?;
    ops::setitem(vm, &o, name.clone(), oc)?;
    let e = py::get(vm, this, "ever_been_changed")?;
    let ebc = py::get(vm, &d, "ever_been_changed")?;
    let set = Value::Obj(vm.types.set.clone());
    let ec = vm.call(&set, vec![ebc], Vec::new())?;
    ops::setitem(vm, &e, name, ec)?;
    Ok(Value::None)
}

fn sb_restore_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = args[1].clone();
    let sds = mget(vm, "store_dicts")?;
    let sd = ops::getitem(vm, &sds, name.clone())?;
    dict_method(vm, &sd, "clear", Vec::new())?;
    let s = py::get(vm, this, "store")?;
    let saved = ops::getitem(vm, &s, name.clone())?;
    dict_method(vm, &sd, "update", vec![saved])?;
    let o = py::get(vm, this, "old")?;
    let old = ops::getitem(vm, &o, name.clone())?;
    let oc = copy_dict(vm, &old)?;
    py::set(vm, &sd, "old", oc)?;
    let ebc = py::get(vm, &sd, "ever_been_changed")?;
    vm.call_method(&ebc, "clear", Vec::new())?;
    let e = py::get(vm, this, "ever_been_changed")?;
    let saved = ops::getitem(vm, &e, name)?;
    vm.call_method(&ebc, "update", vec![saved])?;
    Ok(Value::None)
}

fn sb_restore(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::get(vm, &args[0], "store")?;
    for k in ops::collect(vm, &s)? {
        vm.call_method(&args[0], "restore_one", vec![k])?;
    }
    Ok(Value::None)
}

fn f_make_clean_stores(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let sds = mget(vm, "store_dicts")?;
    let vals = vm.call_method(&sds, "values", Vec::new())?;
    for v in ops::collect(vm, &vals)? {
        let ebc = py::get(vm, &v, "ever_been_changed")?;
        vm.call_method(&ebc, "clear", Vec::new())?;
        vm.call_method(&v, "begin", Vec::new())?;
    }
    let c = mget(vm, "StoreBackup")?;
    let b = vm.call(&c, Vec::new(), Vec::new())?;
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, "clean_store_backup", b)?;
    Ok(Value::None)
}

fn backup(vm: &mut Vm) -> PyResult<Value> {
    let b = mget(vm, "clean_store_backup")?;
    if py::is_none(&b) {
        return Err(py::error(
            vm,
            "The stores have no clean backup (make_clean_stores was not called).",
        ));
    }
    Ok(b)
}

fn f_clean_stores(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let b = backup(vm)?;
    vm.call_method(&b, "restore", Vec::new())
}

fn store_name(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let n = py::string(vm, v)?;
    Ok(py::s(
        vm,
        &if n.starts_with("store.") || n == "store" {
            n
        } else {
            format!("store.{n}")
        },
    ))
}

fn f_clean_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = store_name(vm, &args[0])?;
    let b = backup(vm)?;
    vm.call_method(&b, "restore_one", vec![n])
}

fn f_reset_store_changes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = store_name(vm, &args[0])?;
    let sds = mget(vm, "store_dicts")?;
    let sd = ops::getitem(vm, &sds, n)?;
    vm.call_method(&sd, "begin", Vec::new())
}

// Compiling and running --------------------------------------------------------------------------------

/// Python dialect of a file (`file_compiler_flags`) or the release default.
fn dialect_for(vm: &mut Vm, filename: &str, py: Option<i64>) -> PyResult<Dialect> {
    if let Some(p) = py {
        return Ok(if p >= 3 { Dialect::Py3 } else { Dialect::Py2 });
    }
    let flags = mget(vm, "file_compiler_flags")?;
    let k = py::s(vm, filename);
    if let Some(f) = ops::dict_get(vm, flags.obj().unwrap(), &k)? {
        if py::int(vm, &f)? != 0 {
            return Ok(Dialect::Py3);
        }
    }
    Ok(super::state::dialect(vm))
}

pub fn compile(
    vm: &mut Vm,
    source: &str,
    mode: &'static str,
    filename: &str,
    lineno: u32,
    py: Option<i64>,
) -> PyResult<Rc<crysirror_python::code::Code>> {
    let dialect = dialect_for(vm, filename, py)?;
    let key = (
        lineno,
        Rc::<str>::from(filename),
        Rc::<str>::from(source),
        mode,
        dialect as u8,
    );
    if let Some(c) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(c);
    }
    let e = PyExpr::new(source, filename.into(), lineno);
    let c = crate::pycompile::compile(vm, &e, mode, dialect)
        .map_err(|m| vm.syntax_error(crysirror_python::Error::Runtime(m)))?;
    CACHE.with(|cc| cc.borrow_mut().insert(key, c.clone()));
    Ok(c)
}

fn mode_of(m: &str) -> &'static str {
    match m {
        "eval" => "eval",
        "hide" => "hide",
        _ => "exec",
    }
}

/// `py_compile(source, mode, filename, lineno, ast_node, cache, py)`.
fn f_py_compile(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let source = arg_or(args, kw, 0, "source", Value::None);
    if matches!(source.payload(), Some(crysirror_python::Payload::Code(_))) {
        return Ok(source);
    }
    let mode = arg_or(args, kw, 1, "mode", py::s(vm, "exec"));
    let mode = mode_of(&py::string(vm, &mode)?);
    let mut filename = py::string(vm, &arg_or(args, kw, 2, "filename", py::s(vm, "<none>")))?;
    let mut lineno = py::int(vm, &arg_or(args, kw, 3, "lineno", Value::Int(1)))? as u32;
    let mut pyv = match arg(args, kw, 6, "py") {
        Some(v) if !py::is_none(v) => Some(py::int(vm, v)?),
        _ => None,
    };
    // A renpy.ast.PyExpr carries its own location and dialect.
    if let Some(f) = py::get_opt(vm, &source, "filename")? {
        if !py::is_none(&f) && source.as_str().is_some() {
            filename = py::string(vm, &f)?;
            let l = py::get_or_none(vm, &source, "linenumber")?;
            lineno = py::int(vm, &l).unwrap_or(1) as u32;
            if pyv.is_none() {
                let p = py::get_or_none(vm, &source, "py")?;
                if !py::is_none(&p) {
                    pyv = Some(py::int(vm, &p)?);
                }
            }
        }
    }
    let text = py::string(vm, &source)?;
    let text = if mode == "eval" {
        text.trim().to_owned()
    } else {
        text
    };
    let c = compile(vm, &text, mode, &filename, lineno.max(1), pyv)?;
    Ok(vm.types.new_code(c))
}

fn compile_with(vm: &mut Vm, args: &[Value], kw: Kw, mode: &'static str) -> PyResult<Value> {
    let mut k: Vec<(Rc<str>, Value)> = kw.to_vec();
    k.retain(|(n, _)| &**n != "mode");
    let mut a = vec![args[0].clone(), py::s(vm, mode)];
    if mode == "eval" {
        let t = py::string(vm, &args[0])?;
        if args[0].as_str().is_some() && py::get_opt(vm, &args[0], "filename")?.is_none() {
            a[0] = py::s(vm, t.trim());
        }
    }
    f_py_compile(vm, &a, &k)
}

fn f_py_compile_exec_bytecode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    compile_with(vm, args, kw, "exec")
}

fn f_py_compile_hide_bytecode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    compile_with(vm, args, kw, "hide")
}

fn f_py_compile_eval_bytecode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    compile_with(vm, args, kw, "eval")
}

fn code_of(
    vm: &mut Vm,
    v: &Value,
    mode: &'static str,
) -> PyResult<Rc<crysirror_python::code::Code>> {
    match v.payload() {
        Some(crysirror_python::Payload::Code(c)) => Ok(c.clone()),
        _ => {
            let c = f_py_compile(vm, &[v.clone(), py::s(vm, mode)], &[])?;
            match c.payload() {
                Some(crysirror_python::Payload::Code(c)) => Ok(c.clone()),
                _ => Err(vm.type_error("code expected")),
            }
        }
    }
}

fn globals_of(vm: &mut Vm, g: &Value, store: &str) -> PyResult<ObjRef> {
    let g = if py::is_none(g) {
        store_dict(vm, store)?
    } else {
        g.clone()
    };
    g.obj()
        .cloned()
        .ok_or_else(|| vm.type_error("globals must be a dict"))
}

/// `py_exec_bytecode(bytecode, hide, globals, locals, store)`.
pub fn exec_bytecode(
    vm: &mut Vm,
    code: Rc<crysirror_python::code::Code>,
    hide: bool,
    globals: &Value,
    locals: &Value,
    store: &str,
) -> PyResult<()> {
    let g = globals_of(vm, globals, store)?;
    let l = if hide {
        Some(vm.types.new_dict())
    } else if py::is_none(locals) {
        None
    } else {
        Some(locals.clone())
    };
    vm.run_code(code, g, l)?;
    Ok(())
}

fn f_py_exec_bytecode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let code = code_of(vm, &args[0], "exec")?;
    let hide = py::truthy(vm, &arg_or(args, kw, 1, "hide", Value::Bool(false)))?;
    let g = arg_or(args, kw, 2, "globals", Value::None);
    let l = arg_or(args, kw, 3, "locals", Value::None);
    let store = arg_or(args, kw, 4, "store", py::s(vm, "store"));
    let store = py::string(vm, &store)?;
    exec_bytecode(vm, code, hide, &g, &l, &store)?;
    Ok(Value::None)
}

fn f_py_exec(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let code = code_of(vm, &args[0], "exec")?;
    let hide = py::truthy(vm, &arg_or(args, kw, 1, "hide", Value::Bool(false)))?;
    let store = arg_or(args, kw, 2, "store", Value::None);
    let g = if py::is_none(&store) {
        store_dict(vm, "store")?
    } else {
        store
    };
    let l = if hide { vm.types.new_dict() } else { g.clone() };
    let go = g.obj().cloned().ok_or_else(|| vm.type_error("store"))?;
    vm.run_code(code, go, Some(l))?;
    Ok(Value::None)
}

/// `py_eval_bytecode(bytecode, globals, locals)`.
pub fn eval_bytecode(
    vm: &mut Vm,
    code: Rc<crysirror_python::code::Code>,
    globals: &Value,
    locals: &Value,
) -> PyResult<Value> {
    let g = globals_of(vm, globals, "store")?;
    let l = if py::is_none(locals) {
        None
    } else {
        Some(locals.clone())
    };
    vm.run_code(code, g, l)
}

fn f_py_eval_bytecode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let code = code_of(vm, &args[0], "eval")?;
    let g = arg_or(args, kw, 1, "globals", Value::None);
    let l = arg_or(args, kw, 2, "locals", Value::None);
    eval_bytecode(vm, code, &g, &l)
}

/// `py_eval(code, globals, locals)`: strings are compiled in eval mode.
pub fn eval(vm: &mut Vm, code: &Value, globals: &Value, locals: &Value) -> PyResult<Value> {
    let c = code_of(vm, code, "eval")?;
    eval_bytecode(vm, c, globals, locals)
}

fn f_py_eval(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let g = arg_or(args, kw, 1, "globals", Value::None);
    let l = arg_or(args, kw, 2, "locals", Value::None);
    eval(vm, &args[0], &g, &l)
}

fn f_store_eval(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    f_py_eval(vm, args, kw)
}

fn f_raise_at_location(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    match args[0].obj() {
        Some(o) => Err(crysirror_python::PyErr::Exc(o.clone())),
        None => Err(vm.type_error("exception expected")),
    }
}

fn f_quote_eval(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    // Newlines outside strings become line continuations.
    let s = py::string(vm, &args[0])?;
    if !s.contains('\n') {
        return Ok(args[0].clone());
    }
    let mut out = String::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut string: Option<String> = None;
    while i < cs.len() {
        let c = cs[i];
        match c {
            '\\' => {
                out.push(c);
                if let Some(n) = cs.get(i + 1) {
                    out.push(*n);
                }
                i += 2;
            }
            '\'' | '"' => {
                let triple = cs.get(i + 1) == Some(&c) && cs.get(i + 2) == Some(&c);
                let delim: String = if (string.is_none()
                    || string.as_ref().is_some_and(|s| s.len() == 3))
                    && triple
                {
                    std::iter::repeat_n(c, 3).collect()
                } else {
                    c.to_string()
                };
                if string.as_deref() == Some(delim.as_str()) {
                    string = None;
                } else if string.is_none() {
                    string = Some(delim.clone());
                }
                i += delim.chars().count();
                out.push_str(&delim);
            }
            '\n' => {
                if string.is_none() {
                    out.push('\\');
                }
                out.push('\n');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(py::s(vm, &out))
}

// StoreProxy and pickle helpers ------------------------------------------------------------------------

fn spx_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = py::string(vm, &args[1])?;
    py::import_attr(vm, "store", &k)
}

fn spx_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::import(vm, "store")?;
    let k = py::string(vm, &args[1])?;
    py::set(vm, &s, &k, args[2].clone())?;
    Ok(Value::None)
}

fn spx_delattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::import(vm, "store")?;
    ops::delattr(vm, &s, &args[1])?;
    Ok(Value::None)
}

fn f_method_unpickle(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    py::get(vm, &args[0], &n)
}

fn f_module_unpickle(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    py::import(vm, &n)
}

fn f_module_pickle(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::config_bool(vm, "developer")? {
        let r = ops::repr(vm, &args[0])?;
        return Err(py::error(vm, format!("Could not pickle {r}.")));
    }
    let f = mget(vm, "module_unpickle")?;
    let n = py::get(vm, &args[0], "__name__")?;
    let t = vm.types.new_tuple(vec![n]);
    Ok(vm.types.new_tuple(vec![f, t]))
}

fn f_construct_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("get_store_module", f_get_store_module),
            ("begin_stores", f_begin_stores),
            ("create_store", f_create_store),
            ("make_clean_stores", f_make_clean_stores),
            ("clean_stores", f_clean_stores),
            ("clean_store", f_clean_store),
            ("reset_store_changes", f_reset_store_changes),
            ("quote_eval", f_quote_eval),
            ("py_compile", f_py_compile),
            ("py_compile_exec_bytecode", f_py_compile_exec_bytecode),
            ("py_compile_hide_bytecode", f_py_compile_hide_bytecode),
            ("py_compile_eval_bytecode", f_py_compile_eval_bytecode),
            ("py_exec_bytecode", f_py_exec_bytecode),
            ("py_exec", f_py_exec),
            ("py_eval_bytecode", f_py_eval_bytecode),
            ("py_eval", f_py_eval),
            ("store_eval", f_store_eval),
            ("raise_at_location", f_raise_at_location),
            ("method_unpickle", f_method_unpickle),
            ("module_pickle", f_module_pickle),
            ("module_unpickle", f_module_unpickle),
            ("construct_None", f_construct_none),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in [
        "CompressedList",
        "DetRandom",
        "RevertableDict",
        "RevertableList",
        "RevertableObject",
        "RevertableSet",
        "RollbackRandom",
        "revertable_range",
        "revertable_sorted",
        "MultiRevertable",
        "RevertableDefaultDict",
        "mutator",
    ] {
        if let Ok(v) = py::import_attr(vm, "renpy.revertable", n) {
            set(vm, n, v);
        }
    }
    for n in [
        "deleted",
        "StoreDeleted",
        "AlwaysRollback",
        "NoRollback",
        "SlottedNoRollback",
        "rng",
        "reached",
        "reached_vars",
        "Rollback",
        "RollbackLog",
    ] {
        let v = py::import_attr(vm, "renpy.rollback", n)?;
        set(vm, n, v);
    }
    let obj = Value::Obj(vm.types.object.clone());
    let sm = py::class(
        vm,
        MODULE,
        "StoreModule",
        &[obj.clone()],
        &[("__reduce__", sm_reduce)],
        &[],
    )?;
    set(vm, "StoreModule", Value::Obj(sm));
    let dict_t = Value::Obj(vm.types.dict.clone());
    let sd = py::class(
        vm,
        MODULE,
        "StoreDict",
        &[dict_t],
        &[
            ("__init__", sd_init),
            ("__reduce__", sd_reduce),
            ("reset", sd_reset),
            ("begin", sd_begin),
            ("get_changes", sd_get_changes),
        ],
        &[],
    )?;
    set(vm, "StoreDict", Value::Obj(sd));
    let sb = py::class(
        vm,
        MODULE,
        "StoreBackup",
        &[obj.clone()],
        &[
            ("__init__", sb_init),
            ("backup_one", sb_backup_one),
            ("restore_one", sb_restore_one),
            ("restore", sb_restore),
        ],
        &[],
    )?;
    set(vm, "StoreBackup", Value::Obj(sb));
    let spx = py::class(
        vm,
        MODULE,
        "StoreProxy",
        &[obj],
        &[
            ("__getattr__", spx_getattr),
            ("__setattr__", spx_setattr),
            ("__delattr__", spx_delattr),
        ],
        &[],
    )?;
    set(vm, "StoreProxy", Value::Obj(spx));
    set(vm, "store_dicts", vm.types.new_dict());
    set(vm, "store_modules", vm.types.new_dict());
    set(
        vm,
        "initialized_store_dicts",
        vm.types.new_set(Default::default()),
    );
    set(vm, "clean_store_backup", Value::None);
    set(vm, "file_compiler_flags", vm.types.new_dict());
    set(vm, "py_compile_cache", vm.types.new_dict());
    set(vm, "old_py_compile_cache", vm.types.new_dict());
    set(vm, "compile_warnings", vm.types.new_list(Vec::new()));
    set(vm, "new_compile_flags", Value::Int(0));
    set(vm, "old_compile_flags", Value::Int(0));
    set(vm, "compile_filename", py::s(vm, ""));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
