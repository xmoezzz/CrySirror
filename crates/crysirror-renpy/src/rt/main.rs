// SPDX-License-Identifier: MPL-2.0
//! Engine start-up, after Ren'Py 6.99–8.3 (MIT): `renpy.main` (`main`,
//! `run`, `choose_variants`), `renpy.arguments`, `renpy.dump`,
//! `renpy.display.presplash` and `renpy.warp`.
//!
//! The player has no command line: `renpy.game.args` is upstream's parse
//! of no arguments (`command == "run"`), so the launcher-only commands
//! (`compile`, `lint`, `rmpersistent`, JSON dumps, warping) are reachable
//! only from game code, which can still register its own commands. The
//! host shows the presplash, so the presplash hooks have no window to
//! pump. `.rpe` engine extensions are Python packages for CPython and are
//! not loaded. Archives are mounted by the adapter before boot, so the
//! game's files already include their contents.
//!
//! The release decides the start-up steps that changed between versions
//! (checked against `renpy/main.py` of 6.99.12.4, 7.0.0, 7.3.5, 7.4.11,
//! 7.5.3, 8.0.3, 8.1.3, 8.2.3 and 8.3.7): `loadsave.init` and
//! `log.post_init` from 7.0; the post-init `QuitException` and
//! `quit_callbacks` from 7.4; `at_exit_callbacks` from 7.5; save tokens,
//! the second `loadsave.init`, `fadeout_all` and `save_on_quit_MP` from
//! 8.1; `config.post_init`, `init_keymap` and build info from 8.2. Seen
//! translations are counted after `post_init` before 7.0, and warping
//! runs from `run` before 7.0 (later from `_start` in common code).
use super::{py, state};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MAIN: &str = "renpy.main";
const ARGUMENTS: &str = "renpy.arguments";
const DUMP: &str = "renpy.dump";
const PRESPLASH: &str = "renpy.display.presplash";
const WARP: &str = "renpy.warp";

fn call(vm: &mut Vm, module: &str, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, Vec::new())
}

fn call0(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    call(vm, module, name, Vec::new())
}

fn set_attr(vm: &mut Vm, module: &str, name: &str, v: Value) -> PyResult<()> {
    let m = py::import(vm, module)?;
    py::set(vm, &m, name, v)
}

fn get_attr(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    py::import_attr(vm, module, name)
}

fn method0(vm: &mut Vm, module: &str, obj: &str, name: &str) -> PyResult<Value> {
    let o = get_attr(vm, module, obj)?;
    py::method(vm, &o, name, Vec::new())
}

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    state::rt(vm).profile.at_least(major, minor)
}

fn exc_is(vm: &mut Vm, e: &PyErr, module: &str, name: &str) -> bool {
    let PyErr::Exc(o) = e else { return false };
    match py::import_attr(vm, module, name) {
        Ok(c) => ops::exc_matches(vm, &Value::Obj(o.clone()), &c).unwrap_or(false),
        Err(_) => false,
    }
}

fn context(vm: &mut Vm) -> PyResult<Value> {
    call0(vm, "renpy.game", "context")
}

// renpy.main ----------------------------------------------------------------------

fn log_clock(vm: &mut Vm, what: &str) -> PyResult<()> {
    let m = py::import(vm, MAIN)?;
    let last = py::get(vm, &m, "last_clock")?;
    let last = py::f64_of(vm, &last)?;
    let now = super::link::now(vm);
    super::log::write(vm, &format!("{what} took {:.2}s", now - last));
    py::set(vm, &m, "last_clock", py::f(now))
}

fn f_log_clock(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    log_clock(vm, &s)?;
    Ok(Value::None)
}

fn f_reset_clock(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let now = py::f(super::link::now(vm));
    set_attr(vm, MAIN, "last_clock", now)?;
    Ok(Value::None)
}

/// `run(restart)`: one run of the script from the start label.
fn trace_styles(vm: &mut Vm, at: &str) {
    if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some() {
        let ok = super::style::get_style(vm, "main_menu_text").is_ok();
        let t = super::style::get_style(vm, "main_menu_title").is_ok();
        super::link::diagnostic(
            vm,
            format!("trace: {at}: main_menu_text={ok} main_menu_title={t}"),
        );
    }
}

/// 6.18 `run(restart)`: the init-time steps of later releases (styles,
/// screen analysis, post_init) all happen in `main`.
fn run_618(vm: &mut Vm, restart: Value) -> PyResult<Value> {
    call0(vm, "renpy.python", "clean_stores")?;
    let log = call0(vm, "renpy.python", "RollbackLog")?;
    set_attr(vm, "renpy.game", "log", log)?;
    let ctx_cls = get_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&ctx_cls, vec![Value::Bool(true)], Vec::new())?;
    let contexts = vm.types.new_list(vec![ctx]);
    set_attr(vm, "renpy.game", "contexts", contexts)?;
    let script = get_attr(vm, "renpy.game", "script")?;
    let start = py::s(vm, "_start");
    let has = py::method(vm, &script, "has_label", vec![start])?;
    let start_label = if py::truthy(vm, &has)? { "_start" } else { "start" };
    let ctx = context(vm)?;
    let l = py::s(vm, start_label);
    py::method(vm, &ctx, "goto_label", vec![l])?;
    let warp_label = call0(vm, WARP, "warp")?;
    if !py::is_none(&warp_label) {
        py::method(vm, &ctx, "goto_label", vec![warp_label])?;
        let a = py::s(vm, "_after_warp");
        py::method(vm, &ctx, "call", vec![a])?;
        let config = py::import(vm, "renpy.config")?;
        py::set(vm, &config, "skipping", Value::None)?;
    }
    let _ = (|| -> PyResult<()> {
        let ctime = call0(vm, "time", "ctime")?;
        let line = format!("--- {}", py::string(vm, &ctime)?);
        let s = py::s(vm, &line);
        call(vm, "renpy.exports", "log", vec![s])?;
        let e = py::s(vm, "");
        call(vm, "renpy.exports", "log", vec![e])?;
        Ok(())
    })();
    set_attr(vm, "store", "_restart", restart)?;
    method0(vm, "renpy.display", "interface", "enter_context")?;
    call(vm, "renpy.execution", "run_context", vec![Value::Bool(true)])?;
    Ok(Value::None)
}

/// 6.18 `main` after the init code: post-init steps, then the run loop.
fn after_init_618(vm: &mut Vm) -> PyResult<()> {
    method0(vm, "renpy.game", "script", "save_bytecode")?;
    for f in {
        let g = py::import(vm, "renpy.game")?;
        py::get_items(vm, &g, "post_init")
    }? {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    // 6.12 – 6.18 differ in the post-init steps they run.
    if at_least(vm, 6, 15) {
        call0(vm, "renpy.translation", "init_translation")?;
    }
    call0(vm, "renpy.style", "build_styles")?;
    if at_least(vm, 6, 18) {
        method0(vm, "renpy.game", "script", "analyze")?;
        call0(vm, "renpy.atl", "compile_all")?;
    }
    if at_least(vm, 6, 17) {
        call0(vm, "renpy.display.screen", "prepare_screens")?;
    }
    call0(vm, "renpy.loader", "index_archives")?;
    for n in ["less_memory", "less_mouse", "less_updates"] {
        set_attr(vm, "renpy.game", n, Value::Bool(false))?;
    }
    if at_least(vm, 6, 14) {
        call(vm, DUMP, "dump", vec![Value::Bool(false)])?;
        let ok = call0(vm, ARGUMENTS, "post_init")?;
        if !py::truthy(vm, &ok)? {
            return Ok(());
        }
    }
    let script = get_attr(vm, "renpy.game", "script")?;
    py::set(vm, &script, "all_stmts", Value::None)?;
    call0(vm, "renpy.python", "make_clean_stores")?;
    method0(vm, "renpy.display.im", "cache", "init")?;
    let interface = get_attr(vm, "renpy.game", "interface")?;
    if !py::truthy(vm, &interface)? {
        call0(vm, "renpy.display.core", "Interface")?;
    }
    let info = py::s(vm, "While running game code:");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    set_attr(vm, "renpy", "first_utter_start", Value::Bool(false))?;
    let mut restart = Value::None;
    loop {
        if py::truthy(vm, &restart)? && at_least(vm, 6, 15) {
            call0(vm, "renpy.display.screen", "before_restart")?;
        }
        let r = run_618(vm, restart.clone());
        let egt = py::config(vm, "end_game_transition")?;
        let a = py::s(vm, "_invoke_main_menu");
        let b = py::s(vm, "_main_menu");
        restart = vm.types.new_tuple(vec![egt, a, b]);
        let p = call(vm, "renpy.persistent", "update", vec![Value::Bool(true)]);
        let outcome = match r {
            Ok(_) => Ok(()),
            Err(e) if exc_is(vm, &e, "renpy.game", "FullRestartException") => {
                if let PyErr::Exc(o) = &e {
                    restart = py::get(vm, &Value::Obj(o.clone()), "reason")?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        };
        let _ = method0(vm, "renpy.display", "interface", "finish_pending");
        let ev = get_attr(vm, "renpy.loadsave", "autosave_not_running")?;
        py::method(vm, &ev, "wait", vec![py::f(3.0)])?;
        outcome?;
        p?;
    }
}

fn f_run(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    trace_styles(vm, "run start");
    let restart = args.first().cloned().unwrap_or(Value::None);
    if !at_least(vm, 6, 99) {
        return run_618(vm, restart);
    }
    f_reset_clock(vm, &[], &[])?;
    call0(vm, "renpy.python", "clean_stores")?;
    log_clock(vm, "Cleaning stores")?;
    call0(vm, "renpy.translation", "init_translation")?;
    trace_styles(vm, "after init_translation");
    log_clock(vm, "Init translation")?;
    call0(vm, "renpy.style", "build_styles")?;
    log_clock(vm, "Build styles")?;
    call0(vm, "renpy.sl2.slast", "load_cache")?;
    log_clock(vm, "Load screen analysis")?;
    call0(vm, "renpy.display.screen", "analyze_screens")?;
    log_clock(vm, "Analyze screens")?;
    let restarting = py::truthy(vm, &restart)?;
    if !restarting {
        call0(vm, "renpy.sl2.slast", "save_cache")?;
        log_clock(vm, "Save screen analysis")?;
    }
    call0(vm, "renpy.display.screen", "prepare_screens")?;
    log_clock(vm, "Prepare screens")?;
    if !restarting {
        call0(vm, "renpy.pyanalysis", "save_cache")?;
        method0(vm, "renpy.game", "script", "save_bytecode")?;
        log_clock(vm, "Save bytecode.")?;
    }
    let ok = call0(vm, ARGUMENTS, "post_init")?;
    if !py::truthy(vm, &ok)? {
        if at_least(vm, 7, 4) {
            let q = get_attr(vm, "renpy.game", "QuitException")?;
            let e = vm.call(&q, Vec::new(), Vec::new())?;
            return Err(PyErr::Exc(
                e.obj()
                    .cloned()
                    .ok_or_else(|| vm.type_error("QuitException"))?,
            ));
        }
        call0(vm, "renpy.exports", "quit")?;
    }
    if let Ok(base) = get_attr(vm, "renpy.display.im", "ImageBase") {
        py::set(vm, &base, "obsolete_list", vm.types.new_list(Vec::new()))?;
    }
    if py::config_bool(vm, "clear_lines")? {
        let lines = get_attr(vm, "renpy.scriptedit", "lines")?;
        py::method(vm, &lines, "clear", Vec::new())?;
    }
    call0(vm, PRESPLASH, "sleep")?;
    let log = call0(vm, "renpy.python", "RollbackLog")?;
    set_attr(vm, "renpy.game", "log", log)?;
    let ctx_cls = get_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&ctx_cls, vec![Value::Bool(true)], Vec::new())?;
    let contexts = vm.types.new_list(vec![ctx]);
    set_attr(vm, "renpy.game", "contexts", contexts)?;
    let script = get_attr(vm, "renpy.game", "script")?;
    let start = py::s(vm, "_start");
    let has = py::method(vm, &script, "has_label", vec![start])?;
    let start_label = if py::truthy(vm, &has)? {
        "_start"
    } else {
        "start"
    };
    let ctx = context(vm)?;
    let l = py::s(vm, start_label);
    py::method(vm, &ctx, "goto_label", vec![l])?;
    if !at_least(vm, 7, 0) {
        let warp_label = call0(vm, WARP, "warp")?;
        if !py::is_none(&warp_label) {
            py::method(vm, &ctx, "goto_label", vec![warp_label])?;
            let a = py::s(vm, "_after_warp");
            py::method(vm, &ctx, "call", vec![a])?;
            let config = py::import(vm, "renpy.config")?;
            py::set(vm, &config, "skipping", Value::None)?;
        }
    }
    let _ = (|| -> PyResult<()> {
        let ctime = call0(vm, "time", "ctime")?;
        let line = format!("--- {}", py::string(vm, &ctime)?);
        let s = py::s(vm, &line);
        call(vm, "renpy.exports", "log", vec![s])?;
        let e = py::s(vm, "");
        call(vm, "renpy.exports", "log", vec![e])?;
        Ok(())
    })();
    set_attr(vm, "store", "_restart", restart)?;
    method0(vm, "renpy.display", "interface", "enter_context")?;
    log_clock(vm, &format!("Running {start_label}"))?;
    call(
        vm,
        "renpy.execution",
        "run_context",
        vec![Value::Bool(true)],
    )?;
    Ok(Value::None)
}

/// `choose_variants()`: the variants of the host the game is shown on.
/// The adapter reports the platform in `_crysirror_renpy.variants()`.
fn f_choose_variants(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let host = call0(vm, "_crysirror_renpy", "variants")?;
    let mut vs = py::items(vm, &host)?;
    vs.push(Value::None);
    let l = vm.types.new_list(vs);
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "variants", l)?;
    Ok(Value::None)
}

fn f_load_build_info(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let info = (|| -> PyResult<Value> {
        let fname = py::s(vm, "cache/build_info.json");
        let enc = py::s(vm, "utf-8");
        let f = call(vm, "renpy.exports", "open_file", vec![fname, enc])?;
        call(vm, "json", "load", vec![f])
    })();
    let info = match info {
        Ok(v) => v,
        Err(_) => {
            let d = py::dict(vm);
            py::dict_set(vm, &d, "info", py::dict(vm))?;
            d
        }
    };
    set_attr(vm, "renpy.game", "build_info", info)?;
    Ok(Value::None)
}

fn f_load_rpe(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = args
        .first()
        .map(|v| py::string(vm, v))
        .transpose()?
        .unwrap_or_default();
    Err(py::error(
        vm,
        format!("Ren'Py engine extensions (.rpe) are not supported by this runtime: {n}"),
    ))
}

fn persistent_attr(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let p = get_attr(vm, "renpy.game", "persistent")?;
    py::get_or_none(vm, &p, name)
}

fn count_seen_translates(vm: &mut Vm) -> PyResult<()> {
    let seen = persistent_attr(vm, "_seen_translates")?;
    if py::is_none(&seen) {
        return Ok(());
    }
    let script = get_attr(vm, "renpy.game", "script")?;
    let tr = py::get(vm, &script, "translator")?;
    let dt = py::get(vm, &tr, "default_translates")?;
    let mut n = 0;
    for i in py::items(vm, &seen)? {
        if ops::contains(vm, &dt, &i)? {
            n += 1;
        }
    }
    let g = py::import(vm, "renpy.game")?;
    let c = py::get(vm, &g, "seen_translates_count")?;
    let c = py::int(vm, &c)?;
    py::set(vm, &g, "seen_translates_count", Value::Int(c + n))
}

fn init_phase(vm: &mut Vm) -> PyResult<()> {
    let errors = call0(vm, "renpy.parser", "report_parse_errors")?;
    if py::truthy(vm, &errors)? {
        let c = get_attr(vm, "renpy.game", "ParseErrorException")?;
        let e = vm.call(&c, Vec::new(), Vec::new())?;
        return Err(PyErr::Exc(
            e.obj()
                .cloned()
                .ok_or_else(|| vm.type_error("ParseErrorException"))?,
        ));
    }
    let info = py::s(vm, "While executing init code:");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    let script = get_attr(vm, "renpy.game", "script")?;
    let initcode = py::get_items(vm, &script, "initcode")?;
    let node_cls = get_attr(vm, "renpy.ast", "Node")?;
    let node_cls = node_cls
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("renpy.ast.Node"))?;
    for (i, entry) in initcode.into_iter().enumerate() {
        set_attr(vm, "renpy.game", "initcode_ast_id", Value::Int(i as i64))?;
        let node = ops::getitem(vm, &entry, Value::Int(1))?;
        if ops::isinstance(vm, &node, &node_cls) {
            let ctx = context(vm)?;
            py::method(vm, &ctx, "run", vec![node])?;
        } else {
            vm.call(&node, Vec::new(), Vec::new())?;
        }
    }
    trace_styles(vm, "after initcode");
    let info = py::s(vm, "After initialization, but before game start.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    if !at_least(vm, 6, 99) {
        return after_init_618(vm);
    }
    if at_least(vm, 7, 0) {
        call0(vm, "renpy.log", "post_init")?;
    }
    for f in {
        let t__ = py::import(vm, "renpy.game")?;
        py::get_items(vm, &t__, "post_init")
    }? {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    if at_least(vm, 8, 2) {
        call0(vm, "renpy.config", "post_init")?;
    }
    method0(vm, "renpy.game", "script", "report_duplicate_labels")?;
    if at_least(vm, 7, 0) {
        let names = get_attr(vm, "renpy.display.image", "image_names")?;
        py::method(vm, &names, "sort", Vec::new())?;
    }
    let w = py::config(vm, "screen_width")?;
    let h = py::config(vm, "screen_height")?;
    let size = vm.types.new_tuple(vec![w, h]);
    let p = get_attr(vm, "renpy.game", "persistent")?;
    py::set(vm, &p, "_virtual_size", size)?;
    if !at_least(vm, 7, 0) {
        count_seen_translates(vm)?;
    }
    log_clock(vm, "Running init code")?;
    call0(vm, "renpy.pyanalysis", "load_cache")?;
    method0(vm, "renpy.game", "script", "analyze")?;
    call0(vm, "renpy.atl", "compile_all")?;
    log_clock(vm, "Analyze and compile ATL")?;
    if at_least(vm, 8, 1) {
        call0(vm, "renpy.savelocation", "init")?;
        call0(vm, "renpy.loadsave", "init")?;
    }
    call0(vm, "renpy.loader", "index_archives")?;
    // RENPY_LESS_* come from the environment; the player sets none.
    for n in ["less_memory", "less_mouse", "less_updates"] {
        set_attr(vm, "renpy.game", n, Value::Bool(false))?;
    }
    call(vm, DUMP, "dump", vec![Value::Bool(false)])?;
    method0(vm, "renpy.game", "script", "make_backups")?;
    method0(vm, "renpy.display.im", "cache", "init")?;
    call0(vm, "renpy.python", "make_clean_stores")?;
    log_clock(vm, "Making clean stores")?;
    if at_least(vm, 8, 2) {
        call0(vm, "renpy.display.behavior", "init_keymap")?;
    }
    let interface = get_attr(vm, "renpy.game", "interface")?;
    if !py::truthy(vm, &interface)? {
        call0(vm, "renpy.display.core", "Interface")?;
        log_clock(vm, "Creating interface object")?;
    }
    let mut restart = Value::None;
    loop {
        if py::truthy(vm, &restart)? {
            call0(vm, "renpy.display.screen", "before_restart")?;
        }
        let r = f_run(vm, std::slice::from_ref(&restart), &[]);
        let egt = py::config(vm, "end_game_transition")?;
        let a = py::s(vm, "_invoke_main_menu");
        let b = py::s(vm, "_main_menu");
        restart = vm.types.new_tuple(vec![egt, a, b]);
        let outcome = match r {
            Ok(_) => Ok(()),
            Err(e) if exc_is(vm, &e, "renpy.game", "QuitException") => {
                if at_least(vm, 8, 1) {
                    let _ = call0(vm, "renpy.audio.audio", "fadeout_all");
                }
                Err(e)
            }
            Err(e) if exc_is(vm, &e, "renpy.game", "FullRestartException") => {
                if let PyErr::Exc(o) = &e {
                    restart = py::get(vm, &Value::Obj(o.clone()), "reason")?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        };
        let p = call(vm, "renpy.persistent", "update", vec![Value::Bool(true)]);
        if at_least(vm, 8, 1) {
            let _ = call0(vm, "renpy.persistent", "save_on_quit_MP");
        }
        let _ = method0(vm, "renpy.display", "interface", "finish_pending");
        let ev = get_attr(vm, "renpy.loadsave", "autosave_not_running")?;
        py::method(vm, &ev, "wait", vec![py::f(3.0)])?;
        if at_least(vm, 7, 5) {
            for cb in {
                let t__ = py::import(vm, "renpy.config")?;
                py::get_items(vm, &t__, "at_exit_callbacks")
            }
            .unwrap_or_default()
            {
                vm.call(&cb, Vec::new(), Vec::new())?;
            }
        }
        outcome?;
        p?;
    }
}

fn f_main(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let r = main_body(vm);
    let fin = (|| -> PyResult<()> {
        if at_least(vm, 7, 4) {
            for cb in {
                let t__ = py::import(vm, "renpy.config")?;
                py::get_items(vm, &t__, "quit_callbacks")
            }
            .unwrap_or_default()
            {
                vm.call(&cb, Vec::new(), Vec::new())?;
            }
        }
        call0(vm, "renpy.loader", "auto_quit")?;
        call0(vm, "renpy.savelocation", "quit")?;
        call0(vm, "renpy.translation", "write_updated_strings")?;
        Ok(())
    })();
    r?;
    fin?;
    let handled = get_attr(vm, "renpy.display.error", "error_handled")?;
    if !py::truthy(vm, &handled)? {
        call0(vm, "renpy.display.render", "check_at_shutdown")?;
    }
    Ok(Value::None)
}

/// 6.7 – 6.11 `main()`: the script loads before persistent data and
/// styles, there is no `_errorhandling` module, and the init phase is a
/// flag of `renpy.game` around the init code.
fn main_body_611(vm: &mut Vm) -> PyResult<()> {
    let info = py::s(vm, "Before loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    call0(vm, "renpy.config", "init")?;
    let gamedir = py::config(vm, "gamedir")?;
    set_attr(vm, "renpy.game", "basepath", gamedir.clone())?;
    let commondir = py::config(vm, "commondir")?;
    let mut sp = vec![gamedir];
    if !py::is_none(&commondir) {
        sp.push(commondir);
    }
    let sp = vm.types.new_list(sp);
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "searchpath", sp)?;
    call0(vm, "renpy.loader", "index_archives")?;
    let log = call0(vm, "renpy.python", "RollbackLog")?;
    set_attr(vm, "renpy.game", "log", log)?;
    let store = py::import(vm, "store")?;
    set_attr(vm, "store", "store", store)?;
    let info = py::s(vm, "While loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    let script = call0(vm, "renpy.script", "Script")?;
    set_attr(vm, "renpy.game", "script", script.clone())?;
    // Statements that run while the script loads need a context.
    let ctx_cls = get_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&ctx_cls, vec![Value::Bool(false)], Vec::new())?;
    py::set(vm, &ctx, "init_phase", Value::Bool(true))?;
    let contexts = vm.types.new_list(vec![ctx]);
    set_attr(vm, "renpy.game", "contexts", contexts)?;
    py::method(vm, &script, "load_script", Vec::new())?;
    let errors = call0(vm, "renpy.parser", "report_parse_errors")?;
    if py::truthy(vm, &errors)? {
        let c = get_attr(vm, "renpy.game", "ParseErrorException")?;
        let e = vm.call(&c, Vec::new(), Vec::new())?;
        return Err(PyErr::Exc(e.obj().cloned().ok_or_else(|| vm.type_error("ParseErrorException"))?));
    }
    let info = py::s(vm, "After loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    if py::is_none(&py::get(vm, &config, "savedir")?) {
        let sd = call0(vm, "_crysirror_renpy", "savedir")?;
        py::set(vm, &config, "savedir", sd)?;
    }
    let persistent = call0(vm, "renpy.persistent", "init")?;
    set_attr(vm, "renpy.game", "persistent", persistent.clone())?;
    call0(vm, "renpy.savelocation", "init")?;
    call0(vm, "renpy.persistent", "update")?;
    let persistent = get_attr(vm, "renpy.game", "persistent")?;
    let prefs = py::get(vm, &persistent, "_preferences")?;
    set_attr(vm, "renpy.game", "preferences", prefs.clone())?;
    set_attr(vm, "renpy.game", "seen_session", py::dict(vm))?;
    set_attr(vm, "store", "persistent", persistent)?;
    set_attr(vm, "store", "_preferences", prefs)?;
    let style = call0(vm, "renpy.style", "StyleManager")?;
    set_attr(vm, "renpy.game", "style", style.clone())?;
    set_attr(vm, "store", "style", style)?;
    let info = py::s(vm, "While executing init code:");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    set_attr(vm, "renpy.game", "init_phase", Value::Bool(true))?;
    let initcode = py::get_items(vm, &script, "initcode")?;
    let node_cls = get_attr(vm, "renpy.ast", "Node")?;
    let node_cls = node_cls.obj().cloned().ok_or_else(|| vm.type_error("renpy.ast.Node"))?;
    for entry in initcode {
        let node = ops::getitem(vm, &entry, Value::Int(1))?;
        if ops::isinstance(vm, &node, &node_cls) {
            let ctx = context(vm)?;
            py::method(vm, &ctx, "run", vec![node])?;
        } else {
            vm.call(&node, Vec::new(), Vec::new())?;
        }
    }
    set_attr(vm, "renpy.game", "init_phase", Value::Bool(false))?;
    let ctx = context(vm)?;
    py::set(vm, &ctx, "init_phase", Value::Bool(false))?;
    let info = py::s(vm, "After initialization, but before game start.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    after_init_618(vm)
}

fn main_body(vm: &mut Vm) -> PyResult<()> {
    if !at_least(vm, 6, 12) {
        return main_body_611(vm);
    }
    let info = py::s(vm, "Before loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    if at_least(vm, 6, 14) {
        call0(vm, ARGUMENTS, "pre_init")?;
    }
    if at_least(vm, 6, 17) {
        call0(vm, "renpy.sl2.slparser", "init")?;
    }
    call0(vm, "renpy.config", "init")?;
    let _ = call0(vm, "renpy.gl2.live2d", "reset");
    call0(vm, MAIN, "choose_variants")?;
    let variants = py::config(vm, "variants")?;
    let t = py::s(vm, "touch");
    let touch = ops::contains(vm, &variants, &t)?;
    set_attr(vm, "renpy.display", "touch", Value::Bool(touch))?;
    let gamedir = py::config(vm, "gamedir")?;
    set_attr(vm, "renpy.game", "basepath", gamedir.clone())?;
    let commondir = py::config(vm, "commondir")?;
    let mut sp = vec![gamedir];
    if !py::is_none(&commondir) {
        sp.push(commondir);
    }
    let sp = vm.types.new_list(sp);
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "searchpath", sp)?;
    let archives = py::get(vm, &config, "archives")?;
    py::method(vm, &archives, "reverse", Vec::new())?;
    call0(vm, "renpy.loader", "index_archives")?;
    call0(vm, "renpy.loader", "auto_init")?;
    if at_least(vm, 8, 2) {
        call0(vm, MAIN, "load_build_info")?;
    }
    log_clock(vm, "Early init")?;
    let log = call0(vm, "renpy.python", "RollbackLog")?;
    set_attr(vm, "renpy.game", "log", log)?;
    let store = py::import(vm, "store")?;
    set_attr(vm, "store", "store", store)?;
    let style = call0(vm, "renpy.style", "StyleManager")?;
    set_attr(vm, "renpy.game", "style", style.clone())?;
    set_attr(vm, "store", "style", style)?;
    let ctx_cls = get_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&ctx_cls, vec![Value::Bool(false)], Vec::new())?;
    py::set(vm, &ctx, "init_phase", Value::Bool(true))?;
    let contexts = vm.types.new_list(vec![ctx]);
    set_attr(vm, "renpy.game", "contexts", contexts)?;
    call(
        vm,
        "renpy.execution",
        "not_infinite_loop",
        vec![Value::Int(60)],
    )?;
    let info = py::s(vm, "While loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    let script = call0(vm, "renpy.script", "Script")?;
    set_attr(vm, "renpy.game", "script", script.clone())?;
    let eh = py::s(vm, "_errorhandling");
    call(vm, "renpy.exports", "load_module", vec![eh])?;
    let mut tl = false;
    for n in ["tl/None/common.rpym", "tl/None/common.rpymc"] {
        let s = py::s(vm, n);
        let l = call(vm, "renpy.exports", "loadable", vec![s])?;
        tl |= py::truthy(vm, &l)?;
    }
    if tl {
        let s = py::s(vm, "tl/None/common");
        call(vm, "renpy.exports", "load_module", vec![s])?;
    }
    // `config.init_system_styles` arrives in 6.99; 6.18 builds styles and
    // prepares screens here, before the script loads.
    let iss = py::get_or_none(vm, &config, "init_system_styles")?;
    if !py::is_none(&iss) {
        vm.call(&iss, Vec::new(), Vec::new())?;
    }
    call0(vm, "renpy.style", "build_styles")?;
    if !at_least(vm, 6, 99) {
        call0(vm, "renpy.display.screen", "prepare_screens")?;
    }
    log_clock(vm, "Loading error handling")?;
    py::method(vm, &script, "load_script", Vec::new())?;
    log_clock(vm, "Loading script")?;
    let info = py::s(vm, "After loading the script.");
    set_attr(vm, "renpy.game", "exception_info", info)?;
    if py::is_none(&py::get(vm, &config, "savedir")?) {
        let sd = call0(vm, "_crysirror_renpy", "savedir")?;
        py::set(vm, &config, "savedir", sd)?;
    }
    if at_least(vm, 8, 1) {
        call0(vm, "renpy.savetoken", "init")?;
    }
    let persistent = call0(vm, "renpy.persistent", "init")?;
    set_attr(vm, "renpy.game", "persistent", persistent.clone())?;
    let prefs = py::get(vm, &persistent, "_preferences")?;
    set_attr(vm, "renpy.game", "preferences", prefs)?;
    if at_least(vm, 7, 0) {
        count_seen_translates(vm)?;
    }
    let vs = py::get_or_none(vm, &persistent, "_virtual_size")?;
    if py::truthy(vm, &vs)? {
        let p = py::items(vm, &vs)?;
        py::set(vm, &config, "screen_width", p[0].clone())?;
        py::set(vm, &config, "screen_height", p[1].clone())?;
    }
    call0(vm, "renpy.savelocation", "init")?;
    if at_least(vm, 7, 0) {
        call0(vm, "renpy.loadsave", "init")?;
    }
    if at_least(vm, 8, 1) {
        call0(vm, "renpy.savetoken", "upgrade_all_savefiles")?;
    }
    call0(vm, "renpy.persistent", "update")?;
    let persistent = get_attr(vm, "renpy.game", "persistent")?;
    let prefs = py::get(vm, &persistent, "_preferences")?;
    set_attr(vm, "renpy.game", "preferences", prefs.clone())?;
    log_clock(vm, "Loading persistent")?;
    set_attr(vm, "renpy.game", "seen_session", py::dict(vm))?;
    set_attr(vm, "store", "persistent", persistent)?;
    set_attr(vm, "store", "_preferences", prefs)?;
    let test = get_attr(vm, "renpy.test.testast", "_test")?;
    set_attr(vm, "store", "_test", test)?;
    init_phase(vm)
}

fn init_main(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MAIN,
        &[
            ("log_clock", f_log_clock),
            ("reset_clock", f_reset_clock),
            ("run", f_run),
            ("load_rpe", f_load_rpe),
            ("load_rpe_py", f_load_rpe),
            ("choose_variants", f_choose_variants),
            ("load_build_info", f_load_build_info),
            ("main", f_main),
        ],
    );
    let now = py::f(super::link::now(vm));
    crysirror_python::modules::set(vm, &m, "last_clock", now);
    Ok(m)
}

// renpy.arguments -------------------------------------------------------------------

/// Upstream's defaults for an empty command line.
const ARG_DEFAULTS: &[(&str, Option<bool>)] = &[
    ("trace", None),
    ("compile", Some(false)),
    ("compile_python", Some(false)),
    ("keep_orphan_rpyc", Some(false)),
    ("lint", Some(false)),
    ("errors_in_editor", Some(false)),
    ("safe_mode", Some(false)),
    ("json_dump", None),
    ("json_dump_private", Some(false)),
    ("json_dump_common", Some(false)),
    ("profile_display", Some(false)),
    ("debug_image_cache", Some(false)),
    ("warp", None),
    ("savedir", None),
];

fn ns_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (k, v) in kw {
        py::set(vm, &args[0], k, v.clone())?;
    }
    Ok(Value::None)
}

fn ns_contains(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    Ok(Value::Bool(py::get_opt(vm, &args[0], &n)?.is_some()))
}

fn new_args(vm: &mut Vm) -> PyResult<Value> {
    let ns = get_attr(vm, ARGUMENTS, "Namespace")?;
    let mut kwargs: Vec<(Rc<str>, Value)> = vec![
        (Rc::from("basedir"), py::s(vm, "")),
        (Rc::from("command"), py::s(vm, "run")),
    ];
    for (k, v) in ARG_DEFAULTS {
        let v = match (k, v) {
            (&"trace", _) => Value::Int(0),
            (_, Some(b)) => Value::Bool(*b),
            (_, None) => Value::None,
        };
        kwargs.push((Rc::from(*k), v));
    }
    vm.call(&ns, Vec::new(), kwargs)
}

fn f_bootstrap(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    new_args(vm)
}

/// The `run` command: the command line is empty, so no warp or display
/// profiling is requested.
fn f_cmd_run(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let args = get_attr(vm, "renpy.game", "args")?;
    if py::is_none(&args) {
        let a = new_args(vm)?;
        set_attr(vm, "renpy.game", "args", a)?;
    }
    Ok(Value::Bool(true))
}

fn f_cmd_false(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn f_cmd_rmpersistent(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let loc = get_attr(vm, "renpy.loadsave", "location")?;
    py::method(vm, &loc, "unlink_persistent", Vec::new())?;
    set_attr(
        vm,
        "renpy.persistent",
        "should_save_persistent",
        Value::Bool(false),
    )?;
    Ok(Value::Bool(false))
}

fn f_register_command(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(args, kw, 0, "name", Value::None);
    let function = py::arg_or(args, kw, 1, "function", Value::None);
    let uses_display = py::arg_or(args, kw, 2, "uses_display", Value::Bool(false));
    let commands = get_attr(vm, ARGUMENTS, "commands")?;
    ops::setitem(vm, &commands, name.clone(), function)?;
    let display = get_attr(vm, ARGUMENTS, "display")?;
    ops::setitem(vm, &display, name, uses_display)?;
    Ok(Value::None)
}

fn f_pre_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let reg = |vm: &mut Vm, n: &str, f: Value, d: bool| -> PyResult<()> {
        let n = py::s(vm, n);
        f_register_command(vm, &[n, f, Value::Bool(d)], &[])?;
        Ok(())
    };
    let run = py::import_attr(vm, ARGUMENTS, "run")?;
    reg(vm, "run", run, true)?;
    if let Ok(lint) = py::import_attr(vm, "renpy.lint", "lint") {
        reg(vm, "lint", lint, false)?;
    }
    for n in ["compile", "quit"] {
        let f = py::import_attr(vm, ARGUMENTS, n)?;
        reg(vm, n, f, false)?;
    }
    let f = py::import_attr(vm, ARGUMENTS, "rmpersistent")?;
    reg(vm, "rmpersistent", f, false)?;
    let args = get_attr(vm, "renpy.game", "args")?;
    if py::is_none(&args) {
        let a = new_args(vm)?;
        set_attr(vm, "renpy.game", "args", a)?;
    }
    Ok(Value::None)
}

fn f_post_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let args = get_attr(vm, "renpy.game", "args")?;
    let mut command = py::get(vm, &args, "command")?;
    let lint = py::get_or_none(vm, &args, "lint")?;
    if command.as_str() == Some("run") && py::truthy(vm, &lint)? {
        command = py::s(vm, "lint");
    }
    let commands = get_attr(vm, ARGUMENTS, "commands")?;
    if !ops::contains(vm, &commands, &command)? {
        let c = py::string(vm, &command)?;
        return Err(py::error(vm, format!("Command {c} is unknown.")));
    }
    let f = ops::getitem(vm, &commands, command)?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn init_arguments(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        ARGUMENTS,
        &[
            ("run", f_cmd_run),
            ("compile", f_cmd_false),
            ("quit", f_cmd_false),
            ("rmpersistent", f_cmd_rmpersistent),
            ("register_command", f_register_command),
            ("bootstrap", f_bootstrap),
            ("pre_init", f_pre_init),
            ("post_init", f_post_init),
            ("takes_no_arguments", |_, _, _| Ok(Value::None)),
            ("clean_epic_arguments", |_, _, _| Ok(Value::None)),
            ("clean_macos_arguments", |_, _, _| Ok(Value::None)),
        ],
    );
    let ns = py::class(
        vm,
        ARGUMENTS,
        "Namespace",
        &[],
        &[("__init__", ns_init), ("__contains__", ns_contains)],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "Namespace", Value::Obj(ns));
    crysirror_python::modules::set(vm, &m, "commands", vm.types.new_dict());
    crysirror_python::modules::set(vm, &m, "display", vm.types.new_dict());
    let cc = ops::make_set(
        vm,
        ["compile", "add_from", "merge_strings"]
            .iter()
            .map(|s| vm.types.new_str(s))
            .collect(),
    )?;
    crysirror_python::modules::set(vm, &m, "compile_commands", cc);
    crysirror_python::modules::set(vm, &m, "epic_arguments", Value::None);
    Ok(m)
}

// renpy.dump ------------------------------------------------------------------------

/// `dump(error)`: the JSON dump is requested by `--json-dump`, which the
/// player never passes.
fn f_dump(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, DUMP)?;
    if {
        let t__ = py::get(vm, &m, "completed_dump")?;
        py::truthy(vm, &t__)
    }? {
        return Ok(Value::None);
    }
    py::set(vm, &m, "completed_dump", Value::Bool(true))?;
    let args = get_attr(vm, "renpy.game", "args")?;
    let jd = py::get_or_none(vm, &args, "json_dump")?;
    if py::truthy(vm, &jd)? {
        return Err(py::error(
            vm,
            "JSON dumps (--json-dump) are not supported by this runtime.",
        ));
    }
    Ok(Value::None)
}

fn f_file_exists(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let f = f.replace('\\', "/");
    let rt = state::rt(vm);
    let rel = f.strip_prefix("game/").unwrap_or(&f);
    Ok(Value::Bool(rt.files.read(rel).is_some()))
}

fn init_dump(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        DUMP,
        &[("dump", f_dump), ("file_exists", f_file_exists)],
    );
    for n in ["definitions", "transforms", "screens"] {
        crysirror_python::modules::set(vm, &m, n, vm.types.new_list(Vec::new()));
    }
    crysirror_python::modules::set(vm, &m, "file_exists_cache", vm.types.new_dict());
    crysirror_python::modules::set(vm, &m, "completed_dump", Value::Bool(false));
    Ok(m)
}

// renpy.display.presplash -------------------------------------------------------------

fn none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn init_presplash(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        PRESPLASH,
        &[
            ("start", none),
            ("pump_window", none),
            ("end", none),
            ("sleep", none),
            ("progress", none),
            ("find_file", none),
        ],
    );
    for n in ["window", "progress_bar", "progress_kind"] {
        crysirror_python::modules::set(vm, &m, n, Value::None);
    }
    crysirror_python::modules::set(vm, &m, "done", Value::Bool(true));
    crysirror_python::modules::set(vm, &m, "start_time", py::f(0.0));
    Ok(m)
}

// renpy.warp -------------------------------------------------------------------------

fn isinst(vm: &mut Vm, v: &Value, cls: &str) -> PyResult<bool> {
    let c = py::import_attr(vm, "renpy.ast", cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

/// `warp()`: runs the statements leading to `warp_spec`
/// (`filename:line`) at fast skip, then jumps there.
fn f_warp(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, WARP)?;
    let spec = py::get(vm, &m, "warp_spec")?;
    py::set(vm, &m, "warp_spec", Value::None)?;
    if py::is_none(&spec) {
        return Ok(Value::None);
    }
    let spec = py::string(vm, &spec)?;
    let Some((filename, line)) = spec.split_once(':') else {
        return Err(py::error(vm, "No : found in warp location."));
    };
    let line: i64 = line
        .trim()
        .parse()
        .map_err(|_| vm.value_error(format!("invalid literal for int(): {line:?}")))?;
    if !py::config_bool(vm, "developer")? {
        return Err(py::error(vm, "Can't warp, developer mode disabled."));
    }
    let filename = if filename.starts_with("game/") {
        filename.to_owned()
    } else {
        format!("game/{filename}")
    };
    let script = get_attr(vm, "renpy.game", "script")?;
    let nm = py::get(vm, &script, "namemap")?;
    let values = py::method(vm, &nm, "values", Vec::new())?;
    let set = call(vm, "builtins", "set", vec![values])?;
    let seen = ops::collect(vm, &set)?;
    // prev: id(next) -> (next, chosen predecessor)
    let prev = py::dict(vm);
    let file_of = |vm: &mut Vm, n: &Value| -> PyResult<String> {
        let f = py::get(vm, n, "filename")?;
        py::string(vm, &f)
    };
    let line_of = |vm: &mut Vm, n: &Value| -> PyResult<i64> {
        let l = py::get(vm, n, "linenumber")?;
        py::int(vm, &l)
    };
    let mut add = |vm: &mut Vm, node: &Value, next: Value| -> PyResult<()> {
        if py::is_none(&next) {
            return Ok(());
        }
        let d = prev.obj().cloned().unwrap();
        let old = ops::dict_get(vm, &d, &next)?;
        let Some(old) = old else {
            return ops::dict_set(vm, &d, next, node.clone());
        };
        let (nf, nl) = (file_of(vm, &next)?, line_of(vm, &next)?);
        let (af, al) = (file_of(vm, node)?, line_of(vm, node)?);
        let (bf, bl) = (file_of(vm, &old)?, line_of(vm, &old)?);
        let prefer = |p: &dyn Fn(&str, i64, &str, i64) -> bool| {
            if p(&af, al, &bf, bl) {
                Some(true)
            } else if p(&bf, bl, &af, al) {
                Some(false)
            } else {
                None
            }
        };
        let choice = prefer(&|a, _, b, _| a == nf && b != nf)
            .or_else(|| prefer(&|_, a, _, b| a <= nl && b > nl))
            .or_else(|| prefer(&|_, a, _, b| a >= b))
            .unwrap_or(true);
        ops::dict_set(vm, &d, next, if choice { node.clone() } else { old })
    };
    let items =
        |vm: &mut Vm, v: &Value, n: &str| -> PyResult<Vec<Value>> { py::get_items(vm, v, n) };
    for n in &seen {
        if isinst(vm, n, "Translate")? {
            let lang = py::get_or_none(vm, n, "language")?;
            if py::truthy(vm, &lang)? {
                continue;
            }
        }
        if isinst(vm, n, "Menu")? {
            for i in items(vm, n, "items")? {
                let b = ops::getitem(vm, &i, Value::Int(2))?;
                if !py::is_none(&b) {
                    let first = ops::getitem(vm, &b, Value::Int(0))?;
                    add(vm, n, first)?;
                }
            }
        }
        if isinst(vm, n, "Jump")? {
            let expr = py::get(vm, n, "expression")?;
            let target = py::get(vm, n, "target")?;
            if !py::truthy(vm, &expr)? && ops::contains(vm, &nm, &target)? {
                let t = ops::getitem(vm, &nm, target)?;
                add(vm, n, t)?;
                continue;
            }
        }
        if isinst(vm, n, "While")? {
            let b = py::get(vm, n, "block")?;
            let first = ops::getitem(vm, &b, Value::Int(0))?;
            add(vm, n, first)?;
        }
        if isinst(vm, n, "If")? {
            let mut seen_true = false;
            for e in items(vm, n, "entries")? {
                let c = ops::getitem(vm, &e, Value::Int(0))?;
                let b = ops::getitem(vm, &e, Value::Int(1))?;
                let first = ops::getitem(vm, &b, Value::Int(0))?;
                add(vm, n, first)?;
                seen_true |= c.as_str() == Some("True");
            }
            if seen_true {
                continue;
            }
        }
        if isinst(vm, n, "UserStatement")? {
            let nx = py::method(vm, n, "get_next", Vec::new())?;
            add(vm, n, nx)?;
        } else if let Some(nx) = py::get_opt(vm, n, "next")?
            && !py::is_none(&nx)
        {
            add(vm, n, nx)?;
        }
    }
    let mut best: Option<(i64, Value)> = None;
    for n in &seen {
        if file_of(vm, n)? == filename {
            let l = line_of(vm, n)?;
            if l <= line && best.as_ref().is_none_or(|(b, _)| l >= *b) {
                best = Some((l, n.clone()));
            }
        }
    }
    let Some((_, node)) = best else {
        return Err(py::error(
            vm,
            format!("Could not find a statement to warp to. ({spec})"),
        ));
    };
    let d = prev.obj().cloned().unwrap();
    let mut run = Vec::new();
    let mut n = node.clone();
    while let Some(p) = ops::dict_pop(vm, &d, &n)? {
        run.push(p.clone());
        n = p;
    }
    run.reverse();
    let limit = py::config(vm, "warp_limit")?;
    let limit = py::int(vm, &limit)?.max(0) as usize;
    if run.len() > limit {
        run.drain(..run.len() - limit);
    }
    let config = py::import(vm, "renpy.config")?;
    let fast = py::s(vm, "fast");
    py::set(vm, &config, "skipping", fast)?;
    for n in &run {
        let cw = py::method(vm, n, "can_warp", Vec::new())?;
        if py::truthy(vm, &cw)? {
            let _ = py::method(vm, n, "execute", Vec::new());
        }
    }
    py::set(vm, &config, "skipping", Value::None)?;
    let name = py::get(vm, &node, "name")?;
    if !at_least(vm, 7, 0) {
        return Ok(name);
    }
    set_attr(vm, "renpy.game", "after_rollback", Value::Bool(true))?;
    call0(vm, "renpy.exports", "block_rollback")?;
    let ctx = context(vm)?;
    py::method(vm, &ctx, "goto_label", vec![name.clone()])?;
    let aw = py::s(vm, "_after_warp");
    py::method(vm, &ctx, "come_from", vec![name, aw])?;
    let module = super::state::restart_module(vm);
    let rc = get_attr(vm, module, "RestartContext")?;
    let e = vm.call(&rc, Vec::new(), Vec::new())?;
    Err(PyErr::Exc(
        e.obj()
            .cloned()
            .ok_or_else(|| vm.type_error("RestartContext"))?,
    ))
}

fn init_warp(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, WARP, &[("warp", f_warp)]);
    crysirror_python::modules::set(vm, &m, "warp_spec", Value::None);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MAIN, init_main);
    vm.register_native_module(ARGUMENTS, init_arguments);
    vm.register_native_module(DUMP, init_dump);
    vm.register_native_module(PRESPLASH, init_presplash);
    vm.register_native_module(WARP, init_warp);
}
