// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports.menuexports` (Ren'Py 8.3, MIT): menu filtering and
//! display through the `choice` screen (or the screenless menu).
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};
use std::rc::Rc;

fn exports_module(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, "renpy.exports")
}

fn get_menu_args(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = exports_module(vm)?;
    let a = py::get_or_none(vm, &m, "menu_args")?;
    if py::is_none(&a) {
        let t = vm.types.new_tuple(Vec::new());
        let d = vm.types.new_dict();
        return Ok(vm.types.new_tuple(vec![t, d]));
    }
    let k = py::get_or_none(vm, &m, "menu_kwargs")?;
    Ok(vm.types.new_tuple(vec![a, k]))
}

fn take(kw: &mut Vec<(Rc<str>, Value)>, n: &str) -> Option<Value> {
    kw.iter()
        .position(|(k, _)| &**k == n)
        .map(|i| kw.remove(i).1)
}

fn substitute(vm: &mut Vm, s: Value) -> PyResult<Value> {
    if py::config_bool(vm, "old_substitutions")? {
        let tqd = py::import_attr(vm, "renpy.exports", "tag_quoting_dict")?;
        return ops::binary_op(
            vm,
            &s,
            &tqd,
            crysirror_python::ast::BinOp::Mod,
            false,
            false,
        );
    }
    Ok(s)
}

fn eval(vm: &mut Vm, e: &Value) -> PyResult<Value> {
    super::super::python::eval(vm, e, &Value::None, &Value::None)
}

fn menu(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let items = py::items(vm, &args[0])?;
    let set_expr = arg_or(args, kw, 1, "set_expr", Value::None);
    let margs = arg_or(args, kw, 2, "args", Value::None);
    let mkw = arg_or(args, kw, 3, "kwargs", Value::None);
    let item_arguments = arg_or(args, kw, 4, "item_arguments", Value::None);
    let mut a = if py::truthy(vm, &margs)? {
        py::items(vm, &margs)?
    } else {
        Vec::new()
    };
    let mut k = if py::truthy(vm, &mkw)? {
        py::str_items(vm, &mkw)?
    } else {
        Vec::new()
    };
    let nvl = take(&mut k, "nvl").unwrap_or(Value::Bool(false));
    let mac = py::config(vm, "menu_arguments_callback")?;
    if !py::is_none(&mac) {
        let r = vm.call(&mac, a.clone(), k.clone())?;
        let p = py::items(vm, &r)?;
        a = py::items(vm, &p[0])?;
        k = py::str_items(vm, &p[1])?;
    }
    let mut ia: Vec<(Value, Value)> = if py::is_none(&item_arguments) {
        let mut v = Vec::new();
        for _ in &items {
            v.push((vm.types.new_tuple(Vec::new()), vm.types.new_dict()));
        }
        v
    } else {
        let mut v = Vec::new();
        for x in py::items(vm, &item_arguments)? {
            let p = py::items(vm, &x)?;
            v.push((p[0].clone(), p[1].clone()));
        }
        v
    };
    let mut items = items;
    let mut set = Value::None;
    if py::truthy(vm, &set_expr)? {
        set = eval(vm, &set_expr)?;
        let mut ni = Vec::new();
        let mut nia = Vec::new();
        for (i, x) in items.iter().zip(ia.iter()) {
            let label = ops::getitem(vm, i, Value::Int(0))?;
            if !ops::contains(vm, &set, &label)? {
                ni.push(i.clone());
                nia.push(x.clone());
            }
        }
        items = ni;
        ia = nia;
    }
    let mut new_items = Vec::new();
    if py::config_bool(vm, "menu_actions")? {
        let c = ctx(vm)?;
        let location = py::get(vm, &c, "current")?;
        let include_disabled = py::config_bool(vm, "menu_include_disabled")?;
        for (it, (iargs, ikw)) in items.iter().zip(ia.iter()) {
            let p = py::items(vm, it)?;
            let label = substitute(vm, p[0].clone())?;
            let condition = eval(vm, &p[1])?;
            let cond = py::truthy(vm, &condition)?;
            if !include_disabled && !cond {
                continue;
            }
            if !py::is_none(&p[2]) {
                let cr = py::import_attr(vm, "renpy.ui", "ChoiceReturn")?;
                let action = vm.call(
                    &cr,
                    vec![label.clone(), p[2].clone(), location.clone()],
                    vec![
                        ("sensitive".into(), condition),
                        ("args".into(), iargs.clone()),
                        ("kwargs".into(), ikw.clone()),
                    ],
                )?;
                new_items.push(vm.types.new_tuple(vec![label, action]));
            } else {
                new_items.push(vm.types.new_tuple(vec![label, Value::None]));
            }
        }
    } else {
        for it in &items {
            let p = py::items(vm, it)?;
            let c = eval(vm, &p[1])?;
            if py::truthy(vm, &c)? {
                let label = substitute(vm, p[0].clone())?;
                new_items.push(vm.types.new_tuple(vec![label, p[2].clone()]));
            }
        }
    }
    let mut any = false;
    for it in &new_items {
        let v = ops::getitem(vm, it, Value::Int(1))?;
        if !py::is_none(&v) {
            any = true;
        }
    }
    if !any {
        return Ok(Value::None);
    }
    let m = exports_module(vm)?;
    let old_a = py::get_or_none(vm, &m, "menu_args")?;
    let old_k = py::get_or_none(vm, &m, "menu_kwargs")?;
    py::set(vm, &m, "menu_args", vm.types.new_tuple(a))?;
    let kd = py::kwargs_dict(vm, &k)?;
    py::set(vm, &m, "menu_kwargs", kd)?;
    let store = py::import(vm, "store")?;
    let f = if py::truthy(vm, &nvl)? {
        py::get(vm, &store, "nvl_menu")?
    } else {
        py::get(vm, &store, "menu")?
    };
    let r = vm.call(&f, vec![vm.types.new_list(new_items)], Vec::new());
    py::set(vm, &m, "menu_args", old_a)?;
    py::set(vm, &m, "menu_kwargs", old_k)?;
    let rv = r?;
    if !py::is_none(&set) && !py::is_none(&rv) {
        for it in &items {
            let p = py::items(vm, it)?;
            if ops::eq(vm, &p[2], &rv)? {
                if vm.call_method(&set, "append", vec![p[0].clone()]).is_err() {
                    vm.call_method(&set, "add", vec![p[0].clone()])?;
                }
            }
        }
    }
    Ok(rv)
}

fn choice_for_skipping(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let skipping = py::config(vm, "skipping")?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    let sac = py::get_or_none(vm, &prefs, "skip_after_choices")?;
    if py::truthy(vm, &skipping)? && !py::truthy(vm, &sac)? {
        config_set(vm, "skipping", Value::None)?;
    }
    let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
    if py::config_bool(vm, "autosave_on_choice")? && !py::truthy(vm, &ar)? {
        let fa = py::import_attr(vm, "renpy.loadsave", "force_autosave")?;
        vm.call(&fa, vec![Value::Bool(true)], Vec::new())?;
    }
    Ok(Value::None)
}

fn predict_menu(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "choice_screen_chosen")? {
        return Ok(Value::None);
    }
    let item = vm.types.new_tuple(vec![
        py::s(vm, "Menu Prediction"),
        Value::Bool(true),
        Value::Bool(false),
    ]);
    let items = vm.types.new_list(vec![item]);
    ex(
        vm,
        "predict_screen",
        vec![py::s(vm, "choice")],
        vec![("items".into(), items)],
    )
}

fn display_menu(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let items = args
        .first()
        .cloned()
        .or_else(|| take(&mut kw, "items"))
        .unwrap_or(Value::None);
    let mut opt = |n: &str, i: usize, d: Value, kw: &mut Vec<(Rc<str>, Value)>| {
        args.get(i).cloned().or_else(|| take(kw, n)).unwrap_or(d)
    };
    let window_style = opt("window_style", 1, py::s(vm, "menu_window"), &mut kw);
    let interact_v = opt("interact", 2, Value::Bool(true), &mut kw);
    let mut with_none = opt("with_none", 3, Value::None, &mut kw);
    let caption_style = opt("caption_style", 4, py::s(vm, "menu_caption"), &mut kw);
    let choice_style = opt("choice_style", 5, py::s(vm, "menu_choice"), &mut kw);
    let choice_chosen_style = opt(
        "choice_chosen_style",
        6,
        py::s(vm, "menu_choice_chosen"),
        &mut kw,
    );
    let choice_button_style = opt(
        "choice_button_style",
        7,
        py::s(vm, "menu_choice_button"),
        &mut kw,
    );
    let choice_chosen_button_style = opt(
        "choice_chosen_button_style",
        8,
        py::s(vm, "menu_choice_chosen_button"),
        &mut kw,
    );
    let scope = opt("scope", 9, vm.types.new_tuple(Vec::new()), &mut kw);
    let widget_properties = opt("widget_properties", 10, Value::None, &mut kw);
    let mut screen = opt("screen", 11, py::s(vm, "choice"), &mut kw);
    let ty = opt("type", 12, py::s(vm, "menu"), &mut kw);
    let predict_only = opt("predict_only", 13, Value::Bool(false), &mut kw);
    let layer_arg = opt("_layer", 14, Value::None, &mut kw);
    let a_arg = opt("_args", 15, Value::None, &mut kw);
    let k_arg = opt("_kwargs", 16, Value::None, &mut kw);
    let interact = py::truthy(vm, &interact_v)?;
    let (menu_args, mut menu_kwargs) = if !py::is_none(&a_arg) || !py::is_none(&k_arg) {
        let mut a = if py::truthy(vm, &a_arg)? {
            py::items(vm, &a_arg)?
        } else {
            Vec::new()
        };
        let mut k = if py::truthy(vm, &k_arg)? {
            py::str_items(vm, &k_arg)?
        } else {
            Vec::new()
        };
        let mac = py::config(vm, "menu_arguments_callback")?;
        if !py::is_none(&mac) {
            let r = vm.call(&mac, a.clone(), k.clone())?;
            let p = py::items(vm, &r)?;
            a = py::items(vm, &p[0])?;
            k = py::str_items(vm, &p[1])?;
        }
        (a, k)
    } else {
        let r = get_menu_args(vm, &[], &[])?;
        let p = py::items(vm, &r)?;
        (py::items(vm, &p[0])?, py::str_items(vm, &p[1])?)
    };
    if let Some(s) = take(&mut menu_kwargs, "screen") {
        screen = s;
    }
    if let Some(w) = take(&mut menu_kwargs, "_with_none") {
        with_none = w;
    }
    let mode = take(&mut menu_kwargs, "_mode").unwrap_or_else(|| ty.clone());
    let mut layer = take(&mut menu_kwargs, "_layer").unwrap_or(layer_arg);
    if interact {
        ex(vm, "mode", vec![mode], Vec::new())?;
        choice_for_skipping(vm, &[], &[])?;
        if !py::truthy(vm, &predict_only)? {
            let cew = py::config(vm, "choice_empty_window")?;
            let c = ctx(vm)?;
            let sl = py::get(vm, &c, "scene_lists")?;
            let sw = py::get_or_none(vm, &sl, "shown_window")?;
            if py::truthy(vm, &cew)? && !py::truthy(vm, &sw)? {
                vm.call(
                    &cew,
                    vec![py::s(vm, "")],
                    vec![("interact".into(), Value::Bool(false))],
                )?;
            }
        }
    }
    let items_l = py::items(vm, &items)?;
    let mut choices = Vec::new();
    for it in &items_l {
        let mut val = ops::getitem(vm, it, Value::Int(1))?;
        if isinst(vm, &val, "renpy.ui", "ChoiceReturn")? {
            val = py::get(vm, &val, "value")?;
        }
        if !py::is_none(&val) {
            choices.push(val);
        }
    }
    let choices = vm.types.new_list(choices);
    let mut rf = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    if !ops::contains(vm, &choices, &rf)? {
        rf = Value::None;
    }
    let acd = py::config(vm, "auto_choice_delay")?;
    if py::truthy(vm, &acd)? {
        let rng = py::import_attr(vm, "renpy.exports", "random")?;
        let ch = vm.call_method(&rng, "choice", vec![choices.clone()])?;
        ui(vm, "pausebehavior", vec![acd, ch], Vec::new())?;
    }
    let c = ctx(vm)?;
    let location = py::get(vm, &c, "current")?;
    let ifr = ex(vm, "in_fixed_rollback", Vec::new(), Vec::new())?;
    if py::truthy(vm, &ifr)? && py::config_bool(vm, "fix_rollback_without_choice")? {
        ui(vm, "saybehavior", Vec::new(), Vec::new())?;
    }
    let dict_t = Value::Obj(vm.types.dict.clone());
    let scope = vm.call(&dict_t, vec![scope], Vec::new())?;
    for (k, v) in &menu_kwargs {
        py::dict_set(vm, &scope, k, v.clone())?;
    }
    let hs = ex(vm, "has_screen", vec![screen.clone()], Vec::new())?;
    if py::truthy(vm, &hs)? {
        let mut item_actions = Vec::new();
        let props = if py::is_none(&widget_properties) {
            vm.types.new_dict()
        } else {
            widget_properties
        };
        let cr = py::import_attr(vm, "renpy.ui", "ChoiceReturn")?;
        let me_c = py::import_attr(vm, "renpy.exports", "MenuEntry")?;
        let chosen_cfg = py::config_bool(vm, "choice_screen_chosen")?;
        for it in &items_l {
            let p = py::items(vm, it)?;
            let label = p[0].clone();
            let mut value = p[1].clone();
            if !py::truthy(vm, &label)? {
                value = Value::None;
            }
            let (action, chosen, iargs, ikw) = if isinst(vm, &value, "renpy.ui", "Choice")? {
                let v = py::get(vm, &value, "value")?;
                let action = vm.call(&cr, vec![label.clone(), v, location.clone()], Vec::new())?;
                let chosen = vm.call_method(&action, "get_chosen", Vec::new())?;
                (
                    action,
                    chosen,
                    py::get(vm, &value, "args")?,
                    py::get(vm, &value, "kwargs")?,
                )
            } else if isinst(vm, &value, "renpy.ui", "ChoiceReturn")? {
                let chosen = vm.call_method(&value, "get_chosen", Vec::new())?;
                let a = py::get(vm, &value, "args")?;
                let k = py::get(vm, &value, "kwargs")?;
                (value.clone(), chosen, a, k)
            } else if !py::is_none(&value) {
                let action = vm.call(
                    &cr,
                    vec![label.clone(), value.clone(), location.clone()],
                    Vec::new(),
                )?;
                let chosen = vm.call_method(&action, "get_chosen", Vec::new())?;
                (
                    action,
                    chosen,
                    vm.types.new_tuple(Vec::new()),
                    vm.types.new_dict(),
                )
            } else {
                (
                    Value::None,
                    Value::Bool(false),
                    vm.types.new_tuple(Vec::new()),
                    vm.types.new_dict(),
                )
            };
            let t = if chosen_cfg {
                vm.types
                    .new_tuple(vec![label.clone(), action.clone(), chosen.clone()])
            } else {
                vm.types.new_tuple(vec![label.clone(), action.clone()])
            };
            let me = vm.call(&me_c, vec![t], Vec::new())?;
            py::set(vm, &me, "caption", label)?;
            py::set(vm, &me, "action", action)?;
            py::set(vm, &me, "chosen", chosen)?;
            py::set(vm, &me, "args", iargs)?;
            py::set(vm, &me, "kwargs", ikw)?;
            item_actions.push(me);
        }
        if py::is_none(&layer) {
            layer = if ty.as_str() == Some("nvl") {
                py::config(vm, "nvl_choice_layer")?
            } else {
                py::config(vm, "choice_layer")?
            };
        }
        let mut a = vec![screen.clone()];
        a.extend(menu_args);
        let mut k = py::str_items(vm, &scope)?;
        k.push(("items".into(), vm.types.new_list(item_actions)));
        k.push(("_widget_properties".into(), props));
        k.push(("_transient".into(), Value::Bool(true)));
        k.push(("_layer".into(), layer));
        ex(vm, "show_screen", a, k)?;
    } else {
        ex(vm, "shown_window", Vec::new(), Vec::new())?;
        ui(
            vm,
            "window",
            Vec::new(),
            vec![
                ("style".into(), window_style),
                ("focus".into(), py::s(vm, "menu")),
            ],
        )?;
        let mut k = kw.clone();
        k.extend([
            ("location".into(), location.clone()),
            ("focus".into(), py::s(vm, "choices")),
            ("default".into(), Value::Bool(true)),
            ("caption_style".into(), caption_style),
            ("choice_style".into(), choice_style),
            ("choice_chosen_style".into(), choice_chosen_style),
            ("choice_button_style".into(), choice_button_style),
            (
                "choice_chosen_button_style".into(),
                choice_chosen_button_style,
            ),
        ]);
        ui(vm, "menu", vec![items.clone()], k)?;
    }
    if py::config_bool(vm, "menu_showed_window")? {
        ex(vm, "shown_window", Vec::new(), Vec::new())?;
    }
    let log = |vm: &mut Vm, s: String| -> PyResult<()> {
        ex(vm, "log", vec![py::s(vm, &s)], Vec::new())?;
        Ok(())
    };
    for it in &items_l {
        let p = py::items(vm, it)?;
        let l = py::string(vm, &p[0]).unwrap_or_default();
        if !py::is_none(&p[1]) {
            log(vm, format!("Choice: {l}"))?;
        } else {
            log(vm, l)?;
        }
    }
    log(vm, String::new())?;
    if !interact {
        return Ok(Value::None);
    }
    let rv = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("mouse".into(), py::s(vm, "menu")),
            ("type".into(), ty),
            ("roll_forward".into(), rf),
        ],
    )?;
    let mut chose = false;
    for it in &items_l {
        let p = py::items(vm, it)?;
        let mut val = p[1].clone();
        if isinst(vm, &val, "renpy.ui", "ChoiceReturn")? {
            val = py::get(vm, &val, "value")?;
        }
        if ops::eq(vm, &rv, &val)? {
            let l = py::string(vm, &p[0]).unwrap_or_default();
            log(vm, format!("Player chose: {l}"))?;
            chose = true;
            break;
        }
    }
    if !chose {
        log(vm, "No choice chosen.".into())?;
    }
    log(vm, String::new())?;
    ex(vm, "checkpoint", vec![rv.clone()], Vec::new())?;
    if py::is_none(&with_none) {
        with_none = py::config(vm, "implicit_with_none")?;
    }
    if py::truthy(vm, &with_none)? {
        let i = iface(vm)?;
        vm.call_method(&i, "do_with", vec![Value::None, Value::None])?;
    }
    Ok(rv)
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("get_menu_args", get_menu_args),
        ("menu", menu),
        ("choice_for_skipping", choice_for_skipping),
        ("predict_menu", predict_menu),
        ("display_menu", display_menu),
    ]
}

pub fn values(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let t = Value::Obj(vm.types.tuple.clone());
    let me = py::class(vm, "renpy.exports", "MenuEntry", &[t], &[], &[])?;
    crysirror_python::modules::set(vm, m, "MenuEntry", Value::Obj(me));
    Ok(())
}
