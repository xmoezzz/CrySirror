// SPDX-License-Identifier: MPL-2.0
//! `gc`: the script interface of the cycle collector (`crate::gc`).

use super::{module_with, set};
use crate::builtins::Kw;
use crate::gc;
use crate::object::*;
use crate::vm::{PyResult, Vm};

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "gc", &[
        ("enable", enable),
        ("disable", disable),
        ("isenabled", isenabled),
        ("collect", collect),
        ("get_count", get_count),
        ("get_threshold", get_threshold),
        ("set_threshold", set_threshold),
        ("get_debug", get_debug),
        ("set_debug", set_debug),
        ("get_objects", get_objects),
        ("get_stats", get_stats),
        ("get_referrers", get_referrers),
        ("get_referents", get_referents),
        ("is_tracked", is_tracked),
        ("is_finalized", is_finalized),
        ("freeze", freeze),
        ("unfreeze", unfreeze),
        ("get_freeze_count", get_freeze_count),
    ]);
    for (n, v) in [
        ("DEBUG_STATS", gc::DEBUG_STATS),
        ("DEBUG_COLLECTABLE", gc::DEBUG_COLLECTABLE),
        ("DEBUG_UNCOLLECTABLE", gc::DEBUG_UNCOLLECTABLE),
        ("DEBUG_SAVEALL", gc::DEBUG_SAVEALL),
        ("DEBUG_LEAK", gc::DEBUG_LEAK),
    ] {
        set(vm, &m, n, Value::Int(v as i64));
    }
    set(vm, &m, "garbage", Value::Obj(vm.gc_garbage.clone()));
    set(vm, &m, "callbacks", Value::Obj(vm.gc_callbacks.clone()));
    Ok(m)
}

fn int_arg(vm: &mut Vm, v: &Value, what: &str) -> PyResult<i64> {
    crate::ops::index_of(vm, v).map_err(|_| vm.type_error(format!("{what} must be an integer")))
}

fn kwarg<'a>(a: &'a [Value], k: Kw<'a>, i: usize, name: &str) -> Option<&'a Value> {
    a.get(i).or_else(|| k.iter().find(|(n, _)| &**n == name).map(|(_, v)| v))
}

fn generation(vm: &mut Vm, v: Option<&Value>) -> PyResult<Option<usize>> {
    match v {
        None | Some(Value::None) => Ok(None),
        Some(v) => {
            let g = int_arg(vm, v, "generation")?;
            if !(0..=2).contains(&g) {
                return Err(vm.value_error("invalid generation"));
            }
            Ok(Some(g as usize))
        }
    }
}

fn enable(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    gc::set_enabled(true);
    Ok(Value::None)
}

fn disable(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    gc::set_enabled(false);
    Ok(Value::None)
}

fn isenabled(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(gc::is_enabled()))
}

fn collect(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let g = generation(vm, kwarg(a, k, 0, "generation"))?.unwrap_or(2);
    let n = gc::collect(vm, g);
    // Finalizers and callbacks queued by the collection run before returning.
    gc::safe_point(vm);
    Ok(Value::Int(n as i64))
}

fn triple(vm: &Vm, t: [usize; 3]) -> Value {
    vm.types.new_tuple(t.iter().map(|v| Value::Int(*v as i64)).collect())
}

fn get_count(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(triple(vm, gc::count()))
}

fn get_threshold(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(triple(vm, gc::threshold()))
}

fn set_threshold(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if a.is_empty() || a.len() > 3 {
        return Err(vm.type_error(format!("set_threshold() takes 1 to 3 arguments ({} given)", a.len())));
    }
    let mut t = gc::threshold();
    for (i, v) in a.iter().enumerate() {
        t[i] = int_arg(vm, v, "threshold")?.max(0) as usize;
    }
    gc::set_threshold(&t);
    Ok(Value::None)
}

fn get_debug(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(gc::debug_flags() as i64))
}

fn set_debug(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = int_arg(vm, a.first().unwrap_or(&Value::None), "flags")?;
    gc::set_debug_flags(f as u32);
    Ok(Value::None)
}

fn get_objects(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let g = generation(vm, kwarg(a, k, 0, "generation"))?;
    let objs = gc::objects(g).into_iter().map(Value::Obj).collect();
    Ok(vm.types.new_list(objs))
}

fn get_stats(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    for s in gc::stats() {
        let d = vm.types.new_dict_obj(crate::dict::Dict::new());
        for (k, v) in [("collections", s.collections), ("collected", s.collected), ("uncollectable", s.uncollectable)] {
            let key = vm.types.intern(k);
            d.dict_set_raw(&vm.types, key, Value::Int(v as i64));
        }
        out.push(Value::Obj(d));
    }
    Ok(vm.types.new_list(out))
}

fn get_referrers(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let objs = gc::referrers(a).into_iter().map(Value::Obj).collect();
    Ok(vm.types.new_list(objs))
}

fn get_referents(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    for v in a {
        if let Value::Obj(o) = v {
            out.extend(gc::referents(o).into_iter().map(Value::Obj));
        }
    }
    Ok(vm.types.new_list(out))
}

fn is_tracked(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(a.first().is_some_and(gc::is_tracked)))
}

fn is_finalized(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(a.first().is_some_and(gc::is_finalized)))
}

fn freeze(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    gc::freeze();
    Ok(Value::None)
}

fn unfreeze(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    gc::unfreeze();
    Ok(Value::None)
}

fn get_freeze_count(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(gc::freeze_count() as i64))
}
