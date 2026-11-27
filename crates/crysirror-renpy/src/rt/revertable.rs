// SPDX-License-Identifier: MPL-2.0
//! `renpy.revertable`, after Ren'Py 8.3 `revertable.py` (MIT): the
//! containers and object base whose mutations are journalled in
//! `renpy.game.log.mutated` (object id -> (weakref, clean state)), so the
//! rollback log can restore them and saves can carry their past states.
//!
//! Mutating methods of the builtin base type are wrapped with the
//! journal step (`mutator`); copying methods return revertable results.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.revertable";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn log(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "log")
}

/// Records the clean state of `this` before its first mutation since the
/// last rollback checkpoint.
pub fn journal(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let l = log(vm)?;
    if py::is_none(&l) {
        return Ok(());
    }
    let mutated = py::get_or_none(vm, &l, "mutated")?;
    if py::is_none(&mutated) {
        return Ok(());
    }
    let key = Value::Int(this.id());
    if ops::contains(vm, &mutated, &key)? {
        return Ok(());
    }
    let clean = vm.call_method(this, "_clean", Vec::new())?;
    let wr = Value::Obj(vm.types.weakref.clone());
    let r = vm.call(&wr, vec![this.clone()], Vec::new())?;
    let t = vm.types.new_tuple(vec![r, clean]);
    ops::setitem(vm, &mutated, key, t)?;
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, "mutate_flag", Value::Bool(true))
}

/// Marks a new object as created after the checkpoint (no prior state).
fn born(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let l = log(vm)?;
    if py::is_none(&l) {
        return Ok(());
    }
    let mutated = py::get_or_none(vm, &l, "mutated")?;
    if !py::is_none(&mutated) {
        ops::setitem(vm, &mutated, Value::Int(this.id()), Value::None)?;
    }
    Ok(())
}

/// Data of a wrapped method: (base type, method name).
fn base_method(vm: &mut Vm, data: &Value) -> PyResult<Value> {
    let p = py::items(vm, data)?;
    let n = py::string(vm, &p[1])?;
    py::get(vm, &p[0], &n)
}

/// `mutator(method)`: journal, then the base type's method.
fn mutator_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = args[1].clone();
    journal(vm, &this)?;
    let f = base_method(vm, &args[0])?;
    vm.call(&f, args[1..].to_vec(), kw.to_vec())
}

/// Wraps a copying method so list results are RevertableList.
fn list_result_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = base_method(vm, &args[0])?;
    let rv = vm.call(&f, args[1..].to_vec(), kw.to_vec())?;
    if crysirror_python::ops::is_not_implemented(&rv) {
        return Ok(rv);
    }
    let c = mget(vm, "RevertableList")?;
    vm.call(&c, vec![rv], Vec::new())
}

/// Wraps a set method so set results are RevertableSet.
fn set_result_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = base_method(vm, &args[0])?;
    let rv = vm.call(&f, args[1..].to_vec(), kw.to_vec())?;
    let set = vm.types.set.clone();
    if ops::isinstance(vm, &rv, &set) {
        let c = mget(vm, "RevertableSet")?;
        return vm.call(&c, vec![rv], Vec::new());
    }
    Ok(rv)
}

fn wrap(vm: &mut Vm, base: &Value, name: &str, f: crysirror_python::object::NativeFn) -> Value {
    let n = vm.types.new_str(name);
    let data = vm.types.new_tuple(vec![base.clone(), n]);
    py::closure(vm, "revertable_wrapper", f, data)
}

/// `mutator(f)` for script use: a decorator journalling `self`.
fn f_mutator(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::closure(vm, "mutator", decorated_mutator, args[0].clone()))
}

fn decorated_mutator(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    journal(vm, &args[1])?;
    vm.call(&args[0], args[1..].to_vec(), kw.to_vec())
}

/// `checkpointing(f)`: forces a checkpoint before calling `f`.
fn f_checkpointing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::closure(vm, "checkpointing", decorated_checkpointing, args[0].clone()))
}

fn decorated_checkpointing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    force_checkpoint(vm)?;
    vm.call(&args[0], args[1..].to_vec(), kw.to_vec())
}

fn force_checkpoint(vm: &mut Vm) -> PyResult<()> {
    let cf = py::import_attr(vm, "renpy.game", "context")?;
    let c = vm.call(&cf, Vec::new(), Vec::new())?;
    py::set(vm, &c, "force_checkpoint", Value::Bool(true))
}

fn f_list_wrapper(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::closure(vm, "list_wrapper", decorated_list, args[0].clone()))
}

fn decorated_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rv = vm.call(&args[0], args[1..].to_vec(), kw.to_vec())?;
    let c = mget(vm, "RevertableList")?;
    vm.call(&c, vec![rv], Vec::new())
}

// CompressedList ---------------------------------------------------------------------------------------

fn cl_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let old = py::items(vm, &args[1])?;
    let new = py::items(vm, &args[2])?;
    let empty = |vm: &mut Vm, old: Vec<Value>| -> PyResult<Value> {
        py::set(vm, this, "pre", vm.types.new_list(old))?;
        py::set(vm, this, "start", Value::Int(0))?;
        py::set(vm, this, "end", Value::Int(0))?;
        py::set(vm, this, "post", vm.types.new_list(Vec::new()))?;
        Ok(Value::None)
    };
    if new.is_empty() || old.is_empty() {
        return empty(vm, old);
    }
    let new_center = (new.len() - 1) / 2;
    let pivot = &new[new_center];
    let old_half = (old.len() - 1) / 2;
    let mut old_center = None;
    for i in 0..=old_half {
        if old[old_half - i].is(pivot) {
            old_center = Some(old_half - i);
            break;
        }
        if old_half + i < old.len() && old[old_half + i].is(pivot) {
            old_center = Some(old_half + i);
            break;
        }
    }
    let Some(oc) = old_center else { return empty(vm, old) };
    let (mut ns, mut ne, mut os, mut oe) = (new_center, new_center + 1, oc, oc + 1);
    while ns > 0 && os > 0 && new[ns - 1].is(&old[os - 1]) {
        ns -= 1;
        os -= 1;
    }
    while ne < new.len() && oe < old.len() && new[ne].is(&old[oe]) {
        ne += 1;
        oe += 1;
    }
    py::set(vm, this, "pre", vm.types.new_list(old[..os].to_vec()))?;
    py::set(vm, this, "start", Value::Int(ns as i64))?;
    py::set(vm, this, "end", Value::Int(ne as i64))?;
    py::set(vm, this, "post", vm.types.new_list(old[oe..].to_vec()))?;
    Ok(Value::None)
}

fn cl_decompress(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let new = py::items(vm, &args[1])?;
    let mut rv = py::get_items(vm, this, "pre")?;
    let s = py::get(vm, this, "start")?;
    let e = py::get(vm, this, "end")?;
    let (s, e) = (py::int(vm, &s)? as usize, py::int(vm, &e)? as usize);
    rv.extend(new[s.min(new.len())..e.min(new.len())].iter().cloned());
    rv.extend(py::get_items(vm, this, "post")?);
    Ok(vm.types.new_list(rv))
}

fn cl_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut parts = Vec::new();
    for n in ["pre", "start", "end", "post"] {
        let v = py::get(vm, this, n)?;
        parts.push(ops::str(vm, &v)?);
    }
    Ok(py::s(vm, &format!("<CompressedList {} [{}:{}] {}>", parts[0], parts[1], parts[2], parts[3])))
}

// RevertableList ---------------------------------------------------------------------------------------

fn list_t(vm: &Vm) -> Value {
    Value::Obj(vm.types.list.clone())
}

fn rl_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    born(vm, &args[0])?;
    let lt = list_t(vm);
    let f = py::get(vm, &lt, "__init__")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

fn rl_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let lt = list_t(vm);
    let f = py::get(vm, &lt, "__getitem__")?;
    let rv = vm.call(&f, args.to_vec(), Vec::new())?;
    if matches!(args[1].payload(), Some(crysirror_python::Payload::Slice { .. })) {
        let c = mget(vm, "RevertableList")?;
        return vm.call(&c, vec![rv], Vec::new());
    }
    Ok(rv)
}

fn rl_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let items = py::items(vm, &args[0])?;
    let c = mget(vm, "RevertableList")?;
    let l = vm.types.new_list(items);
    vm.call(&c, vec![l], Vec::new())
}

fn rl_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    journal(vm, &args[0])?;
    let lt = list_t(vm);
    let f = py::get(vm, &lt, "clear")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())
}

fn rl_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let items = py::items(vm, &args[0])?;
    Ok(vm.types.new_list(items))
}

fn rl_compress(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, clean) = (&args[0], &args[1]);
    if !py::truthy(vm, this)? || !py::truthy(vm, clean)? {
        return Ok(clean.clone());
    }
    let lcl = py::config(vm, "list_compression_length")?;
    if py::is_none(&lcl) {
        return Ok(clean.clone());
    }
    let lcl = py::int(vm, &lcl)? as usize;
    if ops::len(vm, this)? < lcl || ops::len(vm, clean)? < lcl {
        return Ok(clean.clone());
    }
    let c = mget(vm, "CompressedList")?;
    vm.call(&c, vec![clean.clone(), this.clone()], Vec::new())
}

fn rl_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, compressed) = (&args[0], &args[1]);
    let cl = mget(vm, "CompressedList")?;
    let items = if cl.obj().is_some_and(|c| ops::isinstance(vm, compressed, c)) {
        let d = vm.call_method(compressed, "decompress", vec![this.clone()])?;
        py::items(vm, &d)?
    } else {
        py::items(vm, compressed)?
    };
    let lt = list_t(vm);
    let clear = py::get(vm, &lt, "clear")?;
    vm.call(&clear, vec![this.clone()], Vec::new())?;
    let extend = py::get(vm, &lt, "extend")?;
    vm.call(&extend, vec![this.clone(), vm.types.new_list(items)], Vec::new())?;
    Ok(Value::None)
}

fn f_revertable_range(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = Value::Obj(vm.types.range.clone());
    let v = vm.call(&r, args.to_vec(), Vec::new())?;
    let c = mget(vm, "RevertableList")?;
    vm.call(&c, vec![v], Vec::new())
}

fn f_revertable_sorted(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::import_attr(vm, "builtins", "sorted")?;
    let v = vm.call(&s, args.to_vec(), kw.to_vec())?;
    let c = mget(vm, "RevertableList")?;
    vm.call(&c, vec![v], Vec::new())
}

// RevertableDict ---------------------------------------------------------------------------------------

fn dict_t(vm: &Vm) -> Value {
    Value::Obj(vm.types.dict.clone())
}

fn rd_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    born(vm, &args[0])?;
    let dt = dict_t(vm);
    let f = py::get(vm, &dt, "__init__")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

fn rd_has_key(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(ops::contains(vm, &args[0], &args[1])?))
}

fn rd_or(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let dict = vm.types.dict.clone();
    if !ops::isinstance(vm, &args[1], &dict) {
        return Ok(vm.types.not_implemented.clone());
    }
    let c = mget(vm, "RevertableDict")?;
    let rv = vm.call(&c, vec![args[0].clone()], Vec::new())?;
    vm.call_method(&rv, "update", vec![args[1].clone()])?;
    Ok(rv)
}

fn rd_ror(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let dict = vm.types.dict.clone();
    if !ops::isinstance(vm, &args[1], &dict) {
        return Ok(vm.types.not_implemented.clone());
    }
    let c = mget(vm, "RevertableDict")?;
    let rv = vm.call(&c, vec![args[1].clone()], Vec::new())?;
    vm.call_method(&rv, "update", vec![args[0].clone()])?;
    Ok(rv)
}

fn rd_ior(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "update", vec![args[1].clone()])?;
    Ok(args[0].clone())
}

fn rd_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = mget(vm, "RevertableDict")?;
    let rv = vm.call(&c, Vec::new(), Vec::new())?;
    vm.call_method(&rv, "update", vec![args[0].clone()])?;
    Ok(rv)
}

fn rd_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut v = Vec::new();
    for (k, x) in ops::mapping_items(vm, &args[0])? {
        v.push(vm.types.new_tuple(vec![k, x]));
    }
    Ok(vm.types.new_list(v))
}

fn rd_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let dt = dict_t(vm);
    let clear = py::get(vm, &dt, "clear")?;
    vm.call(&clear, vec![this.clone()], Vec::new())?;
    let set = py::get(vm, &dt, "__setitem__")?;
    for kv in py::items(vm, &args[1])? {
        let p = py::items(vm, &kv)?;
        vm.call(&set, vec![this.clone(), p[0].clone(), p[1].clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn identity(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[1].clone())
}

fn rdd_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let df = arg_or(args, kw, 1, "default_factory", Value::None);
    py::set(vm, this, "default_factory", df)?;
    let rest: Vec<Value> = std::iter::once(this.clone()).chain(args.iter().skip(2).cloned()).collect();
    let kw: Vec<(Rc<str>, Value)> = kw.iter().filter(|(k, _)| &**k != "default_factory").cloned().collect();
    rd_init(vm, &rest, &kw)
}

fn rdd_missing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let df = py::get_or_none(vm, this, "default_factory")?;
    if py::is_none(&df) {
        let r = ops::repr(vm, &args[1])?;
        return Err(vm.exc_msg("KeyError", &r));
    }
    let rv = vm.call(&df, Vec::new(), Vec::new())?;
    ops::setitem(vm, this, args[1].clone(), rv.clone())?;
    Ok(rv)
}

// RevertableSet ----------------------------------------------------------------------------------------

fn set_t(vm: &Vm) -> Value {
    Value::Obj(vm.types.set.clone())
}

fn rs_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    born(vm, &args[0])?;
    let st = set_t(vm);
    let f = py::get(vm, &st, "__init__")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

fn rs_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, state) = (&args[0], &args[1]);
    let st = set_t(vm);
    let update = py::get(vm, &st, "update")?;
    let items = if matches!(state.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        let d = ops::getitem(vm, state, Value::Int(0))?;
        vm.call_method(&d, "keys", Vec::new())?
    } else {
        state.clone()
    };
    vm.call(&update, vec![this.clone(), items], Vec::new())?;
    Ok(Value::None)
}

fn rs_getstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = vm.types.new_dict();
    for i in ops::collect(vm, &args[0])? {
        ops::setitem(vm, &d, i, Value::Bool(true))?;
    }
    Ok(vm.types.new_tuple(vec![d]))
}

fn rs_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let items = ops::collect(vm, &args[0])?;
    Ok(vm.types.new_list(items))
}

fn rs_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = set_t(vm);
    let clear = py::get(vm, &st, "clear")?;
    vm.call(&clear, vec![args[0].clone()], Vec::new())?;
    let update = py::get(vm, &st, "update")?;
    vm.call(&update, vec![args[0].clone(), args[1].clone()], Vec::new())?;
    Ok(Value::None)
}

// RevertableObject -------------------------------------------------------------------------------------

fn ro_new(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let obj = Value::Obj(vm.types.object.clone());
    let new = py::get(vm, &obj, "__new__")?;
    let this = vm.call(&new, vec![args[0].clone()], Vec::new())?;
    born(vm, &this)?;
    Ok(this)
}

fn ro_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if (args.len() > 1 || !kw.is_empty()) && py::config_bool(vm, "developer").unwrap_or(false) {
        return Err(vm.type_error("object() takes no parameters."));
    }
    Ok(Value::None)
}

fn ro_init_subclass(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = &args[0];
    if py::config_bool(vm, "developer").unwrap_or(false) {
        if let Some(c) = cls.obj() {
            if ops::get_raw_attr(c, "__slots__").is_some() {
                return Err(vm.type_error("Classes with __slots__ do not support rollback. To create a class with slots, inherit from python_object instead."));
            }
        }
    }
    Ok(Value::None)
}

fn object_method(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let obj = Value::Obj(vm.types.object.clone());
    let f = py::get(vm, &obj, name)?;
    vm.call(&f, args, Vec::new())
}

fn ro_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    journal(vm, &args[0])?;
    object_method(vm, "__setattr__", args.to_vec())
}

fn ro_delattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    journal(vm, &args[0])?;
    object_method(vm, "__delattr__", args.to_vec())
}

fn instance_dict(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    py::namespace_of(vm, this)
}

fn ro_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = instance_dict(vm, &args[0])?;
    let rv = vm.types.new_dict();
    for (k, v) in ops::mapping_items(vm, &d)? {
        ops::setitem(vm, &rv, k, v)?;
    }
    Ok(rv)
}

fn ro_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = instance_dict(vm, &args[0])?;
    vm.call_method(&d, "clear", Vec::new())?;
    vm.call_method(&d, "update", vec![args[1].clone()])?;
    Ok(Value::None)
}

// MultiRevertable --------------------------------------------------------------------------------------

fn mr_types(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let cls = ops::type_of(vm, this);
    let mr = mget(vm, "MultiRevertable")?;
    let mut rv = Vec::new();
    for c in ops::mro(&cls) {
        let cv = Value::Obj(c.clone());
        if py::same(&cv, &mr) {
            continue;
        }
        if ops::get_raw_attr(&c, "_rollback").is_some() {
            rv.push(cv);
        }
    }
    Ok(rv)
}

fn mr_rollback_types(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = mr_types(vm, &args[0])?;
    Ok(vm.types.new_list(t))
}

fn mr_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut rv = Vec::new();
    for t in mr_types(vm, &args[0])? {
        let f = py::get(vm, &t, "_clean")?;
        rv.push(vm.call(&f, vec![args[0].clone()], Vec::new())?);
    }
    Ok(vm.types.new_tuple(rv))
}

fn mr_compress(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let clean = py::items(vm, &args[1])?;
    let mut rv = Vec::new();
    for (t, c) in mr_types(vm, &args[0])?.into_iter().zip(clean) {
        let f = py::get(vm, &t, "_compress")?;
        rv.push(vm.call(&f, vec![args[0].clone(), c], Vec::new())?);
    }
    Ok(vm.types.new_tuple(rv))
}

fn mr_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let comp = py::items(vm, &args[1])?;
    for (t, c) in mr_types(vm, &args[0])?.into_iter().zip(comp) {
        let f = py::get(vm, &t, "_rollback")?;
        vm.call(&f, vec![args[0].clone(), c], Vec::new())?;
    }
    Ok(Value::None)
}

// Random -----------------------------------------------------------------------------------------------

fn random_t(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "random", "Random")
}

fn rr_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    born(vm, &args[0])?;
    let r = random_t(vm)?;
    let f = py::get(vm, &r, "__init__")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())
}

fn rr_clean(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "getstate", Vec::new())
}

fn rr_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = random_t(vm)?;
    let f = py::get(vm, &r, "setstate")?;
    vm.call(&f, vec![args[0].clone(), args[1].clone()], Vec::new())
}

/// `checkpointing(mutator(random.Random.<name>))`.
fn rr_checked(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    force_checkpoint(vm)?;
    mutator_call(vm, args, kw)
}

fn rr_random_factory(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut seed = arg_or(args, kw, 1, "seed", Value::None);
    if py::is_none(&seed) {
        seed = vm.call_method(this, "random", Vec::new())?;
    }
    let c = mget(vm, "RollbackRandom")?;
    let new = vm.call(&c, Vec::new(), Vec::new())?;
    vm.call_method(&new, "seed", vec![seed])?;
    Ok(new)
}

fn dr_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = random_t(vm)?;
    let f = py::get(vm, &r, "__init__")?;
    vm.call(&f, vec![args[0].clone()], Vec::new())?;
    py::set(vm, &args[0], "stack", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn dr_random(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let stack = py::get(vm, this, "stack")?;
    let rv = if ops::len(vm, &stack)? > 0 {
        vm.call_method(&stack, "pop", Vec::new())?
    } else {
        let r = random_t(vm)?;
        let f = py::get(vm, &r, "random")?;
        vm.call(&f, vec![this.clone()], Vec::new())?
    };
    let l = log(vm)?;
    if !py::is_none(&l) {
        let cur = py::get_or_none(vm, &l, "current")?;
        if !py::is_none(&cur) {
            let rl = py::get(vm, &cur, "random")?;
            ops::list_append(vm, &rl, rv.clone())?;
        }
    }
    force_checkpoint(vm)?;
    Ok(rv)
}

fn dr_pushback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut l = py::items(vm, &args[1])?;
    l.reverse();
    let stack = py::get(vm, &args[0], "stack")?;
    vm.call_method(&stack, "extend", vec![vm.types.new_list(l)])?;
    Ok(Value::None)
}

fn dr_reset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let stack = py::get(vm, &args[0], "stack")?;
    vm.call_method(&stack, "clear", Vec::new())?;
    Ok(Value::None)
}

// Pickle reconstruction --------------------------------------------------------------------------------

/// `copyreg._reconstructor` with upstream's RevertableSet repair.
fn reconstructor(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (cls, mut base, mut state) = (args[0].clone(), args[1].clone(), args[2].clone());
    let rs = mget(vm, "RevertableSet")?;
    let object = Value::Obj(vm.types.object.clone());
    if py::same(&cls, &rs) && py::same(&base, &object) {
        base = set_t(vm);
        state = vm.types.new_list(Vec::new());
    }
    if py::same(&base, &object) {
        let new = py::get(vm, &object, "__new__")?;
        return vm.call(&new, vec![cls], Vec::new());
    }
    let new = py::get(vm, &base, "__new__")?;
    let obj = vm.call(&new, vec![cls, state.clone()], Vec::new())?;
    let init = py::get(vm, &base, "__init__")?;
    let oinit = py::get(vm, &object, "__init__")?;
    if !py::same(&init, &oinit) {
        vm.call(&init, vec![obj.clone(), state], Vec::new())?;
    }
    Ok(obj)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("mutator", f_mutator),
        ("checkpointing", f_checkpointing),
        ("list_wrapper", f_list_wrapper),
        ("revertable_range", f_revertable_range),
        ("revertable_sorted", f_revertable_sorted),
        ("_reconstructor", reconstructor),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "mutate_flag", Value::Bool(true));
    set(vm, "FUTURE_FLAGS", Value::Int(0x2000 | 0x8000));
    let obj = Value::Obj(vm.types.object.clone());
    let cl = py::class(vm, MODULE, "CompressedList", &[obj.clone()], &[("__init__", cl_init), ("decompress", cl_decompress), ("__repr__", cl_repr)], &[])?;
    set(vm, "CompressedList", Value::Obj(cl));

    let lt = list_t(vm);
    let mut attrs: Vec<(String, Value)> = Vec::new();
    for n in ["__delitem__", "__setitem__", "__iadd__", "__imul__", "append", "extend", "insert", "pop", "remove", "reverse", "sort"] {
        attrs.push((n.into(), wrap(vm, &lt, n, mutator_call)));
    }
    for n in ["__add__", "__mul__", "__rmul__"] {
        attrs.push((n.into(), wrap(vm, &lt, n, list_result_call)));
    }
    let attrs_ref: Vec<(&str, Value)> = attrs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let rl = py::class(vm, MODULE, "RevertableList", &[lt.clone()], &[
        ("__init__", rl_init),
        ("__getitem__", rl_getitem),
        ("copy", rl_copy),
        ("clear", rl_clear),
        ("_clean", rl_clean),
        ("_compress", rl_compress),
        ("_rollback", rl_rollback),
    ], &attrs_ref)?;
    set(vm, "RevertableList", Value::Obj(rl));

    let dt = dict_t(vm);
    let mut attrs: Vec<(String, Value)> = Vec::new();
    for n in ["__delitem__", "__setitem__", "clear", "pop", "popitem", "setdefault", "update"] {
        attrs.push((n.into(), wrap(vm, &dt, n, mutator_call)));
    }
    for n in ["values", "keys", "items"] {
        let f = py::get(vm, &dt, n)?;
        attrs.push((format!("iter{n}"), f));
    }
    let attrs_ref: Vec<(&str, Value)> = attrs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let rd = py::class(vm, MODULE, "RevertableDict", &[dt], &[
        ("__init__", rd_init),
        ("has_key", rd_has_key),
        ("__or__", rd_or),
        ("__ror__", rd_ror),
        ("__ior__", rd_ior),
        ("copy", rd_copy),
        ("_clean", rd_clean),
        ("_compress", identity),
        ("_rollback", rd_rollback),
    ], &attrs_ref)?;
    let rd = Value::Obj(rd);
    set(vm, "RevertableDict", rd.clone());
    let rdd = py::class(vm, MODULE, "RevertableDefaultDict", &[rd], &[("__init__", rdd_init), ("__missing__", rdd_missing)], &[])?;
    set(vm, "RevertableDefaultDict", Value::Obj(rdd));

    let stt = set_t(vm);
    let mut attrs: Vec<(String, Value)> = Vec::new();
    for n in ["__iand__", "__ior__", "__isub__", "__ixor__", "add", "clear", "difference_update", "discard", "intersection_update", "pop", "remove", "symmetric_difference_update", "update"] {
        attrs.push((n.into(), wrap(vm, &stt, n, mutator_call)));
    }
    attrs.push(("union_update".into(), wrap(vm, &stt, "update", mutator_call)));
    for n in ["__and__", "__sub__", "__xor__", "__or__", "copy", "difference", "intersection", "symmetric_difference", "union"] {
        attrs.push((n.into(), wrap(vm, &stt, n, set_result_call)));
    }
    let object_reduce = py::get(vm, &obj, "__reduce__")?;
    let object_reduce_ex = py::get(vm, &obj, "__reduce_ex__")?;
    attrs.push(("__reduce__".into(), object_reduce));
    attrs.push(("__reduce_ex__".into(), object_reduce_ex));
    let attrs_ref: Vec<(&str, Value)> = attrs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let rs = py::class(vm, MODULE, "RevertableSet", &[stt], &[
        ("__init__", rs_init),
        ("__setstate__", rs_setstate),
        ("__getstate__", rs_getstate),
        ("_clean", rs_clean),
        ("_compress", identity),
        ("_rollback", rs_rollback),
    ], &attrs_ref)?;
    set(vm, "RevertableSet", Value::Obj(rs));

    let rn = vm.types.new_builtin("__new__", ro_new);
    let rn = py::staticmethod(vm, rn);
    let ris = vm.types.new_builtin("__init_subclass__", ro_init_subclass);
    let ris = py::classmethod(vm, ris);
    let ro = py::class(vm, MODULE, "RevertableObject", &[obj.clone()], &[
        ("__init__", ro_init),
        ("__setattr__", ro_setattr),
        ("__delattr__", ro_delattr),
        ("_clean", ro_clean),
        ("_compress", identity),
        ("_rollback", ro_rollback),
    ], &[("__new__", rn), ("__init_subclass__", ris)])?;
    set(vm, "RevertableObject", Value::Obj(ro));

    let mr = py::class(vm, MODULE, "MultiRevertable", &[obj], &[
        ("_rollback_types", mr_rollback_types),
        ("_clean", mr_clean),
        ("_compress", mr_compress),
        ("_rollback", mr_rollback),
    ], &[])?;
    set(vm, "MultiRevertable", Value::Obj(mr));

    let rt = random_t(vm)?;
    let mut attrs: Vec<(String, Value)> = Vec::new();
    for n in ["setstate", "getrandbits", "seed", "random"] {
        attrs.push((n.into(), wrap(vm, &rt, n, rr_checked)));
    }
    for n in ["choices", "sample"] {
        if py::get_opt(vm, &rt, n)?.is_some() {
            attrs.push((n.into(), wrap(vm, &rt, n, list_result_call)));
        }
    }
    let attrs_ref: Vec<(&str, Value)> = attrs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let rr = py::class(vm, MODULE, "RollbackRandom", &[rt.clone()], &[
        ("__init__", rr_init),
        ("_clean", rr_clean),
        ("_compress", identity),
        ("_rollback", rr_rollback),
        ("Random", rr_random_factory),
    ], &attrs_ref)?;
    set(vm, "RollbackRandom", Value::Obj(rr));
    let mut attrs: Vec<(String, Value)> = Vec::new();
    for n in ["choices", "sample"] {
        if py::get_opt(vm, &rt, n)?.is_some() {
            attrs.push((n.into(), wrap(vm, &rt, n, list_result_call)));
        }
    }
    let attrs_ref: Vec<(&str, Value)> = attrs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let dr = py::class(vm, MODULE, "DetRandom", &[rt], &[
        ("__init__", dr_init),
        ("random", dr_random),
        ("pushback", dr_pushback),
        ("reset", dr_reset),
        ("Random", rr_random_factory),
    ], &attrs_ref)?;
    set(vm, "DetRandom", Value::Obj(dr));
    // `copyreg._reconstructor = _reconstructor`.
    let recon = py::import_attr(vm, MODULE, "_reconstructor").or_else(|_| Ok::<Value, crysirror_python::PyErr>(vm.types.new_builtin("_reconstructor", reconstructor)))?;
    if let Ok(cr) = py::import(vm, "copyreg") {
        py::set(vm, &cr, "_reconstructor", recon)?;
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
