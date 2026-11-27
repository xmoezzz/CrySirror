// SPDX-License-Identifier: MPL-2.0
//! Object protocols: attribute access with descriptors, calls into types,
//! operators with reflected dispatch, comparison, hashing, iteration and
//! container access. Builtin payloads take native fast paths; user classes
//! that override a special method are dispatched through Python.

use crate::ast::{BinOp, CmpOp, UnaryOp};
use crate::dict::Dict;
use crate::num::{self, Num};
use crate::object::*;
use crate::vm::{GenStep, Kwargs, PyErr, PyResult, Resume, Vm};
use crate::Dialect;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::rc::Rc;

// ---- types ----

pub fn type_of(vm: &Vm, v: &Value) -> ObjRef {
    match v {
        Value::None => vm.types.none_type.clone(),
        Value::Bool(_) => vm.types.bool_.clone(),
        Value::Int(_) => vm.types.int.clone(),
        Value::Float(_) => vm.types.float.clone(),
        Value::Obj(o) => o.class(),
    }
}

pub fn type_name(vm: &Vm, v: &Value) -> String {
    type_of(vm, v).type_data().map(|t| t.name.borrow().to_string()).unwrap_or_default()
}

pub fn is_type(v: &Value) -> bool {
    matches!(v.payload(), Some(Payload::Type(_)))
}

pub fn mro(t: &ObjRef) -> Vec<ObjRef> {
    t.type_data().map(|d| d.mro.borrow().clone()).unwrap_or_default()
}

pub fn is_subclass(a: &ObjRef, b: &ObjRef) -> bool {
    if Rc::ptr_eq(a, b) {
        return true;
    }
    match a.type_data() {
        Some(t) => t.mro.borrow().iter().any(|c| Rc::ptr_eq(c, b)),
        None => false,
    }
}

pub fn isinstance(vm: &Vm, v: &Value, cls: &ObjRef) -> bool {
    is_subclass(&type_of(vm, v), cls)
}

pub fn base_kind(vm: &Vm, v: &Value) -> BaseKind {
    type_of(vm, v).type_data().map(|t| t.base_kind).unwrap_or(BaseKind::Object)
}

/// Looks `name` up along the MRO of `t` (no descriptor binding).
pub fn lookup_type(t: &ObjRef, name: &str) -> Option<Value> {
    let td = t.type_data()?;
    for c in td.mro.borrow().iter() {
        if let Some(v) = c.dict.as_ref().and_then(|d| d.borrow().get_str(name).cloned()) {
            return Some(v);
        }
    }
    None
}

/// Like lookup_type, but returns None when the first definition belongs to an
/// interpreter-implemented type (handled by native fast paths).
pub fn lookup_user(t: &ObjRef, name: &str) -> Option<Value> {
    let td = t.type_data()?;
    if td.builtin {
        return None;
    }
    for c in td.mro.borrow().iter() {
        let builtin = c.type_data().map(|x| x.builtin).unwrap_or(true);
        if let Some(v) = c.dict.as_ref().and_then(|d| d.borrow().get_str(name).cloned()) {
            return if builtin { None } else { Some(v) };
        }
    }
    None
}

pub fn lookup_special(vm: &Vm, v: &Value, name: &str) -> Option<Value> {
    lookup_type(&type_of(vm, v), name)
}

pub fn is_user_type(t: &ObjRef) -> bool {
    t.type_data().map(|d| !d.builtin).unwrap_or(false)
}

// ---- attributes ----

pub fn set_raw_attr(vm: &Vm, o: &ObjRef, name: &str, v: Value) {
    if let Some(d) = o.attr_table() {
        d.borrow_mut().set_str(vm.types.intern(name), v);
    }
}

pub fn get_raw_attr(o: &ObjRef, name: &str) -> Option<Value> {
    o.attr_table().and_then(|d| d.borrow().get_str(name).cloned())
}

/// Descriptor `__get__` for a value found in a type dict.
pub fn descr_get(vm: &mut Vm, attr: &Value, obj: Option<&Value>, owner: &ObjRef) -> PyResult<Value> {
    let Value::Obj(a) = attr else { return Ok(attr.clone()) };
    match &a.payload {
        Payload::Function(_) | Payload::Builtin(_) => match obj {
            Some(o) => Ok(Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: attr.clone(), slf: o.clone() }))),
            None => Ok(attr.clone()),
        },
        Payload::StaticMethod(f) => Ok(f.clone()),
        Payload::ClassMethod(f) => Ok(Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: f.clone(), slf: Value::Obj(owner.clone()) }))),
        Payload::Property { fget, .. } => match obj {
            Some(o) => {
                if fget.is_none() {
                    return Err(vm.attr_error("unreadable attribute"));
                }
                let g = fget.clone();
                vm.call(&g, vec![o.clone()], Vec::new())
            }
            None => Ok(attr.clone()),
        },
        _ => {
            let at = a.class();
            if is_user_type(&at) || matches!(a.payload, Payload::Native(_)) {
                if let Some(get) = lookup_type(&at, "__get__") {
                    let o = obj.cloned().unwrap_or(Value::None);
                    return vm.call(&get, vec![attr.clone(), o, Value::Obj(owner.clone())], Vec::new());
                }
            }
            Ok(attr.clone())
        }
    }
}

fn is_data_descriptor(a: &Value) -> bool {
    match a {
        Value::Obj(o) => match &o.payload {
            Payload::Property { .. } => true,
            _ => {
                let t = o.class();
                (is_user_type(&t) || matches!(o.payload, Payload::Native(_))) && (lookup_type(&t, "__set__").is_some() || lookup_type(&t, "__delete__").is_some())
            }
        },
        _ => false,
    }
}

pub fn getattr(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Value> {
    let n = vm.types.intern(name);
    getattr_v(vm, obj, &n)
}

pub fn getattr_v(vm: &mut Vm, obj: &Value, name: &Value) -> PyResult<Value> {
    let t = type_of(vm, obj);
    if is_user_type(&t) {
        if let Some(ga) = lookup_user(&t, "__getattribute__") {
            let r = vm.call(&ga, vec![obj.clone(), name.clone()], Vec::new());
            return match r {
                Err(e) if vm.is_exc(&e, "AttributeError") => match lookup_type(&t, "__getattr__") {
                    Some(g) => vm.call(&g, vec![obj.clone(), name.clone()], Vec::new()),
                    None => Err(e),
                },
                other => other,
            };
        }
    }
    match generic_getattr(vm, obj, name)? {
        Some(v) => Ok(v),
        None => {
            if let Some(g) = lookup_type(&t, "__getattr__") {
                return vm.call(&g, vec![obj.clone(), name.clone()], Vec::new());
            }
            let n = name.as_str().unwrap_or("");
            if let Value::Obj(o) = obj {
                if let Payload::Type(td) = &o.payload {
                    return Err(vm.attr_error(format!("type object '{}' has no attribute '{}'", td.name.borrow(), n)));
                }
                if let Payload::Module(m, _) = &o.payload {
                    return Err(vm.attr_error(format!("module '{m}' has no attribute '{n}'")));
                }
            }
            Err(vm.attr_error(format!("'{}' object has no attribute '{}'", type_name(vm, obj), n)))
        }
    }
}

pub fn getattr_opt(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Option<Value>> {
    match getattr(vm, obj, name) {
        Ok(v) => Ok(Some(v)),
        Err(e) if vm.is_exc(&e, "AttributeError") => Ok(None),
        Err(e) => Err(e),
    }
}

/// `object.__getattribute__`: returns None when the attribute is missing.
pub fn generic_getattr(vm: &mut Vm, obj: &Value, name: &Value) -> PyResult<Option<Value>> {
    let key = name.as_str().unwrap_or("");
    if let Value::Obj(o) = obj {
        if let Payload::Type(_) = &o.payload {
            return type_getattr(vm, o, key);
        }
        if let Payload::Super { .. } = &o.payload {
            if key != "__class__" {
                if let Some(v) = crate::builtins::super_getattr(vm, o, key)? {
                    return Ok(Some(v));
                }
            }
        }
    }
    let t = type_of(vm, obj);
    let attr = lookup_type(&t, key);
    if let Some(a) = &attr {
        if is_data_descriptor(a) {
            return descr_get(vm, a, Some(obj), &t).map(Some);
        }
    }
    if let Value::Obj(o) = obj {
        if let Payload::Module(_, ns) = &o.payload {
            if key == "__dict__" {
                return Ok(Some(Value::Obj(ns.clone())));
            }
        }
        if let Some(d) = o.attr_table() {
            if key == "__dict__" {
                return Ok(Some(instance_dict_value(vm, o)));
            }
            let v = d.borrow().get_str(key).cloned();
            if let Some(v) = v {
                return Ok(Some(v));
            }
        }
    }
    if let Some(a) = attr {
        return descr_get(vm, &a, Some(obj), &t).map(Some);
    }
    crate::builtins::builtin_attr(vm, obj, key)
}

/// A dict object aliasing an instance namespace is not representable with
/// separate storage, so `__dict__` returns a proxy object that reads/writes
/// the instance namespace through the `mappingproxy` native type.
pub fn instance_dict_value(vm: &mut Vm, o: &ObjRef) -> Value {
    crate::builtins::namespace_proxy(vm, o)
}

fn type_getattr(vm: &mut Vm, t: &ObjRef, key: &str) -> PyResult<Option<Value>> {
    let meta = t.class();
    let meta_attr = lookup_type(&meta, key);
    if let Some(a) = &meta_attr {
        if is_data_descriptor(a) {
            return descr_get(vm, a, Some(&Value::Obj(t.clone())), &meta).map(Some);
        }
    }
    if let Some(a) = lookup_type(t, key) {
        return descr_get(vm, &a, None, t).map(Some);
    }
    if let Some(a) = meta_attr {
        return descr_get(vm, &a, Some(&Value::Obj(t.clone())), &meta).map(Some);
    }
    crate::builtins::builtin_attr(vm, &Value::Obj(t.clone()), key)
}

pub fn setattr(vm: &mut Vm, obj: &Value, name: &Value, v: Value) -> PyResult<()> {
    let t = type_of(vm, obj);
    if is_user_type(&t) {
        if let Some(sa) = lookup_user(&t, "__setattr__") {
            vm.call(&sa, vec![obj.clone(), name.clone(), v], Vec::new())?;
            return Ok(());
        }
    }
    generic_setattr(vm, obj, name, v)
}

pub fn generic_setattr(vm: &mut Vm, obj: &Value, name: &Value, v: Value) -> PyResult<()> {
    let key = name.as_str().unwrap_or("");
    let t = type_of(vm, obj);
    let meta_or_type = t.clone();
    if let Some(a) = lookup_type(&meta_or_type, key) {
        if let Value::Obj(ao) = &a {
            match &ao.payload {
                Payload::Property { fset, .. } => {
                    if fset.is_none() {
                        return Err(vm.attr_error(format!("can't set attribute '{key}'")));
                    }
                    let f = fset.clone();
                    vm.call(&f, vec![obj.clone(), v], Vec::new())?;
                    return Ok(());
                }
                _ => {
                    let at = ao.class();
                    if is_user_type(&at) || matches!(ao.payload, Payload::Native(_)) {
                        if let Some(set) = lookup_type(&at, "__set__") {
                            vm.call(&set, vec![a.clone(), obj.clone(), v], Vec::new())?;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    if let Value::Obj(o) = obj {
        if key == "__class__" {
            if let Value::Obj(c) = &v {
                if c.type_data().is_some() {
                    o.set_class(c.clone());
                    return Ok(());
                }
            }
            return Err(vm.type_error("__class__ must be set to a class"));
        }
        if crate::builtins::builtin_setattr(vm, o, key, &v)? {
            return Ok(());
        }
        if let Some(d) = o.attr_table() {
            if key == "__dict__" {
                let items = mapping_items(vm, &v)?;
                let mut nd = d.borrow_mut();
                nd.clear();
                for (k, val) in items {
                    nd.set_str(k, val);
                }
                return Ok(());
            }
            d.borrow_mut().set_str(name.clone(), v);
            return Ok(());
        }
    }
    Err(vm.attr_error(format!("'{}' object has no attribute '{}'", type_name(vm, obj), key)))
}

pub fn delattr(vm: &mut Vm, obj: &Value, name: &Value) -> PyResult<()> {
    let t = type_of(vm, obj);
    if is_user_type(&t) {
        if let Some(da) = lookup_user(&t, "__delattr__") {
            vm.call(&da, vec![obj.clone(), name.clone()], Vec::new())?;
            return Ok(());
        }
    }
    generic_delattr(vm, obj, name)
}

pub fn generic_delattr(vm: &mut Vm, obj: &Value, name: &Value) -> PyResult<()> {
    let key = name.as_str().unwrap_or("");
    let t = type_of(vm, obj);
    if let Some(Value::Obj(ao)) = lookup_type(&t, key) {
        if let Payload::Property { fdel, .. } = &ao.payload {
            if fdel.is_none() {
                return Err(vm.attr_error(format!("can't delete attribute '{key}'")));
            }
            let f = fdel.clone();
            vm.call(&f, vec![obj.clone()], Vec::new())?;
            return Ok(());
        }
        let at = ao.class();
        if is_user_type(&at) {
            if let Some(del) = lookup_type(&at, "__delete__") {
                vm.call(&del, vec![Value::Obj(ao.clone()), obj.clone()], Vec::new())?;
                return Ok(());
            }
        }
    }
    if let Value::Obj(o) = obj {
        if let Some(d) = o.attr_table() {
            if d.borrow_mut().del_str(key).is_some() {
                return Ok(());
            }
        }
    }
    Err(vm.attr_error(format!("'{}' object has no attribute '{}'", type_name(vm, obj), key)))
}

/// Binds a function found on a type to an instance.
pub fn bind(vm: &mut Vm, f: &Value, obj: &Value) -> Value {
    match f.payload() {
        Some(Payload::Function(_)) | Some(Payload::Builtin(_)) => Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: f.clone(), slf: obj.clone() })),
        _ => {
            let t = type_of(vm, obj);
            descr_get(vm, f, Some(obj), &t).unwrap_or_else(|_| f.clone())
        }
    }
}

pub fn call_special(vm: &mut Vm, obj: &Value, name: &str, args: Vec<Value>) -> PyResult<Option<Value>> {
    let Some(f) = lookup_special(vm, obj, name) else { return Ok(None) };
    let mut a = Vec::with_capacity(args.len() + 1);
    a.push(obj.clone());
    a.extend(args);
    match f.payload() {
        Some(Payload::Function(_)) | Some(Payload::Builtin(_)) => vm.call(&f, a, Vec::new()).map(Some),
        _ => {
            let t = type_of(vm, obj);
            let bound = descr_get(vm, &f, Some(obj), &t)?;
            vm.call(&bound, a[1..].to_vec(), Vec::new()).map(Some)
        }
    }
}

// ---- type calls / instance creation ----

pub fn call_type(vm: &mut Vm, cls: &ObjRef, args: Vec<Value>, kwargs: Kwargs) -> PyResult<Value> {
    // type(x)
    if Rc::ptr_eq(cls, &vm.types.type_) && args.len() == 1 && kwargs.is_empty() {
        return Ok(Value::Obj(type_of(vm, &args[0])));
    }
    let meta = cls.class();
    if is_user_type(&meta) {
        if let Some(c) = lookup_user(&meta, "__call__") {
            let mut a = vec![Value::Obj(cls.clone())];
            a.extend(args);
            return vm.call(&c, a, kwargs);
        }
    }
    type_call_default(vm, cls, args, kwargs)
}

pub fn type_call_default(vm: &mut Vm, cls: &ObjRef, args: Vec<Value>, kwargs: Kwargs) -> PyResult<Value> {
    let new = lookup_type(cls, "__new__");
    let mut a = Vec::with_capacity(args.len() + 1);
    a.push(Value::Obj(cls.clone()));
    a.extend(args.iter().cloned());
    let obj = match new {
        Some(n) => {
            let f = match n.payload() {
                Some(Payload::StaticMethod(f)) => f.clone(),
                _ => n,
            };
            vm.call(&f, a, kwargs.clone())?
        }
        None => crate::builtins::object_new(vm, &a, &kwargs)?,
    };
    if isinstance(vm, &obj, cls) {
        let t = type_of(vm, &obj);
        if let Some(init) = lookup_type(&t, "__init__") {
            let mut a = Vec::with_capacity(args.len() + 1);
            a.push(obj.clone());
            a.extend(args);
            let r = vm.call(&init, a, kwargs)?;
            if !r.is_none() {
                return Err(vm.type_error("__init__() should return None"));
            }
        }
    }
    Ok(obj)
}

/// Allocates an instance of `cls` with the payload its builtin base needs.
pub fn alloc_instance(vm: &Vm, cls: &ObjRef) -> ObjRef {
    let td = cls.type_data().unwrap();
    let payload = match td.base_kind {
        BaseKind::List => Payload::List(RefCell::new(Vec::new())),
        BaseKind::Dict => Payload::Dict(RefCell::new(Dict::new())),
        BaseKind::Set => Payload::Set(RefCell::new(Dict::new())),
        BaseKind::ByteArray => Payload::ByteArray(RefCell::new(Vec::new())),
        _ => Payload::None,
    };
    let dict = if td.no_dict && td.builtin { None } else { Some(Dict::new()) };
    let _ = vm;
    new_obj(cls.clone(), dict, payload)
}

// ---- truthiness / length ----

pub fn truthy(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    Ok(match v {
        Value::None => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Obj(o) => {
            let t = o.class();
            if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
                if let Some(r) = call_special_user(vm, v, &t, &["__bool__", "__nonzero__"])? {
                    return match r {
                        Value::Bool(b) => Ok(b),
                        Value::Int(i) => Ok(i != 0),
                        _ => Err(vm.type_error("__bool__ should return bool")),
                    };
                }
                if let Some(r) = call_special_user(vm, v, &t, &["__len__"])? {
                    return Ok(index_of(vm, &r)? != 0);
                }
            }
            match &o.payload {
                Payload::Str(s) => s.char_len() > 0,
                Payload::Bytes(b) => !b.is_empty(),
                Payload::ByteArray(b) => !b.borrow().is_empty(),
                Payload::Tuple(t) => !t.is_empty(),
                Payload::List(l) => !l.borrow().is_empty(),
                Payload::Dict(d) | Payload::Set(d) => !d.borrow().is_empty(),
                Payload::FrozenSet(d) => !d.is_empty(),
                Payload::Int(b) => !b.is_zero(),
                Payload::Float(f) => *f != 0.0,
                Payload::Complex(r, i) => *r != 0.0 || *i != 0.0,
                Payload::Range(a, b, s) => range_len(*a, *b, *s) > 0,
                Payload::DictView { dict, .. } => match &dict.payload {
                    Payload::Dict(d) => !d.borrow().is_empty(),
                    _ => true,
                },
                _ => true,
            }
        }
    })
}

/// Calls the first special method found (user or native type), or None.
fn call_special_user(vm: &mut Vm, v: &Value, t: &ObjRef, names: &[&str]) -> PyResult<Option<Value>> {
    for n in names {
        let f = if is_user_type(t) { lookup_user(t, n) } else { lookup_type(t, n) };
        if let Some(f) = f {
            return vm.call(&f, vec![v.clone()], Vec::new()).map(Some);
        }
    }
    Ok(None)
}

pub fn range_len(a: i64, b: i64, s: i64) -> i64 {
    if s > 0 && a < b {
        (b - a - 1) / s + 1
    } else if s < 0 && a > b {
        (a - b - 1) / (-s) + 1
    } else {
        0
    }
}

pub fn len(vm: &mut Vm, v: &Value) -> PyResult<usize> {
    if let Value::Obj(o) = v {
        let t = o.class();
        if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
            if let Some(r) = call_special_user(vm, v, &t, &["__len__"])? {
                let n = index_of(vm, &r)?;
                if n < 0 {
                    return Err(vm.value_error("__len__() should return >= 0"));
                }
                return Ok(n as usize);
            }
        }
        let n = match &o.payload {
            Payload::Str(s) => s.char_len(),
            Payload::Bytes(b) => b.len(),
            Payload::ByteArray(b) => b.borrow().len(),
            Payload::Tuple(t) => t.len(),
            Payload::List(l) => l.borrow().len(),
            Payload::Dict(d) | Payload::Set(d) => d.borrow().len(),
            Payload::FrozenSet(d) => d.len(),
            Payload::Range(a, b, s) => range_len(*a, *b, *s) as usize,
            Payload::DictView { dict, .. } => match &dict.payload {
                Payload::Dict(d) => d.borrow().len(),
                _ => return len(vm, &Value::Obj(dict.clone())),
            },
            _ => return Err(vm.type_error(format!("object of type '{}' has no len()", o.class_name()))),
        };
        return Ok(n);
    }
    Err(vm.type_error(format!("object of type '{}' has no len()", type_name(vm, v))))
}

pub fn index_of(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(*b as i64),
        Value::Obj(o) => match &o.payload {
            Payload::Int(b) => b.to_i64().ok_or_else(|| vm.exc_msg("OverflowError", "Python int too large to convert to C ssize_t")),
            _ => {
                if let Some(r) = call_special(vm, v, "__index__", Vec::new())? {
                    return index_of(vm, &r);
                }
                Err(vm.type_error(format!("'{}' object cannot be interpreted as an integer", o.class_name())))
            }
        },
        _ => Err(vm.type_error(format!("'{}' object cannot be interpreted as an integer", type_name(vm, v)))),
    }
}

pub fn to_f64(vm: &mut Vm, v: &Value) -> PyResult<f64> {
    match num::num_of(v) {
        Some(Num::I(i)) => Ok(i as f64),
        Some(Num::B(b)) => b.to_f64().ok_or_else(|| vm.exc_msg("OverflowError", "int too large to convert to float")),
        Some(Num::F(f)) => Ok(f),
        _ => {
            if let Some(r) = call_special(vm, v, "__float__", Vec::new())? {
                return to_f64(vm, &r);
            }
            if let Some(r) = call_special(vm, v, "__index__", Vec::new())? {
                return to_f64(vm, &r);
            }
            Err(vm.type_error(format!("must be real number, not {}", type_name(vm, v))))
        }
    }
}

// ---- hashing / equality ----

pub fn hash(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    Ok(match v {
        Value::None => 0x1000,
        Value::Bool(b) => *b as i64,
        Value::Int(i) => crate::hash::hash_int(*i),
        Value::Float(f) => crate::hash::hash_float(*f),
        Value::Obj(o) => {
            let t = o.class();
            if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
                if let Some(h) = lookup_type(&t, "__hash__") {
                    if h.is_none() {
                        return Err(vm.type_error(format!("unhashable type: '{}'", o.class_name())));
                    }
                    if !matches!(h.payload(), Some(Payload::Builtin(_))) || matches!(o.payload, Payload::Native(_)) {
                        let r = vm.call(&h, vec![v.clone()], Vec::new())?;
                        return match num::num_of(&r) {
                            Some(Num::I(i)) => Ok(i),
                            Some(Num::B(b)) => Ok(crate::hash::hash_bigint(&b)),
                            _ => Err(vm.type_error("__hash__ method should return an integer")),
                        };
                    }
                }
            }
            match &o.payload {
                Payload::Str(s) => s.hash(),
                Payload::Bytes(b) => crate::hash::hash_bytes(b),
                Payload::Int(b) => crate::hash::hash_bigint(b),
                Payload::Float(f) => crate::hash::hash_float(*f),
                Payload::Complex(r, i) => crate::hash::hash_float(*r).wrapping_add(crate::hash::hash_float(*i).wrapping_mul(1000003)),
                Payload::Tuple(items) => {
                    let mut hs = Vec::with_capacity(items.len());
                    for it in items.iter() {
                        hs.push(hash(vm, it)?);
                    }
                    crate::hash::hash_tuple(&hs)
                }
                Payload::FrozenSet(d) => {
                    let mut h: u64 = 1927868237u64.wrapping_mul(d.len() as u64 + 1);
                    for e in d.iter() {
                        let x = e.hash as u64;
                        h ^= (x ^ (x << 16) ^ 89869747).wrapping_mul(3644798167);
                    }
                    h as i64
                }
                Payload::List(_) | Payload::Dict(_) | Payload::Set(_) | Payload::ByteArray(_) => {
                    return Err(vm.type_error(format!("unhashable type: '{}'", o.class_name())));
                }
                Payload::Method { func, slf } => {
                    let a = hash(vm, func)?;
                    let b = slf.id();
                    a ^ b
                }
                Payload::Slice(..) => return Err(vm.type_error("unhashable type: 'slice'")),
                _ => {
                    // Classes defining __eq__ without __hash__ are unhashable.
                    if is_user_type(&t) && lookup_user(&t, "__eq__").is_some() && lookup_user(&t, "__hash__").is_none() && vm_dialect(vm) == Dialect::Py3 {
                        return Err(vm.type_error(format!("unhashable type: '{}'", o.class_name())));
                    }
                    (v.id() >> 4) ^ (v.id() << 60)
                }
            }
        }
    })
}

pub fn vm_dialect(vm: &Vm) -> Dialect {
    vm.frames.last().map(|f| f.code.dialect).unwrap_or(vm.dialect_default)
}

/// Native equality for builtin payloads. None means "not decided natively".
fn native_eq(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<Option<bool>> {
    if let (Some(x), Some(y)) = (num::num_of(a), num::num_of(b)) {
        return Ok(Some(num::num_eq(&x, &y)));
    }
    let (Value::Obj(oa), Value::Obj(ob)) = (a, b) else {
        return Ok(match (a, b) {
            (Value::None, Value::None) => Some(true),
            _ => None,
        });
    };
    Ok(Some(match (&oa.payload, &ob.payload) {
        (Payload::Str(x), Payload::Str(y)) => x.as_str() == y.as_str(),
        (Payload::Bytes(x), Payload::Bytes(y)) => x == y,
        (Payload::ByteArray(x), Payload::ByteArray(y)) => *x.borrow() == *y.borrow(),
        (Payload::Bytes(x), Payload::ByteArray(y)) | (Payload::ByteArray(y), Payload::Bytes(x)) => **x == *y.borrow(),
        (Payload::Tuple(x), Payload::Tuple(y)) => seq_eq(vm, x, y)?,
        (Payload::List(x), Payload::List(y)) => {
            let (x, y) = (x.borrow().clone(), y.borrow().clone());
            seq_eq(vm, &x, &y)?
        }
        (Payload::Dict(x), Payload::Dict(y)) => {
            let (ix, ly) = (x.borrow().items(), y.borrow().len());
            if ix.len() != ly {
                false
            } else {
                let mut ok = true;
                for (k, v) in ix {
                    match dict_get(vm, ob, &k)? {
                        Some(w) if eq(vm, &v, &w)? => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                ok
            }
        }
        (Payload::Set(_) | Payload::FrozenSet(_), Payload::Set(_) | Payload::FrozenSet(_)) => {
            let (ka, kb) = (set_keys(a), set_keys(b));
            if ka.len() != kb.len() {
                false
            } else {
                let mut ok = true;
                for k in ka {
                    if !set_contains(vm, b, &k)? {
                        ok = false;
                        break;
                    }
                }
                ok
            }
        }
        (Payload::Range(a1, b1, s1), Payload::Range(a2, b2, s2)) => {
            let (l1, l2) = (range_len(*a1, *b1, *s1), range_len(*a2, *b2, *s2));
            l1 == l2 && (l1 == 0 || (a1 == a2 && (l1 == 1 || s1 == s2)))
        }
        (Payload::Slice(a1, b1, c1), Payload::Slice(a2, b2, c2)) => eq(vm, a1, a2)? && eq(vm, b1, b2)? && eq(vm, c1, c2)?,
        (Payload::Method { func: f1, slf: s1 }, Payload::Method { func: f2, slf: s2 }) => f1.is(f2) && s1.is(s2),
        (Payload::DictView { dict: d1, kind: k1 }, Payload::DictView { dict: d2, kind: k2 }) if *k1 == 0 && *k2 == 0 => {
            let a = Value::Obj(d1.clone());
            let b = Value::Obj(d2.clone());
            let ka = collect(vm, &a)?;
            let kb = collect(vm, &b)?;
            let sa = make_set(vm, ka)?;
            let sb = make_set(vm, kb)?;
            return native_eq(vm, &sa, &sb);
        }
        _ => return Ok(None),
    }))
}

fn seq_eq(vm: &mut Vm, x: &[Value], y: &[Value]) -> PyResult<bool> {
    if x.len() != y.len() {
        return Ok(false);
    }
    for (a, b) in x.iter().zip(y) {
        if !a.is(b) && !eq(vm, a, b)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn eq(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    // Fast paths.
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => return Ok(x == y),
        (Value::Obj(x), Value::Obj(y)) => {
            if let (Payload::Str(s), Payload::Str(t)) = (&x.payload, &y.payload) {
                if !is_user_type(&x.class()) && !is_user_type(&y.class()) {
                    return Ok(Rc::ptr_eq(x, y) || s.as_str() == t.as_str());
                }
            }
        }
        _ => {}
    }
    let r = compare(vm, a, b, CmpOp::Eq)?;
    truthy(vm, &r)
}

pub fn ne(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    Ok(!eq(vm, a, b)?)
}

/// Equality used by containers: identity first.
pub fn same_or_eq(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    if a.is(b) {
        return Ok(true);
    }
    eq(vm, a, b)
}

// ---- comparison ----

fn cmp_names(op: CmpOp) -> (&'static str, &'static str) {
    match op {
        CmpOp::Eq => ("__eq__", "__eq__"),
        CmpOp::NotEq => ("__ne__", "__ne__"),
        CmpOp::Lt => ("__lt__", "__gt__"),
        CmpOp::LtE => ("__le__", "__ge__"),
        CmpOp::Gt => ("__gt__", "__lt__"),
        CmpOp::GtE => ("__ge__", "__le__"),
        _ => ("", ""),
    }
}

pub fn compare(vm: &mut Vm, a: &Value, b: &Value, op: CmpOp) -> PyResult<Value> {
    match op {
        CmpOp::Is => return Ok(Value::Bool(a.is(b))),
        CmpOp::IsNot => return Ok(Value::Bool(!a.is(b))),
        CmpOp::In => return Ok(Value::Bool(contains(vm, b, a)?)),
        CmpOp::NotIn => return Ok(Value::Bool(!contains(vm, b, a)?)),
        _ => {}
    }
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        return Ok(Value::Bool(match op {
            CmpOp::Eq => x == y,
            CmpOp::NotEq => x != y,
            CmpOp::Lt => x < y,
            CmpOp::LtE => x <= y,
            CmpOp::Gt => x > y,
            _ => x >= y,
        }));
    }
    let (name, rname) = cmp_names(op);
    let ta = type_of(vm, a);
    let tb = type_of(vm, b);
    let ua = is_user_type(&ta) || matches!(a.payload(), Some(Payload::Native(_)));
    let ub = is_user_type(&tb) || matches!(b.payload(), Some(Payload::Native(_)));
    if ua || ub {
        let fa = if ua { user_or_native(&ta, name, a) } else { None };
        let fb = if ub { user_or_native(&tb, rname, b) } else { None };
        // Reflected first when b's type is a proper subclass of a's.
        let b_first = fb.is_some() && !Rc::ptr_eq(&ta, &tb) && is_subclass(&tb, &ta);
        if b_first {
            let r = vm.call(fb.as_ref().unwrap(), vec![b.clone(), a.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                return Ok(r);
            }
        }
        if let Some(f) = &fa {
            let r = vm.call(f, vec![a.clone(), b.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                return Ok(r);
            }
        }
        if !b_first {
            if let Some(f) = &fb {
                let r = vm.call(f, vec![b.clone(), a.clone()], Vec::new())?;
                if !is_not_implemented(&r) {
                    return Ok(r);
                }
            }
        }
        if op == CmpOp::NotEq {
            // Default __ne__ inverts __eq__.
            if let Some(f) = if ua { user_or_native(&ta, "__eq__", a) } else { None } {
                let r = vm.call(&f, vec![a.clone(), b.clone()], Vec::new())?;
                if !is_not_implemented(&r) {
                    return Ok(Value::Bool(!truthy(vm, &r)?));
                }
            }
        }
        // Python 2 __cmp__.
        if let Some(f) = if ua { lookup_user(&ta, "__cmp__") } else { None } {
            let r = vm.call(&f, vec![a.clone(), b.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                let c = index_of(vm, &r)?;
                return Ok(Value::Bool(cmp_result(op, c.signum() as i32)));
            }
        }
    }
    match op {
        CmpOp::Eq | CmpOp::NotEq => {
            let r = match native_eq(vm, a, b)? {
                Some(r) => r,
                None => a.is(b),
            };
            Ok(Value::Bool(if op == CmpOp::Eq { r } else { !r }))
        }
        _ => match native_order(vm, a, b)? {
            Some(c) => Ok(Value::Bool(cmp_result(op, c))),
            None => {
                if vm_dialect(vm) == Dialect::Py2 {
                    let c = py2_fallback_order(vm, a, b);
                    return Ok(Value::Bool(cmp_result(op, c)));
                }
                let sym = match op {
                    CmpOp::Lt => "<",
                    CmpOp::LtE => "<=",
                    CmpOp::Gt => ">",
                    _ => ">=",
                };
                Err(vm.type_error(format!("'{}' not supported between instances of '{}' and '{}'", sym, type_name(vm, a), type_name(vm, b))))
            }
        },
    }
}

fn user_or_native(t: &ObjRef, name: &str, v: &Value) -> Option<Value> {
    if matches!(v.payload(), Some(Payload::Native(_))) {
        // Host types keep their special methods in builtin type dicts.
        return lookup_type(t, name).filter(|f| !is_object_default(t, f, name));
    }
    lookup_user(t, name)
}

fn is_object_default(_t: &ObjRef, f: &Value, _name: &str) -> bool {
    matches!(f.payload(), Some(Payload::Builtin(b)) if b.name.starts_with("object."))
}

pub fn is_not_implemented(v: &Value) -> bool {
    matches!(v.payload(), Some(Payload::NotImplemented))
}

fn cmp_result(op: CmpOp, c: i32) -> bool {
    // 2: incomparable sets, 3: NaN.
    if c == 2 || c == 3 {
        return op == CmpOp::NotEq;
    }
    match op {
        CmpOp::Lt => c < 0,
        CmpOp::LtE => c <= 0,
        CmpOp::Gt => c > 0,
        CmpOp::GtE => c >= 0,
        CmpOp::Eq => c == 0,
        _ => c != 0,
    }
}

/// Three-way native ordering; None when not orderable natively.
pub fn native_order(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<Option<i32>> {
    if let (Some(x), Some(y)) = (num::num_of(a), num::num_of(b)) {
        if matches!(x, Num::C(..)) || matches!(y, Num::C(..)) {
            return Ok(None);
        }
        // 3 marks an unordered pair (NaN): every ordering test is false.
        return Ok(Some(num::num_cmp(&x, &y).unwrap_or(3)));
    }
    let (Value::Obj(oa), Value::Obj(ob)) = (a, b) else { return Ok(None) };
    Ok(match (&oa.payload, &ob.payload) {
        (Payload::Str(x), Payload::Str(y)) => Some(ord(x.as_str().cmp(y.as_str()))),
        (Payload::Bytes(x), Payload::Bytes(y)) => Some(ord(x.cmp(y))),
        (Payload::Tuple(x), Payload::Tuple(y)) => seq_order(vm, &x.to_vec(), &y.to_vec())?,
        (Payload::List(x), Payload::List(y)) => {
            let (x, y) = (x.borrow().clone(), y.borrow().clone());
            seq_order(vm, &x, &y)?
        }
        (Payload::Set(_) | Payload::FrozenSet(_), Payload::Set(_) | Payload::FrozenSet(_)) => {
            // Subset ordering: not a total order; encode as -1/0/1 or None.
            let sub = is_subset(vm, a, b)?;
            let sup = is_subset(vm, b, a)?;
            match (sub, sup) {
                (true, true) => Some(0),
                (true, false) => Some(-1),
                (false, true) => Some(1),
                _ => Some(2),
            }
        }
        _ => None,
    })
}

fn ord(o: std::cmp::Ordering) -> i32 {
    match o {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn seq_order(vm: &mut Vm, x: &[Value], y: &[Value]) -> PyResult<Option<i32>> {
    for (a, b) in x.iter().zip(y) {
        if a.is(b) || eq(vm, a, b)? {
            continue;
        }
        let lt = compare(vm, a, b, CmpOp::Lt)?;
        return Ok(Some(if truthy(vm, &lt)? { -1 } else { 1 }));
    }
    Ok(Some(ord(x.len().cmp(&y.len()))))
}

/// Python 2 ordering of unrelated types: None first, numbers next, then by
/// type name, then by identity.
fn py2_fallback_order(vm: &Vm, a: &Value, b: &Value) -> i32 {
    let rank = |v: &Value| -> u8 {
        match v {
            Value::None => 0,
            _ if num::num_of(v).is_some() => 1,
            _ => 2,
        }
    };
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return if ra < rb { -1 } else { 1 };
    }
    let (na, nb) = (type_name(vm, a), type_name(vm, b));
    if na != nb {
        return ord(na.cmp(&nb));
    }
    ord(a.id().cmp(&b.id()))
}

// ---- containment ----

pub fn contains(vm: &mut Vm, container: &Value, item: &Value) -> PyResult<bool> {
    if let Value::Obj(o) = container {
        let t = o.class();
        if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
            let f = if is_user_type(&t) { lookup_user(&t, "__contains__") } else { lookup_type(&t, "__contains__") };
            if let Some(f) = f {
                let r = vm.call(&f, vec![container.clone(), item.clone()], Vec::new())?;
                return truthy(vm, &r);
            }
            if is_user_type(&t) && matches!(o.payload, Payload::None) {
                return iter_contains(vm, container, item);
            }
        }
        match &o.payload {
            Payload::Str(s) => {
                let Some(sub) = item.as_str() else {
                    return Err(vm.type_error(format!("'in <string>' requires string as left operand, not {}", type_name(vm, item))));
                };
                return Ok(s.as_str().contains(sub));
            }
            Payload::Bytes(_) | Payload::ByteArray(_) => {
                let hay = bytes_of(container).unwrap();
                return match item {
                    Value::Int(i) => Ok(hay.contains(&(*i as u8))),
                    _ => {
                        let n = bytes_of(item).ok_or_else(|| vm.type_error("a bytes-like object is required"))?;
                        Ok(n.is_empty() || hay.windows(n.len()).any(|w| w == &n[..]))
                    }
                };
            }
            Payload::Dict(_) => return Ok(dict_get(vm, o, item)?.is_some()),
            Payload::Set(_) | Payload::FrozenSet(_) => return set_contains(vm, container, item),
            Payload::DictView { dict, kind } => {
                return match kind {
                    0 => Ok(dict_get(vm, dict, item)?.is_some()),
                    2 => {
                        let Some(Payload::Tuple(kv)) = item.payload() else { return Ok(false) };
                        if kv.len() != 2 {
                            return Ok(false);
                        }
                        match dict_get(vm, dict, &kv[0])? {
                            Some(v) => same_or_eq(vm, &v, &kv[1]),
                            None => Ok(false),
                        }
                    }
                    _ => iter_contains(vm, container, item),
                };
            }
            Payload::Tuple(t) => {
                let t = t.to_vec();
                for x in &t {
                    if same_or_eq(vm, x, item)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Payload::List(l) => {
                let l = l.borrow().clone();
                for x in &l {
                    if same_or_eq(vm, x, item)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Payload::Range(a, b, s) => {
                if let Some(Num::I(i)) = num::num_of(item) {
                    let (a, b, s) = (*a, *b, *s);
                    let inr = if s > 0 { i >= a && i < b } else { i <= a && i > b };
                    return Ok(inr && (i - a) % s == 0);
                }
                return iter_contains(vm, container, item);
            }
            _ => {}
        }
    }
    iter_contains(vm, container, item)
}

fn iter_contains(vm: &mut Vm, container: &Value, item: &Value) -> PyResult<bool> {
    let it = iter(vm, container)?;
    while let Some(x) = next(vm, &it)? {
        if same_or_eq(vm, &x, item)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn bytes_of(v: &Value) -> Option<Vec<u8>> {
    match v.payload()? {
        Payload::Bytes(b) => Some(b.to_vec()),
        Payload::ByteArray(b) => Some(b.borrow().clone()),
        _ => None,
    }
}

// ---- dict / set primitives ----

fn dict_cell(o: &ObjRef) -> Option<&RefCell<Dict>> {
    match &o.payload {
        Payload::Dict(d) | Payload::Set(d) => Some(d),
        _ => None,
    }
}

fn native_key_eq(a: &Value, b: &Value) -> Option<bool> {
    if a.is(b) {
        return Some(true);
    }
    match (a, b) {
        (Value::Obj(x), Value::Obj(y)) => match (&x.payload, &y.payload) {
            (Payload::Str(s), Payload::Str(t)) if !is_user_type(&x.class()) && !is_user_type(&y.class()) => Some(s.as_str() == t.as_str()),
            _ => None,
        },
        (Value::Obj(x), _) | (_, Value::Obj(x)) if matches!(x.payload, Payload::Str(_)) => Some(false),
        _ => match (num::num_of(a), num::num_of(b)) {
            (Some(x), Some(y)) => Some(num::num_eq(&x, &y)),
            _ => None,
        },
    }
}

/// Index of `key` in the table of dict/set object `o`.
fn table_find(vm: &mut Vm, cell: &RefCell<Dict>, key: &Value, h: i64) -> PyResult<Option<usize>> {
    let cands = cell.borrow().candidates(h);
    for idx in cands {
        let k = match cell.borrow().entry(idx) {
            Some(e) => e.key.clone(),
            None => continue,
        };
        let same = match native_key_eq(&k, key) {
            Some(b) => b,
            None => eq(vm, &k, key)?,
        };
        if same {
            return Ok(Some(idx));
        }
    }
    Ok(None)
}

pub fn dict_get(vm: &mut Vm, d: &ObjRef, key: &Value) -> PyResult<Option<Value>> {
    let Some(cell) = dict_cell(d) else { return Ok(None) };
    if let Some(s) = key.as_pystr() {
        if !matches!(key, Value::Obj(o) if is_user_type(&o.class())) {
            return Ok(cell.borrow().get_str(s.as_str()).cloned());
        }
    }
    let h = hash(vm, key)?;
    match table_find(vm, cell, key, h)? {
        Some(i) => Ok(cell.borrow().entry(i).map(|e| e.value.clone())),
        None => Ok(None),
    }
}

pub fn dict_set(vm: &mut Vm, d: &ObjRef, key: Value, v: Value) -> PyResult<()> {
    let Some(cell) = dict_cell(d) else { return Err(vm.type_error("not a dict")) };
    let h = hash(vm, &key)?;
    match table_find(vm, cell, &key, h)? {
        Some(i) => {
            if let Some(e) = cell.borrow_mut().entry_mut(i) {
                e.value = v;
            }
        }
        None => {
            cell.borrow_mut().insert_new(h, key, v);
        }
    }
    Ok(())
}

pub fn dict_del(vm: &mut Vm, d: &ObjRef, key: Value) -> PyResult<()> {
    match dict_pop(vm, d, &key)? {
        Some(_) => Ok(()),
        None => Err(vm.key_error(key)),
    }
}

pub fn dict_pop(vm: &mut Vm, d: &ObjRef, key: &Value) -> PyResult<Option<Value>> {
    let Some(cell) = dict_cell(d) else { return Ok(None) };
    let h = hash(vm, key)?;
    match table_find(vm, cell, key, h)? {
        Some(i) => Ok(cell.borrow_mut().remove_at(i).map(|e| e.value)),
        None => Ok(None),
    }
}

pub fn set_add(vm: &mut Vm, s: &Value, item: Value) -> PyResult<()> {
    let Value::Obj(o) = s else { return Err(vm.type_error("not a set")) };
    let cell = match &o.payload {
        Payload::Set(d) => d,
        _ => return Err(vm.type_error("not a set")),
    };
    let h = hash(vm, &item)?;
    if table_find(vm, cell, &item, h)?.is_none() {
        cell.borrow_mut().insert_new(h, item, Value::None);
    }
    Ok(())
}

pub fn set_discard(vm: &mut Vm, s: &Value, item: &Value) -> PyResult<bool> {
    let Some(Payload::Set(cell)) = s.payload() else { return Ok(false) };
    let h = hash(vm, item)?;
    match table_find(vm, cell, item, h)? {
        Some(i) => {
            cell.borrow_mut().remove_at(i);
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn set_contains(vm: &mut Vm, s: &Value, item: &Value) -> PyResult<bool> {
    let h = match hash(vm, item) {
        Ok(h) => h,
        Err(e) if vm.is_exc(&e, "TypeError") && matches!(item.payload(), Some(Payload::Set(_))) => {
            // set in set: compare as frozenset
            let keys = set_keys(item);
            let fs = vm.types.new_frozenset(Dict::new());
            let _ = keys;
            hash(vm, &fs)?
        }
        Err(e) => return Err(e),
    };
    match s.payload() {
        Some(Payload::Set(cell)) => Ok(table_find(vm, cell, item, h)?.is_some()),
        Some(Payload::FrozenSet(d)) => {
            let cands = d.candidates(h);
            for idx in cands {
                let k = d.entry(idx).unwrap().key.clone();
                if same_or_eq(vm, &k, item)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn set_keys(s: &Value) -> Vec<Value> {
    match s.payload() {
        Some(Payload::Set(d)) => d.borrow().keys(),
        Some(Payload::FrozenSet(d)) => d.keys(),
        _ => Vec::new(),
    }
}

pub fn make_set(vm: &mut Vm, items: Vec<Value>) -> PyResult<Value> {
    let s = vm.types.new_set(Dict::new());
    for it in items {
        set_add(vm, &s, it)?;
    }
    Ok(s)
}

/// Builds a Dict table (for frozenset construction) from items.
pub fn make_table(vm: &mut Vm, items: Vec<Value>) -> PyResult<Dict> {
    let s = make_set(vm, items)?;
    let Some(Payload::Set(d)) = s.payload() else { unreachable!() };
    Ok(d.borrow().clone())
}

fn is_subset(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    for k in set_keys(a) {
        if !set_contains(vm, b, &k)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn list_append(vm: &mut Vm, l: &Value, v: Value) -> PyResult<()> {
    match l.payload() {
        Some(Payload::List(lv)) => {
            lv.borrow_mut().push(v);
            Ok(())
        }
        _ => Err(vm.type_error("not a list")),
    }
}

/// `mapping[key]` for namespace lookups on any mapping (dict fast path).
pub fn mapping_get_str(vm: &mut Vm, m: &ObjRef, key: &str) -> PyResult<Option<Value>> {
    if let Payload::Dict(d) = &m.payload {
        if !is_user_type(&m.class()) || lookup_user(&m.class(), "__getitem__").is_none() {
            let r = d.borrow().get_str(key).cloned();
            if r.is_none() && is_user_type(&m.class()) {
                if let Some(missing) = lookup_user(&m.class(), "__missing__") {
                    let k = vm.types.new_str(key);
                    return match vm.call(&missing, vec![Value::Obj(m.clone()), k], Vec::new()) {
                        Ok(v) => Ok(Some(v)),
                        Err(e) if vm.is_exc(&e, "KeyError") => Ok(None),
                        Err(e) => Err(e),
                    };
                }
            }
            return Ok(r);
        }
    }
    let k = vm.types.new_str(key);
    match getitem(vm, &Value::Obj(m.clone()), k) {
        Ok(v) => Ok(Some(v)),
        Err(e) if vm.is_exc(&e, "KeyError") => Ok(None),
        Err(e) => Err(e),
    }
}

/// (key, value) pairs of a mapping: dict payload or keys()/__getitem__.
pub fn mapping_items(vm: &mut Vm, m: &Value) -> PyResult<Vec<(Value, Value)>> {
    if let Value::Obj(o) = m {
        if let Payload::Dict(d) = &o.payload {
            if !is_user_type(&o.class()) || (lookup_user(&o.class(), "keys").is_none() && lookup_user(&o.class(), "__getitem__").is_none()) {
                return Ok(d.borrow().items());
            }
        }
    }
    let keys = call_special(vm, m, "keys", Vec::new())?;
    let Some(keys) = keys else {
        // Sequence of pairs.
        let mut out = Vec::new();
        for item in collect(vm, m)? {
            let kv = collect(vm, &item)?;
            if kv.len() != 2 {
                return Err(vm.value_error("dictionary update sequence element has wrong length"));
            }
            out.push((kv[0].clone(), kv[1].clone()));
        }
        return Ok(out);
    };
    let mut out = Vec::new();
    for k in collect(vm, &keys)? {
        let v = getitem(vm, m, k.clone())?;
        out.push((k, v));
    }
    Ok(out)
}

pub fn is_mapping(vm: &Vm, v: &Value) -> bool {
    matches!(v.payload(), Some(Payload::Dict(_))) || lookup_special(vm, v, "keys").is_some() && lookup_special(vm, v, "__getitem__").is_some()
}

pub fn is_sequence_for_match(vm: &Vm, v: &Value) -> bool {
    match v.payload() {
        Some(Payload::Str(_)) | Some(Payload::Bytes(_)) | Some(Payload::ByteArray(_)) | Some(Payload::Dict(_)) => false,
        Some(Payload::List(_)) | Some(Payload::Tuple(_)) | Some(Payload::Range(..)) => true,
        _ => lookup_special(vm, v, "__getitem__").is_some() && lookup_special(vm, v, "__len__").is_some() && !is_mapping(vm, v),
    }
}

pub fn match_class(vm: &mut Vm, subj: &Value, cls: &Value, npos: usize, names: &Value) -> PyResult<Value> {
    let Some(c) = cls.obj().filter(|c| c.type_data().is_some()) else {
        return Err(vm.type_error("called match pattern must be a type"));
    };
    if !isinstance(vm, subj, c) {
        return Ok(Value::None);
    }
    let mut out = Vec::new();
    if npos > 0 {
        let kind = c.type_data().unwrap().base_kind;
        let self_match = matches!(kind, BaseKind::Int | BaseKind::Bool | BaseKind::Float | BaseKind::Str | BaseKind::Bytes | BaseKind::Tuple | BaseKind::List | BaseKind::Dict | BaseKind::Set | BaseKind::FrozenSet)
            && lookup_type(c, "__match_args__").is_none();
        if self_match {
            if npos > 1 {
                return Err(vm.type_error("too many positional sub-patterns"));
            }
            out.push(subj.clone());
        } else {
            let ma = lookup_type(c, "__match_args__").ok_or_else(|| vm.type_error("class does not accept positional sub-patterns"))?;
            let ma = collect(vm, &ma)?;
            if npos > ma.len() {
                return Err(vm.type_error("too many positional sub-patterns"));
            }
            for n in &ma[..npos] {
                match getattr_v(vm, subj, n) {
                    Ok(v) => out.push(v),
                    Err(e) if vm.is_exc(&e, "AttributeError") => return Ok(Value::None),
                    Err(e) => return Err(e),
                }
            }
        }
    }
    for n in collect(vm, names)? {
        match getattr_v(vm, subj, &n) {
            Ok(v) => out.push(v),
            Err(e) if vm.is_exc(&e, "AttributeError") => return Ok(Value::None),
            Err(e) => return Err(e),
        }
    }
    Ok(vm.types.new_tuple(out))
}

// ---- items ----

pub fn normalize_index(vm: &Vm, i: i64, len: usize, what: &str) -> PyResult<usize> {
    let n = len as i64;
    let j = if i < 0 { i + n } else { i };
    if j < 0 || j >= n {
        return Err(vm.index_error(format!("{what} index out of range")));
    }
    Ok(j as usize)
}

/// Resolves a slice against a length: (start, stop, step, count).
pub fn slice_indices(vm: &mut Vm, sl: &Value, len: usize) -> PyResult<(i64, i64, i64, usize)> {
    let Some(Payload::Slice(a, b, c)) = sl.payload() else { unreachable!() };
    let (a, b, c) = (a.clone(), b.clone(), c.clone());
    let step = if c.is_none() { 1 } else { index_of(vm, &c)? };
    if step == 0 {
        return Err(vm.value_error("slice step cannot be zero"));
    }
    let n = len as i64;
    let clamp = |v: i64, lo: i64, hi: i64| v.max(lo).min(hi);
    let (lo, hi) = if step > 0 { (0, n) } else { (-1, n - 1) };
    let start = if a.is_none() {
        if step > 0 { 0 } else { n - 1 }
    } else {
        let s = slice_bound(vm, &a)?;
        clamp(if s < 0 { s + n } else { s }, lo, hi)
    };
    let stop = if b.is_none() {
        if step > 0 { n } else { -1 }
    } else {
        let s = slice_bound(vm, &b)?;
        clamp(if s < 0 { s + n } else { s }, lo, hi)
    };
    let count = if step > 0 {
        if stop > start { ((stop - start - 1) / step + 1) as usize } else { 0 }
    } else if start > stop {
        ((start - stop - 1) / (-step) + 1) as usize
    } else {
        0
    };
    Ok((start, stop, step, count))
}

fn slice_bound(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    match num::num_of(v) {
        Some(Num::B(b)) => Ok(if b.is_negative() { i64::MIN / 2 } else { i64::MAX / 2 }),
        _ => index_of(vm, v),
    }
}

fn slice_take<T: Clone>(items: &[T], start: i64, step: i64, count: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(count);
    let mut i = start;
    for _ in 0..count {
        out.push(items[i as usize].clone());
        i += step;
    }
    out
}

pub fn getitem(vm: &mut Vm, o: &Value, key: Value) -> PyResult<Value> {
    let Value::Obj(obj) = o else {
        return Err(vm.type_error(format!("'{}' object is not subscriptable", type_name(vm, o))));
    };
    let t = obj.class();
    if is_user_type(&t) || matches!(obj.payload, Payload::Native(_)) {
        let f = if is_user_type(&t) { lookup_user(&t, "__getitem__") } else { lookup_type(&t, "__getitem__") };
        if let Some(f) = f {
            return vm.call(&f, vec![o.clone(), key], Vec::new());
        }
    }
    let is_slice = matches!(key.payload(), Some(Payload::Slice(..)));
    match &obj.payload {
        Payload::List(l) => {
            if is_slice {
                let items = l.borrow().clone();
                let (s, _, st, n) = slice_indices(vm, &key, items.len())?;
                let r = slice_take(&items, s, st, n);
                return Ok(vm.types.new_list(r));
            }
            let i = index_of(vm, &key)?;
            let l = l.borrow();
            let j = normalize_index(vm, i, l.len(), "list")?;
            Ok(l[j].clone())
        }
        Payload::Tuple(items) => {
            if is_slice {
                let (s, _, st, n) = slice_indices(vm, &key, items.len())?;
                let r = slice_take(items, s, st, n);
                return Ok(vm.types.new_tuple(r));
            }
            let i = index_of(vm, &key)?;
            let j = normalize_index(vm, i, items.len(), "tuple")?;
            Ok(items[j].clone())
        }
        Payload::Str(s) => {
            if is_slice {
                let (start, _, st, n) = slice_indices(vm, &key, s.char_len())?;
                if st == 1 {
                    let r = s.slice_chars(start as usize, start as usize + n).to_string();
                    return Ok(vm.types.new_string(r));
                }
                let chars: Vec<char> = s.as_str().chars().collect();
                let r: String = slice_take(&chars, start, st, n).into_iter().collect();
                return Ok(vm.types.new_string(r));
            }
            let i = index_of(vm, &key)?;
            let j = normalize_index(vm, i, s.char_len(), "string")?;
            let c = s.char_at(j).unwrap();
            let mut b = [0u8; 4];
            Ok(vm.types.new_str(c.encode_utf8(&mut b)))
        }
        Payload::Bytes(b) => {
            if is_slice {
                let (s, _, st, n) = slice_indices(vm, &key, b.len())?;
                let r = slice_take(b, s, st, n);
                return Ok(vm.types.new_bytes(r));
            }
            let i = index_of(vm, &key)?;
            let j = normalize_index(vm, i, b.len(), "index")?;
            Ok(Value::Int(b[j] as i64))
        }
        Payload::ByteArray(b) => {
            let b = b.borrow().clone();
            if is_slice {
                let (s, _, st, n) = slice_indices(vm, &key, b.len())?;
                let r = slice_take(&b, s, st, n);
                return Ok(Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(r)))));
            }
            let i = index_of(vm, &key)?;
            let j = normalize_index(vm, i, b.len(), "bytearray")?;
            Ok(Value::Int(b[j] as i64))
        }
        Payload::Dict(_) => match dict_get(vm, obj, &key)? {
            Some(v) => Ok(v),
            None => {
                if is_user_type(&t) {
                    if let Some(m) = lookup_user(&t, "__missing__") {
                        return vm.call(&m, vec![o.clone(), key], Vec::new());
                    }
                }
                if let Some(m) = crate::builtins::dict_missing(vm, obj) {
                    return vm.call(&m, vec![key], Vec::new());
                }
                Err(vm.key_error(key))
            }
        },
        Payload::Range(a, b, s) => {
            let (a, b, s) = (*a, *b, *s);
            let n = range_len(a, b, s) as usize;
            if is_slice {
                let (st, _, step, cnt) = slice_indices(vm, &key, n)?;
                let start = a + st * s;
                let nstep = s * step;
                let stop = start + nstep * cnt as i64;
                return Ok(Value::Obj(new_obj(vm.types.range.clone(), None, Payload::Range(start, stop, nstep))));
            }
            let i = index_of(vm, &key)?;
            let j = normalize_index(vm, i, n, "range object")?;
            Ok(Value::Int(a + j as i64 * s))
        }
        Payload::Type(_) => {
            // Generic alias `list[int]` → the class itself; __class_getitem__.
            if let Some(cg) = lookup_type(obj, "__class_getitem__") {
                let f = match cg.payload() {
                    Some(Payload::ClassMethod(f)) => f.clone(),
                    _ => cg,
                };
                return vm.call(&f, vec![o.clone(), key], Vec::new());
            }
            Ok(o.clone())
        }
        _ => {
            if let Some(f) = lookup_type(&t, "__getitem__") {
                return vm.call(&f, vec![o.clone(), key], Vec::new());
            }
            Err(vm.type_error(format!("'{}' object is not subscriptable", obj.class_name())))
        }
    }
}

pub fn setitem(vm: &mut Vm, o: &Value, key: Value, v: Value) -> PyResult<()> {
    let Value::Obj(obj) = o else {
        return Err(vm.type_error(format!("'{}' object does not support item assignment", type_name(vm, o))));
    };
    let t = obj.class();
    if is_user_type(&t) || matches!(obj.payload, Payload::Native(_)) {
        let f = if is_user_type(&t) { lookup_user(&t, "__setitem__") } else { lookup_type(&t, "__setitem__") };
        if let Some(f) = f {
            vm.call(&f, vec![o.clone(), key, v], Vec::new())?;
            return Ok(());
        }
    }
    match &obj.payload {
        Payload::Dict(_) => dict_set(vm, obj, key, v),
        Payload::List(l) => {
            if matches!(key.payload(), Some(Payload::Slice(..))) {
                let items = collect(vm, &v)?;
                let len = l.borrow().len();
                let (start, stop, step, n) = slice_indices(vm, &key, len)?;
                let mut lb = l.borrow_mut();
                if step == 1 {
                    let stop = stop.max(start);
                    lb.splice(start as usize..stop as usize, items);
                    return Ok(());
                }
                if items.len() != n {
                    drop(lb);
                    return Err(vm.value_error(format!("attempt to assign sequence of size {} to extended slice of size {}", items.len(), n)));
                }
                let mut i = start;
                for it in items {
                    lb[i as usize] = it;
                    i += step;
                }
                return Ok(());
            }
            let i = index_of(vm, &key)?;
            let len = l.borrow().len();
            let j = normalize_index(vm, i, len, "list assignment")?;
            l.borrow_mut()[j] = v;
            Ok(())
        }
        Payload::ByteArray(b) => {
            let i = index_of(vm, &key)?;
            let len = b.borrow().len();
            let j = normalize_index(vm, i, len, "bytearray")?;
            let x = index_of(vm, &v)?;
            b.borrow_mut()[j] = x as u8;
            Ok(())
        }
        _ => {
            if let Some(f) = lookup_type(&t, "__setitem__") {
                vm.call(&f, vec![o.clone(), key, v], Vec::new())?;
                return Ok(());
            }
            Err(vm.type_error(format!("'{}' object does not support item assignment", obj.class_name())))
        }
    }
}

pub fn delitem(vm: &mut Vm, o: &Value, key: Value) -> PyResult<()> {
    let Value::Obj(obj) = o else {
        return Err(vm.type_error(format!("'{}' object does not support item deletion", type_name(vm, o))));
    };
    let t = obj.class();
    if is_user_type(&t) || matches!(obj.payload, Payload::Native(_)) {
        let f = if is_user_type(&t) { lookup_user(&t, "__delitem__") } else { lookup_type(&t, "__delitem__") };
        if let Some(f) = f {
            vm.call(&f, vec![o.clone(), key], Vec::new())?;
            return Ok(());
        }
    }
    match &obj.payload {
        Payload::Dict(_) => dict_del(vm, obj, key),
        Payload::List(l) => {
            if matches!(key.payload(), Some(Payload::Slice(..))) {
                let len = l.borrow().len();
                let (start, _, step, n) = slice_indices(vm, &key, len)?;
                let mut idx: Vec<usize> = (0..n).map(|k| (start + k as i64 * step) as usize).collect();
                idx.sort_unstable();
                let mut lb = l.borrow_mut();
                for i in idx.into_iter().rev() {
                    lb.remove(i);
                }
                return Ok(());
            }
            let i = index_of(vm, &key)?;
            let len = l.borrow().len();
            let j = normalize_index(vm, i, len, "list assignment")?;
            l.borrow_mut().remove(j);
            Ok(())
        }
        _ => {
            if let Some(f) = lookup_type(&t, "__delitem__") {
                vm.call(&f, vec![o.clone(), key], Vec::new())?;
                return Ok(());
            }
            Err(vm.type_error(format!("'{}' object does not support item deletion", obj.class_name())))
        }
    }
}

// ---- iteration ----

pub fn iter(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let Value::Obj(o) = v else {
        return Err(vm.type_error(format!("'{}' object is not iterable", type_name(vm, v))));
    };
    let t = o.class();
    if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
        let f = if is_user_type(&t) { lookup_user(&t, "__iter__") } else { lookup_type(&t, "__iter__") };
        if let Some(f) = f {
            if f.is_none() {
                return Err(vm.type_error(format!("'{}' object is not iterable", o.class_name())));
            }
            return vm.call(&f, vec![v.clone()], Vec::new());
        }
        if is_user_type(&t) && matches!(o.payload, Payload::None) {
            if lookup_user(&t, "__getitem__").is_some() {
                return Ok(vm.types.new_iter(IterState::Seq { seq: v.clone(), idx: 0 }));
            }
            return Err(vm.type_error(format!("'{}' object is not iterable", o.class_name())));
        }
    }
    let st = match &o.payload {
        Payload::List(_) | Payload::Tuple(_) | Payload::Bytes(_) | Payload::ByteArray(_) => IterState::Seq { seq: v.clone(), idx: 0 },
        Payload::Str(_) => IterState::StrChars { s: o.clone(), byte: 0 },
        Payload::Dict(d) => IterState::DictKeys { dict: o.clone(), idx: 0, len: d.borrow().len() },
        Payload::Set(d) => IterState::Values { items: d.borrow().keys(), idx: 0 },
        Payload::FrozenSet(d) => IterState::Values { items: d.keys(), idx: 0 },
        Payload::Range(a, b, s) => IterState::Range { cur: *a, stop: *b, step: *s },
        Payload::BigRange(r) => IterState::BigRange { cur: r.0.clone(), stop: r.1.clone(), step: r.2.clone() },
        Payload::DictView { dict, kind } => {
            let Payload::Dict(d) = &dict.payload else { return Err(vm.type_error("bad view")) };
            let len = d.borrow().len();
            match kind {
                0 => IterState::DictKeys { dict: dict.clone(), idx: 0, len },
                1 => IterState::DictValues { dict: dict.clone(), idx: 0, len },
                _ => IterState::DictItems { dict: dict.clone(), idx: 0, len },
            }
        }
        Payload::Iter(_) | Payload::Generator(_) => return Ok(v.clone()),
        _ => {
            if let Some(f) = lookup_type(&t, "__iter__") {
                return vm.call(&f, vec![v.clone()], Vec::new());
            }
            if lookup_type(&t, "__getitem__").is_some() {
                return Ok(vm.types.new_iter(IterState::Seq { seq: v.clone(), idx: 0 }));
            }
            return Err(vm.type_error(format!("'{}' object is not iterable", o.class_name())));
        }
    };
    Ok(vm.types.new_iter(st))
}

/// Advances an iterator: Some(item) or None when exhausted.
pub fn next(vm: &mut Vm, it: &Value) -> PyResult<Option<Value>> {
    let Value::Obj(o) = it else {
        return Err(vm.type_error(format!("'{}' object is not an iterator", type_name(vm, it))));
    };
    match &o.payload {
        Payload::Iter(st) => iter_next(vm, o, st),
        Payload::Generator(_) => match vm.gen_resume(o, Resume::Send(Value::None))? {
            GenStep::Yielded(v) => Ok(Some(v)),
            GenStep::Returned(_) => Ok(None),
        },
        _ => {
            let t = o.class();
            let f = lookup_type(&t, "__next__").or_else(|| lookup_type(&t, "next"));
            let Some(f) = f else {
                return Err(vm.type_error(format!("'{}' object is not an iterator", o.class_name())));
            };
            match vm.call(&f, vec![it.clone()], Vec::new()) {
                Ok(v) => Ok(Some(v)),
                Err(e) if vm.is_exc(&e, "StopIteration") => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

fn iter_next(vm: &mut Vm, o: &ObjRef, st: &RefCell<IterState>) -> PyResult<Option<Value>> {
    // Simple states advance under one borrow; states needing calls release it.
    enum Call {
        Item(Value),
        Seq(Value, usize),
        Enumerate(Value, Value),
        Zip(Vec<Value>, bool),
        Map(Value, Vec<Value>),
        Filter(Value, Value, bool),
        Callable(Value, Value),
        Native,
    }
    let call = {
        let mut s = st.borrow_mut();
        match &mut *s {
            IterState::Exhausted => return Ok(None),
            IterState::Seq { seq, idx } => match seq.payload() {
                Some(Payload::List(l)) => {
                    let l = l.borrow();
                    if *idx < l.len() {
                        *idx += 1;
                        Call::Item(l[*idx - 1].clone())
                    } else {
                        drop(l);
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                }
                Some(Payload::Tuple(t)) => {
                    if *idx < t.len() {
                        *idx += 1;
                        Call::Item(t[*idx - 1].clone())
                    } else {
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                }
                Some(Payload::Bytes(b)) => {
                    if *idx < b.len() {
                        *idx += 1;
                        Call::Item(Value::Int(b[*idx - 1] as i64))
                    } else {
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                }
                Some(Payload::ByteArray(b)) => {
                    let b = b.borrow();
                    if *idx < b.len() {
                        *idx += 1;
                        Call::Item(Value::Int(b[*idx - 1] as i64))
                    } else {
                        drop(b);
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                }
                _ => {
                    *idx += 1;
                    Call::Seq(seq.clone(), *idx - 1)
                }
            },
            IterState::StrChars { s: so, byte } => {
                let Payload::Str(ps) = &so.payload else { unreachable!() };
                let rest = &ps.as_str()[*byte..];
                match rest.chars().next() {
                    Some(c) => {
                        *byte += c.len_utf8();
                        let mut b = [0u8; 4];
                        Call::Item(vm.types.new_str(c.encode_utf8(&mut b)))
                    }
                    None => {
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                }
            }
            IterState::Range { cur, stop, step } => {
                if (*step > 0 && *cur < *stop) || (*step < 0 && *cur > *stop) {
                    let v = *cur;
                    *cur += *step;
                    Call::Item(Value::Int(v))
                } else {
                    *s = IterState::Exhausted;
                    return Ok(None);
                }
            }
            IterState::BigRange { cur, stop, step } => {
                if (step.is_positive() && *cur < *stop) || (step.is_negative() && *cur > *stop) {
                    let v = cur.clone();
                    *cur += &*step;
                    Call::Item(vm.types.new_bigint(v))
                } else {
                    *s = IterState::Exhausted;
                    return Ok(None);
                }
            }
            IterState::DictKeys { dict, idx, len } | IterState::DictValues { dict, idx, len } | IterState::DictItems { dict, idx, len } => {
                let Payload::Dict(d) = &dict.payload else { unreachable!() };
                let d = d.borrow();
                if d.len() != *len {
                    drop(d);
                    return Err(vm.runtime_error("dictionary changed size during iteration"));
                }
                let mut found = None;
                while *idx < d.slots() {
                    let i = *idx;
                    *idx += 1;
                    if let Some(e) = d.entry(i) {
                        found = Some((e.key.clone(), e.value.clone()));
                        break;
                    }
                }
                drop(d);
                let kind = match &*s {
                    IterState::DictKeys { .. } => 0,
                    IterState::DictValues { .. } => 1,
                    _ => 2,
                };
                match found {
                    None => {
                        *s = IterState::Exhausted;
                        return Ok(None);
                    }
                    Some((k, v)) => Call::Item(match kind {
                        0 => k,
                        1 => v,
                        _ => vm.types.new_tuple(vec![k, v]),
                    }),
                }
            }
            IterState::Values { items, idx } => {
                if *idx < items.len() {
                    *idx += 1;
                    Call::Item(items[*idx - 1].clone())
                } else {
                    *s = IterState::Exhausted;
                    return Ok(None);
                }
            }
            IterState::Reversed { seq, idx } => {
                if *idx < 0 {
                    *s = IterState::Exhausted;
                    return Ok(None);
                }
                *idx -= 1;
                Call::Seq(seq.clone(), (*idx + 1) as usize)
            }
            IterState::Enumerate { it, n } => {
                let r = (it.clone(), n.clone());
                let next = match &*n {
                    Value::Int(i) => Value::Int(*i + 1),
                    other => other.clone(),
                };
                *n = next;
                Call::Enumerate(r.0, r.1)
            }
            IterState::Zip { its, strict } => Call::Zip(its.clone(), *strict),
            IterState::Map { func, its } => Call::Map(func.clone(), its.clone()),
            IterState::Filter { func, it, invert } => Call::Filter(func.clone(), it.clone(), *invert),
            IterState::Callable { func, sentinel } => Call::Callable(func.clone(), sentinel.clone()),
            IterState::Native(_) => Call::Native,
        }
    };
    match call {
        Call::Item(v) => Ok(Some(v)),
        Call::Seq(seq, i) => match getitem(vm, &seq, Value::Int(i as i64)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if vm.is_exc(&e, "IndexError") || vm.is_exc(&e, "StopIteration") => {
                *st.borrow_mut() = IterState::Exhausted;
                Ok(None)
            }
            Err(e) => Err(e),
        },
        Call::Enumerate(it, n) => match next(vm, &it)? {
            Some(v) => {
                if !matches!(n, Value::Int(_)) {
                    let n2 = binary_op(vm, &n, &Value::Int(1), BinOp::Add, false, false)?;
                    if let IterState::Enumerate { n: slot, .. } = &mut *st.borrow_mut() {
                        *slot = n2;
                    }
                }
                Ok(Some(vm.types.new_tuple(vec![n, v])))
            }
            None => {
                *st.borrow_mut() = IterState::Exhausted;
                Ok(None)
            }
        },
        Call::Zip(its, strict) => {
            let mut out = Vec::with_capacity(its.len());
            for (i, it) in its.iter().enumerate() {
                match next(vm, it)? {
                    Some(v) => out.push(v),
                    None => {
                        if strict && (i > 0 || its[1..].iter().any(|_| true)) {
                            if i > 0 {
                                *st.borrow_mut() = IterState::Exhausted;
                                return Err(vm.value_error(format!("zip() argument {} is shorter than argument 1", i + 1)));
                            }
                            for (j, other) in its[1..].iter().enumerate() {
                                if next(vm, other)?.is_some() {
                                    *st.borrow_mut() = IterState::Exhausted;
                                    return Err(vm.value_error(format!("zip() argument {} is longer than argument 1", j + 2)));
                                }
                            }
                        }
                        *st.borrow_mut() = IterState::Exhausted;
                        return Ok(None);
                    }
                }
            }
            if its.is_empty() {
                return Ok(None);
            }
            Ok(Some(vm.types.new_tuple(out)))
        }
        Call::Map(func, its) => {
            let mut args = Vec::with_capacity(its.len());
            for it in &its {
                match next(vm, it)? {
                    Some(v) => args.push(v),
                    None => {
                        *st.borrow_mut() = IterState::Exhausted;
                        return Ok(None);
                    }
                }
            }
            vm.call(&func, args, Vec::new()).map(Some)
        }
        Call::Filter(func, it, invert) => loop {
            let Some(v) = next(vm, &it)? else {
                *st.borrow_mut() = IterState::Exhausted;
                return Ok(None);
            };
            let keep = if func.is_none() { truthy(vm, &v)? } else {
                let r = vm.call(&func, vec![v.clone()], Vec::new())?;
                truthy(vm, &r)?
            };
            if keep != invert {
                return Ok(Some(v));
            }
        },
        Call::Callable(func, sentinel) => {
            let v = vm.call0(&func)?;
            if eq(vm, &v, &sentinel)? {
                *st.borrow_mut() = IterState::Exhausted;
                return Ok(None);
            }
            Ok(Some(v))
        }
        Call::Native => crate::builtins::native_iter_next(vm, o),
    }
}

pub fn collect(vm: &mut Vm, v: &Value) -> PyResult<Vec<Value>> {
    if let Value::Obj(o) = v {
        if !is_user_type(&o.class()) {
            match &o.payload {
                Payload::List(l) => return Ok(l.borrow().clone()),
                Payload::Tuple(t) => return Ok(t.to_vec()),
                _ => {}
            }
        }
    }
    let it = iter(vm, v)?;
    let mut out = Vec::new();
    while let Some(x) = next(vm, &it)? {
        out.push(x);
    }
    Ok(out)
}

pub fn stop_value(vm: &mut Vm, e: &PyErr) -> Value {
    match e {
        PyErr::Exc(o) => get_raw_attr(o, "value").unwrap_or_else(|| {
            let _ = vm;
            Value::None
        }),
        _ => Value::None,
    }
}

// ---- exceptions ----

pub fn add_traceback(vm: &mut Vm, exc: &ObjRef, entry: (Rc<str>, u32, Rc<str>)) {
    let Some(d) = &exc.dict else { return };
    let tb = d.borrow().get_str("__traceback__").cloned();
    let list = match tb {
        Some(l @ Value::Obj(_)) if matches!(l.payload(), Some(Payload::List(_))) => l,
        _ => {
            let l = vm.types.new_list(Vec::new());
            d.borrow_mut().set_str(vm.types.intern("__traceback__"), l.clone());
            l
        }
    };
    let fname = vm.types.new_str(&entry.0);
    let name = vm.types.new_str(&entry.2);
    let t = vm.types.new_tuple(vec![fname, Value::Int(entry.1 as i64), name]);
    if let Some(Payload::List(l)) = list.payload() {
        let mut l = l.borrow_mut();
        // Frames are visited innermost first; tracebacks list outermost first.
        l.insert(0, t);
    }
}

pub fn exc_matches(vm: &mut Vm, exc: &Value, cls: &Value) -> PyResult<bool> {
    match cls.payload() {
        Some(Payload::Tuple(t)) => {
            for c in t.to_vec() {
                if exc_matches(vm, exc, &c)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Some(Payload::Type(_)) => Ok(isinstance(vm, exc, cls.obj().unwrap())),
        _ => Err(vm.type_error("catching classes that do not inherit from BaseException is not allowed")),
    }
}

// ---- operators ----

fn op_names(op: BinOp) -> (&'static str, &'static str, &'static str) {
    match op {
        BinOp::Add => ("__add__", "__radd__", "__iadd__"),
        BinOp::Sub => ("__sub__", "__rsub__", "__isub__"),
        BinOp::Mult => ("__mul__", "__rmul__", "__imul__"),
        BinOp::MatMult => ("__matmul__", "__rmatmul__", "__imatmul__"),
        BinOp::Div => ("__truediv__", "__rtruediv__", "__itruediv__"),
        BinOp::Mod => ("__mod__", "__rmod__", "__imod__"),
        BinOp::Pow => ("__pow__", "__rpow__", "__ipow__"),
        BinOp::LShift => ("__lshift__", "__rlshift__", "__ilshift__"),
        BinOp::RShift => ("__rshift__", "__rrshift__", "__irshift__"),
        BinOp::BitOr => ("__or__", "__ror__", "__ior__"),
        BinOp::BitXor => ("__xor__", "__rxor__", "__ixor__"),
        BinOp::BitAnd => ("__and__", "__rand__", "__iand__"),
        BinOp::FloorDiv => ("__floordiv__", "__rfloordiv__", "__ifloordiv__"),
    }
}

pub fn op_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mult => "*",
        BinOp::MatMult => "@",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "** or pow()",
        BinOp::LShift => "<<",
        BinOp::RShift => ">>",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::BitAnd => "&",
        BinOp::FloorDiv => "//",
    }
}

fn special_for(t: &ObjRef, v: &Value, name: &str) -> Option<Value> {
    if matches!(v.payload(), Some(Payload::Native(_))) {
        return lookup_type(t, name);
    }
    lookup_user(t, name)
}

pub fn binary_op(vm: &mut Vm, a: &Value, b: &Value, op: BinOp, inplace: bool, classic_div: bool) -> PyResult<Value> {
    // Integer fast paths.
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        if let Some(v) = num::int_fast(vm, *x, *y, op, classic_div)? {
            return Ok(v);
        }
    }
    let ta = type_of(vm, a);
    let tb = type_of(vm, b);
    let (name, rname, iname) = op_names(op);
    let name = if op == BinOp::Div && classic_div { "__div__" } else { name };
    let rname = if op == BinOp::Div && classic_div { "__rdiv__" } else { rname };
    let ua = is_user_type(&ta) || matches!(a.payload(), Some(Payload::Native(_)));
    let ub = is_user_type(&tb) || matches!(b.payload(), Some(Payload::Native(_)));
    if inplace && ua {
        if let Some(f) = special_for(&ta, a, iname) {
            let r = vm.call(&f, vec![a.clone(), b.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                return Ok(r);
            }
        }
    }
    if inplace && !ua {
        // In-place list/set/dict updates keep identity.
        if let Some(r) = native_inplace(vm, a, b, op)? {
            return Ok(r);
        }
    }
    if ua || ub {
        let mut fa = if ua { special_for(&ta, a, name) } else { None };
        if fa.is_none() && ua && op == BinOp::Div && classic_div {
            fa = special_for(&ta, a, "__truediv__");
        }
        let mut fb = if ub && !Rc::ptr_eq(&ta, &tb) { special_for(&tb, b, rname) } else { None };
        if fb.is_none() && ub && !Rc::ptr_eq(&ta, &tb) && op == BinOp::Div && classic_div {
            fb = special_for(&tb, b, "__rtruediv__");
        }
        let b_first = fb.is_some() && is_subclass(&tb, &ta);
        if b_first {
            let r = vm.call(fb.as_ref().unwrap(), vec![b.clone(), a.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                return Ok(r);
            }
        }
        if let Some(f) = &fa {
            let r = vm.call(f, vec![a.clone(), b.clone()], Vec::new())?;
            if !is_not_implemented(&r) {
                return Ok(r);
            }
        }
        if !b_first {
            if let Some(f) = &fb {
                let r = vm.call(f, vec![b.clone(), a.clone()], Vec::new())?;
                if !is_not_implemented(&r) {
                    return Ok(r);
                }
            }
        }
    }
    if let Some(r) = native_binary(vm, a, b, op, classic_div)? {
        return Ok(r);
    }
    Err(vm.type_error(format!(
        "unsupported operand type(s) for {}: '{}' and '{}'",
        op_symbol(op),
        type_name(vm, a),
        type_name(vm, b)
    )))
}

fn native_inplace(vm: &mut Vm, a: &Value, b: &Value, op: BinOp) -> PyResult<Option<Value>> {
    let Value::Obj(o) = a else { return Ok(None) };
    match (&o.payload, op) {
        (Payload::List(l), BinOp::Add) => {
            let items = collect(vm, b)?;
            l.borrow_mut().extend(items);
            Ok(Some(a.clone()))
        }
        (Payload::List(l), BinOp::Mult) => {
            let n = index_of(vm, b)?.max(0) as usize;
            let cur = l.borrow().clone();
            let mut out = Vec::with_capacity(cur.len() * n);
            for _ in 0..n {
                out.extend(cur.iter().cloned());
            }
            *l.borrow_mut() = out;
            Ok(Some(a.clone()))
        }
        (Payload::Set(_), BinOp::BitOr | BinOp::BitAnd | BinOp::Sub | BinOp::BitXor) if matches!(b.payload(), Some(Payload::Set(_) | Payload::FrozenSet(_))) => {
            let r = native_binary(vm, a, b, op, false)?.unwrap();
            let Some(Payload::Set(rd)) = r.payload() else { unreachable!() };
            let Payload::Set(d) = &o.payload else { unreachable!() };
            *d.borrow_mut() = rd.borrow().clone();
            Ok(Some(a.clone()))
        }
        (Payload::Dict(_), BinOp::BitOr) if matches!(b.payload(), Some(Payload::Dict(_))) => {
            for (k, v) in mapping_items(vm, b)? {
                dict_set(vm, o, k, v)?;
            }
            Ok(Some(a.clone()))
        }
        (Payload::ByteArray(x), BinOp::Add) => {
            let y = bytes_of(b).ok_or_else(|| vm.type_error("can't concat"))?;
            x.borrow_mut().extend(y);
            Ok(Some(a.clone()))
        }
        _ => Ok(None),
    }
}

fn repeat<T: Clone>(items: &[T], n: i64) -> Vec<T> {
    let n = n.max(0) as usize;
    let mut out = Vec::with_capacity(items.len() * n);
    for _ in 0..n {
        out.extend_from_slice(items);
    }
    out
}

pub fn native_binary(vm: &mut Vm, a: &Value, b: &Value, op: BinOp, classic_div: bool) -> PyResult<Option<Value>> {
    if let (Some(x), Some(y)) = (num::num_of(a), num::num_of(b)) {
        return num::num_binary(vm, x, y, op, classic_div).map(Some);
    }
    let (pa, pb) = (a.payload(), b.payload());
    match op {
        BinOp::Add => match (pa, pb) {
            (Some(Payload::Str(x)), Some(Payload::Str(y))) => {
                let mut s = String::with_capacity(x.as_str().len() + y.as_str().len());
                s.push_str(x.as_str());
                s.push_str(y.as_str());
                return Ok(Some(vm.types.new_string(s)));
            }
            (Some(Payload::List(x)), Some(Payload::List(y))) => {
                let mut v = x.borrow().clone();
                v.extend(y.borrow().iter().cloned());
                return Ok(Some(vm.types.new_list(v)));
            }
            (Some(Payload::Tuple(x)), Some(Payload::Tuple(y))) => {
                let mut v = x.to_vec();
                v.extend(y.iter().cloned());
                return Ok(Some(vm.types.new_tuple(v)));
            }
            (Some(Payload::Bytes(_) | Payload::ByteArray(_)), Some(Payload::Bytes(_) | Payload::ByteArray(_))) => {
                let mut v = bytes_of(a).unwrap();
                v.extend(bytes_of(b).unwrap());
                if matches!(pa, Some(Payload::ByteArray(_))) {
                    return Ok(Some(Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(v))))));
                }
                return Ok(Some(vm.types.new_bytes(v)));
            }
            _ => {}
        },
        BinOp::Mult => {
            let (seq, n) = match (num::num_of(a), num::num_of(b)) {
                (Some(Num::I(n)), None) => (b, n),
                (None, Some(Num::I(n))) => (a, n),
                (Some(Num::B(_)), None) | (None, Some(Num::B(_))) => return Err(vm.exc_msg("OverflowError", "cannot fit 'int' into an index-sized integer")),
                _ => return Ok(None),
            };
            match seq.payload() {
                Some(Payload::Str(s)) => return Ok(Some(vm.types.new_string(s.as_str().repeat(n.max(0) as usize)))),
                Some(Payload::List(l)) => {
                    let r = repeat(&l.borrow(), n);
                    return Ok(Some(vm.types.new_list(r)));
                }
                Some(Payload::Tuple(t)) => return Ok(Some(vm.types.new_tuple(repeat(t, n)))),
                Some(Payload::Bytes(x)) => return Ok(Some(vm.types.new_bytes(repeat(x, n)))),
                _ => {}
            }
        }
        BinOp::Mod => match pa {
            Some(Payload::Str(fmt)) => {
                let f = fmt.as_str().to_string();
                let s = crate::format::percent_format(vm, &f, b)?;
                return Ok(Some(vm.types.new_string(s)));
            }
            Some(Payload::Bytes(fmt)) => {
                let f = String::from_utf8_lossy(fmt).to_string();
                let s = crate::format::percent_format(vm, &f, b)?;
                return Ok(Some(vm.types.new_bytes(s.into_bytes())));
            }
            _ => {}
        },
        BinOp::BitOr | BinOp::BitAnd | BinOp::Sub | BinOp::BitXor => {
            let is_set = |p: Option<&Payload>| matches!(p, Some(Payload::Set(_) | Payload::FrozenSet(_)));
            if is_set(pa) && is_set(pb) {
                let ka = set_keys(a);
                let kb = set_keys(b);
                let mut out = Vec::new();
                match op {
                    BinOp::BitOr => {
                        out.extend(ka);
                        out.extend(kb);
                    }
                    BinOp::BitAnd => {
                        for k in ka {
                            if set_contains(vm, b, &k)? {
                                out.push(k);
                            }
                        }
                    }
                    BinOp::Sub => {
                        for k in ka {
                            if !set_contains(vm, b, &k)? {
                                out.push(k);
                            }
                        }
                    }
                    _ => {
                        for k in ka {
                            if !set_contains(vm, b, &k)? {
                                out.push(k);
                            }
                        }
                        for k in kb {
                            if !set_contains(vm, a, &k)? {
                                out.push(k);
                            }
                        }
                    }
                }
                let t = make_table(vm, out)?;
                return Ok(Some(if matches!(pa, Some(Payload::FrozenSet(_))) { vm.types.new_frozenset(t) } else { vm.types.new_set(t) }));
            }
            if op == BinOp::BitOr {
                if let (Some(Payload::Dict(x)), Some(Payload::Dict(_))) = (pa, pb) {
                    let d = vm.types.new_dict_obj(x.borrow().clone());
                    for (k, v) in mapping_items(vm, b)? {
                        dict_set(vm, &d, k, v)?;
                    }
                    return Ok(Some(Value::Obj(d)));
                }
            }
            // Dict key views behave like sets.
            if let (Some(Payload::DictView { kind: 0, .. }), _) | (_, Some(Payload::DictView { kind: 0, .. })) = (pa, pb) {
                let la = collect(vm, a)?;
                let lb = collect(vm, b)?;
                let sa = make_set(vm, la)?;
                let sb = make_set(vm, lb)?;
                return native_binary(vm, &sa, &sb, op, false);
            }
        }
        _ => {}
    }
    Ok(None)
}

pub fn unary_op(vm: &mut Vm, a: &Value, op: UnaryOp) -> PyResult<Value> {
    if op == UnaryOp::Not {
        return Ok(Value::Bool(!truthy(vm, a)?));
    }
    let t = type_of(vm, a);
    let name = match op {
        UnaryOp::USub => "__neg__",
        UnaryOp::UAdd => "__pos__",
        UnaryOp::Invert => "__invert__",
        UnaryOp::Not => unreachable!(),
    };
    if is_user_type(&t) || matches!(a.payload(), Some(Payload::Native(_))) {
        if let Some(f) = special_for(&t, a, name) {
            return vm.call(&f, vec![a.clone()], Vec::new());
        }
    }
    match num::num_of(a) {
        Some(n) => num::num_unary(vm, n, op),
        None => Err(vm.type_error(format!("bad operand type for unary {}: '{}'", match op {
            UnaryOp::USub => "-",
            UnaryOp::UAdd => "+",
            _ => "~",
        }, type_name(vm, a)))),
    }
}

// ---- conversion to text ----

pub fn repr(vm: &mut Vm, v: &Value) -> PyResult<String> {
    if let Value::Obj(o) = v {
        let t = o.class();
        if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
            if let Some(f) = special_for(&t, v, "__repr__").filter(|f| !is_object_default(&t, f, "__repr__")) {
                let r = vm.call(&f, vec![v.clone()], Vec::new())?;
                return match r.as_str() {
                    Some(s) => Ok(s.to_string()),
                    None => Err(vm.type_error(format!("__repr__ returned non-string (type {})", type_name(vm, &r)))),
                };
            }
        }
    }
    crate::format::native_repr(vm, v)
}

pub fn str(vm: &mut Vm, v: &Value) -> PyResult<String> {
    match v {
        Value::Obj(o) => {
            if let Payload::Str(s) = &o.payload {
                if !is_user_type(&o.class()) {
                    return Ok(s.as_str().to_string());
                }
            }
            let t = o.class();
            if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
                let f = special_for(&t, v, "__str__").filter(|f| !is_object_default(&t, f, "__str__"));
                let f = f.or_else(|| special_for(&t, v, "__unicode__"));
                if let Some(f) = f {
                    let r = vm.call(&f, vec![v.clone()], Vec::new())?;
                    return match r.as_str() {
                        Some(s) => Ok(s.to_string()),
                        None => Err(vm.type_error(format!("__str__ returned non-string (type {})", type_name(vm, &r)))),
                    };
                }
                if matches!(o.payload, Payload::Str(_)) {
                    return Ok(o.as_str().unwrap().as_str().to_string());
                }
            }
            if crate::ops::is_subclass(&t, &vm.types.exc("BaseException")) {
                return crate::format::exception_str(vm, o);
            }
            repr(vm, v)
        }
        _ => crate::format::native_repr(vm, v),
    }
}

pub fn ascii(vm: &mut Vm, v: &Value) -> PyResult<String> {
    let r = repr(vm, v)?;
    let mut out = String::with_capacity(r.len());
    for c in r.chars() {
        let n = c as u32;
        if n < 0x80 {
            out.push(c);
        } else if n < 0x100 {
            out.push_str(&format!("\\x{n:02x}"));
        } else if n < 0x10000 {
            out.push_str(&format!("\\u{n:04x}"));
        } else {
            out.push_str(&format!("\\U{n:08x}"));
        }
    }
    Ok(out)
}

/// `format(value, spec)`.
pub fn format(vm: &mut Vm, v: &Value, spec: &str) -> PyResult<String> {
    if let Value::Obj(o) = v {
        let t = o.class();
        if is_user_type(&t) || matches!(o.payload, Payload::Native(_)) {
            if let Some(f) = special_for(&t, v, "__format__") {
                let s = vm.types.new_str(spec);
                let r = vm.call(&f, vec![v.clone(), s], Vec::new())?;
                return Ok(r.as_str().unwrap_or("").to_string());
            }
        }
    }
    crate::format::format_value(vm, v, spec)
}

pub fn to_bigint(v: &Value) -> Option<BigInt> {
    match num::num_of(v)? {
        Num::I(i) => Some(BigInt::from(i)),
        Num::B(b) => Some(b),
        _ => None,
    }
}
