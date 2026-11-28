// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports` loader, media, input, persistent and fetch functions
//! (Ren'Py 8.3 `loaderexports`, `mediaexports`, `inputexports`,
//! `persistentexports`, `fetchexports`; MIT).
use super::{
    super::py::{self, *},
    util::*,
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, object::NativeFn, ops};
use std::rc::Rc;

fn loader(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.loader", name)?;
    vm.call(&f, args, kw)
}

// loaderexports ----------------------------------------------------------------------------------------

fn loadable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let directory = arg_or(args, kw, 1, "directory", Value::None);
    let tl = arg_or(args, kw, 2, "tl", Value::Bool(true));
    loader(
        vm,
        "loadable",
        vec![args[0].clone()],
        vec![("tl".into(), tl), ("directory".into(), directory)],
    )
}

fn exists(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(
        loader(vm, "transfn", vec![args[0].clone()], Vec::new()).is_ok(),
    ))
}

fn open_file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut encoding = arg_or(args, kw, 1, "encoding", Value::None);
    let directory = arg_or(args, kw, 2, "directory", Value::None);
    let rv = loader(
        vm,
        "load",
        vec![args[0].clone()],
        vec![("directory".into(), directory)],
    )?;
    if py::is_none(&encoding) {
        encoding = py::config(vm, "open_file_encoding")?;
    }
    let br = py::import_attr(vm, "io", "BufferedReader")?;
    let rv = vm.call(&br, vec![rv], Vec::new())?;
    if py::truthy(vm, &encoding)? {
        let tw = py::import_attr(vm, "io", "TextIOWrapper")?;
        return vm.call(
            &tw,
            vec![rv],
            vec![
                ("encoding".into(), encoding),
                ("errors".into(), py::s(vm, "surrogateescape")),
            ],
        );
    }
    py2_view(vm, rv)
}

/// Ren'Py 6/7 returns Python 2 byte strings; the Python 2 dialect reads them as text.
fn py2_view(vm: &mut Vm, f: Value) -> PyResult<Value> {
    if !matches!(
        super::super::state::dialect(vm),
        crysirror_python::Dialect::Py2
    ) {
        return Ok(f);
    }
    let view = py::import_attr(vm, "renpy.compat", "Py2File")?;
    vm.call(&view, vec![f], Vec::new())
}

fn file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let enc = arg_or(args, kw, 1, "encoding", Value::None);
    open_file(vm, &[args[0].clone(), enc], &[])
}

fn notl_file(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = loader(
        vm,
        "load",
        vec![args[0].clone()],
        vec![("tl".into(), Value::Bool(false))],
    )?;
    py2_view(vm, f)
}

fn list_files(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let common = kwb(vm, arg(args, kw, 0, "common"), false)?;
    let r = super::super::state::rt(vm);
    let mut names: Vec<String> = r
        .files
        .list()
        .into_iter()
        .filter(|n| !n.starts_with("saves/"))
        .collect();
    if common {
        names.extend(r.files.common_list());
    }
    names.sort();
    names.dedup();
    let v: Vec<Value> = names.iter().map(|n| vm.types.new_str(n)).collect();
    Ok(vm.types.new_list(v))
}

fn fsencode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let force = kwb(vm, arg(args, kw, 1, "force"), false)?;
    let py2 = matches!(
        super::super::state::dialect(vm),
        crysirror_python::Dialect::Py2
    );
    if (!py2 && !force) || args[0].as_str().is_none() {
        return Ok(args[0].clone());
    }
    Ok(vm
        .types
        .new_bytes(args[0].as_str().unwrap().as_bytes().to_vec()))
}

fn fsdecode(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args[0].as_str().is_some() {
        return Ok(args[0].clone());
    }
    let b = ops::bytes_of(&args[0]).unwrap_or_default();
    Ok(py::s(vm, &String::from_utf8_lossy(&b)))
}

fn munge(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    if !name.starts_with("__") || name.ends_with("__") {
        return Ok(args[0].clone());
    }
    let filename = arg_or(args, kw, 1, "filename", Value::None);
    let filename = if py::is_none(&filename) {
        super::current_filename(vm)?
    } else {
        py::string(vm, &filename)?
    };
    Ok(py::s(
        vm,
        &format!("{}{}", crate::lexer::munge_prefix(&filename), &name[2..]),
    ))
}

// mediaexports -----------------------------------------------------------------------------------------

fn audio(
    vm: &mut Vm,
    module: &str,
    name: &str,
    args: Vec<Value>,
    kw: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, kw)
}

fn movie_cutscene(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = args[0].clone();
    let delay = arg_or(args, kw, 1, "delay", Value::None);
    let loops = arg_or(args, kw, 2, "loops", Value::Int(0));
    let stop_music = kwb(vm, arg(args, kw, 3, "stop_music"), true)?;
    ex(vm, "mode", vec![py::s(vm, "movie")], Vec::new())?;
    if stop_music {
        audio(
            vm,
            "renpy.audio.audio",
            "set_force_stop",
            vec![py::s(vm, "music"), Value::Bool(true)],
            Vec::new(),
        )?;
    }
    ex(
        vm,
        "movie_start_fullscreen",
        vec![filename],
        vec![("loops".into(), loops)],
    )?;
    ui(vm, "saybehavior", Vec::new(), Vec::new())?;
    if py::is_none(&delay) || py::f64_of(vm, &delay)? < 0. {
        ui(
            vm,
            "soundstopbehavior",
            vec![py::s(vm, "movie")],
            Vec::new(),
        )?;
    } else {
        ui(
            vm,
            "pausebehavior",
            vec![delay, Value::Bool(false)],
            Vec::new(),
        )?;
    }
    let l = log(vm)?;
    let fwd = py::get(vm, &l, "forward")?;
    let rf = if py::truthy(vm, &fwd)? {
        Value::Bool(true)
    } else {
        Value::None
    };
    let rv = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("suppress_overlay".into(), Value::Bool(true)),
            ("roll_forward".into(), rf),
        ],
    )?;
    ex(vm, "movie_stop", Vec::new(), Vec::new())?;
    if stop_music {
        audio(
            vm,
            "renpy.audio.audio",
            "set_force_stop",
            vec![py::s(vm, "music"), Value::Bool(false)],
            Vec::new(),
        )?;
    }
    Ok(rv)
}

fn toggle_music(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn music_start(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let loops = arg_or(args, kw, 1, "loops", Value::Bool(true));
    let fadeout = arg_or(args, kw, 2, "fadeout", Value::None);
    let fadein = arg_or(args, kw, 3, "fadein", Value::Int(0));
    audio(
        vm,
        "renpy.audio.music",
        "play",
        vec![args[0].clone()],
        vec![
            ("loop".into(), loops),
            ("fadeout".into(), fadeout),
            ("fadein".into(), fadein),
        ],
    )
}

fn music_stop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let fadeout = arg_or(args, kw, 0, "fadeout", Value::None);
    audio(
        vm,
        "renpy.audio.music",
        "stop",
        Vec::new(),
        vec![("fadeout".into(), fadeout)],
    )
}

fn play(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = args[0].clone();
    if py::is_none(&filename) {
        return Ok(Value::None);
    }
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut channel = args
        .get(1)
        .cloned()
        .or_else(|| {
            kw.iter()
                .position(|(k, _)| &**k == "channel")
                .map(|i| kw.remove(i).1)
        })
        .unwrap_or(Value::None);
    if py::is_none(&channel) {
        channel = py::config(vm, "play_channel")?;
    }
    kw.push(("channel".into(), channel));
    kw.push(("loop".into(), Value::Bool(false)));
    audio(vm, "renpy.audio.music", "play", vec![filename], kw)
}

// inputexports -----------------------------------------------------------------------------------------

fn input(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut kw: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut pos = args.iter().cloned();
    let mut p = |n: &str, d: Value, kw: &mut Vec<(Rc<str>, Value)>| {
        pos.next()
            .or_else(|| {
                kw.iter()
                    .position(|(k, _)| &**k == n)
                    .map(|i| kw.remove(i).1)
            })
            .unwrap_or(d)
    };
    let prompt = p("prompt", Value::None, &mut kw);
    let mut default = p("default", py::s(vm, ""), &mut kw);
    let allow = p("allow", Value::None, &mut kw);
    let exclude = p("exclude", py::s(vm, "{}"), &mut kw);
    let length = p("length", Value::None, &mut kw);
    let mut with_none = p("with_none", Value::None, &mut kw);
    let pixel_width = p("pixel_width", Value::None, &mut kw);
    let screen = p("screen", py::s(vm, "input"), &mut kw);
    let mask = p("mask", Value::None, &mut kw);
    let copypaste = p("copypaste", Value::Bool(true), &mut kw);
    let multiline = p("multiline", Value::Bool(false), &mut kw);
    if py::config_bool(vm, "disable_input")? {
        return Ok(default);
    }
    let fixed = ex(vm, "in_fixed_rollback", Vec::new(), Vec::new())?;
    let fixed = py::truthy(vm, &fixed)?;
    ex(vm, "mode", vec![py::s(vm, "input")], Vec::new())?;
    let mut rf = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    if rf.as_str().is_none() {
        rf = Value::None;
    }
    if !py::is_none(&rf) {
        default = rf.clone();
    }
    let sp = py::import_attr(vm, "renpy.easy", "split_properties")?;
    let kd = py::kwargs_dict(vm, &kw)?;
    let r = vm.call(&sp, vec![kd, py::s(vm, "show_"), py::s(vm, "")], Vec::new())?;
    let parts = py::items(vm, &r)?;
    let rest = py::str_items(vm, &parts[1])?;
    if !rest.is_empty() {
        let names: Vec<String> = rest.iter().map(|(k, _)| k.to_string()).collect();
        return Err(vm.type_error(format!(
            "renpy.input() got unexpected keyword argument(s): {}",
            names.join(", ")
        )));
    }
    let show_properties = py::str_items(vm, &parts[0])?;
    let hs = ex(vm, "has_screen", vec![screen.clone()], Vec::new())?;
    if py::truthy(vm, &hs)? {
        let wp = vm.types.new_dict();
        let ip = vm.types.new_dict();
        for (k, v) in [
            ("default", default),
            ("length", length),
            ("allow", allow),
            ("exclude", exclude),
            ("editable", Value::Bool(!fixed)),
            ("pixel_width", pixel_width),
            ("mask", mask),
            ("copypaste", copypaste),
            ("multiline", multiline),
        ] {
            py::dict_set(vm, &ip, k, v)?;
        }
        py::dict_set(vm, &wp, "input", ip)?;
        let mut k = show_properties;
        k.push(("_transient".into(), Value::Bool(true)));
        k.push(("_widget_properties".into(), wp));
        k.push(("prompt".into(), prompt));
        ex(vm, "show_screen", vec![screen], k)?;
    } else {
        if screen.as_str() != Some("input") {
            let s = py::string(vm, &screen)?;
            return Err(py::error(vm, format!("The '{s}' screen does not exist.")));
        }
        ui(
            vm,
            "window",
            Vec::new(),
            vec![("style".into(), py::s(vm, "input_window"))],
        )?;
        ui(vm, "vbox", Vec::new(), Vec::new())?;
        ui(
            vm,
            "text",
            vec![prompt],
            vec![("style".into(), py::s(vm, "input_prompt"))],
        )?;
        let w = ui(
            vm,
            "input",
            vec![default],
            vec![
                ("length".into(), length),
                ("style".into(), py::s(vm, "input_text")),
                ("allow".into(), allow),
                ("exclude".into(), exclude),
            ],
        )?;
        if fixed {
            vm.call_method(&w, "disable", Vec::new())?;
        }
        ui(vm, "close", Vec::new(), Vec::new())?;
    }
    ex(vm, "shown_window", Vec::new(), Vec::new())?;
    let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
    if py::config_bool(vm, "autosave_on_input")? && !py::truthy(vm, &ar)? {
        let fa = py::import_attr(vm, "renpy.loadsave", "force_autosave")?;
        vm.call(&fa, vec![Value::Bool(true)], Vec::new())?;
    }
    if fixed {
        ui(vm, "saybehavior", Vec::new(), Vec::new())?;
    }
    let rv = ui(
        vm,
        "interact",
        Vec::new(),
        vec![
            ("mouse".into(), py::s(vm, "prompt")),
            ("type".into(), py::s(vm, "input")),
            ("roll_forward".into(), rf),
        ],
    )?;
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

fn get_editable_input_value(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::import_attr(vm, "renpy.display.behavior", "current_input_value")?;
    let b = py::import_attr(vm, "renpy.display.behavior", "input_value_active")?;
    Ok(vm.types.new_tuple(vec![a, b]))
}

fn set_editable_input_value(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, "renpy.display.behavior")?;
    py::set(vm, &m, "current_input_value", args[0].clone())?;
    py::set(vm, &m, "input_value_active", args[1].clone())?;
    Ok(Value::None)
}

// persistentexports ------------------------------------------------------------------------------------

fn persistent_field(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    py::get(vm, &p, n)
}

fn strip_audio(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let s = py::string(vm, v)?;
    let s = if s.starts_with('<') {
        s.find('>').map(|i| s[i + 1..].to_owned()).unwrap_or(s)
    } else {
        s
    };
    Ok(py::s(vm, &s))
}

fn image_key(vm: &mut Vm, v: &Value, stringify: bool) -> PyResult<Value> {
    let t = if matches!(v.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        v.clone()
    } else {
        let s = py::string(vm, v)?;
        let parts = s.split_whitespace().map(|p| vm.types.new_str(p)).collect();
        vm.types.new_tuple(parts)
    };
    if !stringify {
        return Ok(t);
    }
    let mut parts = Vec::new();
    for i in py::items(vm, &t)? {
        let s = ops::str(vm, &i)?;
        parts.push(py::s(vm, &s));
    }
    Ok(vm.types.new_tuple(parts))
}

fn seen_in(vm: &mut Vm, field: &str, key: Value) -> PyResult<Value> {
    let d = persistent_field(vm, field)?;
    Ok(Value::Bool(ops::contains(vm, &d, &key)?))
}

fn mark(vm: &mut Vm, field: &str, key: Value, seen: bool) -> PyResult<Value> {
    let d = persistent_field(vm, field)?;
    if seen {
        ops::setitem(vm, &d, key, Value::Bool(true))?;
    } else if ops::contains(vm, &d, &key)? {
        ops::delitem(vm, &d, key)?;
    }
    Ok(Value::None)
}

fn seen_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    seen_in(vm, "_seen_ever", args[0].clone())
}

fn mark_label_seen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = ops::str(vm, &args[0])?;
    let k = py::s(vm, &s);
    mark(vm, "_seen_ever", k, true)
}

fn mark_label_unseen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mark(vm, "_seen_ever", args[0].clone(), false)
}

fn seen_audio(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = strip_audio(vm, &args[0])?;
    seen_in(vm, "_seen_audio", k)
}

fn mark_audio_seen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = strip_audio(vm, &args[0])?;
    mark(vm, "_seen_audio", k, true)
}

fn mark_audio_unseen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = strip_audio(vm, &args[0])?;
    mark(vm, "_seen_audio", k, false)
}

fn seen_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = image_key(vm, &args[0], false)?;
    seen_in(vm, "_seen_images", k)
}

fn mark_image_seen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = image_key(vm, &args[0], true)?;
    mark(vm, "_seen_images", k, true)
}

fn mark_image_unseen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = image_key(vm, &args[0], false)?;
    mark(vm, "_seen_images", k, false)
}

fn save_persistent(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.persistent", "update")?;
    vm.call(&f, vec![Value::Bool(true)], Vec::new())
}

fn is_seen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ever = arg_or(args, kw, 0, "ever", Value::Bool(true));
    let c = ctx(vm)?;
    vm.call_method(&c, "seen_current", vec![ever])
}

// fetchexports -----------------------------------------------------------------------------------------

/// Games have no network access in CrySirror: every fetch fails with
/// `FetchError`, as an unreachable server does upstream.
fn fetch(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let url = args
        .first()
        .map(|u| py::string(vm, u))
        .transpose()?
        .unwrap_or_default();
    let c = py::import_attr(vm, "renpy.exports", "FetchError")?;
    let e = vm.call(
        &c,
        vec![py::s(
            vm,
            &format!("Network access is not available ({url})."),
        )],
        Vec::new(),
    )?;
    Err(PyErr::Exc(e.obj().cloned().unwrap()))
}

fn fetch_pause(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("loadable", loadable),
        ("exists", exists),
        ("open_file", open_file),
        ("file", file),
        ("notl_file", notl_file),
        ("list_files", list_files),
        ("fsencode", fsencode),
        ("fsdecode", fsdecode),
        ("munge", munge),
        ("movie_cutscene", movie_cutscene),
        ("toggle_music", toggle_music),
        ("music_start", music_start),
        ("music_stop", music_stop),
        ("play", play),
        ("input", input),
        ("web_input", input),
        ("get_editable_input_value", get_editable_input_value),
        ("set_editable_input_value", set_editable_input_value),
        ("seen_label", seen_label),
        ("mark_label_seen", mark_label_seen),
        ("mark_label_unseen", mark_label_unseen),
        ("seen_audio", seen_audio),
        ("mark_audio_seen", mark_audio_seen),
        ("mark_audio_unseen", mark_audio_unseen),
        ("seen_image", seen_image),
        ("mark_image_seen", mark_image_seen),
        ("mark_image_unseen", mark_image_unseen),
        ("save_persistent", save_persistent),
        ("is_seen", is_seen),
        ("fetch", fetch),
        ("fetch_emscripten", fetch),
        ("fetch_requests", fetch),
        ("fetch_pause", fetch_pause),
    ]
}

pub fn values(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let exc = Value::Obj(vm.types.exc("Exception"));
    let fe = py::class(vm, "renpy.exports", "FetchError", &[exc], &[], &[])?;
    crysirror_python::modules::set(vm, m, "FetchError", Value::Obj(fe));
    crysirror_python::modules::set(vm, m, "proxies", Value::None);
    Ok(())
}
