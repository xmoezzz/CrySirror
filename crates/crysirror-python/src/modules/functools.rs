// SPDX-License-Identifier: MPL-2.0
//! `_functools`: `reduce`, `partial` and `cmp_to_key`. The Python-level
//! `functools` module (wraps, total_ordering, lru_cache…) is in `pysrc`.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::rc::Rc;

pub fn reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "reduce", a, 2, 3)?;
    let it = ops::iter(vm, &a[1])?;
    let mut acc = match a.get(2) {
        Some(v) => v.clone(),
        None => ops::next(vm, &it)?.ok_or_else(|| vm.type_error("reduce() of empty iterable with no initial value"))?,
    };
    while let Some(v) = ops::next(vm, &it)? {
        acc = vm.call(&a[0], vec![acc, v], Vec::new())?;
    }
    Ok(acc)
}

fn partial_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    if a.len() < 2 {
        return Err(vm.type_error("partial() requires a callable"));
    }
    let cls = a[0].obj().cloned().unwrap();
    let (mut func, mut args, kw) = (a[1].clone(), a[2..].to_vec(), vm.types.new_dict_obj(Dict::new()));
    // Flatten nested partials.
    let pc = vm.classes.partial.clone().unwrap();
    if let Value::Obj(fo) = &func {
        if Rc::ptr_eq(&fo.class(), &pc) {
            let inner_f = ops::get_raw_attr(fo, "func").unwrap();
            let inner_a = ops::get_raw_attr(fo, "args").unwrap();
            let inner_k = ops::get_raw_attr(fo, "keywords").unwrap();
            let mut na = ops::collect(vm, &inner_a)?;
            na.extend(args);
            args = na;
            for (kk, v) in ops::mapping_items(vm, &inner_k)? {
                ops::dict_set(vm, &kw, kk, v)?;
            }
            func = inner_f;
        }
    }
    for (kk, v) in k {
        let key = vm.types.new_str(kk);
        ops::dict_set(vm, &kw, key, v.clone())?;
    }
    let o = new_obj(cls, Some(Dict::new()), Payload::None);
    ops::set_raw_attr(vm, &o, "func", func);
    let t = vm.types.new_tuple(args);
    ops::set_raw_attr(vm, &o, "args", t);
    ops::set_raw_attr(vm, &o, "keywords", Value::Obj(kw));
    Ok(Value::Obj(o))
}

fn partial_call(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let func = ops::get_raw_attr(o, "func").unwrap_or(Value::None);
    let pa = ops::get_raw_attr(o, "args").unwrap_or(Value::None);
    let pk = ops::get_raw_attr(o, "keywords").unwrap_or(Value::None);
    let mut args = ops::collect(vm, &pa)?;
    args.extend(a[1..].iter().cloned());
    let mut kwargs: crate::vm::Kwargs = Vec::new();
    for (kk, v) in ops::mapping_items(vm, &pk)? {
        let ks: Rc<str> = Rc::from(kk.as_str().unwrap_or(""));
        if !k.iter().any(|(n, _)| *n == ks) {
            kwargs.push((ks, v));
        }
    }
    kwargs.extend(k.iter().cloned());
    vm.call(&func, args, kwargs)
}

fn partial_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let func = ops::get_raw_attr(o, "func").unwrap_or(Value::None);
    let pa = ops::get_raw_attr(o, "args").unwrap_or(Value::None);
    let mut parts = vec![ops::repr(vm, &func)?];
    for x in ops::collect(vm, &pa)? {
        parts.push(ops::repr(vm, &x)?);
    }
    let s = format!("functools.partial({})", parts.join(", "));
    Ok(vm.types.new_string(s))
}

fn partial_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    // partial objects are not descriptors: return self.
    let _ = vm;
    Ok(a[0].clone())
}

fn partial_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let cls = Value::Obj(o.class());
    let func = ops::get_raw_attr(o, "func").unwrap_or(Value::None);
    let pa = ops::get_raw_attr(o, "args").unwrap_or(Value::None);
    let pk = ops::get_raw_attr(o, "keywords").unwrap_or(Value::None);
    let fa = vm.types.new_tuple(vec![func.clone()]);
    let state = vm.types.new_tuple(vec![func, pa, pk, Value::None]);
    Ok(vm.types.new_tuple(vec![cls, fa, state]))
}

fn partial_setstate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let st = ops::collect(vm, &a[1])?;
    if st.len() < 3 {
        return Err(vm.type_error("invalid partial state"));
    }
    ops::set_raw_attr(vm, o, "func", st[0].clone());
    let args = if st[1].is_none() { vm.types.new_tuple(Vec::new()) } else { st[1].clone() };
    ops::set_raw_attr(vm, o, "args", args);
    let kw = if st[2].is_none() { vm.types.new_dict() } else { st[2].clone() };
    ops::set_raw_attr(vm, o, "keywords", kw);
    Ok(Value::None)
}

fn cmp_to_key(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "cmp_to_key", a, 1, 1)?;
    Ok(vm.types.new_builtin_data("K", make_key, a[0].clone()))
}

fn make_key(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = key_class(vm);
    let o = new_obj(cls, Some(Dict::new()), Payload::None);
    ops::set_raw_attr(vm, &o, "cmp", a[0].clone());
    ops::set_raw_attr(vm, &o, "obj", a[1].clone());
    Ok(Value::Obj(o))
}

fn key_class(vm: &mut Vm) -> ObjRef {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "_functools") {
        if let Some(Value::Obj(c)) = m.dict_get_str_raw("KeyWrapper") {
            return c;
        }
    }
    vm.types.object.clone()
}

macro_rules! key_cmp {
    ($($name:ident => $test:expr),*) => {
        $(
            fn $name(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                let (Value::Obj(x), Value::Obj(y)) = (&a[0], &a[1]) else { return Ok(vm.types.not_implemented.clone()) };
                let cmp = ops::get_raw_attr(x, "cmp").unwrap();
                let xo = ops::get_raw_attr(x, "obj").unwrap();
                let yo = ops::get_raw_attr(y, "obj").unwrap_or(Value::None);
                let r = vm.call(&cmp, vec![xo, yo], Vec::new())?;
                let n = ops::index_of(vm, &r)?;
                Ok(Value::Bool($test(n)))
            }
        )*
    };
}

key_cmp!(key_lt => |n: i64| n < 0, key_gt => |n: i64| n > 0, key_le => |n: i64| n <= 0, key_ge => |n: i64| n >= 0, key_eq => |n: i64| n == 0);

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_functools", &[("reduce", reduce), ("cmp_to_key", cmp_to_key)]);
    let pc = vm.types.new_builtin_type("partial", &vm.types.object.clone(), BaseKind::Object, false);
    let n = vm.types.new_builtin("__new__", partial_new);
    let n = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(n)));
    vm.types.set_attr(&pc, "__new__", n);
    vm.types.add_method(&pc, "__call__", partial_call);
    vm.types.add_method(&pc, "__repr__", partial_repr);
    vm.types.add_method(&pc, "__get__", partial_get);
    vm.types.add_method(&pc, "__reduce__", partial_reduce);
    vm.types.add_method(&pc, "__setstate__", partial_setstate);
    let mn = vm.types.new_str("functools");
    vm.types.set_attr(&pc, "__module__", mn);
    vm.classes.partial = Some(pc.clone());
    set(vm, &m, "partial", Value::Obj(pc));
    let kc = vm.types.new_builtin_type("KeyWrapper", &vm.types.object.clone(), BaseKind::Object, false);
    for (n, f) in [("__lt__", key_lt as NativeFn), ("__gt__", key_gt), ("__le__", key_le), ("__ge__", key_ge), ("__eq__", key_eq)] {
        vm.types.add_method(&kc, n, f);
    }
    set(vm, &m, "KeyWrapper", Value::Obj(kc));
    Ok(m)
}
