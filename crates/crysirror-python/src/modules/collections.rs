// SPDX-License-Identifier: MPL-2.0
//! `_collections`: `OrderedDict`, `defaultdict` and `deque` (dict/list
//! storage). `namedtuple`, `Counter` and the ABCs are Python source.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

fn static_new(vm: &Vm, t: &ObjRef, f: NativeFn) {
    let n = vm.types.new_builtin("__new__", f);
    let n = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(n)));
    vm.types.set_attr(t, "__new__", n);
}

fn dict_of<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a std::cell::RefCell<Dict>> {
    match v.payload() {
        Some(Payload::Dict(d)) => Ok(d),
        _ => Err(vm.type_error("dict expected")),
    }
}

// ---- OrderedDict ----

fn od_move_to_end(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [key, last] = crate::builtins::parse(vm, "move_to_end", &a[1..], k, ["key", "last"], 1)?;
    let last = match last {
        Some(v) => ops::truthy(vm, &v)?,
        None => true,
    };
    let key = key.unwrap();
    let h = ops::hash(vm, &key)?;
    let d = dict_of(vm, &a[0])?;
    let cands = d.borrow().candidates(h);
    for idx in cands {
        let k2 = d.borrow().entry(idx).map(|e| e.key.clone());
        if let Some(k2) = k2 {
            if ops::same_or_eq(vm, &k2, &key)? {
                d.borrow_mut().move_entry(idx, last);
                return Ok(Value::None);
            }
        }
    }
    Err(vm.key_error(key))
}

fn od_popitem(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [last] = crate::builtins::parse(vm, "popitem", &a[1..], k, ["last"], 0)?;
    let last = match last {
        Some(v) => ops::truthy(vm, &v)?,
        None => true,
    };
    let d = dict_of(vm, &a[0])?;
    let e = if last {
        d.borrow_mut().pop_last()
    } else {
        let idx = (0..d.borrow().slots()).find(|i| d.borrow().entry(*i).is_some());
        idx.and_then(|i| d.borrow_mut().remove_at(i))
    };
    match e {
        Some(e) => Ok(vm.types.new_tuple(vec![e.key, e.value])),
        None => Err(vm.exc_msg("KeyError", "dictionary is empty")),
    }
}

fn od_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = dict_of(vm, &a[0])?.borrow().items();
    let name = ops::type_name(vm, &a[0]);
    if items.is_empty() {
        return Ok(vm.types.new_string(format!("{name}()")));
    }
    let mut parts = Vec::new();
    for (k, v) in items {
        parts.push(format!("({}, {})", ops::repr(vm, &k)?, ops::repr(vm, &v)?));
    }
    Ok(vm.types.new_string(format!("{name}([{}])", parts.join(", "))))
}

fn od_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &a[0]));
    let items = dict_of(vm, &a[0])?.borrow().items();
    let items: Vec<Value> = items.into_iter().map(|(k, v)| vm.types.new_tuple(vec![k, v])).collect();
    let it = vm.types.new_iter(IterState::Values { items, idx: 0 });
    let empty = vm.types.new_tuple(Vec::new());
    let state = match a[0].obj().and_then(|o| o.dict.as_ref()) {
        Some(d) if !d.borrow().is_empty() => Value::Obj(vm.types.new_dict_obj(d.borrow().clone())),
        _ => Value::None,
    };
    Ok(vm.types.new_tuple(vec![cls, empty, state, Value::None, it]))
}

fn od_eq(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = dict_of(vm, &a[0])?.borrow().items();
    let od = vm.classes.ordereddict.clone().unwrap();
    if ops::isinstance(vm, &a[1], &od) {
        let y = dict_of(vm, &a[1])?.borrow().items();
        if x.len() != y.len() {
            return Ok(Value::Bool(false));
        }
        for ((k1, v1), (k2, v2)) in x.iter().zip(&y) {
            if !ops::eq(vm, k1, k2)? || !ops::eq(vm, v1, v2)? {
                return Ok(Value::Bool(false));
            }
        }
        return Ok(Value::Bool(true));
    }
    let eqf = vm.types.dict.dict.as_ref().unwrap().borrow().get_str("__eq__").cloned().unwrap();
    vm.call(&eqf, a.to_vec(), Vec::new())
}

// ---- defaultdict ----

fn dd_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let factory = a.get(1).cloned().unwrap_or(Value::None);
    if !factory.is_none() && !crate::builtins::is_callable(vm, &factory) {
        return Err(vm.type_error("first argument must be callable or None"));
    }
    ops::set_raw_attr(vm, o, "default_factory", factory);
    crate::methods::dict_update_from(vm, o, if a.len() > 2 { &a[2..3] } else { &[] }, k)?;
    Ok(Value::None)
}

fn dd_missing(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let f = ops::get_raw_attr(o, "default_factory").unwrap_or(Value::None);
    if f.is_none() {
        return Err(vm.key_error(a[1].clone()));
    }
    let v = vm.call0(&f)?;
    ops::dict_set(vm, o, a[1].clone(), v.clone())?;
    Ok(v)
}

fn dd_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let f = ops::get_raw_attr(o, "default_factory").unwrap_or(Value::None);
    let fr = ops::repr(vm, &f)?;
    let plain = Value::Obj(vm.types.new_dict_obj(dict_of(vm, &a[0])?.borrow().clone()));
    let dr = ops::repr(vm, &plain)?;
    Ok(vm.types.new_string(format!("defaultdict({fr}, {dr})")))
}

fn dd_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let cls = Value::Obj(o.class());
    let f = ops::get_raw_attr(o, "default_factory").unwrap_or(Value::None);
    let args = if f.is_none() { vm.types.new_tuple(Vec::new()) } else { vm.types.new_tuple(vec![f]) };
    let items = dict_of(vm, &a[0])?.borrow().items();
    let items: Vec<Value> = items.into_iter().map(|(k, v)| vm.types.new_tuple(vec![k, v])).collect();
    let it = vm.types.new_iter(IterState::Values { items, idx: 0 });
    Ok(vm.types.new_tuple(vec![cls, args, Value::None, Value::None, it]))
}

fn dd_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let f = ops::get_raw_attr(o, "default_factory").unwrap_or(Value::None);
    let cls = Value::Obj(o.class());
    let n = vm.call(&cls, vec![f], Vec::new())?;
    let items = dict_of(vm, &a[0])?.borrow().items();
    for (k, v) in items {
        ops::setitem(vm, &n, k, v)?;
    }
    Ok(n)
}

// ---- deque ----

fn dq_list<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a std::cell::RefCell<Vec<Value>>> {
    match v.payload() {
        Some(Payload::List(l)) => Ok(l),
        _ => Err(vm.type_error("deque expected")),
    }
}

fn dq_maxlen(v: &Value) -> Option<usize> {
    match v.obj().and_then(|o| ops::get_raw_attr(o, "maxlen")) {
        Some(Value::Int(n)) if n >= 0 => Some(n as usize),
        _ => None,
    }
}

fn dq_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [it, maxlen] = crate::builtins::parse(vm, "deque", &a[1..], k, ["iterable", "maxlen"], 0)?;
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let ml = match maxlen {
        Some(v) if !v.is_none() => {
            let n = ops::index_of(vm, &v)?;
            if n < 0 {
                return Err(vm.value_error("maxlen must be non-negative"));
            }
            Value::Int(n)
        }
        _ => Value::None,
    };
    ops::set_raw_attr(vm, o, "maxlen", ml);
    let mut items = match it {
        Some(v) if !v.is_none() => ops::collect(vm, &v)?,
        _ => Vec::new(),
    };
    if let Some(m) = dq_maxlen(&a[0]) {
        if items.len() > m {
            items.drain(..items.len() - m);
        }
    }
    *dq_list(vm, &a[0])?.borrow_mut() = items;
    Ok(Value::None)
}

fn dq_append(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let m = dq_maxlen(&a[0]);
    let l = dq_list(vm, &a[0])?;
    let mut b = l.borrow_mut();
    b.push(a[1].clone());
    if let Some(m) = m {
        while b.len() > m {
            b.remove(0);
        }
    }
    Ok(Value::None)
}

fn dq_appendleft(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let m = dq_maxlen(&a[0]);
    let l = dq_list(vm, &a[0])?;
    let mut b = l.borrow_mut();
    b.insert(0, a[1].clone());
    if let Some(m) = m {
        b.truncate(m);
    }
    Ok(Value::None)
}

fn dq_pop(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = dq_list(vm, &a[0])?.borrow_mut().pop();
    v.ok_or_else(|| vm.index_error("pop from an empty deque"))
}

fn dq_popleft(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let l = dq_list(vm, &a[0])?;
    if l.borrow().is_empty() {
        return Err(vm.index_error("pop from an empty deque"));
    }
    Ok(l.borrow_mut().remove(0))
}

fn dq_extend(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    for v in ops::collect(vm, &a[1])? {
        dq_append(vm, &[a[0].clone(), v], &[])?;
    }
    Ok(Value::None)
}

fn dq_extendleft(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    for v in ops::collect(vm, &a[1])? {
        dq_appendleft(vm, &[a[0].clone(), v], &[])?;
    }
    Ok(Value::None)
}

fn dq_rotate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = match a.get(1) {
        Some(v) => ops::index_of(vm, v)?,
        None => 1,
    };
    let l = dq_list(vm, &a[0])?;
    let mut b = l.borrow_mut();
    let len = b.len() as i64;
    if len > 0 {
        let k = n.rem_euclid(len) as usize;
        b.rotate_right(k);
    }
    Ok(Value::None)
}

fn dq_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = dq_list(vm, &a[0])?.borrow().clone();
    let l = vm.types.new_list(items);
    let r = ops::repr(vm, &l)?;
    let s = match dq_maxlen(&a[0]) {
        Some(m) => format!("deque({r}, maxlen={m})"),
        None => format!("deque({r})"),
    };
    Ok(vm.types.new_string(s))
}

fn dq_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &a[0]));
    let items = dq_list(vm, &a[0])?.borrow().clone();
    let l = vm.types.new_list(items);
    let ml = a[0].obj().and_then(|o| ops::get_raw_attr(o, "maxlen")).unwrap_or(Value::None);
    let args = vm.types.new_tuple(vec![l, ml]);
    Ok(vm.types.new_tuple(vec![cls, args]))
}

fn dq_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &a[0]));
    let items = dq_list(vm, &a[0])?.borrow().clone();
    let l = vm.types.new_list(items);
    let ml = a[0].obj().and_then(|o| ops::get_raw_attr(o, "maxlen")).unwrap_or(Value::None);
    vm.call(&cls, vec![l, ml], Vec::new())
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_collections", &[]);
    let dict = vm.types.dict.clone();
    let od = vm.types.new_builtin_type("OrderedDict", &dict, BaseKind::Dict, false);
    vm.types.add_method(&od, "move_to_end", od_move_to_end);
    vm.types.add_method(&od, "popitem", od_popitem);
    vm.types.add_method(&od, "__repr__", od_repr);
    vm.types.add_method(&od, "__reduce__", od_reduce);
    vm.types.add_method(&od, "__eq__", od_eq);
    let mn = vm.types.new_str("collections");
    vm.types.set_attr(&od, "__module__", mn.clone());
    vm.classes.ordereddict = Some(od.clone());
    set(vm, &m, "OrderedDict", Value::Obj(od));

    let dd = vm.types.new_builtin_type("defaultdict", &dict, BaseKind::Dict, false);
    vm.types.add_method(&dd, "__init__", dd_init);
    vm.types.add_method(&dd, "__missing__", dd_missing);
    vm.types.add_method(&dd, "__repr__", dd_repr);
    vm.types.add_method(&dd, "__reduce__", dd_reduce);
    vm.types.add_method(&dd, "copy", dd_copy);
    vm.types.add_method(&dd, "__copy__", dd_copy);
    vm.types.set_attr(&dd, "__module__", mn.clone());
    vm.classes.defaultdict = Some(dd.clone());
    set(vm, &m, "defaultdict", Value::Obj(dd));

    let list = vm.types.list.clone();
    let dq = vm.types.new_builtin_type("deque", &list, BaseKind::List, false);
    let lnew = vm.types.list.dict.as_ref().unwrap().borrow().get_str("__new__").cloned().unwrap();
    vm.types.set_attr(&dq, "__new__", lnew);
    for (n, f) in [
        ("__init__", dq_init as NativeFn), ("append", dq_append), ("appendleft", dq_appendleft), ("pop", dq_pop), ("popleft", dq_popleft),
        ("extend", dq_extend), ("extendleft", dq_extendleft), ("rotate", dq_rotate), ("__repr__", dq_repr), ("__reduce__", dq_reduce),
        ("copy", dq_copy), ("__copy__", dq_copy),
    ] {
        vm.types.add_method(&dq, n, f);
    }
    vm.types.set_attr(&dq, "__module__", mn);
    vm.classes.deque = Some(dq.clone());
    set(vm, &m, "deque", Value::Obj(dq));
    let _ = static_new;
    Ok(m)
}
