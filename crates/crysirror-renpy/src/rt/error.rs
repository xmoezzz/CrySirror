// SPDX-License-Identifier: MPL-2.0
//! `renpy.error` (traceback reports) and `renpy.display.error` (the
//! `_exception` and `_parse_errors` screens), after Ren'Py 8.3 (MIT).
//!
//! Interpreter tracebacks are `(filename, line, name)` entries, outermost
//! first; `Context.run` prepends its script location as upstream's
//! `report_traceback` frame does. Source lines are read from the game's
//! files. There is no log directory, so `traceback.txt` and `errors.txt`
//! are kept as in-memory logs (`log::lines(vm, "traceback")`) and the
//! report is sent to the owner as a diagnostic.
//!
//! Ren'Py 8.4 replaced both modules' report API; releases from 8.4 on
//! add and use the `rt::error84` natives instead.
use super::{error84, log, py, state};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const ERROR: &str = "renpy.error";
const DISPLAY_ERROR: &str = "renpy.display.error";

/// The lines of a game or common file by its traceback name, without
/// line endings (what `linecache` serves).
pub(super) fn file_lines(vm: &Vm, filename: &str) -> Option<Vec<String>> {
    let rt = state::try_rt(vm)?;
    let f = filename.replace('\\', "/");
    let rel = f.strip_prefix("game/").unwrap_or(&f);
    let data = rt.files.read(rel).or_else(|| {
        rt.files.common_read(
            f.strip_prefix("renpy/common/")
                .or(f.strip_prefix("common/"))
                .unwrap_or(rel),
        )
    })?;
    let text = String::from_utf8_lossy(&data);
    Some(text.lines().map(str::to_owned).collect())
}

/// Line `n` (1-based) of a game file, stripped as `linecache` lines are
/// by `traceback.format_list`.
fn source_line(vm: &Vm, filename: &str, n: i64) -> String {
    file_lines(vm, filename)
        .and_then(|l| l.into_iter().nth((n.max(1) - 1) as usize))
        .map(|l| l.trim().to_owned())
        .unwrap_or_default()
}

/// `traceback_list(tb)` over the interpreter's entry list.
fn traceback_list(vm: &mut Vm, tb: &Value) -> PyResult<Vec<(String, i64, String, String)>> {
    let mut rv = Vec::new();
    if py::is_none(tb) {
        return Ok(rv);
    }
    for e in py::items(vm, tb)? {
        let p = py::items(vm, &e)?;
        let file = py::string(vm, &p[0])?;
        let line = py::int(vm, &p[1])?;
        let name = py::string(vm, &p[2])?;
        let text = match p.get(3) {
            Some(t) if !py::is_none(t) => py::string(vm, t)?,
            _ => source_line(vm, &file, line),
        };
        rv.push((file, line, name, text));
    }
    Ok(rv)
}

fn filter_traceback_list(
    tl: &[(String, i64, String, String)],
) -> Vec<(String, i64, String, String)> {
    tl.iter()
        .filter(|t| t.0.ends_with(".rpy") && !t.0.replace('\\', "/").starts_with("common/"))
        .cloned()
        .collect()
}

/// `traceback.format_list`.
fn write_traceback_list(out: &mut String, l: &[(String, i64, String, String)]) {
    for (file, line, what, text) in l {
        out.push_str(&format!("  File \"{file}\", line {line}, in {what}\n"));
        if !text.is_empty() {
            out.push_str(&format!("    {text}\n"));
        }
    }
}

fn exc_name(vm: &mut Vm, e: &Value) -> PyResult<String> {
    let t = ops::type_of(vm, e);
    let n = py::get(vm, &Value::Obj(t), "__name__")?;
    py::string(vm, &n)
}

fn ctime(vm: &mut Vm) -> PyResult<String> {
    let c = py::import_attr(vm, "time", "ctime")?;
    let v = vm.call(&c, Vec::new(), Vec::new())?;
    py::string(vm, &v)
}

/// `report_exception(e, editor=True)` -> `(simple, full, traceback_fn)`.
fn f_report_exception(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = py::arg_or(args, kw, 0, "e", Value::None);
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "debug_sound", Value::Bool(false))?;
    let tb = py::get_or_none(vm, &e, "__traceback__")?;
    let full_tl = traceback_list(vm, &tb)?;
    let simple_tl = filter_traceback_list(&full_tl);
    let info = py::import_attr(vm, "renpy.game", "exception_info")?;
    let info = ops::str(vm, &info)?.to_string();
    let name = exc_name(vm, &e)?;
    let msg = ops::str(vm, &e)?.to_string();
    let mut simple = format!("{info}\n");
    write_traceback_list(&mut simple, &simple_tl);
    simple.push_str(&format!("{name}: {msg}\n"));
    let mut full = "Full traceback:\n".to_owned();
    write_traceback_list(&mut full, &full_tl);
    full.push_str(&format!("{name}: {msg}\n"));
    vm.host.write_stdout(&format!("\n{full}\n{simple}"));
    full.push('\n');
    full.push_str("CrySirror\n");
    let v = py::import_attr(vm, "renpy", "version")?;
    full.push_str(&format!("{}\n", py::string(vm, &v)?));
    let gname = py::config(vm, "name")?;
    let gver = py::config(vm, "version")?;
    full.push_str(&format!(
        "{} {}\n",
        ops::str(vm, &gname)?,
        ops::str(vm, &gver)?
    ));
    full.push_str(&format!("{}\n", ctime(vm)?));
    let file = format!(
        "I'm sorry, but an uncaught exception occurred.\n\n{simple}\n-- Full Traceback ------------------------------------------------------------\n\n{full}"
    );
    log::push(vm, "traceback", &file);
    super::link::diagnostic(vm, format!("renpy exception: {simple}\n{full}"));
    let s = py::s(vm, &simple);
    let f = py::s(vm, &full);
    let n = py::s(vm, "traceback.txt");
    Ok(vm.types.new_tuple(vec![s, f, n]))
}

/// `open_error_file(fn, mode)`: an in-memory file whose text becomes the
/// log of the same name when closed.
fn f_open_error_file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(args, kw, 0, "fn", Value::None);
    let cls = py::import_attr(vm, ERROR, "_ErrorFile")?;
    let f = vm.call(&cls, vec![name.clone()], Vec::new())?;
    Ok(vm.types.new_tuple(vec![f, name]))
}

fn ef_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "name", args[1].clone())?;
    py::set(vm, &args[0], "parts", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn ef_write(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let parts = py::get(vm, &args[0], "parts")?;
    ops::list_append(vm, &parts, args[1].clone())?;
    Ok(Value::None)
}

fn ef_enter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn ef_close(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let parts = py::get(vm, &args[0], "parts")?;
    let mut text = String::new();
    for p in py::items(vm, &parts)? {
        text.push_str(&py::string(vm, &p)?);
    }
    let n = py::get(vm, &args[0], "name")?;
    let n = py::string(vm, &n)?;
    let base = n.rsplit_once('.').map_or(n.as_str(), |(b, _)| b).to_owned();
    log::push(vm, &base, text.trim_start_matches('\u{feff}'));
    py::set(vm, &args[0], "parts", vm.types.new_list(Vec::new()))?;
    Ok(Value::Bool(false))
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn init_error(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        ERROR,
        &[
            ("report_exception", f_report_exception),
            ("open_error_file", f_open_error_file),
        ],
    );
    let ef = py::class(
        vm,
        ERROR,
        "_ErrorFile",
        &[],
        &[
            ("__init__", ef_init),
            ("write", ef_write),
            ("__enter__", ef_enter),
            ("__exit__", ef_close),
            ("close", ef_close),
            ("flush", noop),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "_ErrorFile", Value::Obj(ef));
    crysirror_python::modules::set(vm, &m, "FSENCODING", py::s(vm, "utf-8"));
    if error84::active(vm) {
        error84::install_error(vm, &m)?;
    }
    Ok(m)
}

// renpy.display.error ------------------------------------------------------------

fn call_attr(
    vm: &mut Vm,
    module: &str,
    name: &str,
    args: Vec<Value>,
    kwargs: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, kwargs)
}

fn f_call_exception_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let config = py::import(vm, "renpy.config")?;
    let old_quit = py::get(vm, &config, "quit_action")?;
    let quit = py::import_attr(vm, "renpy.exports", "quit")?;
    py::set(vm, &config, "quit_action", quit)?;
    let r = (|| {
        let ctx = call_attr(vm, "renpy.game", "context", Vec::new(), Vec::new())?;
        let sl = py::get(vm, &ctx, "scene_lists")?;
        for layer in py::get_items(vm, &config, "layers")? {
            py::method(vm, &sl, "clear", vec![layer])?;
        }
        let mut kwargs: Vec<(Rc<str>, Value)> =
            kw.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        kwargs.push((Rc::from("_transient"), Value::Bool(true)));
        call_attr(
            vm,
            "renpy.exports",
            "show_screen",
            vec![args[0].clone()],
            kwargs,
        )?;
        let k = |s: &str, v: Value| (Rc::<str>::from(s), v);
        let kwargs = vec![
            k("mouse", py::s(vm, "screen")),
            k("type", py::s(vm, "screen")),
            k("suppress_overlay", Value::Bool(true)),
            k("suppress_underlay", Value::Bool(true)),
        ];
        call_attr(vm, "renpy.ui", "interact", Vec::new(), kwargs)
    })();
    py::set(vm, &config, "quit_action", old_quit)?;
    r
}

fn f_rollback_action(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    call_attr(
        vm,
        "renpy.exports",
        "rollback",
        Vec::new(),
        vec![(Rc::from("force"), Value::Bool(true))],
    )
}

fn f_init_display(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let config = py::import(vm, "renpy.config")?;
    let persistent = py::import_attr(vm, "renpy.game", "persistent")?;
    let gl2 = match py::get_opt(vm, &persistent, "_gl2")? {
        Some(v) if !py::is_none(&v) => v,
        _ => Value::Bool(true),
    };
    py::set(vm, &config, "gl2", gl2)?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    py::method(vm, &prefs, "check", Vec::new())?;
    let iss = py::get(vm, &config, "init_system_styles")?;
    if !py::is_none(&iss) {
        vm.call(&iss, Vec::new(), Vec::new())?;
    }
    let interface = py::import_attr(vm, "renpy.game", "interface")?;
    if !py::truthy(vm, &interface)? {
        call_attr(
            vm,
            "renpy.display.core",
            "Interface",
            Vec::new(),
            Vec::new(),
        )?;
        call_attr(vm, "renpy.loader", "index_archives", Vec::new(), Vec::new())?;
        let cache = py::import_attr(vm, "renpy.display.im", "cache")?;
        py::method(vm, &cache, "init", Vec::new())?;
    }
    let interface = py::import_attr(vm, "renpy.game", "interface")?;
    py::method(vm, &interface, "start", Vec::new())?;
    call_attr(vm, "renpy.ui", "reset", Vec::new(), Vec::new())?;
    Ok(Value::None)
}

fn f_error_dump(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    call_attr(
        vm,
        "renpy.dump",
        "dump",
        vec![Value::Bool(true)],
        Vec::new(),
    )
}

fn set_handled(vm: &mut Vm) -> PyResult<()> {
    let m = py::import(vm, DISPLAY_ERROR)?;
    py::set(vm, &m, "error_handled", Value::Bool(true))?;
    f_error_dump(vm, &[], &[])?;
    Ok(())
}

/// The first checks of both reports: true when the screen cannot be shown.
fn cannot_show(vm: &mut Vm, screen: &str) -> PyResult<bool> {
    let args = py::import_attr(vm, "renpy.game", "args")?;
    let cmd = py::get_or_none(vm, &args, "command")?;
    if cmd.as_str() != Some("run") {
        return Ok(true);
    }
    let s = py::s(vm, screen);
    let has = call_attr(vm, "renpy.exports", "has_screen", vec![s], Vec::new())?;
    Ok(!py::truthy(vm, &has)?)
}

fn is_control(vm: &mut Vm, e: &PyErr) -> bool {
    let PyErr::Exc(o) = e else { return true };
    match py::import_attr(vm, "renpy.game", "CONTROL_EXCEPTIONS") {
        Ok(c) => ops::exc_matches(vm, &Value::Obj(o.clone()), &c).unwrap_or(false),
        Err(_) => false,
    }
}

fn log_while_handling(vm: &mut Vm) {
    if let Ok(d) = py::import(vm, "renpy.display")
        && let Ok(Some(l)) = py::get_opt(vm, &d, "log")
    {
        let s = py::s(vm, "While handling exception:");
        let _ = py::method(vm, &l, "write", vec![s]);
        let _ = py::method(vm, &l, "exception", Vec::new());
    }
}

fn f_report_exception_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let short = py::arg_or(args, kw, 0, "short", Value::None);
    let full = py::arg_or(args, kw, 1, "full", Value::None);
    let traceback_fn = py::arg_or(args, kw, 2, "traceback_fn", Value::None);
    set_handled(vm)?;
    if cannot_show(vm, "_exception")? {
        return Ok(Value::Bool(true));
    }
    if f_init_display(vm, &[], &[]).is_err() {
        return Ok(Value::Bool(true));
    }
    let draw = py::import_attr(vm, "renpy.display", "draw")?;
    if py::is_none(&draw) {
        return Ok(Value::Bool(true));
    }
    let (mut ignore, mut rollback, mut reload) = (Value::None, Value::None, Value::None);
    let _ = (|| -> PyResult<()> {
        let ctx = call_attr(vm, "renpy.game", "context", Vec::new(), Vec::new())?;
        let init_phase = py::get(vm, &ctx, "init_phase")?;
        if !py::truthy(vm, &init_phase)? {
            if py::config_bool(vm, "rollback_enabled")? {
                rollback = py::import_attr(vm, DISPLAY_ERROR, "rollback_action")?;
            }
            let l = py::s(vm, "_save_reload_game");
            reload = call_attr(
                vm,
                "renpy.exports",
                "curried_call_in_new_context",
                vec![l],
                Vec::new(),
            )?;
        } else {
            reload = py::import_attr(vm, "renpy.exports", "utter_restart")?;
        }
        let top = call_attr(
            vm,
            "renpy.game",
            "context",
            vec![Value::Int(-1)],
            Vec::new(),
        )?;
        let nn = py::get(vm, &top, "next_node")?;
        if !py::is_none(&nn) {
            ignore = call_attr(
                vm,
                "renpy.ui",
                "returns",
                vec![Value::Bool(false)],
                Vec::new(),
            )?;
        }
        Ok(())
    })();
    let r = (|| -> PyResult<()> {
        let screen = py::import_attr(vm, DISPLAY_ERROR, "call_exception_screen")?;
        let k = |s: &str, v: Value| (Rc::<str>::from(s), v);
        let kwargs = vec![
            k("short", short),
            k("full", full),
            k("rollback_action", rollback),
            k("reload_action", reload),
            k("ignore_action", ignore),
            k("traceback_fn", traceback_fn),
        ];
        let name = py::s(vm, "_exception");
        call_attr(
            vm,
            "renpy.game",
            "invoke_in_new_context",
            vec![screen, name],
            kwargs,
        )?;
        let im = py::import(vm, "renpy.display.im")?;
        let ignored = py::get(vm, &im, "ignored_images")?;
        let to_ignore = py::get(vm, &im, "images_to_ignore")?;
        py::method(vm, &ignored, "update", vec![to_ignore])?;
        let store = py::import(vm, "store")?;
        let ia = py::get_or_none(vm, &store, "_ignore_action")?;
        if !py::is_none(&ia) {
            call_attr(vm, "renpy.display.behavior", "run", vec![ia], Vec::new())?;
        }
        let config = py::import(vm, "renpy.config")?;
        py::set(vm, &config, "raise_image_exceptions", Value::Bool(false))?;
        py::set(
            vm,
            &config,
            "raise_image_load_exceptions",
            Value::Bool(false),
        )?;
        Ok(())
    })();
    match r {
        Ok(()) => Ok(Value::None),
        Err(e) if is_control(vm, &e) => Err(e),
        Err(e) => {
            log_while_handling(vm);
            Err(e)
        }
    }
}

fn f_report_parse_errors(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let errors = py::arg_or(args, kw, 0, "errors", Value::None);
    let error_fn = py::arg_or(args, kw, 1, "error_fn", Value::None);
    set_handled(vm)?;
    if cannot_show(vm, "_parse_errors")? {
        return Ok(Value::Bool(true));
    }
    if py::is_none(&py::config(vm, "savedir")?) {
        return Ok(Value::Bool(true));
    }
    f_init_display(vm, &[], &[])?;
    let reload = py::import_attr(vm, "renpy.exports", "utter_restart")?;
    let screen = py::import_attr(vm, DISPLAY_ERROR, "call_exception_screen")?;
    let k = |s: &str, v: Value| (Rc::<str>::from(s), v);
    let name = py::s(vm, "_parse_errors");
    let kwargs = vec![
        k("reload_action", reload),
        k("errors", errors),
        k("error_fn", error_fn),
    ];
    match call_attr(
        vm,
        "renpy.game",
        "invoke_in_new_context",
        vec![screen, name],
        kwargs,
    ) {
        Ok(_) => Ok(Value::None),
        Err(e) if is_control(vm, &e) => Err(e),
        Err(e) => {
            log_while_handling(vm);
            Err(e)
        }
    }
}

fn init_display_error(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        DISPLAY_ERROR,
        &[
            ("call_exception_screen", f_call_exception_screen),
            ("rollback_action", f_rollback_action),
            ("init_display", f_init_display),
            ("error_dump", f_error_dump),
            ("report_exception", f_report_exception_screen),
            ("report_parse_errors", f_report_parse_errors),
        ],
    );
    crysirror_python::modules::set(vm, &m, "error_handled", Value::Bool(false));
    if error84::active(vm) {
        error84::install_display_error(vm, &m);
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(ERROR, init_error);
    vm.register_native_module(DISPLAY_ERROR, init_display_error);
}
