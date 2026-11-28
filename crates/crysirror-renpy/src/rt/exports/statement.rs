// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.statementexports` (Ren'Py 8.3, MIT): imagemap, pause,
//! with, jump/call/return, call_screen and default statements.
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{PyErr, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};
use std::rc::Rc;

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter()
        .position(|(k, _)| &**k == n)
        .map(|i| kw.remove(i).1)
}

fn with_none_after(vm: &mut Vm, with_none: Value) -> PyResult<()> {
    let w = if py::is_none(&with_none) {
        py::config(vm, "implicit_with_none")?
    } else {
        with_none
    };
    if py::truthy(vm, &w)? {
        let i = iface(vm)?;
        vm.call_method(&i, "do_with", vec![Value::None, Value::None])?;
    }
    Ok(())
}

fn imagemap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut props: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut pos = args.iter().cloned();
    let mut p = |n: &str, props: &mut Vec<(Rc<str>, Value)>| pos.next().or_else(|| take(props, n));
    let ground = p("ground", &mut props).unwrap_or(Value::None);
    let selected = p("selected", &mut props).unwrap_or(Value::None);
    let hotspots = p("hotspots", &mut props).unwrap_or(Value::None);
    let unselected = p("unselected", &mut props).unwrap_or(Value::None);
    let overlays = p("overlays", &mut props).unwrap_or(Value::Bool(false));
    let style = p("style", &mut props).unwrap_or_else(|| py::s(vm, "imagemap"));
    let mouse = p("mouse", &mut props).unwrap_or_else(|| py::s(vm, "imagemap"));
    let with_none = p("with_none", &mut props).unwrap_or(Value::None);
    ex(vm, "mode", vec![py::s(vm, "imagemap")], Vec::new())?;
    let mut k = props;
    k.push(("unselected".into(), unselected));
    k.push(("style".into(), style));
    ui(
        vm,
        "imagemap_compat",
        vec![ground, selected, hotspots.clone()],
        k,
    )?;
    let mut rf = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    let mut results = Vec::new();
    for h in py::items(vm, &hotspots)? {
        let parts = py::items(vm, &h)?;
        if let Some(r) = parts.get(4) {
            results.push(r.clone());
        }
    }
    let results = vm.types.new_list(results);
    if !ops::contains(vm, &results, &rf)? {
        rf = Value::None;
    }
    let ifr = ex(vm, "in_fixed_rollback", Vec::new(), Vec::new())?;
    if py::truthy(vm, &ifr)? && py::config_bool(vm, "fix_rollback_without_choice")? {
        ui(vm, "saybehavior", Vec::new(), Vec::new())?;
    }
    let so = !py::truthy(vm, &overlays)?;
    let rv = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("suppress_overlay".into(), Value::Bool(so)),
            ("type".into(), py::s(vm, "imagemap")),
            ("mouse".into(), mouse),
            ("roll_forward".into(), rf),
        ],
    )?;
    ex(vm, "checkpoint", vec![rv.clone()], Vec::new())?;
    with_none_after(vm, with_none)?;
    Ok(rv)
}

fn is_bool_or_jump(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    if matches!(v, Value::Bool(_)) {
        return Ok(true);
    }
    for c in ["CallException", "JumpException"] {
        let cls = py::import_attr(vm, "renpy.game", c)?;
        if let Some(cls) = cls.obj() {
            if std::rc::Rc::ptr_eq(&crysirror_python::ops::type_of(vm, v), cls) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn pause(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut delay = arg_or(args, kw, 0, "delay", Value::None);
    let music = arg_or(args, kw, 1, "music", Value::None);
    let with_none = arg_or(args, kw, 2, "with_none", Value::None);
    let hard = kwb(vm, arg(args, kw, 3, "hard"), false)?;
    let predict = kwb(vm, arg(args, kw, 4, "predict"), false)?;
    let checkpoint = arg_or(args, kw, 5, "checkpoint", Value::None);
    let modal = arg_or(args, kw, 6, "modal", Value::None);
    let skipping = py::config(vm, "skipping")?;
    if skipping.as_str() == Some("fast") {
        return Ok(Value::Bool(false));
    }
    let checkpoint = if py::is_none(&checkpoint) {
        py::is_none(&delay)
    } else {
        py::truthy(vm, &checkpoint)?
    };
    let mut rf = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    if !is_bool_or_jump(vm, &rf)? {
        rf = Value::None;
    }
    let after_rollback = py::import_attr(vm, "renpy.game", "after_rollback")?;
    let after_rollback = py::truthy(vm, &after_rollback)?;
    let par = py::config_bool(vm, "pause_after_rollback")?;
    if !py::is_none(&delay) && after_rollback && !par {
        let rv = if py::is_none(&rf) {
            Value::Bool(false)
        } else {
            rf
        };
        if checkpoint {
            ex(
                vm,
                "checkpoint",
                vec![rv.clone()],
                vec![
                    ("keep_rollback".into(), Value::Bool(true)),
                    ("hard".into(), Value::Bool(false)),
                ],
            )?;
        }
        return Ok(rv);
    }
    ex(vm, "mode", vec![py::s(vm, "pause")], Vec::new())?;
    if !py::is_none(&music) {
        let gd = py::import_attr(vm, "renpy.audio.music", "get_delay")?;
        let nd = vm.call(&gd, vec![music], Vec::new())?;
        if !py::is_none(&nd) {
            delay = nd;
        }
    }
    if !py::is_none(&delay) && after_rollback && py::is_none(&rf) {
        delay = Value::Int(0);
    }
    let afm = if py::is_none(&delay) {
        py::s(vm, " ")
    } else {
        Value::None
    };
    if hard || !store_true(vm, "_dismiss_pause")? {
        ui(
            vm,
            "saybehavior",
            Vec::new(),
            vec![
                ("afm".into(), afm),
                ("dismiss".into(), py::s(vm, "dismiss_hard_pause")),
                ("dismiss_unfocused".into(), vm.types.new_list(Vec::new())),
            ],
        )?;
    } else {
        ui(vm, "saybehavior", Vec::new(), vec![("afm".into(), afm)])?;
    }
    if predict {
        let di = py::import_attr(vm, "renpy.display", "interface")?;
        py::set(vm, &di, "force_prediction", Value::Bool(true))?;
        let ppb = py::import_attr(vm, "renpy.display.behavior", "PredictPauseBehavior")?;
        let b = vm.call(&ppb, Vec::new(), Vec::new())?;
        ui(vm, "add", vec![b], Vec::new())?;
    }
    let r = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("mouse".into(), py::s(vm, "pause")),
            ("type".into(), py::s(vm, "pause")),
            ("roll_forward".into(), rf),
            ("pause".into(), delay.clone()),
            ("pause_modal".into(), modal),
        ],
    );
    let rv = match r {
        Ok(v) => v,
        Err(e) if is_jump_or_call(vm, &e) => exc_value(&e).unwrap(),
        Err(e) => return Err(e),
    };
    if checkpoint {
        let hard = par || py::is_none(&delay);
        ex(
            vm,
            "checkpoint",
            vec![rv.clone()],
            vec![
                ("keep_rollback".into(), Value::Bool(true)),
                ("hard".into(), Value::Bool(hard)),
            ],
        )?;
    }
    with_none_after(vm, with_none)?;
    if isinst(vm, &rv, "renpy.game", "JumpException")?
        || isinst(vm, &rv, "renpy.game", "CallException")?
    {
        return Err(PyErr::Exc(rv.obj().cloned().unwrap()));
    }
    Ok(rv)
}

fn with_statement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut trans = arg_or(args, kw, 0, "trans", Value::None);
    let always = kwb(vm, arg(args, kw, 1, "always"), false)?;
    let paired = arg_or(args, kw, 2, "paired", Value::None);
    let clear = arg_or(args, kw, 3, "clear", Value::Bool(true));
    let c = ctx(vm)?;
    let ip = py::get_or_none(vm, &c, "init_phase")?;
    if py::truthy(vm, &ip)? {
        return Err(py::error(
            vm,
            "With statements may not run while in init phase.",
        ));
    }
    let skipping = py::config(vm, "skipping")?;
    if py::truthy(vm, &skipping)? {
        trans = Value::None;
    }
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    let t = py::get_or_none(vm, &prefs, "transitions")?;
    if !(py::truthy(vm, &t)? || always) {
        trans = Value::None;
    }
    ex(vm, "mode", vec![py::s(vm, "with")], Vec::new())?;
    let dict = vm.types.dict.clone();
    if ops::isinstance(vm, &trans, &dict) {
        for (k, v) in ops::mapping_items(vm, &trans)? {
            if py::is_none(&k) {
                continue;
            }
            ex(vm, "transition", vec![v], vec![("layer".into(), k)])?;
        }
        if !ops::contains(vm, &trans, &Value::None)? {
            return Ok(Value::None);
        }
        trans = ops::getitem(vm, &trans, Value::None)?;
    }
    let i = iface(vm)?;
    py::method_kw(
        vm,
        &i,
        "do_with",
        vec![trans, paired],
        vec![("clear".into(), clear)],
    )
}

fn jump(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Err(raise(
        vm,
        "renpy.game",
        "JumpException",
        vec![args[0].clone()],
    ))
}

fn call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let from_current = take(&mut kw, "from_current").unwrap_or(Value::Bool(false));
    let a = vm.types.new_tuple(args[1..].to_vec());
    let k = py::kwargs_dict(vm, &kw)?;
    let c = py::import_attr(vm, "renpy.game", "CallException")?;
    let e = vm.call(
        &c,
        vec![args[0].clone(), a, k],
        vec![("from_current".into(), from_current)],
    )?;
    Err(PyErr::Exc(e.obj().cloned().unwrap()))
}

fn return_statement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "value", Value::None);
    store_set(vm, "_return", v)?;
    Err(raise(
        vm,
        "renpy.game",
        "JumpException",
        vec![py::s(vm, "_renpy_return")],
    ))
}

fn call_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let name = args[0].clone();
    let mode = take(&mut kw, "_mode").unwrap_or_else(|| py::s(vm, "screen"));
    ex(vm, "mode", vec![mode], Vec::new())?;
    let with_none = match take(&mut kw, "_with_none") {
        Some(v) => v,
        None => py::config(vm, "implicit_with_none")?,
    };
    let mut k = kw;
    k.push(("_transient".into(), Value::Bool(true)));
    ex(vm, "show_screen", args.to_vec(), k)?;
    let mut rf = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    let grf = py::import_attr(vm, "renpy.display.screen", "get_screen_roll_forward")?;
    let mut can = vm.call(&grf, vec![name], Vec::new())?;
    if py::is_none(&can) {
        can = py::config(vm, "call_screen_roll_forward")?;
    }
    if !py::truthy(vm, &can)? {
        rf = Value::None;
    }
    let r = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("mouse".into(), py::s(vm, "screen")),
            ("type".into(), py::s(vm, "screen")),
            ("roll_forward".into(), rf),
        ],
    );
    let rv = match r {
        Ok(v) => v,
        Err(e) if is_jump_or_call(vm, &e) => exc_value(&e).unwrap(),
        Err(e) => return Err(e),
    };
    ex(vm, "checkpoint", vec![rv.clone()], Vec::new())?;
    if py::truthy(vm, &with_none)? {
        let i = iface(vm)?;
        vm.call_method(&i, "do_with", vec![Value::None, Value::None])?;
    }
    if isinst(vm, &rv, "renpy.game", "JumpException")?
        || isinst(vm, &rv, "renpy.game", "CallException")?
    {
        return Err(PyErr::Exc(rv.obj().cloned().unwrap()));
    }
    Ok(rv)
}

fn execute_default_statement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let start = kwb(vm, arg(args, kw, 0, "start"), false)?;
    let ds = py::import_attr(vm, "renpy.ast", "default_statements")?;
    for h in py::items(vm, &ds)? {
        match super::super::nodes::id_of(&h) {
            Some(id) => super::super::astexec::execute_default(vm, id, start)?,
            None => {
                vm.call_method(&h, "execute_default", vec![Value::Bool(start)])?;
            }
        }
    }
    let cbs = py::config(vm, "after_default_callbacks")?;
    for cb in py::items(vm, &cbs)? {
        vm.call(&cb, Vec::new(), Vec::new())?;
    }
    // 8.6 runs config.after_init_callbacks once, after the first defaults.
    if super::super::state::rt(vm).profile.at_least(8, 6) {
        let exports = py::import(vm, "renpy.exports")?;
        let ran = py::get(vm, &exports, "_ran_after_init_callbacks")?;
        if !py::truthy(vm, &ran)? {
            let aic = py::config(vm, "after_init_callbacks")?;
            for cb in py::items(vm, &aic)? {
                vm.call(&cb, Vec::new(), Vec::new())?;
            }
        }
        py::set(vm, &exports, "_ran_after_init_callbacks", Value::Bool(true))?;
    }
    Ok(Value::None)
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("imagemap", imagemap),
        ("pause", pause),
        ("with_statement", with_statement),
        ("with", with_statement),
        ("jump", jump),
        ("call", call),
        ("return_statement", return_statement),
        ("call_screen", call_screen),
        ("execute_default_statement", execute_default_statement),
    ]
}
