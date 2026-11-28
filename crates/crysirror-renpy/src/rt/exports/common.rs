// SPDX-License-Identifier: MPL-2.0
//! `renpy.*` functions added in 8.4 – 8.6 (MIT): 8.4's translation-seen
//! tracking (`persistentexports`), 8.5's `is_in_test`,
//! `filename_line_override` and `get_mouse_names`, and 8.6's
//! `commonexports.callback`, `clipboardexports`, `is_warping`,
//! `get_user_age`, `get_rollback_identifier`, `can_warp_say`,
//! `run_initcode`, `unlocked_config`, shader prediction and
//! `get_statement_info`. The clipboard functions go through
//! `renpy.pygame.scrap`, as upstream; the host clipboard carries text, so
//! other MIME types raise there.
use super::super::py;
use crysirror_python::{PyResult, Value, Vm, builtins::Kw, ops};

type Native = crysirror_python::object::NativeFn;

fn partial(vm: &mut Vm, name: Value, late: Value) -> PyResult<Value> {
    let p = py::import_attr(vm, "functools", "partial")?;
    let cb = py::import_attr(vm, "renpy.exports", "callback")?;
    vm.call(&p, vec![cb], vec![("name".into(), name), ("late".into(), late)])
}

/// `callback(f=None, name=None, *, late=False)`.
fn callback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::arg_or(args, kw, 0, "f", Value::None);
    let name = py::arg_or(args, kw, 1, "name", Value::None);
    let late = py::kw(kw, "late").cloned().unwrap_or(Value::Bool(false));
    if py::is_none(&f) {
        return partial(vm, name, late);
    }
    if f.as_str().is_some() {
        if !py::is_none(&name) {
            return Err(py::error(vm, "renpy.callback: f is a string, but name is not None."));
        }
        return partial(vm, f, late);
    }
    let name = if py::is_none(&name) {
        let n = py::get(vm, &f, "__name__")?;
        let n = py::string(vm, &n)?;
        let n = n.strip_suffix("_callback").unwrap_or(&n);
        n.strip_suffix("_callbacks").unwrap_or(n).to_owned()
    } else {
        py::string(vm, &name)?
    };
    let config = py::import(vm, "renpy.config")?;
    for attr in [format!("{name}_callbacks"), format!("{name}_callback")] {
        let Some(target) = py::get_opt(vm, &config, &attr)? else {
            continue;
        };
        if ops::truthy(vm, &late)? {
            // Registers when config.after_init_callbacks runs.
            let cb = py::import_attr(vm, "renpy.exports", "callback")?;
            let p = py::import_attr(vm, "functools", "partial")?;
            let n = vm.types.new_string(name);
            let deferred =
                vm.call(&p, vec![cb, f.clone()], vec![("name".into(), n), ("late".into(), Value::Bool(false))])?;
            let after = py::get(vm, &config, "after_init_callbacks")?;
            vm.call_method(&after, "append", vec![deferred])?;
            return Ok(f);
        }
        if matches!(target.payload(), Some(crysirror_python::object::Payload::List(_))) {
            if !ops::contains(vm, &target, &f)? {
                vm.call_method(&target, "append", vec![f.clone()])?;
            }
        } else if py::is_none(&target) {
            py::set(vm, &config, &attr, f.clone())?;
        } else {
            return Err(py::error(
                vm,
                format!("renpy.callback: config.{attr} is not a list or None - has it already been set to a callback?"),
            ));
        }
        return Ok(f);
    }
    Err(py::error(
        vm,
        format!("renpy.callback: Neither config.{name}_callbacks nor config.{name}_callback exists."),
    ))
}

fn scrap(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.pygame.scrap", name)?;
    vm.call(&f, args, Vec::new())
}

fn scrap_text(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.pygame.scrap", "SCRAP_TEXT")
}

fn get_clipboard_text(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let t = scrap_text(vm)?;
    let b = scrap(vm, "get", vec![t])?;
    let utf8 = vm.types.new_str("utf-8");
    vm.call_method(&b, "decode", vec![utf8])
}

fn put_clipboard_text(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let text = py::arg_or(args, kw, 0, "text", Value::None);
    let utf8 = vm.types.new_str("utf-8");
    let b = vm.call_method(&text, "encode", vec![utf8])?;
    let t = scrap_text(vm)?;
    scrap(vm, "put", vec![t, b])
}

fn get_clipboard_data(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let m = py::arg_or(args, kw, 0, "mime_type", Value::None);
    scrap(vm, "get_data", vec![m])
}

fn get_clipboard_mime_types(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    scrap(vm, "get_mime_types", Vec::new())
}

fn put_clipboard_data(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = py::arg_or(args, kw, 0, "data_dict", Value::None);
    scrap(vm, "put_data", vec![d])
}

fn read_file(vm: &mut Vm, filename: &Value, absolute: &Value, text: bool) -> PyResult<Value> {
    let f = if ops::truthy(vm, absolute)? {
        let open = py::import_attr(vm, "builtins", "open")?;
        let mut a = vec![filename.clone(), vm.types.new_str(if text { "r" } else { "rb" })];
        if text {
            a.push(vm.types.new_str("utf-8"));
        }
        vm.call(&open, a, Vec::new())?
    } else if text {
        let of = py::import_attr(vm, "renpy.exports", "open_file")?;
        let utf8 = vm.types.new_str("utf-8");
        vm.call(&of, vec![filename.clone(), utf8], Vec::new())?
    } else {
        let load = py::import_attr(vm, "renpy.loader", "load")?;
        let images = vm.types.new_str("images");
        vm.call(&load, vec![filename.clone(), images], Vec::new())?
    };
    let data = vm.call_method(&f, "read", Vec::new())?;
    vm.call_method(&f, "close", Vec::new())?;
    Ok(data)
}

fn put_clipboard_image_file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = py::arg_or(args, kw, 0, "filename", Value::None);
    let absolute = py::arg_or(args, kw, 1, "absolute_path", Value::Bool(false));
    let data = read_file(vm, &filename, &absolute, false)?;
    let name = py::string(vm, &filename)?;
    let ext = name.rsplit_once('.').map(|(_, e)| format!(".{}", e.to_lowercase())).unwrap_or_default();
    let mime = match ext.as_str() {
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".webp" => "image/webp",
        ".avif" => "image/avif",
        ".svg" => "image/svg+xml",
        ".bmp" => "image/bmp",
        ".tga" => "image/x-tga",
        _ => return Err(vm.value_error(format!("Unsupported image format: {ext}"))),
    };
    let d = vm.types.new_dict();
    let k = vm.types.new_str(mime);
    ops::setitem(vm, &d, k, data.clone())?;
    if mime != "image/png" {
        let bio = py::import_attr(vm, "io", "BytesIO")?;
        let src = vm.call(&bio, vec![data], Vec::new())?;
        let load = py::import_attr(vm, "renpy.pygame.image", "load")?;
        let surf = vm.call(&load, vec![src], vec![("namehint".into(), filename.clone())])?;
        let out = vm.call(&bio, Vec::new(), Vec::new())?;
        let save = py::import_attr(vm, "renpy.display.module", "save_png")?;
        vm.call(&save, vec![surf, out.clone(), Value::Int(0)], Vec::new())?;
        let png = vm.call_method(&out, "getvalue", Vec::new())?;
        let k = vm.types.new_str("image/png");
        ops::setitem(vm, &d, k, png)?;
    }
    scrap(vm, "put_data", vec![d])
}

fn put_clipboard_text_file(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = py::arg_or(args, kw, 0, "filename", Value::None);
    let absolute = py::arg_or(args, kw, 1, "absolute_path", Value::Bool(false));
    let text = read_file(vm, &filename, &absolute, true)?;
    put_clipboard_text(vm, &[text], &[])
}

/// The 8.6 functions of these modules.
pub fn functions_86() -> Vec<(&'static str, Native)> {
    vec![
        ("callback", callback),
        ("get_clipboard_text", get_clipboard_text),
        ("put_clipboard_text", put_clipboard_text),
        ("get_clipboard_data", get_clipboard_data),
        ("get_clipboard_mime_types", get_clipboard_mime_types),
        ("put_clipboard_data", put_clipboard_data),
        ("put_clipboard_image_file", put_clipboard_image_file),
        ("put_clipboard_text_file", put_clipboard_text_file),
    ]
}

// 8.4: translation-seen tracking -------------------------------------------------------------

fn seen_set(vm: &mut Vm) -> PyResult<Value> {
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    py::get(vm, &p, "_seen_translates")
}

fn hashed(vm: &mut Vm, tlid: &Value) -> PyResult<Value> {
    let h = py::import_attr(vm, "renpy.astsupport", "hash64")?;
    vm.call(&h, vec![tlid.clone()], Vec::new())
}

fn is_seen_allowed(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let replay = py::import_attr(vm, "store", "_in_replay")?;
    let nrs = py::config(vm, "no_replay_seen")?;
    Ok(Value::Bool(!(ops::truthy(vm, &replay)? && ops::truthy(vm, &nrs)?)))
}

fn mark_translation_seen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tlid = py::arg_or(args, kw, 0, "tlid", Value::None);
    let h = hashed(vm, &tlid)?;
    let seen = seen_set(vm)?;
    let hs = py::config(vm, "hash_seen")?;
    let v = if ops::truthy(vm, &hs)? { h } else { tlid };
    vm.call_method(&seen, "add", vec![v])?;
    Ok(Value::None)
}

fn mark_translation_unseen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tlid = py::arg_or(args, kw, 0, "tlid", Value::None);
    let h = hashed(vm, &tlid)?;
    let seen = seen_set(vm)?;
    vm.call_method(&seen, "discard", vec![h])?;
    vm.call_method(&seen, "discard", vec![tlid])?;
    Ok(Value::None)
}

fn seen_translation(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let tlid = py::arg_or(args, kw, 0, "tlid", Value::None);
    let h = hashed(vm, &tlid)?;
    let seen = seen_set(vm)?;
    Ok(Value::Bool(ops::contains(vm, &seen, &tlid)? || ops::contains(vm, &seen, &h)?))
}

pub fn functions_84() -> Vec<(&'static str, Native)> {
    vec![
        ("is_seen_allowed", is_seen_allowed),
        ("mark_translation_seen", mark_translation_seen),
        ("mark_translation_unseen", mark_translation_unseen),
        ("seen_translation", seen_translation),
    ]
}

// 8.5 ----------------------------------------------------------------------------------------

fn is_in_test(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.test.testexecution", "is_in_test")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn get_mouse_names(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::import_attr(vm, "renpy.display", "interface")?;
    if !ops::truthy(vm, &i)? {
        let d = vm.types.new_str("default");
        return Ok(vm.types.new_list(vec![d]));
    }
    vm.call_method(&i, "get_mouse_names", Vec::new())
}

/// A context manager over a push / restore pair.
fn ctx_manager(vm: &mut Vm, name: &str, enter: Native, exit: Native, state: Vec<(&str, Value)>) -> PyResult<Value> {
    let c = py::class(vm, "renpy.exports", name, &[], &[("__enter__", enter), ("__exit__", exit)], &[])?;
    let o = vm.call(&Value::Obj(c), Vec::new(), Vec::new())?;
    for (k, v) in state {
        py::set(vm, &o, k, v)?;
    }
    Ok(o)
}

fn flo_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let stack = py::import_attr(vm, "renpy.exports", "filename_line_override_stack")?;
    let f = py::get(vm, &args[0], "filename")?;
    let l = py::get(vm, &args[0], "line")?;
    let t = vm.types.new_tuple(vec![f, l]);
    vm.call_method(&stack, "append", vec![t])?;
    Ok(Value::None)
}

fn flo_exit(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let stack = py::import_attr(vm, "renpy.exports", "filename_line_override_stack")?;
    vm.call_method(&stack, "pop", Vec::new())?;
    Ok(Value::Bool(false))
}

fn filename_line_override(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::arg_or(args, kw, 0, "filename", Value::None);
    let l = py::arg_or(args, kw, 1, "line", Value::None);
    ctx_manager(vm, "filename_line_override", flo_enter, flo_exit, vec![("filename", f), ("line", l)])
}

pub fn functions_85() -> Vec<(&'static str, Native)> {
    vec![
        ("is_in_test", is_in_test),
        ("get_mouse_names", get_mouse_names),
        ("filename_line_override", filename_line_override),
    ]
}

// 8.6 ----------------------------------------------------------------------------------------

fn is_warping(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    py::import_attr(vm, "renpy.warp", "warping")
}

/// `get_user_age()`: the age range comes from Android / iOS age APIs;
/// elsewhere upstream returns `(-1, 150)`.
fn get_user_age(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_tuple(vec![Value::Int(-1), Value::Int(150)]))
}

fn get_rollback_identifier(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let ctx = vm.call(&ctx, Vec::new(), Vec::new())?;
    let rb = py::get(vm, &ctx, "rollback")?;
    let warping = py::import_attr(vm, "renpy.warp", "warping")?;
    if !ops::truthy(vm, &rb)? || ops::truthy(vm, &warping)? {
        return Ok(Value::None);
    }
    let log = py::import_attr(vm, "renpy.game", "log")?;
    let current = py::get(vm, &log, "current")?;
    if py::is_none(&current) {
        return Ok(Value::None);
    }
    py::get(vm, &current, "identifier")
}

fn can_warp_say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut who = py::arg_or(args, kw, 0, "who", Value::None);
    if py::is_none(&who) {
        who = py::import_attr(vm, "store", "narrator")?;
    } else if who.as_str().is_some() {
        who = py::import_attr(vm, "store", "say")?;
    }
    let w = py::get_opt(vm, &who, "warp")?.unwrap_or(Value::Bool(false));
    Ok(Value::Bool(ops::truthy(vm, &w)?))
}

fn run_initcode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let initcode = py::arg_or(args, kw, 0, "initcode", Value::None);
    let kwargs = py::arg_or(args, kw, 1, "kwargs", Value::None);
    let cls = py::import_attr(vm, "renpy.execution", "Context")?;
    let ctx = vm.call(&cls, vec![Value::Bool(false)], Vec::new())?;
    py::set(vm, &ctx, "init_phase", Value::Bool(true))?;
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    vm.call_method(&contexts, "append", vec![ctx.clone()])?;
    let body = (|| -> PyResult<()> {
        if !py::is_none(&kwargs) {
            vm.call_method(&ctx, "make_dynamic", vec![kwargs.clone()])?;
            let store = py::import(vm, "store")?;
            let d = py::get(vm, &store, "__dict__")?;
            vm.call_method(&d, "update", vec![kwargs.clone()])?;
        }
        for pair in py::items(vm, &initcode)? {
            let node = ops::getitem(vm, &pair, Value::Int(1))?;
            vm.call_method(&node, "execute_init", Vec::new())?;
        }
        Ok(())
    })();
    let fin = vm.call_method(&ctx, "pop_all_dynamic", Vec::new());
    vm.call_method(&contexts, "pop", Vec::new())?;
    body?;
    fin?;
    Ok(Value::None)
}

fn uc_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let config = py::import(vm, "renpy.config")?;
    let old = py::get(vm, &config, "locked")?;
    py::set(vm, &args[0], "old_locked", old)?;
    py::set(vm, &config, "locked", Value::Bool(false))?;
    Ok(Value::None)
}

fn uc_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let config = py::import(vm, "renpy.config")?;
    let old = py::get(vm, &args[0], "old_locked")?;
    py::set(vm, &config, "locked", old)?;
    Ok(Value::Bool(false))
}

fn unlocked_config(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    ctx_manager(vm, "unlocked_config", uc_enter, uc_exit, Vec::new())
}

fn shader_tuple(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let shaders = args.first().cloned().unwrap_or(Value::None);
    let items = py::items(vm, &shaders)?;
    let tuple = Value::Obj(vm.types.tuple.clone());
    if items.len() == 1 && items[0].as_str().is_none() {
        return vm.call(&tuple, vec![items[0].clone()], Vec::new());
    }
    vm.call(&tuple, vec![shaders], Vec::new())
}

fn predict_shader(vm: &mut Vm, args: &[Value], add: bool) -> PyResult<Value> {
    let shaders = vm.types.new_tuple(args.to_vec());
    let t = shader_tuple(vm, &[shaders], &[])?;
    let store = py::import(vm, "store")?;
    let cur = py::get(vm, &store, "_predict_shader")?;
    let rs = py::import_attr(vm, "renpy.revertable", "RevertableSet")?;
    let new = vm.call(&rs, vec![cur], Vec::new())?;
    vm.call_method(&new, if add { "add" } else { "discard" }, vec![t])?;
    py::set(vm, &store, "_predict_shader", new)?;
    Ok(Value::None)
}

fn start_predict_shader(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    predict_shader(vm, args, true)
}

fn stop_predict_shader(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    predict_shader(vm, args, false)
}

fn si_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let node = py::arg_or(&args[1..], kw, 0, "node", Value::None);
    let tlid = py::arg_or(&args[1..], kw, 1, "tlid", Value::None);
    for n in ["filename", "linenumber"] {
        let v = py::get(vm, &node, n)?;
        py::set(vm, &args[0], n, v)?;
    }
    py::set(vm, &args[0], "tlid", tlid)?;
    Ok(Value::None)
}

fn get_statement_info(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let mut node = py::import_attr(vm, "renpy.display.predict", "statement")?;
    let tlid = if !py::is_none(&node) {
        let tlids = py::import_attr(vm, "renpy.display.predict", "tlids")?;
        let items = py::items(vm, &tlids)?;
        items.first().cloned().unwrap_or(Value::None)
    } else {
        let ctx = py::import_attr(vm, "renpy.game", "context")?;
        let ctx = vm.call(&ctx, Vec::new(), Vec::new())?;
        let script = py::import_attr(vm, "renpy.game", "script")?;
        let namemap = py::get(vm, &script, "namemap")?;
        let current = py::get(vm, &ctx, "current")?;
        node = vm.call_method(&namemap, "get", vec![current, Value::None])?;
        let t = py::get_or_none(vm, &ctx, "translate_identifier")?;
        if ops::truthy(vm, &t)? { t } else { py::get_or_none(vm, &ctx, "deferred_translate_identifier")? }
    };
    if py::is_none(&node) {
        return Ok(Value::None);
    }
    let cls = py::import_attr(vm, "renpy.exports", "StatementInfo")?;
    vm.call(&cls, vec![node, tlid], Vec::new())
}

pub fn functions_86_more() -> Vec<(&'static str, Native)> {
    vec![
        ("is_warping", is_warping),
        ("get_user_age", get_user_age),
        ("get_rollback_identifier", get_rollback_identifier),
        ("can_warp_say", can_warp_say),
        ("run_initcode", run_initcode),
        ("unlocked_config", unlocked_config),
        ("_shader_tuple", shader_tuple),
        ("start_predict_shader", start_predict_shader),
        ("stop_predict_shader", stop_predict_shader),
        ("get_statement_info", get_statement_info),
    ]
}

/// Values of the 8.5 / 8.6 modules.
pub fn values(vm: &mut Vm, m: &crysirror_python::ObjRef) -> PyResult<()> {
    let p = &super::super::state::rt(vm).profile;
    let (v85, v86) = (p.at_least(8, 5), p.at_least(8, 6));
    if v85 {
        let l = vm.types.new_list(Vec::new());
        crysirror_python::modules::set(vm, m, "filename_line_override_stack", l);
    }
    if v86 {
        let c = py::class(vm, "renpy.exports", "StatementInfo", &[], &[("__init__", si_init)], &[])?;
        crysirror_python::modules::set(vm, m, "StatementInfo", Value::Obj(c));
        crysirror_python::modules::set(vm, m, "_ran_after_init_callbacks", Value::Bool(false));
    }
    Ok(())
}
