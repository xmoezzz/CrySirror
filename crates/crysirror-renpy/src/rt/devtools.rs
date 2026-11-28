// SPDX-License-Identifier: MPL-2.0
//! `renpy.performance` and `renpy.debug` (7.0+), after Ren'Py's
//! `performance.py` and `debug.py` (MIT).
//!
//! `performance` keeps the frame performance log (`fpl`) of
//! `config.profile`: `log`/`PPP` record `(time, depth, event, args)` and
//! `analyze` writes the per-depth timing report to `renpy.log.real_stdout`
//! and the display log. `PPP` is also entered into builtins, as upstream.
//! `debug.init_main_thread_open` replaces `open` with a reporting wrapper
//! when `RENPY_DEBUG_MAIN_THREAD_OPEN` is set.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};

const PERF: &str = "renpy.performance";
const DEBUG: &str = "renpy.debug";
const DEPTH_LEVELS: usize = 4;

fn gate(vm: &mut Vm, name: &str) -> PyResult<()> {
    if super::state::rt(vm).profile.at_least(7, 0) {
        Ok(())
    } else {
        Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{name}'")))
    }
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn perf(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, PERF)
}

fn f_clear(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = perf(vm)?;
    let l = vm.types.new_list(Vec::new());
    py::set(vm, &m, "fpl", l)?;
    py::set(vm, &m, "running", Value::Bool(true))?;
    Ok(Value::None)
}

fn log(vm: &mut Vm, depth: Value, event: Value, args: Vec<Value>) -> PyResult<Value> {
    let profile = py::config(vm, "profile")?;
    let m = perf(vm)?;
    let running = py::get(vm, &m, "running")?;
    if !crysirror_python::ops::truthy(vm, &profile)? || !crysirror_python::ops::truthy(vm, &running)? {
        return Ok(Value::None);
    }
    let fpl = py::get(vm, &m, "fpl")?;
    let a = vm.types.new_tuple(args);
    let entry = vm.types.new_tuple(vec![Value::Float(now()), depth, event, a]);
    vm.call_method(&fpl, "append", vec![entry])?;
    Ok(Value::None)
}

fn f_log(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args.len() < 2 {
        return Err(vm.type_error("log() missing required arguments: 'depth' and 'event'"));
    }
    log(vm, args[0].clone(), args[1].clone(), args[2..].to_vec())
}

/// `PPP(event, *args)` or `PPP(level, event, *args)`; upstream logs at
/// depth 3 either way.
fn f_ppp(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let Some(first) = args.first() else {
        return Err(vm.type_error("PPP() missing required argument: 'event'"));
    };
    let (event, rest) = if matches!(first, Value::Int(_)) {
        (args.get(1).cloned().unwrap_or(Value::None), args.get(2..).unwrap_or(&[]).to_vec())
    } else {
        (first.clone(), args[1..].to_vec())
    };
    log(vm, Value::Int(3), event, rest)
}

fn write_both(vm: &mut Vm, s: &str) -> PyResult<()> {
    let out = py::import_attr(vm, "renpy.log", "real_stdout")?;
    let v = vm.types.new_str(s);
    vm.call_method(&out, "write", vec![v])?;
    super::log::write(vm, s);
    Ok(())
}

fn f_analyze(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = perf(vm)?;
    let fpl = py::get(vm, &m, "fpl")?;
    let entries = py::items(vm, &fpl)?;
    if entries.is_empty() {
        return Ok(Value::None);
    }
    let frames = py::config(vm, "frames")?;
    if py::int(vm, &frames)? < 30 {
        return Ok(Value::None);
    }
    let mut rows = Vec::with_capacity(entries.len());
    for e in &entries {
        let t = py::items(vm, e)?;
        rows.push((py::f64_of(vm, &t[0])?, py::int(vm, &t[1])?, t[2].clone(), t[3].clone()));
    }
    let start = rows[0].0;
    let to_event = py::config(vm, "profile_to_event")?;
    let mut end = None;
    for (t, _, event, _) in &rows {
        if crysirror_python::ops::eq(vm, event, &to_event)? {
            end = Some(*t);
            break;
        }
    }
    let Some(end) = end else {
        return Ok(Value::None);
    };
    let profile_time = py::config(vm, "profile_time")?;
    let profile_time = py::f64_of(vm, &profile_time)?;
    let once = py::import_attr(vm, "renpy.display", "interface")
        .and_then(|i| py::get_or_none(vm, &i, "profile_once"))
        .unwrap_or(Value::Bool(false));
    if end - start < profile_time && !crysirror_python::ops::truthy(vm, &once)? {
        return Ok(Value::None);
    }
    write_both(vm, "\n")?;
    let mut times = [start; DEPTH_LEVELS];
    for (t, depth, event, args) in rows {
        let dt: Vec<f64> =
            times.iter().enumerate().map(|(i, it)| if i as i64 <= depth { 1_000_000.0 * (t - it) } else { 0.0 }).collect();
        let a = py::items(vm, &args)?;
        let text = vm.call_method(&event, "format", a)?;
        let text = py::string(vm, &text)?.replace('%', "%%");
        let col = |v: f64| {
            let r = format!("{:.0}", v);
            let r = if v >= 0.0 && !r.starts_with('-') { format!(" {r}") } else { r };
            format!("{r:>7}")
        };
        let s = format!("{} {} {} {} {}\n", col(dt[0]), col(dt[1]), col(dt[2]), col(dt[3]), text);
        write_both(vm, &s)?;
        for slot in times.iter_mut().skip(depth.max(0) as usize) {
            *slot = t;
        }
    }
    Ok(Value::None)
}

fn init_performance(vm: &mut Vm) -> PyResult<ObjRef> {
    gate(vm, PERF)?;
    let m = crysirror_python::modules::module_with(
        vm,
        PERF,
        &[("clear", f_clear), ("log", f_log), ("PPP", f_ppp), ("analyze", f_analyze)],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "fpl", vm.types.new_list(Vec::new()));
    set(vm, "DEPTH_LEVELS", Value::Int(DEPTH_LEVELS as i64));
    set(vm, "running", Value::Bool(false));
    let ppp = py::get_raw(&m, "PPP");
    let b = Value::Obj(vm.builtins_module.clone());
    py::set(vm, &b, "PPP", ppp)?;
    Ok(m)
}

// renpy.debug ---------------------------------------------------------------------------------

fn f_replacement_open(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let m = py::import(vm, DEBUG)?;
    let real = py::get(vm, &m, "real_open")?;
    let kwargs = kw.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let rv = vm.call(&real, args.to_vec(), kwargs)?;
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    if !crysirror_python::ops::truthy(vm, &contexts)? {
        return Ok(rv);
    }
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let ctx = vm.call(&ctx, Vec::new(), Vec::new())?;
    let init_phase = py::get(vm, &ctx, "init_phase")?;
    // The script thread is Ren'Py's main thread.
    if crysirror_python::ops::truthy(vm, &init_phase)? {
        return Ok(rv);
    }
    let report = py::get(vm, &m, "report")?;
    if !crysirror_python::ops::truthy(vm, &report)? {
        return Ok(rv);
    }
    let env = py::import_attr(vm, "os", "environ")?;
    let k = vm.types.new_str("RENPY_DEBUG_MAIN_THREAD_OPEN");
    let mode = crysirror_python::ops::getitem(vm, &env, k)?;
    if mode.as_str() == Some("stack") {
        py::set(vm, &m, "report", Value::Bool(false))?;
        let tb = py::import_attr(vm, "traceback", "print_stack")?;
        vm.call(&tb, Vec::new(), Vec::new())?;
        py::set(vm, &m, "report", Value::Bool(true))?;
    }
    let a = vm.types.new_tuple(args.to_vec());
    let r = crysirror_python::ops::repr(vm, &a)?;
    let when = py::import_attr(vm, "time", "strftime")?;
    let fmt = vm.types.new_str("%H:%M:%S");
    let when = vm.call(&when, vec![fmt], Vec::new())?;
    let when = py::string(vm, &when)?;
    super::log::write(vm, &format!("{when} In main thread: open{r}"));
    Ok(rv)
}

fn f_init_main_thread_open(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let env = py::import_attr(vm, "os", "environ")?;
    let k = vm.types.new_str("RENPY_DEBUG_MAIN_THREAD_OPEN");
    if !crysirror_python::ops::contains(vm, &env, &k)? {
        return Ok(Value::None);
    }
    let f = py::import_attr(vm, DEBUG, "replacement_open")?;
    let b = Value::Obj(vm.builtins_module.clone());
    py::set(vm, &b, "open", f)?;
    Ok(Value::None)
}

fn init_debug(vm: &mut Vm) -> PyResult<ObjRef> {
    gate(vm, DEBUG)?;
    let m = crysirror_python::modules::module_with(
        vm,
        DEBUG,
        &[("replacement_open", f_replacement_open), ("init_main_thread_open", f_init_main_thread_open)],
    );
    let open = py::import_attr(vm, "builtins", "open")?;
    crysirror_python::modules::set(vm, &m, "real_open", open);
    crysirror_python::modules::set(vm, &m, "report", Value::Bool(true));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(PERF, init_performance);
    vm.register_native_module(DEBUG, init_debug);
}
