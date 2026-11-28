// SPDX-License-Identifier: MPL-2.0
//! `renpy.execution`, after Ren'Py 8.3 `execution.py` (MIT): the
//! execution `Context` (current node name, call/return stacks, dynamic
//! variable scopes, scene lists and translation state), its statement loop,
//! `run_context` and the infinite-loop guard. Contexts are saved in every
//! original save; `after_upgrade` covers versions 0-16.
//!
//! Node names are those of `super::nodes`; `next_node` holds a node handle
//! and is not saved.
use super::{
    nodes,
    py::{self, *},
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::cell::Cell;

const MODULE: &str = "renpy.execution";

thread_local! {
    static IL_STATEMENTS: Cell<u32> = const { Cell::new(0) };
    static IL_TIME: Cell<f64> = const { Cell::new(0.) };
}

fn g(vm: &mut Vm, o: &Value, n: &str) -> PyResult<Value> {
    py::get_or_none(vm, o, n)
}

fn gb(vm: &mut Vm, o: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, o, n)?;
    py::truthy(vm, &v)
}

fn st(vm: &mut Vm, o: &Value, n: &str, v: Value) -> PyResult<()> {
    py::set(vm, o, n, v)
}

fn store(vm: &mut Vm) -> PyResult<Value> {
    super::python::store_dict(vm, "store")
}

fn store_set(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let s = store(vm)?;
    py::dict_set(vm, &s, n, v)
}

fn store_get(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let s = store(vm)?;
    let k = py::s(vm, n);
    Ok(ops::dict_get(vm, s.obj().unwrap(), &k)?.unwrap_or(Value::None))
}

fn game(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", n)
}

fn raise_new(vm: &mut Vm, module: &str, cls: &str, args: Vec<Value>) -> PyErr {
    match py::import_attr(vm, module, cls).and_then(|c| vm.call(&c, args, Vec::new())) {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error(cls),
        Err(e) => e,
    }
}

/// Whether `e` is an instance of the class `module.cls` (or a subclass).
pub fn exc_isinstance(vm: &mut Vm, e: &PyErr, module: &str, cls: &str) -> bool {
    let PyErr::Exc(o) = e else { return false };
    match py::import_attr(vm, module, cls) {
        Ok(c) => c
            .obj()
            .is_some_and(|c| ops::isinstance(vm, &Value::Obj(o.clone()), c)),
        Err(_) => false,
    }
}

fn exc_value(e: &PyErr) -> Value {
    match e {
        PyErr::Exc(o) => Value::Obj(o.clone()),
        _ => Value::None,
    }
}

fn is_control(vm: &mut Vm, e: &PyErr) -> bool {
    if matches!(e, PyErr::Suspend(..)) {
        return true;
    }
    let ce = match game(vm, "CONTROL_EXCEPTIONS") {
        Ok(c) => c,
        Err(_) => return false,
    };
    let v = exc_value(e);
    py::seq(&ce)
        .unwrap_or_default()
        .iter()
        .any(|c| c.obj().is_some_and(|c| ops::isinstance(vm, &v, c)))
}

// Infinite loop guard ----------------------------------------------------------------------------------

/// `check_infinite_loop()`: 1000 statements without an interaction for a
/// minute of host time raise. Host time stands still within a tick, so the
/// guard compares the host clock as upstream compares wall time.
pub fn check_infinite_loop(vm: &mut Vm) -> PyResult<()> {
    let n = IL_STATEMENTS.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    if n <= 1000 {
        return Ok(());
    }
    IL_STATEMENTS.with(|c| c.set(0));
    let now = super::link::now(vm);
    let il = IL_TIME.with(|c| c.get());
    if now > il && il > 0. {
        IL_TIME.with(|c| c.set(now + 60.));
        return Err(py::error(vm, "Possible infinite loop."));
    }
    if il == 0. {
        IL_TIME.with(|c| c.set(now + 60.));
    }
    Ok(())
}

pub fn not_infinite_loop(vm: &mut Vm, delay: f64) {
    let d = if py::config_bool(vm, "developer").unwrap_or(false) {
        delay
    } else {
        delay * 5.
    };
    let now = super::link::now(vm);
    IL_TIME.with(|c| c.set(now + d));
}

fn f_check_infinite_loop(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    check_infinite_loop(vm)?;
    Ok(Value::None)
}

fn f_not_infinite_loop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::f64_of(vm, &args[0])?;
    not_infinite_loop(vm, d);
    Ok(Value::None)
}

// LineLogEntry -----------------------------------------------------------------------------------------

fn lle_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    st(vm, this, "filename", args[1].clone())?;
    st(vm, this, "line", args[2].clone())?;
    let n = g(vm, &args[3], "name")?;
    st(vm, this, "node_name", n)?;
    st(vm, this, "abnormal", args[4].clone())?;
    let cbs = py::config(vm, "line_log_callbacks")?;
    for cb in py::items(vm, &cbs)? {
        vm.call(&cb, vec![this.clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn lle_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    let c = py::import_attr(vm, MODULE, "LineLogEntry")?;
    if !c.obj().is_some_and(|c| ops::isinstance(vm, b, c)) {
        return Ok(Value::Bool(false));
    }
    let eq = |vm: &mut Vm, n: &str| -> PyResult<bool> {
        let x = g(vm, a, n)?;
        let y = g(vm, b, n)?;
        ops::eq(vm, &x, &y)
    };
    let na = g(vm, a, "node_name")?;
    let nb = g(vm, b, "node_name")?;
    Ok(Value::Bool(
        eq(vm, "filename")? && eq(vm, "line")? && (na.is(&nb) || ops::eq(vm, &na, &nb)?),
    ))
}

fn lle_ne(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = lle_eq(vm, args, kw)?;
    Ok(Value::Bool(!py::truthy(vm, &r)?))
}

fn lle_node(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = g(vm, &args[0], "node_name")?;
    let id = nodes::lookup(vm, &n)?;
    nodes::handle(vm, id)
}

// Context ----------------------------------------------------------------------------------------------

fn new_list(vm: &Vm) -> Value {
    vm.types.new_list(Vec::new())
}

fn revertable(vm: &mut Vm, cls: &str, args: Vec<Value>) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.revertable", cls)?;
    vm.call(&c, args, Vec::new())
}

fn ctx_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let rollback = arg_or(args, kw, 1, "rollback", Value::Bool(false));
    let context = arg_or(args, kw, 2, "context", Value::None);
    let clear = arg_or(args, kw, 3, "clear", Value::Bool(false));
    st(vm, this, "current", Value::None)?;
    for n in [
        "call_location_stack",
        "return_stack",
        "abnormal_stack",
        "line_log",
    ] {
        st(vm, this, n, new_list(vm))?;
    }
    let d0 = vm.types.new_dict();
    st(vm, this, "dynamic_stack", vm.types.new_list(vec![d0]))?;
    st(vm, this, "rollback", rollback.clone())?;
    st(vm, this, "runtime", Value::Int(0))?;
    let info = revertable(vm, "RevertableObject", Vec::new())?;
    st(vm, this, "info", info.clone())?;
    st(vm, this, "seen", Value::Bool(false))?;
    st(vm, this, "abnormal", Value::Bool(true))?;
    st(vm, this, "last_abnormal", Value::Bool(false))?;
    st(vm, this, "music", vm.types.new_dict())?;
    st(vm, this, "movie", vm.types.new_dict())?;
    for n in ["interacting", "init_phase", "force_checkpoint"] {
        st(vm, this, n, Value::Bool(false))?;
    }
    for n in [
        "defer_rollback",
        "exception_handler",
        "say_attributes",
        "temporary_attributes",
    ] {
        st(vm, this, n, Value::None)?;
    }
    let sii = py::import_attr(vm, "renpy.display.image", "ShownImageInfo")?;
    let oldsl;
    if py::truthy(vm, &context)? {
        oldsl = g(vm, &context, "scene_lists")?;
        let rt = g(vm, &context, "runtime")?;
        st(vm, this, "runtime", rt)?;
        let ci = g(vm, &context, "info")?;
        let sd = py::namespace_of(vm, &ci)?;
        let dd = py::namespace_of(vm, &info)?;
        vm.call_method(&dd, "update", vec![sd])?;
        let dict_t = Value::Obj(vm.types.dict.clone());
        let m = g(vm, &context, "music")?;
        let m = vm.call(&dict_t, vec![m], Vec::new())?;
        st(vm, this, "music", m)?;
        let mv = g(vm, &context, "movie")?;
        let mv = vm.call(&dict_t, vec![mv], Vec::new())?;
        st(vm, this, "movie", mv)?;
        let ci = g(vm, &context, "images")?;
        let im = vm.call(&sii, vec![ci], Vec::new())?;
        st(vm, this, "images", im)?;
    } else {
        oldsl = Value::None;
        let im = vm.call(&sii, vec![Value::None], Vec::new())?;
        st(vm, this, "images", im)?;
    }
    let slc = py::import_attr(vm, "renpy.display.scenelists", "SceneLists")?;
    let images = g(vm, this, "images")?;
    let sl = vm.call(&slc, vec![oldsl, images], Vec::new())?;
    st(vm, this, "scene_lists", sl.clone())?;
    // context_copy_remove_screens arrived in 7.3.
    let ccrs = if super::state::rt(vm).profile.at_least(7, 3) {
        py::config(vm, "context_copy_remove_screens")?
    } else {
        vm.types.new_list(Vec::new())
    };
    for i in py::items(vm, &ccrs)? {
        let scr = py::s(vm, "screens");
        vm.call_method(&sl, "remove", vec![scr, i, Value::None])?;
    }
    let names: Vec<Value> = [
        "_return",
        "_args",
        "_kwargs",
        "mouse_visible",
        "suppress_overlay",
        "_side_image_attributes",
    ]
    .iter()
    .map(|n| vm.types.new_str(n))
    .collect();
    let names = vm.types.new_list(names);
    vm.call_method(this, "make_dynamic", vec![names])?;
    let ds = g(vm, this, "dynamic_stack")?;
    ops::list_append(vm, &ds, vm.types.new_dict())?;
    if py::truthy(vm, &clear)? {
        let layers = if matches!(clear, Value::Bool(true)) {
            py::config(vm, "context_clear_layers")?
        } else {
            clear
        };
        for i in py::items(vm, &layers)? {
            py::method_kw(vm, &sl, "clear", Vec::new(), vec![("layer".into(), i)])?;
        }
    }
    let start = py::s(vm, "start");
    let modes = revertable(vm, "RevertableList", vec![vm.types.new_list(vec![start])])?;
    st(vm, this, "modes", modes)?;
    st(vm, this, "use_modes", Value::Bool(true))?;
    let prefs = game(vm, "preferences")?;
    let lang = if py::is_none(&prefs) {
        Value::None
    } else {
        g(vm, &prefs, "language")?
    };
    st(vm, this, "translate_language", lang)?;
    for n in [
        "translate_identifier",
        "alternate_translate_identifier",
        "deferred_translate_identifier",
    ] {
        st(vm, this, n, Value::None)?;
    }
    Ok(Value::None)
}

fn ctx_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 1 {
        let sl = g(vm, this, "scene_lists")?;
        let pi = g(vm, this, "predict_info")?;
        let im = g(vm, &pi, "images")?;
        st(vm, &sl, "image_predict_info", im)?;
    }
    if version < 2 {
        st(vm, this, "abnormal", Value::Bool(false))?;
        st(vm, this, "last_abnormal", Value::Bool(false))?;
    }
    if version < 3 {
        st(vm, this, "music", vm.types.new_dict())?;
    }
    if version < 4 {
        st(vm, this, "interacting", Value::Bool(false))?;
    }
    if version < 5 {
        let start = py::s(vm, "start");
        let modes = revertable(vm, "RevertableList", vec![vm.types.new_list(vec![start])])?;
        st(vm, this, "modes", modes)?;
        st(vm, this, "use_modes", Value::Bool(true))?;
    }
    if version < 6 {
        let pi = g(vm, this, "predict_info")?;
        let im = g(vm, &pi, "images")?;
        st(vm, this, "images", im)?;
    }
    if version < 7 {
        st(vm, this, "init_phase", Value::Bool(false))?;
        st(vm, this, "next_node", Value::None)?;
    }
    if version < 8 {
        st(vm, this, "defer_rollback", Value::None)?;
    }
    if version < 9 {
        st(vm, this, "translate_language", Value::None)?;
        st(vm, this, "translate_identifier", Value::None)?;
    }
    if version < 10 {
        st(vm, this, "exception_handler", Value::None)?;
    }
    if version < 11 {
        st(vm, this, "say_attributes", Value::None)?;
    }
    if version < 13 {
        st(vm, this, "line_log", new_list(vm))?;
    }
    if version < 14 {
        st(vm, this, "movie", vm.types.new_dict())?;
    }
    if version < 15 {
        let rs = g(vm, this, "return_stack")?;
        let n = ops::len(vm, &rs)?;
        st(
            vm,
            this,
            "abnormal_stack",
            vm.types.new_list(vec![Value::Bool(false); n]),
        )?;
    }
    if version < 16 {
        st(vm, this, "alternate_translate_identifier", Value::None)?;
    }
    Ok(Value::None)
}

fn ctx_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cur = g(vm, &args[0], "current")?;
    if let Ok(Some(id)) = nodes::find(vm, &cur) {
        let r = super::state::rt(vm);
        let s = {
            let script = r.script.borrow();
            let n = script.arena.get(id);
            format!("<Context: {}:{}>", n.filename, n.linenumber)
        };
        return Ok(py::s(vm, &s));
    }
    Ok(py::s(vm, "<Context>"))
}

fn ctx_replace_node(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (old, new) = (&args[1], &args[2]);
    let new_name = g(vm, new, "name")?;
    let one = |vm: &mut Vm, name: Value| -> PyResult<Value> {
        let id = nodes::find(vm, &name)?;
        if id.is_some() && id == nodes::id_of(old) {
            return Ok(new_name.clone());
        }
        Ok(name)
    };
    let cur = g(vm, this, "current")?;
    let cur = one(vm, cur)?;
    st(vm, this, "current", cur)?;
    let mut rs = Vec::new();
    for i in py::get_items(vm, this, "return_stack")? {
        rs.push(one(vm, i)?);
    }
    st(vm, this, "return_stack", vm.types.new_list(rs))?;
    Ok(Value::None)
}

/// The store dict a dotted dynamic name lives in, and its last part.
fn dynamic_target(vm: &mut Vm, name: &str) -> PyResult<(Value, String)> {
    let mut d = store(vm)?;
    let mut name = name.to_owned();
    while let Some((head, tail)) = name.clone().split_once('.') {
        let k = py::s(vm, head);
        let m = ops::dict_get(vm, d.obj().unwrap(), &k)?.unwrap_or(Value::None);
        let sm = py::import_attr(vm, "renpy.python", "StoreModule")?;
        let is_module = matches!(m.payload(), Some(crysirror_python::Payload::Module(..)))
            || sm.obj().is_some_and(|c| ops::isinstance(vm, &m, c));
        if !is_module {
            return Err(py::error(vm, format!("{head} is not a valid namespace.")));
        }
        d = py::namespace_of(vm, &m)?;
        name = tail.to_owned();
    }
    Ok((d, name))
}

fn ctx_make_dynamic(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = arg_or(args, kw, 1, "names", Value::None);
    let context = py::truthy(vm, &arg_or(args, kw, 2, "context", Value::Bool(false)))?;
    let ds = g(vm, this, "dynamic_stack")?;
    let target = ops::getitem(vm, &ds, Value::Int(if context { 0 } else { -1 }))?;
    let delete = py::import_attr(vm, MODULE, "Delete")?;
    for i in py::items(vm, &names)? {
        if ops::contains(vm, &target, &i)? {
            continue;
        }
        let n = py::string(vm, &i)?;
        let (d, leaf) = dynamic_target(vm, &n)?;
        let k = py::s(vm, &leaf);
        let v = match ops::dict_get(vm, d.obj().unwrap(), &k)? {
            Some(v) => v,
            None => vm.call(&delete, Vec::new(), Vec::new())?,
        };
        ops::setitem(vm, &target, i, v)?;
    }
    Ok(Value::None)
}

pub fn pop_dynamic(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let ds = g(vm, this, "dynamic_stack")?;
    if ops::len(vm, &ds)? == 0 {
        return Ok(());
    }
    let dynamic = vm.call_method(&ds, "pop", Vec::new())?;
    let delete = py::import_attr(vm, MODULE, "Delete")?;
    for (k, v) in ops::mapping_items(vm, &dynamic)? {
        let mut d = store(vm)?;
        let mut name = py::string(vm, &k)?;
        while let Some((head, tail)) = name.clone().split_once('.') {
            let hk = py::s(vm, head);
            let m = ops::getitem(vm, &d, hk)?;
            d = py::namespace_of(vm, &m)?;
            name = tail.to_owned();
        }
        let key = py::s(vm, &name);
        if delete.obj().is_some_and(|c| ops::isinstance(vm, &v, c)) {
            vm.call_method(&d, "pop", vec![key, Value::None])?;
        } else {
            ops::setitem(vm, &d, key, v)?;
        }
    }
    Ok(())
}

fn ctx_pop_dynamic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pop_dynamic(vm, &args[0])?;
    Ok(Value::None)
}

fn ctx_pop_all_dynamic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ds = g(vm, &args[0], "dynamic_stack")?;
    while ops::len(vm, &ds)? > 0 {
        pop_dynamic(vm, &args[0])?;
    }
    Ok(Value::None)
}

fn ctx_pop_dynamic_roots(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let roots = &args[1];
    let delete = py::import_attr(vm, MODULE, "Delete")?;
    for dynamic in py::get_items(vm, this, "dynamic_stack")?.iter().rev() {
        for (k, v) in ops::mapping_items(vm, dynamic)? {
            let n = py::string(vm, &k)?;
            let name = py::s(vm, &format!("store.{n}"));
            if delete.obj().is_some_and(|c| ops::isinstance(vm, &v, c))
                && ops::contains(vm, roots, &name)?
            {
                ops::delitem(vm, roots, name)?;
            } else {
                ops::setitem(vm, roots, name, v)?;
            }
        }
    }
    Ok(Value::None)
}

fn ctx_goto_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "current", args[1].clone())?;
    Ok(Value::None)
}

fn ctx_check_stacks(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ds = g(vm, this, "dynamic_stack")?;
    let rs = g(vm, this, "return_stack")?;
    let (d, r) = (ops::len(vm, &ds)?, ops::len(vm, &rs)?);
    if d != r + 2 {
        while ops::len(vm, &ds)? < r + 2 {
            ops::list_append(vm, &ds, vm.types.new_dict())?;
        }
        while ops::len(vm, &ds)? > r + 2 {
            pop_dynamic(vm, this)?;
        }
        return Err(py::error(
            vm,
            format!("Potential return stack corruption: dynamic={d} return={r}"),
        ));
    }
    Ok(Value::None)
}

fn ctx_report_traceback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if py::truthy(vm, &args.get(2).cloned().unwrap_or(Value::Bool(false)))? {
        return Ok(Value::None);
    }
    let mut rv = Vec::new();
    let mut names = py::get_items(vm, this, "call_location_stack")?;
    names.push(g(vm, this, "current")?);
    let last = names.len() - 1;
    for (i, n) in names.into_iter().enumerate() {
        if let Ok(Some(id)) = nodes::find(vm, &n) {
            let r = super::state::rt(vm);
            let (f, l) = {
                let s = r.script.borrow();
                let node = s.arena.get(id);
                (node.filename.to_string(), node.linenumber)
            };
            if !f.replace('\\', "/").starts_with("common/") {
                let what = if i == last { "script" } else { "script call" };
                let t = vec![
                    py::s(vm, &f),
                    Value::Int(l as i64),
                    py::s(vm, what),
                    Value::None,
                ];
                rv.push(vm.types.new_tuple(t));
            }
        }
    }
    Ok(vm.types.new_list(rv))
}

fn ctx_come_from(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "come_from_name", args[1].clone())?;
    st(vm, &args[0], "come_from_label", args[2].clone())?;
    Ok(Value::None)
}

fn ctx_mark_seen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "seen", Value::Bool(true))?;
    Ok(Value::None)
}

pub fn call(vm: &mut Vm, this: &Value, label: &Value, return_site: Value) -> PyResult<Value> {
    let current = g(vm, this, "current")?;
    if !py::truthy(vm, &current)? {
        return Err(py::error(
            vm,
            "Context not capable of executing Ren'Py code.",
        ));
    }
    let return_site = if py::is_none(&return_site) {
        current.clone()
    } else {
        return_site
    };
    let cls = g(vm, this, "call_location_stack")?;
    ops::list_append(vm, &cls, current)?;
    let rs = g(vm, this, "return_stack")?;
    ops::list_append(vm, &rs, return_site)?;
    let ds = g(vm, this, "dynamic_stack")?;
    ops::list_append(vm, &ds, vm.types.new_dict())?;
    let abs = g(vm, this, "abnormal_stack")?;
    let la = g(vm, this, "last_abnormal")?;
    ops::list_append(vm, &abs, la)?;
    st(vm, this, "current", label.clone())?;
    let names = vm
        .types
        .new_list(vec![py::s(vm, "_args"), py::s(vm, "_kwargs")]);
    vm.call_method(this, "make_dynamic", vec![names])?;
    store_set(vm, "_args", Value::None)?;
    store_set(vm, "_kwargs", Value::None)?;
    let id = nodes::lookup(vm, label)?;
    nodes::handle(vm, id)
}

fn ctx_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rs = arg_or(args, kw, 2, "return_site", Value::None);
    call(vm, &args[0], &args[1], rs)
}

fn ctx_pop_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let rs = g(vm, this, "return_stack")?;
    if ops::len(vm, &rs)? == 0 {
        if py::config_bool(vm, "developer")? {
            return Err(py::error(vm, "No call on call stack."));
        }
        return Ok(Value::None);
    }
    vm.call_method(&rs, "pop", Vec::new())?;
    let cls = g(vm, this, "call_location_stack")?;
    vm.call_method(&cls, "pop", Vec::new())?;
    pop_dynamic(vm, this)?;
    let abs = g(vm, this, "abnormal_stack")?;
    vm.call_method(&abs, "pop", Vec::new())?;
    Ok(Value::None)
}

fn ctx_lookup_return(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pop = py::truthy(vm, &arg_or(args, kw, 1, "pop", Value::Bool(true)))?;
    let rs = g(vm, this, "return_stack")?;
    let cls = g(vm, this, "call_location_stack")?;
    let abs = g(vm, this, "abnormal_stack")?;
    while ops::len(vm, &rs)? > 0 {
        let top = ops::getitem(vm, &rs, Value::Int(-1))?;
        let mut node = match nodes::find(vm, &top)? {
            Some(id) => Some(nodes::handle(vm, id)?),
            None => None,
        };
        if node.is_none() {
            let cl = ops::getitem(vm, &cls, Value::Int(-1))?;
            if let Some(id) = nodes::find(vm, &cl)? {
                let h = nodes::handle(vm, id)?;
                let n = g(vm, &h, "next")?;
                node = (!py::is_none(&n)).then_some(n);
            }
        }
        let Some(node) = node else {
            if !pop {
                return Ok(Value::None);
            }
            let rnf = py::config(vm, "return_not_found_label")?;
            if py::truthy(vm, &rnf)? {
                while ops::len(vm, &rs)? > 1 {
                    ctx_pop_call(vm, &[this.clone()], &[])?;
                }
                let id = nodes::lookup(vm, &rnf)?;
                let n = nodes::handle(vm, id)?;
                vm.call_method(&rs, "pop", Vec::new())?;
                vm.call_method(&cls, "pop", Vec::new())?;
                let a = vm.call_method(&abs, "pop", Vec::new())?;
                st(vm, this, "abnormal", a)?;
                return Ok(n);
            }
            if py::config_bool(vm, "developer")? {
                let r = ops::repr(vm, &top)?;
                return Err(py::error(vm, format!("Could not find return label {r}.")));
            }
            vm.call_method(&rs, "pop", Vec::new())?;
            vm.call_method(&cls, "pop", Vec::new())?;
            pop_dynamic(vm, this)?;
            let a = vm.call_method(&abs, "pop", Vec::new())?;
            st(vm, this, "abnormal", a)?;
            continue;
        };
        if pop {
            vm.call_method(&rs, "pop", Vec::new())?;
            vm.call_method(&cls, "pop", Vec::new())?;
            let a = vm.call_method(&abs, "pop", Vec::new())?;
            st(vm, this, "abnormal", a)?;
        }
        return Ok(node);
    }
    Ok(Value::None)
}

fn ctx_rollback_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = py::import_attr(vm, MODULE, "Context")?;
    let rb = g(vm, this, "rollback")?;
    let rv = vm.call(&c, vec![rb, this.clone()], Vec::new())?;
    for n in ["call_location_stack", "return_stack", "abnormal_stack"] {
        let l = py::get_items(vm, this, n)?;
        st(vm, &rv, n, vm.types.new_list(l))?;
    }
    let mut ds = Vec::new();
    for d in py::get_items(vm, this, "dynamic_stack")? {
        ds.push(vm.call_method(&d, "copy", Vec::new())?);
    }
    st(vm, &rv, "dynamic_stack", vm.types.new_list(ds))?;
    for n in [
        "current",
        "runtime",
        "info",
        "translate_language",
        "translate_identifier",
        "abnormal",
        "last_abnormal",
    ] {
        let v = g(vm, this, n)?;
        st(vm, &rv, n, v)?;
    }
    st(vm, &rv, "interacting", Value::Bool(false))?;
    Ok(rv)
}

fn ctx_predict_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prs = py::get_items(vm, this, "predict_return_stack").unwrap_or_default();
    let mut l = prs;
    l.push(args[2].clone());
    st(vm, this, "predict_return_stack", vm.types.new_list(l))?;
    let id = nodes::lookup(vm, &args[1])?;
    nodes::handle(vm, id)
}

fn ctx_predict_return(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prs = g(vm, this, "predict_return_stack")?;
    if !py::truthy(vm, &prs)? {
        return Ok(Value::None);
    }
    let mut l = py::items(vm, &prs)?;
    let label = l.pop().unwrap();
    st(vm, this, "predict_return_stack", vm.types.new_list(l))?;
    let id = nodes::lookup(vm, &label)?;
    nodes::handle(vm, id)
}

/// `predict()`: a generator in upstream; the interface's prediction
/// coroutine iterates it. Each predicted statement is one item.
fn ctx_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let current = g(vm, this, "current")?;
    let mut out = Vec::new();
    let cb = py::config(vm, "predict_statements_callback")?;
    if !py::truthy(vm, &current)? || py::is_none(&cb) {
        return Ok(vm.types.new_list(out));
    }
    let old_images = g(vm, this, "images")?;
    let rs = g(vm, this, "return_stack")?;
    let mut queue: Vec<(Value, Value, Value)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let labels = vm.call(&cb, vec![current], Vec::new())?;
    for label in ops::collect(vm, &labels)? {
        let Some(id) = nodes::find(vm, &label)? else {
            continue;
        };
        if seen.insert(id) {
            queue.push((nodes::handle(vm, id)?, old_images.clone(), rs.clone()));
        }
    }
    let limit = py::config(vm, "predict_statements")?;
    let limit = py::int(vm, &limit)? as usize;
    let sii = py::import_attr(vm, "renpy.display.image", "ShownImageInfo")?;
    let mut i = 0;
    while i < limit && i < queue.len() {
        let (node, images, return_stack) = queue[i].clone();
        let im = vm.call(&sii, vec![images], Vec::new())?;
        st(vm, this, "images", im.clone())?;
        st(vm, this, "predict_return_stack", return_stack)?;
        let r = vm.call_method(&node, "predict", Vec::new());
        if let Ok(next) = r {
            for n in ops::collect(vm, &next).unwrap_or_default() {
                let Some(id) = nodes::id_of(&n) else { continue };
                if seen.insert(id) {
                    let prs = g(vm, this, "predict_return_stack")?;
                    let im = g(vm, this, "images")?;
                    queue.push((n, im, prs));
                }
            }
        } else if py::config_bool(vm, "debug_prediction")? {
            r?;
        }
        st(vm, this, "images", old_images.clone())?;
        st(vm, this, "predict_return_stack", Value::None)?;
        out.push(Value::Bool(true));
        i += 1;
    }
    out.push(Value::Bool(false));
    Ok(vm.types.new_list(out))
}

fn ctx_seen_current(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cur = g(vm, this, "current")?;
    if !py::truthy(vm, &cur)? {
        return Ok(Value::Bool(false));
    }
    let ever = py::truthy(vm, &args[1])?;
    let seen = if ever {
        let p = game(vm, "persistent")?;
        g(vm, &p, "_seen_ever")?
    } else {
        game(vm, "seen_session")?
    };
    Ok(Value::Bool(ops::contains(vm, &seen, &cur)?))
}

fn ctx_do_deferred_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let dr = g(vm, this, "defer_rollback")?;
    if !py::truthy(vm, &dr)? {
        return Ok(Value::None);
    }
    let p = py::items(vm, &dr)?;
    st(vm, this, "defer_rollback", Value::None)?;
    let f = py::import_attr(vm, "renpy.exports", "rollback")?;
    let force = Value::Bool(py::truthy(vm, &p[0])?);
    vm.call(&f, vec![force, p[1].clone()], Vec::new())
}

fn ctx_get_return_stack(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::get_items(vm, &args[0], "return_stack")?;
    Ok(vm.types.new_list(l))
}

fn ctx_set_return_stack(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let l = py::items(vm, &args[1])?;
    let n = l.len();
    st(vm, this, "return_stack", vm.types.new_list(l))?;
    let cls = g(vm, this, "call_location_stack")?;
    let ds = g(vm, this, "dynamic_stack")?;
    while ops::len(vm, &cls)? > n {
        vm.call_method(&cls, "pop", Vec::new())?;
        let d = vm.call_method(&ds, "pop", Vec::new())?;
        let last = ops::getitem(vm, &ds, Value::Int(-1))?;
        vm.call_method(&d, "update", vec![last])?;
        ops::setitem(vm, &ds, Value::Int(-1), d)?;
    }
    while ops::len(vm, &cls)? < n {
        ops::list_append(vm, &cls, py::s(vm, "unknown location"))?;
        ops::list_append(vm, &ds, vm.types.new_dict())?;
    }
    Ok(Value::None)
}

// The statement loop -----------------------------------------------------------------------------------

fn begin_rollback(vm: &mut Vm, this: &Value, node: &Value, first: bool) -> PyResult<()> {
    let br = store_get(vm, "_begin_rollback")?;
    let rollback = g(vm, node, "rollback")?;
    let rollback = py::string(vm, &rollback).unwrap_or_default();
    let (mut update, mut force) = if !py::truthy(vm, &br)? {
        (false, false)
    } else if first || gb(vm, this, "force_checkpoint")? || rollback == "force" {
        (true, true)
    } else if !py::config_bool(vm, "all_nodes_rollback")? && rollback == "never" {
        (false, false)
    } else {
        (true, false)
    };
    let log = game(vm, "log")?;
    if !py::is_none(&log) {
        let fwd = g(vm, &log, "forward")?;
        if py::truthy(vm, &fwd)? {
            let f0 = ops::getitem(vm, &fwd, Value::Int(0))?;
            let n0 = ops::getitem(vm, &f0, Value::Int(0))?;
            let name = g(vm, node, "name")?;
            if ops::eq(vm, &n0, &name)? {
                update = true;
                force = true;
            }
        }
    }
    if update && gb(vm, this, "rollback")? && !py::is_none(&log) {
        py::method_kw(
            vm,
            &log,
            "begin",
            Vec::new(),
            vec![("force".into(), Value::Bool(force))],
        )?;
        if gb(vm, this, "force_checkpoint")? {
            st(vm, &log, "force_checkpoint", Value::Bool(true))?;
            st(vm, this, "force_checkpoint", Value::Bool(false))?;
        }
    }
    Ok(())
}

/// Reports an error raised by game code, giving `exception_handler` and
/// `config.exception_handler` the chance to handle it. Returns Ok when it
/// was handled (execution continues at the next node).
fn handle_error(vm: &mut Vm, this: &Value, e: PyErr) -> PyResult<()> {
    // Upstream's traceback starts at this `Context.run` frame, which
    // reports the script location through `report_traceback`.
    if let PyErr::Exc(o) = &e
        && let Ok(Value::Obj(l)) = ctx_report_traceback(
            vm,
            &[this.clone(), py::s(vm, "run"), Value::Bool(false)],
            &[],
        )
    {
        let ev = Value::Obj(o.clone());
        let tb = py::get_or_none(vm, &ev, "__traceback__")?;
        let mut all = py::items(vm, &Value::Obj(l))?;
        if !py::is_none(&tb) {
            all.extend(py::items(vm, &tb)?);
        }
        let nl = vm.types.new_list(all);
        py::set(vm, &ev, "__traceback__", nl)?;
    }
    if super::error84::active(vm) {
        return super::error84::handle_exception(vm, this, e);
    }
    let report = py::import_attr(vm, "renpy.error", "report_exception");
    let (short, full, tbf) = match report {
        Ok(f) => {
            let ev = exc_value(&e);
            let r = vm.call(&f, vec![ev], vec![("editor".into(), Value::Bool(false))])?;
            let p = py::items(vm, &r)?;
            (p[0].clone(), p[1].clone(), p[2].clone())
        }
        Err(_) => return Err(e),
    };
    let h = g(vm, this, "exception_handler")?;
    let handled = if !py::is_none(&h) {
        vm.call(
            &h,
            vec![short.clone(), full.clone(), tbf.clone()],
            Vec::new(),
        )?;
        true
    } else {
        let ch = py::config(vm, "exception_handler")?;
        if !py::is_none(&ch) {
            let r = vm.call(
                &ch,
                vec![short.clone(), full.clone(), tbf.clone()],
                Vec::new(),
            )?;
            py::truthy(vm, &r)?
        } else {
            false
        }
    };
    if !handled {
        let rep = py::import_attr(vm, "renpy.display.error", "report_exception")?;
        let r = match vm.call(&rep, vec![short, full, tbf], Vec::new()) {
            Ok(r) => r,
            Err(ce) if is_control(vm, &ce) => return Err(ce),
            Err(_) => return Err(e),
        };
        if py::truthy(vm, &r)? {
            return Err(e);
        }
    }
    Ok(())
}

pub fn run(vm: &mut Vm, this: &Value, start: Value) -> PyResult<()> {
    st(vm, this, "exception_handler", Value::None)?;
    st(vm, this, "abnormal", Value::Bool(true))?;
    let mut node = if py::is_none(&start) {
        let cur = g(vm, this, "current")?;
        let id = nodes::lookup(vm, &cur)?;
        nodes::handle(vm, id)?
    } else {
        start
    };
    let mut first = true;
    while !py::is_none(&node) {
        let name = g(vm, &node, "name")?;
        let cfn = g(vm, this, "come_from_name")?;
        if !py::is_none(&cfn) && ops::eq(vm, &name, &cfn)? {
            st(vm, this, "come_from_name", Value::None)?;
            let cfl = g(vm, this, "come_from_label")?;
            node = call(vm, this, &cfl, name.clone())?;
            let names = vm
                .types
                .new_list(vec![py::s(vm, "_return"), py::s(vm, "_begin_rollback")]);
            vm.call_method(this, "make_dynamic", vec![names])?;
            store_set(vm, "_begin_rollback", Value::Bool(false))?;
        }
        let name = g(vm, &node, "name")?;
        st(vm, this, "current", name.clone())?;
        let a = g(vm, this, "abnormal")?;
        st(vm, this, "last_abnormal", a)?;
        st(vm, this, "abnormal", Value::Bool(false))?;
        st(vm, this, "defer_rollback", Value::None)?;
        if py::config_bool(vm, "line_log")? {
            let c = py::import_attr(vm, MODULE, "LineLogEntry")?;
            let f = g(vm, &node, "filename")?;
            let l = g(vm, &node, "linenumber")?;
            let la = g(vm, this, "last_abnormal")?;
            let e = vm.call(&c, vec![f, l, node.clone(), la], Vec::new())?;
            let ll = g(vm, this, "line_log")?;
            if !ops::contains(vm, &ll, &e)? {
                ops::list_append(vm, &ll, e)?;
            }
        }
        begin_rollback(vm, this, &node, first)?;
        first = false;
        st(vm, this, "seen", Value::Bool(false))?;
        let r = (|| -> PyResult<()> {
            check_infinite_loop(vm)?;
            let gm = py::import(vm, "renpy.game")?;
            py::set(
                vm,
                &gm,
                "exception_info",
                py::s(vm, "While running game code:"),
            )?;
            st(vm, this, "next_node", Value::None)?;
            let id = nodes::id_of(&node).ok_or_else(|| vm.type_error("node handle"))?;
            if std::env::var_os("CRYSIRROR_RENPY_TRACE_NODES").is_some() {
                let r = super::state::rt(vm);
                let script = r.script.borrow();
                let n = script.arena.get(id);
                eprintln!("renpy node {}:{}", n.filename, n.linenumber);
            }
            match super::astexec::execute(vm, id) {
                Ok(()) => {
                    let nn = g(vm, this, "next_node")?;
                    if py::config_bool(vm, "developer")? && py::truthy(vm, &nn)? {
                        ctx_check_stacks(vm, &[this.clone()], &[])?;
                    }
                    Ok(())
                }
                Err(e) if is_control(vm, &e) => {
                    st(vm, this, "translate_interaction", Value::None)?;
                    Err(e)
                }
                Err(e) => {
                    st(vm, this, "translate_interaction", Value::None)?;
                    handle_error(vm, this, e)
                }
            }
        })();
        match r {
            Ok(()) => node = g(vm, this, "next_node")?,
            Err(e) if exc_isinstance(vm, &e, "renpy.game", "JumpException") => {
                let ev = exc_value(&e);
                let a = py::get(vm, &ev, "args")?;
                let target = ops::getitem(vm, &a, Value::Int(0))?;
                let id = nodes::lookup(vm, &target)?;
                node = nodes::handle(vm, id)?;
                st(vm, this, "abnormal", Value::Bool(true))?;
            }
            Err(e) if exc_isinstance(vm, &e, "renpy.game", "CallException") => {
                let ev = exc_value(&e);
                let from_current = py::get_or_none(vm, &ev, "from_current")?;
                let return_site = if py::truthy(vm, &from_current)? {
                    let ss = g(vm, &node, "statement_start")?;
                    g(vm, &ss, "name")?
                } else {
                    let nn = g(vm, this, "next_node")?;
                    if py::is_none(&nn) {
                        return Err(py::error(
                            vm,
                            "renpy.call can't be used when the next node is undefined.",
                        ));
                    }
                    g(vm, &nn, "name")?
                };
                let label = py::get(vm, &ev, "label")?;
                node = call(vm, this, &label, return_site)?;
                st(vm, this, "abnormal", Value::Bool(true))?;
                let a = py::get(vm, &ev, "args")?;
                store_set(vm, "_args", a)?;
                let k = py::get(vm, &ev, "kwargs")?;
                store_set(vm, "_kwargs", k)?;
            }
            Err(e) => return Err(e),
        }
        if gb(vm, this, "seen")? {
            let cur = g(vm, this, "current")?;
            let p = game(vm, "persistent")?;
            if !py::is_none(&p) {
                let se = g(vm, &p, "_seen_ever")?;
                if !py::is_none(&se) {
                    ops::setitem(vm, &se, cur.clone(), Value::Bool(true))?;
                }
            }
            let ss = game(vm, "seen_session")?;
            ops::setitem(vm, &ss, cur, Value::Bool(true))?;
        }
    }
    let log = game(vm, "log")?;
    if gb(vm, this, "rollback")? && py::truthy(vm, &log)? {
        vm.call_method(&log, "complete", Vec::new())?;
    }
    Ok(())
}

fn ctx_run(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let node = arg_or(args, kw, 1, "node", Value::None);
    run(vm, &args[0], node)?;
    Ok(Value::None)
}

/// `run_context(top)`: runs the current context until it returns,
/// restarting it on `RestartContext` (and on `RestartTopContext` at top).
pub fn run_context(vm: &mut Vm, top: bool) -> PyResult<Value> {
    let cc = py::config(vm, "context_callback")?;
    if !py::is_none(&cc) {
        vm.call(&cc, Vec::new(), Vec::new())?;
    }
    loop {
        let cf = game(vm, "context")?;
        let context = vm.call(&cf, Vec::new(), Vec::new())?;
        match run(vm, &context, Value::None) {
            Ok(()) => {
                let rv = store_get(vm, "_return")?;
                vm.call_method(&context, "pop_all_dynamic", Vec::new())?;
                return Ok(rv);
            }
            Err(e) if exc_isinstance(vm, &e, "renpy.game", "RestartContext") => continue,
            Err(e) if exc_isinstance(vm, &e, "renpy.game", "RestartTopContext") => {
                if top {
                    continue;
                }
                return Err(e);
            }
            Err(e) => {
                if !matches!(e, PyErr::Suspend(..)) {
                    vm.call_method(&context, "pop_all_dynamic", Vec::new())?;
                }
                return Err(e);
            }
        }
    }
}

fn f_run_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let top = py::truthy(vm, &args.first().cloned().unwrap_or(Value::Bool(false)))?;
    run_context(vm, top)
}

fn f_reset_all_contexts(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cf = game(vm, "context")?;
    let old = vm.call(&cf, Vec::new(), Vec::new())?;
    let nn = g(vm, &old, "next_node")?;
    if py::is_none(&nn) {
        return Err(py::error(
            vm,
            "The renpy.reset_all_contexts function can only be called as the last thing in a python statement.",
        ));
    }
    let contexts = game(vm, "contexts")?;
    while ops::len(vm, &contexts)? > 0 {
        let c = vm.call_method(&contexts, "pop", Vec::new())?;
        vm.call_method(&c, "pop_all_dynamic", Vec::new())?;
    }
    let ctx = py::import_attr(vm, MODULE, "Context")?;
    let c = vm.call(&ctx, vec![Value::Bool(true)], Vec::new())?;
    let name = g(vm, &nn, "name")?;
    vm.call_method(&c, "goto_label", vec![name])?;
    ops::list_append(vm, &contexts, c)?;
    Err(raise_new(vm, "renpy.game", "RestartTopContext", Vec::new()))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("check_infinite_loop", f_check_infinite_loop),
            ("not_infinite_loop", f_not_infinite_loop),
            ("run_context", f_run_context),
            ("reset_all_contexts", f_reset_all_contexts),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "il_statements", Value::Int(0));
    set(vm, "il_time", Value::Int(0));
    let obj = Value::Obj(vm.types.object.clone());
    let del = py::class(vm, MODULE, "Delete", &[obj.clone()], &[], &[])?;
    set(vm, "Delete", Value::Obj(del));
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let pi = py::class(vm, MODULE, "PredictInfo", &[object.clone()], &[], &[])?;
    set(vm, "PredictInfo", Value::Obj(pi));
    let node_p = vm.types.new_builtin("node", lle_node);
    let node_p = py::property(vm, node_p, None)?;
    let lle = py::class(
        vm,
        MODULE,
        "LineLogEntry",
        &[obj],
        &[
            ("__init__", lle_init),
            ("__eq__", lle_eq),
            ("__ne__", lle_ne),
        ],
        &[("node", node_p), ("__hash__", Value::None)],
    )?;
    set(vm, "LineLogEntry", Value::Obj(lle));
    let nosave = vm.types.new_list(vec![py::s(vm, "next_node")]);
    let ctx = py::class(
        vm,
        MODULE,
        "Context",
        &[object],
        &[
            ("__init__", ctx_init),
            ("__repr__", ctx_repr),
            ("after_upgrade", ctx_after_upgrade),
            ("replace_node", ctx_replace_node),
            ("make_dynamic", ctx_make_dynamic),
            ("pop_dynamic", ctx_pop_dynamic),
            ("pop_all_dynamic", ctx_pop_all_dynamic),
            ("pop_dynamic_roots", ctx_pop_dynamic_roots),
            ("goto_label", ctx_goto_label),
            ("check_stacks", ctx_check_stacks),
            ("report_traceback", ctx_report_traceback),
            ("come_from", ctx_come_from),
            ("run", ctx_run),
            ("mark_seen", ctx_mark_seen),
            ("call", ctx_call),
            ("pop_call", ctx_pop_call),
            ("lookup_return", ctx_lookup_return),
            ("rollback_copy", ctx_rollback_copy),
            ("predict_call", ctx_predict_call),
            ("predict_return", ctx_predict_return),
            ("predict", ctx_predict),
            ("seen_current", ctx_seen_current),
            ("do_deferred_rollback", ctx_do_deferred_rollback),
            ("get_return_stack", ctx_get_return_stack),
            ("set_return_stack", ctx_set_return_stack),
        ],
        &[
            ("__version__", Value::Int(16)),
            ("nosave", nosave),
            ("next_node", Value::None),
            ("force_checkpoint", Value::Bool(false)),
            ("come_from_name", Value::None),
            ("come_from_label", Value::None),
            ("temporary_attributes", Value::None),
            ("deferred_translate_identifier", Value::None),
            ("predict_return_stack", Value::None),
        ],
    )?;
    set(vm, "Context", Value::Obj(ctx));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
