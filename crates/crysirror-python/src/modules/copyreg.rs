// SPDX-License-Identifier: MPL-2.0
//! `copyreg` and `object.__reduce_ex__`: the reduce protocol shared by
//! `copy` and `pickle` (protocol 2 layout for every protocol).

use super::{module_with, set};
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

fn newobj(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a.first().cloned().ok_or_else(|| vm.type_error("__newobj__ requires a class"))?;
    let new = ops::getattr(vm, &cls, "__new__")?;
    vm.call(&new, a.to_vec(), k.to_vec())
}

fn newobj_ex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "__newobj_ex__", a, 3, 3)?;
    let new = ops::getattr(vm, &a[0], "__new__")?;
    let mut args = vec![a[0].clone()];
    args.extend(ops::collect(vm, &a[1])?);
    let mut kw = Vec::new();
    for (key, v) in ops::mapping_items(vm, &a[2])? {
        kw.push((std::rc::Rc::from(key.as_str().unwrap_or("")), v));
    }
    vm.call(&new, args, kw)
}

fn reconstructor(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "_reconstructor", a, 3, 3)?;
    let (cls, base, state) = (&a[0], &a[1], &a[2]);
    let is_object = base.obj().map(|b| std::rc::Rc::ptr_eq(b, &vm.types.object)).unwrap_or(false);
    let obj = if is_object {
        crate::builtins::object_new(vm, &[cls.clone()], &[])?
    } else {
        let new = ops::getattr(vm, base, "__new__")?;
        let o = vm.call(&new, vec![cls.clone(), state.clone()], Vec::new())?;
        if let Some(init) = ops::lookup_type(base.obj().unwrap(), "__init__") {
            vm.call(&init, vec![o.clone(), state.clone()], Vec::new())?;
        }
        o
    };
    Ok(obj)
}

fn noop(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn copyreg_pickle(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "pickle", a, 2, 3)?;
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "copyreg") {
        if let Some(Value::Obj(dt)) = m.dict_get_str_raw("dispatch_table") {
            ops::dict_set(vm, &dt, a[0].clone(), a[1].clone())?;
        }
    }
    Ok(Value::None)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "copyreg") {
        return Ok(m);
    }
    let m = module_with(vm, "copyreg", &[
        ("__newobj__", newobj),
        ("__newobj_ex__", newobj_ex),
        ("_reconstructor", reconstructor),
        ("pickle", copyreg_pickle),
        ("constructor", noop),
        ("add_extension", noop),
        ("remove_extension", noop),
        ("clear_extension_cache", noop),
    ]);
    let dt = vm.types.new_dict();
    set(vm, &m, "dispatch_table", dt);
    let k = vm.types.new_str("copy_reg");
    vm.modules.dict_set_raw(&vm.types, k, Value::Obj(m.clone()));
    Ok(m)
}

fn copyreg_fn(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let m = crate::import::import_module(vm, "copyreg")?;
    ops::getattr(vm, &m, name)
}

/// `object.__reduce__`: the default reduction (never a user override).
pub fn object_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    default_reduce(vm, a[0].clone())
}

fn is_object_reduce(r: &Value) -> bool {
    matches!(r.payload(), Some(Payload::Builtin(b)) if b.func as usize == object_reduce as NativeFnPtr as usize)
}

type NativeFnPtr = fn(&mut Vm, &[Value], Kw) -> PyResult<Value>;

/// `object.__reduce_ex__(protocol)`: a `__reduce__` defined by the class
/// takes precedence over the default reduction.
pub fn object_reduce_ex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let obj = a[0].clone();
    let t = ops::type_of(vm, &obj);
    if let Some(r) = ops::lookup_user(&t, "__reduce__")
        && !is_object_reduce(&r)
    {
        return vm.call(&r, vec![obj], Vec::new());
    }
    default_reduce(vm, obj)
}

/// Default reduction: `(copyreg.__newobj__, (cls, *newargs), state,
/// listitems, dictitems)`.
fn default_reduce(vm: &mut Vm, obj: Value) -> PyResult<Value> {
    let t = ops::type_of(vm, &obj);
    let Value::Obj(o) = &obj else {
        return Err(vm.type_error(format!("cannot pickle '{}' object", ops::type_name(vm, &obj))));
    };
    match &o.payload {
        Payload::Function(_) | Payload::Builtin(_) | Payload::Generator(_) | Payload::Code(_) | Payload::Iter(_) | Payload::Cell(_) => {
            return Err(vm.type_error(format!("cannot pickle '{}' object", o.class_name())));
        }
        _ => {}
    }
    let newobj = copyreg_fn(vm, "__newobj__")?;
    let mut args = vec![Value::Obj(t.clone())];
    if let Some(gna) = ops::lookup_type(&t, "__getnewargs__") {
        let r = vm.call(&gna, vec![obj.clone()], Vec::new())?;
        args.extend(ops::collect(vm, &r)?);
    }
    let state = match ops::lookup_user(&t, "__getstate__") {
        Some(gs) => vm.call(&gs, vec![obj.clone()], Vec::new())?,
        None => match o.attr_table() {
            Some(d) if !d.borrow().is_empty() && !matches!(o.payload, Payload::Module(..)) => {
                let mut dd = d.borrow().clone();
                dd.compact();
                Value::Obj(vm.types.new_dict_obj(dd))
            }
            _ => Value::None,
        },
    };
    let listitems = match &o.payload {
        Payload::List(l) => {
            let items = l.borrow().clone();
            vm.types.new_iter(IterState::Values { items, idx: 0 })
        }
        _ => Value::None,
    };
    let dictitems = match &o.payload {
        Payload::Dict(d) => {
            let items = d.borrow().items().into_iter().map(|(k, v)| vm.types.new_tuple(vec![k, v])).collect();
            vm.types.new_iter(IterState::Values { items, idx: 0 })
        }
        _ => Value::None,
    };
    let at = vm.types.new_tuple(args);
    Ok(vm.types.new_tuple(vec![newobj, at, state, listitems, dictitems]))
}
