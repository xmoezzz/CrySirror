// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.predict`, after Ren'Py 8.3 `predict.py` (MIT): the
//! predicted-displayable set, deferred screen predictions and the
//! prediction coroutine the interface advances between frames.
//!
//! Upstream's coroutine is a Python generator; here it is a native object
//! with the same `send`/`__next__` protocol and the same yield points, so
//! the interface spreads prediction over frames identically. Prediction
//! only starts image work early on the shared image service; it never
//! blocks or changes what is shown.
use super::py;
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, rc::Rc};

const MODULE: &str = "renpy.display.predict";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn set_predicting(vm: &mut Vm, v: bool) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, "predicting", Value::Bool(v))
}

pub fn displayable(vm: &mut Vm, d: &Value) -> PyResult<()> {
    if py::is_none(d) {
        return Ok(());
    }
    let predicted = mget(vm, "predicted")?;
    if ops::set_contains(vm, &predicted, d)? {
        return Ok(());
    }
    ops::set_add(vm, &predicted, d.clone())?;
    let cb = py::builtin(vm, "predict_one", visit_predict_one);
    vm.call_method(d, "visit_all", vec![cb])?;
    Ok(())
}

fn visit_predict_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "predict_one", Vec::new())
}

fn visit_predict_one_action(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "predict_one_action", Vec::new())
}

fn f_displayable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    displayable(vm, &args.first().cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

fn f_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let screens = mget(vm, "screens")?;
    let a = vm.types.new_tuple(args[1..].to_vec());
    let k = py::kwargs_dict(vm, kw)?;
    let t = vm.types.new_tuple(vec![args[0].clone(), a, k]);
    ops::list_append(vm, &screens, t)?;
    Ok(Value::None)
}

fn f_reset(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, MODULE)?;
    let cache = py::import_attr(vm, "renpy.display.im", "cache")?;
    let gt = py::get_or_none(vm, &cache, "get_texture")?;
    py::set(vm, &m, "image", gt)?;
    let predicted = mget(vm, "predicted")?;
    vm.call_method(&predicted, "clear", Vec::new())?;
    let screens = mget(vm, "screens")?;
    vm.call_method(&screens, "clear", Vec::new())?;
    Ok(Value::None)
}

fn debug_prediction(vm: &mut Vm) -> bool {
    py::config_bool(vm, "debug_prediction").unwrap_or(false)
}

fn soft(vm: &mut Vm, r: PyResult<()>) -> PyResult<()> {
    match r {
        Err(e) if matches!(e, PyErr::Suspend(..)) || debug_prediction(vm) => Err(e),
        _ => Ok(()),
    }
}

// The coroutine ----------------------------------------------------------------------------------------

enum Phase {
    Start,
    PredictSet {
        items: Vec<Value>,
        i: usize,
    },
    Context {
        iter: Option<Value>,
    },
    Layers,
    WaitExpensive,
    Expensive {
        callbacks: Vec<Value>,
        i: usize,
    },
    Screens {
        items: Vec<(Value, Value, Value)>,
        i: usize,
    },
    Actions,
    Queue {
        i: usize,
        waiting: bool,
    },
    Final,
    Done,
}

struct Coroutine {
    root: Value,
    phase: Phase,
    predicted_screens: Vec<Value>,
}

struct CoRef(Rc<RefCell<Coroutine>>);

fn co_of(vm: &Vm, v: &Value) -> PyResult<Rc<RefCell<Coroutine>>> {
    py::native::<CoRef, _>(v, |c| c.0.clone())
        .ok_or_else(|| vm.type_error("prediction coroutine expected"))
}

fn take_phase(co: &Rc<RefCell<Coroutine>>) -> Phase {
    std::mem::replace(&mut co.borrow_mut().phase, Phase::Done)
}

fn set_phase(co: &Rc<RefCell<Coroutine>>, p: Phase) {
    co.borrow_mut().phase = p;
}

/// Runs the coroutine to its next yield; `None` means it finished.
fn resume(vm: &mut Vm, co: &Rc<RefCell<Coroutine>>, sent: &Value) -> PyResult<Option<Value>> {
    let mut sent_truthy = py::truthy(vm, sent)?;
    loop {
        match take_phase(co) {
            Phase::Start => {
                let cache = py::import_attr(vm, "renpy.display.im", "cache")?;
                if let Some(f) = py::get_opt(vm, &cache, "start_prediction")? {
                    vm.call(&f, Vec::new(), Vec::new())?;
                }
                let set = py::import_attr(vm, "store", "_predict_set").unwrap_or(Value::None);
                let items = if py::is_none(&set) {
                    Vec::new()
                } else {
                    ops::collect(vm, &set)?
                };
                set_phase(co, Phase::PredictSet { items, i: 0 });
                return Ok(Some(Value::Bool(true)));
            }
            Phase::PredictSet { items, i } => {
                let m = py::import(vm, MODULE)?;
                let cache = py::import_attr(vm, "renpy.display.im", "cache")?;
                let pi = py::get_or_none(vm, &cache, "preload_image")?;
                py::set(vm, &m, "image", pi)?;
                if let Some(d) = items.get(i).cloned() {
                    set_predicting(vm, true)?;
                    let r = displayable(vm, &d);
                    soft(vm, r)?;
                    set_predicting(vm, false)?;
                    set_phase(co, Phase::PredictSet { items, i: i + 1 });
                    return Ok(Some(Value::Bool(true)));
                }
                set_phase(co, Phase::Context { iter: None });
            }
            Phase::Context { iter } => {
                set_predicting(vm, true)?;
                let iter = match iter {
                    Some(it) => it,
                    None => {
                        let ctx = py::import_attr(vm, "renpy.game", "context")?;
                        let c = vm.call(&ctx, Vec::new(), Vec::new())?;
                        let p = vm.call_method(&c, "predict", Vec::new())?;
                        ops::iter(vm, &p)?
                    }
                };
                match ops::next(vm, &iter)? {
                    Some(_) => {
                        set_predicting(vm, false)?;
                        set_phase(co, Phase::Context { iter: Some(iter) });
                        return Ok(Some(Value::Bool(true)));
                    }
                    None => set_phase(co, Phase::Layers),
                }
            }
            Phase::Layers => {
                let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
                if ops::len(vm, &contexts)? >= 2 {
                    let c = ops::getitem(vm, &contexts, Value::Int(-2))?;
                    let sls = py::get(vm, &c, "scene_lists")?;
                    let layers = py::get(vm, &sls, "layers")?;
                    let vals = vm.call_method(&layers, "values", Vec::new())?;
                    for l in ops::collect(vm, &vals)? {
                        for sle in ops::collect(vm, &l)? {
                            let d = py::get_or_none(vm, &sle, "displayable")?;
                            let _ = displayable(vm, &d);
                        }
                    }
                } else {
                    let cbs = py::config(vm, "predict_callbacks")?;
                    for cb in py::items(vm, &cbs)? {
                        vm.call(&cb, Vec::new(), Vec::new())?;
                    }
                }
                set_predicting(vm, false)?;
                set_phase(co, Phase::WaitExpensive);
                return Ok(Some(Value::Bool(false)));
            }
            Phase::WaitExpensive => {
                if !sent_truthy {
                    set_phase(co, Phase::WaitExpensive);
                    return Ok(Some(Value::Bool(false)));
                }
                set_predicting(vm, true)?;
                let cbs = py::config(vm, "expensive_predict_callbacks")?;
                let callbacks = py::items(vm, &cbs)?;
                set_phase(co, Phase::Expensive { callbacks, i: 0 });
            }
            Phase::Expensive { callbacks, i } => {
                set_predicting(vm, true)?;
                if let Some(cb) = callbacks.get(i).cloned() {
                    let more = vm.call(&cb, Vec::new(), Vec::new())?;
                    let next = if py::truthy(vm, &more)? { i } else { i + 1 };
                    set_predicting(vm, false)?;
                    set_phase(co, Phase::Expensive { callbacks, i: next });
                    return Ok(Some(Value::Bool(false)));
                }
                let ps = py::import_attr(vm, "store", "_predict_screen").unwrap_or(Value::None);
                let mut items = Vec::new();
                if !py::is_none(&ps) {
                    for (name, value) in ops::mapping_items(vm, &ps)? {
                        let a = ops::getitem(vm, &value, Value::Int(0))?;
                        let k = ops::getitem(vm, &value, Value::Int(1))?;
                        items.push((name, a, k));
                    }
                }
                set_phase(co, Phase::Screens { items, i: 0 });
            }
            Phase::Screens { items, i } => {
                if let Some((name, a, k)) = items.get(i).cloned() {
                    set_predicting(vm, true)?;
                    let t = vm.types.new_tuple(vec![name.clone(), a.clone(), k.clone()]);
                    co.borrow_mut().predicted_screens.push(t);
                    predict_screen(vm, &name, &a, &k)?;
                    set_predicting(vm, false)?;
                    set_phase(co, Phase::Screens { items, i: i + 1 });
                    return Ok(Some(Value::Bool(false)));
                }
                set_phase(co, Phase::Actions);
            }
            Phase::Actions => {
                set_predicting(vm, true)?;
                let root = co.borrow().root.clone();
                let cb = py::builtin(vm, "predict_one_action", visit_predict_one_action);
                let r = vm.call_method(&root, "visit_all", vec![cb]).map(|_| ());
                soft(vm, r)?;
                set_predicting(vm, false)?;
                set_phase(
                    co,
                    Phase::Queue {
                        i: 0,
                        waiting: true,
                    },
                );
            }
            Phase::Queue { i, waiting } => {
                let screens = mget(vm, "screens")?;
                if i >= ops::len(vm, &screens)? {
                    set_phase(co, Phase::Final);
                    continue;
                }
                if waiting {
                    // `while not (yield False): continue`
                    set_phase(co, Phase::Queue { i, waiting: false });
                    return Ok(Some(Value::Bool(false)));
                }
                if !sent_truthy {
                    set_phase(co, Phase::Queue { i, waiting: false });
                    return Ok(Some(Value::Bool(false)));
                }
                let t = ops::getitem(vm, &screens, Value::Int(i as i64))?;
                let mut seen = false;
                for p in co.borrow().predicted_screens.clone() {
                    if ops::eq(vm, &p, &t)? {
                        seen = true;
                        break;
                    }
                }
                if !seen {
                    co.borrow_mut().predicted_screens.push(t.clone());
                    let name = ops::getitem(vm, &t, Value::Int(0))?;
                    if !py::string(vm, &name)?.starts_with('_') {
                        let a = ops::getitem(vm, &t, Value::Int(1))?;
                        let k = ops::getitem(vm, &t, Value::Int(2))?;
                        set_predicting(vm, true)?;
                        predict_screen(vm, &name, &a, &k)?;
                        set_predicting(vm, false)?;
                    }
                }
                set_phase(
                    co,
                    Phase::Queue {
                        i: i + 1,
                        waiting: true,
                    },
                );
                sent_truthy = false;
            }
            Phase::Final => {
                set_phase(co, Phase::Done);
                return Ok(Some(Value::None));
            }
            Phase::Done => return Ok(None),
        }
    }
}

fn predict_screen(vm: &mut Vm, name: &Value, a: &Value, k: &Value) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.display.screen", "predict_screen")?;
    let mut args = vec![name.clone()];
    args.extend(py::items(vm, a)?);
    let kw = py::str_items(vm, k)?;
    vm.call(&f, args, kw)?;
    Ok(())
}

fn co_send(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let co = co_of(vm, &args[0])?;
    let v = args.get(1).cloned().unwrap_or(Value::None);
    match resume(vm, &co, &v)? {
        Some(v) => Ok(v),
        None => Err(vm.exc_msg("StopIteration", "")),
    }
}

fn co_next(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    co_send(vm, &[args[0].clone(), Value::None], &[])
}

fn co_iter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn co_close(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let co = co_of(vm, &args[0])?;
    set_phase(&co, Phase::Done);
    set_predicting(vm, false)?;
    Ok(Value::None)
}

fn f_prediction_coroutine(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = mget(vm, "PredictionCoroutine")?;
    let c = c
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("PredictionCoroutine"))?;
    let co = Coroutine {
        root: args.first().cloned().unwrap_or(Value::None),
        phase: Phase::Start,
        predicted_screens: Vec::new(),
    };
    Ok(vm
        .types
        .new_native(&c, Box::new(CoRef(Rc::new(RefCell::new(co))))))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("displayable", f_displayable),
            ("screen", f_screen),
            ("reset", f_reset),
            ("prediction_coroutine", f_prediction_coroutine),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "image", Value::None);
    set(vm, "predicted", vm.types.new_set(Default::default()));
    set(vm, "predicting", Value::Bool(false));
    set(vm, "screens", vm.types.new_list(Vec::new()));
    let c = vm.types.new_builtin_type(
        "PredictionCoroutine",
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Native,
        true,
    );
    for (n, f) in [
        ("send", co_send as crysirror_python::object::NativeFn),
        ("__next__", co_next),
        ("next", co_next),
        ("__iter__", co_iter),
        ("close", co_close),
    ] {
        vm.types.add_method(&c, n, f);
    }
    let mn = vm.types.new_str(MODULE);
    vm.types.set_attr(&c, "__module__", mn);
    set(vm, "PredictionCoroutine", Value::Obj(c));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
