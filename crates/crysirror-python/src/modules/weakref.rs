// SPDX-License-Identifier: MPL-2.0
//! `_weakref`: weak reference objects and proxies. Referent bookkeeping,
//! clearing and callbacks live in `crate::gc`; this module provides the
//! script-visible types. Proxies are `Payload::WeakRef` objects of two
//! classes defined in Python (so attribute access and every operator can be
//! forwarded through ordinary special-method lookup).

use super::{module_with, set};
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const PROXY_SRC: &str = r##"
import operator as _op

def _unwrap(o):
    t = type(o)
    if t is ProxyType or t is CallableProxyType:
        return _proxy_target(o)
    return o

class ProxyType(object):
    __slots__ = ()
    __hash__ = None
    def __getattribute__(self, name):
        return getattr(_proxy_target(self), name)
    def __setattr__(self, name, value):
        setattr(_proxy_target(self), name, value)
    def __delattr__(self, name):
        delattr(_proxy_target(self), name)
    def __repr__(self):
        return _proxy_repr(self)
    def __str__(self):
        return str(_proxy_target(self))
    def __bytes__(self):
        return bytes(_proxy_target(self))
    def __bool__(self):
        return bool(_proxy_target(self))
    __nonzero__ = __bool__
    def __len__(self):
        return len(_proxy_target(self))
    def __iter__(self):
        return iter(_proxy_target(self))
    def __next__(self):
        return next(_proxy_target(self))
    next = __next__
    def __reversed__(self):
        return reversed(_proxy_target(self))
    def __contains__(self, item):
        return _unwrap(item) in _proxy_target(self)
    def __getitem__(self, key):
        return _proxy_target(self)[key]
    def __setitem__(self, key, value):
        _proxy_target(self)[key] = value
    def __delitem__(self, key):
        del _proxy_target(self)[key]
    def __index__(self):
        return _op.index(_proxy_target(self))
    def __int__(self):
        return int(_proxy_target(self))
    def __float__(self):
        return float(_proxy_target(self))
    def __abs__(self):
        return abs(_proxy_target(self))
    def __neg__(self):
        return -_proxy_target(self)
    def __pos__(self):
        return +_proxy_target(self)
    def __invert__(self):
        return ~_proxy_target(self)
    def __eq__(self, other):
        return _proxy_target(self) == _unwrap(other)
    def __ne__(self, other):
        return _proxy_target(self) != _unwrap(other)
    def __lt__(self, other):
        return _proxy_target(self) < _unwrap(other)
    def __le__(self, other):
        return _proxy_target(self) <= _unwrap(other)
    def __gt__(self, other):
        return _proxy_target(self) > _unwrap(other)
    def __ge__(self, other):
        return _proxy_target(self) >= _unwrap(other)

def _forward(f):
    def op(self, other):
        return f(_unwrap(self), _unwrap(other))
    def rop(self, other):
        return f(_unwrap(other), _unwrap(self))
    return op, rop

for _n, _f in (('add', _op.add), ('sub', _op.sub), ('mul', _op.mul), ('truediv', _op.truediv),
               ('floordiv', _op.floordiv), ('mod', _op.mod), ('pow', pow), ('lshift', _op.lshift),
               ('rshift', _op.rshift), ('and', _op.and_), ('or', _op.or_), ('xor', _op.xor),
               ('divmod', divmod)):
    _a, _b = _forward(_f)
    setattr(ProxyType, '__%s__' % _n, _a)
    setattr(ProxyType, '__r%s__' % _n, _b)
    if _n != 'divmod':
        setattr(ProxyType, '__i%s__' % _n, _a)
del _n, _f, _a, _b

class CallableProxyType(ProxyType):
    __slots__ = ()
    def __call__(self, *args, **kwargs):
        return _proxy_target(self)(*args, **kwargs)

ProxyType.__name__ = ProxyType.__qualname__ = 'weakproxy'
CallableProxyType.__name__ = CallableProxyType.__qualname__ = 'weakcallableproxy'

def proxy(object, callback=None):
    return _new_proxy(CallableProxyType if callable(object) else ProxyType, object, callback)

def _remove_dead_weakref(d, key):
    try:
        wr = d[key]
    except KeyError:
        return
    if wr() is None:
        del d[key]
"##;

/// Attaches the methods of the builtin `weakref` (`ReferenceType`) type.
pub fn install_type(vm: &mut Vm) {
    let t = vm.types.weakref.clone();
    let b = vm.types.new_builtin("__new__", weakref_new);
    let sm = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(crate::dict::Dict::new()), Payload::StaticMethod(b)));
    vm.types.set_attr(&t, "__new__", sm);
    vm.types.add_method(&t, "__init__", weakref_init);
    vm.types.add_method(&t, "__call__", weakref_call);
    vm.types.add_method(&t, "__hash__", weakref_hash);
    vm.types.add_method(&t, "__eq__", weakref_eq);
    vm.types.add_method(&t, "__ne__", weakref_ne);
    let get = vm.types.new_builtin("__callback__", weakref_callback);
    let prop = Value::Obj(new_obj(vm.types.property.clone(), None, Payload::Property { fget: get, fset: Value::None, fdel: Value::None, doc: Value::None }));
    vm.types.set_attr(&t, "__callback__", prop);
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_weakref", &[
        ("getweakrefcount", getweakrefcount),
        ("getweakrefs", getweakrefs),
        ("_new_proxy", new_proxy),
        ("_proxy_target", proxy_target),
        ("_proxy_repr", proxy_repr),
    ]);
    let r = Value::Obj(vm.types.weakref.clone());
    set(vm, &m, "ref", r.clone());
    set(vm, &m, "ReferenceType", r);
    let Payload::Module(_, ns) = &m.payload else { unreachable!() };
    let ns = ns.clone();
    let code = vm.compile_exec(PROXY_SRC, "<frozen _weakref>", crate::Dialect::Py3).map_err(|e| vm.syntax_error(e))?;
    vm.run_code(code, ns.clone(), None)?;
    vm.classes.proxy = ns.dict_get_str_raw("ProxyType").and_then(|v| v.obj().cloned());
    vm.classes.callable_proxy = ns.dict_get_str_raw("CallableProxyType").and_then(|v| v.obj().cloned());
    Ok(m)
}

fn arg<'a>(a: &'a [Value], k: Kw<'a>, i: usize, name: &str) -> Option<&'a Value> {
    a.get(i).or_else(|| k.iter().find(|(n, _)| &**n == name).map(|(_, v)| v))
}

/// CPython refuses weak references to instances of most builtin value types.
fn check_referent(vm: &Vm, v: &Value) -> PyResult<ObjRef> {
    let ok = match v {
        Value::Obj(o) => {
            ops::is_user_type(&o.class())
                || matches!(
                    o.payload,
                    Payload::Type(_) | Payload::Function(_) | Payload::Builtin(_) | Payload::Method { .. } | Payload::Set(_) | Payload::FrozenSet(_) | Payload::Generator(_) | Payload::Code(_) | Payload::Module(..) | Payload::Native(_)
                )
        }
        _ => false,
    };
    match v {
        Value::Obj(o) if ok => Ok(o.clone()),
        _ => Err(vm.type_error(format!("cannot create weak reference to '{}' object", ops::type_name(vm, v)))),
    }
}

/// Creates (or, without a callback, reuses) a weakref of class `cls`.
fn make_ref(vm: &Vm, cls: &ObjRef, target: &ObjRef, callback: Value) -> Value {
    let user = ops::is_user_type(cls);
    if callback.is_none() && (Rc::ptr_eq(cls, &vm.types.weakref) || is_proxy_class(vm, cls)) {
        for w in crate::gc::weakrefs_of(target) {
            if Rc::ptr_eq(&w.class(), cls) && matches!(&w.payload, Payload::WeakRef(d) if d.callback.borrow().is_none()) {
                return Value::Obj(w);
            }
        }
    }
    let dict = if user && !is_proxy_class(vm, cls) { Some(crate::dict::Dict::new()) } else { None };
    let data = WeakRefData { target: RefCell::new(Rc::downgrade(target)), callback: RefCell::new(callback), hash: Cell::new(None) };
    let w = new_obj(cls.clone(), dict, Payload::WeakRef(data));
    crate::gc::add_weakref(target, &w);
    Value::Obj(w)
}

fn is_proxy_class(vm: &Vm, cls: &ObjRef) -> bool {
    [&vm.classes.proxy, &vm.classes.callable_proxy].into_iter().flatten().any(|p| Rc::ptr_eq(p, cls))
}

fn weakref_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Some(Value::Obj(cls)) = a.first() else { return Err(vm.type_error("weakref.__new__(X): X is not a type object")) };
    let Some(ob) = arg(a, k, 1, "ob") else { return Err(vm.type_error("__new__ expected at least 1 argument, got 0")) };
    let target = check_referent(vm, ob)?;
    let cb = arg(a, k, 2, "callback").cloned().unwrap_or(Value::None);
    Ok(make_ref(vm, &cls.clone(), &target, cb))
}

fn weakref_init(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn weak_data<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a WeakRefData> {
    match v.payload() {
        Some(Payload::WeakRef(d)) => Ok(d),
        _ => Err(vm.type_error("descriptor requires a 'weakref' object")),
    }
}

fn weakref_call(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = weak_data(vm, &a[0])?;
    Ok(d.upgrade().map(Value::Obj).unwrap_or(Value::None))
}

/// Implemented here rather than through `ops::hash`, which would call a
/// subclass's Python `__hash__` that may delegate back to this method.
fn weakref_hash(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = weak_data(vm, &a[0])?;
    if let Some(h) = d.hash.get() {
        return Ok(Value::Int(h));
    }
    let Some(t) = d.upgrade() else { return Err(vm.type_error("weak object has gone away")) };
    let h = ops::hash(vm, &Value::Obj(t))?;
    d.hash.set(Some(h));
    Ok(Value::Int(h))
}

fn weakref_eq(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = weak_data(vm, &a[0])?;
    let Some(Payload::WeakRef(y)) = a.get(1).and_then(|v| v.payload()) else { return Ok(vm.types.not_implemented.clone()) };
    match (x.upgrade(), y.upgrade()) {
        (Some(p), Some(q)) => Ok(Value::Bool(ops::eq(vm, &Value::Obj(p), &Value::Obj(q))?)),
        _ => Ok(Value::Bool(a[0].is(&a[1]))),
    }
}

fn weakref_ne(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    match weakref_eq(vm, a, k)? {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        other => Ok(other),
    }
}

fn weakref_callback(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(weak_data(vm, &a[0])?.callback.borrow().clone())
}

fn getweakrefcount(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(match a.first() {
        Some(Value::Obj(o)) => crate::gc::weakrefs_of(o).len() as i64,
        _ => 0,
    }))
}

fn getweakrefs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let refs = match a.first() {
        Some(Value::Obj(o)) => crate::gc::weakrefs_of(o).into_iter().map(Value::Obj).collect(),
        _ => Vec::new(),
    };
    Ok(vm.types.new_list(refs))
}

fn new_proxy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Some(Value::Obj(cls)) = a.first() else { return Err(vm.type_error("proxy class expected")) };
    let target = check_referent(vm, a.get(1).unwrap_or(&Value::None))?;
    let cb = a.get(2).cloned().unwrap_or(Value::None);
    Ok(make_ref(vm, &cls.clone(), &target, cb))
}

fn proxy_target(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match a.first().and_then(|v| v.payload()) {
        Some(Payload::WeakRef(d)) => match d.upgrade() {
            Some(t) => Ok(Value::Obj(t)),
            None => Err(vm.exc_msg("ReferenceError", "weakly-referenced object no longer exists")),
        },
        _ => Err(vm.type_error("weakproxy expected")),
    }
}

fn proxy_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = &a[0];
    let name = p.obj().map(|o| o.class_name()).unwrap_or_default();
    let s = match p.payload() {
        Some(Payload::WeakRef(d)) => match d.upgrade() {
            Some(t) => format!("<{name} at 0x{:x}; to '{}' at 0x{:x}>", p.id(), t.class_name(), Value::Obj(t.clone()).id()),
            None => format!("<{name} at 0x{:x}; dead>", p.id()),
        },
        _ => format!("<{name}>"),
    };
    Ok(vm.types.new_string(s))
}
