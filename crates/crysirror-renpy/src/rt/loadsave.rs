// SPDX-License-Identifier: MPL-2.0
//! `renpy.loadsave`, after Ren'Py 8.3 `loadsave.py` (MIT): save slots as
//! zip files of the pickled `(roots, log)`, slot metadata caches and
//! autosave.
//!
//! The save record follows the release: JSON keys `_save_name` (6.x),
//! `_renpy_version` and `_version` (7.0), `_game_runtime` and `_ctime`
//! (8.2); `signatures` (8.1); `persistent.update()` before saving (8.0);
//! autosave to `auto-1` with cycling, or from 8.3 through `_auto` and
//! `config.autosave_prefix_callback`. Autosave runs on the script thread,
//! as upstream does on the web, so it never races the game.
use super::{
    persistent,
    py::{self, *},
    savestore::{self},
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.loadsave";

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm).map(|r| r.profile.at_least(major, minor)).unwrap_or(true)
}

fn m(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, MODULE)
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn location(vm: &mut Vm) -> PyResult<Value> {
    let l = mget(vm, "location")?;
    if py::is_none(&l) {
        return Err(py::error(vm, "Saving is not available before the save location is initialized."));
    }
    Ok(l)
}

fn cfg(vm: &mut Vm, n: &str) -> Value {
    py::config(vm, n).unwrap_or(Value::None)
}

fn cfg_true(vm: &mut Vm, n: &str) -> bool {
    let v = cfg(vm, n);
    py::truthy(vm, &v).unwrap_or(false)
}

fn store_true(vm: &mut Vm, n: &str) -> bool {
    super::exports::util::store_true(vm, n).unwrap_or(false)
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn bytes_of(vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
    match v.payload() {
        Some(crysirror_python::Payload::Bytes(b)) => Ok(b.to_vec()),
        _ => Ok(py::string(vm, v)?.into_bytes()),
    }
}

// SaveRecord ---------------------------------------------------------------------

fn sr_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["screenshot", "extra_info", "json", "log"].iter().enumerate() {
        py::set(vm, this, n, args.get(i + 1).cloned().unwrap_or(Value::None))?;
    }
    py::set(vm, this, "first_filename", Value::None)?;
    Ok(Value::None)
}

/// `SaveRecord.write_file(filename)`: the zip, or a copy of the file
/// this record was first written to.
fn sr_write_file(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::string(vm, &args[1])?;
    let st = savestore::store(vm);
    let first = py::get(vm, this, "first_filename")?;
    if !py::is_none(&first) {
        let f = py::string(vm, &first)?;
        if let Some(b) = st.read(&f) {
            st.write(&filename, &b).map_err(|e| vm.exc_msg("OSError", e))?;
            return Ok(Value::None);
        }
    }
    let shot = py::get(vm, this, "screenshot")?;
    let shot = if py::is_none(&shot) { None } else { Some(bytes_of(vm, &shot)?) };
    let extra = py::get(vm, this, "extra_info")?;
    let extra = py::string(vm, &extra)?;
    let json = py::get(vm, this, "json")?;
    let json = bytes_of(vm, &json)?;
    let version = py::import_attr(vm, "renpy", "version")?;
    let version = py::string(vm, &version)?;
    let log = py::get(vm, this, "log")?;
    let log = bytes_of(vm, &log)?;
    let signatures = if at_least(vm, 8, 1) { Some(super::savetoken::sign_data(vm, &log)) } else { None };
    let mut entries: Vec<(&str, &[u8])> = Vec::new();
    if let Some(s) = &shot {
        entries.push(("screenshot.png", s));
    }
    entries.push(("extra_info", extra.as_bytes()));
    entries.push(("json", &json));
    entries.push(("renpy_version", version.as_bytes()));
    entries.push(("log", &log));
    if let Some(s) = &signatures {
        entries.push(("signatures", s.as_bytes()));
    }
    let z = savestore::zip_write(&entries, savestore::dos_time(now()));
    st.write(&filename, &z).map_err(|e| vm.exc_msg("OSError", e))?;
    py::set(vm, this, "first_filename", vm.types.new_str(&filename))?;
    Ok(Value::None)
}

// Saving ------------------------------------------------------------------------

fn mutate_flag(vm: &mut Vm, set: Option<bool>) -> PyResult<bool> {
    let mut rv = false;
    for module in ["renpy.revertable", "renpy.python"] {
        let Ok(mo) = py::import(vm, module) else { continue };
        match set {
            Some(b) => py::set(vm, &mo, "mutate_flag", Value::Bool(b))?,
            None => {
                let v = py::get_opt(vm, &mo, "mutate_flag")?.unwrap_or(Value::Bool(false));
                rv |= py::truthy(vm, &v)?;
            }
        }
    }
    Ok(rv)
}

fn save_abort(vm: &mut Vm) -> PyErr {
    super::exports::util::raise(vm, MODULE, "SaveAbort", Vec::new())
}

/// `save(slotname, extra_info='', mutate_flag=False, include_screenshot=True)`.
pub fn save(vm: &mut Vm, slot: &Value, extra_info: &Value, mutate: bool, include_screenshot: bool) -> PyResult<()> {
    if !py::config_bool(vm, "save").unwrap_or(true) {
        return Ok(());
    }
    if !mutate && at_least(vm, 8, 0) {
        persistent::update(vm, false)?;
    }
    if mutate {
        mutate_flag(vm, Some(false))?;
    }
    let log = py::import_attr(vm, "renpy.game", "log")?;
    let roots = vm.call_method(&log, "freeze", vec![Value::None])?;
    let pair = vm.types.new_tuple(vec![roots.clone(), log.clone()]);
    let dumps = py::import_attr(vm, "renpy.compat.pickle", "dumps")?;
    let data = match vm.call(&dumps, vec![pair], Vec::new()) {
        Ok(d) => d,
        Err(e) => {
            if mutate {
                return Err(e);
            }
            if let Some(bad) = find_bad_reduction(vm, &roots, &log)? {
                let msg = py::error_text(vm, &e);
                return Err(py::error(vm, format!("{msg} (perhaps {bad})")));
            }
            return Err(e);
        }
    };
    if mutate && mutate_flag(vm, None)? {
        return Err(save_abort(vm));
    }
    let screenshot = if include_screenshot {
        let iface = py::import_attr(vm, "renpy.game", "interface")?;
        if py::is_none(&iface) { Value::None } else { vm.call_method(&iface, "get_screenshot", Vec::new())? }
    } else {
        Value::None
    };
    let json = py::dict(vm);
    py::dict_set(vm, &json, "_save_name", extra_info.clone())?;
    if at_least(vm, 7, 0) {
        let vt = py::import_attr(vm, "renpy", "version_tuple")?;
        let items = py::items(vm, &vt)?;
        py::dict_set(vm, &json, "_renpy_version", vm.types.new_list(items))?;
        let v = cfg(vm, "version");
        py::dict_set(vm, &json, "_version", v)?;
    }
    if at_least(vm, 8, 2) {
        let rt = super::exports::util::ex(vm, "get_game_runtime", Vec::new(), Vec::new())?;
        py::dict_set(vm, &json, "_game_runtime", rt)?;
        py::dict_set(vm, &json, "_ctime", Value::Float(now()))?;
    }
    let cbs = cfg(vm, "save_json_callbacks");
    if !py::is_none(&cbs) {
        for f in py::items(vm, &cbs)? {
            vm.call(&f, vec![json.clone()], Vec::new())?;
        }
    }
    let jd = py::import_attr(vm, "json", "dumps")?;
    let json = vm.call(&jd, vec![json], Vec::new())?;
    let sr = mget(vm, "SaveRecord")?;
    let record = vm.call(&sr, vec![screenshot, extra_info.clone(), json, data], Vec::new())?;
    let loc = location(vm)?;
    vm.call_method(&loc, "save", vec![slot.clone(), record])?;
    vm.call_method(&loc, "scan", Vec::new())?;
    clear_slot(vm, slot)?;
    Ok(())
}

fn f_save(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let extra = arg_or(&args[1..], kw, 0, "extra_info", py::s(vm, ""));
    let mutate = arg_or(&args[1..], kw, 1, "mutate_flag", Value::Bool(false));
    let shot = arg_or(&args[1..], kw, 2, "include_screenshot", Value::Bool(true));
    let (mutate, shot) = (py::truthy(vm, &mutate)?, py::truthy(vm, &shot)?);
    save(vm, &args[0], &extra, mutate, shot)?;
    Ok(Value::None)
}

/// A path to the first object in the save roots that cannot be reduced.
fn find_bad_reduction(vm: &mut Vm, roots: &Value, log: &Value) -> PyResult<Option<String>> {
    fn visit(vm: &mut Vm, o: &Value, path: &str, seen: &mut std::collections::HashSet<usize>, depth: usize) -> Option<String> {
        if depth > 64 {
            return None;
        }
        let Value::Obj(ob) = o else { return None };
        if !seen.insert(Rc::as_ptr(ob) as *const u8 as usize) {
            return None;
        }
        match o.payload() {
            Some(crysirror_python::Payload::Str(_) | crysirror_python::Payload::Bytes(_)) => return None,
            Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_)) => {
                for (i, v) in py::items(vm, o).ok()?.iter().enumerate() {
                    if let Some(r) = visit(vm, v, &format!("{path}[{i}]"), seen, depth + 1) {
                        return Some(r);
                    }
                }
                return None;
            }
            Some(crysirror_python::Payload::Dict(_)) => {
                for (k, v) in ops::mapping_items(vm, o).ok()? {
                    let kr = ops::repr(vm, &k).unwrap_or_default();
                    if let Some(r) = visit(vm, &v, &format!("{path}[{kr}]"), seen, depth + 1) {
                        return Some(r);
                    }
                }
                return None;
            }
            _ => {}
        }
        if ob.class_name() == "module" {
            let r = ops::repr(vm, o).unwrap_or_default();
            return Some(format!("{path} = {}", r.chars().take(160).collect::<String>()));
        }
        let red = match vm.call_method(o, "__reduce_ex__", vec![Value::Int(2)]) {
            Ok(r) => r,
            Err(_) => {
                let copy = py::import_attr(vm, "copy", "copy").ok()?;
                if vm.call(&copy, vec![o.clone()], Vec::new()).is_ok() {
                    return None;
                }
                let r = ops::repr(vm, o).unwrap_or_default();
                return Some(format!("{path} = {}", r.chars().take(160).collect::<String>()));
            }
        };
        if red.as_str().is_some() {
            return None;
        }
        let parts = py::items(vm, &red).ok()?;
        if let Some(state) = parts.get(2).filter(|s| !py::is_none(s)) {
            if matches!(state.payload(), Some(crysirror_python::Payload::Dict(_))) {
                for (k, v) in ops::mapping_items(vm, state).ok()? {
                    let ks = py::string(vm, &k).unwrap_or_default();
                    if let Some(r) = visit(vm, &v, &format!("{path}.{ks}"), seen, depth + 1) {
                        return Some(r);
                    }
                }
            } else if let Some(r) = visit(vm, state, &format!("{path}.__getstate__()"), seen, depth + 1) {
                return Some(r);
            }
        }
        None
    }
    let mut seen = std::collections::HashSet::new();
    for (k, v) in ops::mapping_items(vm, roots)? {
        let ks = py::string(vm, &k)?;
        if let Some(r) = visit(vm, &v, &ks, &mut seen, 0) {
            return Ok(Some(r));
        }
    }
    Ok(visit(vm, log, "renpy.game.log", &mut seen, 0))
}

// Autosave ---------------------------------------------------------------------

fn autosave_prefix(vm: &mut Vm) -> PyResult<String> {
    let cb = cfg(vm, "autosave_prefix_callback");
    if !py::is_none(&cb) {
        let r = vm.call(&cb, Vec::new(), Vec::new())?;
        return py::string(vm, &r);
    }
    Ok("auto-".into())
}

fn extra_info(vm: &mut Vm) -> PyResult<Value> {
    let f = cfg(vm, "auto_save_extra_info");
    if py::truthy(vm, &f)? {
        return vm.call(&f, Vec::new(), Vec::new());
    }
    Ok(py::s(vm, ""))
}

fn slot(vm: &Vm, s: &str) -> Value {
    vm.types.new_str(s)
}

/// The autosave itself (`autosave_thread_function`).
fn do_autosave(vm: &mut Vm, take_screenshot: bool) -> PyResult<()> {
    let count = cfg(vm, "autosave_slots");
    let count = py::int(vm, &count).unwrap_or(10);
    let prefix = autosave_prefix(vm)?;
    let extra = extra_info(vm)?;
    if take_screenshot {
        super::exports::util::ex(vm, "take_screenshot", Vec::new(), vec![("background".into(), Value::Bool(true))])?;
    }
    if at_least(vm, 8, 3) {
        save(vm, &slot(vm, "_auto"), &extra, true, true)?;
        cycle_saves(vm, &prefix, count)?;
        rename_save(vm, &slot(vm, "_auto"), &slot(vm, &format!("{prefix}1")))?;
        let mo = m(vm)?;
        py::set(vm, &mo, "did_autosave", Value::Bool(true))?;
    } else {
        cycle_saves(vm, &prefix, count)?;
        save(vm, &slot(vm, &format!("{prefix}1")), &extra, true, true)?;
    }
    let mo = m(vm)?;
    py::set(vm, &mo, "autosave_counter", Value::Int(0))?;
    Ok(())
}

fn f_autosave(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let freq = cfg(vm, "autosave_frequency");
    if !py::truthy(vm, &freq)? {
        return Ok(Value::None);
    }
    if has_autosave_off(vm) || cfg_true(vm, "skipping") {
        return Ok(Value::None);
    }
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    if ops::len(vm, &contexts)? > 1 {
        return Ok(Value::None);
    }
    let mo = m(vm)?;
    let c = py::get(vm, &mo, "autosave_counter")?;
    let c = py::int(vm, &c)? + 1;
    py::set(vm, &mo, "autosave_counter", Value::Int(c))?;
    if c < py::int(vm, &freq)? {
        return Ok(Value::None);
    }
    if store_true(vm, "main_menu") || !store_true(vm, "_autosave") {
        return Ok(Value::None);
    }
    force_autosave(vm, true, false)?;
    Ok(Value::None)
}

fn has_autosave_off(vm: &mut Vm) -> bool {
    let v = cfg(vm, "has_autosave");
    matches!(v, Value::Bool(false))
}

pub fn force_autosave(vm: &mut Vm, take_screenshot: bool, block: bool) -> PyResult<()> {
    if has_autosave_off(vm) {
        return Ok(());
    }
    let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
    let ir = super::exports::util::ex(vm, "in_rollback", Vec::new(), Vec::new())?;
    if py::truthy(vm, &ar)? || py::truthy(vm, &ir)? {
        return Ok(());
    }
    if at_least(vm, 8, 0) && !store_true(vm, "_autosave") {
        return Ok(());
    }
    if store_true(vm, "main_menu") || store_true(vm, "_in_replay") {
        return Ok(());
    }
    if block {
        let extra = extra_info(vm)?;
        let prefix = autosave_prefix(vm)?;
        let count = cfg(vm, "autosave_slots");
        let count = py::int(vm, &count).unwrap_or(10);
        cycle_saves(vm, &prefix, count)?;
        if take_screenshot {
            super::exports::util::ex(vm, "take_screenshot", Vec::new(), Vec::new())?;
        }
        save(vm, &slot(vm, &format!("{prefix}1")), &extra, false, true)?;
        return Ok(());
    }
    // Upstream ignores autosave failures.
    let _ = do_autosave(vm, take_screenshot);
    Ok(())
}

fn f_force_autosave(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "take_screenshot", Value::Bool(false));
    let b = arg_or(args, kw, 1, "block", Value::Bool(false));
    let (t, b) = (py::truthy(vm, &t)?, py::truthy(vm, &b)?);
    force_autosave(vm, t, b)?;
    Ok(Value::None)
}

// Slots --------------------------------------------------------------------------

fn get_cache(vm: &mut Vm, slot: &Value) -> PyResult<Value> {
    let cache = mget(vm, "cache")?;
    let c = vm.call_method(&cache, "get", vec![slot.clone(), Value::None])?;
    if !py::is_none(&c) {
        return Ok(c);
    }
    let cls = mget(vm, "Cache")?;
    let c = vm.call(&cls, vec![slot.clone()], Vec::new())?;
    ops::setitem(vm, &cache, slot.clone(), c.clone())?;
    Ok(c)
}

fn f_get_cache(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_cache(vm, &args[0])
}

fn clear_slot(vm: &mut Vm, slot: &Value) -> PyResult<()> {
    let c = get_cache(vm, slot)?;
    vm.call_method(&c, "clear", Vec::new())?;
    let nsc = mget(vm, "newest_slot_cache")?;
    vm.call_method(&nsc, "clear", Vec::new())?;
    let acc = mget(vm, "accessed_slots")?;
    if ops::contains(vm, &acc, slot)? {
        vm.call_method(&acc, "discard", vec![slot.clone()])?;
        super::exports::util::ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
    }
    Ok(())
}

fn f_clear_slot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    clear_slot(vm, &args[0])?;
    Ok(Value::None)
}

fn f_clear_cache(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let acc = mget(vm, "accessed_slots")?;
    vm.call_method(&acc, "clear", Vec::new())?;
    let cache = mget(vm, "cache")?;
    let vals = vm.call_method(&cache, "values", Vec::new())?;
    for c in py::items(vm, &vals)? {
        vm.call_method(&c, "clear", Vec::new())?;
    }
    let nsc = mget(vm, "newest_slot_cache")?;
    vm.call_method(&nsc, "clear", Vec::new())?;
    super::exports::util::ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
    Ok(Value::None)
}

fn unknown(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "unknown")
}

fn c_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "slotname", args[1].clone())?;
    c_clear(vm, &args[..1], &[])
}

fn c_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let u = unknown(vm)?;
    for n in ["mtime", "json", "screenshot"] {
        py::set(vm, &args[0], n, u.clone())?;
    }
    Ok(Value::None)
}

fn cached(vm: &mut Vm, this: &Value, field: &str) -> PyResult<Value> {
    let v = py::get(vm, this, field)?;
    let u = unknown(vm)?;
    if !py::same(&v, &u) {
        return Ok(v);
    }
    let slot = py::get(vm, this, "slotname")?;
    let loc = location(vm)?;
    let v = vm.call_method(&loc, field, vec![slot])?;
    py::set(vm, this, field, v.clone())?;
    Ok(v)
}

fn wrap_json(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    match d.payload() {
        Some(crysirror_python::Payload::List(_)) => {
            let mut out = Vec::new();
            for i in py::items(vm, d)? {
                out.push(wrap_json(vm, &i)?);
            }
            Ok(vm.types.new_list(out))
        }
        Some(crysirror_python::Payload::Dict(_)) => {
            let rd = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
            let n = vm.call(&rd, Vec::new(), Vec::new())?;
            for (k, v) in ops::mapping_items(vm, d)? {
                let w = wrap_json(vm, &v)?;
                ops::setitem(vm, &n, k, w)?;
            }
            Ok(n)
        }
        _ => Ok(d.clone()),
    }
}

fn f_wrap_json(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    wrap_json(vm, &args[0])
}

fn c_get_mtime(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    cached(vm, &args[0], "mtime")
}

fn c_get_json(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let j = cached(vm, &args[0], "json")?;
    if at_least(vm, 7, 4) { wrap_json(vm, &j) } else { Ok(j) }
}

fn c_get_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    cached(vm, &args[0], "screenshot")
}

fn c_preload(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for f in ["mtime", "json", "screenshot"] {
        cached(vm, &args[0], f)?;
    }
    Ok(Value::None)
}

fn slots(vm: &mut Vm, regexp: &Value) -> PyResult<Vec<Value>> {
    let loc = location(vm)?;
    let l = vm.call_method(&loc, "list", Vec::new())?;
    let mut items = py::items(vm, &l)?;
    if !py::is_none(regexp) {
        let rm = py::import_attr(vm, "re", "match")?;
        let mut kept = Vec::new();
        for i in items {
            let r = vm.call(&rm, vec![regexp.clone(), i.clone()], Vec::new())?;
            if py::truthy(vm, &r)? {
                kept.push(i);
            }
        }
        items = kept;
    }
    let mut keyed: Vec<(String, Value)> = Vec::new();
    for i in items {
        keyed.push((py::string(vm, &i)?, i));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(keyed.into_iter().map(|(_, v)| v).collect())
}

fn f_list_slots(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = arg_or(args, kw, 0, "regexp", Value::None);
    let s = slots(vm, &r)?;
    Ok(vm.types.new_list(s))
}

fn f_list_saved_games(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = arg_or(args, kw, 0, "regexp", py::s(vm, "."));
    let fast = arg_or(args, kw, 1, "fast", Value::Bool(false));
    let s = slots(vm, &r)?;
    if py::truthy(vm, &fast)? {
        return Ok(vm.types.new_list(s));
    }
    let mut out = Vec::new();
    for slot in s {
        let c = get_cache(vm, &slot)?;
        let j = c_get_json(vm, &[c.clone()], &[])?;
        let extra = if py::is_none(&j) { py::s(vm, "") } else { vm.call_method(&j, "get", vec![py::s(vm, "_save_name"), py::s(vm, "")])? };
        let shot = c_get_screenshot(vm, &[c.clone()], &[])?;
        let mt = c_get_mtime(vm, &[c], &[])?;
        out.push(vm.types.new_tuple(vec![slot, extra, shot, mt]));
    }
    Ok(vm.types.new_list(out))
}

fn f_scan_saved_game(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = get_cache(vm, &args[0])?;
    let mt = c_get_mtime(vm, &[c.clone()], &[])?;
    if py::is_none(&mt) {
        return Ok(Value::None);
    }
    let j = c_get_json(vm, &[c.clone()], &[])?;
    if py::is_none(&j) {
        return Ok(Value::None);
    }
    let extra = vm.call_method(&j, "get", vec![py::s(vm, "_save_name"), py::s(vm, "")])?;
    let shot = c_get_screenshot(vm, &[c], &[])?;
    if py::is_none(&shot) {
        return Ok(Value::None);
    }
    Ok(vm.types.new_tuple(vec![extra, shot, mt]))
}

fn f_newest_slot(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = arg_or(args, kw, 0, "regexp", Value::None);
    let nsc = mget(vm, "newest_slot_cache")?;
    let u = unknown(vm)?;
    let rv = vm.call_method(&nsc, "get", vec![r.clone(), u.clone()])?;
    if !py::same(&rv, &u) {
        return Ok(rv);
    }
    let mut best = (0.0, Value::None);
    for s in slots(vm, &r)? {
        let c = get_cache(vm, &s)?;
        let mt = c_get_mtime(vm, &[c], &[])?;
        if py::is_none(&mt) {
            continue;
        }
        let t = py::f64_of(vm, &mt)?;
        if t >= best.0 {
            best = (t, s);
        }
    }
    ops::setitem(vm, &nsc, r, best.1.clone())?;
    Ok(best.1)
}

fn accessed(vm: &mut Vm, slot: &Value) -> PyResult<()> {
    let a = mget(vm, "accessed_slots")?;
    ops::set_add(vm, &a, slot.clone())
}

fn f_slot_mtime(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    accessed(vm, &args[0])?;
    let c = get_cache(vm, &args[0])?;
    c_get_mtime(vm, &[c], &[])
}

fn f_slot_json(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    accessed(vm, &args[0])?;
    let c = get_cache(vm, &args[0])?;
    c_get_json(vm, &[c], &[])
}

fn f_slot_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    accessed(vm, &args[0])?;
    let c = get_cache(vm, &args[0])?;
    c_get_screenshot(vm, &[c], &[])
}

fn f_can_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    accessed(vm, &args[0])?;
    let c = get_cache(vm, &args[0])?;
    let mt = c_get_mtime(vm, &[c], &[])?;
    Ok(Value::Bool(py::truthy(vm, &mt)?))
}

/// `load(filename)`: unpickles a slot and restores it (`unfreeze`).
fn f_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let loc = location(vm)?;
    let r = vm.call_method(&loc, "load", vec![args[0].clone()])?;
    let log_data = if at_least(vm, 8, 1) {
        let p = py::items(vm, &r)?;
        let data = bytes_of(vm, &p[0])?;
        let sig = py::string(vm, &p[1])?;
        if !super::savetoken::check_load(vm, &data, &sig)? {
            return Ok(Value::None);
        }
        p[0].clone()
    } else {
        r
    };
    let loads = py::import_attr(vm, "renpy.compat.pickle", "loads")?;
    let pair = vm.call(&loads, vec![log_data], Vec::new())?;
    let p = py::items(vm, &pair)?;
    let (roots, log) = (p[0].clone(), p[1].clone());
    py::method_kw(vm, &log, "unfreeze", vec![roots], vec![("label".into(), py::s(vm, "_after_load"))])?;
    Ok(Value::None)
}

fn rename_save(vm: &mut Vm, old: &Value, new: &Value) -> PyResult<()> {
    let loc = location(vm)?;
    vm.call_method(&loc, "rename", vec![old.clone(), new.clone()])?;
    clear_slot(vm, old)?;
    clear_slot(vm, new)
}

fn cycle_saves(vm: &mut Vm, name: &str, count: i64) -> PyResult<()> {
    let mut i = count - 1;
    while i > 0 {
        rename_save(vm, &slot(vm, &format!("{name}{i}")), &slot(vm, &format!("{name}{}", i + 1)))?;
        i -= 1;
    }
    Ok(())
}

fn f_unlink_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let loc = location(vm)?;
    vm.call_method(&loc, "unlink", vec![args[0].clone()])?;
    clear_slot(vm, &args[0])?;
    Ok(Value::None)
}

fn f_rename_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    rename_save(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn f_copy_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let loc = location(vm)?;
    vm.call_method(&loc, "copy", vec![args[0].clone(), args[1].clone()])?;
    clear_slot(vm, &args[1])?;
    Ok(Value::None)
}

fn f_cycle_saves(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    let c = py::int(vm, &args[1])?;
    cycle_saves(vm, &n, c)?;
    Ok(Value::None)
}

fn f_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    for s in slots(vm, &Value::None)? {
        if py::string(vm, &s)?.starts_with('_') {
            continue;
        }
        let c = get_cache(vm, &s)?;
        c_preload(vm, &[c], &[])?;
    }
    Ok(Value::None)
}

fn f_safe_rename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (o, n) = (py::string(vm, &args[0])?, py::string(vm, &args[1])?);
    let st = savestore::store(vm);
    if let Some(b) = st.read(&o) {
        let _ = st.write(&n, &b);
        let _ = st.remove(&o);
    }
    Ok(Value::None)
}

fn f_save_dump(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    super::log::write(vm, "config.save_dump is not available in this runtime.");
    Ok(Value::None)
}

fn f_find_bad_reduction(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(match find_bad_reduction(vm, &args[0], &args[1])? {
        Some(s) => vm.types.new_str(&s),
        None => Value::None,
    })
}

// A set-only `threading.Event` stand-in (`autosave_not_running`).
fn ev_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_flag", Value::Bool(true))?;
    Ok(Value::None)
}

fn ev_is_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_flag")
}

fn ev_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_flag", Value::Bool(true))?;
    Ok(Value::None)
}

fn ev_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_flag", Value::Bool(false))?;
    Ok(Value::None)
}

fn ev_wait(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_flag")
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let mo = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("save", f_save),
            ("autosave", f_autosave),
            ("force_autosave", f_force_autosave),
            ("scan_saved_game", f_scan_saved_game),
            ("list_saved_games", f_list_saved_games),
            ("list_slots", f_list_slots),
            ("newest_slot", f_newest_slot),
            ("slot_mtime", f_slot_mtime),
            ("slot_json", f_slot_json),
            ("slot_screenshot", f_slot_screenshot),
            ("can_load", f_can_load),
            ("load", f_load),
            ("unlink_save", f_unlink_save),
            ("rename_save", f_rename_save),
            ("copy_save", f_copy_save),
            ("cycle_saves", f_cycle_saves),
            ("wrap_json", f_wrap_json),
            ("get_cache", f_get_cache),
            ("clear_slot", f_clear_slot),
            ("clear_cache", f_clear_cache),
            ("init", f_init),
            ("safe_rename", f_safe_rename),
            ("save_dump", f_save_dump),
            ("find_bad_reduction", f_find_bad_reduction),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &mo, n, v);
    let suffix = py::import_attr(vm, "renpy", "savegame_suffix")?;
    set(vm, "savegame_suffix", suffix);
    let exc = Value::Obj(vm.types.exc("Exception"));
    let sa = py::class(vm, MODULE, "SaveAbort", &[exc], &[], &[])?;
    set(vm, "SaveAbort", Value::Obj(sa));
    let sr = py::class(vm, MODULE, "SaveRecord", &[], &[("__init__", sr_init), ("write_file", sr_write_file)], &[])?;
    set(vm, "SaveRecord", Value::Obj(sr));
    let cache = py::class(vm, MODULE, "Cache", &[], &[("__init__", c_init), ("clear", c_clear), ("get_mtime", c_get_mtime), ("get_json", c_get_json), ("get_screenshot", c_get_screenshot), ("preload", c_preload)], &[])?;
    set(vm, "Cache", Value::Obj(cache));
    let ev = py::class(vm, MODULE, "_Event", &[], &[("__init__", ev_init), ("is_set", ev_is_set), ("set", ev_set), ("clear", ev_clear), ("wait", ev_wait)], &[])?;
    let e = vm.call(&Value::Obj(ev), Vec::new(), Vec::new())?;
    set(vm, "autosave_not_running", e);
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let u = vm.call(&sentinel, vec![py::s(vm, "unknown")], Vec::new())?;
    set(vm, "unknown", u);
    set(vm, "cache", py::dict(vm));
    set(vm, "newest_slot_cache", py::dict(vm));
    set(vm, "accessed_slots", vm.types.new_set(Default::default()));
    set(vm, "location", Value::None);
    set(vm, "autosave_thread", Value::None);
    set(vm, "autosave_counter", Value::Int(0));
    set(vm, "did_autosave", Value::Bool(false));
    for n in ["_MultiPersistent", "MultiPersistent"] {
        let v = py::import_attr(vm, "renpy.persistent", n)?;
        set(vm, n, v);
    }
    Ok(mo)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
