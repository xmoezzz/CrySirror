// SPDX-License-Identifier: MPL-2.0
//! `renpy.loader`, after Ren'Py 8.3 `loader.py` (MIT): file lookup over
//! the game's file set (`state::Files`: loose files and mounted archive
//! members, already indexed by the host) and the engine's common files.
//!
//! Search order and names follow upstream: `config.search_prefixes`,
//! with `tl/<language>/` variants first, a case-insensitive name map,
//! `config.file_open_callback` and `config.loadable_callback`. Archives
//! are mounted by the host, so `archives` is empty and `index_archives`
//! has nothing to index; there is no filesystem auto-reload. Paths given
//! out (`transfn`, `get_path`) are virtual paths under `config.gamedir`.
//! Game Python modules are served by the VM host (`boot::RtHost`), with
//! `add_python_directory` adding search prefixes.
use super::{
    py::{self, *},
    state::rt,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{collections::HashMap, rc::Rc};

const MODULE: &str = "renpy.loader";

#[derive(Default)]
struct Names {
    /// lower(name) -> (name, common).
    map: HashMap<String, (String, bool)>,
}

fn names(vm: &mut Vm) -> Rc<Names> {
    if let Some(n) = vm.ext_get::<Rc<Names>>() {
        return n.clone();
    }
    let r = rt(vm);
    let mut n = Names::default();
    for f in r.files.common_list() {
        n.map.insert(f.to_lowercase(), (f, true));
    }
    for f in r.files.list() {
        n.map.insert(f.to_lowercase(), (f, false));
    }
    let n = Rc::new(n);
    vm.ext_set(n.clone());
    n
}

/// The actual (name, common) of a game-relative file, if it exists.
pub fn resolve(vm: &mut Vm, name: &str) -> Option<(String, bool)> {
    names(vm).map.get(&name.to_lowercase()).cloned()
}

pub fn read(vm: &mut Vm, name: &str) -> Option<Vec<u8>> {
    let (n, common) = resolve(vm, name)?;
    let r = rt(vm);
    if common { r.files.common_read(&n) } else { r.files.read(&n) }
}

fn cfg(vm: &mut Vm, name: &str) -> Value {
    py::config(vm, name).unwrap_or(Value::None)
}

fn check_backslash(vm: &mut Vm, name: &str) -> PyResult<()> {
    if name.contains('\\') && py::config_bool(vm, "reject_backslash").unwrap_or(true) {
        return Err(py::error(vm, format!("Backslash in filename, use '/' instead: '{name}'")));
    }
    Ok(())
}

/// `get_prefixes(tl=True, directory=None)`.
fn prefixes(vm: &mut Vm, tl: bool, directory: Option<&str>) -> PyResult<Vec<String>> {
    let language = if tl {
        let p = py::import_attr(vm, "renpy.game", "preferences").unwrap_or(Value::None);
        let l = if py::is_none(&p) { Value::None } else { py::get_or_none(vm, &p, "language")? };
        if py::is_none(&l) { None } else { Some(py::string(vm, &l)?) }
    } else {
        None
    };
    let tld = cfg(vm, "tl_directory");
    let tld = if py::is_none(&tld) { "tl".to_owned() } else { py::string(vm, &tld)? };
    let sp = cfg(vm, "search_prefixes");
    let sp: Vec<String> = if py::is_none(&sp) {
        vec![String::new()]
    } else {
        let mut v = Vec::new();
        for p in py::items(vm, &sp)? {
            v.push(py::string(vm, &p)?);
        }
        v
    };
    let mut rv = Vec::new();
    for p in &sp {
        if let Some(l) = &language {
            rv.push(format!("{tld}/{l}/{p}"));
        }
        rv.push(p.clone());
    }
    if let Some(d) = directory {
        if let Some(l) = &language {
            rv.push(format!("{tld}/{l}/{d}/"));
        }
        rv.push(format!("{d}/"));
    }
    Ok(rv)
}

fn normalize(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    let mut slash = false;
    for c in name.chars() {
        if c == '/' {
            if slash {
                continue;
            }
            slash = true;
        } else {
            slash = false;
        }
        s.push(c);
    }
    s.trim_start_matches('/').to_owned()
}

/// The game-relative file `load(name, directory=directory)` would open:
/// (actual name, common), without reading it.
pub fn find(vm: &mut Vm, name: &str, directory: Option<&str>) -> PyResult<Option<(String, bool)>> {
    check_backslash(vm, name)?;
    let name = normalize(name);
    for p in prefixes(vm, true, directory)? {
        if let Some(r) = resolve(vm, &format!("{p}{name}")) {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

fn bytes_io(vm: &mut Vm, data: Vec<u8>, name: &str) -> PyResult<Value> {
    let c = py::import_attr(vm, "io", "BytesIO")?;
    let b = vm.types.new_bytes(data);
    let f = vm.call(&c, vec![b], Vec::new())?;
    let _ = py::set(vm, &f, "name", vm.types.new_str(name));
    Ok(f)
}

/// `load_core(name)`: the callbacks, then the file set.
fn load_core(vm: &mut Vm, name: &str) -> PyResult<Option<Value>> {
    let cbs = py::import_attr(vm, MODULE, "file_open_callbacks")?;
    for cb in py::items(vm, &cbs)? {
        let n = vm.types.new_str(name);
        let r = vm.call(&cb, vec![n], Vec::new())?;
        if !py::is_none(&r) {
            return Ok(Some(r));
        }
    }
    let foc = cfg(vm, "file_open_callback");
    if !py::is_none(&foc) {
        let n = vm.types.new_str(name);
        let r = vm.call(&foc, vec![n], Vec::new())?;
        if !py::is_none(&r) {
            return Ok(Some(r));
        }
    }
    match read(vm, name) {
        Some(d) => Ok(Some(bytes_io(vm, d, name)?)),
        None => Ok(None),
    }
}

fn f_load(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let directory = arg_or(&args[1..], kw, 0, "directory", Value::None);
    let tl = arg_or(&args[1..], kw, 1, "tl", Value::Bool(true));
    check_backslash(vm, &name)?;
    let name = normalize(&name);
    let dir = if py::is_none(&directory) { None } else { Some(py::string(vm, &directory)?) };
    let tl = py::truthy(vm, &tl)?;
    for p in prefixes(vm, tl, dir.as_deref())? {
        if let Some(f) = load_core(vm, &format!("{p}{name}"))? {
            return Ok(f);
        }
    }
    Err(vm.exc_msg("IOError", format!("Couldn't find file '{name}'.")))
}

fn loadable_core(vm: &mut Vm, name: &str) -> PyResult<bool> {
    let cache = py::import_attr(vm, MODULE, "loadable_cache")?;
    let k = vm.types.new_str(&name.to_lowercase());
    if let Some(v) = ops::dict_get(vm, cache.obj().unwrap(), &k)? {
        return py::truthy(vm, &v);
    }
    let r = resolve(vm, name).is_some();
    ops::setitem(vm, &cache, k, Value::Bool(r))?;
    Ok(r)
}

/// `loadable(name, tl=True, directory=None)`.
pub fn loadable(vm: &mut Vm, name: &str, tl: bool, directory: Option<&str>) -> PyResult<bool> {
    let name = name.trim_start_matches('/');
    let lc = cfg(vm, "loadable_callback");
    if !py::is_none(&lc) {
        let n = vm.types.new_str(name);
        let r = vm.call(&lc, vec![n], Vec::new())?;
        if py::truthy(vm, &r)? {
            return Ok(true);
        }
    }
    for p in prefixes(vm, tl, directory)? {
        if loadable_core(vm, &format!("{p}{name}"))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn f_loadable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let tl = arg_or(&args[1..], kw, 0, "tl", Value::Bool(true));
    let directory = arg_or(&args[1..], kw, 1, "directory", Value::None);
    let tl = py::truthy(vm, &tl)?;
    let dir = if py::is_none(&directory) { None } else { Some(py::string(vm, &directory)?) };
    Ok(Value::Bool(loadable(vm, &name, tl, dir.as_deref())?))
}

fn f_loadable_core(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    Ok(Value::Bool(loadable_core(vm, &name)?))
}

fn dir_of(vm: &mut Vm, common: bool) -> PyResult<String> {
    let d = cfg(vm, if common { "commondir" } else { "gamedir" });
    if py::is_none(&d) {
        return Ok(if common { "renpy/common".into() } else { "game".into() });
    }
    py::string(vm, &d)
}

/// `transfn(name)`: the (virtual) path of a loose or common file.
fn f_transfn(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let name = name.trim_start_matches('/');
    check_backslash(vm, name)?;
    match resolve(vm, name) {
        Some((n, common)) => {
            let d = dir_of(vm, common)?;
            Ok(vm.types.new_str(&format!("{}/{n}", d.trim_end_matches('/'))))
        }
        None => Err(py::error(vm, format!("Couldn't find file '{name}'."))),
    }
}

fn f_get_path(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let d = dir_of(vm, false)?;
    Ok(vm.types.new_str(&format!("{}/{}", d.trim_end_matches('/'), name.trim_start_matches('/'))))
}

fn f_get_prefixes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tl = arg_or(args, kw, 0, "tl", Value::Bool(true));
    let directory = arg_or(args, kw, 1, "directory", Value::None);
    let tl = py::truthy(vm, &tl)?;
    let dir = if py::is_none(&directory) { None } else { Some(py::string(vm, &directory)?) };
    let p = prefixes(vm, tl, dir.as_deref())?;
    Ok(vm.types.new_list(p.iter().map(|s| vm.types.new_str(s)).collect()))
}

fn f_check_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    check_backslash(vm, &name)?;
    if py::config_bool(vm, "reject_relative").unwrap_or(false) && name.split('/').any(|p| p == "." || p == "..") {
        return Err(py::error(vm, format!("Filenames may not contain relative directories like '.' and '..': '{name}'")));
    }
    Ok(Value::None)
}

fn adler32(data: &[u8], start: u32) -> u32 {
    let (mut a, mut b) = (start & 0xffff, start >> 16);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn f_get_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let cache = py::import_attr(vm, MODULE, "hash_cache")?;
    if let Some(v) = ops::dict_get(vm, cache.obj().unwrap(), &args[0])? {
        return Ok(v);
    }
    let h = read(vm, &normalize(&name)).map(|d| adler32(&d, 0)).unwrap_or(0);
    ops::setitem(vm, &cache, args[0].clone(), Value::Int(h as i64))?;
    Ok(Value::Int(h as i64))
}

/// `listdirfiles(common=True)` -> `[(directory, name)]`.
fn f_listdirfiles(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let common = arg_or(args, kw, 0, "common", Value::Bool(true));
    let common = py::truthy(vm, &common)?;
    let r = rt(vm);
    let gd = dir_of(vm, false)?;
    let mut out = Vec::new();
    let gdv = vm.types.new_str(&gd);
    for f in r.files.list() {
        let n = vm.types.new_str(&f);
        out.push(vm.types.new_tuple(vec![gdv.clone(), n]));
    }
    if common {
        let cd = dir_of(vm, true)?;
        let cdv = vm.types.new_str(&cd);
        for f in r.files.common_list() {
            let n = vm.types.new_str(&f);
            out.push(vm.types.new_tuple(vec![cdv.clone(), n]));
        }
    }
    Ok(vm.types.new_list(out))
}

fn f_add_python_directory(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut p = py::string(vm, &args[0])?;
    if !p.is_empty() && !p.ends_with('/') {
        p.push('/');
    }
    super::boot::add_python_prefix(vm, p);
    Ok(Value::None)
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn f_check_autoreload(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("load", f_load),
            ("loadable", f_loadable),
            ("loadable_core", f_loadable_core),
            ("transfn", f_transfn),
            ("get_path", f_get_path),
            ("get_prefixes", f_get_prefixes),
            ("check_name", f_check_name),
            ("get_hash", f_get_hash),
            ("listdirfiles", f_listdirfiles),
            ("add_python_directory", f_add_python_directory),
            ("init_importer", noop),
            ("quit_importer", noop),
            ("index_archives", noop),
            ("cleardirfiles", noop),
            ("scandirfiles", noop),
            ("auto_init", noop),
            ("auto_quit", noop),
            ("add_auto", noop),
            ("check_autoreload", f_check_autoreload),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in ["archives", "archive_handlers", "file_open_callbacks", "scandirfiles_callbacks", "game_files", "common_files", "apks", "game_apks", "split_apks"] {
        set(vm, n, vm.types.new_list(Vec::new()));
    }
    for n in ["loadable_cache", "hash_cache", "lower_map", "remote_files", "auto_mtimes"] {
        set(vm, n, py::dict(vm));
    }
    set(vm, "needs_autoreload", vm.types.new_set(Default::default()));
    set(vm, "reloading", Value::Bool(false));
    let exc = Value::Obj(vm.types.exc("Exception"));
    let dn = py::class(vm, MODULE, "DownloadNeeded", &[exc], &[], &[])?;
    set(vm, "DownloadNeeded", Value::Obj(dn));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
