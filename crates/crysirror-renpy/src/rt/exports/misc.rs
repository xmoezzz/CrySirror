// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports` script, restart, predict, platform, debug and action
//! functions (Ren'Py 8.3 `scriptexports`, `restartexports`,
//! `predictexports`, `platformexports`, `debugexports`, `actionexports`;
//! MIT).
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};
use std::rc::Rc;

fn script(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "script")
}

// scriptexports ----------------------------------------------------------------------------------------

fn has_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(super::super::nodes::find(vm, &args[0])?.is_some()))
}

fn get_all_labels(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let r = super::super::state::rt(vm);
    let labels: Vec<String> = r.script.borrow().labels.keys().cloned().collect();
    let l: Vec<Value> = labels.iter().map(|s| vm.types.new_str(s)).collect();
    let rs = py::import_attr(vm, "renpy.revertable", "RevertableSet")?;
    let l = vm.types.new_list(l);
    vm.call(&rs, vec![l], Vec::new())
}

fn munged_filename(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let fl = get_filename_line(vm, &[], &[])?;
    let f = ops::getitem(vm, &fl, Value::Int(0))?;
    let f = py::string(vm, &f)?;
    Ok(py::s(vm, &crate::lexer::munge_prefix(&f)))
}

/// Runs init code `(priority, node)` in a fresh init-phase context.
fn run_initcode(vm: &mut Vm, initcode: &Value, kwargs: &[(Rc<str>, Value)]) -> PyResult<()> {
    let cc = py::import_attr(vm, "renpy.execution", "Context")?;
    let c = vm.call(&cc, vec![Value::Bool(false)], Vec::new())?;
    py::set(vm, &c, "init_phase", Value::Bool(true))?;
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    ops::list_append(vm, &contexts, c.clone())?;
    if !kwargs.is_empty() {
        let names: Vec<Value> = kwargs.iter().map(|(k, _)| vm.types.new_str(k)).collect();
        let names = vm.types.new_list(names);
        vm.call_method(&c, "make_dynamic", vec![names])?;
        let store = py::import(vm, "store")?;
        for (k, v) in kwargs {
            py::set(vm, &store, k, v.clone())?;
        }
    }
    let r = (|| -> PyResult<()> {
        for pair in py::items(vm, initcode)? {
            let node = ops::getitem(vm, &pair, Value::Int(1))?;
            if super::super::nodes::id_of(&node).is_some() {
                let ctx = ctx(vm)?;
                vm.call_method(&ctx, "run", vec![node])?;
            } else {
                vm.call(&node, Vec::new(), Vec::new())?;
            }
        }
        Ok(())
    })();
    vm.call_method(&c, "pop_all_dynamic", Vec::new())?;
    vm.call_method(&contexts, "pop", Vec::new())?;
    r
}

fn unlocked<R>(vm: &mut Vm, f: impl FnOnce(&mut Vm) -> PyResult<R>) -> PyResult<R> {
    let old = py::config(vm, "locked")?;
    config_set(vm, "locked", Value::Bool(false))?;
    let r = f(vm);
    config_set(vm, "locked", old)?;
    r
}

fn load_module(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ip = super::context::is_init_phase(vm, &[], &[])?;
    if !py::truthy(vm, &ip)? {
        return Err(py::error(vm, "Module loading is only allowed in init code."));
    }
    let lm = py::import_attr(vm, "renpy.exports", "loaded_modules")?;
    if ops::set_contains(vm, &lm, &args[0])? {
        return Ok(Value::None);
    }
    ops::set_add(vm, &lm, args[0].clone())?;
    let name = args[0].clone();
    let kw = kw.to_vec();
    unlocked(vm, |vm| {
        let s = script(vm)?;
        let initcode = vm.call_method(&s, "load_module", vec![name])?;
        run_initcode(vm, &initcode, &kw)
    })?;
    Ok(Value::None)
}

fn load_string(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = args[0].clone();
    let filename = arg_or(args, kw, 1, "filename", py::s(vm, "<string>"));
    let g = py::import(vm, "renpy.game")?;
    let oei = py::get_or_none(vm, &g, "exception_info")?;
    let r = unlocked(vm, |vm| {
        let sc = script(vm)?;
        let r = vm.call_method(&sc, "load_string", vec![filename, s])?;
        let p = py::items(vm, &r)?;
        if py::is_none(&p[0]) {
            return Ok(Value::None);
        }
        run_initcode(vm, &p[1], &[])?;
        let first = ops::getitem(vm, &p[0], Value::Int(0))?;
        py::get(vm, &first, "name")
    });
    py::set(vm, &g, "exception_info", oei)?;
    r
}

fn load_language(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let language = args[0].clone();
    if py::is_none(&language) || !py::config_bool(vm, "defer_tl_scripts")? {
        return Ok(Value::None);
    }
    let sc = script(vm)?;
    let ll = py::get(vm, &sc, "load_languages")?;
    if ops::contains(vm, &ll, &language)? {
        return Ok(Value::None);
    }
    let g = py::import(vm, "renpy.game")?;
    let oei = py::get_or_none(vm, &g, "exception_info")?;
    let r = unlocked(vm, |vm| {
        ops::set_add(vm, &ll, language.clone())?;
        let initcode = vm.call_method(&sc, "load_script", Vec::new())?;
        run_initcode(vm, &initcode, &[])
    });
    py::set(vm, &g, "exception_info", oei)?;
    r?;
    Ok(Value::None)
}

fn include_module(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ip = super::context::is_init_phase(vm, &[], &[])?;
    if !py::truthy(vm, &ip)? {
        return Err(py::error(vm, "Module loading is only allowed in init code."));
    }
    let sc = script(vm)?;
    vm.call_method(&sc, "include_module", vec![args[0].clone()])
}

// restartexports ---------------------------------------------------------------------------------------

fn save_quit_slot(vm: &mut Vm, save: bool) -> PyResult<()> {
    let qs = store_get(vm, "_quit_slot")?;
    if save && !py::is_none(&qs) {
        let store = py::import(vm, "store")?;
        let sn = py::get_opt(vm, &store, "save_name")?.unwrap_or_else(|| py::s(vm, ""));
        let f = py::import_attr(vm, "renpy.loadsave", "save")?;
        vm.call(&f, vec![qs, sn], Vec::new())?;
    }
    Ok(())
}

fn full_restart(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut transition = arg_or(args, kw, 0, "transition", Value::Bool(false));
    let label = arg_or(args, kw, 1, "label", py::s(vm, "_invoke_main_menu"));
    let target = arg_or(args, kw, 2, "target", py::s(vm, "_main_menu"));
    let save = kwb(vm, arg(args, kw, 3, "save"), false)?;
    save_quit_slot(vm, save)?;
    if matches!(transition, Value::Bool(false)) {
        transition = py::config(vm, "end_game_transition")?;
    }
    let t = vm.types.new_tuple(vec![transition, label, target]);
    Err(raise(vm, "renpy.game", "FullRestartException", vec![t]))
}

fn utter_restart(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let keep = arg_or(args, kw, 0, "keep_renderer", Value::Bool(false));
    let session = py::import_attr(vm, "renpy", "session")?;
    py::dict_set(vm, &session, "_keep_renderer", keep)?;
    Err(raise(vm, "renpy.game", "UtterRestartException", Vec::new()))
}

fn reload_script(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if store_true(vm, "_in_replay")? {
        return Ok(Value::None);
    }
    let s = ex(vm, "get_screen", vec![py::s(vm, "menu")], Vec::new())?;
    let session = py::import_attr(vm, "renpy", "session")?;
    py::dict_set(vm, &session, "_reload", Value::Bool(true))?;
    for k in ["_reload_screen", "_main_menu_screen"] {
        let kv = py::s(vm, k);
        if ops::contains(vm, &session, &kv)? {
            return utter_restart(vm, &[], &[]);
        }
    }
    let record = |vm: &mut Vm, prefix: &str| -> PyResult<()> {
        if py::is_none(&s) {
            return Ok(());
        }
        let sn = py::get(vm, &s, "screen_name")?;
        let n0 = ops::getitem(vm, &sn, Value::Int(0))?;
        py::dict_set(vm, &session, prefix, n0)?;
        let scope = py::get(vm, &s, "scope")?;
        let a = vm.call_method(&scope, "get", vec![py::s(vm, "_args"), vm.types.new_tuple(Vec::new())])?;
        py::dict_set(vm, &session, &format!("{prefix}_args"), a)?;
        let k = vm.call_method(&scope, "get", vec![py::s(vm, "_kwargs"), vm.types.new_dict()])?;
        py::dict_set(vm, &session, &format!("{prefix}_kwargs"), k)?;
        Ok(())
    };
    if !store_true(vm, "main_menu")? {
        record(vm, "_reload_screen")?;
        let f = py::import_attr(vm, "renpy.game", "call_in_new_context")?;
        vm.call(&f, vec![py::s(vm, "_save_reload_game")], Vec::new())?;
        Ok(Value::None)
    } else {
        record(vm, "_main_menu_screen")?;
        utter_restart(vm, &[], &[])
    }
}

fn quit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let relaunch = arg_or(args, kw, 0, "relaunch", Value::Bool(false));
    let status = arg_or(args, kw, 1, "status", Value::Int(0));
    let save = kwb(vm, arg(args, kw, 2, "save"), false)?;
    save_quit_slot(vm, save)?;
    if super::super::nodes::find(vm, &py::s(vm, "quit"))?.is_some() {
        let f = py::import_attr(vm, "renpy.game", "call_in_new_context")?;
        vm.call(&f, vec![py::s(vm, "quit")], Vec::new())?;
    }
    let c = py::import_attr(vm, "renpy.game", "QuitException")?;
    let e = vm.call(&c, Vec::new(), vec![("relaunch".into(), relaunch), ("status".into(), status)])?;
    Err(PyErr::Exc(e.obj().cloned().unwrap()))
}

fn set_autoreload(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = py::import(vm, "renpy")?;
    py::set(vm, &r, "autoreload", args[0].clone())?;
    Ok(Value::None)
}

fn get_autoreload(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy", "autoreload")
}

// predictexports ---------------------------------------------------------------------------------------

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn glob_match(pat: &str, s: &str) -> bool {
    fn m(p: &[char], s: &[char]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some('*'), _) => m(&p[1..], s) || (!s.is_empty() && m(p, &s[1..])),
            (Some('?'), Some(_)) => m(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    m(&p, &s)
}

fn expand_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = &args[0];
    let Some(s) = d.as_str().map(|s| s.to_owned()) else { return Ok(vm.types.new_list(vec![d.clone()])) };
    if !s.contains('*') {
        return Ok(vm.types.new_list(vec![d.clone()]));
    }
    let l = if s.contains('.') { ex(vm, "list_files", vec![Value::Bool(false)], Vec::new())? } else { ex(vm, "list_images", Vec::new(), Vec::new())? };
    let mut rv = Vec::new();
    for i in py::items(vm, &l)? {
        if glob_match(&s, &py::string(vm, &i)?) {
            rv.push(i);
        }
    }
    Ok(vm.types.new_list(rv))
}

fn change_predict(vm: &mut Vm, args: &[Value], add: bool) -> PyResult<Value> {
    let rs = py::import_attr(vm, "renpy.revertable", "RevertableSet")?;
    let old = store_get(vm, "_predict_set")?;
    let new = vm.call(&rs, vec![old], Vec::new())?;
    for i in args {
        let ds = expand_predict(vm, &[i.clone()], &[])?;
        for d in py::items(vm, &ds)? {
            let d = super::super::easy::displayable(vm, &d, &Value::None, false)?;
            vm.call_method(&new, if add { "add" } else { "discard" }, vec![d])?;
        }
    }
    store_set(vm, "_predict_set", new)?;
    Ok(Value::None)
}

fn start_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    change_predict(vm, args, true)
}

fn stop_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    change_predict(vm, args, false)
}

fn predict_screens(vm: &mut Vm) -> PyResult<Value> {
    let rd = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
    let old = store_get(vm, "_predict_screen")?;
    vm.call(&rd, vec![old], Vec::new())
}

fn start_predict_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let new = predict_screens(vm)?;
    let a = vm.types.new_tuple(args[1..].to_vec());
    let k = py::kwargs_dict(vm, kw)?;
    let t = vm.types.new_tuple(vec![a, k]);
    ops::setitem(vm, &new, args[0].clone(), t)?;
    store_set(vm, "_predict_screen", new)?;
    Ok(Value::None)
}

fn stop_predict_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let new = predict_screens(vm)?;
    vm.call_method(&new, "pop", vec![args[0].clone(), Value::None])?;
    store_set(vm, "_predict_screen", new)?;
    Ok(Value::None)
}

fn predicting(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display.predict", "predicting")
}

// platformexports --------------------------------------------------------------------------------------

fn variant(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let variants = py::config(vm, "variants")?;
    if args[0].as_str().is_some() {
        return Ok(Value::Bool(ops::contains(vm, &variants, &args[0])?));
    }
    for n in py::items(vm, &args[0])? {
        if ops::contains(vm, &variants, &n)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn invoke_in_thread(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    // Script code runs on the script thread only: the call happens now,
    // followed by the interaction restart upstream's thread performs.
    let r = vm.call(&args[0], args[1..].to_vec(), kw.to_vec());
    if let Err(e) = r {
        if matches!(e, PyErr::Suspend(..)) {
            return Err(e);
        }
        let s = py::error_text(vm, &e);
        super::super::link::diagnostic(vm, format!("invoke_in_thread: {s}"));
    }
    super::display::restart_interaction(vm, &[], &[])
}

fn invoke_in_main_thread(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ip = super::context::is_init_phase(vm, &[], &[])?;
    if py::truthy(vm, &ip)? {
        return Err(py::error(vm, "invoke_in_main_thread may not be called during the init phase."));
    }
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    let q = py::get(vm, &i, "invoke_queue")?;
    let a = vm.types.new_tuple(args[1..].to_vec());
    let k = py::kwargs_dict(vm, kw)?;
    let t = vm.types.new_tuple(vec![args[0].clone(), a, k]);
    ops::list_append(vm, &q, t)?;
    Ok(Value::None)
}

fn falsy(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn open_url(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::import_attr(vm, "renpy.game", "preferences")?;
    py::set(vm, &p, "fullscreen", Value::Bool(false))?;
    // Opening a browser is a host decision; nothing is launched.
    Ok(Value::None)
}

// debugexports -----------------------------------------------------------------------------------------

fn warp_to_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let w = py::import(vm, "renpy.warp")?;
    py::set(vm, &w, "warp_spec", args[0].clone())?;
    full_restart(vm, &[], &[])
}

fn get_filename_line(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = ctx(vm)?;
    let cur = py::get_or_none(vm, &c, "current")?;
    match super::super::nodes::find(vm, &cur)? {
        Some(id) => {
            let r = super::super::state::rt(vm);
            let (f, l) = {
                let s = r.script.borrow();
                let n = s.arena.get(id);
                (n.filename.to_string(), n.linenumber)
            };
            Ok(vm.types.new_tuple(vec![py::s(vm, &f), Value::Int(l as i64)]))
        }
        None => Ok(vm.types.new_tuple(vec![py::s(vm, "unknown"), Value::Int(0)])),
    }
}

/// `log(msg)`: with `config.log` set, lines go to the host diagnostics
/// (the runtime has no writable game directory).
fn log(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::config(vm, "log")?;
    if !py::truthy(vm, &l)? || py::is_none(&args[0]) {
        return Ok(Value::None);
    }
    let msg = crysirror_python::ops::str(vm, &args[0])?;
    super::super::link::diagnostic(vm, msg.to_string());
    Ok(Value::None)
}

fn push_error_handler(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let h = py::import_attr(vm, "renpy.exports", "_error_handlers")?;
    ops::list_append(vm, &h, args[0].clone())?;
    Ok(Value::None)
}

fn pop_error_handler(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let h = py::import_attr(vm, "renpy.exports", "_error_handlers")?;
    vm.call_method(&h, "pop", Vec::new())?;
    Ok(Value::None)
}

fn error_default(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::string(vm, &args[0])?;
    Err(py::error(vm, m))
}

fn error(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let h = py::import_attr(vm, "renpy.exports", "_error_handlers")?;
    let last = ops::getitem(vm, &h, Value::Int(-1))?;
    vm.call(&last, vec![args[0].clone()], Vec::new())
}

fn write_log(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut s = crysirror_python::ops::str(vm, &args[0])?.to_string();
    if args.len() > 1 {
        let t = vm.types.new_tuple(args[1..].to_vec());
        let r = ops::binary_op(vm, &args[0], &t, crysirror_python::ast::BinOp::Mod, false, false)?;
        s = crysirror_python::ops::str(vm, &r)?.to_string();
    }
    super::super::link::diagnostic(vm, s);
    Ok(Value::None)
}

// actionexports ----------------------------------------------------------------------------------------

fn notify(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::config(vm, "notify")?;
    vm.call(&n, vec![args[0].clone()], Vec::new())
}

fn display_notify(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ex(vm, "hide_screen", vec![py::s(vm, "notify")], Vec::new())?;
    ex(vm, "show_screen", vec![py::s(vm, "notify")], vec![("message".into(), args[0].clone())])?;
    if let Ok(f) = py::import_attr(vm, "renpy.text.extras", "filter_alt_text") {
        let t = vm.call(&f, vec![args[0].clone()], Vec::new())?;
        if let Ok(tts) = py::import(vm, "renpy.display.tts") {
            py::set(vm, &tts, "notify_text", t)?;
        }
    }
    super::display::restart_interaction(vm, &[], &[])
}

fn confirm(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let store = py::import(vm, "store")?;
    let ret = py::get(vm, &store, "Return")?;
    let y = vm.call(&ret, vec![Value::Bool(true)], Vec::new())?;
    let n = vm.call(&ret, vec![Value::Bool(false)], Vec::new())?;
    let layout = py::get(vm, &store, "layout")?;
    let ys = py::get(vm, &layout, "yesno_screen")?;
    vm.call(&ys, vec![args[0].clone()], vec![("yes".into(), y), ("no".into(), n)])?;
    ui(vm, "interact", Vec::new(), Vec::new())
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("has_label", has_label),
        ("get_all_labels", get_all_labels),
        ("munged_filename", munged_filename),
        ("load_module", load_module),
        ("load_string", load_string),
        ("load_language", load_language),
        ("include_module", include_module),
        ("full_restart", full_restart),
        ("utter_restart", utter_restart),
        ("reload_script", reload_script),
        ("quit", quit),
        ("set_autoreload", set_autoreload),
        ("get_autoreload", get_autoreload),
        ("cache_pin", noop),
        ("cache_unpin", noop),
        ("expand_predict", expand_predict),
        ("start_predict", start_predict),
        ("stop_predict", stop_predict),
        ("start_predict_screen", start_predict_screen),
        ("stop_predict_screen", stop_predict_screen),
        ("predicting", predicting),
        ("variant", variant),
        ("vibrate", noop),
        ("invoke_in_thread", invoke_in_thread),
        ("invoke_in_main_thread", invoke_in_main_thread),
        ("get_on_battery", falsy),
        ("get_sdl_dll", noop),
        ("get_sdl_window_pointer", noop),
        ("check_permission", falsy),
        ("request_permission", falsy),
        ("open_url", open_url),
        ("warp_to_line", warp_to_line),
        ("get_filename_line", get_filename_line),
        ("log", log),
        ("push_error_handler", push_error_handler),
        ("pop_error_handler", pop_error_handler),
        ("error", error),
        ("write_log", write_log),
        ("notify", notify),
        ("display_notify", display_notify),
        ("confirm", confirm),
    ]
}

pub fn values(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    set(vm, "loaded_modules", vm.types.new_set(Default::default()));
    let e = vm.types.new_builtin("_error", error_default);
    set(vm, "_error", e.clone());
    set(vm, "_error_handlers", vm.types.new_list(vec![e]));
    set(vm, "logfile", Value::None);
    set(vm, "old_battery", Value::Bool(false));
    set(vm, "sdl_dll", Value::Bool(false));
    Ok(())
}
