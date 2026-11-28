// SPDX-License-Identifier: MPL-2.0
//! `renpy.persistent`, `renpy.preferences` and `renpy.savelocation`, after
//! Ren'Py `persistent.py`, `preferences.py` and `savelocation.py` (MIT).
//!
//! Persistent data is the zlib-compressed pickle of `Persistent` (with
//! the 8.1+ save-token signatures after the stream), kept as `persistent`
//! in the Save mount; merging, change tracking and `MultiPersistent`
//! follow upstream. `Preferences` takes the field set of the game's
//! release: the 6.18 and 6.99 classes, or for 7.x/8.x the generated
//! `Preference(...)` tables (`preference_tables.rs`), whose length sets
//! `__version__` so pickled preferences upgrade as upstream's do.
//!
//! The save location is the client's Save mount. `FileLocation`s for
//! other directories (`game/saves`, `config.extra_savedirs`) are inactive,
//! which upstream's `MultiLocation` skips. `MultiPersistent` files live
//! in `multipersistent/` of the same mount, so they are not shared with
//! other games.
use super::{
    py::{self, *},
    savestore::{self, SaveStore},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::{
    io::{Read, Write},
    rc::Rc,
};

const PERSISTENT: &str = "renpy.persistent";
const PREFERENCES: &str = "renpy.preferences";
const SAVELOCATION: &str = "renpy.savelocation";
const MAX_PERSISTENT: u64 = 64 * 1024 * 1024;

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm)
        .map(|r| r.profile.at_least(major, minor))
        .unwrap_or(true)
}

fn store(vm: &Vm) -> Rc<SaveStore> {
    savestore::store(vm)
}

fn cfg(vm: &mut Vm, name: &str) -> Value {
    py::config(vm, name).unwrap_or(Value::None)
}

fn pickle(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.compat.pickle", name)?;
    vm.call(&f, args, Vec::new())
}

fn bytes_of(vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
    match v.payload() {
        Some(crysirror_python::Payload::Bytes(b)) => Ok(b.to_vec()),
        _ => Err(vm.type_error("expected bytes")),
    }
}

fn log_exception(vm: &mut Vm, what: &str, e: &crysirror_python::PyErr) {
    let msg = py::error_text(vm, e);
    super::log::write(vm, &format!("{what}: {msg}"));
}

// ---------------------------------------------------------------------------
// Preferences.

/// The 6.x preference fields (name, default) in `__init__` order.
const PREFS_6_99: &[(&str, &str)] = &[
    ("fullscreen", "False"),
    ("skip_unseen", "False"),
    ("text_cps", "0"),
    ("afm_time", "0"),
    ("afm_enable", "True"),
    ("using_afm_enable", "False"),
    ("voice_sustain", "False"),
    ("mouse_move", "False"),
    ("show_empty_window", "True"),
    ("wait_voice", "True"),
    ("afm_after_click", "False"),
    ("transitions", "2"),
    ("video_image_fallback", "False"),
    ("skip_after_choices", "False"),
    ("volumes", "{}"),
    ("mute", "{}"),
    ("joymap", "dict()"),
    ("physical_size", "None"),
    ("virtual_size", "None"),
    ("renderer", "'auto'"),
    ("performance_test", "True"),
    ("language", "None"),
    ("self_voicing", "False"),
    ("emphasize_audio", "False"),
    ("pad_enabled", "True"),
    ("mobile_rollback_side", "'disable'"),
    ("desktop_rollback_side", "'disable'"),
];

const PREFS_6_18: &[(&str, &str)] = &[
    ("fullscreen", "False"),
    ("skip_unseen", "False"),
    ("text_cps", "0"),
    ("afm_time", "0"),
    ("afm_enable", "True"),
    ("voice_sustain", "False"),
    ("mouse_move", "False"),
    ("show_empty_window", "True"),
    ("wait_voice", "True"),
    ("afm_after_click", "False"),
    ("transitions", "2"),
    ("skip_after_choices", "False"),
    ("volumes", "{}"),
    ("mute", "{}"),
    (
        "joymap",
        "dict(joy_left='Axis 0.0 Negative', joy_right='Axis 0.0 Positive', joy_up='Axis 0.1 Negative', joy_down='Axis 0.1 Positive', joy_dismiss='Button 0.0')",
    ),
    ("physical_size", "None"),
    ("renderer", "'auto'"),
    ("performance_test", "True"),
    ("language", "None"),
    ("self_voicing", "False"),
    ("emphasize_audio", "False"),
];

/// 6.x `after_upgrade` steps: (below version, fields set).
const UPGRADE_6: &[(i64, &[(&str, &str)])] = &[
    (1, &[("mute_volumes", "0")]),
    (2, &[("using_afm_enable", "False")]),
    (3, &[("physical_size", "None")]),
    (4, &[("renderer", "'auto'"), ("performance_test", "True")]),
    (5, &[("language", "None")]),
    (6, &[("wait_voice", "True")]),
    (7, &[("voice_sustain", "False")]),
    (8, &[("mouse_move", "False")]),
    (9, &[("afm_after_click", "False")]),
    (11, &[("show_empty_window", "True")]),
    (13, &[("self_voicing", "False")]),
    (14, &[("emphasize_audio", "False")]),
    (15, &[("pad_enabled", "True")]),
    (
        17,
        &[
            ("mobile_rollback_side", "'disable'"),
            ("desktop_rollback_side", "'disable'"),
        ],
    ),
    (
        18,
        &[("virtual_size", "None"), ("video_image_fallback", "False")],
    ),
];

struct PrefTable {
    version: i64,
    /// (name, default source, types source).
    fields: Vec<(&'static str, &'static str, &'static str)>,
    table: bool,
}

fn pref_table(vm: &Vm) -> PrefTable {
    let v = super::state::try_rt(vm)
        .map(|r| r.profile.version)
        .unwrap_or([8, 3, 0, 0]);
    if v[0] < 7 {
        let (fields, version) = if (v[0], v[1]) < (6, 99) {
            (PREFS_6_18, 14)
        } else {
            (PREFS_6_99, 18)
        };
        return PrefTable {
            version,
            fields: fields.iter().map(|(n, d)| (*n, *d, "")).collect(),
            table: false,
        };
    }
    let t = super::preference_tables::TABLES;
    let e = t
        .iter()
        .rev()
        .find(|(a, b, ..)| (*a, *b) <= (v[0], v[1]))
        .unwrap_or(&t[0]);
    PrefTable {
        version: e.4.len() as i64 + e.3,
        fields: e.4.to_vec(),
        table: true,
    }
}

fn eval_literal(vm: &mut Vm, src: &str) -> PyResult<Value> {
    let ns = py::import(vm, "renpy.compat")?;
    let g = py::namespace_of(vm, &ns)?;
    let eval = py::import_attr(vm, "builtins", "eval")?;
    let s = vm.types.new_str(src);
    let g = vm.call_method(&g, "copy", Vec::new())?;
    vm.call(&eval, vec![s, g], Vec::new())
}

fn pref_init_fields(vm: &mut Vm, this: &Value, only_missing: bool) -> PyResult<()> {
    let t = pref_table(vm);
    let copy = py::import_attr(vm, "copy", "copy")?;
    for (n, d, _) in &t.fields {
        if only_missing && py::get_opt(vm, this, n)?.is_some() && namespace_has(vm, this, n)? {
            continue;
        }
        let v = eval_literal(vm, d)?;
        let v = vm.call(&copy, vec![v], Vec::new())?;
        py::set(vm, this, n, v)?;
    }
    Ok(())
}

fn namespace_has(vm: &mut Vm, this: &Value, n: &str) -> PyResult<bool> {
    let d = py::namespace_of(vm, this)?;
    let k = vm.types.new_str(n);
    ops::contains(vm, &d, &k)
}

fn p_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pref_init_fields(vm, &args[0], pref_table(vm).table)?;
    Ok(Value::None)
}

fn p_init_method(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pref_init_fields(vm, &args[0], true)?;
    Ok(Value::None)
}

fn p_reset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    pref_init_fields(vm, &args[0], false)?;
    Ok(Value::None)
}

fn p_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if pref_table(vm).table {
        pref_init_fields(vm, this, true)?;
        return Ok(Value::None);
    }
    let version = py::int(vm, &args[1])?;
    let old = !at_least(vm, 6, 99);
    for (below, fields) in UPGRADE_6 {
        if old && *below > 14 {
            break;
        }
        if version < *below {
            for (n, d) in *fields {
                let v = eval_literal(vm, d)?;
                py::set(vm, this, n, v)?;
            }
        }
    }
    Ok(Value::None)
}

fn p_check(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = pref_table(vm);
    if !t.table {
        return Ok(Value::None);
    }
    if py::get_opt(vm, this, "gl_powersave")?.is_some_and(|v| v.as_str() == Some("auto")) {
        py::set(vm, this, "gl_powersave", Value::Bool(true))?;
    }
    let mut error = Value::None;
    let isinstance = py::import_attr(vm, "builtins", "isinstance")?;
    for (n, d, types) in &t.fields {
        let mut v = py::get_opt(vm, this, n)?.unwrap_or(Value::None);
        if let Some(crysirror_python::Payload::Bytes(b)) = v.payload() {
            v = vm.types.new_str(&String::from_utf8_lossy(b));
            py::set(vm, this, n, v.clone())?;
        }
        let ty = eval_literal(vm, types)?;
        let ok = vm.call(&isinstance, vec![v.clone(), ty.clone()], Vec::new())?;
        if !py::truthy(vm, &ok)? {
            let (r, tr) = (ops::repr(vm, &v)?, ops::repr(vm, &ty)?);
            error = vm.types.new_str(&format!(
                "Preference {n} has wrong type. {r} is not of type {tr}."
            ));
            let dv = eval_literal(vm, d)?;
            py::set(vm, this, n, dv)?;
        }
    }
    Ok(error)
}

fn mixer_dict(vm: &mut Vm, this: &Value, n: &str) -> PyResult<Value> {
    py::get(vm, this, n)
}

fn preserve(vm: &mut Vm) -> bool {
    py::config_bool(vm, "preserve_volume_when_muted").unwrap_or(false)
}

fn p_set_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, mixer, volume) = (&args[0], &args[1], &args[2]);
    let nonzero = py::truthy(vm, volume)?;
    if !preserve(vm) && nonzero {
        let mute = mixer_dict(vm, this, "mute")?;
        ops::setitem(vm, &mute, mixer.clone(), Value::Bool(false))?;
    }
    let vols = mixer_dict(vm, this, "volumes")?;
    ops::setitem(vm, &vols, mixer.clone(), volume.clone())?;
    Ok(Value::None)
}

fn p_get_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, mixer) = (&args[0], &args[1]);
    let vols = mixer_dict(vm, this, "volumes")?;
    if !ops::contains(vm, &vols, mixer)? {
        return Ok(Value::Float(0.0));
    }
    if !preserve(vm) {
        let mute = mixer_dict(vm, this, "mute")?;
        let m = vm.call_method(&mute, "get", vec![mixer.clone(), Value::Bool(false)])?;
        if py::truthy(vm, &m)? {
            return Ok(Value::Float(0.0));
        }
    }
    ops::getitem(vm, &vols, mixer.clone())
}

fn db_range(vm: &mut Vm) -> f64 {
    py::config_f64(vm, "volume_db_range").unwrap_or(60.0)
}

fn p_set_mixer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut volume = py::f64_of(vm, &args[2])?;
    if volume > 0.0 {
        let r = db_range(vm);
        volume = 10f64.powf((r * volume - r) / 20.0);
    }
    p_set_volume(
        vm,
        &[args[0].clone(), args[1].clone(), Value::Float(volume)],
        &[],
    )
}

fn p_get_mixer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = p_get_volume(vm, args, &[])?;
    let v = py::f64_of(vm, &v)?;
    if v == 0.0 {
        return Ok(Value::Int(0));
    }
    let r = db_range(vm);
    Ok(Value::Float((20.0 * v.log10() + r) / r))
}

fn p_set_mute(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, mixer, mute) = (&args[0], &args[1], &args[2]);
    let m = mixer_dict(vm, this, "mute")?;
    ops::setitem(vm, &m, mixer.clone(), mute.clone())?;
    if !preserve(vm) && !py::truthy(vm, mute)? {
        let vols = mixer_dict(vm, this, "volumes")?;
        let v = vm.call_method(&vols, "get", vec![mixer.clone(), Value::Float(1.0)])?;
        if py::f64_of(vm, &v)? == 0.0 {
            ops::setitem(vm, &vols, mixer.clone(), Value::Float(1.0))?;
        }
    }
    Ok(Value::None)
}

fn p_get_mute(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, mixer) = (&args[0], &args[1]);
    let vols = mixer_dict(vm, this, "volumes")?;
    if !ops::contains(vm, &vols, mixer)? {
        return Ok(Value::Bool(false));
    }
    let m = mixer_dict(vm, this, "mute")?;
    ops::getitem(vm, &m, mixer.clone())
}

fn all_mixers(vm: &mut Vm) -> PyResult<Vec<Value>> {
    let f = py::import_attr(vm, "renpy.audio.music", "get_all_mixers")?;
    let r = vm.call(&f, Vec::new(), Vec::new())?;
    py::items(vm, &r)
}

fn p_init_mixers(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut mixers = all_mixers(vm)?;
    if at_least(vm, 8, 2) {
        mixers.push(py::s(vm, "main"));
        mixers.push(py::s(vm, "voice"));
    }
    let vols = mixer_dict(vm, this, "volumes")?;
    let mute = mixer_dict(vm, this, "mute")?;
    for m in mixers {
        vm.call_method(&vols, "setdefault", vec![m.clone(), Value::Float(1.0)])?;
        vm.call_method(&mute, "setdefault", vec![m, Value::Bool(false)])?;
    }
    Ok(Value::None)
}

fn p_get_all_mixers(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = all_mixers(vm)?;
    Ok(vm.types.new_list(m))
}

fn vars_eq(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    let (da, db) = (py::namespace_of(vm, a)?, py::namespace_of(vm, b));
    match db {
        Ok(db) => ops::eq(vm, &da, &db),
        Err(_) => Ok(false),
    }
}

fn p_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(vars_eq(vm, &args[0], &args[1])?))
}

fn p_ne(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(!vars_eq(vm, &args[0], &args[1])?))
}

fn p_init_rollback_side(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for n in ["mobile_rollback_side", "desktop_rollback_side"] {
        py::set(vm, &args[0], n, py::s(vm, "disable"))?;
    }
    Ok(Value::None)
}

fn preferences_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, PREFERENCES, &[]);
    let base = py::import_attr(vm, "renpy.object", "Object")?;
    let t = pref_table(vm);
    let methods: py::Methods = &[
        ("__init__", p_init),
        ("init", p_init_method),
        ("reset", p_reset),
        ("check", p_check),
        ("set_volume", p_set_volume),
        ("get_volume", p_get_volume),
        ("set_mixer", p_set_mixer),
        ("get_mixer", p_get_mixer),
        ("set_mute", p_set_mute),
        ("get_mute", p_get_mute),
        ("init_mixers", p_init_mixers),
        ("get_all_mixers", p_get_all_mixers),
        ("init_rollback_side", p_init_rollback_side),
        ("__eq__", p_eq),
        ("__ne__", p_ne),
    ];
    let c = py::class(
        vm,
        PREFERENCES,
        "Preferences",
        &[base],
        methods,
        &[("__version__", Value::Int(t.version))],
    )?;
    let f = vm.types.new_builtin("after_upgrade", p_after_upgrade);
    vm.types.set_attr(&c, "after_upgrade", f);
    // `Preferences` defines `__eq__`; instances are not hashed upstream.
    vm.types.set_attr(&c, "__hash__", Value::None);
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "Preferences", Value::Obj(c.clone()));
    if t.table {
        let pc = py::class(vm, PREFERENCES, "Preference", &[], &[], &[])?;
        let mut all = Vec::new();
        for (n, d, ty) in &t.fields {
            let p = vm.call(&Value::Obj(pc.clone()), Vec::new(), Vec::new())?;
            py::set(vm, &p, "name", vm.types.new_str(n))?;
            let dv = eval_literal(vm, d)?;
            py::set(vm, &p, "default", dv)?;
            let tv = eval_literal(vm, ty)?;
            py::set(vm, &p, "types", tv)?;
            all.push(p);
        }
        set(vm, "Preference", Value::Obj(pc));
        set(vm, "all_preferences", vm.types.new_list(all));
    }
    let game = py::import(vm, "renpy.game")?;
    py::set(vm, &game, "Preferences", Value::Obj(c.clone()))?;
    let p = vm.call(&Value::Obj(c), Vec::new(), Vec::new())?;
    py::set(vm, &game, "preferences", p)?;
    Ok(m)
}

// ---------------------------------------------------------------------------
// Persistent.

fn pe_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "_update", Vec::new())?;
    Ok(Value::None)
}

fn pe_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::namespace_of(vm, &args[0])?;
    vm.call_method(&d, "update", vec![args[1].clone()])?;
    Ok(Value::None)
}

fn pe_getstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::namespace_of(vm, &args[0])
}

fn pe_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    if n.starts_with("__") && n.ends_with("__") {
        return Err(vm.attr_error(format!("Persistent object has no attribute '{n}'")));
    }
    Ok(Value::None)
}

fn pe_hasattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::namespace_of(vm, &args[0])?;
    Ok(Value::Bool(ops::contains(vm, &d, &args[1])?))
}

fn pe_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let progress = arg_or(&args[1..], kw, 0, "progress", Value::Bool(false));
    let d = py::namespace_of(vm, this)?;
    let keys = vm.call_method(&d, "keys", Vec::new())?;
    for k in py::items(vm, &keys)? {
        if py::string(vm, &k)?.starts_with('_') {
            continue;
        }
        ops::delitem(vm, &d, k)?;
    }
    if py::truthy(vm, &progress)? {
        for n in ["_seen_ever", "_seen_images", "_chosen", "_seen_audio"] {
            let v = py::get(vm, this, n)?;
            vm.call_method(&v, "clear", Vec::new())?;
        }
    }
    if at_least(vm, 7, 0) {
        let f = py::import_attr(vm, "renpy.exports", "execute_default_statement")?;
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn pe_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let get = |vm: &mut Vm, n: &str| py::get(vm, this, n);
    if py::is_none(&get(vm, "_preferences")?) {
        let c = py::import_attr(vm, PREFERENCES, "Preferences")?;
        let p = vm.call(&c, Vec::new(), Vec::new())?;
        py::set(vm, this, "_preferences", p)?;
    }
    for n in ["_seen_ever", "_seen_images", "_chosen", "_seen_audio"] {
        let v = get(vm, n)?;
        if !py::truthy(vm, &v)? {
            py::set(vm, this, n, py::dict(vm))?;
        }
    }
    if at_least(vm, 8, 0) {
        let sa = get(vm, "_seen_audio")?;
        let nd = py::dict(vm);
        for k in py::items(vm, &sa)? {
            let s = ops::str(vm, &k)?;
            ops::setitem(vm, &nd, vm.types.new_str(&s), Value::Bool(true))?;
        }
        py::set(vm, this, "_seen_audio", nd)?;
    }
    let st = get(vm, "_seen_translates")?;
    if !py::truthy(vm, &st)? {
        py::set(
            vm,
            this,
            "_seen_translates",
            vm.types.new_set(Default::default()),
        )?;
    }
    if py::is_none(&get(vm, "_changed")?) {
        let d = py::dict(vm);
        if at_least(vm, 6, 99) {
            for n in [
                "_preferences",
                "_seen_ever",
                "_chosen",
                "_seen_audio",
                "_seen_translates",
            ] {
                py::dict_set(vm, &d, n, Value::Int(0))?;
            }
        }
        py::set(vm, this, "_changed", d)?;
    }
    Ok(Value::None)
}

fn persistent_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, PERSISTENT, "Persistent")
}

fn safe_deepcopy(vm: &mut Vm, o: &Value) -> PyResult<Value> {
    let dc = py::import_attr(vm, "copy", "deepcopy")?;
    let rv = vm.call(&dc, vec![o.clone()], Vec::new())?;
    if !ops::eq(vm, o, &rv)? {
        if py::config_bool(vm, "developer").unwrap_or(false) {
            let r = ops::repr(vm, o)?;
            return Err(py::error(
                vm,
                format!("To be persisted, {r} must support equality comparison."),
            ));
        }
        return Ok(o.clone());
    }
    Ok(rv)
}

fn backup(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, PERSISTENT, "backup")
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `find_changes()`: whether persistent data changed since the backup.
fn find_changes(vm: &mut Vm) -> PyResult<bool> {
    let mut rv = false;
    let t = Value::Float(now());
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    let pvars = py::namespace_of(vm, &p)?;
    let b = backup(vm)?;
    let mut fields: Vec<Value> = py::items(vm, &b)?;
    for k in py::items(vm, &pvars)? {
        let mut found = false;
        for f in &fields {
            if ops::eq(vm, f, &k)? {
                found = true;
                break;
            }
        }
        if !found {
            fields.push(k);
        }
    }
    let changed = py::get(vm, &p, "_changed")?;
    for f in fields {
        if f.as_str() == Some("_changed") {
            continue;
        }
        let old = vm.call_method(&b, "get", vec![f.clone(), Value::None])?;
        let new = vm.call_method(&pvars, "get", vec![f.clone(), Value::None])?;
        if !ops::eq(vm, &new, &old)? {
            ops::setitem(vm, &changed, f.clone(), t.clone())?;
            let c = safe_deepcopy(vm, &new)?;
            ops::setitem(vm, &b, f, c)?;
            rv = true;
        }
    }
    Ok(rv)
}

/// Decodes stored persistent bytes (zlib pickle, then signatures).
fn decode(vm: &mut Vm, bytes: &[u8]) -> PyResult<Option<Value>> {
    let mut d = ZlibDecoder::new(bytes);
    let mut raw = Vec::new();
    if (&mut d).take(MAX_PERSISTENT).read_to_end(&mut raw).is_err() {
        return Ok(None);
    }
    let used = d.total_in() as usize;
    let signatures = String::from_utf8_lossy(&bytes[used.min(bytes.len())..]).into_owned();
    if at_least(vm, 8, 1) && !super::savetoken::check_persistent(vm, &raw, &signatures) {
        return Ok(None);
    }
    let b = vm.types.new_bytes(raw);
    let p = match pickle(vm, "loads", vec![b]) {
        Ok(p) => p,
        Err(e) => {
            log_exception(vm, "Loading persistent", &e);
            return Ok(None);
        }
    };
    vm.call_method(&p, "_update", Vec::new())?;
    Ok(Some(p))
}

/// `load(filename)` for a Save-mount name.
fn load_file(vm: &mut Vm, name: &str) -> PyResult<Option<Value>> {
    let Some(bytes) = store(vm).read(name) else {
        return Ok(None);
    };
    decode(vm, &bytes)
}

fn f_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    Ok(load_file(vm, &n)?.unwrap_or(Value::None))
}

fn f_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = match load_file(vm, "persistent.new")? {
        Some(p) => p,
        None => match load_file(vm, "persistent")? {
            Some(p) => p,
            None => {
                let c = persistent_class(vm)?;
                vm.call(&c, Vec::new(), Vec::new())?
            }
        },
    };
    let b = backup(vm)?;
    let d = py::namespace_of(vm, &p)?;
    for (k, v) in ops::mapping_items(vm, &d)? {
        let c = safe_deepcopy(vm, &v)?;
        ops::setitem(vm, &b, k, c)?;
    }
    Ok(p)
}

fn f_register_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = py::import_attr(vm, PERSISTENT, "registry")?;
    ops::setitem(vm, &r, args[0].clone(), args[1].clone())?;
    Ok(Value::None)
}

fn f_default_merge(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[1].clone())
}

fn f_dictset_merge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let current = &args[2];
    vm.call_method(current, "update", vec![args[0].clone()])?;
    vm.call_method(current, "update", vec![args[1].clone()])?;
    Ok(current.clone())
}

/// `merge(other)`: newest-wins per field, through registered merges.
fn merge(vm: &mut Vm, other: &Value) -> PyResult<()> {
    let t = now();
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    let pvars = py::namespace_of(vm, &p)?;
    let ovars = py::namespace_of(vm, other)?;
    let mut fields = py::items(vm, &pvars)?;
    for k in py::items(vm, &ovars)? {
        if !ops::contains(vm, &pvars, &k)? {
            fields.push(k);
        }
    }
    let pc = py::get(vm, &p, "_changed")?;
    let oc = py::get(vm, other, "_changed")?;
    let registry = py::import_attr(vm, PERSISTENT, "registry")?;
    let default = py::import_attr(vm, PERSISTENT, "default_merge")?;
    let b = backup(vm)?;
    for f in fields {
        let pval = vm.call_method(&pvars, "get", vec![f.clone(), Value::None])?;
        let oval = vm.call_method(&ovars, "get", vec![f.clone(), Value::None])?;
        if ops::eq(vm, &pval, &oval)? {
            continue;
        }
        let pt = vm.call_method(&pc, "get", vec![f.clone(), Value::Int(0)])?;
        let ot = vm.call_method(&oc, "get", vec![f.clone(), Value::Int(0)])?;
        let pt = py::f64_of(vm, &pt)?;
        let ot = py::f64_of(vm, &ot)?.min(t);
        let (new, old, tt) = if pt >= ot {
            (pval.clone(), oval, pt)
        } else {
            (oval, pval.clone(), ot)
        };
        let mf = vm.call_method(&registry, "get", vec![f.clone(), default.clone()])?;
        let val = vm.call(&mf, vec![old, new, pval], Vec::new())?;
        ops::setitem(vm, &pvars, f.clone(), val.clone())?;
        let c = safe_deepcopy(vm, &val)?;
        ops::setitem(vm, &b, f.clone(), c)?;
        ops::setitem(vm, &pc, f, Value::Float(tt))?;
    }
    Ok(())
}

fn f_merge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    merge(vm, &args[0])?;
    Ok(Value::None)
}

fn location(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.loadsave", "location")
}

fn persistent_mtime(vm: &mut Vm) -> PyResult<f64> {
    let v = py::import_attr(vm, PERSISTENT, "persistent_mtime")?;
    py::f64_of(vm, &v)
}

fn set_persistent_mtime(vm: &mut Vm, t: f64) -> PyResult<()> {
    let m = py::import(vm, PERSISTENT)?;
    py::set(vm, &m, "persistent_mtime", Value::Float(t))
}

fn load_pairs(vm: &mut Vm) -> PyResult<Vec<(f64, Value)>> {
    let loc = location(vm)?;
    if py::is_none(&loc) {
        return Ok(Vec::new());
    }
    let pairs = vm.call_method(&loc, "load_persistent", Vec::new())?;
    let mut out = Vec::new();
    for p in py::items(vm, &pairs)? {
        let it = py::items(vm, &p)?;
        out.push((py::f64_of(vm, &it[0])?, it[1].clone()));
    }
    Ok(out)
}

/// `update(force_save=False)`.
pub fn update(vm: &mut Vm, force_save: bool) -> PyResult<()> {
    let need_save = find_changes(vm)? || force_save;
    let mut pairs = load_pairs(vm)?;
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pm = persistent_mtime(vm)?;
    let mut mtime = pm;
    for (t, other) in pairs {
        mtime = t;
        if t <= pm || py::is_none(&other) {
            continue;
        }
        merge(vm, &other)?;
    }
    set_persistent_mtime(vm, mtime)?;
    if need_save {
        save(vm)?;
    }
    Ok(())
}

fn f_update(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(args, kw, 0, "force_save", Value::Bool(false));
    let f = py::truthy(vm, &f)?;
    update(vm, f)?;
    Ok(Value::None)
}

fn f_check_update(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let pm = persistent_mtime(vm)?;
    if load_pairs(vm)?.iter().any(|(t, _)| *t > pm) {
        update(vm, false)?;
        let f = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

/// `save()`.
pub fn save(vm: &mut Vm) -> PyResult<()> {
    if !py::config_bool(vm, "save_persistent").unwrap_or(true) {
        return Ok(());
    }
    let ssp = py::import_attr(vm, PERSISTENT, "should_save_persistent")?;
    if !py::truthy(vm, &ssp)? {
        return Ok(());
    }
    let r = (|| -> PyResult<()> {
        let p = py::import_attr(vm, "renpy.game", "persistent")?;
        let data = pickle(vm, "dumps", vec![p])?;
        let data = bytes_of(vm, &data)?;
        let mut e = ZlibEncoder::new(Vec::new(), Compression::new(3));
        e.write_all(&data)
            .map_err(|err| py::error(vm, err.to_string()))?;
        let mut compressed = e.finish().map_err(|err| py::error(vm, err.to_string()))?;
        if at_least(vm, 8, 1) {
            compressed.extend(super::savetoken::sign_data(vm, &data).into_bytes());
        }
        let loc = location(vm)?;
        let b = vm.types.new_bytes(compressed);
        vm.call_method(&loc, "save_persistent", vec![b])?;
        Ok(())
    })();
    if let Err(e) = r {
        if py::config_bool(vm, "developer").unwrap_or(false) {
            return Err(e);
        }
    }
    let mut pm = persistent_mtime(vm)?;
    for (t, _) in load_pairs(vm)? {
        pm = pm.max(t);
    }
    set_persistent_mtime(vm, pm)
}

fn f_save(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    save(vm)?;
    Ok(Value::None)
}

fn f_safe_deepcopy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    safe_deepcopy(vm, &args[0])
}

fn f_find_changes(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(find_changes(vm)?))
}

// MultiPersistent ---------------------------------------------------------------

fn mp_getstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::namespace_of(vm, &args[0])?;
    let s = vm.call_method(&d, "copy", Vec::new())?;
    for n in ["_filename", "_name", "_save_on_quit"] {
        let k = vm.types.new_str(n);
        if ops::contains(vm, &s, &k)? {
            ops::delitem(vm, &s, k)?;
        }
    }
    Ok(s)
}

fn mp_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    if n.starts_with("__") && n.ends_with("__") {
        return Err(vm.attr_error(n));
    }
    Ok(Value::None)
}

fn mp_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = py::get(vm, this, "_filename")?;
    let f = py::string(vm, &f)?;
    let data = pickle(vm, "dumps", vec![this.clone()])?;
    let data = bytes_of(vm, &data)?;
    if let Err(e) = store(vm).write(&f, &data) {
        if py::config_bool(vm, "developer").unwrap_or(false) {
            return Err(vm.exc_msg("OSError", e));
        }
    }
    Ok(Value::None)
}

fn mp_instances(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, PERSISTENT, "MP_instances")
}

fn f_multi_persistent(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let soq = arg_or(&args[1..], kw, 0, "save_on_quit", Value::Bool(false));
    let ctx = super::exports::util::ctx(vm)?;
    let ip = py::get(vm, &ctx, "init_phase")?;
    if !py::truthy(vm, &ip)? {
        return Err(py::error(
            vm,
            "MultiPersistent objects must be created during the init phase.",
        ));
    }
    let inst = mp_instances(vm)?;
    for i in py::items(vm, &inst)? {
        let n = py::get_or_none(vm, &i, "_name")?;
        if n.as_str() == Some(name.as_str()) {
            return Ok(i);
        }
    }
    let file = format!("multipersistent/{name}");
    let cls = py::import_attr(vm, PERSISTENT, "_MultiPersistent")?;
    let mut rv = Value::None;
    if let Some(data) = store(vm).read(&file) {
        let b = vm.types.new_bytes(data);
        match pickle(vm, "loads", vec![b]) {
            Ok(v) => rv = v,
            Err(e) => log_exception(vm, &format!("Loading MultiPersistent at '{file}'"), &e),
        }
    }
    if py::is_none(&rv) {
        rv = vm.call(&cls, Vec::new(), Vec::new())?;
    }
    py::set(vm, &rv, "_filename", vm.types.new_str(&file))?;
    py::set(vm, &rv, "_name", vm.types.new_str(&name))?;
    py::set(vm, &rv, "_save_on_quit", soq)?;
    vm.call_method(&inst, "append", vec![rv.clone()])?;
    Ok(rv)
}

fn f_save_mp(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let inst = mp_instances(vm)?;
    for i in py::items(vm, &inst)? {
        vm.call_method(&i, "save", Vec::new())?;
    }
    Ok(Value::None)
}

fn f_save_on_quit_mp(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let inst = mp_instances(vm)?;
    for i in py::items(vm, &inst)? {
        let s = py::get_or_none(vm, &i, "_save_on_quit")?;
        if py::truthy(vm, &s)? {
            vm.call_method(&i, "save", Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn f_get_mp(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let inst = mp_instances(vm)?;
    for i in py::items(vm, &inst)? {
        let n = py::get_or_none(vm, &i, "_name")?;
        if ops::eq(vm, &n, &args[0])? {
            return Ok(i);
        }
    }
    Ok(Value::None)
}

fn persistent_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        PERSISTENT,
        &[
            ("safe_deepcopy", f_safe_deepcopy),
            ("find_changes", f_find_changes),
            ("load", f_load),
            ("init", f_init),
            ("register_persistent", f_register_persistent),
            ("default_merge", f_default_merge),
            ("dictset_merge", f_dictset_merge),
            ("merge", f_merge),
            ("check_update", f_check_update),
            ("update", f_update),
            ("save", f_save),
            ("save_MP", f_save_mp),
            ("save_on_quit_MP", f_save_on_quit_mp),
            ("get_MP", f_get_mp),
            ("MultiPersistent", f_multi_persistent),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let pc = py::class(
        vm,
        PERSISTENT,
        "Persistent",
        &[],
        &[
            ("__init__", pe_init),
            ("__setstate__", pe_setstate),
            ("__getstate__", pe_getstate),
            ("__getattr__", pe_getattr),
            ("_hasattr", pe_hasattr),
            ("_clear", pe_clear),
            ("_update", pe_update),
        ],
        &[],
    )?;
    set(vm, "Persistent", Value::Obj(pc.clone()));
    let mp = py::class(
        vm,
        PERSISTENT,
        "_MultiPersistent",
        &[],
        &[
            ("__getstate__", mp_getstate),
            ("__setstate__", pe_setstate),
            ("__getattr__", mp_getattr),
            ("save", mp_save),
        ],
        &[
            ("_filename", py::s(vm, "")),
            ("_name", py::s(vm, "")),
            ("_save_on_quit", Value::Bool(false)),
        ],
    )?;
    set(vm, "_MultiPersistent", Value::Obj(mp.clone()));
    set(vm, "backup", py::dict(vm));
    let reg = py::dict(vm);
    let dsm = py::get_raw(&m, "dictset_merge");
    for n in ["_seen_ever", "_seen_images", "_seen_audio", "_chosen"] {
        py::dict_set(vm, &reg, n, dsm.clone())?;
    }
    set(vm, "registry", reg);
    set(vm, "persistent_mtime", Value::Int(0));
    set(vm, "should_save_persistent", Value::Bool(true));
    set(vm, "MP_instances", vm.types.new_list(Vec::new()));
    let game = py::import(vm, "renpy.game")?;
    py::set(vm, &game, "Persistent", Value::Obj(pc.clone()))?;
    let p = vm.call(&Value::Obj(pc), Vec::new(), Vec::new())?;
    py::set(vm, &game, "persistent", p)?;
    let _ = mp;
    Ok(m)
}

// ---------------------------------------------------------------------------
// Save locations.

fn suffix(vm: &mut Vm) -> PyResult<String> {
    let s = py::import_attr(vm, "renpy", "savegame_suffix")?;
    py::string(vm, &s)
}

fn clear_slot(vm: &mut Vm, slot: &Value) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.loadsave", "clear_slot")?;
    vm.call(&f, vec![slot.clone()], Vec::new())?;
    Ok(())
}

fn active(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let a = py::get(vm, this, "active")?;
    py::truthy(vm, &a)
}

fn fl_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let dir = args.get(1).cloned().unwrap_or(Value::None);
    let savedir = py::config(vm, "savedir").unwrap_or(Value::None);
    // Only the save directory maps to the client's Save mount.
    let is_save = py::is_none(&dir) || ops::eq(vm, &dir, &savedir)?;
    py::set(vm, this, "directory", dir)?;
    py::set(vm, this, "active", Value::Bool(is_save))?;
    py::set(vm, this, "mtimes", py::dict(vm))?;
    py::set(vm, this, "persistent", py::s(vm, "persistent"))?;
    py::set(vm, this, "persistent_mtime", Value::Int(0))?;
    py::set(vm, this, "persistent_data", Value::None)?;
    Ok(Value::None)
}

fn fl_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[1])?;
    let sfx = suffix(vm)?;
    Ok(vm.types.new_str(&format!("{s}{sfx}")))
}

fn slot_file(vm: &mut Vm, slot: &Value) -> PyResult<String> {
    let s = py::string(vm, slot)?;
    Ok(format!("{s}{}", suffix(vm)?))
}

fn fl_scan(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if !active(vm, this)? {
        return Ok(Value::None);
    }
    let st = store(vm);
    let sfx = suffix(vm)?;
    let old = py::get(vm, this, "mtimes")?;
    let new = py::dict(vm);
    for f in st.list("") {
        let Some(slot) = f.strip_suffix(&sfx) else {
            continue;
        };
        if let Some(t) = st.mtime(&f) {
            py::dict_set(vm, &new, slot, Value::Float(t))?;
        }
    }
    py::set(vm, this, "mtimes", new.clone())?;
    for (slot, t) in ops::mapping_items(vm, &new)? {
        let o = vm.call_method(&old, "get", vec![slot.clone(), Value::None])?;
        if !ops::eq(vm, &o, &t)? {
            clear_slot(vm, &slot)?;
        }
    }
    for slot in py::items(vm, &old)? {
        if !ops::contains(vm, &new, &slot)? {
            clear_slot(vm, &slot)?;
        }
    }
    let pm = py::get(vm, this, "persistent_mtime")?;
    let pm = py::f64_of(vm, &pm)?;
    for pfn in ["persistent.new", "persistent"] {
        let Some(t) = st.mtime(pfn) else { continue };
        if t != pm {
            if let Some(d) = load_file(vm, pfn)? {
                py::set(vm, this, "persistent_mtime", Value::Float(t))?;
                py::set(vm, this, "persistent_data", d)?;
                break;
            }
        }
    }
    Ok(Value::None)
}

fn fl_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, slot, record) = (&args[0], &args[1], &args[2]);
    let f = slot_file(vm, slot)?;
    let name = vm.types.new_str(&f);
    vm.call_method(record, "write_file", vec![name])?;
    fl_scan(vm, &[this.clone()], &[])
}

fn fl_list(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::get(vm, &args[0], "mtimes")?;
    let k = py::items(vm, &m)?;
    Ok(vm.types.new_list(k))
}

fn fl_list_files(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::get(vm, &args[0], "mtimes")?;
    let mut out = Vec::new();
    for s in py::items(vm, &m)? {
        let f = slot_file(vm, &s)?;
        out.push(vm.types.new_str(&f));
    }
    Ok(vm.types.new_list(out))
}

fn fl_mtime(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::get(vm, &args[0], "mtimes")?;
    vm.call_method(&m, "get", vec![args[1].clone(), Value::None])
}

fn fl_path(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[1])?;
    match store(vm).mtime(&f) {
        Some(t) => Ok(vm
            .types
            .new_tuple(vec![Value::Float(t), vm.types.new_str(&f)])),
        None => Ok(vm.types.new_tuple(vec![Value::Int(0), Value::None])),
    }
}

fn slot_bytes(vm: &mut Vm, slot: &Value) -> PyResult<Option<Vec<u8>>> {
    let f = slot_file(vm, slot)?;
    Ok(store(vm).read(&f))
}

fn fl_json(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let Some(b) = slot_bytes(vm, &args[1])? else {
        return Ok(Value::None);
    };
    let Ok(entries) = savestore::zip_read(&b) else {
        return Ok(Value::None);
    };
    if let Some(j) = entries.iter().find(|e| e.name == "json") {
        let loads = py::import_attr(vm, "json", "loads")?;
        let s = vm.types.new_str(&String::from_utf8_lossy(&j.data));
        if let Ok(v) = vm.call(&loads, vec![s], Vec::new()) {
            return Ok(v);
        }
    }
    let d = py::dict(vm);
    if let Some(e) = entries.iter().find(|e| e.name == "extra_info") {
        py::dict_set(
            vm,
            &d,
            "_save_name",
            vm.types.new_str(&String::from_utf8_lossy(&e.data)),
        )?;
    }
    Ok(d)
}

fn fl_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mtime = fl_mtime(vm, args, &[])?;
    if py::is_none(&mtime) {
        return Ok(Value::None);
    }
    let Some(b) = slot_bytes(vm, &args[1])? else {
        return Ok(Value::None);
    };
    let Ok(entries) = savestore::zip_read(&b) else {
        return Ok(Value::None);
    };
    let member = if entries.iter().any(|e| e.name == "screenshot.tga") {
        "screenshot.tga"
    } else if entries.iter().any(|e| e.name == "screenshot.png") {
        "screenshot.png"
    } else {
        return Ok(Value::None);
    };
    let f = slot_file(vm, &args[1])?;
    let zfi = py::import_attr(vm, "renpy.display.im", "ZipFileImage")?;
    let (fv, mv) = (vm.types.new_str(&f), vm.types.new_str(member));
    vm.call(&zfi, vec![fv, mv, mtime], Vec::new())
}

fn fl_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = slot_file(vm, &args[1])?;
    let Some(b) = store(vm).read(&f) else {
        return Err(vm.exc_msg("IOError", format!("Couldn't load {f}.")));
    };
    let entries = savestore::zip_read(&b).map_err(|e| vm.exc_msg("BadZipfile", e))?;
    let log = entries
        .iter()
        .find(|e| e.name == "log")
        .ok_or_else(|| vm.exc_msg("KeyError", "There is no item named 'log' in the archive"))?;
    let token = entries
        .iter()
        .find(|e| e.name == "signatures")
        .map(|e| String::from_utf8_lossy(&e.data).into_owned())
        .unwrap_or_default();
    let l = vm.types.new_bytes(log.data.clone());
    if at_least(vm, 8, 1) {
        let t = vm.types.new_str(&token);
        return Ok(vm.types.new_tuple(vec![l, t]));
    }
    Ok(l)
}

fn fl_unlink(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = slot_file(vm, &args[1])?;
    store(vm).remove(&f).map_err(|e| vm.exc_msg("OSError", e))?;
    fl_scan(vm, &args[..1], &[])
}

fn fl_rename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (old, new) = (slot_file(vm, &args[1])?, slot_file(vm, &args[2])?);
    let st = store(vm);
    let Some(b) = st.read(&old) else {
        return Ok(Value::None);
    };
    st.write(&new, &b).map_err(|e| vm.exc_msg("OSError", e))?;
    st.remove(&old).map_err(|e| vm.exc_msg("OSError", e))?;
    fl_scan(vm, &args[..1], &[])
}

fn fl_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (old, new) = (slot_file(vm, &args[1])?, slot_file(vm, &args[2])?);
    let st = store(vm);
    let Some(b) = st.read(&old) else {
        return Ok(Value::None);
    };
    st.write(&new, &b).map_err(|e| vm.exc_msg("OSError", e))?;
    fl_scan(vm, &args[..1], &[])
}

fn fl_load_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "persistent_data")?;
    if py::truthy(vm, &d)? {
        let t = py::get(vm, &args[0], "persistent_mtime")?;
        let pair = vm.types.new_tuple(vec![t, d]);
        return Ok(vm.types.new_list(vec![pair]));
    }
    Ok(vm.types.new_list(Vec::new()))
}

fn fl_save_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !active(vm, &args[0])? {
        return Ok(Value::None);
    }
    let data = bytes_of(vm, &args[1])?;
    let st = store(vm);
    st.write("persistent", &data)
        .map_err(|e| vm.exc_msg("OSError", e))?;
    let _ = st.remove("persistent.new");
    let t = st.mtime("persistent").unwrap_or(0.0);
    py::set(vm, &args[0], "persistent_mtime", Value::Float(t))?;
    Ok(Value::None)
}

fn fl_unlink_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if active(vm, &args[0])? {
        let _ = store(vm).remove("persistent");
    }
    Ok(Value::None)
}

fn fl_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, SAVELOCATION, "FileLocation")?;
    if !c.obj().is_some_and(|c| ops::isinstance(vm, &args[1], c)) {
        return Ok(Value::Bool(false));
    }
    let (a, b) = (
        py::get(vm, &args[0], "directory")?,
        py::get(vm, &args[1], "directory")?,
    );
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

// MultiLocation ---------------------------------------------------------------

fn ml_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "locations", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn ml_active(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let l = py::get(vm, this, "locations")?;
    let mut out = Vec::new();
    for i in py::items(vm, &l)? {
        if active(vm, &i)? {
            out.push(i);
        }
    }
    Ok(out)
}

fn ml_active_locations(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = ml_active(vm, &args[0])?;
    Ok(vm.types.new_list(a))
}

fn save_enabled(vm: &mut Vm) -> bool {
    py::config_bool(vm, "save").unwrap_or(true)
}

fn ml_newest(vm: &mut Vm, this: &Value, slot: &Value) -> PyResult<Value> {
    if !save_enabled(vm) {
        return Ok(Value::None);
    }
    let mut best = (-1.0, Value::None);
    for l in ml_active(vm, this)? {
        let t = vm.call_method(&l, "mtime", vec![slot.clone()])?;
        if !py::is_none(&t) {
            let t = py::f64_of(vm, &t)?;
            if t > best.0 {
                best = (t, l);
            }
        }
    }
    Ok(best.1)
}

fn ml_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::get(vm, &args[0], "locations")?;
    if !ops::contains(vm, &l, &args[1])? {
        vm.call_method(&l, "append", vec![args[1].clone()])?;
    }
    Ok(Value::None)
}

fn ml_each(vm: &mut Vm, args: &[Value], method: &str, gated: bool) -> PyResult<Value> {
    if gated && !save_enabled(vm) {
        return Ok(Value::None);
    }
    for l in ml_active(vm, &args[0])? {
        vm.call_method(&l, method, args[1..].to_vec())?;
    }
    Ok(Value::None)
}

fn ml_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !save_enabled(vm) {
        return Ok(Value::None);
    }
    let a = ml_active(vm, &args[0])?;
    if a.is_empty() {
        return Err(py::error(vm, "Not saved - no valid save locations."));
    }
    for l in a {
        vm.call_method(&l, "save", args[1..].to_vec())?;
    }
    Ok(Value::None)
}

fn ml_list(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !save_enabled(vm) {
        return Ok(vm.types.new_list(Vec::new()));
    }
    let s = vm.types.new_set(Default::default());
    for l in ml_active(vm, &args[0])? {
        let r = vm.call_method(&l, "list", Vec::new())?;
        vm.call_method(&s, "update", vec![r])?;
    }
    let items = py::items(vm, &s)?;
    Ok(vm.types.new_list(items))
}

fn ml_list_files(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    if save_enabled(vm) {
        for l in ml_active(vm, &args[0])? {
            let r = vm.call_method(&l, "list_files", Vec::new())?;
            out.extend(py::items(vm, &r)?);
        }
    }
    Ok(vm.types.new_list(out))
}

fn ml_path(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut best: Option<(f64, Value)> = None;
    for l in ml_active(vm, &args[0])? {
        let r = vm.call_method(&l, "path", vec![args[1].clone()])?;
        let it = py::items(vm, &r)?;
        let t = py::f64_of(vm, &it[0])?;
        if best.as_ref().is_none_or(|b| t >= b.0) {
            best = Some((t, r));
        }
    }
    Ok(best
        .map(|b| b.1)
        .unwrap_or_else(|| vm.types.new_tuple(vec![Value::Int(0), Value::None])))
}

fn ml_slot_call(vm: &mut Vm, args: &[Value], method: &str) -> PyResult<Value> {
    let l = ml_newest(vm, &args[0], &args[1])?;
    if py::is_none(&l) {
        if method == "load" {
            return Err(vm.attr_error("'NoneType' object has no attribute 'load'"));
        }
        return Ok(Value::None);
    }
    vm.call_method(&l, method, vec![args[1].clone()])
}

fn ml_mtime(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_slot_call(vm, args, "mtime")
}

fn ml_json(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_slot_call(vm, args, "json")
}

fn ml_screenshot(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_slot_call(vm, args, "screenshot")
}

fn ml_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_slot_call(vm, args, "load")
}

fn ml_newest_m(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_newest(vm, &args[0], &args[1])
}

fn ml_unlink(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_each(vm, args, "unlink", true)
}

fn ml_rename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_each(vm, args, "rename", true)
}

fn ml_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_each(vm, args, "copy", true)
}

fn ml_load_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    for l in ml_active(vm, &args[0])? {
        let r = vm.call_method(&l, "load_persistent", Vec::new())?;
        out.extend(py::items(vm, &r)?);
    }
    Ok(vm.types.new_list(out))
}

fn ml_save_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_each(vm, args, "save_persistent", false)
}

fn ml_unlink_persistent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ml_each(vm, args, "unlink_persistent", false)
}

fn ml_scan(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::get(vm, &args[0], "locations")?;
    for i in py::items(vm, &l)? {
        vm.call_method(&i, "scan", Vec::new())?;
    }
    Ok(Value::None)
}

fn ml_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, SAVELOCATION, "MultiLocation")?;
    if !c.obj().is_some_and(|c| ops::isinstance(vm, &args[1], c)) {
        return Ok(Value::Bool(false));
    }
    let (a, b) = (
        py::get(vm, &args[0], "locations")?,
        py::get(vm, &args[1], "locations")?,
    );
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

/// `savelocation.init()`: the save mount, then the inactive game and
/// extra directories.
fn sl_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ml = py::import_attr(vm, SAVELOCATION, "MultiLocation")?;
    let fl = py::import_attr(vm, SAVELOCATION, "FileLocation")?;
    let location = vm.call(&ml, Vec::new(), Vec::new())?;
    let savedir = py::config(vm, "savedir").unwrap_or(Value::None);
    let main = vm.call(&fl, vec![savedir], Vec::new())?;
    vm.call_method(&location, "add", vec![main])?;
    let gd = cfg(vm, "gamedir");
    if !py::is_none(&gd) {
        let gd = py::string(vm, &gd)?;
        let p = vm.types.new_str(&format!("{gd}/saves"));
        let l = vm.call(&fl, vec![p], Vec::new())?;
        vm.call_method(&location, "add", vec![l])?;
    }
    let extra = cfg(vm, "extra_savedirs");
    if !py::is_none(&extra) {
        for i in py::items(vm, &extra)? {
            let l = vm.call(&fl, vec![i], Vec::new())?;
            vm.call_method(&location, "add", vec![l])?;
        }
    }
    vm.call_method(&location, "scan", Vec::new())?;
    let ls = py::import(vm, "renpy.loadsave")?;
    py::set(vm, &ls, "location", location)?;
    Ok(Value::None)
}

fn ret_true(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn sfl_enter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn sfl_exit(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn savelocation_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        SAVELOCATION,
        &[
            ("init", sl_init),
            ("quit", noop),
            ("pause_syncfs", noop),
            ("resume_syncfs", noop),
            ("syncfs", noop),
            ("zip_saves", ret_true),
            ("unzip_saves", ret_true),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let fl = py::class(
        vm,
        SAVELOCATION,
        "FileLocation",
        &[],
        &[
            ("__init__", fl_init),
            ("filename", fl_filename),
            ("sync", noop),
            ("scan", fl_scan),
            ("save", fl_save),
            ("list", fl_list),
            ("list_files", fl_list_files),
            ("mtime", fl_mtime),
            ("path", fl_path),
            ("json", fl_json),
            ("screenshot", fl_screenshot),
            ("load", fl_load),
            ("unlink", fl_unlink),
            ("rename", fl_rename),
            ("copy", fl_copy),
            ("load_persistent", fl_load_persistent),
            ("save_persistent", fl_save_persistent),
            ("unlink_persistent", fl_unlink_persistent),
            ("__eq__", fl_eq),
        ],
        &[],
    )?;
    set(vm, "FileLocation", Value::Obj(fl));
    let ml = py::class(
        vm,
        SAVELOCATION,
        "MultiLocation",
        &[],
        &[
            ("__init__", ml_init),
            ("active_locations", ml_active_locations),
            ("newest", ml_newest_m),
            ("add", ml_add),
            ("save", ml_save),
            ("list", ml_list),
            ("list_files", ml_list_files),
            ("path", ml_path),
            ("mtime", ml_mtime),
            ("json", ml_json),
            ("screenshot", ml_screenshot),
            ("load", ml_load),
            ("unlink", ml_unlink),
            ("rename", ml_rename),
            ("copy", ml_copy),
            ("load_persistent", ml_load_persistent),
            ("save_persistent", ml_save_persistent),
            ("unlink_persistent", ml_unlink_persistent),
            ("scan", ml_scan),
            ("__eq__", ml_eq),
        ],
        &[],
    )?;
    set(vm, "MultiLocation", Value::Obj(ml));
    let sfl = py::class(
        vm,
        SAVELOCATION,
        "SyncfsLock",
        &[],
        &[("__enter__", sfl_enter), ("__exit__", sfl_exit)],
        &[],
    )?;
    set(vm, "SyncfsLock", Value::Obj(sfl));
    set(vm, "scan_thread", Value::None);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(PREFERENCES, preferences_module);
    vm.register_native_module(PERSISTENT, persistent_module);
    vm.register_native_module(SAVELOCATION, savelocation_module);
}
