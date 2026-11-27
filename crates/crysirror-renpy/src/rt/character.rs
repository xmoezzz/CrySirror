// SPDX-License-Identifier: MPL-2.0
//! `renpy.character`, after Ren'Py 8.3 `character.py` (MIT), with the
//! 6.x behaviour (checked against 6.18 and 6.99.12.4) where it differs:
//! unique say-screen tags and widget return in `show_display_say`,
//! `interact`-driven dialogue behaviour, `resolve_say_attributes(predict,
//! wanted, remove)`, callbacks without per-segment arguments and `_`
//! splitting of character properties.
//!
//! Features added during 7.x/8.x (`retain`, `multiple`, `{done}`,
//! `{nw=}` delays, extended callback arguments, `scry_extend`,
//! `nointeract_callbacks`, ...) are tied to the release (tag syntax) or
//! to the config variables that release defines, so a game sees the
//! behaviour of its own engine.
//!
//! The classes are ordinary Python classes with native methods, so the
//! common `NVLCharacter` and game subclasses override `do_show`,
//! `do_display` and friends as upstream.
use super::{
    exports::util::{ctx, ex, is_jump_or_call, isinst, store_get, store_set, ui},
    py::{self, *},
};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.character";

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm).map(|r| r.profile.at_least(major, minor)).unwrap_or(true)
}

/// Before 7.0.
fn e6(vm: &Vm) -> bool {
    !at_least(vm, 7, 0)
}

/// `renpy.config.<name>`, or `default` when the release has no such
/// variable.
fn cfg(vm: &mut Vm, name: &str, default: Value) -> Value {
    let m = match py::import(vm, "renpy.config") {
        Ok(m) => m,
        Err(_) => return default,
    };
    py::get_opt(vm, &m, name).ok().flatten().unwrap_or(default)
}

fn cfg_true(vm: &mut Vm, name: &str) -> PyResult<bool> {
    let v = cfg(vm, name, Value::None);
    py::truthy(vm, &v)
}

fn has_cfg(vm: &mut Vm, name: &str) -> bool {
    py::import(vm, "renpy.config").ok().and_then(|m| py::get_opt(vm, &m, name).ok().flatten()).is_some()
}

fn call_list(vm: &mut Vm, name: &str) -> PyResult<()> {
    let l = cfg(vm, name, Value::None);
    if py::is_none(&l) {
        return Ok(());
    }
    for f in py::items(vm, &l)? {
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(())
}

fn kw_of(items: &[(&str, Value)]) -> Vec<(Rc<str>, Value)> {
    items.iter().map(|(k, v)| (Rc::from(*k), v.clone())).collect()
}

fn dict_get(vm: &mut Vm, d: &Value, k: &str) -> PyResult<Value> {
    let key = vm.types.new_str(k);
    vm.call_method(d, "get", vec![key, Value::None])
}

fn dict_copy(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    vm.call_method(d, "copy", Vec::new())
}

fn is_str(v: &Value) -> bool {
    v.as_str().is_some()
}

fn substitute(vm: &mut Vm, s: &Value, translate: bool) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    let r = vm.call(&f, vec![s.clone()], kw_of(&[("translate", Value::Bool(translate))]))?;
    ops::getitem(vm, &r, Value::Int(0))
}

fn py_eval(vm: &mut Vm, e: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.python", "py_eval")?;
    vm.call(&f, vec![e.clone()], Vec::new())
}

fn displayable_or_none(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.easy", "displayable_or_none")?;
    vm.call(&f, vec![v.clone()], Vec::new())
}

fn unique_duplicate(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    if py::is_none(d) {
        return Ok(Value::None);
    }
    let dup = py::get_opt(vm, d, "_duplicatable")?.unwrap_or(Value::Bool(false));
    if !py::truthy(vm, &dup)? {
        return Ok(d.clone());
    }
    let rv = vm.call_method(d, "_duplicate", vec![Value::None])?;
    vm.call_method(&rv, "_unique", Vec::new())?;
    Ok(rv)
}

// ---------------------------------------------------------------------------
// DialogueTextTags.

/// The parsed pauses of a line of dialogue.
pub struct Dtt {
    pub text: String,
    pub pause_start: Vec<usize>,
    pub pause_end: Vec<usize>,
    pub pause_delay: Vec<Option<f64>>,
    pub no_wait: bool,
    pub has_done: bool,
    pub fast: bool,
}

enum Piece<'a> {
    Text(&'a str),
    Quoted,
    Tag { full: &'a str, tag: &'a str, value: Option<&'a str> },
}

/// `TAG_RE.split(s)`: `{{` and `{p}`, `{w}`, `{nw}`, `{fast}` (and
/// `{done}` from 7.4) with an optional `=value`.
fn split_tags(s: &str, done: bool) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        if b[i] != b'{' {
            i += 1;
            continue;
        }
        if b.get(i + 1) == Some(&b'{') {
            out.push(Piece::Text(&s[start..i]));
            out.push(Piece::Quoted);
            i += 2;
            start = i;
            continue;
        }
        let rest = &s[i + 1..];
        let mut matched = None;
        for tag in ["p", "w", "nw", "fast", "done"] {
            if tag == "done" && !done {
                continue;
            }
            let Some(after) = rest.strip_prefix(tag) else { continue };
            if let Some(after) = after.strip_prefix('}') {
                let _ = after;
                matched = Some((tag, None, 1 + tag.len() + 1));
            } else if let Some(v) = after.strip_prefix('=') {
                if let Some(end) = v.find('}') {
                    matched = Some((tag, Some(&v[..end]), 1 + tag.len() + 1 + end + 1));
                }
            }
            if matched.is_some() {
                break;
            }
        }
        match matched {
            Some((tag, value, len)) => {
                out.push(Piece::Text(&s[start..i]));
                out.push(Piece::Tag { full: &s[i..i + len], tag, value });
                i += len;
                start = i;
            }
            None => i += 1,
        }
    }
    out.push(Piece::Text(&s[start..]));
    out
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

pub fn parse_dtt(vm: &mut Vm, s: &str) -> PyResult<Dtt> {
    let done = at_least(vm, 7, 4);
    let nw_value = at_least(vm, 8, 2);
    let mut d = Dtt { text: String::new(), pause_start: vec![0], pause_end: Vec::new(), pause_delay: Vec::new(), no_wait: false, has_done: false, fast: false };
    let mut n = 0usize;
    let mut push = |d: &mut Dtt, n: &mut usize, t: &str| {
        d.text.push_str(t);
        *n += clen(t);
    };
    let pieces = split_tags(s, done);
    let mut it = pieces.into_iter();
    let mut finished = false;
    for p in it.by_ref() {
        match p {
            Piece::Text(t) => push(&mut d, &mut n, t),
            Piece::Quoted => push(&mut d, &mut n, "{{"),
            Piece::Tag { full, tag, value } => {
                let value = match value {
                    Some(v) => Some(v.trim().parse::<f64>().map_err(|_| vm.exc_msg("ValueError", format!("could not convert string to float: '{v}'")))?),
                    None => None,
                };
                match tag {
                    "p" | "w" => {
                        d.pause_start.push(n);
                        d.pause_end.push(n);
                        d.pause_delay.push(value);
                    }
                    "nw" => {
                        d.no_wait = true;
                        if nw_value && value.is_some() {
                            d.pause_start.push(n);
                            d.pause_end.push(n);
                            d.pause_delay.push(value);
                        }
                    }
                    "fast" => {
                        d.pause_start = vec![n];
                        d.pause_end.clear();
                        d.pause_delay.clear();
                        d.no_wait = false;
                        d.fast = true;
                    }
                    _ => {
                        d.has_done = true;
                        push(&mut d, &mut n, full);
                        finished = true;
                        break;
                    }
                }
                push(&mut d, &mut n, full);
            }
        }
    }
    d.pause_end.push(n);
    if finished {
        for p in it {
            match p {
                Piece::Text(t) => push(&mut d, &mut n, t),
                Piece::Quoted => push(&mut d, &mut n, "{{"),
                Piece::Tag { full, .. } => push(&mut d, &mut n, full),
            }
        }
    }
    d.pause_delay.push(if d.no_wait { Some(0.0) } else { None });
    Ok(d)
}

fn opt_f(v: Option<f64>) -> Value {
    v.map(Value::Float).unwrap_or(Value::None)
}

fn dtt_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::string(vm, &args[1])?;
    let d = parse_dtt(vm, &s)?;
    let ints = |vm: &Vm, v: &[usize]| vm.types.new_list(v.iter().map(|i| Value::Int(*i as i64)).collect());
    py::set(vm, this, "text", vm.types.new_str(&d.text))?;
    py::set(vm, this, "pause_start", ints(vm, &d.pause_start))?;
    py::set(vm, this, "pause_end", ints(vm, &d.pause_end))?;
    let delays = vm.types.new_list(d.pause_delay.iter().map(|v| opt_f(*v)).collect());
    py::set(vm, this, "pause_delay", delays)?;
    py::set(vm, this, "no_wait", Value::Bool(d.no_wait))?;
    if !e6(vm) {
        py::set(vm, this, "has_done", Value::Bool(d.has_done))?;
        py::set(vm, this, "fast", Value::Bool(d.fast))?;
    }
    Ok(Value::None)
}

fn new_dtt(vm: &mut Vm, what: &Value) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "DialogueTextTags")?;
    vm.call(&c, vec![what.clone()], Vec::new())
}

// ---------------------------------------------------------------------------
// Callbacks.

/// `Callbacks(callbacks, interact, type, cb_args, multiple)`. Releases
/// before the class existed called each callback with `interact`, `type`
/// and the character's `cb_args` only.
fn cb_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["callbacks", "interact", "type", "cb_args", "multiple"].iter().enumerate() {
        py::set(vm, this, n, args.get(i + 1).cloned().unwrap_or(Value::None))?;
    }
    for n in ["what", "start", "end", "delay", "last_segment"] {
        py::set(vm, this, n, Value::None)?;
    }
    Ok(Value::None)
}

fn cb_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let callbacks = py::get(vm, this, "callbacks")?;
    if !py::truthy(vm, &callbacks)? {
        return Ok(Value::None);
    }
    let mut kwargs: Vec<(Rc<str>, Value)> = kw.to_vec();
    kwargs.push(("type".into(), py::get(vm, this, "type")?));
    kwargs.push(("interact".into(), py::get(vm, this, "interact")?));
    // `config.character_callback_compat` arrived with the extended
    // arguments; a release without it gets the old call.
    if has_cfg(vm, "character_callback_compat") || at_least(vm, 8, 3) {
        let compat = cfg(vm, "character_callback_compat", Value::None);
        if py::is_none(&compat) {
            for n in ["what", "start", "end", "delay", "last_segment", "multiple"] {
                let v = py::get(vm, this, n)?;
                if !py::is_none(&v) {
                    kwargs.push((n.into(), v));
                }
            }
            kwargs.push(("please_ignore_unknown_keyword_arguments".into(), Value::None));
        }
    }
    let cba = py::get(vm, this, "cb_args")?;
    for (k, v) in py::str_items(vm, &cba)? {
        kwargs.retain(|(n, _)| *n != k);
        kwargs.push((k, v));
    }
    for c in py::items(vm, &callbacks)? {
        vm.call(&c, args[1..].to_vec(), kwargs.clone())?;
    }
    Ok(Value::None)
}

fn cb_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "Callbacks")?;
    let mut a = Vec::new();
    for n in ["callbacks", "interact", "type", "cb_args", "multiple"] {
        a.push(py::get(vm, &args[0], n)?);
    }
    let rv = vm.call(&c, a, Vec::new())?;
    let d = py::namespace_of(vm, &args[0])?;
    let nd = py::namespace_of(vm, &rv)?;
    vm.call_method(&nd, "update", vec![d])?;
    Ok(rv)
}

fn new_callbacks(vm: &mut Vm, callbacks: Value, interact: Value, type_: Value, cb_args: Value, multiple: Value) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "Callbacks")?;
    vm.call(&c, vec![callbacks, interact, type_, cb_args, multiple], Vec::new())
}

fn fire(vm: &mut Vm, cb: &Value, event: &str) -> PyResult<()> {
    vm.call(cb, vec![py::s(vm, event)], Vec::new())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Say screens.

/// `predict_show_display_say(...)`.
fn f_predict_show_display_say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = ["who", "what", "who_args", "what_args", "window_args", "image", "two_window", "side_image", "screen", "properties"];
    let mut v: Vec<Value> = names.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    if py::is_none(&v[5]) {
        v[5] = Value::Bool(false);
    }
    if py::is_none(&v[6]) {
        v[6] = Value::Bool(false);
    }
    let mut extra: Vec<(Rc<str>, Value)> = kw.iter().filter(|(k, _)| !names.contains(&&**k)).cloned().collect();
    let predict = py::import_attr(vm, "renpy.easy", "predict")?;
    if py::truthy(vm, &v[7])? {
        vm.call(&predict, vec![v[7].clone()], Vec::new())?;
    }
    let sia = store_get(vm, "_side_image_attributes")?;
    if py::truthy(vm, &sia)? {
        let ir = py::import_attr(vm, "renpy.display.image", "ImageReference")?;
        let mut parts = vec![py::s(vm, "side")];
        parts.extend(py::items(vm, &sia)?);
        let t = vm.types.new_tuple(parts);
        let r = vm.call(&ir, vec![t], Vec::new())?;
        vm.call(&predict, vec![r], Vec::new())?;
    }
    if py::truthy(vm, &v[5])? {
        if v[5].as_str() != Some("<Dynamic>") {
            vm.call(&predict, vec![v[0].clone()], Vec::new())?;
        }
        extra.push(("image".into(), v[5].clone()));
    }
    if py::truthy(vm, &v[8])? {
        let props = compute_widget_properties(vm, &v[2], &v[3], &v[4], &v[9], &Value::None, &Value::None)?;
        let ps = py::import_attr(vm, "renpy.display.predict", "screen")?;
        let mut kwargs = vec![
            ("_widget_properties".into(), props),
            ("who".into(), v[0].clone()),
            ("what".into(), v[1].clone()),
            ("two_window".into(), v[6].clone()),
            ("side_image".into(), v[7].clone()),
        ];
        kwargs.extend(extra);
        vm.call(&ps, vec![v[8].clone()], kwargs)?;
    }
    Ok(Value::None)
}

/// `compute_widget_properties(who_args, what_args, window_args,
/// properties, variant=None, multiple=None)`.
fn compute_widget_properties(vm: &mut Vm, who_args: &Value, what_args: &Value, window_args: &Value, properties: &Value, variant: &Value, multiple: &Value) -> PyResult<Value> {
    let style_args = |vm: &mut Vm, d: &Value, name: &str| -> PyResult<Value> {
        let mut style = dict_get(vm, d, "style")?;
        if py::is_none(&style) {
            if py::is_none(multiple) {
                return Ok(d.clone());
            }
            style = py::s(vm, name);
        }
        let ir = ex(vm, "in_rollback", Vec::new(), Vec::new())?;
        let in_rollback = py::truthy(vm, &ir)?;
        if !in_rollback && !py::truthy(vm, variant)? && !py::truthy(vm, multiple)? {
            return Ok(d.clone());
        }
        let d = dict_copy(vm, d)?;
        if let Some(s) = style.as_str() {
            let mut s = s.to_owned();
            if !py::is_none(multiple) {
                let m = py::items(vm, multiple)?;
                let (a, b) = (py::int(vm, &m[0])?, py::int(vm, &m[1])?);
                s = format!("block{a}_multiple{b}_{s}");
            }
            let mgr = py::import_attr(vm, "store", "style")?;
            style = py::get(vm, &mgr, &s)?;
            if !py::is_none(variant) {
                style = ops::getitem(vm, &style, variant.clone())?;
            }
            if in_rollback {
                style = ops::getitem(vm, &style, py::s(vm, "rollback"))?;
            }
        }
        py::dict_set(vm, &d, "style", style)?;
        Ok(d)
    };
    let who = style_args(vm, who_args, "who")?;
    let what = style_args(vm, what_args, "what")?;
    let window = style_args(vm, window_args, "window")?;
    let rv = if py::is_none(properties) { py::dict(vm) } else {
        let d = Value::Obj(vm.types.dict.clone());
        vm.call(&d, vec![properties.clone()], Vec::new())?
    };
    if !e6(vm) && !py::is_none(properties) {
        let prefixes = cfg(vm, "character_id_prefixes", Value::None);
        if !py::is_none(&prefixes) {
            for p in py::items(vm, &prefixes)? {
                let pn = py::string(vm, &p)?;
                let sub = dict_get(vm, properties, &pn)?;
                let sub = if py::is_none(&sub) { py::dict(vm) } else { sub };
                let v = style_args(vm, &sub, &pn)?;
                ops::setitem(vm, &rv, p, v)?;
            }
        }
    }
    py::dict_set(vm, &rv, "window", window)?;
    py::dict_set(vm, &rv, "what", what)?;
    py::dict_set(vm, &rv, "who", who)?;
    Ok(rv)
}

fn f_compute_widget_properties(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let get = |i: usize, n: &str| arg_or(args, kw, i, n, Value::None);
    // 6.x: (who_args, what_args, window_args, variant=None).
    if e6(vm) {
        return compute_widget_properties(vm, &get(0, "who_args"), &get(1, "what_args"), &get(2, "window_args"), &Value::None, &get(3, "variant"), &Value::None);
    }
    compute_widget_properties(vm, &get(0, "who_args"), &get(1, "what_args"), &get(2, "window_args"), &get(3, "properties"), &get(4, "variant"), &get(5, "multiple"))
}

const SDS_NAMES: &[&str] = &[
    "who", "what", "who_args", "what_args", "window_args", "image", "side_image", "two_window", "two_window_vbox_properties", "who_window_properties",
    "say_vbox_properties", "transform", "variant", "screen", "layer", "properties", "multiple", "retain",
];

/// `show_display_say(who, what, ...)`: shows the say screen (or builds
/// the legacy window) and returns the `what` widget, or from 7.0 the
/// `(tag, "what", layer)` that names it.
fn f_show_display_say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut v: Vec<Value> = SDS_NAMES.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    let mut extra: Vec<(Rc<str>, Value)> = kw.iter().filter(|(k, _)| !SDS_NAMES.contains(&&**k)).cloned().collect();
    for i in [2, 3, 4, 8, 9, 10, 15] {
        if py::is_none(&v[i]) {
            v[i] = py::dict(vm);
        }
    }
    for i in [5, 7] {
        if py::is_none(&v[i]) {
            v[i] = Value::Bool(false);
        }
    }
    let (who, what, who_args) = (v[0].clone(), v[1].clone(), v[2].clone());
    let (image, side_image, two_window) = (v[5].clone(), v[6].clone(), v[7].clone());
    let (variant, screen, mut layer, multiple, retain) = (v[12].clone(), v[13].clone(), v[14].clone(), v[16].clone(), v[17].clone());
    let props = if e6(vm) {
        compute_widget_properties(vm, &v[2], &v[3], &v[4], &Value::None, &variant, &Value::None)?
    } else {
        compute_widget_properties(vm, &v[2], &v[3], &v[4], &v[15], &variant, &multiple)?
    };
    let has_screen = py::import_attr(vm, "renpy.display.screen", "has_screen")?;
    let screen_ok = py::truthy(vm, &screen)? && {
        let r = vm.call(&has_screen, vec![screen.clone()], Vec::new())?;
        py::truthy(vm, &r)?
    };
    if screen_ok {
        if py::is_none(&layer) {
            layer = cfg(vm, "say_layer", py::s(vm, "screens"));
        }
        let mut screen = screen.clone();
        let mut tag = screen.clone();
        if e6(vm) {
            let s = py::string(vm, &screen)?;
            let mut index = 0;
            loop {
                let showing = ex(vm, "showing", vec![tag.clone()], Vec::new())?;
                if !py::truthy(vm, &showing)? {
                    break;
                }
                index += 1;
                tag = vm.types.new_str(&format!("{s}{index}"));
            }
        } else if py::truthy(vm, &retain)? {
            tag = retain.clone();
        } else if py::truthy(vm, &multiple)? {
            let s = py::string(vm, &screen)?;
            let ms = vm.types.new_str(&format!("multiple_{s}"));
            let r = vm.call(&has_screen, vec![ms.clone()], Vec::new())?;
            if py::truthy(vm, &r)? {
                screen = ms;
                extra.push(("multiple".into(), multiple.clone()));
            }
            let m = py::items(vm, &multiple)?;
            let (a, b) = (py::int(vm, &m[0])?, py::int(vm, &m[1])?);
            tag = vm.types.new_str(&format!("block{a}_multiple{b}_{s}"));
        }
        if py::truthy(vm, &image)? {
            extra.push(("image".into(), image.clone()));
        }
        let old_args = cfg_true(vm, "old_say_args")?;
        if !py::is_none(&side_image) || old_args {
            extra.push(("side_image".into(), side_image.clone()));
        }
        if py::truthy(vm, &two_window)? || old_args {
            extra.push(("two_window".into(), two_window.clone()));
        }
        let transient = if e6(vm) { true } else { !py::truthy(vm, &retain)? };
        let mut kwargs: Vec<(Rc<str>, Value)> = vec![
            ("_widget_properties".into(), props),
            ("_transient".into(), Value::Bool(transient)),
            ("_tag".into(), tag.clone()),
            ("who".into(), who),
            ("what".into(), what),
            ("_layer".into(), layer.clone()),
        ];
        kwargs.extend(extra);
        let ss = py::import_attr(vm, "renpy.display.screen", "show_screen")?;
        vm.call(&ss, vec![screen.clone()], kwargs)?;
        ex(vm, "shown_window", Vec::new(), Vec::new())?;
        if e6(vm) {
            let gw = py::import_attr(vm, "renpy.display.screen", "get_widget")?;
            return vm.call(&gw, vec![screen, py::s(vm, "what"), layer], Vec::new());
        }
        return Ok(vm.types.new_tuple(vec![tag, py::s(vm, "what"), layer]));
    }
    let merge_style = |vm: &mut Vm, style: &str, properties: &Value| -> PyResult<Vec<(Rc<str>, Value)>> {
        let mgr = py::import_attr(vm, "store", "style")?;
        let mut s = py::get(vm, &mgr, style)?;
        if !py::is_none(&variant) {
            s = ops::getitem(vm, &s, variant.clone())?;
        }
        let ir = ex(vm, "in_rollback", Vec::new(), Vec::new())?;
        if py::truthy(vm, &ir)? {
            s = ops::getitem(vm, &s, py::s(vm, "rollback"))?;
        }
        let mut rv: Vec<(Rc<str>, Value)> = vec![("style".into(), s)];
        for (k, v) in py::str_items(vm, properties)? {
            rv.retain(|(n, _)| *n != k);
            rv.push((k, v));
        }
        Ok(rv)
    };
    let handle_who = |vm: &mut Vm| -> PyResult<()> {
        if !py::truthy(vm, &who)? {
            return Ok(());
        }
        if py::truthy(vm, &image)? {
            let wp = dict_get(vm, &props, "who")?;
            let kw = py::str_items(vm, &wp)?;
            let mut kw2 = vec![("loose".into(), Value::Bool(true))];
            kw2.extend(kw);
            let im = py::import_attr(vm, "renpy.display.im", "image")?;
            let d = vm.call(&im, vec![who.clone()], kw2)?;
            ui(vm, "add", vec![d], Vec::new())?;
        } else {
            let kw = py::str_items(vm, &who_args)?;
            ui(vm, "text", vec![who.clone()], kw)?;
        }
        Ok(())
    };
    if py::truthy(vm, &v[11])? {
        ui(vm, "at", vec![v[11].clone()], Vec::new())?;
    }
    let two = py::truthy(vm, &two_window)?;
    if two {
        let k = merge_style(vm, "say_two_window_vbox", &v[8])?;
        ui(vm, "vbox", Vec::new(), k)?;
        let k = merge_style(vm, "say_who_window", &v[9])?;
        ui(vm, "window", Vec::new(), k)?;
        handle_who(vm)?;
    }
    let wp = dict_get(vm, &props, "window")?;
    let k = py::str_items(vm, &wp)?;
    ui(vm, "window", Vec::new(), k)?;
    let k = merge_style(vm, "say_vbox", &v[10])?;
    ui(vm, "vbox", Vec::new(), k)?;
    if !two {
        handle_who(vm)?;
    }
    let tp = dict_get(vm, &props, "what")?;
    let k = py::str_items(vm, &tp)?;
    let rv = ui(vm, "text", vec![v[1].clone()], k)?;
    ui(vm, "close", Vec::new(), Vec::new())?;
    if two {
        ui(vm, "close", Vec::new(), Vec::new())?;
    }
    if py::truthy(vm, &side_image)? {
        ui(vm, "image", vec![side_image], Vec::new())?;
    }
    ex(vm, "shown_window", Vec::new(), Vec::new())?;
    Ok(rv)
}

// ---------------------------------------------------------------------------
// SlowDone.

const SD_NAMES: &[&str] = &["ctc", "ctc_position", "callback", "interact", "type", "cb_args", "delay", "ctc_kwargs", "last_pause", "no_wait"];

fn sd_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in SD_NAMES.iter().enumerate() {
        let d = match *n {
            "ctc_kwargs" => py::dict(vm),
            "last_pause" => Value::Bool(true),
            "no_wait" => Value::Bool(false),
            _ => Value::None,
        };
        let v = arg_or(&args[1..], kw, i, n, d);
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn sd_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let g = |vm: &mut Vm, n: &str| py::get(vm, this, n);
    let (ctc, pos, interact, delay) = (g(vm, "ctc")?, g(vm, "ctc_position")?, g(vm, "interact")?, g(vm, "delay")?);
    let old = e6(vm);
    let zero_delay = matches!(delay, Value::Int(0)) || matches!(delay, Value::Float(f) if f == 0.0);
    let show_ctc = if old { true } else { py::truthy(vm, &interact)? && !zero_delay };
    if show_ctc {
        let has_screen = py::import_attr(vm, "renpy.display.screen", "has_screen")?;
        let r = vm.call(&has_screen, vec![py::s(vm, "ctc")], Vec::new())?;
        let ss = py::import_attr(vm, "renpy.display.screen", "show_screen")?;
        if py::truthy(vm, &r)? {
            let mut a = vec![py::s(vm, "ctc")];
            if py::truthy(vm, &ctc)? {
                a.push(ctc.clone());
            }
            let mut kwargs: Vec<(Rc<str>, Value)> = vec![("_transient".into(), Value::Bool(true))];
            if !old {
                kwargs.push(("_ignore_extra_kwargs".into(), Value::Bool(true)));
                let ck = g(vm, "ctc_kwargs")?;
                kwargs.extend(py::str_items(vm, &ck)?);
            }
            vm.call(&ss, a, kwargs)?;
            ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
        } else if py::truthy(vm, &ctc)? && pos.as_str() == Some("fixed") {
            vm.call(&ss, vec![py::s(vm, "_ctc")], kw_of(&[("_transient", Value::Bool(true)), ("ctc", ctc.clone())]))?;
            ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
        }
    }
    if !py::is_none(&delay) {
        let mut kwargs = Vec::new();
        if !old && has_cfg(vm, "nointeract_callbacks") {
            let last = g(vm, "last_pause")?;
            let nw = g(vm, "no_wait")?;
            let lp = py::truthy(vm, &last)?;
            kwargs.push(("voice".into(), Value::Bool(lp && !py::truthy(vm, &nw)?)));
            kwargs.push(("self_voicing".into(), Value::Bool(lp)));
        }
        ui(vm, "pausebehavior", vec![delay, Value::Bool(true)], kwargs)?;
        ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
    }
    let cb = g(vm, "callback")?;
    if old {
        let mut kwargs = vec![("interact".into(), interact), ("type".into(), g(vm, "type")?)];
        let cba = g(vm, "cb_args")?;
        kwargs.extend(py::str_items(vm, &cba)?);
        for c in py::items(vm, &cb)? {
            vm.call(&c, vec![py::s(vm, "slow_done")], kwargs.clone())?;
        }
    } else {
        fire(vm, &cb, "slow_done")?;
    }
    Ok(Value::None)
}

fn new_slow_done(vm: &mut Vm, values: Vec<Value>) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "SlowDone")?;
    vm.call(&c, values, Vec::new())
}

// ---------------------------------------------------------------------------
// display_say.

const DS_NAMES: &[&str] = &[
    "who", "what", "show_function", "interact", "slow", "afm", "ctc", "ctc_pause", "ctc_position", "all_at_once", "cb_args", "with_none", "callback",
    "type", "checkpoint", "ctc_timedpause", "ctc_force", "advance", "multiple", "dtt", "retain",
];

struct Say {
    who: Value,
    what: Value,
    show_function: Value,
    interact: bool,
    slow: bool,
    afm: Value,
    ctc: Value,
    ctc_pause: Value,
    ctc_position: Value,
    all_at_once: bool,
    cb_args: Value,
    with_none: Value,
    callback: Value,
    type_: Value,
    checkpoint: Value,
    ctc_timedpause: Value,
    ctc_force: bool,
    advance: bool,
    multiple: Value,
    dtt: Value,
    retain: bool,
}

fn say_args(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Say> {
    if let Some((k, _)) = kw.iter().find(|(k, _)| !DS_NAMES.contains(&&**k)) {
        return Err(vm.type_error(format!("display_say() got an unexpected keyword argument '{k}'")));
    }
    let v: Vec<Value> = DS_NAMES.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    let b = |vm: &mut Vm, i: usize, d: bool| -> PyResult<bool> { if py::is_none(&v[i]) && DS_NAMES[i] != "interact" { Ok(d) } else { py::truthy(vm, &v[i]) } };
    Ok(Say {
        who: v[0].clone(),
        what: v[1].clone(),
        show_function: v[2].clone(),
        interact: b(vm, 3, false)?,
        slow: b(vm, 4, false)?,
        afm: v[5].clone(),
        ctc: v[6].clone(),
        ctc_pause: v[7].clone(),
        ctc_position: v[8].clone(),
        all_at_once: b(vm, 9, false)?,
        cb_args: if py::is_none(&v[10]) { py::dict(vm) } else { v[10].clone() },
        with_none: v[11].clone(),
        callback: v[12].clone(),
        type_: v[13].clone(),
        checkpoint: if kw.iter().any(|(k, _)| &**k == "checkpoint") || args.len() > 14 { v[14].clone() } else { Value::Bool(true) },
        ctc_timedpause: v[15].clone(),
        ctc_force: b(vm, 16, false)?,
        advance: if kw.iter().any(|(k, _)| &**k == "advance") || args.len() > 17 { py::truthy(vm, &v[17])? } else { true },
        multiple: v[18].clone(),
        dtt: v[19].clone(),
        retain: b(vm, 20, false)?,
    })
}

fn callback_list(vm: &mut Vm, callback: &Value) -> PyResult<Value> {
    let mut cb = callback.clone();
    if py::is_none(&cb) {
        let c = cfg(vm, "character_callback", Value::None);
        cb = if py::truthy(vm, &c)? { vm.types.new_list(vec![c]) } else { vm.types.new_list(Vec::new()) };
    }
    if !matches!(cb.payload(), Some(crysirror_python::Payload::List(_))) {
        cb = vm.types.new_list(vec![cb]);
    }
    let all = cfg(vm, "all_character_callbacks", Value::None);
    let mut items = if py::is_none(&all) { Vec::new() } else { py::items(vm, &all)? };
    items.extend(py::items(vm, &cb)?);
    Ok(vm.types.new_list(items))
}

fn preferences(vm: &mut Vm, name: &str) -> PyResult<bool> {
    let p = py::import_attr(vm, "renpy.game", "preferences")?;
    let v = py::get(vm, &p, name)?;
    py::truthy(vm, &v)
}

fn seen_current(vm: &mut Vm) -> PyResult<bool> {
    let c = ctx(vm)?;
    let r = vm.call_method(&c, "seen_current", vec![Value::Bool(true)])?;
    py::truthy(vm, &r)
}

fn skipping(vm: &mut Vm) -> Value {
    cfg(vm, "skipping", Value::None)
}

fn set_skipping_none(vm: &mut Vm) -> PyResult<()> {
    super::exports::util::config_set(vm, "skipping", Value::None)
}

fn checkpoint(vm: &mut Vm, data: Value, hard: Option<Value>) -> PyResult<()> {
    let kw = match hard {
        Some(h) => vec![("hard".into(), h)],
        None => Vec::new(),
    };
    ex(vm, "checkpoint", vec![data], kw)?;
    Ok(())
}

fn with_none_end(vm: &mut Vm, with_none: &Value) -> PyResult<()> {
    let wn = if py::is_none(with_none) { cfg(vm, "implicit_with_none", Value::Bool(true)) } else { with_none.clone() };
    if py::truthy(vm, &wn)? {
        let iface = py::import_attr(vm, "renpy.game", "interface")?;
        vm.call_method(&iface, "do_with", vec![Value::None, Value::None])?;
    }
    Ok(())
}

fn ctc_set(vm: &mut Vm, what_text: &Value, method: &str, ctc: &Value, position: &str) -> PyResult<()> {
    match position {
        "nestled" => {
            vm.call_method(what_text, method, vec![ctc.clone()])?;
        }
        "nestled-close" => {
            let l = vm.types.new_list(vec![py::s(vm, "\u{feff}"), ctc.clone()]);
            vm.call_method(what_text, method, vec![l])?;
        }
        _ => {}
    }
    Ok(())
}

/// `display_say` as of 6.18–6.99.
fn display_say_6(vm: &mut Vm, s: Say) -> PyResult<()> {
    let interact = s.interact;
    if interact && !preferences(vm, "skip_unseen")? && !seen_current(vm)? && skipping(vm).as_str() == Some("fast") {
        set_skipping_none(vm)?;
    }
    if interact && skipping(vm).as_str() == Some("fast") {
        ex(vm, "with_statement", vec![Value::None], Vec::new())?;
        return Ok(());
    }
    let callback = callback_list(vm, &s.callback)?;
    let cbs = py::items(vm, &callback)?;
    let mut base_kw: Vec<(Rc<str>, Value)> = vec![("interact".into(), Value::Bool(interact)), ("type".into(), s.type_.clone())];
    base_kw.extend(py::str_items(vm, &s.cb_args)?);
    let call_all = |vm: &mut Vm, ev: &str| -> PyResult<()> {
        for c in &cbs {
            vm.call(c, vec![py::s(vm, ev)], base_kw.clone())?;
        }
        Ok(())
    };
    call_all(vm, "begin")?;
    let rfi = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    let roll_forward = if py::truthy(vm, &rfi)? { Value::Bool(false) } else { Value::None };
    let after_rollback = py::import_attr(vm, "renpy.game", "after_rollback")?;
    let (mut slow, mut all_at_once) = (s.slow, s.all_at_once);
    let skip = skipping(vm);
    if py::truthy(vm, &after_rollback)? {
        slow = false;
        all_at_once = true;
    } else if py::truthy(vm, &skip)? && (preferences(vm, "skip_unseen")? || seen_current(vm)?) {
        slow = false;
        all_at_once = true;
    }
    if !interact || preferences(vm, "self_voicing")? {
        all_at_once = true;
    }
    let what = py::string(vm, &s.what)?;
    let dtt = parse_dtt(vm, &what)?;
    let (starts, ends, delays) = if all_at_once {
        (vec![dtt.pause_start[0]], vec![clen(&dtt.text)], vec![*dtt.pause_delay.last().unwrap()])
    } else {
        (dtt.pause_start.clone(), dtt.pause_end.clone(), dtt.pause_delay.clone())
    };
    let position = s.ctc_position.as_str().unwrap_or("").to_owned();
    for (i, ((start, end), delay)) in starts.iter().zip(&ends).zip(&delays).enumerate() {
        let last_pause = i == starts.len() - 1;
        let behavior = if interact {
            let ad = cfg(vm, "say_allow_dismiss", Value::None);
            ui(vm, "saybehavior", Vec::new(), kw_of(&[("allow_dismiss", ad)]))?
        } else {
            Value::None
        };
        let mut what_ctc = if last_pause {
            s.ctc.clone()
        } else if delay.is_some() {
            if py::truthy(vm, &s.ctc_timedpause)? { s.ctc_timedpause.clone() } else { s.ctc_pause.clone() }
        } else {
            s.ctc_pause.clone()
        };
        if !(interact || s.ctc_force) {
            what_ctc = Value::None;
        }
        what_ctc = displayable_or_none(vm, &what_ctc)?;
        what_ctc = unique_duplicate(vm, &what_ctc)?;
        if *delay == Some(0.0) {
            what_ctc = Value::None;
        }
        let slow_done = new_slow_done(vm, vec![what_ctc.clone(), s.ctc_position.clone(), callback.clone(), Value::Bool(interact), s.type_.clone(), s.cb_args.clone(), opt_f(*delay)])?;
        call_all(vm, "show")?;
        let ws = vm.types.new_str(&dtt.text);
        let what_text = vm.call(&s.show_function, vec![s.who.clone(), ws], Vec::new())?;
        if !isinst(vm, &what_text, "renpy.text.text", "Text")? {
            return Err(py::error(vm, "The say screen (or show_function) must return a Text object."));
        }
        if py::truthy(vm, &what_ctc)? && position == "nestled" {
            vm.call_method(&what_text, "set_ctc", vec![what_ctc.clone()])?;
        }
        py::set(vm, &what_text, "start", Value::Int(*start as i64))?;
        py::set(vm, &what_text, "end", Value::Int(*end as i64))?;
        py::set(vm, &what_text, "slow", Value::Bool(slow))?;
        py::set(vm, &what_text, "slow_done", slow_done.clone())?;
        vm.call_method(&what_text, "update", Vec::new())?;
        call_all(vm, "show_done")?;
        if !py::is_none(&behavior) && py::truthy(vm, &s.afm)? {
            vm.call_method(&behavior, "set_text", vec![what_text.clone()])?;
        }
        if !slow {
            vm.call(&slow_done, Vec::new(), Vec::new())?;
        }
        if interact {
            let rv = ui(vm, "interact", Vec::new(), kw_of(&[("mouse", py::s(vm, "say")), ("type", s.type_.clone()), ("roll_forward", roll_forward.clone())]))?;
            if matches!(rv, Value::Bool(false)) {
                break;
            }
            if !last_pause {
                call_list(vm, "say_sustain_callbacks")?;
            }
        }
    }
    if interact {
        if !dtt.no_wait {
            if py::truthy(vm, &s.checkpoint)? {
                checkpoint(vm, Value::Bool(true), None)?;
            }
        } else {
            let g = py::import(vm, "renpy.game")?;
            py::set(vm, &g, "after_rollback", after_rollback)?;
        }
        with_none_end(vm, &s.with_none)?;
    }
    call_all(vm, "end")?;
    Ok(())
}

/// `display_say` from 7.0 (8.3 logic; newer pieces follow the config).
fn display_say_7(vm: &mut Vm, s: Say) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    let mut interact = s.interact;
    let mut advance = s.advance;
    let final_;
    if py::is_none(&s.multiple) {
        final_ = interact;
        py::set(vm, &m, "afm_text_queue", vm.types.new_list(Vec::new()))?;
    } else {
        let mp = py::items(vm, &s.multiple)?;
        let (step, total) = (py::int(vm, &mp[0])?, py::int(vm, &mp[1])?);
        if step == 1 {
            py::set(vm, &m, "afm_text_queue", vm.types.new_list(Vec::new()))?;
        }
        if step == total {
            final_ = interact;
        } else {
            final_ = false;
            interact = false;
        }
    }
    if !final_ {
        advance = false;
    }
    if final_ && !preferences(vm, "skip_unseen")? && !seen_current(vm)? && skipping(vm).as_str() == Some("fast") {
        set_skipping_none(vm)?;
    }
    let hard = has_cfg(vm, "nointeract_callbacks");
    if advance && skipping(vm).as_str() == Some("fast") {
        call_list(vm, "fast_skipping_callbacks")?;
        ex(vm, "with_statement", vec![Value::None], Vec::new())?;
        checkpoint(vm, Value::Bool(true), hard.then(|| s.checkpoint.clone()))?;
        return Ok(());
    }
    if !interact {
        call_list(vm, "nointeract_callbacks")?;
    }
    let cbs = callback_list(vm, &s.callback)?;
    let callback = new_callbacks(vm, cbs, Value::Bool(interact), s.type_.clone(), s.cb_args.clone(), s.multiple.clone())?;
    let mut roll_forward = ex(vm, "roll_forward_info", Vec::new(), Vec::new())?;
    if matches!(roll_forward, Value::Bool(true)) {
        roll_forward = Value::Bool(false);
    }
    let after_rollback = py::import_attr(vm, "renpy.game", "after_rollback")?;
    let (mut slow, mut all_at_once) = (s.slow, s.all_at_once);
    let skip = skipping(vm);
    if py::truthy(vm, &after_rollback)? {
        slow = false;
        all_at_once = true;
    } else if py::truthy(vm, &skip)? && advance && (preferences(vm, "skip_unseen")? || seen_current(vm)?) {
        slow = false;
        all_at_once = true;
    }
    if !interact || preferences(vm, "self_voicing")? {
        all_at_once = true;
    }
    let dtt_v = if py::is_none(&s.dtt) { new_dtt(vm, &s.what)? } else { s.dtt.clone() };
    let gl = |vm: &mut Vm, n: &str| -> PyResult<Vec<Value>> { py::get_items(vm, &dtt_v, n) };
    let (ps, pe, pd) = (gl(vm, "pause_start")?, gl(vm, "pause_end")?, gl(vm, "pause_delay")?);
    let (starts, ends, delays) = if all_at_once { (vec![ps[0].clone()], vec![pe.last().unwrap().clone()], vec![pd.last().unwrap().clone()]) } else { (ps, pe, pd) };
    let dtt_text = py::get(vm, &dtt_v, "text")?;
    let no_wait = py::get(vm, &dtt_v, "no_wait")?;
    let no_wait = py::truthy(vm, &no_wait)?;
    let fast = py::get_opt(vm, &dtt_v, "fast")?.unwrap_or(Value::Bool(false));
    let mut retain_tag = py::s(vm, "_retain_0");
    if s.retain {
        let mut n = 0;
        loop {
            retain_tag = vm.types.new_str(&format!("_retain_{n}"));
            let r = ex(vm, "get_screen", vec![retain_tag.clone()], Vec::new())?;
            if !py::truthy(vm, &r)? {
                break;
            }
            n += 1;
        }
    }
    py::set(vm, &callback, "what", dtt_text.clone())?;
    fire(vm, &callback, "begin")?;
    if py::truthy(vm, &fast)? {
        call_list(vm, "say_sustain_callbacks")?;
    }
    let mut ctc = s.ctc.clone();
    let position = s.ctc_position.as_str().unwrap_or("").to_owned();
    let body = (|| -> PyResult<()> {
        for (i, ((start, end), delay)) in starts.iter().zip(&ends).zip(&delays).enumerate() {
            let last_pause = i == starts.len() - 1;
            let pause_callback = vm.call_method(&callback, "copy", Vec::new())?;
            py::set(vm, &pause_callback, "start", start.clone())?;
            py::set(vm, &pause_callback, "end", end.clone())?;
            py::set(vm, &pause_callback, "delay", delay.clone())?;
            py::set(vm, &pause_callback, "last_segment", Value::Bool(last_pause))?;
            let behavior = if advance {
                let ad = cfg(vm, "say_allow_dismiss", Value::None);
                let mut kw = kw_of(&[("allow_dismiss", ad)]);
                if has_cfg(vm, "nointeract_callbacks") {
                    kw.push(("dialogue_pause".into(), delay.clone()));
                }
                ui(vm, "saybehavior", Vec::new(), kw)?
            } else {
                Value::None
            };
            let (mut what_ctc, kind) = if last_pause {
                (ctc.clone(), "last")
            } else if !py::is_none(delay) {
                (if py::truthy(vm, &s.ctc_timedpause)? { s.ctc_timedpause.clone() } else { s.ctc_pause.clone() }, "timedpause")
            } else {
                (s.ctc_pause.clone(), "pause")
            };
            let ctc_kwargs = py::dict(vm);
            py::dict_set(vm, &ctc_kwargs, "ctc_kind", py::s(vm, kind))?;
            py::dict_set(vm, &ctc_kwargs, "ctc_last", ctc.clone())?;
            py::dict_set(vm, &ctc_kwargs, "ctc_pause", s.ctc_pause.clone())?;
            py::dict_set(vm, &ctc_kwargs, "ctc_timedpause", s.ctc_timedpause.clone())?;
            if !(interact || s.ctc_force) {
                what_ctc = Value::None;
            }
            what_ctc = displayable_or_none(vm, &what_ctc)?;
            ctc = displayable_or_none(vm, &ctc)?;
            what_ctc = unique_duplicate(vm, &what_ctc)?;
            if !py::same(&ctc, &what_ctc) {
                ctc = unique_duplicate(vm, &ctc)?;
            }
            fire(vm, &pause_callback, "show")?;
            let slow_done = new_slow_done(
                vm,
                vec![what_ctc.clone(), s.ctc_position.clone(), pause_callback.clone(), Value::Bool(interact), s.type_.clone(), s.cb_args.clone(), delay.clone(), ctc_kwargs, Value::Bool(last_pause), Value::Bool(no_wait)],
            )?;
            let mut extend_text = String::new();
            if cfg_true(vm, "scry_extend")? {
                let mut scry = ex(vm, "scry", Vec::new(), Vec::new())?;
                if !py::is_none(&scry) {
                    scry = vm.call_method(&scry, "next", Vec::new())?;
                }
                let dne = py::import_attr(vm, "renpy.ast", "DoesNotExtend")?;
                let mut n = 0;
                while py::truthy(vm, &scry)? && n < 64 {
                    let et = py::get_or_none(vm, &scry, "extend_text")?;
                    if py::same(&et, &dne) {
                        break;
                    } else if !py::is_none(&et) {
                        let f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
                        if let Ok(r) = vm.call(&f, vec![et], kw_of(&[("scope", Value::None), ("force", Value::Bool(false)), ("translate", Value::Bool(true))])) {
                            if let Ok(t) = ops::getitem(vm, &r, Value::Int(0)) {
                                extend_text.push_str(&py::string(vm, &t)?);
                            }
                        }
                    }
                    scry = vm.call_method(&scry, "next", Vec::new())?;
                    n += 1;
                }
                if !extend_text.is_empty() {
                    extend_text = format!("{{done}}{extend_text}");
                }
            }
            let mut show_kw = Vec::new();
            if py::truthy(vm, &s.multiple)? {
                show_kw.push(("multiple".into(), s.multiple.clone()));
            }
            if s.retain {
                show_kw.push(("retain".into(), retain_tag.clone()));
            }
            let mut what_text = vm.call(&s.show_function, vec![s.who.clone(), dtt_text.clone()], show_kw)?;
            if matches!(what_text.payload(), Some(crysirror_python::Payload::Tuple(_))) {
                let t = py::items(vm, &what_text)?;
                if i != 0 && cfg_true(vm, "say_replace_event")? {
                    let gs = py::import_attr(vm, "renpy.display.screen", "get_screen")?;
                    let sd = vm.call(&gs, vec![t[0].clone(), t[2].clone()], Vec::new())?;
                    if !py::is_none(&sd) {
                        vm.call_method(&sd, "set_transform_event", vec![py::s(vm, "replace")])?;
                        if !s.retain && !last_pause {
                            let c = ctx(vm)?;
                            let sls = py::get(vm, &c, "scene_lists")?;
                            vm.call_method(&sls, "set_transient_prefix", vec![t[2].clone(), t[0].clone(), py::s(vm, "replaced")])?;
                        }
                    }
                }
                let gw = py::import_attr(vm, "renpy.display.screen", "get_widget")?;
                what_text = vm.call(&gw, vec![t[0].clone(), t[1].clone(), t[2].clone()], Vec::new())?;
            }
            let queue = py::get(vm, &m, "afm_text_queue")?;
            if !py::truthy(vm, &s.multiple)? {
                py::set(vm, &m, "afm_text_queue", vm.types.new_list(vec![what_text.clone()]))?;
            } else {
                vm.call_method(&queue, "append", vec![what_text.clone()])?;
            }
            let zero = matches!(delay, Value::Int(0)) || matches!(delay, Value::Float(f) if *f == 0.0);
            if zero {
                what_ctc = Value::None;
                if extend_text.is_empty() {
                    ctc = Value::None;
                }
            }
            let what_nonempty = py::truthy(vm, &dtt_text)?;
            let afm = py::truthy(vm, &s.afm)?;
            let mut slow_now = slow;
            if interact || what_nonempty || !py::is_none(&what_ctc) || (!py::is_none(&behavior) && afm) {
                if !isinst(vm, &what_text, "renpy.text.text", "Text")? {
                    return Err(py::error(vm, "The say screen (or show_function) must return a Text object."));
                }
                if py::truthy(vm, &what_ctc)? {
                    if (!extend_text.is_empty() || !last_pause) && (position == "nestled" || position == "nestled-close") {
                        let fixed = py::import_attr(vm, "store", "Fixed")?;
                        what_ctc = vm.call(&fixed, vec![what_ctc.clone()], kw_of(&[("xsize", Value::Int(0))]))?;
                    }
                    ctc_set(vm, &what_text, "set_ctc", &what_ctc, &position)?;
                }
                if (!extend_text.is_empty() || !last_pause) && py::truthy(vm, &ctc)? && py::get_opt(vm, &what_text, "set_last_ctc")?.is_some() {
                    ctc_set(vm, &what_text, "set_last_ctc", &ctc, &position)?;
                }
                let tl = py::get(vm, &what_text, "text")?;
                let t0 = ops::getitem(vm, &tl, Value::Int(0))?;
                if ops::eq(vm, &t0, &dtt_text)? {
                    if !extend_text.is_empty() {
                        let t0s = py::string(vm, &t0)?;
                        let ext = vm.types.new_str(&format!("{t0s}{extend_text}"));
                        ops::setitem(vm, &tl, Value::Int(0), ext)?;
                    }
                    py::set(vm, &what_text, "start", start.clone())?;
                    py::set(vm, &what_text, "end", end.clone())?;
                    py::set(vm, &what_text, "slow", Value::Bool(slow))?;
                    py::set(vm, &what_text, "slow_done", slow_done.clone())?;
                    vm.call_method(&what_text, "update", Vec::new())?;
                } else if cfg_true(vm, "developer")? {
                    return Err(py::error(vm, "The displayable with id 'what' was not given the exact contents of the what variable given to the say screen."));
                }
                if !py::is_none(&behavior) && afm {
                    let q = py::import_attr(vm, MODULE, "afm_text_queue")?;
                    let items = py::items(vm, &q)?;
                    vm.call_method(&behavior, "set_text", items)?;
                }
            } else {
                slow_now = false;
            }
            fire(vm, &pause_callback, "show_done")?;
            if !slow_now {
                vm.call(&slow_done, Vec::new(), Vec::new())?;
            }
            if final_ {
                let rv = ui(vm, "interact", Vec::new(), kw_of(&[("mouse", py::s(vm, "say")), ("type", s.type_.clone()), ("roll_forward", roll_forward.clone())]))?;
                if matches!(rv, Value::Bool(false)) {
                    break;
                }
                if isinst(vm, &rv, "renpy.game", "JumpException")? || isinst(vm, &rv, "renpy.game", "CallException")? {
                    return Err(PyErr::Exc(rv.obj().unwrap().clone()));
                }
                if !last_pause {
                    call_list(vm, "say_sustain_callbacks")?;
                }
            }
        }
        Ok(())
    })();
    let exception = match body {
        Ok(()) => None,
        Err(e) if is_jump_or_call(vm, &e) => Some(e),
        Err(e) => return Err(e),
    };
    if final_ {
        if !no_wait {
            match &exception {
                None => checkpoint(vm, Value::Bool(true), hard.then(|| s.checkpoint.clone()))?,
                Some(e) => {
                    let ev = super::exports::util::exc_value(e).unwrap_or(Value::None);
                    checkpoint(vm, ev, None)?;
                }
            }
        } else {
            let g = py::import(vm, "renpy.game")?;
            py::set(vm, &g, "after_rollback", after_rollback)?;
        }
        with_none_end(vm, &s.with_none)?;
    }
    fire(vm, &callback, "end")?;
    match exception {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn f_display_say(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = say_args(vm, args, kw)?;
    if e6(vm) {
        display_say_6(vm, s)?;
    } else {
        display_say_7(vm, s)?;
    }
    Ok(Value::None)
}

// ---------------------------------------------------------------------------
// ADVCharacter.

const CHAR_ATTRS: &[&str] = &["name", "who_prefix", "who_suffix", "what_prefix", "what_suffix", "show_function", "predict_function", "condition", "dynamic", "screen", "mode", "voice_tag"];
const DISPLAY_ARGS: &[&str] = &["interact", "slow", "afm", "ctc", "ctc_pause", "ctc_timedpause", "ctc_position", "all_at_once", "with_none", "callback", "type", "advance", "retain"];

fn not_set(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "NotSet")
}

fn pop(vm: &mut Vm, props: &Value, n: &str) -> PyResult<Option<Value>> {
    let k = vm.types.new_str(n);
    if ops::contains(vm, props, &k)? {
        return Ok(Some(vm.call_method(props, "pop", vec![k])?));
    }
    Ok(None)
}

fn character_prefixes(vm: &mut Vm) -> PyResult<Vec<String>> {
    let mut p: Vec<String> = ["show", "cb", "what", "window", "who"].iter().map(|s| s.to_string()).collect();
    let extra = cfg(vm, "character_id_prefixes", Value::None);
    if !py::is_none(&extra) {
        for i in py::items(vm, &extra)? {
            p.push(py::string(vm, &i)?);
        }
    }
    Ok(p)
}

fn defaultdict_dict(vm: &mut Vm) -> PyResult<Value> {
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let d = Value::Obj(vm.types.dict.clone());
    vm.call(&dd, vec![d], Vec::new())
}

/// The 7.x split of character properties into the prefixed dicts.
fn split_into(vm: &mut Vm, properties: &Value, props: &Value) -> PyResult<()> {
    let prefixes = character_prefixes(vm)?;
    let mut sargs: Vec<Value> = prefixes.iter().map(|p| vm.types.new_str(&format!("{p}_"))).collect();
    sargs.push(py::s(vm, ""));
    let sp = py::import_attr(vm, "renpy.easy", "split_properties")?;
    let mut a = vec![properties.clone()];
    a.extend(sargs);
    let split = vm.call(&sp, a, Vec::new())?;
    let split = py::items(vm, &split)?;
    for (p, d) in prefixes.iter().zip(&split) {
        let target = ops::getitem(vm, props, vm.types.new_str(p))?;
        vm.call_method(&target, "update", vec![d.clone()])?;
    }
    let who = ops::getitem(vm, props, py::s(vm, "who"))?;
    vm.call_method(&who, "update", vec![split.last().unwrap().clone()])?;
    Ok(())
}

fn c_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ns = not_set(vm)?;
    let name = args.get(1).cloned().or_else(|| py::kw(kw, "name").cloned()).unwrap_or_else(|| ns.clone());
    let mut kind = args.get(2).cloned().or_else(|| py::kw(kw, "kind").cloned()).unwrap_or(Value::None);
    let properties = py::dict(vm);
    for (k, v) in kw {
        if &**k == "name" || &**k == "kind" {
            continue;
        }
        py::dict_set(vm, &properties, k, v.clone())?;
    }
    if py::is_none(&kind) {
        kind = store_get(vm, "adv")?;
    }
    if !py::same(&name, &ns) {
        py::dict_set(vm, &properties, "name", name)?;
    }
    let has_kind = py::truthy(vm, &kind)?;
    for n in CHAR_ATTRS {
        let v = match pop(vm, &properties, n)? {
            Some(v) => v,
            None => py::get(vm, &kind, n)?,
        };
        py::set(vm, this, n, v)?;
    }
    let image_tag = if cfg_true(vm, "new_character_image_argument")? {
        match pop(vm, &properties, "image")? {
            Some(v) => v,
            None => py::get_or_none(vm, &kind, "image_tag")?,
        }
    } else {
        Value::None
    };
    py::set(vm, this, "image_tag", image_tag)?;
    let da = py::dict(vm);
    let kind_da = if has_kind { py::get(vm, &kind, "display_args")? } else { Value::None };
    let old = e6(vm);
    for n in DISPLAY_ARGS {
        if old && (*n == "advance" || *n == "retain") {
            continue;
        }
        let mut v = match pop(vm, &properties, n)? {
            Some(v) => v,
            None if !py::is_none(&kind_da) => {
                let k = vm.types.new_str(n);
                if ops::contains(vm, &kind_da, &k)? {
                    ops::getitem(vm, &kind_da, k)?
                } else if *n == "advance" {
                    Value::Bool(true)
                } else if *n == "retain" {
                    Value::Bool(false)
                } else {
                    return Err(vm.exc_msg("KeyError", format!("'{n}'")));
                }
            }
            None if *n == "advance" => Value::Bool(true),
            None if *n == "retain" => Value::Bool(false),
            None => return Err(vm.attr_error("'bool' object has no attribute 'display_args'")),
        };
        if n.starts_with("ctc") && *n != "ctc_position" {
            v = displayable_or_none(vm, &v)?;
        }
        py::dict_set(vm, &da, n, v)?;
    }
    py::set(vm, this, "display_args", da)?;
    if !old {
        let sn = pop(vm, &properties, "statement_name")?.unwrap_or(Value::None);
        py::set(vm, this, "_statement_name", sn)?;
        let props = defaultdict_dict(vm)?;
        if has_kind {
            let kp = py::get_opt(vm, &kind, "properties")?.unwrap_or_else(|| py::dict(vm));
            for (k, v) in ops::mapping_items(vm, &kp)? {
                let d = Value::Obj(vm.types.dict.clone());
                let c = vm.call(&d, vec![v], Vec::new())?;
                ops::setitem(vm, &props, k, c)?;
            }
        }
        py::set(vm, this, "properties", props)?;
    }
    for n in ["who_args", "what_args", "window_args", "show_args", "cb_args"] {
        let v = if has_kind {
            let k = py::get(vm, &kind, n)?;
            dict_copy(vm, &k)?
        } else if n == "who_args" || n == "what_args" {
            let d = py::dict(vm);
            py::dict_set(vm, &d, "substitute", Value::Bool(false))?;
            d
        } else {
            py::dict(vm)
        };
        py::set(vm, this, n, v)?;
    }
    if !cfg_true(vm, "new_character_image_argument")? {
        if let Some(img) = pop(vm, &properties, "image")? {
            let sa = py::get(vm, this, "show_args")?;
            py::dict_set(vm, &sa, "image", img)?;
        }
    }
    if let Some(sa) = pop(vm, &properties, "slow_abortable")? {
        let wa = py::get(vm, this, "what_args")?;
        py::dict_set(vm, &wa, "slow_abortable", sa)?;
    }
    if old {
        for (k, v) in py::str_items(vm, &properties)? {
            let target = match k.split_once('_') {
                Some(("show", s)) => Some(("show_args", s)),
                Some(("cb", s)) => Some(("cb_args", s)),
                Some(("what", s)) => Some(("what_args", s)),
                Some(("window", s)) => Some(("window_args", s)),
                Some(("who", s)) => Some(("who_args", s)),
                _ => None,
            };
            match target {
                Some((attr, s)) => {
                    let d = py::get(vm, this, attr)?;
                    py::dict_set(vm, &d, s, v)?;
                }
                None => {
                    let d = py::get(vm, this, "who_args")?;
                    py::dict_set(vm, &d, &k, v)?;
                }
            }
        }
        return Ok(Value::None);
    }
    let props = py::get(vm, this, "properties")?;
    split_into(vm, &properties, &props)?;
    for (p, attr) in [("show", "show_args"), ("cb", "cb_args"), ("what", "what_args"), ("window", "window_args"), ("who", "who_args")] {
        let d = vm.call_method(&props, "pop", vec![py::s(vm, p)])?;
        let t = py::get(vm, this, attr)?;
        vm.call_method(&t, "update", vec![d])?;
    }
    Ok(Value::None)
}

fn c_copy(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ns = not_set(vm)?;
    let name = args.get(1).cloned().or_else(|| py::kw(kw, "name").cloned()).unwrap_or(ns);
    let ty = Value::Obj(ops::type_of(vm, this));
    let mut k: Vec<(Rc<str>, Value)> = kw.iter().filter(|(k, _)| &**k != "name").cloned().collect();
    k.push(("kind".into(), this.clone()));
    vm.call(&ty, vec![name], k)
}

fn c_do_add(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `get_show_properties(extra_properties)` (8.1).
fn show_properties(vm: &mut Vm, this: &Value, extra: &Value) -> PyResult<(Value, Value, Value, Value, Value, Value)> {
    let g = |vm: &mut Vm, n: &str| py::get(vm, this, n);
    let mut screen = g(vm, "screen")?;
    let (mut show, mut who, mut what, mut window) = (g(vm, "show_args")?, g(vm, "who_args")?, g(vm, "what_args")?, g(vm, "window_args")?);
    let mut properties = py::get_opt(vm, this, "properties")?.unwrap_or_else(|| py::dict(vm));
    if py::truthy(vm, extra)? {
        let extra = dict_copy(vm, extra)?;
        if let Some(s) = pop(vm, &extra, "screen")? {
            screen = s;
        }
        show = dict_copy(vm, &show)?;
        who = dict_copy(vm, &who)?;
        what = dict_copy(vm, &what)?;
        window = dict_copy(vm, &window)?;
        let np = defaultdict_dict(vm)?;
        for (k, v) in ops::mapping_items(vm, &properties)? {
            let c = dict_copy(vm, &v)?;
            ops::setitem(vm, &np, k, c)?;
        }
        properties = np;
        split_into(vm, &extra, &properties)?;
        for (p, d) in [("show", &show), ("who", &who), ("what", &what), ("window", &window)] {
            let v = vm.call_method(&properties, "pop", vec![py::s(vm, p)])?;
            vm.call_method(d, "update", vec![v])?;
        }
    }
    Ok((screen, show, who, what, window, properties))
}

fn c_get_show_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b, c, d, e, f) = show_properties(vm, &args[0], &args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(vm.types.new_tuple(vec![a, b, c, d, e, f]))
}

fn c_do_show(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (who, what) = (args[1].clone(), args[2].clone());
    let f = py::get(vm, this, "show_function")?;
    if e6(vm) {
        let mut k: Vec<(Rc<str>, Value)> = vec![
            ("who_args".into(), py::get(vm, this, "who_args")?),
            ("what_args".into(), py::get(vm, this, "what_args")?),
            ("window_args".into(), py::get(vm, this, "window_args")?),
            ("screen".into(), py::get(vm, this, "screen")?),
        ];
        let sa = py::get(vm, this, "show_args")?;
        k.extend(py::str_items(vm, &sa)?);
        return vm.call(&f, vec![who, what], k);
    }
    let multiple = arg_or(&args[3..], kw, 0, "multiple", Value::None);
    let extra = arg_or(&args[3..], kw, 1, "extra_properties", Value::None);
    let retain = arg_or(&args[3..], kw, 2, "retain", Value::None);
    let (screen, show, who_a, what_a, window_a, properties) = show_properties(vm, this, &extra)?;
    let mut k: Vec<(Rc<str>, Value)> = vec![
        ("who_args".into(), who_a),
        ("what_args".into(), what_a),
        ("window_args".into(), window_a),
        ("screen".into(), screen),
        ("properties".into(), properties),
    ];
    let mut sa = py::str_items(vm, &show)?;
    if !py::is_none(&multiple) {
        sa.retain(|(n, _)| &**n != "multiple");
        sa.push(("multiple".into(), multiple));
    }
    if py::truthy(vm, &retain)? {
        sa.retain(|(n, _)| &**n != "retain");
        sa.push(("retain".into(), retain));
    }
    k.extend(sa);
    vm.call(&f, vec![who, what], k)
}

fn c_do_done(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut k = Vec::new();
    if let Some(m) = args.get(3).cloned().or_else(|| py::kw(kw, "multiple").cloned()) {
        if !e6(vm) {
            k.push(("multiple".into(), m));
        }
    }
    py::method_kw(vm, &args[0], "add_history", vec![py::s(vm, "adv"), args[1].clone(), args[2].clone()], k)?;
    Ok(Value::None)
}

fn c_do_extend(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    vm.call_method(&args[0], "pop_history", Vec::new())?;
    Ok(Value::None)
}

fn c_do_display(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let show = py::get(vm, &args[0], "do_show")?;
    let ds = py::import_attr(vm, MODULE, "display_say")?;
    vm.call(&ds, vec![args[1].clone(), args[2].clone(), show], kw.to_vec())
}

fn c_do_predict(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = py::get(vm, this, "predict_function")?;
    let extra = arg_or(&args[3..], kw, 0, "extra_properties", Value::None);
    let (screen, show, who_a, what_a, window_a, properties) = show_properties(vm, this, &extra)?;
    let mut k: Vec<(Rc<str>, Value)> = vec![("who_args".into(), who_a), ("what_args".into(), what_a), ("window_args".into(), window_a), ("screen".into(), screen)];
    if !e6(vm) {
        k.push(("properties".into(), properties));
    }
    k.extend(py::str_items(vm, &show)?);
    vm.call(&f, vec![args[1].clone(), args[2].clone()], k)
}

fn images(vm: &mut Vm) -> PyResult<Value> {
    let c = ctx(vm)?;
    py::get(vm, &c, "images")
}

fn tuple_with(vm: &mut Vm, first: &[Value], rest: &Value) -> PyResult<Value> {
    let mut v = first.to_vec();
    if !py::is_none(rest) {
        v.extend(py::items(vm, rest)?);
    }
    Ok(vm.types.new_tuple(v))
}

/// 6.x `resolve_say_attributes(predict, wanted=[], remove=[])`.
fn resolve_6(vm: &mut Vm, this: &Value, predict: bool, wanted: &Value, remove: &Value) -> PyResult<()> {
    let attrs = ex(vm, "get_say_attributes", Vec::new(), Vec::new())?;
    if !(py::truthy(vm, &attrs)? || py::truthy(vm, wanted)? || py::truthy(vm, remove)?) {
        return Ok(());
    }
    let tag = py::get(vm, this, "image_tag")?;
    if !py::truthy(vm, &tag)? {
        if py::truthy(vm, &attrs)? && !predict {
            let r = ops::repr(vm, &attrs)?;
            return Err(py::error(vm, format!("Say has image attributes {r}, but there's no image tag associated with the speaking character.")));
        }
        return Ok(());
    }
    let tagged = tuple_with(vm, &[tag.clone()], &attrs)?;
    let imgs = images(vm)?;
    let tl = cfg(vm, "tag_layer", Value::None);
    let master = py::s(vm, "master");
    let layer = vm.call_method(&tl, "get", vec![tag.clone(), master.clone()])?;
    let t1 = vm.types.new_tuple(vec![tag.clone()]);
    let showing = vm.call_method(&imgs, "showing", vec![layer.clone(), t1])?;
    if py::truthy(vm, &showing)? {
        let mut new_image = vm.call_method(&imgs, "apply_attributes", vec![layer.clone(), tag.clone(), tagged.clone(), wanted.clone(), remove.clone()])?;
        if py::is_none(&new_image) {
            new_image = tagged;
        }
        let exact = py::method_kw(vm, &imgs, "showing", vec![layer, new_image.clone()], kw_of(&[("exact", Value::Bool(true))]))?;
        if py::truthy(vm, &exact)? {
            return Ok(());
        }
        if predict {
            vm.call_method(&imgs, "predict_show", vec![new_image])?;
        } else {
            let trans = cfg(vm, "say_attribute_transition", Value::None);
            if !py::is_none(&trans) {
                ex(vm, "with_statement", vec![Value::None], Vec::new())?;
            }
            ex(vm, "show", vec![new_image], Vec::new())?;
            if !py::is_none(&trans) {
                ex(vm, "with_statement", vec![trans], Vec::new())?;
            }
        }
    } else {
        py::method_kw(vm, &imgs, "predict_show", vec![master, tagged], kw_of(&[("show", Value::Bool(false))]))?;
    }
    Ok(())
}

/// 7.x/8.x `resolve_say_attributes(predict, attrs)`: True when shown.
fn resolve_7(vm: &mut Vm, this: &Value, predict: bool, attrs: &Value) -> PyResult<bool> {
    if !py::truthy(vm, attrs)? {
        return Ok(false);
    }
    let tag = py::get(vm, this, "image_tag")?;
    if !py::truthy(vm, &tag)? {
        if !predict {
            let r = ops::repr(vm, attrs)?;
            return Err(py::error(vm, format!("Say has image attributes {r}, but there's no image tag associated with the speaking character.")));
        }
        return Ok(false);
    }
    let attrs_t = tuple_with(vm, &[], attrs)?;
    let tagged = tuple_with(vm, &[tag.clone()], &attrs_t)?;
    let imgs = images(vm)?;
    let layer = ex(vm, "default_layer", vec![Value::None, tag.clone()], Vec::new())?;
    let t1 = vm.types.new_tuple(vec![tag.clone()]);
    let showing = vm.call_method(&imgs, "showing", vec![layer.clone(), t1])?;
    if py::truthy(vm, &showing)? {
        let mut new_image = vm.call_method(&imgs, "apply_attributes", vec![layer.clone(), tag.clone(), tagged.clone()])?;
        if py::is_none(&new_image) {
            new_image = tagged;
        }
        let exact = py::method_kw(vm, &imgs, "showing", vec![layer, new_image.clone()], kw_of(&[("exact", Value::Bool(true))]))?;
        if py::truthy(vm, &exact)? {
            return Ok(false);
        }
        let show_image = tuple_with(vm, &[tag], &attrs_t)?;
        if predict {
            ex(vm, "predict_show", vec![new_image], Vec::new())?;
        } else {
            ex(vm, "show", vec![show_image], Vec::new())?;
            return Ok(true);
        }
    } else if cfg_true(vm, "say_attributes_use_side_image")? {
        let spt = cfg(vm, "side_image_prefix_tag", py::s(vm, "side"));
        let mut parts = vec![spt];
        parts.extend(py::items(vm, &tagged)?);
        let tagged2 = vm.types.new_tuple(parts);
        let mut new_image = vm.call_method(&imgs, "apply_attributes", vec![layer.clone(), tag.clone(), tagged2.clone()])?;
        if py::is_none(&new_image) {
            new_image = tagged2;
        }
        let items = py::items(vm, &new_image)?;
        let rest = vm.types.new_tuple(items[1..].to_vec());
        py::method_kw(vm, &imgs, "predict_show", vec![layer, rest], kw_of(&[("show", Value::Bool(false))]))?;
    } else {
        py::method_kw(vm, &imgs, "predict_show", vec![layer, tagged], kw_of(&[("show", Value::Bool(false))]))?;
    }
    Ok(false)
}

fn c_resolve_say_attributes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let predict = py::truthy(vm, &args[1])?;
    if e6(vm) {
        let wanted = arg_or(&args[2..], kw, 0, "wanted", vm.types.new_list(Vec::new()));
        let remove = arg_or(&args[2..], kw, 1, "remove", vm.types.new_list(Vec::new()));
        resolve_6(vm, &args[0], predict, &wanted, &remove)?;
        return Ok(Value::None);
    }
    let attrs = arg_or(&args[2..], kw, 0, "attrs", Value::None);
    Ok(if resolve_7(vm, &args[0], predict, &attrs)? { Value::Bool(true) } else { Value::None })
}

fn resolve_call(vm: &mut Vm, this: &Value, predict: bool, attrs: &Value) -> PyResult<bool> {
    let r = vm.call_method(this, "resolve_say_attributes", vec![Value::Bool(predict), attrs.clone()])?;
    py::truthy(vm, &r)
}

fn get_attributes(vm: &mut Vm, layer: Value, tag: &Value) -> PyResult<Value> {
    let imgs = images(vm)?;
    vm.call_method(&imgs, "get_attributes", vec![layer, tag.clone()])
}

/// `handle_say_attributes(predicting, interact)` (7.3+).
fn c_handle_say_attributes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let predicting = py::truthy(vm, &args[1])?;
    let interact = py::truthy(vm, &args[2])?;
    let c = ctx(vm)?;
    let attrs = py::get_or_none(vm, &c, "say_attributes")?;
    py::set(vm, &c, "say_attributes", Value::None)?;
    let mut temporary = py::get_or_none(vm, &c, "temporary_attributes")?;
    py::set(vm, &c, "temporary_attributes", Value::None)?;
    if interact {
        let mut t = if py::truthy(vm, &temporary)? { py::items(vm, &temporary)? } else { Vec::new() };
        let sa = cfg(vm, "speaking_attribute", Value::None);
        if !py::is_none(&sa) {
            t.insert(0, sa);
        }
        temporary = vm.types.new_list(t);
    }
    let tag = py::get(vm, this, "image_tag")?;
    let before = get_attributes(vm, Value::None, &tag)?;
    let mut mode: Option<&str> = None;
    if resolve_call(vm, this, predicting, &attrs)? {
        mode = Some("permanent");
    }
    if !py::truthy(vm, &tag)? {
        return Ok(Value::None);
    }
    let mut saved = Value::None;
    if py::truthy(vm, &temporary)? {
        saved = get_attributes(vm, Value::None, &tag)?;
        if resolve_call(vm, this, predicting, &temporary)? {
            mode = Some(if mode.is_some() { "both" } else { "temporary" });
        }
    }
    if let Some(m) = mode {
        let after = get_attributes(vm, Value::None, &tag)?;
        vm.call_method(this, "handle_say_transition", vec![py::s(vm, m), before, after])?;
    }
    if py::truthy(vm, &temporary)? {
        let imgs = images(vm)?;
        return Ok(vm.types.new_tuple(vec![saved, imgs]));
    }
    Ok(Value::None)
}

fn c_handle_say_transition(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let set_c = Value::Obj(vm.types.set.clone());
    let before = vm.call(&set_c, vec![args[2].clone()], Vec::new())?;
    let after = vm.call(&set_c, vec![args[3].clone()], Vec::new())?;
    if ops::eq(vm, &before, &after)? {
        return Ok(Value::None);
    }
    let cb = cfg(vm, "say_attribute_transition_callback", Value::None);
    if py::is_none(&cb) {
        return Ok(Value::None);
    }
    let tag = py::get(vm, &args[0], "image_tag")?;
    let mut a = vec![tag, args[1].clone()];
    if cfg_true(vm, "say_attribute_transition_callback_attrs")? {
        a.push(before);
        a.push(after);
    }
    let r = vm.call(&cb, a, Vec::new())?;
    let r = py::items(vm, &r)?;
    let (trans, layer) = (r[0].clone(), r.get(1).cloned().unwrap_or(Value::None));
    if !py::is_none(&trans) {
        if py::is_none(&layer) {
            ex(vm, "with_statement", vec![trans], Vec::new())?;
        } else {
            ex(vm, "transition", vec![trans], kw_of(&[("layer", layer)]))?;
        }
    }
    Ok(Value::None)
}

fn c_restore_say_attributes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, predicting, state) = (&args[0], py::truthy(vm, &args[1])?, &args[2]);
    if py::is_none(state) {
        return Ok(Value::None);
    }
    let st = py::items(vm, state)?;
    let (attrs, imgs) = (st[0].clone(), st[1].clone());
    let tag = py::get(vm, this, "image_tag")?;
    if !py::truthy(vm, &tag)? {
        return Ok(Value::None);
    }
    let cur_imgs = images(vm)?;
    if !py::same(&imgs, &cur_imgs) {
        return Ok(Value::None);
    }
    let current = get_attributes(vm, Value::None, &tag)?;
    if ops::eq(vm, &attrs, &current)? {
        return Ok(Value::None);
    }
    let mut parts = vec![tag.clone()];
    let attr_items = py::items(vm, &attrs)?;
    parts.extend(attr_items.iter().cloned());
    for i in py::items(vm, &current)? {
        let mut found = false;
        for a in &attr_items {
            if ops::eq(vm, a, &i)? {
                found = true;
                break;
            }
        }
        if !found {
            let s = py::string(vm, &i)?;
            parts.push(vm.types.new_str(&format!("-{s}")));
        }
    }
    let with_attrs = vm.types.new_tuple(parts);
    let t1 = vm.types.new_tuple(vec![tag]);
    let showing = vm.call_method(&imgs, "showing", vec![Value::None, t1])?;
    if py::truthy(vm, &showing)? {
        if !predicting {
            ex(vm, "show", vec![with_attrs], Vec::new())?;
            return Ok(Value::Bool(true));
        }
        ex(vm, "predict_show", vec![with_attrs], Vec::new())?;
    } else {
        py::method_kw(vm, &imgs, "predict_show", vec![Value::None, with_attrs], kw_of(&[("show", Value::Bool(false))]))?;
    }
    Ok(Value::None)
}

fn who_name(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let who = py::get(vm, this, "name")?;
    let dynamic = py::get(vm, this, "dynamic")?;
    if py::truthy(vm, &dynamic)? {
        if !e6(vm) && crysirror_python::builtins::is_callable(vm, &who) {
            return vm.call(&who, Vec::new(), Vec::new());
        }
        return py_eval(vm, &who);
    }
    Ok(who)
}

fn c_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let who = who_name(vm, &args[0])?;
    substitute(vm, &who, true)
}

fn c_format(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = c_str(vm, &args[..1], &[])?;
    let f = py::import_attr(vm, "builtins", "format")?;
    vm.call(&f, vec![s, args[1].clone()], Vec::new())
}

fn c_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get(vm, &args[0], "name")?;
    let r = ops::repr(vm, &n)?;
    Ok(vm.types.new_str(&format!("<Character: {r}>")))
}

fn c_empty_window(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let empty = py::s(vm, "");
    let base = kw_of(&[("interact", Value::Bool(false)), ("_call_done", Value::Bool(false))]);
    if e6(vm) {
        vm.call(this, vec![empty], base)?;
        return Ok(Value::None);
    }
    let multiple = arg_or(&args[1..], kw, 0, "multiple", Value::None);
    let name = py::get(vm, this, "name")?;
    let wp = py::get(vm, this, "what_prefix")?;
    let ws = py::get(vm, this, "what_suffix")?;
    if cfg_true(vm, "fast_empty_window")? && py::is_none(&name) && !(py::truthy(vm, &wp)? || py::truthy(vm, &ws)?) && py::is_none(&multiple) {
        vm.call_method(this, "do_show", vec![Value::None, empty])?;
        return Ok(Value::None);
    }
    if py::truthy(vm, &multiple)? {
        let n = py::int(vm, &multiple)?;
        for _ in 0..n {
            let mut k = base.clone();
            k.push(("multiple".into(), multiple.clone()));
            vm.call(this, vec![empty.clone()], k)?;
        }
    } else {
        vm.call(this, vec![empty], base)?;
    }
    Ok(Value::None)
}

fn c_has_character_arguments(_: &mut Vm, _: &[Value], kw: Kw) -> PyResult<Value> {
    let safe = ["interact", "_mode", "_call_done", "multiple", "_with_none"];
    Ok(Value::Bool(kw.iter().all(|(k, _)| safe.contains(&&**k))))
}

fn prefix_suffix(vm: &mut Vm, thing: &str, prefix: &Value, body: &Value, suffix: &Value, translate: bool) -> PyResult<Value> {
    let sub = |vm: &mut Vm, s: &str| -> PyResult<String> {
        let v = vm.types.new_str(s);
        let r = substitute(vm, &v, true)?;
        py::string(vm, &r)
    };
    let (p, b, s) = (py::string(vm, prefix)?, py::string(vm, body)?, py::string(vm, suffix)?);
    if !cfg_true(vm, "new_substitutions")? {
        return Ok(vm.types.new_str(&format!("{p}{b}{s}")));
    }
    let compat = cfg(vm, "who_what_sub_compat", Value::Int(0));
    let compat = py::int(vm, &compat).unwrap_or(0);
    let quoted = format!("[[{thing}]");
    let var = format!("[{thing}]");
    let body_sub = {
        let v = vm.types.new_str(&b);
        let r = substitute(vm, &v, translate)?;
        py::string(vm, &r)?
    };
    let rv = match compat {
        0 => sub(vm, &format!("{p}{quoted}{s}"))?.replace(&var, &body_sub),
        1 => {
            let (sp, ss) = (sub(vm, &p)?, sub(vm, &s)?);
            sub(vm, &format!("{sp}{quoted}{ss}"))?.replace(&var, &body_sub)
        }
        _ => format!("{}{}{}", sub(vm, &p)?, body_sub, sub(vm, &s)?),
    };
    Ok(vm.types.new_str(&rv))
}

fn c_prefix_suffix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let thing = py::string(vm, &args[1])?;
    prefix_suffix(vm, &thing, &args[2], &args[3], &args[4], true)
}

fn condition_false(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let c = py::get(vm, this, "condition")?;
    if py::is_none(&c) {
        return Ok(false);
    }
    let r = py_eval(vm, &c)?;
    Ok(!py::truthy(vm, &r)?)
}

fn log_line(vm: &mut Vm, s: Value) -> PyResult<()> {
    ex(vm, "log", vec![s], Vec::new())?;
    Ok(())
}

/// 6.x `__call__(what, interact=True, _call_done=True, **kwargs)`.
fn call_6(vm: &mut Vm, this: &Value, what: &Value, interact: bool, call_done: bool, kw: &[(Rc<str>, Value)]) -> PyResult<Value> {
    if condition_false(vm, this)? {
        return Ok(Value::Bool(true));
    }
    if !is_str(what) {
        let r = ops::repr(vm, what)?;
        return Err(py::error(vm, format!("Character expects its what argument to be a string, got {r}.")));
    }
    let sa = cfg(vm, "speaking_attribute", Value::None);
    let speaking = if interact && !py::is_none(&sa) { vm.types.new_list(vec![sa]) } else { vm.types.new_list(Vec::new()) };
    let empty = vm.types.new_list(Vec::new());
    py::method_kw(vm, this, "resolve_say_attributes", vec![Value::Bool(false)], kw_of(&[("wanted", speaking.clone())]))?;
    let old_side = store_get(vm, "_side_image_attributes")?;
    let tag = py::get(vm, this, "image_tag")?;
    let attrs = if py::truthy(vm, &tag)? {
        let a = get_attributes(vm, py::s(vm, "master"), &tag)?;
        tuple_with(vm, &[tag.clone()], &a)?
    } else {
        Value::None
    };
    store_set(vm, "_side_image_attributes", attrs)?;
    let vtc = cfg(vm, "voice_tag_callback", Value::None);
    if !py::is_none(&vtc) {
        let vt = py::get(vm, this, "voice_tag")?;
        vm.call(&vtc, vec![vt], Vec::new())?;
    }
    let r = (|| -> PyResult<()> {
        if interact {
            let mode = py::get(vm, this, "mode")?;
            ex(vm, "mode", vec![mode], Vec::new())?;
        }
        let da = py::get(vm, this, "display_args")?;
        let da = dict_copy(vm, &da)?;
        for (k, v) in kw {
            py::dict_set(vm, &da, k, v.clone())?;
        }
        let di = dict_get(vm, &da, "interact")?;
        let di = py::truthy(vm, &di)? && interact;
        py::dict_set(vm, &da, "interact", Value::Bool(di))?;
        let mut who = who_name(vm, this)?;
        if !py::is_none(&who) {
            let (p, s) = (py::get(vm, this, "who_prefix")?, py::get(vm, this, "who_suffix")?);
            who = prefix_suffix(vm, "who", &p, &who, &s, true)?;
        }
        let c = ctx(vm)?;
        let tbl = py::get_or_none(vm, &c, "translate_block_language")?;
        let (p, s) = (py::get(vm, this, "what_prefix")?, py::get(vm, this, "what_suffix")?);
        let what = prefix_suffix(vm, "what", &p, what, &s, py::is_none(&tbl))?;
        vm.call_method(this, "do_add", vec![who.clone(), what.clone()])?;
        let mut k: Vec<(Rc<str>, Value)> = vec![("cb_args".into(), py::get(vm, this, "cb_args")?)];
        k.extend(py::str_items(vm, &da)?);
        py::method_kw(vm, this, "do_display", vec![who.clone(), what.clone()], k)?;
        if call_done {
            vm.call_method(this, "do_done", vec![who.clone(), what.clone()])?;
            if py::truthy(vm, &who)? && is_str(&who) {
                log_line(vm, who)?;
            }
            log_line(vm, what)?;
            log_line(vm, py::s(vm, ""))?;
        }
        Ok(())
    })();
    if interact {
        store_set(vm, "_side_image_attributes", old_side)?;
        py::method_kw(vm, this, "resolve_say_attributes", vec![Value::Bool(false)], kw_of(&[("remove", speaking)]))?;
    }
    let _ = empty;
    r?;
    Ok(Value::None)
}

/// 7.x/8.x `__call__`.
fn call_7(vm: &mut Vm, this: &Value, what: &Value, interact: bool, call_done: bool, multiple: Value, kw: &[(Rc<str>, Value)]) -> PyResult<Value> {
    let mut kw = kw.to_vec();
    let take = |kw: &mut Vec<(Rc<str>, Value)>, n: &str| -> Value {
        match kw.iter().position(|(k, _)| &**k == n) {
            Some(i) => kw.remove(i).1,
            None => Value::None,
        }
    };
    let mode_kw = take(&mut kw, "_mode");
    let with_none = take(&mut kw, "_with_none");
    if !kw.is_empty() {
        let ch = py::import_attr(vm, MODULE, "Character")?;
        let mut k = kw.clone();
        k.push(("kind".into(), this.clone()));
        let c = vm.call(&ch, Vec::new(), k)?;
        return vm.call(&c, vec![what.clone()], kw_of(&[("interact", Value::Bool(interact)), ("_call_done", Value::Bool(call_done)), ("multiple", multiple), ("_mode", mode_kw), ("_with_none", with_none)]));
    }
    if condition_false(vm, this)? {
        return Ok(Value::Bool(true));
    }
    if !is_str(what) {
        let r = ops::repr(vm, what)?;
        return Err(py::error(vm, format!("Character expects its what argument to be a string, got {r}.")));
    }
    if store_get(vm, "_side_image_attributes_reset").and_then(|v| py::truthy(vm, &v)).unwrap_or(false) {
        store_set(vm, "_side_image_attributes", Value::None)?;
        store_set(vm, "_side_image_attributes_reset", Value::Bool(false))?;
    }
    let m = py::import(vm, MODULE)?;
    let mut multiple = multiple;
    if py::is_none(&multiple) {
        py::set(vm, &m, "multiple_count", Value::Int(0))?;
    } else {
        let mc = py::get(vm, &m, "multiple_count")?;
        let n = py::int(vm, &mc)? + 1;
        let total = py::int(vm, &multiple)?;
        multiple = vm.types.new_tuple(vec![Value::Int(n), Value::Int(total)]);
        py::set(vm, &m, "multiple_count", Value::Int(if n == total { 0 } else { n }))?;
    }
    let has_handle = py::get_opt(vm, this, "handle_say_attributes")?.is_some();
    let old_attr_state = if has_handle { vm.call_method(this, "handle_say_attributes", vec![Value::Bool(false), Value::Bool(interact)])? } else { Value::None };
    let mut old_side = Value::None;
    let tag = py::get(vm, this, "image_tag")?;
    if py::is_none(&multiple) {
        old_side = store_get(vm, "_side_image_attributes")?;
        let attrs = if py::truthy(vm, &tag)? {
            let a = get_attributes(vm, Value::None, &tag)?;
            tuple_with(vm, &[tag.clone()], &a)?
        } else {
            Value::None
        };
        store_set(vm, "_side_image_attributes", attrs)?;
        if !interact && has_cfg(vm, "nointeract_callbacks") {
            store_set(vm, "_side_image_attributes_reset", Value::Bool(true))?;
        }
    }
    let vtc = cfg(vm, "voice_tag_callback", Value::None);
    if !py::is_none(&vtc) {
        let vt = py::get(vm, this, "voice_tag")?;
        vm.call(&vtc, vec![vt], Vec::new())?;
    }
    let r = (|| -> PyResult<()> {
        let c = ctx(vm)?;
        if interact {
            let mode = if py::truthy(vm, &mode_kw)? { mode_kw.clone() } else { py::get(vm, this, "mode")? };
            ex(vm, "mode", vec![mode], Vec::new())?;
        } else if py::get_opt(vm, &c, "deferred_translate_identifier")?.is_some() {
            let ti = py::get_or_none(vm, &c, "translate_identifier")?;
            py::set(vm, &c, "deferred_translate_identifier", ti)?;
        }
        let da = py::get(vm, this, "display_args")?;
        let da = dict_copy(vm, &da)?;
        let di = dict_get(vm, &da, "interact")?;
        let di = py::truthy(vm, &di)? && interact;
        py::dict_set(vm, &da, "interact", Value::Bool(di))?;
        if !py::is_none(&multiple) {
            py::dict_set(vm, &da, "multiple", multiple.clone())?;
        }
        if !py::is_none(&with_none) {
            py::dict_set(vm, &da, "with_none", with_none.clone())?;
        }
        let mut who = who_name(vm, this)?;
        if !py::is_none(&who) {
            let (p, s) = (py::get(vm, this, "who_prefix")?, py::get(vm, this, "who_suffix")?);
            who = prefix_suffix(vm, "who", &p, &who, &s, true)?;
        }
        let (p, s) = (py::get(vm, this, "what_prefix")?, py::get(vm, this, "what_suffix")?);
        let what = prefix_suffix(vm, "what", &p, what, &s, true)?;
        let mk = if py::is_none(&multiple) { Vec::new() } else { vec![("multiple".into(), multiple.clone())] };
        py::method_kw(vm, this, "do_add", vec![who.clone(), what.clone()], mk.clone())?;
        let dtt = new_dtt(vm, &what)?;
        let hcd = cfg_true(vm, "history_current_dialogue")?;
        if hcd {
            py::method_kw(vm, this, "add_history", vec![py::s(vm, "current"), who.clone(), what.clone()], kw_of(&[("multiple", multiple.clone())]))?;
        }
        let mut k: Vec<(Rc<str>, Value)> = vec![("cb_args".into(), py::get(vm, this, "cb_args")?)];
        if at_least(vm, 7, 4) {
            k.push(("dtt".into(), dtt.clone()));
        }
        for (kk, vv) in py::str_items(vm, &da)? {
            if e6(vm) || (&*kk != "retain" || at_least(vm, 8, 2)) {
                k.push((kk, vv));
            }
        }
        py::method_kw(vm, this, "do_display", vec![who.clone(), what.clone()], k)?;
        if hcd {
            vm.call_method(this, "pop_history", Vec::new())?;
        }
        let has_done = py::get_opt(vm, &dtt, "has_done")?.unwrap_or(Value::Bool(false));
        if call_done && !py::truthy(vm, &has_done)? {
            py::method_kw(vm, this, "do_done", vec![who.clone(), what.clone()], mk)?;
            if py::truthy(vm, &who)? && is_str(&who) {
                log_line(vm, who)?;
            }
            log_line(vm, what)?;
            log_line(vm, py::s(vm, ""))?;
        }
        Ok(())
    })();
    if py::is_none(&multiple) && interact {
        store_set(vm, "_side_image_attributes", old_side)?;
    }
    if has_handle {
        let mut before = Value::None;
        if !py::is_none(&old_attr_state) {
            let st = py::items(vm, &old_attr_state)?;
            before = vm.call_method(&st[1], "get_attributes", vec![Value::None, tag.clone()])?;
        }
        let restored = vm.call_method(this, "restore_say_attributes", vec![Value::Bool(false), old_attr_state.clone(), Value::Bool(interact)])?;
        if py::truthy(vm, &restored)? {
            let st = py::items(vm, &old_attr_state)?;
            let after = vm.call_method(&st[1], "get_attributes", vec![Value::None, tag])?;
            vm.call_method(this, "handle_say_transition", vec![py::s(vm, "restore"), before, after])?;
        }
    }
    r?;
    Ok(Value::None)
}

fn c_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let what = args.get(1).cloned().or_else(|| py::kw(kw, "what").cloned()).unwrap_or(Value::None);
    let interact = arg_or(&args[1..], kw, 1, "interact", Value::Bool(true));
    let interact = py::truthy(vm, &interact)?;
    let call_done = arg_or(&args[1..], kw, 2, "_call_done", Value::Bool(true));
    let call_done = py::truthy(vm, &call_done)?;
    if e6(vm) {
        let rest: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "what" | "interact" | "_call_done")).cloned().collect();
        return call_6(vm, this, &what, interact, call_done, &rest);
    }
    let multiple = arg_or(&args[1..], kw, 3, "multiple", Value::None);
    let rest: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "what" | "interact" | "_call_done" | "multiple")).cloned().collect();
    call_7(vm, this, &what, interact, call_done, multiple, &rest)
}

fn c_statement_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if condition_false(vm, &args[0])? {
        return Ok(py::s(vm, "say-condition-false"));
    }
    let sn = py::get_opt(vm, &args[0], "_statement_name")?.unwrap_or(Value::None);
    if !py::is_none(&sn) {
        return Ok(sn);
    }
    Ok(py::s(vm, "say"))
}

fn c_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let what = args[1].clone();
    let old = e6(vm);
    let mut state = Value::None;
    if old {
        vm.call_method(this, "resolve_say_attributes", vec![Value::Bool(true)])?;
        let sa = cfg(vm, "speaking_attribute", Value::None);
        if !py::is_none(&sa) {
            let w = vm.types.new_list(vec![sa]);
            py::method_kw(vm, this, "resolve_say_attributes", vec![Value::Bool(true)], kw_of(&[("wanted", w)]))?;
        }
    } else if py::get_opt(vm, this, "handle_say_attributes")?.is_some() {
        state = vm.call_method(this, "handle_say_attributes", vec![Value::Bool(true), Value::Bool(true)])?;
    }
    let old_side = store_get(vm, "_side_image_attributes")?;
    let tag = py::get(vm, this, "image_tag")?;
    let attrs = if py::truthy(vm, &tag)? {
        let a = get_attributes(vm, py::s(vm, "master"), &tag)?;
        tuple_with(vm, &[tag.clone()], &a)?
    } else {
        Value::None
    };
    store_set(vm, "_side_image_attributes", attrs)?;
    let r = (|| -> PyResult<Value> {
        let dynamic = py::get(vm, this, "dynamic")?;
        let who = if py::truthy(vm, &dynamic)? { py::s(vm, "<Dynamic>") } else { py::get(vm, this, "name")? };
        vm.call_method(this, "do_predict", vec![who, what])
    })();
    store_set(vm, "_side_image_attributes", old_side)?;
    if !old && py::get_opt(vm, this, "restore_say_attributes")?.is_some() {
        vm.call_method(this, "restore_say_attributes", vec![Value::Bool(true), state, Value::Bool(true)])?;
    }
    r
}

fn c_will_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if condition_false(vm, &args[0])? {
        return Ok(Value::Bool(false));
    }
    let da = py::get(vm, &args[0], "display_args")?;
    ops::getitem(vm, &da, py::s(vm, "interact"))
}

fn c_add_history(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let hl = cfg(vm, "history_length", Value::None);
    if py::is_none(&hl) {
        return Ok(Value::None);
    }
    let enabled = store_get(vm, "_history")?;
    if !py::truthy(vm, &enabled)? {
        return Ok(Value::None);
    }
    let history = store_get(vm, "_history_list")?;
    let he = py::import_attr(vm, MODULE, "HistoryEntry")?;
    let h = vm.call(&he, Vec::new(), Vec::new())?;
    py::set(vm, &h, "kind", args[1].clone())?;
    py::set(vm, &h, "who", args[2].clone())?;
    py::set(vm, &h, "what", args[3].clone())?;
    for n in ["who_args", "what_args", "window_args", "show_args", "image_tag"] {
        let v = py::get(vm, this, n)?;
        py::set(vm, &h, n, v)?;
    }
    if !e6(vm) {
        let multiple = arg_or(&args[4..], kw, 0, "multiple", Value::None);
        py::set(vm, &h, "multiple", multiple)?;
    }
    let c = ctx(vm)?;
    let rb = py::get(vm, &c, "rollback")?;
    let rid = if py::truthy(vm, &rb)? {
        let log = py::import_attr(vm, "renpy.game", "log")?;
        let cur = py::get(vm, &log, "current")?;
        py::get(vm, &cur, "identifier")?
    } else {
        Value::None
    };
    py::set(vm, &h, "rollback_identifier", rid)?;
    for (k, v) in kw {
        if &**k != "multiple" {
            py::set(vm, &h, k, v.clone())?;
        }
    }
    let hc = cfg(vm, "history_callbacks", Value::None);
    if !py::is_none(&hc) {
        for f in py::items(vm, &hc)? {
            vm.call(&f, vec![h.clone()], Vec::new())?;
        }
    }
    vm.call_method(&history, "append", vec![h])?;
    let limit = py::int(vm, &hl)?;
    while ops::len(vm, &history)? as i64 > limit {
        vm.call_method(&history, "pop", vec![Value::Int(0)])?;
    }
    Ok(Value::None)
}

fn c_pop_history(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&cfg(vm, "history_length", Value::None)) {
        return Ok(Value::None);
    }
    let enabled = store_get(vm, "_history")?;
    if !py::truthy(vm, &enabled)? {
        return Ok(Value::None);
    }
    let history = store_get(vm, "_history_list")?;
    if e6(vm) || py::truthy(vm, &history)? {
        vm.call_method(&history, "pop", Vec::new())?;
    }
    Ok(Value::None)
}

fn f_character(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ns = not_set(vm)?;
    let name = args.first().cloned().or_else(|| py::kw(kw, "name").cloned()).unwrap_or(ns);
    let mut kind = args.get(1).cloned().or_else(|| py::kw(kw, "kind").cloned()).unwrap_or(Value::None);
    if py::is_none(&kind) {
        kind = store_get(vm, "adv")?;
    }
    if at_least(vm, 8, 1) {
        if let Some(c) = py::get_opt(vm, &kind, "character")? {
            kind = c;
        }
    }
    let ty = Value::Obj(ops::type_of(vm, &kind));
    let mut k: Vec<(Rc<str>, Value)> = kw.iter().filter(|(k, _)| !matches!(&**k, "name" | "kind")).cloned().collect();
    k.push(("kind".into(), kind));
    vm.call(&ty, vec![name], k)
}

fn f_dynamic_character(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut k = kw.to_vec();
    k.push(("dynamic".into(), Value::Bool(true)));
    f_character(vm, &args[..1], &k)
}

fn he_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    if !Rc::ptr_eq(&ops::type_of(vm, a), &ops::type_of(vm, b)) {
        return Ok(Value::Bool(false));
    }
    let (da, db) = (py::namespace_of(vm, a)?, py::namespace_of(vm, b)?);
    Ok(Value::Bool(ops::eq(vm, &da, &db)?))
}

fn he_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::namespace_of(vm, &args[0])?;
    let items = vm.call_method(&d, "items", Vec::new())?;
    let sorted = py::import_attr(vm, "builtins", "sorted")?;
    let s = vm.call(&sorted, vec![items], Vec::new())?;
    let items = py::items(vm, &s)?;
    let t = vm.types.new_tuple(items);
    Ok(Value::Int(ops::hash(vm, &t)?))
}

fn he_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let who = py::get_or_none(vm, &args[0], "who")?;
    let what = py::get_or_none(vm, &args[0], "what")?;
    let (a, b) = (ops::repr(vm, &who)?, ops::repr(vm, &what)?);
    Ok(vm.types.new_str(&format!("<History {a} {b}>")))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("predict_show_display_say", f_predict_show_display_say),
            ("compute_widget_properties", f_compute_widget_properties),
            ("show_display_say", f_show_display_say),
            ("display_say", f_display_say),
            ("Character", f_character),
            ("DynamicCharacter", f_dynamic_character),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let dtt = py::class(vm, MODULE, "DialogueTextTags", &[], &[("__init__", dtt_init)], &[])?;
    set(vm, "DialogueTextTags", Value::Obj(dtt));
    let cb = py::class(vm, MODULE, "Callbacks", &[], &[("__init__", cb_init), ("__call__", cb_call), ("copy", cb_copy)], &[])?;
    set(vm, "Callbacks", Value::Obj(cb));
    let sd = py::class(vm, MODULE, "SlowDone", &[], &[("__init__", sd_init), ("__call__", sd_call)], &[("delay", Value::None), ("last_pause", Value::Bool(true)), ("no_wait", Value::Bool(false))])?;
    set(vm, "SlowDone", Value::Obj(sd));
    let obj = py::import_attr(vm, "renpy.object", "Object")?;
    let he = py::class(vm, MODULE, "HistoryEntry", &[obj], &[("__eq__", he_eq), ("__hash__", he_hash), ("__repr__", he_repr)], &[("multiple", Value::None), ("who", Value::None), ("what", Value::None)])?;
    set(vm, "HistoryEntry", Value::Obj(he));
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let ns = vm.call(&sentinel, vec![py::s(vm, "NotSet")], Vec::new())?;
    set(vm, "NotSet", ns);
    let special = vm.types.new_list(["what_prefix", "what_suffix", "who_prefix", "who_suffix", "show_function"].iter().map(|s| vm.types.new_str(s)).collect());
    let methods: py::Methods = &[
        ("__init__", c_init),
        ("copy", c_copy),
        ("do_add", c_do_add),
        ("get_show_properties", c_get_show_properties),
        ("do_show", c_do_show),
        ("do_done", c_do_done),
        ("do_extend", c_do_extend),
        ("do_display", c_do_display),
        ("do_predict", c_do_predict),
        ("resolve_say_attributes", c_resolve_say_attributes),
        ("handle_say_attributes", c_handle_say_attributes),
        ("handle_say_transition", c_handle_say_transition),
        ("restore_say_attributes", c_restore_say_attributes),
        ("__str__", c_str),
        ("__unicode__", c_str),
        ("__format__", c_format),
        ("__repr__", c_repr),
        ("empty_window", c_empty_window),
        ("has_character_arguments", c_has_character_arguments),
        ("prefix_suffix", c_prefix_suffix),
        ("__call__", c_call),
        ("statement_name", c_statement_name),
        ("predict", c_predict),
        ("will_interact", c_will_interact),
        ("add_history", c_add_history),
        ("pop_history", c_pop_history),
    ];
    let adv = py::class(vm, MODULE, "ADVCharacter", &[], methods, &[("special_properties", special), ("voice_tag", Value::None), ("properties", py::dict(vm)), ("_statement_name", Value::None)])?;
    if e6(vm) {
        // Not part of the 6.x class.
        for n in ["handle_say_attributes", "handle_say_transition", "restore_say_attributes", "has_character_arguments", "prefix_suffix", "statement_name", "get_show_properties"] {
            if let Some(d) = adv.attr_table() {
                d.borrow_mut().del_str(n);
            }
        }
    }
    set(vm, "ADVCharacter", Value::Obj(adv));
    set(vm, "afm_text_queue", vm.types.new_list(Vec::new()));
    set(vm, "multiple_count", Value::Int(0));
    set(vm, "less_pauses", Value::Bool(false));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
