// SPDX-License-Identifier: MPL-2.0
//! `renpy.rollback`, after Ren'Py 8.3 `rollback.py` (MIT): the rollback
//! log saved in every original save file. `Rollback` entries hold a copy of
//! the context, store deltas and the compressed state of mutated
//! revertable objects; `RollbackLog` walks them back, fixes rollback and
//! freezes/unfreezes roots for save and load. Before 8.0 these classes
//! lived in `renpy.python`, which re-exports them.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::Cell, collections::HashSet};

const MODULE: &str = "renpy.rollback";

thread_local! {
    static SERIAL: Cell<i64> = const { Cell::new(0) };
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn g(vm: &mut Vm, o: &Value, n: &str) -> PyResult<Value> {
    py::get_or_none(vm, o, n)
}

fn gb(vm: &mut Vm, o: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, o, n)?;
    py::truthy(vm, &v)
}

fn gi(vm: &mut Vm, o: &Value, n: &str) -> PyResult<i64> {
    let v = py::get_or_none(vm, o, n)?;
    if py::is_none(&v) {
        return Ok(0);
    }
    py::int(vm, &v)
}

fn st(vm: &mut Vm, o: &Value, n: &str, v: Value) -> PyResult<()> {
    py::set(vm, o, n, v)
}

fn game(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", n)
}

fn context(vm: &mut Vm) -> PyResult<Value> {
    let f = game(vm, "context")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn log(vm: &mut Vm) -> PyResult<Value> {
    game(vm, "log")
}

fn store_dicts(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.python", "store_dicts")
}

fn has_label(vm: &mut Vm, name: &Value) -> PyResult<bool> {
    Ok(super::nodes::find(vm, name)?.is_some())
}

fn raise(vm: &mut Vm, module: &str, cls: &str) -> PyErr {
    match py::import_attr(vm, module, cls).and_then(|c| vm.call(&c, Vec::new(), Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error(cls),
        Err(e) => e,
    }
}

fn deleted(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "deleted")
}

/// `isinstance(v, classes)` for a class or tuple of classes.
fn isinstance_any(vm: &mut Vm, v: &Value, classes: &Value) -> PyResult<bool> {
    let list = py::seq(classes).unwrap_or_else(|| vec![classes.clone()]);
    Ok(list
        .iter()
        .any(|c| c.obj().is_some_and(|c| ops::isinstance(vm, v, c))))
}

// Markers ----------------------------------------------------------------------------------------------

fn sd_reduce(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::s(vm, "deleted"))
}

fn always_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cls = args[0].clone();
    let ro = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let new = py::get(vm, &ro, "__new__")?;
    let _ = kw;
    let this = vm.call(&new, vec![cls], Vec::new())?;
    let l = log(vm)?;
    if !py::is_none(&l) {
        let m = g(vm, &l, "mutated")?;
        if !py::is_none(&m) {
            vm.call_method(&m, "pop", vec![Value::Int(this.id()), Value::None])?;
        }
    }
    Ok(this)
}

// Reachability ---------------------------------------------------------------------------------------

fn norollback_types(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "NOROLLBACK_TYPES")
}

/// `reached(obj, reachable, wait)`: marks everything reachable from `obj`.
fn reached(
    vm: &mut Vm,
    obj: &Value,
    reachable: &Value,
    seen: &mut HashSet<i64>,
    types: &Value,
) -> PyResult<()> {
    let mut work = vec![obj.clone()];
    while let Some(obj) = work.pop() {
        let idobj = obj.id();
        if !seen.insert(idobj) {
            continue;
        }
        ops::setitem(vm, reachable, Value::Int(idobj), obj.clone())?;
        if !matches!(obj, Value::Obj(_)) {
            continue;
        }
        if isinstance_any(vm, &obj, types).unwrap_or(false) {
            continue;
        }
        let dict = py::namespace_of(vm, &obj).ok();
        if let Some(d) = dict {
            let nosave = py::get_opt(vm, &obj, "nosave").ok().flatten();
            let skip = match nosave {
                Some(ns) if !py::is_none(&ns) => {
                    let nr = py::get_opt(vm, &obj, "noreach")
                        .ok()
                        .flatten()
                        .unwrap_or(ns);
                    py::items(vm, &nr)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                        .collect::<Vec<_>>()
                }
                _ => Vec::new(),
            };
            if let Ok(items) = ops::mapping_items(vm, &d) {
                for (k, v) in items {
                    if k.as_str().is_some_and(|k| skip.iter().any(|s| s == k)) {
                        continue;
                    }
                    work.push(v);
                }
            }
        }
        if obj.as_str().is_some() || ops::bytes_of(&obj).is_some() {
            continue;
        }
        match obj.payload() {
            Some(crysirror_python::Payload::List(l)) => work.extend(l.borrow().iter().cloned()),
            Some(crysirror_python::Payload::Tuple(t)) => work.extend(t.iter().cloned()),
            Some(crysirror_python::Payload::Dict(_)) => {
                for (k, v) in ops::mapping_items(vm, &obj).unwrap_or_default() {
                    work.push(k);
                    work.push(v);
                }
            }
            Some(crysirror_python::Payload::Set(_) | crysirror_python::Payload::FrozenSet(_)) => {
                if let Ok(items) = ops::collect(vm, &obj) {
                    work.extend(items);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn f_reached(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let types = norollback_types(vm)?;
    let mut seen = HashSet::new();
    for (k, _) in ops::mapping_items(vm, &args[1])? {
        if let Value::Int(i) = k {
            seen.insert(i);
        }
    }
    reached(vm, &args[0], &args[1], &mut seen, &types)?;
    Ok(Value::None)
}

fn reached_vars(
    vm: &mut Vm,
    store: &Value,
    reachable: &Value,
    seen: &mut HashSet<i64>,
    types: &Value,
) -> PyResult<()> {
    let vals = vm.call_method(store, "values", Vec::new())?;
    for v in ops::collect(vm, &vals)? {
        reached(vm, &v, reachable, seen, types)?;
    }
    let contexts = game(vm, "contexts")?;
    for c in py::items(vm, &contexts)? {
        for n in ["info", "music"] {
            let v = g(vm, &c, n)?;
            reached(vm, &v, reachable, seen, types)?;
        }
        for d in py::get_items(vm, &c, "dynamic_stack")? {
            let vals = vm.call_method(&d, "values", Vec::new())?;
            for v in ops::collect(vm, &vals)? {
                reached(vm, &v, reachable, seen, types)?;
            }
        }
    }
    Ok(())
}

fn f_reached_vars(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let types = norollback_types(vm)?;
    let mut seen = HashSet::new();
    reached_vars(vm, &args[0], &args[1], &mut seen, &types)?;
    Ok(Value::None)
}

// Rollback ---------------------------------------------------------------------------------------------

fn rb_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ctx = context(vm)?;
    let copy = vm.call_method(&ctx, "rollback_copy", Vec::new())?;
    st(vm, this, "context", copy)?;
    st(vm, this, "objects", vm.types.new_list(Vec::new()))?;
    st(vm, this, "purged", Value::Bool(false))?;
    st(vm, this, "random", vm.types.new_list(Vec::new()))?;
    st(vm, this, "forward", Value::None)?;
    if super::state::fixed_forward(vm) {
        st(vm, this, "fixed", Value::Bool(false))?;
    }
    st(vm, this, "stores", vm.types.new_dict())?;
    st(vm, this, "delta_ebc", vm.types.new_dict())?;
    for n in [
        "retain_after_load",
        "checkpoint",
        "hard_checkpoint",
        "not_greedy",
    ] {
        st(vm, this, n, Value::Bool(false))?;
    }
    let l = log(vm)?;
    let cs = if py::is_none(&l) {
        Value::Bool(false)
    } else {
        g(vm, &l, "checkpointing_suspended")?
    };
    st(vm, this, "checkpointing_suspended", cs)?;
    let generation = mget(vm, "generation")?;
    let serial = SERIAL.with(|s| {
        let v = s.get();
        s.set(v + 1);
        v
    });
    let id = vm.types.new_tuple(vec![generation, Value::Int(serial)]);
    st(vm, this, "identifier", id)?;
    Ok(Value::None)
}

fn rb_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 2 {
        let stores = vm.types.new_dict();
        let main = vm.types.new_dict();
        let del = deleted(vm)?;
        for i in py::get_items(vm, this, "store")? {
            let p = py::items(vm, &i)?;
            match p.len() {
                2 => ops::setitem(vm, &main, p[0].clone(), p[1].clone())?,
                1 => ops::setitem(vm, &main, p[0].clone(), del.clone())?,
                _ => {}
            }
        }
        py::dict_set(vm, &stores, "store", main)?;
        st(vm, this, "stores", stores)?;
    }
    if version < 3 {
        st(vm, this, "retain_after_load", Value::Bool(false))?;
    }
    if version < 4 {
        let c = g(vm, this, "checkpoint")?;
        st(vm, this, "hard_checkpoint", c)?;
    }
    if version < 5 {
        st(vm, this, "delta_ebc", vm.types.new_dict())?;
    }
    Ok(Value::None)
}

fn rb_purge_unreachable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let reachable = args[1].clone();
    if gb(vm, this, "purged")? {
        return Ok(Value::Bool(false));
    }
    st(vm, this, "purged", Value::Bool(true))?;
    let types = norollback_types(vm)?;
    let mut seen: HashSet<i64> = HashSet::new();
    for (k, _) in ops::mapping_items(vm, &reachable)? {
        if let Value::Int(i) = k {
            seen.insert(i);
        }
    }
    let del = deleted(vm)?;
    let stores = g(vm, this, "stores")?;
    let vals = vm.call_method(&stores, "values", Vec::new())?;
    for changes in ops::collect(vm, &vals)? {
        for (_, v) in ops::mapping_items(vm, &changes)? {
            if !py::same(&v, &del) {
                reached(vm, &v, &reachable, &mut seen, &types)?;
            }
        }
    }
    let ctx = g(vm, this, "context")?;
    for n in ["info", "music", "movie", "modes"] {
        let v = g(vm, &ctx, n)?;
        reached(vm, &v, &reachable, &mut seen, &types)?;
    }
    for d in py::get_items(vm, &ctx, "dynamic_stack")? {
        let vals = vm.call_method(&d, "values", Vec::new())?;
        for v in ops::collect(vm, &vals)? {
            reached(vm, &v, &reachable, &mut seen, &types)?;
        }
    }
    let sl = g(vm, &ctx, "scene_lists")?;
    let all = vm.call_method(&sl, "get_all_displayables", Vec::new())?;
    reached(vm, &all, &reachable, &mut seen, &types)?;
    let objects = py::get_items(vm, this, "objects")?;
    let mut new_objects = Vec::new();
    let mut done: HashSet<i64> = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for pair in &objects {
            let p = py::items(vm, pair)?;
            let id_o = p[0].id();
            if done.contains(&id_o) || !ops::contains(vm, &reachable, &Value::Int(id_o))? {
                continue;
            }
            done.insert(id_o);
            if isinstance_any(vm, &p[0], &types).unwrap_or(false) {
                continue;
            }
            changed = true;
            new_objects.push(pair.clone());
            reached(vm, &p[1], &reachable, &mut seen, &types)?;
        }
    }
    let objs = g(vm, this, "objects")?;
    vm.call_method(&objs, "clear", Vec::new())?;
    vm.call_method(&objs, "extend", vec![vm.types.new_list(new_objects)])?;
    Ok(Value::Bool(true))
}

fn rb_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sds = store_dicts(vm)?;
    let objects = py::get_items(vm, this, "objects")?;
    for pair in objects.iter().rev() {
        let p = py::items(vm, pair)?;
        if py::is_none(&p[1]) {
            continue;
        }
        match vm.call_method(&p[0], "_rollback", vec![p[1].clone()]) {
            Ok(_) => {}
            Err(e) => {
                if py::get_opt(vm, &p[0], "_rollback")?.is_some() {
                    return Err(e);
                }
                let ex = py::config(vm, "ex_rollback_classes")?;
                let ex = py::items(vm, &ex)?;
                let ex = vm.types.new_tuple(ex);
                if isinstance_any(vm, &p[0], &ex).unwrap_or(false)
                    || !py::config_bool(vm, "developer")?
                {
                    continue;
                }
                let t = ops::type_name(vm, &p[0]);
                return Err(py::error(
                    vm,
                    format!(
                        "Load or rollback failed because class {t} does not inherit from store.object, but did in the past. If this was an intentional change, add the class to config.ex_rollback_classes."
                    ),
                ));
            }
        }
    }
    let del = deleted(vm)?;
    let stores = g(vm, this, "stores")?;
    for (name, changes) in ops::mapping_items(vm, &stores)? {
        let store = vm.call_method(&sds, "get", vec![name, Value::None])?;
        if py::is_none(&store) {
            continue;
        }
        for (k, v) in ops::mapping_items(vm, &changes)? {
            if py::same(&v, &del) {
                vm.call_method(&store, "pop", vec![k, Value::None])?;
            } else {
                ops::setitem(vm, &store, k, v)?;
            }
        }
    }
    let debc = g(vm, this, "delta_ebc")?;
    for (name, changes) in ops::mapping_items(vm, &debc)? {
        let store = vm.call_method(&sds, "get", vec![name, Value::None])?;
        if py::is_none(&store) {
            continue;
        }
        let ebc = g(vm, &store, "ever_been_changed")?;
        for k in ops::collect(vm, &changes)? {
            ops::set_discard(vm, &ebc, &k)?;
        }
    }
    let rng = mget(vm, "rng")?;
    let r = g(vm, this, "random")?;
    vm.call_method(&rng, "pushback", vec![r])?;
    vm.call_method(this, "rollback_control", Vec::new())?;
    Ok(Value::None)
}

fn rb_rollback_control(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let contexts = game(vm, "contexts")?;
    let mut l = py::items(vm, &contexts)?;
    l.pop();
    l.push(g(vm, this, "context")?);
    let gm = py::import(vm, "renpy.game")?;
    py::set(vm, &gm, "contexts", vm.types.new_list(l))?;
    let lg = log(vm)?;
    let cs = g(vm, this, "checkpointing_suspended")?;
    st(vm, &lg, "checkpointing_suspended", cs)?;
    Ok(Value::None)
}

// RollbackLog ------------------------------------------------------------------------------------------

fn new_rollback(vm: &mut Vm) -> PyResult<Value> {
    let c = mget(vm, "Rollback")?;
    vm.call(&c, Vec::new(), Vec::new())
}

fn rl_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    st(vm, this, "log", vm.types.new_list(Vec::new()))?;
    st(vm, this, "current", Value::None)?;
    st(vm, this, "mutated", vm.types.new_dict())?;
    st(vm, this, "rollback_limit", Value::Int(0))?;
    st(vm, this, "rollback_block", Value::Int(0))?;
    let fixed_forward = super::state::fixed_forward(vm);
    if !fixed_forward {
        st(vm, this, "rollback_is_fixed", Value::Bool(false))?;
        st(vm, this, "fixed_rollback_boundary", Value::None)?;
    }
    for n in [
        "checkpointing_suspended",
        "rolled_forward",
        "retain_after_load_flag",
        "force_checkpoint",
    ] {
        st(vm, this, n, Value::Bool(false))?;
    }
    st(vm, this, "forward", vm.types.new_list(Vec::new()))?;
    st(vm, this, "old_store", vm.types.new_dict())?;
    let rng = mget(vm, "rng")?;
    vm.call_method(&rng, "reset", Vec::new())?;
    st(vm, this, "did_interaction", Value::Bool(true))?;
    Ok(Value::None)
}

fn rl_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "mutated", vm.types.new_dict())?;
    st(vm, &args[0], "rolled_forward", Value::Bool(false))?;
    Ok(Value::None)
}

fn rl_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 2 {
        let ebc = g(vm, this, "ever_been_changed")?;
        let d = vm.types.new_dict();
        let set = Value::Obj(vm.types.set.clone());
        let s = vm.call(&set, vec![ebc], Vec::new())?;
        py::dict_set(vm, &d, "store", s)?;
        st(vm, this, "ever_been_changed", d)?;
    }
    let fixed_forward = super::state::fixed_forward(vm);
    if version < 3 && !fixed_forward {
        st(vm, this, "rollback_is_fixed", Value::Bool(false))?;
        st(vm, this, "fixed_rollback_boundary", Value::None)?;
    }
    if version < 4 {
        st(vm, this, "retain_after_load_flag", Value::Bool(false))?;
    }
    let entries = py::get_items(vm, this, "log")?;
    if version < 5 {
        let limit = gi(vm, this, "rollback_limit")?;
        if limit > 0 {
            let start = entries.len().saturating_sub(limit as usize);
            let mut n = 0;
            for rb in &entries[start..] {
                if gb(vm, rb, "hard_checkpoint")? {
                    n += 1;
                }
            }
            st(vm, this, "rollback_limit", Value::Int(n))?;
        }
    }
    if version < 6 {
        let mut hard = 0;
        for e in &entries {
            if gb(vm, e, "hard_checkpoint")? {
                hard += 1;
            }
        }
        let limit = gi(vm, this, "rollback_limit")?;
        let block = (hard - limit).max(0);
        st(vm, this, "rollback_block", Value::Int(block))?;
        st(vm, this, "rollback_limit", Value::Int(hard - block))?;
    }
    if version < 7 && fixed_forward {
        let fc = mget(vm, "Forward")?;
        let mut out = Vec::new();
        for pair in py::get_items(vm, this, "forward")? {
            let p = py::items(vm, &pair)?;
            out.push(vm.call(&fc, vec![p[0].clone(), p[1].clone(), Value::Bool(false)], Vec::new())?);
        }
        st(vm, this, "forward", vm.types.new_list(out))?;
    }
    Ok(Value::None)
}

fn is_str(v: &Value) -> bool {
    v.as_str().is_some()
}

fn rl_begin(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let force = match arg(args, kw, 1, "force") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    st(vm, this, "identifier_cache", Value::None)?;
    let ctx = context(vm)?;
    if !gb(vm, &ctx, "rollback")? {
        return Ok(Value::None);
    }
    let current = g(vm, this, "current")?;
    let ignore = if force || gb(vm, this, "did_interaction")? {
        false
    } else if !py::is_none(&current) {
        if gb(vm, &current, "checkpoint")? || gb(vm, &current, "retain_after_load")? {
            false
        } else {
            let cc = g(vm, &ctx, "current")?;
            let rc = g(vm, &current, "context")?;
            let rcc = g(vm, &rc, "current")?;
            !(is_str(&cc) && !is_str(&rcc))
        }
    } else {
        false
    };
    if ignore {
        return Ok(Value::None);
    }
    st(vm, this, "did_interaction", Value::Bool(false))?;
    if !py::is_none(&current) {
        rl_complete(vm, &[this.clone(), Value::Bool(true)], &[])?;
    } else {
        let f = py::import_attr(vm, "renpy.python", "begin_stores")?;
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    let lv = g(vm, this, "log")?;
    let rl = py::config(vm, "rollback_length")?;
    let rl = py::int(vm, &rl)?;
    while ops::len(vm, &lv)? as i64 > rl {
        let e = vm.call_method(&lv, "pop", vec![Value::Int(0)])?;
        if gb(vm, &e, "hard_checkpoint")? {
            let b = gi(vm, this, "rollback_block")?;
            if b > 0 {
                st(vm, this, "rollback_block", Value::Int(b - 1))?;
            } else {
                let l = gi(vm, this, "rollback_limit")?;
                st(vm, this, "rollback_limit", Value::Int(l - 1))?;
            }
        }
    }
    let n = ops::len(vm, &lv)?;
    if n >= 2 && !super::state::fixed_forward(vm) {
        let e = ops::getitem(vm, &lv, Value::Int(-2))?;
        let ec = g(vm, &e, "context")?;
        let ecc = g(vm, &ec, "current")?;
        let frb = g(vm, this, "fixed_rollback_boundary")?;
        if ops::eq(vm, &ecc, &frb)? {
            st(vm, this, "rollback_is_fixed", Value::Bool(false))?;
        }
    }
    let fwd = g(vm, this, "forward")?;
    if !super::state::fixed_forward(vm) && gb(vm, this, "rollback_is_fixed")? && !py::truthy(vm, &fwd)? {
        let cur = g(vm, this, "current")?;
        let cc = g(vm, &cur, "context")?;
        let ccc = g(vm, &cc, "current")?;
        st(vm, this, "fixed_rollback_boundary", ccc)?;
        st(vm, this, "rollback_is_fixed", Value::Bool(false))?;
    }
    let rb = new_rollback(vm)?;
    let ral = g(vm, this, "retain_after_load_flag")?;
    st(vm, &rb, "retain_after_load", ral)?;
    st(vm, this, "current", rb.clone())?;
    ops::list_append(vm, &lv, rb)?;
    let m = g(vm, this, "mutated")?;
    vm.call_method(&m, "clear", Vec::new())?;
    let rev = py::import(vm, "renpy.revertable")?;
    py::set(vm, &rev, "mutate_flag", Value::Bool(true))?;
    st(vm, this, "rolled_forward", Value::Bool(false))?;
    Ok(Value::None)
}

fn rl_replace_node(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for i in py::get_items(vm, &args[0], "log")? {
        let c = g(vm, &i, "context")?;
        vm.call_method(&c, "replace_node", args[1..3].to_vec())?;
    }
    Ok(Value::None)
}

fn rl_complete(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let begin = match arg(args, kw, 1, "begin") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    if gb(vm, this, "force_checkpoint")? {
        py::method_kw(
            vm,
            this,
            "checkpoint",
            Vec::new(),
            vec![("hard".into(), Value::Bool(false))],
        )?;
        st(vm, this, "force_checkpoint", Value::Bool(false))?;
    }
    let current = g(vm, this, "current")?;
    let stores = g(vm, &current, "stores")?;
    let debc = g(vm, &current, "delta_ebc")?;
    let sds = store_dicts(vm)?;
    for (name, sd) in ops::mapping_items(vm, &sds)? {
        let prev = vm.call_method(&stores, "get", vec![name.clone(), Value::None])?;
        let delta = vm.call_method(&sd, "get_changes", vec![Value::Bool(begin), prev])?;
        if py::truthy(vm, &delta)? {
            let p = py::items(vm, &delta)?;
            ops::setitem(vm, &stores, name.clone(), p[0].clone())?;
            ops::setitem(vm, &debc, name, p[1].clone())?;
        }
    }
    let objects = g(vm, &current, "objects")?;
    vm.call_method(&objects, "clear", Vec::new())?;
    let mutated = g(vm, this, "mutated")?;
    for (_, v) in ops::mapping_items(vm, &mutated)? {
        if py::is_none(&v) {
            continue;
        }
        let p = py::items(vm, &v)?;
        let obj = vm.call(&p[0], Vec::new(), Vec::new())?;
        if py::is_none(&obj) {
            continue;
        }
        let compressed = vm.call_method(&obj, "_compress", vec![p[1].clone()])?;
        let t = vm.types.new_tuple(vec![obj, compressed]);
        ops::list_append(vm, &objects, t)?;
    }
    Ok(Value::None)
}

fn rl_get_roots(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rv = vm.types.new_dict();
    let del = deleted(vm)?;
    let sds = store_dicts(vm)?;
    for (sname, sd) in ops::mapping_items(vm, &sds)? {
        let sname = py::string(vm, &sname)?;
        let ebc = g(vm, &sd, "ever_been_changed")?;
        for name in ops::collect(vm, &ebc)? {
            let n = py::string(vm, &name)?;
            let v = if ops::contains(vm, &sd, &name)? {
                ops::getitem(vm, &sd, name)?
            } else {
                del.clone()
            };
            py::dict_set(vm, &rv, &format!("{sname}.{n}"), v)?;
        }
    }
    let contexts = game(vm, "contexts")?;
    let cs = py::items(vm, &contexts)?;
    for c in cs.iter().skip(1).rev() {
        vm.call_method(c, "pop_dynamic_roots", vec![rv.clone()])?;
    }
    let _ = args;
    Ok(rv)
}

fn rl_purge_unreachable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let roots = args[1].clone();
    let store_module = py::import_attr(vm, "renpy.python", "StoreModule")?;
    let snr = mget(vm, "SlottedNoRollback")?;
    let types = vm.types.new_tuple(vec![
        Value::Obj(vm.types.module.clone()),
        store_module,
        snr,
        Value::Obj(vm.types.type_.clone()),
    ]);
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, "NOROLLBACK_TYPES", types.clone())?;
    let reachable = vm.types.new_dict();
    let mut seen = HashSet::new();
    reached_vars(vm, &roots, &reachable, &mut seen, &types)?;
    let entries = py::get_items(vm, this, "log")?;
    for i in entries.iter().rev() {
        let r = vm.call_method(i, "purge_unreachable", vec![reachable.clone(), Value::None])?;
        if !py::truthy(vm, &r)? {
            break;
        }
    }
    Ok(Value::None)
}

fn rl_in_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = g(vm, &args[0], "forward")?;
    Ok(Value::Bool(py::truthy(vm, &f)?))
}

fn rl_in_fixed_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !super::state::fixed_forward(vm) {
        return g(vm, &args[0], "rollback_is_fixed");
    }
    // `self.forward and self.forward[0].fixed`
    let f = g(vm, &args[0], "forward")?;
    if !py::truthy(vm, &f)? {
        return Ok(f);
    }
    let first = ops::getitem(vm, &f, Value::Int(0))?;
    g(vm, &first, "fixed")
}

fn in_fixed_rollback(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let v = rl_in_fixed_rollback(vm, std::slice::from_ref(this), &[])?;
    py::truthy(vm, &v)
}

/// A forward entry's (name, data): a `Forward` from 8.4, a pair before.
fn forward_entry(vm: &mut Vm, e: &Value) -> PyResult<(Value, Value)> {
    if super::state::fixed_forward(vm) {
        Ok((g(vm, e, "name")?, g(vm, e, "data")?))
    } else {
        let p = py::items(vm, e)?;
        Ok((p[0].clone(), p[1].clone()))
    }
}

fn current_name(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let cur = g(vm, this, "current")?;
    let c = g(vm, &cur, "context")?;
    g(vm, &c, "current")
}

fn rl_forward_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = g(vm, this, "forward")?;
    if py::truthy(vm, &f)? {
        let first = ops::getitem(vm, &f, Value::Int(0))?;
        let (name, data) = forward_entry(vm, &first)?;
        let cn = current_name(vm, this)?;
        if ops::eq(vm, &cn, &name)? {
            return Ok(data);
        }
    }
    Ok(Value::None)
}

fn rl_checkpoint(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let data = arg_or(args, kw, 1, "data", Value::None);
    let keep_rollback = match arg(args, kw, 2, "keep_rollback") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let mut hard = arg_or(args, kw, 3, "hard", Value::Bool(true));
    let current = g(vm, this, "current")?;
    if gb(vm, this, "checkpointing_suspended")? {
        hard = Value::Bool(false);
        st(vm, &current, "not_greedy", Value::Bool(true))?;
    }
    let hard_t = py::truthy(vm, &hard)?;
    if hard_t {
        st(vm, this, "retain_after_load_flag", Value::Bool(false))?;
    }
    let ctx = context(vm)?;
    if !gb(vm, &ctx, "rollback")? {
        return Ok(Value::None);
    }
    st(vm, &current, "checkpoint", Value::Bool(true))?;
    if hard_t && !gb(vm, &current, "hard_checkpoint")? {
        let limit = gi(vm, this, "rollback_limit")?;
        let hrl = py::config(vm, "hard_rollback_limit")?;
        let hrl = py::int(vm, &hrl)?;
        if limit < hrl {
            st(vm, this, "rollback_limit", Value::Int(limit + 1))?;
        } else {
            let b = gi(vm, this, "rollback_block")?;
            st(vm, this, "rollback_block", Value::Int(b + 1))?;
        }
        if hard.as_str() == Some("not_greedy") {
            st(vm, &current, "not_greedy", Value::Bool(true))?;
        } else {
            st(vm, &current, "hard_checkpoint", hard.clone())?;
        }
    }
    let fwd = g(vm, this, "forward")?;
    let has_fwd = py::truthy(vm, &fwd)?;
    let fixed_forward = super::state::fixed_forward(vm);
    if in_fixed_rollback(vm, this)? && has_fwd {
        let first = ops::getitem(vm, &fwd, Value::Int(0))?;
        let (name, fdata) = forward_entry(vm, &first)?;
        let cn = current_name(vm, this)?;
        if ops::eq(vm, &cn, &name)? {
            st(vm, &current, "forward", fdata)?;
            if fixed_forward {
                let fixed = g(vm, &first, "fixed")?;
                st(vm, &current, "fixed", fixed)?;
            }
            vm.call_method(&fwd, "pop", vec![Value::Int(0)])?;
        } else if !fixed_forward {
            // 8.4+ tests `not self.forward` here, which cannot hold.
            st(vm, &current, "forward", data)?;
            vm.call_method(&fwd, "clear", Vec::new())?;
        }
    } else if !py::is_none(&data) {
        if has_fwd {
            let first = ops::getitem(vm, &fwd, Value::Int(0))?;
            let (name, fdata) = forward_entry(vm, &first)?;
            let cn = current_name(vm, this)?;
            let rf = gb(vm, this, "rolled_forward")?;
            if ops::eq(vm, &cn, &name)? && ops::eq(vm, &data, &fdata)? && (keep_rollback || rf) {
                vm.call_method(&fwd, "pop", vec![Value::Int(0)])?;
            } else {
                vm.call_method(&fwd, "clear", Vec::new())?;
            }
        }
        st(vm, &current, "forward", data)?;
    }
    Ok(Value::None)
}

fn rl_suspend_checkpointing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    st(vm, this, "checkpointing_suspended", args[1].clone())?;
    let cur = g(vm, this, "current")?;
    st(vm, &cur, "not_greedy", Value::Bool(true))?;
    let contexts = game(vm, "contexts")?;
    let c0 = ops::getitem(vm, &contexts, Value::Int(0))?;
    st(vm, &c0, "force_checkpoint", Value::Bool(true))?;
    Ok(Value::None)
}

fn rl_block(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let purge = match arg(args, kw, 1, "purge") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let b = gi(vm, this, "rollback_block")?;
    let l = gi(vm, this, "rollback_limit")?;
    st(vm, this, "rollback_block", Value::Int(b + l))?;
    st(vm, this, "rollback_limit", Value::Int(0))?;
    let cur = g(vm, this, "current")?;
    if !py::is_none(&cur) {
        st(vm, &cur, "not_greedy", Value::Bool(true))?;
    }
    let ctx = context(vm)?;
    st(vm, &ctx, "force_checkpoint", Value::Bool(true))?;
    if purge {
        st(vm, this, "rollback_block", Value::Int(0))?;
        let lv = g(vm, this, "log")?;
        vm.call_method(&lv, "clear", Vec::new())?;
    }
    Ok(Value::None)
}

fn rl_retain_after_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let p = py::import_attr(vm, "renpy.display.predict", "predicting")?;
    if py::truthy(vm, &p)? {
        return Ok(Value::None);
    }
    st(vm, this, "retain_after_load_flag", Value::Bool(true))?;
    let cur = g(vm, this, "current")?;
    st(vm, &cur, "retain_after_load", Value::Bool(true))?;
    for rb in py::get_items(vm, this, "log")?.iter().rev() {
        if gb(vm, rb, "hard_checkpoint")? {
            break;
        }
        st(vm, rb, "retain_after_load", Value::Bool(true))?;
    }
    let ctx = context(vm)?;
    st(vm, &ctx, "force_checkpoint", Value::Bool(true))?;
    Ok(Value::None)
}

fn rl_fix_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let lv = g(vm, this, "log")?;
    if super::state::fixed_forward(vm) {
        for rb in py::items(vm, &lv)? {
            st(vm, &rb, "fixed", Value::Bool(true))?;
        }
    } else if !gb(vm, this, "rollback_is_fixed")? && ops::len(vm, &lv)? > 1 {
        let e = ops::getitem(vm, &lv, Value::Int(-2))?;
        let c = g(vm, &e, "context")?;
        let cc = g(vm, &c, "current")?;
        st(vm, this, "fixed_rollback_boundary", cc)?;
    }
    let ctx = context(vm)?;
    st(vm, &ctx, "force_checkpoint", Value::Bool(true))?;
    Ok(Value::None)
}

fn rl_can_rollback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if super::state::deferred_rollback(vm) {
        let checkpoints = py::int(vm, &arg_or(args, kw, 1, "checkpoints", Value::Int(1)))?;
        let force = py::truthy(vm, &arg_or(args, kw, 2, "force", Value::Bool(false)))?;
        return Ok(Value::Bool(can_rollback_to(vm, &args[0], checkpoints, force)?));
    }
    Ok(Value::Bool(gi(vm, &args[0], "rollback_limit")? > 0))
}

fn rl_load_failed(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut lfl = py::config(vm, "load_failed_label")?;
    if crysirror_python::builtins::is_callable(vm, &lfl) {
        lfl = vm.call(&lfl, Vec::new(), Vec::new())?;
    }
    if !py::truthy(vm, &lfl)? {
        return Err(py::error(
            vm,
            "Couldn't find a place to stop rolling back. Perhaps the script changed in an incompatible way?",
        ));
    }
    let lv = g(vm, this, "log")?;
    let rb = vm.call_method(&lv, "pop", Vec::new())?;
    vm.call_method(&rb, "rollback", Vec::new())?;
    let depth = py::import_attr(vm, "renpy.exports", "call_stack_depth")?;
    let pop = py::import_attr(vm, "renpy.exports", "pop_call")?;
    loop {
        let d = vm.call(&depth, Vec::new(), Vec::new())?;
        if !py::truthy(vm, &d)? {
            break;
        }
        vm.call(&pop, Vec::new(), Vec::new())?;
    }
    let contexts = game(vm, "contexts")?;
    let c0 = ops::getitem(vm, &contexts, Value::Int(0))?;
    st(vm, &c0, "force_checkpoint", Value::Bool(true))?;
    vm.call_method(&c0, "goto_label", vec![lfl])?;
    let module = super::state::restart_module(vm);
    Err(raise(vm, module, "RestartTopContext"))
}

struct RollbackArgs {
    checkpoints: i64,
    force: bool,
    label: Value,
    greedy: bool,
    on_load: bool,
    abnormal: bool,
    current_label: Value,
}

fn rollback_args(vm: &mut Vm, args: &[Value], kw: Kw, with_force: bool) -> PyResult<RollbackArgs> {
    let flag = |vm: &mut Vm, i: usize, n: &str, d: bool| -> PyResult<bool> {
        match arg(args, kw, i, n) {
            Some(v) => py::truthy(vm, v),
            None => Ok(d),
        }
    };
    // `rollback_core` has no `force` parameter.
    let o = usize::from(!with_force);
    Ok(RollbackArgs {
        checkpoints: py::int(vm, &arg_or(args, kw, 1, "checkpoints", Value::Int(0)))?,
        force: if with_force { flag(vm, 2, "force", false)? } else { false },
        label: arg_or(args, kw, 3 - o, "label", Value::None),
        greedy: flag(vm, 4 - o, "greedy", true)?,
        on_load: flag(vm, 5 - o, "on_load", false)?,
        abnormal: flag(vm, 6 - o, "abnormal", true)?,
        current_label: arg_or(args, kw, 7 - o, "current_label", Value::None),
    })
}

/// 8.5.1+ `can_rollback(checkpoints=1, force=False)`: whether a labelled
/// statement lies `checkpoints` hard checkpoints back.
fn can_rollback_to(vm: &mut Vm, this: &Value, mut checkpoints: i64, force: bool) -> PyResult<bool> {
    if checkpoints != 0 && gi(vm, this, "rollback_limit")? <= 0 && !force {
        return Ok(false);
    }
    for rb in py::get_items(vm, this, "log")?.iter().rev() {
        if gb(vm, rb, "hard_checkpoint")? {
            checkpoints -= 1;
        }
        if checkpoints <= 0 {
            let c = g(vm, rb, "context")?;
            let cc = g(vm, &c, "current")?;
            if has_label(vm, &cc)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// `rollback(...)`. From 8.5.1 it only checks and raises
/// `RollbackException`, which `run_context` performs via `rollback_core`.
fn rl_rollback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = rollback_args(vm, args, kw, true)?;
    let this = &args[0];
    if super::state::deferred_rollback(vm) {
        if !can_rollback_to(vm, this, a.checkpoints, a.force)? {
            return Ok(Value::None);
        }
        let c = mget(vm, "RollbackException")?;
        let e = vm.call(
            &c,
            vec![
                Value::Int(a.checkpoints),
                a.label,
                Value::Bool(a.greedy),
                Value::Bool(a.on_load),
                Value::Bool(a.abnormal),
                a.current_label,
            ],
            Vec::new(),
        )?;
        return Err(PyErr::Exc(e.obj().cloned().ok_or_else(|| vm.type_error("RollbackException"))?));
    }
    if a.checkpoints != 0 && gi(vm, this, "rollback_limit")? <= 0 && !a.force {
        return Ok(Value::None);
    }
    rollback_impl(vm, this, a, false)
}

fn rl_rollback_core(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = rollback_args(vm, args, kw, false)?;
    if !a.on_load {
        vm.call_method(&args[0], "complete", vec![Value::Bool(false)])?;
    }
    rollback_impl(vm, &args[0], a, true)
}

/// The rollback itself: `rollback` before 8.5.1, `rollback_core` after
/// (`core`), which keeps nested contexts, keys the load paths on
/// `on_load`, and returns instead of raising a restart.
fn rollback_impl(vm: &mut Vm, this: &Value, a: RollbackArgs, core: bool) -> PyResult<Value> {
    let RollbackArgs { mut checkpoints, force, mut label, greedy, on_load, abnormal, current_label } = a;
    let fixed_forward = super::state::fixed_forward(vm);
    let roots = vm.call_method(this, "get_roots", Vec::new())?;
    vm.call_method(this, "purge_unreachable", vec![roots])?;
    let lv = g(vm, this, "log")?;
    let mut revlog: Vec<Value> = Vec::new();
    let mut found = false;
    while ops::len(vm, &lv)? > 0 {
        let rb = vm.call_method(&lv, "pop", Vec::new())?;
        revlog.push(rb.clone());
        let hard = gb(vm, &rb, "hard_checkpoint")?;
        if hard {
            let l = gi(vm, this, "rollback_limit")?;
            if l > 0 {
                st(vm, this, "rollback_limit", Value::Int(l - 1))?;
            } else {
                let b = gi(vm, this, "rollback_block")?;
                if b > 0 {
                    st(vm, this, "rollback_block", Value::Int(b - 1))?;
                }
            }
        }
        if hard || (on_load && gb(vm, &rb, "checkpoint")?) {
            checkpoints -= 1;
        }
        if checkpoints <= 0 {
            let c = g(vm, &rb, "context")?;
            let cc = g(vm, &c, "current")?;
            if has_label(vm, &cc)? {
                found = true;
                break;
            }
        }
    }
    if !found {
        revlog.reverse();
        vm.call_method(&lv, "extend", vec![vm.types.new_list(revlog)])?;
        if (core && on_load) || (!core && force) {
            vm.call_method(this, "load_failed", Vec::new())?;
        }
        return Ok(Value::None);
    }
    while greedy && ops::len(vm, &lv)? > 0 {
        // 8.5.1+ keeps the retained entry and stops after it.
        if core {
            if let Some(last) = revlog.last() {
                if gb(vm, last, "retain_after_load")? {
                    break;
                }
            }
        }
        let rb = ops::getitem(vm, &lv, Value::Int(-1))?;
        let c = g(vm, &rb, "context")?;
        let cc = g(vm, &c, "current")?;
        if !has_label(vm, &cc)?
            || gb(vm, &rb, "hard_checkpoint")?
            || gb(vm, &rb, "not_greedy")?
            || (!core && gb(vm, &rb, "retain_after_load")?)
        {
            break;
        }
        revlog.push(vm.call_method(&lv, "pop", Vec::new())?);
    }
    let gm = py::import(vm, "renpy.game")?;
    let contexts = game(vm, "contexts")?;
    let old_contexts = py::items(vm, &contexts)?;
    let ctx = context(vm)?;
    let replace_context = !core && !gb(vm, &ctx, "rollback")?;
    let other_contexts = if replace_context {
        py::set(
            vm,
            &gm,
            "contexts",
            vm.types.new_list(old_contexts[..1].to_vec()),
        )?;
        old_contexts[1..].to_vec()
    } else {
        Vec::new()
    };
    let r = (|| -> PyResult<Value> {
        let mut retained = Value::None;
        if on_load {
            if let Some(last) = revlog.last() {
                if gb(vm, last, "retain_after_load")? {
                    retained = revlog.pop().unwrap();
                    st(vm, this, "retain_after_load_flag", Value::Bool(true))?;
                }
            }
        }
        let mut come_from = Value::None;
        if !py::is_none(&current_label) {
            let ctx = context(vm)?;
            come_from = g(vm, &ctx, "current")?;
            label = current_label.clone();
        }
        let fwd = g(vm, this, "forward")?;
        for rb in &revlog {
            vm.call_method(rb, "rollback", Vec::new())?;
            let c = g(vm, rb, "context")?;
            let cc = g(vm, &c, "current")?;
            if !fixed_forward {
                let frb = g(vm, this, "fixed_rollback_boundary")?;
                if ops::eq(vm, &cc, &frb)? && py::truthy(vm, &cc)? {
                    st(vm, this, "rollback_is_fixed", Value::Bool(true))?;
                }
            }
            let f = g(vm, rb, "forward")?;
            if !py::is_none(&f) {
                let t = if fixed_forward {
                    let fc = mget(vm, "Forward")?;
                    let fixed = g(vm, rb, "fixed")?;
                    vm.call(&fc, vec![cc, f, fixed], Vec::new())?
                } else {
                    vm.types.new_tuple(vec![cc, f])
                };
                vm.call_method(&fwd, "insert", vec![Value::Int(0), t])?;
            }
        }
        if !py::is_none(&retained) {
            vm.call_method(&retained, "rollback_control", Vec::new())?;
            if core {
                let c = g(vm, &retained, "context")?;
                let copy = vm.call_method(&c, "rollback_copy", Vec::new())?;
                st(vm, &retained, "context", copy)?;
                st(vm, this, "current", retained.clone())?;
            }
            ops::list_append(vm, &lv, retained)?;
        }
        Ok(come_from)
    })();
    let mut come_from = match r {
        Ok(c) => c,
        Err(e) => {
            py::set(vm, &gm, "contexts", vm.types.new_list(old_contexts))?;
            return Err(e);
        }
    };
    if !py::is_none(&label) && py::is_none(&come_from) {
        let ctx = context(vm)?;
        come_from = g(vm, &ctx, "current")?;
    }
    if !py::is_none(&come_from) {
        let ctx = context(vm)?;
        vm.call_method(&ctx, "come_from", vec![come_from, label])?;
    }
    let iface = game(vm, "interface")?;
    if !py::is_none(&iface) {
        st(vm, &iface, "suppress_transition", Value::Bool(abnormal))?;
    }
    if (core && on_load) || (!core && force) {
        let rng = mget(vm, "rng")?;
        vm.call_method(&rng, "reset", Vec::new())?;
        let fwd = g(vm, this, "forward")?;
        vm.call_method(&fwd, "clear", Vec::new())?;
    }
    py::set(vm, &gm, "after_rollback", Value::Bool(abnormal))?;
    let ar = py::import_attr(vm, "renpy.audio.audio", "rollback")?;
    vm.call(&ar, Vec::new(), Vec::new())?;
    let contexts = game(vm, "contexts")?;
    for c in py::items(vm, &contexts)? {
        let sl = g(vm, &c, "scene_lists")?;
        vm.call_method(&sl, "remove_all_hidden", Vec::new())?;
    }
    vm.call_method(&contexts, "extend", vec![vm.types.new_list(other_contexts)])?;
    let eds = py::import_attr(vm, "renpy.exports", "execute_default_statement")?;
    vm.call(&eds, vec![Value::Bool(false)], Vec::new())?;
    let m = g(vm, this, "mutated")?;
    vm.call_method(&m, "clear", Vec::new())?;
    let bs = py::import_attr(vm, "renpy.python", "begin_stores")?;
    vm.call(&bs, Vec::new(), Vec::new())?;
    let rb = new_rollback(vm)?;
    let target = if replace_context {
        ops::getitem(vm, &contexts, Value::Int(0))?
    } else {
        context(vm)?
    };
    let copy = vm.call_method(&target, "rollback_copy", Vec::new())?;
    st(vm, &rb, "context", copy)?;
    st(vm, this, "current", rb.clone())?;
    let lv = g(vm, this, "log")?;
    if !py::is_none(&lv) {
        ops::list_append(vm, &lv, rb)?;
    }
    if core {
        return Ok(Value::None);
    }
    Err(raise(
        vm,
        "renpy.game",
        if replace_context {
            "RestartTopContext"
        } else {
            "RestartContext"
        },
    ))
}

fn rl_freeze(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let wait = arg_or(args, kw, 1, "wait", Value::None);
    rl_complete(vm, &[this.clone(), Value::Bool(false)], &[])?;
    let roots = vm.call_method(this, "get_roots", Vec::new())?;
    py::method_kw(
        vm,
        this,
        "purge_unreachable",
        vec![roots.clone()],
        vec![("wait".into(), wait)],
    )?;
    let cur = g(vm, this, "current")?;
    st(vm, &cur, "purged", Value::Bool(false))?;
    Ok(roots)
}

fn rl_discard_freeze(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for i in py::get_items(vm, &args[0], "log")? {
        st(vm, &i, "purged", Value::Bool(false))?;
    }
    Ok(Value::None)
}

/// `unfreeze(roots, label=None)`. From 8.5.1 it checks that the log can
/// roll back and raises `UnfreezeException`, which the top `run_context`
/// performs via `unfreeze_core`.
fn rl_unfreeze(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if !super::state::deferred_rollback(vm) {
        return rl_unfreeze_core(vm, args, kw);
    }
    let this = &args[0];
    let roots = arg_or(args, kw, 1, "roots", Value::None);
    let label = arg_or(args, kw, 2, "label", Value::None);
    if !can_rollback_to(vm, this, 0, true)? {
        let lfl = py::config(vm, "load_failed_label")?;
        if !py::truthy(vm, &lfl)? {
            return Err(py::error(vm, "Could not load the game. Perhaps the script changed in an incompatible way."));
        }
    }
    let c = mget(vm, "UnfreezeException")?;
    let e = vm.call(&c, vec![this.clone(), roots, label], Vec::new())?;
    Err(PyErr::Exc(e.obj().cloned().ok_or_else(|| vm.type_error("UnfreezeException"))?))
}

fn rl_unfreeze_core(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let roots = arg_or(args, kw, 1, "roots", Value::None);
    let label = arg_or(args, kw, 2, "label", Value::None);
    let br = py::import_attr(vm, "renpy.display.screen", "before_restart")?;
    vm.call(&br, Vec::new(), Vec::new())?;
    if super::state::try_rt(vm).is_some_and(|r| r.profile.at_least(8, 6)) {
        // Pose selections are unsaved and must not outlive the old state.
        if let Ok(f) = py::import_attr(vm, "renpy.gl2.live2d", "reset_pose_states") {
            vm.call(&f, Vec::new(), Vec::new())?;
        }
    }
    let gm = py::import(vm, "renpy.game")?;
    py::set(vm, &gm, "log", this.clone())?;
    let cs = py::import_attr(vm, "renpy.python", "clean_stores")?;
    vm.call(&cs, Vec::new(), Vec::new())?;
    let sds = store_dicts(vm)?;
    let del = deleted(vm)?;
    for (name, value) in ops::mapping_items(vm, &roots)? {
        let n = py::string(vm, &name)?;
        let (sname, key) = match n.rsplit_once('.') {
            Some((a, b)) => (a.to_owned(), b.to_owned()),
            None => ("store".to_owned(), n.clone()),
        };
        let sk = py::s(vm, &sname);
        if !ops::contains(vm, &sds, &sk)? {
            continue;
        }
        let store = ops::getitem(vm, &sds, sk)?;
        let ebc = g(vm, &store, "ever_been_changed")?;
        let k = py::s(vm, &key);
        ops::set_add(vm, &ebc, k.clone())?;
        if py::same(&value, &del) {
            vm.call_method(&store, "pop", vec![k, Value::None])?;
        } else {
            ops::setitem(vm, &store, k, value)?;
        }
    }
    let gr = py::import_attr(vm, "store", "_greedy_rollback").unwrap_or(Value::Bool(true));
    let session = py::import_attr(vm, "renpy", "session")?;
    let greedy = vm.call_method(&session, "pop", vec![py::s(vm, "_greedy_rollback"), gr])?;
    py::method_kw(
        vm,
        this,
        "rollback",
        vec![Value::Int(0)],
        vec![
            ("force".into(), Value::Bool(true)),
            ("label".into(), label),
            ("greedy".into(), greedy),
            ("on_load".into(), Value::Bool(true)),
        ],
    )
}

fn rl_build_identifier_cache(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ic = g(vm, this, "identifier_cache")?;
    if !py::is_none(&ic) {
        return Ok(Value::None);
    }
    let mut limit = gi(vm, this, "rollback_limit")?;
    let mut checkpoints = 1;
    let cache = vm.types.new_dict();
    st(vm, this, "identifier_cache", cache.clone())?;
    for i in py::get_items(vm, this, "log")?.iter().rev() {
        let id = g(vm, i, "identifier")?;
        if !py::is_none(&id) {
            let c = g(vm, i, "context")?;
            let cc = g(vm, &c, "current")?;
            if has_label(vm, &cc)? {
                ops::setitem(vm, &cache, id, Value::Int(checkpoints))?;
            }
        }
        if gb(vm, i, "hard_checkpoint")? {
            checkpoints += 1;
        }
        if gb(vm, i, "checkpoint")? {
            limit -= 1;
        }
        if limit == 0 {
            break;
        }
    }
    Ok(Value::None)
}

fn rl_get_identifier_checkpoints(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    rl_build_identifier_cache(vm, &args[..1], &[])?;
    let ic = g(vm, &args[0], "identifier_cache")?;
    vm.call_method(&ic, "get", vec![args[1].clone(), Value::None])
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let fixed_forward = super::state::fixed_forward(vm);
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("reached", f_reached), ("reached_vars", f_reached_vars)],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let obj = Value::Obj(vm.types.object.clone());
    let sd = py::class(
        vm,
        MODULE,
        "StoreDeleted",
        &[obj.clone()],
        &[("__reduce__", sd_reduce)],
        &[],
    )?;
    let dv = vm.call(&Value::Obj(sd.clone()), Vec::new(), Vec::new())?;
    set(vm, "StoreDeleted", Value::Obj(sd));
    set(vm, "deleted", dv);
    let slots = vm.types.new_tuple(Vec::new());
    let snr = py::class(
        vm,
        MODULE,
        "SlottedNoRollback",
        &[obj],
        &[],
        &[("__slots__", slots)],
    )?;
    let snr = Value::Obj(snr);
    set(vm, "SlottedNoRollback", snr.clone());
    let nr = py::class(vm, MODULE, "NoRollback", &[snr], &[], &[])?;
    set(vm, "NoRollback", Value::Obj(nr));
    let ro = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let an = vm.types.new_builtin("__new__", always_new);
    let an = py::staticmethod(vm, an);
    let ar = py::class(vm, MODULE, "AlwaysRollback", &[ro], &[], &[("__new__", an)])?;
    set(vm, "AlwaysRollback", Value::Obj(ar));
    set(vm, "NOROLLBACK_TYPES", vm.types.new_tuple(Vec::new()));
    set(vm, "generation", Value::Float(super::link::now(vm)));
    set(vm, "serial", Value::Int(0));
    let dr = py::import_attr(vm, "renpy.revertable", "DetRandom")?;
    let rng = vm.call(&dr, Vec::new(), Vec::new())?;
    set(vm, "rng", rng);
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let rb = py::class(
        vm,
        MODULE,
        "Rollback",
        &[object.clone()],
        &[
            ("__init__", rb_init),
            ("after_upgrade", rb_after_upgrade),
            ("purge_unreachable", rb_purge_unreachable),
            ("rollback", rb_rollback),
            ("rollback_control", rb_rollback_control),
        ],
        &[
            ("__version__", Value::Int(5)),
            ("identifier", Value::None),
            ("not_greedy", Value::Bool(false)),
            ("checkpointing_suspended", Value::Bool(false)),
        ],
    )?;
    if fixed_forward {
        vm.types.set_attr(&rb, "fixed", Value::Bool(false));
    }
    set(vm, "Rollback", Value::Obj(rb));
    let ns: Vec<Value> = ["old_store", "mutated", "identifier_cache"]
        .iter()
        .map(|s| vm.types.new_str(s))
        .collect();
    let nosave = vm.types.new_list(ns);
    let rl = py::class(
        vm,
        MODULE,
        "RollbackLog",
        &[object],
        &[
            ("__init__", rl_init),
            ("after_setstate", rl_after_setstate),
            ("after_upgrade", rl_after_upgrade),
            ("begin", rl_begin),
            ("replace_node", rl_replace_node),
            ("complete", rl_complete),
            ("get_roots", rl_get_roots),
            ("purge_unreachable", rl_purge_unreachable),
            ("in_rollback", rl_in_rollback),
            ("in_fixed_rollback", rl_in_fixed_rollback),
            ("forward_info", rl_forward_info),
            ("checkpoint", rl_checkpoint),
            ("suspend_checkpointing", rl_suspend_checkpointing),
            ("block", rl_block),
            ("retain_after_load", rl_retain_after_load),
            ("fix_rollback", rl_fix_rollback),
            ("can_rollback", rl_can_rollback),
            ("load_failed", rl_load_failed),
            ("rollback", rl_rollback),
            ("freeze", rl_freeze),
            ("discard_freeze", rl_discard_freeze),
            ("unfreeze", rl_unfreeze),
            ("unfreeze_core", rl_unfreeze_core),
            ("rollback_core", rl_rollback_core),
            ("build_identifier_cache", rl_build_identifier_cache),
            ("get_identifier_checkpoints", rl_get_identifier_checkpoints),
        ],
        &[
            ("__version__", Value::Int(if fixed_forward { 7 } else { 6 })),
            ("nosave", nosave),
            ("identifier_cache", Value::None),
            ("force_checkpoint", Value::Bool(false)),
        ],
    )?;
    set(vm, "RollbackLog", Value::Obj(rl));
    if fixed_forward {
        let f = py::class(vm, MODULE, "Forward", &[], &[("__init__", forward_init)], &[])?;
        set(vm, "Forward", Value::Obj(f));
    }
    if super::state::deferred_rollback(vm) {
        let base = Value::Obj(vm.types.exc("BaseException"));
        let re = py::class(
            vm,
            MODULE,
            "RollbackException",
            &[base.clone()],
            &[("__init__", rbexc_init), ("perform_rollback", rbexc_perform)],
            &[],
        )?;
        set(vm, "RollbackException", Value::Obj(re));
        let ue = py::class(
            vm,
            MODULE,
            "UnfreezeException",
            &[base],
            &[("__init__", ufexc_init), ("perform_unfreeze", ufexc_perform)],
            &[],
        )?;
        set(vm, "UnfreezeException", Value::Obj(ue));
    }
    Ok(m)
}

/// `Forward(name, data, fixed)` (8.4+).
fn forward_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, n) in ["name", "data", "fixed"].iter().enumerate() {
        let v = arg_or(&args[1..], kw, i, n, Value::None);
        st(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

const RBEXC_FIELDS: &[(&str, fn() -> Value)] = &[
    ("checkpoints", || Value::Int(0)),
    ("label", || Value::None),
    ("greedy", || Value::Bool(true)),
    ("on_load", || Value::Bool(false)),
    ("abnormal", || Value::Bool(true)),
    ("current_label", || Value::None),
];

fn rbexc_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, (n, d)) in RBEXC_FIELDS.iter().enumerate() {
        let v = arg_or(&args[1..], kw, i, n, d());
        st(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

/// `RollbackException.perform_rollback()`: `renpy.game.log.rollback_core`.
fn rbexc_perform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut a = vec![g(vm, &args[0], "checkpoints")?];
    let mut kw = Vec::new();
    for (n, _) in &RBEXC_FIELDS[1..] {
        kw.push(((*n).into(), g(vm, &args[0], n)?));
    }
    let lg = log(vm)?;
    let core = g(vm, &lg, "rollback_core")?;
    a.truncate(1);
    vm.call(&core, a, kw)
}

fn ufexc_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, n) in ["log", "roots", "label"].iter().enumerate() {
        let v = arg_or(&args[1..], kw, i, n, Value::None);
        st(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

/// `UnfreezeException.perform_unfreeze()`: `self.log.unfreeze_core`.
fn ufexc_perform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let lg = g(vm, &args[0], "log")?;
    let roots = g(vm, &args[0], "roots")?;
    let label = g(vm, &args[0], "label")?;
    vm.call_method(&lg, "unfreeze_core", vec![roots, label])
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
