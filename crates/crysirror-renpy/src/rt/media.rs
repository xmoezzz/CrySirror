// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.tts` and `renpy.display.video`, after Ren'Py 8.3
//! `tts.py` and `video.py` (MIT).
//!
//! Self-voicing text goes to the host as speech output (`link::speak`)
//! through `config.tts_function`'s default; the "clipboard" and "debug"
//! modes behave as upstream.
//!
//! Movies play on their audio channel as upstream does (`movie=True`
//! channels): the host decodes the file named by the channel's play
//! command, mixes its audio and reports the frame size. A `Movie`'s render
//! is a video texture of that channel (`TexKind::Video`), which the
//! adapter binds to the decoded frames; before the first frame size is
//! reported the `start_image` (or nothing) is shown, as upstream does
//! while the first frame decodes.
use super::{
    py::{self, *},
    render::{self, TexKind, Texture},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const TTS: &str = "renpy.display.tts";
const VIDEO: &str = "renpy.display.video";

fn cfg(vm: &mut Vm, n: &str) -> Value {
    py::config(vm, n).unwrap_or(Value::None)
}

fn prefs(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "preferences")
}

// ---------------------------------------------------------------------------
// TTS.

fn self_voicing(vm: &mut Vm) -> PyResult<Value> {
    let p = prefs(vm)?;
    py::get_or_none(vm, &p, "self_voicing")
}

fn t_default_tts_function(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let s = s.trim();
    if s.is_empty() {
        return Ok(Value::None);
    }
    let sv = self_voicing(vm)?;
    match sv.as_str() {
        Some("clipboard") => {
            super::set_clipboard_text(vm, s.to_owned());
        }
        Some("debug") => {
            super::exports::util::ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
        }
        _ => super::link::speak(vm, s.to_owned()),
    }
    Ok(Value::None)
}

fn queue(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, TTS, "tts_queue")
}

fn t_tick(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let q = queue(vm)?;
    let items = py::items(vm, &q)?;
    if items.is_empty() {
        return Ok(Value::None);
    }
    let mut parts = Vec::new();
    for i in &items {
        parts.push(py::string(vm, i)?);
    }
    vm.call_method(&q, "clear", Vec::new())?;
    let s = vm.types.new_str(&parts.join(" "));
    let m = py::import(vm, TTS)?;
    py::set(vm, &m, "last_spoken", s.clone())?;
    let f = cfg(vm, "tts_function");
    let f = if py::is_none(&f) { py::import_attr(vm, TTS, "default_tts_function")? } else { f };
    let _ = vm.call(&f, vec![s], Vec::new());
    Ok(Value::None)
}

fn t_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sv = self_voicing(vm)?;
    if py::truthy(vm, &sv)? {
        let q = queue(vm)?;
        vm.call_method(&q, "append", vec![args[0].clone()])?;
    }
    Ok(Value::None)
}

fn apply_substitutions(vm: &mut Vm, s: Value) -> PyResult<Value> {
    let subs = py::import_attr(vm, TTS, "tts_substitutions")?;
    let mut s = s;
    for pr in py::items(vm, &subs)? {
        let p = py::items(vm, &pr)?;
        let (pattern, replacement) = (p[0].clone(), py::string(vm, &p[1])?);
        let repl = py::closure(vm, "replace", t_replace, vm.types.new_str(&replacement));
        s = vm.call_method(&pattern, "sub", vec![repl, s])?;
    }
    Ok(s)
}

fn t_replace(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let replacement = py::string(vm, &args[0])?;
    let m = &args[1];
    let old = vm.call_method(m, "group", vec![Value::Int(0)])?;
    let o = py::string(vm, &old)?;
    let mut is = |f: &str| -> bool {
        let r = vm.call_method(&old, f, Vec::new());
        r.ok().is_some_and(|v| matches!(v, Value::Bool(true)))
    };
    let template = if is("istitle") {
        title(&replacement)
    } else if is("isupper") {
        replacement.to_uppercase()
    } else if is("islower") {
        replacement.to_lowercase()
    } else {
        replacement
    };
    let _ = o;
    let t = vm.types.new_str(&template);
    vm.call_method(m, "expand", vec![t])
}

fn title(s: &str) -> String {
    let mut out = String::new();
    let mut start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if start { out.extend(c.to_uppercase()) } else { out.extend(c.to_lowercase()) }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

pub fn speak(vm: &mut Vm, s: &Value, translate: bool, force: bool) -> PyResult<()> {
    let sv = self_voicing(vm)?;
    if !force && !py::truthy(vm, &sv)? {
        return Ok(());
    }
    let mut s = s.clone();
    if translate {
        let f = py::import_attr(vm, "renpy.translation", "translate_string")?;
        s = vm.call(&f, vec![s], Vec::new())?;
    }
    let s = apply_substitutions(vm, s)?;
    let q = queue(vm)?;
    vm.call_method(&q, "append", vec![s])?;
    Ok(())
}

fn t_speak(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(&args[1..], kw, 0, "translate", Value::Bool(true));
    let f = arg_or(&args[1..], kw, 1, "force", Value::Bool(false));
    let (t, f) = (py::truthy(vm, &t)?, py::truthy(vm, &f)?);
    speak(vm, &args[0], t, f)?;
    Ok(Value::None)
}

fn t_speak_extra_alt(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let gf = py::import_attr(vm, "renpy.display.focus", "get_focused")?;
    let d = vm.call(&gf, Vec::new(), Vec::new())?;
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let st = py::get(vm, &d, "style")?;
    let s = py::get(vm, &st, "extra_alt")?;
    if !py::is_none(&s) {
        speak(vm, &s, true, false)?;
    }
    Ok(Value::None)
}

fn t_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let subs = cfg(vm, "tts_substitutions");
    if py::is_none(&subs) {
        return Ok(Value::None);
    }
    let out = py::import_attr(vm, TTS, "tts_substitutions")?;
    let compile = py::import_attr(vm, "re", "compile")?;
    let escape = py::import_attr(vm, "re", "escape")?;
    let ic = py::import_attr(vm, "re", "IGNORECASE")?;
    for pr in py::items(vm, &subs)? {
        let p = py::items(vm, &pr)?;
        let (mut pattern, mut replacement) = (p[0].clone(), p[1].clone());
        if pattern.as_str().is_some() {
            let e = vm.call(&escape, vec![pattern], Vec::new())?;
            let src = format!(r"\b{}\b", py::string(vm, &e)?);
            let s = vm.types.new_str(&src);
            pattern = vm.call(&compile, vec![s, ic.clone()], Vec::new())?;
            let r = py::string(vm, &replacement)?.replace('\\', "\\\\");
            replacement = vm.types.new_str(&r);
        }
        let t = vm.types.new_tuple(vec![pattern, replacement]);
        vm.call_method(&out, "append", vec![t])?;
    }
    Ok(Value::None)
}

fn t_set_root(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, TTS)?;
    py::set(vm, &m, "root", args[0].clone())?;
    Ok(Value::None)
}

fn t_false(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn t_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn tts_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        TTS,
        &[
            ("default_tts_function", t_default_tts_function),
            ("tick", t_tick),
            ("tts", t_tts),
            ("speak", t_speak),
            ("speak_extra_alt", t_speak_extra_alt),
            ("init", t_init),
            ("set_root", t_set_root),
            ("periodic", t_noop),
            ("is_active", t_false),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "tts_queue", vm.types.new_list(Vec::new()));
    set(vm, "tts_substitutions", vm.types.new_list(Vec::new()));
    set(vm, "last_spoken", py::s(vm, ""));
    set(vm, "root", Value::None);
    set(vm, "process", Value::None);
    set(vm, "platform_tts", Value::None);
    Ok(m)
}

// ---------------------------------------------------------------------------
// Video.

fn music(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.audio.music", name)?;
    vm.call(&f, args, kw)
}

fn vget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, VIDEO, n)
}

fn vset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = py::import(vm, VIDEO)?;
    py::set(vm, &m, n, v)
}

fn playing_on(vm: &mut Vm, channel: &Value) -> PyResult<bool> {
    let r = music(vm, "get_playing", vec![], vec![("channel".into(), channel.clone())])?;
    py::truthy(vm, &r)
}

fn v_movie_stop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let only = arg_or(args, kw, 1, "only_fullscreen", Value::Bool(false));
    let fs = vget(vm, "fullscreen")?;
    if !py::truthy(vm, &fs)? && py::truthy(vm, &only)? {
        return Ok(Value::None);
    }
    music(vm, "stop", vec![], vec![("channel".into(), py::s(vm, "movie"))])?;
    Ok(Value::None)
}

fn v_movie_start(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let lu = py::import_attr(vm, "renpy.game", "less_updates").unwrap_or(Value::Bool(false));
    if py::truthy(vm, &lu)? {
        return Ok(Value::None);
    }
    let size = arg_or(&args[1..], kw, 0, "size", Value::None);
    let loops = arg_or(&args[1..], kw, 1, "loops", Value::Int(0));
    if !py::is_none(&size) {
        vset(vm, "default_size", size)?;
    }
    let loops = py::int(vm, &loops)?;
    let (files, loop_) = if loops == -1 {
        (vec![args[0].clone()], true)
    } else {
        (vec![args[0].clone(); (loops + 1).max(1) as usize], false)
    };
    let l = vm.types.new_list(files);
    music(vm, "play", vec![l], vec![("channel".into(), py::s(vm, "movie")), ("loop".into(), Value::Bool(loop_))])?;
    Ok(Value::None)
}

fn v_early_interact(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    for n in ["displayable_channels", "channel_movie"] {
        let d = vget(vm, n)?;
        vm.call_method(&d, "clear", Vec::new())?;
    }
    Ok(Value::None)
}

fn v_interact(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let tex = vget(vm, "texture")?;
    let keys = vm.call_method(&tex, "keys", Vec::new())?;
    for k in py::items(vm, &keys)? {
        if !playing_on(vm, &k)? {
            ops::delitem(vm, &tex, k)?;
        }
    }
    let movie = py::s(vm, "movie");
    let fullscreen = if playing_on(vm, &movie)? {
        let dc = vget(vm, "displayable_channels")?;
        let mut any = false;
        for k in py::items(vm, &dc)? {
            let first = ops::getitem(vm, &k, Value::Int(0))?;
            if first.as_str() == Some("movie") {
                any = true;
                break;
            }
        }
        !any
    } else {
        false
    };
    vset(vm, "fullscreen", Value::Bool(fullscreen))?;
    Ok(Value::Bool(fullscreen))
}

/// The video frame size reported for a channel's playing movie.
fn frame_size(vm: &mut Vm, channel: &Value) -> PyResult<Option<[u32; 2]>> {
    let c = super::audio::get_channel(vm, channel)?;
    let n = py::get(vm, &c, "_number")?;
    if py::is_none(&n) {
        return Ok(None);
    }
    let n = py::int(vm, &n)?.to_string();
    Ok(super::link::audio_status(vm).into_iter().find(|a| a.channel == n).and_then(|a| a.video_size))
}

/// `get_movie_texture(channel, mask_channel=None, side_mask=False,
/// mipmap=None)` -> (texture or None, new).
fn v_get_movie_texture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let channel = args[0].clone();
    if !playing_on(vm, &channel)? {
        return Ok(vm.types.new_tuple(vec![Value::None, Value::Bool(false)]));
    }
    let Some([w, h]) = frame_size(vm, &channel)? else {
        return Ok(vm.types.new_tuple(vec![Value::None, Value::Bool(false)]));
    };
    let name = py::string(vm, &channel)?;
    let tex = vget(vm, "texture")?;
    let new = !ops::contains(vm, &tex, &channel)?;
    let t = render::new_texture(vm, Texture { kind: TexKind::Video(name), width: w as f64, height: h as f64, crop: None, full: None, opaque: true });
    ops::setitem(vm, &tex, channel, t.clone())?;
    Ok(vm.types.new_tuple(vec![t, Value::Bool(new)]))
}

/// `resize_movie(r, width, height)`: scales the render to fit, centered.
fn v_resize_movie(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = &args[0];
    let (width, height) = (py::f64_of(vm, &args[1])?, py::f64_of(vm, &args[2])?);
    let sz = vm.call_method(r, "get_size", Vec::new())?;
    let s = py::items(vm, &sz)?;
    let (tw, th) = (py::f64_of(vm, &s[0])?, py::f64_of(vm, &s[1])?);
    if tw <= 0.0 || th <= 0.0 {
        return Ok(r.clone());
    }
    let ratio = (width / tw).min(height / th);
    let rc = py::import_attr(vm, "renpy.display.render", "Render")?;
    let rv = vm.call(&rc, vec![Value::Float(width), Value::Float(height)], Vec::new())?;
    let mc = py::import_attr(vm, "renpy.display.matrix", "Matrix")?;
    let scale = py::get(vm, &mc, "scale")?;
    let m = vm.call(&scale, vec![Value::Float(ratio), Value::Float(ratio), Value::Float(1.0)], Vec::new())?;
    let inv = vm.call(&scale, vec![Value::Float(1.0 / ratio), Value::Float(1.0 / ratio), Value::Float(1.0)], Vec::new())?;
    let sub = vm.call(&rc, vec![Value::Float(tw * ratio), Value::Float(th * ratio)], Vec::new())?;
    py::set(vm, &sub, "forward", inv)?;
    py::set(vm, &sub, "reverse", m)?;
    let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    vm.call_method(&sub, "blit", vec![r.clone(), pos])?;
    let pos = vm.types.new_tuple(vec![Value::Float((width - tw * ratio) / 2.0), Value::Float((height - th * ratio) / 2.0)]);
    vm.call_method(&rv, "blit", vec![sub, pos])?;
    Ok(rv)
}

fn v_render_movie(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = v_get_movie_texture(vm, &args[..1], &[])?;
    let tex = ops::getitem(vm, &r, Value::Int(0))?;
    if py::is_none(&tex) {
        return Ok(Value::None);
    }
    let sz = vm.call_method(&tex, "get_size", Vec::new())?;
    let s = py::items(vm, &sz)?;
    let rc = py::import_attr(vm, "renpy.display.render", "Render")?;
    let rv = vm.call(&rc, s, Vec::new())?;
    let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    vm.call_method(&rv, "blit", vec![tex, pos])?;
    v_resize_movie(vm, &[rv, args[1].clone(), args[2].clone()], &[])
}

fn v_default_play_callback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let new = &args[1];
    let (play, channel, loop_) = (py::get(vm, new, "_play")?, py::get(vm, new, "channel")?, py::get(vm, new, "loop")?);
    music(vm, "play", vec![play], vec![("channel".into(), channel), ("loop".into(), loop_.clone())])?;
    let mask = py::get(vm, new, "mask")?;
    if py::truthy(vm, &mask)? {
        let mc = py::get(vm, new, "mask_channel")?;
        music(vm, "play", vec![mask], vec![("channel".into(), mc), ("loop".into(), loop_)])?;
    }
    Ok(Value::None)
}

fn any_loadable(vm: &mut Vm, name: &Value) -> PyResult<bool> {
    let names = if name.as_str().is_some() { vec![name.clone()] } else { py::items(vm, name)? };
    for n in names {
        let s = py::string(vm, &n)?;
        let s = match s.strip_prefix('<').and_then(|r| r.split_once('>')) {
            Some((_, f)) => f.to_owned(),
            None => s,
        };
        if super::resloader::loadable(vm, &s, true, Some("audio"))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn m_any_loadable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(any_loadable(vm, &args[1])?))
}

fn ensure_channel(vm: &mut Vm, this: &Value, name: &Value) -> PyResult<()> {
    if py::is_none(name) {
        return Ok(());
    }
    let d = music(vm, "channel_defined", vec![name.clone()], Vec::new())?;
    if py::truthy(vm, &d)? {
        return Ok(());
    }
    let mask = py::get(vm, this, "mask")?;
    let framedrop = py::truthy(vm, &mask)?;
    let mixer = cfg(vm, "movie_mixer");
    music(vm, "register_channel", vec![name.clone(), mixer], vec![
        ("loop".into(), Value::Bool(true)),
        ("stop_on_mute".into(), Value::Bool(false)),
        ("movie".into(), Value::Bool(true)),
        ("framedrop".into(), Value::Bool(framedrop)),
        ("force".into(), Value::Bool(true)),
    ])?;
    Ok(())
}

fn ensure_channels(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let c = py::get(vm, this, "channel")?;
    ensure_channel(vm, this, &c)?;
    let mc = py::get(vm, this, "mask_channel")?;
    ensure_channel(vm, this, &mc)
}

fn m_ensure_channel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ensure_channel(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn m_ensure_channels(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ensure_channels(vm, &args[0])?;
    Ok(Value::None)
}

fn next_serial(vm: &mut Vm) -> PyResult<i64> {
    let s = vget(vm, "movie_channel_serial")?;
    let n = py::int(vm, &s)?;
    vset(vm, "movie_channel_serial", Value::Int(n + 1))?;
    Ok(n)
}

const MOVIE_ARGS: &[&str] = &["fps", "size", "channel", "play", "mask", "mask_channel", "image", "play_callback", "side_mask", "loop", "start_image", "group", "keep_last_frame"];

fn m_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v: Vec<Value> = MOVIE_ARGS.iter().enumerate().map(|(i, n)| arg_or(&args[1..], kw, i, n, Value::None)).collect();
    let props: Vec<_> = kw.iter().filter(|(k, _)| !MOVIE_ARGS.contains(&&**k)).cloned().collect();
    let base = py::import_attr(vm, "renpy.display.displayable", "Displayable")?;
    let init = py::get(vm, &base, "__init__")?;
    vm.call(&init, vec![this.clone()], props)?;
    let mut channel = if py::is_none(&v[2]) { py::s(vm, "movie") } else { v[2].clone() };
    let play = v[3].clone();
    let has_play = py::truthy(vm, &play)?;
    if channel.as_str() == Some("movie") && has_play {
        let smc = cfg(vm, "single_movie_channel");
        if py::truthy(vm, &smc)? {
            channel = smc;
        } else if {
            let a = cfg(vm, "auto_movie_channel");
            py::truthy(vm, &a)?
        } {
            let n = next_serial(vm)?;
            channel = vm.types.new_str(&format!("_movie_{n}"));
        }
    }
    py::set(vm, this, "size", v[1].clone())?;
    py::set(vm, this, "channel", channel.clone())?;
    py::set(vm, this, "loop", if py::is_none(&v[9]) { Value::Bool(true) } else { v[9].clone() })?;
    py::set(vm, this, "_original_play", play.clone())?;
    if !py::is_none(&play) && any_loadable(vm, &play)? {
        py::set(vm, this, "_play", play)?;
    }
    let side_mask = py::truthy(vm, &v[8])?;
    let mask = if side_mask { Value::None } else { v[4].clone() };
    py::set(vm, this, "mask", mask.clone())?;
    let mc = if py::is_none(&mask) {
        Value::None
    } else if py::is_none(&v[5]) {
        let c = py::string(vm, &channel)?;
        vm.types.new_str(&format!("{c}_mask"))
    } else {
        v[5].clone()
    };
    py::set(vm, this, "mask_channel", mc)?;
    py::set(vm, this, "side_mask", Value::Bool(side_mask))?;
    ensure_channels(vm, this)?;
    let don = py::import_attr(vm, "renpy.easy", "displayable_or_none")?;
    let image = vm.call(&don, vec![v[6].clone()], Vec::new())?;
    py::set(vm, this, "image", image.clone())?;
    let start = vm.call(&don, vec![v[10].clone()], Vec::new())?;
    py::set(vm, this, "start_image", start.clone())?;
    py::set(vm, this, "play_callback", v[7].clone())?;
    let mut group = v[11].clone();
    if py::is_none(&group) && py::truthy(vm, &v[12])? {
        let cls = vget(vm, "Movie")?;
        let s = py::get(vm, &cls, "keep_last_frame_serial")?;
        let s = py::int(vm, &s)?;
        py::set(vm, &cls, "keep_last_frame_serial", Value::Int(s + 1))?;
        group = vm.types.new_str(&format!("_keep_last_frame_{s}"));
    }
    py::set(vm, this, "group", group)?;
    for d in [image, start] {
        if !py::is_none(&d) {
            let dup = py::get_opt(vm, &d, "_duplicatable")?.unwrap_or(Value::Bool(false));
            if py::truthy(vm, &dup)? {
                py::set(vm, this, "_duplicatable", Value::Bool(true))?;
            }
        }
    }
    Ok(Value::None)
}

fn m_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let op = py::get(vm, this, "_original_play")?;
    let play = if py::is_none(&op) { py::get(vm, this, "_play")? } else { op };
    if !py::is_none(&play) && any_loadable(vm, &play)? {
        py::set(vm, this, "_original_play", play.clone())?;
        py::set(vm, this, "_play", play)?;
    } else {
        py::set(vm, this, "_play", Value::None)?;
        py::set(vm, this, "_original_play", play)?;
    }
    let ch = py::get(vm, this, "channel")?;
    if let Some(c) = ch.as_str() {
        if c.contains(' ') || c.contains('/') {
            let n = next_serial(vm)?;
            let name = vm.types.new_str(&format!("_movie_{n}"));
            py::set(vm, this, "channel", name)?;
            let mc = py::get(vm, this, "mask_channel")?;
            if !py::is_none(&mc) {
                let m = vm.types.new_str(&format!("_movie_{n}_mask"));
                py::set(vm, this, "mask_channel", m)?;
            }
        }
    }
    Ok(Value::None)
}

fn m_duplicate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let dup = py::get_opt(vm, this, "_duplicatable")?.unwrap_or(Value::Bool(false));
    if !py::truthy(vm, &dup)? {
        return Ok(this.clone());
    }
    let rv = vm.call_method(this, "_copy", vec![args[1].clone()])?;
    for n in ["image", "start_image"] {
        let d = py::get(vm, &rv, n)?;
        if !py::is_none(&d) {
            let dd = py::get_opt(vm, &d, "_duplicatable")?.unwrap_or(Value::Bool(false));
            if py::truthy(vm, &dd)? {
                let c = vm.call_method(&d, "_duplicate", vec![args[1].clone()])?;
                py::set(vm, &rv, n, c)?;
            }
        }
    }
    Ok(rv)
}

fn m_handles_event(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(args[1].as_str() == Some("show")))
}

fn m_set_transform_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args[1].as_str() == Some("show") {
        let rc = vget(vm, "reset_channels")?;
        let c = py::get(vm, &args[0], "channel")?;
        ops::set_add(vm, &rc, c)?;
    }
    Ok(Value::None)
}

fn sub_render(vm: &mut Vm, d: &Value, w: &Value, h: &Value, st: &Value, at: &Value) -> PyResult<Value> {
    let render = py::import_attr(vm, "renpy.display.render", "render")?;
    let surf = vm.call(&render, vec![d.clone(), w.clone(), h.clone(), st.clone(), at.clone()], Vec::new())?;
    let sz = vm.call_method(&surf, "get_size", Vec::new())?;
    let s = py::items(vm, &sz)?;
    let rc = py::import_attr(vm, "renpy.display.render", "Render")?;
    let rv = vm.call(&rc, s, Vec::new())?;
    let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    vm.call_method(&rv, "blit", vec![surf, pos])?;
    Ok(rv)
}

fn m_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = (args[1].clone(), args[2].clone(), args[3].clone(), args[4].clone());
    ensure_channels(vm, this)?;
    let channel = py::get(vm, this, "channel")?;
    let play = py::get(vm, this, "_play")?;
    let p = prefs(vm)?;
    let vif = py::get_opt(vm, &p, "video_image_fallback")?.unwrap_or(Value::Bool(false));
    if py::truthy(vm, &play)? && !matches!(vif, Value::Bool(true)) {
        let cm = vget(vm, "channel_movie")?;
        let cur = vm.call_method(&cm, "get", vec![channel.clone(), Value::None])?;
        if !py::same(&cur, this) {
            ops::setitem(vm, &cm, channel.clone(), this.clone())?;
        }
    }
    let mut not_playing = !playing_on(vm, &channel)?;
    let rc = vget(vm, "reset_channels")?;
    if ops::contains(vm, &rc, &channel)? {
        not_playing = false;
    }
    let group = py::get(vm, this, "group")?;
    let gt = vget(vm, "group_texture")?;
    if !py::is_none(&group) && ops::contains(vm, &gt, &group)? {
        not_playing = false;
    }
    let image = py::get(vm, this, "image")?;
    if !py::is_none(&image) && not_playing {
        return sub_render(vm, &image, &w, &h, &st, &at);
    }
    let mask_channel = py::get(vm, this, "mask_channel")?;
    let r = v_get_movie_texture(vm, &[channel.clone(), mask_channel], &[])?;
    let mut tex = ops::getitem(vm, &r, Value::Int(0))?;
    if !py::is_none(&group) {
        if py::is_none(&tex) {
            tex = vm.call_method(&gt, "get", vec![group.clone(), Value::None])?;
        } else {
            ops::setitem(vm, &gt, group, tex.clone())?;
        }
    }
    let start = py::get(vm, this, "start_image")?;
    let rcls = py::import_attr(vm, "renpy.display.render", "Render")?;
    let mut rv = if !not_playing && !py::is_none(&tex) {
        let sz = vm.call_method(&tex, "get_size", Vec::new())?;
        let s = py::items(vm, &sz)?;
        let rv = vm.call(&rcls, s, Vec::new())?;
        let pos = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
        vm.call_method(&rv, "blit", vec![tex, pos])?;
        rv
    } else if !not_playing && !py::is_none(&start) {
        sub_render(vm, &start, &w, &h, &st, &at)?
    } else {
        vm.call(&rcls, vec![Value::Int(0), Value::Int(0)], Vec::new())?
    };
    let size = py::get(vm, this, "size")?;
    if !py::is_none(&size) {
        let s = py::items(vm, &size)?;
        rv = v_resize_movie(vm, &[rv, s[0].clone(), s[1].clone()], &[])?;
    }
    let redraw = py::import_attr(vm, "renpy.display.render", "redraw")?;
    vm.call(&redraw, vec![this.clone(), Value::Float(0.1)], Vec::new())?;
    Ok(rv)
}

fn m_play(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, old) = (&args[0], &args[1]);
    ensure_channels(vm, this)?;
    let old_play = if py::is_none(old) { Value::None } else { py::get(vm, old, "_play")? };
    let play = py::get(vm, this, "_play")?;
    if py::is_none(&play) && py::is_none(&old_play) {
        return Ok(Value::None);
    }
    let replay = py::config_bool(vm, "replay_movie_sprites").unwrap_or(true);
    if !ops::eq(vm, &play, &old_play)? || replay {
        if py::truthy(vm, &play)? {
            let cb = py::get(vm, this, "play_callback")?;
            if py::is_none(&cb) {
                v_default_play_callback(vm, &[old.clone(), this.clone()], &[])?;
            } else {
                vm.call(&cb, vec![old.clone(), this.clone()], Vec::new())?;
            }
        } else {
            let ch = py::get(vm, this, "channel")?;
            music(vm, "stop", vec![], vec![("channel".into(), ch), ("fadeout".into(), Value::Int(0))])?;
            let mask = py::get(vm, this, "mask")?;
            if py::truthy(vm, &mask)? {
                let mc = py::get(vm, this, "mask_channel")?;
                music(vm, "stop", vec![], vec![("channel".into(), mc), ("fadeout".into(), Value::Int(0))])?;
            }
        }
    }
    Ok(Value::None)
}

fn m_stop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    ensure_channels(vm, this)?;
    let play = py::get(vm, this, "_play")?;
    if !py::truthy(vm, &play)? {
        return Ok(Value::None);
    }
    for (n, need_mask) in [("channel", false), ("mask_channel", true)] {
        if need_mask {
            let mask = py::get(vm, this, "mask")?;
            if !py::truthy(vm, &mask)? {
                continue;
            }
        }
        let ch = py::get(vm, this, n)?;
        let d = music(vm, "channel_defined", vec![ch.clone()], Vec::new())?;
        if py::truthy(vm, &d)? {
            music(vm, "stop", vec![], vec![("channel".into(), ch), ("fadeout".into(), Value::Int(0))])?;
        }
    }
    Ok(Value::None)
}

fn m_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    ensure_channels(vm, this)?;
    let dc = vget(vm, "displayable_channels")?;
    let (c, mc) = (py::get(vm, this, "channel")?, py::get(vm, this, "mask_channel")?);
    let k = vm.types.new_tuple(vec![c, mc]);
    let l = ops::getitem(vm, &dc, k)?;
    vm.call_method(&l, "append", vec![this.clone()])?;
    let redraw = py::import_attr(vm, "renpy.display.render", "redraw")?;
    vm.call(&redraw, vec![this.clone(), Value::Int(0)], Vec::new())?;
    Ok(Value::None)
}

fn m_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "image")?;
    let b = py::get(vm, &args[0], "start_image")?;
    Ok(vm.types.new_list(vec![a, b]))
}

fn v_playing(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let movie = py::s(vm, "movie");
    if playing_on(vm, &movie)? {
        return Ok(Value::Bool(true));
    }
    let dc = vget(vm, "displayable_channels")?;
    for k in py::items(vm, &dc)? {
        let c = ops::getitem(vm, &k, Value::Int(0))?;
        if playing_on(vm, &c)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::None)
}

fn v_update_playing(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let ctx = super::exports::util::ctx(vm)?;
    let old = py::get(vm, &ctx, "movie")?;
    let cm = vget(vm, "channel_movie")?;
    let last = vget(vm, "last_channel_movie")?;
    let rc = vget(vm, "reset_channels")?;
    let replay = py::config_bool(vm, "replay_movie_sprites").unwrap_or(true);
    for (c, m) in ops::mapping_items(vm, &cm)? {
        let o = vm.call_method(&old, "get", vec![c.clone(), Value::None])?;
        let l = vm.call_method(&last, "get", vec![c.clone(), Value::None])?;
        let lp = py::get(vm, &m, "loop")?;
        if ops::contains(vm, &rc, &c)? && replay {
            vm.call_method(&m, "play", vec![o])?;
        } else if !py::same(&o, &m) {
            vm.call_method(&m, "play", vec![o])?;
        } else if py::truthy(vm, &lp)? && !py::same(&l, &m) {
            vm.call_method(&m, "play", vec![l])?;
        }
    }
    let mut stopped = Vec::new();
    for (c, m) in ops::mapping_items(vm, &last)? {
        if !ops::contains(vm, &cm, &c)? {
            stopped.push(c);
            vm.call_method(&m, "stop", Vec::new())?;
        }
    }
    for (c, m) in ops::mapping_items(vm, &old)? {
        if !ops::contains(vm, &cm, &c)? {
            let mut done = false;
            for s in &stopped {
                if ops::eq(vm, s, &c)? {
                    done = true;
                }
            }
            if !done {
                vm.call_method(&m, "stop", Vec::new())?;
            }
        }
    }
    let d = Value::Obj(vm.types.dict.clone());
    let n = vm.call(&d, vec![cm], Vec::new())?;
    py::set(vm, &ctx, "movie", n.clone())?;
    vset(vm, "last_channel_movie", n)?;
    vm.call_method(&rc, "clear", Vec::new())?;
    Ok(Value::None)
}

fn v_frequent(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    v_update_playing(vm, &[], &[])?;
    let old = vget(vm, "group_texture")?;
    let new = py::dict(vm);
    let dc = vget(vm, "displayable_channels")?;
    let vals = vm.call_method(&dc, "values", Vec::new())?;
    for movies in py::items(vm, &vals)? {
        for m in py::items(vm, &movies)? {
            let g = py::get(vm, &m, "group")?;
            if !py::is_none(&g) {
                let t = vm.call_method(&old, "get", vec![g.clone(), Value::None])?;
                ops::setitem(vm, &new, g, t)?;
            }
        }
    }
    vset(vm, "group_texture", new)?;
    let fs = vget(vm, "fullscreen")?;
    if py::truthy(vm, &fs)? {
        let movie = py::s(vm, "movie");
        return Ok(Value::Bool(frame_size(vm, &movie)?.is_some()));
    }
    Ok(Value::Bool(false))
}

fn video_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        VIDEO,
        &[
            ("movie_stop", v_movie_stop),
            ("movie_start", v_movie_start),
            ("movie_start_fullscreen", v_movie_start),
            ("movie_start_displayable", v_movie_start),
            ("early_interact", v_early_interact),
            ("interact", v_interact),
            ("get_movie_texture", v_get_movie_texture),
            ("resize_movie", v_resize_movie),
            ("render_movie", v_render_movie),
            ("default_play_callback", v_default_play_callback),
            ("playing", v_playing),
            ("update_playing", v_update_playing),
            ("frequent", v_frequent),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in ["texture", "channel_movie", "group_texture", "last_channel_movie"] {
        set(vm, n, py::dict(vm));
    }
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let list = Value::Obj(vm.types.list.clone());
    let dc = vm.call(&dd, vec![list], Vec::new())?;
    set(vm, "displayable_channels", dc);
    set(vm, "reset_channels", vm.types.new_set(Default::default()));
    set(vm, "fullscreen", Value::Bool(false));
    set(vm, "current_movie", Value::None);
    set(vm, "default_size", vm.types.new_tuple(vec![Value::Int(400), Value::Int(300)]));
    set(vm, "movie_channel_serial", Value::Int(0));
    let base = py::import_attr(vm, "renpy.display.displayable", "Displayable")?;
    let mc = py::class(
        vm,
        VIDEO,
        "Movie",
        &[base],
        &[
            ("__init__", m_init),
            ("any_loadable", m_any_loadable),
            ("after_setstate", m_after_setstate),
            ("ensure_channel", m_ensure_channel),
            ("ensure_channels", m_ensure_channels),
            ("_duplicate", m_duplicate),
            ("_handles_event", m_handles_event),
            ("set_transform_event", m_set_transform_event),
            ("render", m_render),
            ("play", m_play),
            ("stop", m_stop),
            ("per_interact", m_per_interact),
            ("visit", m_visit),
        ],
        &[
            ("fullscreen", Value::Bool(false)),
            ("channel", py::s(vm, "movie")),
            ("_play", Value::None),
            ("_original_play", Value::None),
            ("mask", Value::None),
            ("mask_channel", Value::None),
            ("side_mask", Value::Bool(false)),
            ("image", Value::None),
            ("start_image", Value::None),
            ("play_callback", Value::None),
            ("loop", Value::Bool(true)),
            ("group", Value::None),
            ("keep_last_frame_serial", Value::Int(0)),
        ],
    )?;
    set(vm, "Movie", Value::Obj(mc));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(TTS, tts_module);
    vm.register_native_module(VIDEO, video_module);
}
