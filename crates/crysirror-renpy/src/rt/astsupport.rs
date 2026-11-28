// SPDX-License-Identifier: MPL-2.0
//! `renpy.astsupport` and `renpy.cslots`, after Ren'Py 8.4–8.6
//! `astsupport.pyx` / `cslots.pyx` (MIT).
//!
//! `hash32`/`hash64` are FNV-1a over the code points of `str(s)`, stable
//! across runs (tlids and `PyExpr.hashcode` are stored in `.rpyc` and
//! persistent files). `PyExpr` is the 8.4+ expression string carrying
//! `filename`, `linenumber`, `column`, `py` and `hashcode`.
//!
//! cslots stores only the non-default slot values of an object. Here a
//! `Slot` is a data descriptor whose value lives in the instance
//! namespace under the slot's own name (the descriptor always wins the
//! lookup), and a value equal to the default (same type) is removed, so
//! defaults are never stored, as upstream. Compression is a memory
//! layout detail upstream; `_compress`/`_decompress` keep the observable
//! state unchanged. Pickling (`__reduce_ex__` / `__setstate__`) follows
//! upstream's `(copyreg.__newobj__, (cls,), (None, slots))` form.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const ASTSUPPORT: &str = "renpy.astsupport";
const CSLOTS: &str = "renpy.cslots";

/// FNV-1a over code points, 32 bit.
pub fn hash32(s: &str) -> u32 {
    let mut rv: u32 = 0x811c9dc5;
    for c in s.chars() {
        rv ^= c as u32;
        rv = rv.wrapping_mul(0x01000193);
    }
    rv
}

/// FNV-1a over code points, 64 bit (each code point xored as 32 bits).
pub fn hash64(s: &str) -> u64 {
    let mut rv: u64 = 0xcbf29ce484222325;
    for c in s.chars() {
        rv ^= c as u32 as u64;
        rv = rv.wrapping_mul(0x100000001b3);
    }
    rv
}

fn str_of(vm: &mut Vm, v: &Value) -> PyResult<String> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_owned());
    }
    ops::str(vm, v)
}

fn f_hash32(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = str_of(vm, &args[0])?;
    Ok(Value::Int(hash32(&s) as i64))
}

fn f_hash64(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = str_of(vm, &args[0])?;
    let h = hash64(&s);
    Ok(match i64::try_from(h) {
        Ok(i) => Value::Int(i),
        Err(_) => vm.types.new_bigint(h.into()),
    })
}

// PyExpr (8.4+) -------------------------------------------------------------------------------

fn pe_new(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = args[0].clone();
    let a = &args[1..];
    let (s, filename, linenumber, pyv, hashcode, column) = match a.len() {
        6 => (a[0].clone(), a[1].clone(), a[2].clone(), a[3].clone(), a[4].clone(), a[5].clone()),
        5 => (a[0].clone(), a[1].clone(), a[2].clone(), a[3].clone(), a[4].clone(), Value::Int(0)),
        4 => (a[0].clone(), a[1].clone(), a[2].clone(), a[3].clone(), Value::None, Value::Int(0)),
        3 => (a[0].clone(), a[1].clone(), a[2].clone(), Value::Int(2), Value::None, Value::Int(0)),
        0 => return Err(py::error(vm, "PyExpr.__new__ called incorrectly.")),
        _ => {
            return Err(py::error(vm, "PyExpr.__new__ called with invalid arguments."));
        }
    };
    let p = py::int(vm, &pyv)?;
    if p != 2 && p != 3 {
        return Err(vm.exc_msg(
            "ValueError",
            "PyExpr was given an invalid value for its py argument. Did you put the column in the py argument?",
        ));
    }
    let str_t = Value::Obj(vm.types.str_.clone());
    let new = py::get(vm, &str_t, "__new__")?;
    let this = vm.call(&new, vec![cls, s.clone()], Vec::new())?;
    let hashcode = if py::is_none(&hashcode) {
        let text = str_of(vm, &s)?;
        Value::Int(hash32(&text) as i64)
    } else {
        hashcode
    };
    py::set(vm, &this, "filename", filename)?;
    py::set(vm, &this, "linenumber", linenumber)?;
    py::set(vm, &this, "column", column)?;
    py::set(vm, &this, "py", Value::Int(p))?;
    py::set(vm, &this, "hashcode", hashcode)?;
    Ok(this)
}

fn pe_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cls = py::import_attr(vm, ASTSUPPORT, "PyExpr")?;
    let s = str_of(vm, this)?;
    let mut fields = vec![py::s(vm, &s)];
    for n in ["filename", "linenumber", "py", "hashcode", "column"] {
        fields.push(py::get_or_none(vm, this, n)?);
    }
    let t = vm.types.new_tuple(fields);
    Ok(vm.types.new_tuple(vec![cls, t]))
}

/// `make_pyexpr(s, filename, linenumber, column, text, pos)`: the lexer's
/// constructor, advancing line and column over `text[:pos]`.
fn f_make_pyexpr(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(args, kw, 0, "s", Value::None);
    let filename = arg_or(args, kw, 1, "filename", Value::None);
    let mut line = py::int(vm, &arg_or(args, kw, 2, "linenumber", Value::Int(0)))?;
    let mut column = py::int(vm, &arg_or(args, kw, 3, "column", Value::Int(0)))?;
    let text = py::string(vm, &arg_or(args, kw, 4, "text", py::s(vm, "")))?;
    let pos = py::int(vm, &arg_or(args, kw, 5, "pos", Value::Int(0)))?.max(0) as usize;
    for c in text.chars().take(pos) {
        if c == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    let code = str_of(vm, &s)?;
    let cls = py::import_attr(vm, ASTSUPPORT, "PyExpr")?;
    let h = Value::Int(hash32(&code) as i64);
    vm.call(&cls, vec![s, filename, Value::Int(line), Value::Int(3), h, Value::Int(column)], Vec::new())
}

fn init_astsupport(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        ASTSUPPORT,
        &[("hash32", f_hash32), ("hash64", f_hash64), ("make_pyexpr", f_make_pyexpr)],
    );
    let str_t = Value::Obj(vm.types.str_.clone());
    let new = vm.types.new_builtin("__new__", pe_new);
    let new = py::staticmethod(vm, new);
    let pe = py::class(vm, ASTSUPPORT, "PyExpr", &[str_t], &[("__reduce__", pe_reduce)], &[("__new__", new)])?;
    crysirror_python::modules::set(vm, &m, "PyExpr", Value::Obj(pe));
    Ok(m)
}

// cslots ----------------------------------------------------------------------------------------

fn slot_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(&args[1..], kw, 0, "default_value", Value::None);
    py::set(vm, &args[0], "default_value", d)?;
    Ok(Value::None)
}

fn islot_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(&args[1..], kw, 0, "default_value", Value::Int(0));
    py::set(vm, &args[0], "default_value", d)?;
    Ok(Value::None)
}

fn slot_name(vm: &mut Vm, slot: &Value) -> PyResult<String> {
    let n = py::get_or_none(vm, slot, "name")?;
    if py::is_none(&n) {
        return Err(vm.exc_msg("AttributeError", "Slot is not bound to a class."));
    }
    py::string(vm, &n)
}

fn instance_ns(vm: &mut Vm, instance: &Value) -> PyResult<Value> {
    py::namespace_of(vm, instance)
}

fn slot_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (slot, instance) = (&args[0], args.get(1).cloned().unwrap_or(Value::None));
    if py::is_none(&instance) {
        return Ok(slot.clone());
    }
    let name = slot_name(vm, slot)?;
    let ns = instance_ns(vm, &instance)?;
    let key = py::s(vm, &name);
    match ops::getitem(vm, &ns, key) {
        Ok(v) => Ok(v),
        Err(_) => py::get(vm, slot, "default_value"),
    }
}

fn store(vm: &mut Vm, slot: &Value, instance: &Value, value: Value) -> PyResult<Value> {
    let name = slot_name(vm, slot)?;
    let default = py::get(vm, slot, "default_value")?;
    let same_type = std::rc::Rc::ptr_eq(&ops::type_of(vm, &value), &ops::type_of(vm, &default));
    let is_default = same_type && ops::eq(vm, &value, &default)?;
    let ns = instance_ns(vm, instance)?;
    let key = py::s(vm, &name);
    if is_default {
        let _ = ops::delitem(vm, &ns, key);
    } else {
        ops::setitem(vm, &ns, key, value)?;
    }
    Ok(Value::None)
}

fn slot_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[1]) {
        return Err(vm.exc_msg("AttributeError", "Slot is not bound to an instance."));
    }
    store(vm, &args[0], &args[1], args[2].clone())
}

fn islot_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[1]) {
        return Err(vm.exc_msg("AttributeError", "Slot is not bound to an instance."));
    }
    let v = py::int(vm, &args[2])?;
    if !(0..=u32::MAX as i64).contains(&v) {
        return Err(vm.exc_msg("OverflowError", "value too large to convert to unsigned int"));
    }
    store(vm, &args[0], &args[1], Value::Int(v))
}

fn slot_class_getitem(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn obj_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `_kill`: drops every stored slot value.
fn obj_kill(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &args[0]));
    let fields = py::get_items(vm, &cls, "_cslot_fields")?;
    let ns = instance_ns(vm, &args[0])?;
    for f in fields {
        let _ = ops::delitem(vm, &ns, f);
    }
    Ok(Value::None)
}

fn obj_reduce_ex(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cls = Value::Obj(ops::type_of(vm, this));
    let hg = py::get(vm, &cls, "_cslot_has_getstate")?;
    let state = if py::truthy(vm, &hg)? {
        py::method(vm, this, "__getstate__", Vec::new())?
    } else {
        let slots = py::dict(vm);
        let ln = py::get(vm, &cls, "_cslot_linenumbers")?;
        if py::truthy(vm, &ln)? {
            let l = py::get(vm, this, "linenumber")?;
            let c = py::get(vm, this, "col_offset")?;
            py::dict_set(vm, &slots, "linenumber", l)?;
            py::dict_set(vm, &slots, "col_offset", c)?;
        }
        let ns = instance_ns(vm, this)?;
        for f in py::get_items(vm, &cls, "_cslot_fields")? {
            if let Ok(v) = ops::getitem(vm, &ns, f.clone()) {
                ops::setitem(vm, &slots, f, v)?;
            }
        }
        vm.types.new_tuple(vec![Value::None, slots])
    };
    let newobj = py::import_attr(vm, "copyreg", "__newobj__")?;
    let a = vm.types.new_tuple(vec![cls]);
    Ok(vm.types.new_tuple(vec![newobj, a, state]))
}

fn obj_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let state = &args[1];
    let cls = Value::Obj(ops::type_of(vm, this));
    let setters = py::get(vm, &cls, "_cslot_setters")?;
    let dicts = if let Some(Payload::Tuple(t)) = state.payload() {
        t.to_vec()
    } else if matches!(state.payload(), Some(Payload::Dict(_))) {
        vec![state.clone()]
    } else {
        return Err(vm.type_error("Invalid state type."));
    };
    let mut name = Value::None;
    for d in dicts {
        if py::is_none(&d) {
            continue;
        }
        for (k, v) in py::str_items(vm, &d)? {
            let key = py::s(vm, &k);
            match ops::getitem(vm, &setters, key) {
                Ok(f) => {
                    vm.call(&f, vec![this.clone(), v], Vec::new())?;
                }
                Err(_) if &*k == "name" => name = v,
                Err(_) => {}
            }
        }
    }
    if py::truthy(vm, &name)? {
        let _ = py::set(vm, this, "name", name);
    }
    Ok(Value::None)
}

/// Upstream's `CObject.linenumber.__set__`/`col_offset.__set__`.
fn set_linenumber(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "linenumber", args[1].clone())?;
    Ok(Value::None)
}

fn set_col_offset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "col_offset", args[1].clone())?;
    Ok(Value::None)
}

fn is_classvar(vm: &mut Vm, annotation: &Value) -> bool {
    let Ok(get_origin) = py::import_attr(vm, "typing", "get_origin") else {
        return false;
    };
    let Ok(cv) = py::import_attr(vm, "typing", "ClassVar") else {
        return false;
    };
    match vm.call(&get_origin, vec![annotation.clone()], Vec::new()) {
        Ok(o) => py::same(&o, &cv),
        Err(_) => false,
    }
}

/// `Metaclass.__new__(cls, name, bases, namespace, **kwds)`.
fn meta_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (mcls, name, bases, namespace) = (&args[0], &args[1], &args[2], &args[3]);
    let base_list = py::items(vm, bases)?;
    if base_list.len() != 1 {
        return Err(vm.type_error("cslots.Object only supports single inheritance."));
    }
    py::dict_set(vm, namespace, "__slots__", vm.types.new_list(Vec::new()))?;
    let _ = ops::delitem(vm, namespace, py::s(vm, "__slots__"));
    let base = &base_list[0];
    let cobject = py::import_attr(vm, CSLOTS, "CObject")?;
    let (mut count, setters, fields, linenumbers, has_getstate) = if py::same(base, &cobject) {
        let setters = py::dict(vm);
        let l = py::builtin(vm, "linenumber", set_linenumber);
        let c = py::builtin(vm, "col_offset", set_col_offset);
        py::dict_set(vm, &setters, "linenumber", l)?;
        py::dict_set(vm, &setters, "col_offset", c)?;
        (0i64, setters, vm.types.new_list(Vec::new()), Value::Bool(false), false)
    } else {
        let count = py::get(vm, base, "_cslot_count")?;
        let count = py::int(vm, &count)?;
        let s = py::get(vm, base, "_cslot_setters")?;
        let setters = py::method(vm, &s, "copy", Vec::new())?;
        let f = py::get_items(vm, base, "_cslot_fields")?;
        let l = py::get(vm, base, "_cslot_linenumbers")?;
        let g = py::get(vm, base, "_cslot_has_getstate")?;
        let g = py::truthy(vm, &g)?;
        (count, setters, vm.types.new_list(f), l, g)
    };
    let type_new = py::get(vm, &Value::Obj(vm.types.type_.clone()), "__new__")?;
    let mut targs = vec![mcls.clone(), name.clone(), bases.clone(), namespace.clone()];
    targs.truncate(4);
    let rv = vm.call(&type_new, targs, kw.to_vec())?;
    let slot_cls = py::import_attr(vm, CSLOTS, "Slot")?;
    let islot_cls = py::import_attr(vm, CSLOTS, "IntegerSlot")?;
    let annotations = py::get_opt(vm, &rv, "__annotations__")?;
    if let Some(ann) = annotations {
        let int_t = Value::Obj(vm.types.int.clone());
        for (k, v) in py::str_items(vm, &ann)? {
            if is_classvar(vm, &v) {
                continue;
            }
            let key = py::s(vm, &k);
            let slot = if py::same(&v, &int_t) {
                let d = ops::getitem(vm, namespace, key.clone()).unwrap_or(Value::Int(0));
                vm.call(&islot_cls, vec![d], Vec::new())?
            } else {
                let d = ops::getitem(vm, namespace, key.clone()).unwrap_or(Value::None);
                vm.call(&slot_cls, vec![d], Vec::new())?
            };
            ops::setitem(vm, namespace, key, slot.clone())?;
            py::set(vm, &rv, &k, slot)?;
        }
    }
    for (k, v) in py::str_items(vm, namespace)? {
        if !ops::isinstance(vm, &v, slot_cls.obj().expect("Slot class")) {
            continue;
        }
        py::set(vm, &v, "number", Value::Int(count))?;
        py::set(vm, &v, "name", py::s(vm, &k))?;
        py::method(vm, &fields, "append", vec![py::s(vm, &k)])?;
        let setter = py::get(vm, &v, "__set__")?;
        py::dict_set(vm, &setters, &k, setter)?;
        count += 1;
    }
    py::set(vm, &rv, "_cslot_count", Value::Int(count))?;
    py::set(vm, &rv, "_cslot_setters", setters)?;
    py::set(vm, &rv, "_cslot_fields", fields)?;
    let own = ops::getitem(vm, namespace, py::s(vm, "_cslot_linenumbers")).unwrap_or(linenumbers);
    py::set(vm, &rv, "_cslot_linenumbers", own)?;
    let defines = ops::contains(vm, namespace, &py::s(vm, "__getstate__"))?;
    py::set(vm, &rv, "_cslot_has_getstate", Value::Bool(has_getstate || defines))?;
    Ok(rv)
}

fn init_cslots(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, CSLOTS, &[]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let type_t = Value::Obj(vm.types.type_.clone());
    let cmeta = py::class(
        vm,
        CSLOTS,
        "CMetaclass",
        &[type_t],
        &[],
        &[
            ("_cslot_count", Value::Int(0)),
            ("_cslot_linenumbers", Value::Bool(false)),
            ("_cslot_has_getstate", Value::Bool(false)),
        ],
    )?;
    set(vm, "CMetaclass", Value::Obj(cmeta.clone()));
    let cobject = py::class(
        vm,
        CSLOTS,
        "CObject",
        &[],
        &[
            ("_kill", obj_kill),
            ("_compress", obj_noop),
            ("_decompress", obj_noop),
            ("__reduce_ex__", obj_reduce_ex),
            ("__setstate__", obj_setstate),
        ],
        &[("linenumber", Value::Int(0)), ("col_offset", Value::Int(0))],
    )?;
    set(vm, "CObject", Value::Obj(cobject.clone()));
    let cgi = py::builtin(vm, "__class_getitem__", slot_class_getitem);
    let cgi = py::classmethod(vm, cgi);
    let slot = py::class(
        vm,
        CSLOTS,
        "Slot",
        &[],
        &[("__init__", slot_init), ("__get__", slot_get), ("__set__", slot_set)],
        &[
            ("__class_getitem__", cgi),
            ("number", Value::Int(0)),
            ("default_value", Value::None),
            ("name", Value::None),
        ],
    )?;
    set(vm, "Slot", Value::Obj(slot.clone()));
    let islot = py::class(
        vm,
        CSLOTS,
        "IntegerSlot",
        &[Value::Obj(slot)],
        &[("__init__", islot_init), ("__set__", islot_set)],
        &[],
    )?;
    set(vm, "IntegerSlot", Value::Obj(islot));
    let mnew = py::builtin(vm, "__new__", meta_new);
    let mnew = py::staticmethod(vm, mnew);
    let meta = py::class(vm, CSLOTS, "Metaclass", &[Value::Obj(cmeta)], &[], &[("__new__", mnew)])?;
    set(vm, "Metaclass", Value::Obj(meta.clone()));
    let ns = vm.types.new_dict();
    py::dict_set(vm, &ns, "__module__", py::s(vm, CSLOTS))?;
    let bases = vm.types.new_tuple(vec![Value::Obj(cobject)]);
    let object = vm.call(&Value::Obj(meta), vec![py::s(vm, "Object"), bases, ns], Vec::new())?;
    set(vm, "Object", object);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(ASTSUPPORT, init_astsupport);
    vm.register_native_module(CSLOTS, init_cslots);
}

use crysirror_python::Payload;
