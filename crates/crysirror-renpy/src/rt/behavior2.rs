// SPDX-License-Identifier: MPL-2.0
//! Second half of `renpy.display.behavior` (after Ren'Py 8.3, MIT): text
//! input, adjustments and bars, timers, mouse areas, OnEvent, AreaPicker
//! and the web input stub.
use super::{
    behavior::{ignore_event, map_event, map_name, run},
    events,
    layout::{rargs, style as sty},
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.display.behavior";

fn module(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, MODULE)
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = module(vm)?;
    py::set(vm, &m, n, v)
}

fn timeout(vm: &mut Vm, t: f64) -> PyResult<()> {
    let i = py::import_attr(vm, "renpy.game", "interface")?;
    if !py::is_none(&i) {
        vm.call_method(&i, "timeout", vec![Value::Float(t.max(0.))])?;
    }
    Ok(())
}

fn evargs(vm: &mut Vm, args: &[Value]) -> PyResult<(Value, f64, f64, f64)> {
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let st = py::f64_of(vm, &args.get(4).cloned().unwrap_or(Value::Int(0)))?;
    Ok((ev, x, y, st))
}

fn split(
    args: &[Value],
    kw: Kw,
    names: &[&str],
    defaults: Vec<Value>,
) -> (Vec<Value>, Vec<(Rc<str>, Value)>) {
    let mut vals = defaults;
    for (i, v) in args.iter().skip(1).enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    (vals, props)
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn cs(v: &[char]) -> String {
    v.iter().collect()
}

// Input bookkeeping ----------------------------------------------------------------------------------

fn f_input_pre(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l1 = vm.types.new_list(Vec::new());
    let l2 = vm.types.new_list(Vec::new());
    mset(vm, "input_values", l1)?;
    mset(vm, "inputs", l2)?;
    mset(vm, "default_input_value", Value::None)?;
    Ok(Value::None)
}

fn f_input_post(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let values = mget(vm, "input_values")?;
    let values = py::items(vm, &values)?;
    let cur = mget(vm, "current_input_value")?;
    let mut found = false;
    for v in &values {
        if ops::eq(vm, v, &cur)? {
            found = true;
            break;
        }
    }
    if !found {
        let d = mget(vm, "default_input_value")?;
        mset(vm, "current_input_value", d)?;
        mset(vm, "input_value_active", Value::Bool(true))?;
    }
    let cur = mget(vm, "current_input_value")?;
    let active = {
        let v = mget(vm, "input_value_active")?;
        ops::truthy(vm, &v)?
    };
    let inputs = mget(vm, "inputs")?;
    for i in py::items(vm, &inputs)? {
        let value = py::get(vm, &i, "value")?;
        let ed = py::get_or_none(vm, &value, "editable")?;
        let editable = ops::eq(vm, &value, &cur)? && active && ops::truthy(vm, &ed)?;
        let content = vm.call_method(&value, "get_text", Vec::new())?;
        let ie = {
            let v = py::get_or_none(vm, &i, "editable")?;
            ops::truthy(vm, &v)?
        };
        let ic = py::get_or_none(vm, &i, "content")?;
        if ie != editable || !ops::eq(vm, &content, &ic)? {
            vm.call_method(
                &i,
                "update_text",
                vec![content.clone(), Value::Bool(editable)],
            )?;
            let n = ops::len(vm, &content)?;
            py::set(vm, &i, "caret_pos", Value::Int(n as i64))?;
        }
    }
    Ok(Value::None)
}

// CaretBlink ------------------------------------------------------------------------------------------

fn caret_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (vals, mut props) = split(
        args,
        kw,
        &["caret", "caret_blink"],
        vec![Value::None, Value::None],
    );
    if !props.iter().any(|(k, _)| &**k == "yalign") {
        props.push(("yalign".into(), Value::Float(0.)));
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    let mut caret = super::easy::displayable(vm, &vals[0], &Value::None, true)?;
    let dup = py::get_or_none(vm, &caret, "_duplicatable")?;
    if ops::truthy(vm, &dup)? {
        caret = vm.call_method(&caret, "_duplicate", vec![Value::None])?;
        vm.call_method(&caret, "_unique", Vec::new())?;
    }
    py::set(vm, this, "caret", caret)?;
    py::set(vm, this, "caret_blink", vals[1].clone())?;
    py::set(vm, this, "st_base", Value::Int(0))?;
    Ok(Value::None)
}

fn caret_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "caret")?;
    vm.call_method(&c, "get_placement", Vec::new())
}

fn caret_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "caret")?;
    Ok(vm.types.new_list(vec![c]))
}

fn caret_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let base = {
        let v = py::get_or_none(vm, this, "st_base")?;
        py::f64_of(vm, &v).unwrap_or(0.)
    };
    let st = st - base;
    let caret = py::get(vm, this, "caret")?;
    let cr = render::render(vm, &caret, w, h, st, at)?;
    let (cw, _) = super::layout::size(&cr);
    let rv = super::layout::new(vm, cw, h);
    let blink = {
        let v = py::get(vm, this, "caret_blink")?;
        py::f64_of(vm, &v)?
    };
    let ttl = blink - st.rem_euclid(blink);
    if ttl > blink / 2. {
        super::layout::blit(vm, &rv, &cr, 0., 0.)?;
    }
    render::redraw(vm, this, ttl % (blink / 2.));
    Ok(rv)
}

// Input -----------------------------------------------------------------------------------------------

const INPUT_NAMES: &[&str] = &[
    "default",
    "length",
    "style",
    "allow",
    "exclude",
    "prefix",
    "suffix",
    "changed",
    "button",
    "replaces",
    "editable",
    "pixel_width",
    "value",
    "copypaste",
    "caret_blink",
    "multiline",
    "action",
];

fn input_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let e = vm.types.new_str("");
    let defaults = vec![
        e.clone(),
        Value::None,
        vm.types.new_str("input"),
        Value::None,
        Value::None,
        e.clone(),
        e.clone(),
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(true),
        Value::None,
        Value::None,
        Value::Bool(false),
        Value::None,
        Value::Bool(false),
        Value::None,
    ];
    let (mut v, props) = split(args, kw, INPUT_NAMES, defaults);
    let text = py::import_attr(vm, "renpy.text.text", "Text")?;
    let ti = py::get(vm, &text, "__init__")?;
    let mut tp = props.clone();
    tp.push(("style".into(), v[2].clone()));
    tp.push(("replaces".into(), v[9].clone()));
    tp.push(("substitute".into(), Value::Bool(false)));
    vm.call(&ti, vec![this.clone(), e.clone()], tp)?;
    if py::is_none(&v[14]) {
        v[14] = py::config(vm, "input_caret_blink").unwrap_or(Value::None);
    }
    if ops::truthy(vm, &v[12])? {
        py::set(vm, this, "value", v[12].clone())?;
        v[7] = py::get(vm, &v[12], "set_text")?;
        v[0] = vm.call_method(&v[12], "get_text", Vec::new())?;
    }
    if py::is_none(&v[0]) {
        let msg = if py::is_none(&v[12]) {
            "The default value of an input must not be None."
        } else {
            "The data accessed by an InputValue must not be None."
        };
        return Err(py::error(vm, msg));
    }
    let ds = py::string(vm, &v[0])?;
    let default = vm.types.new_string(ds);
    py::set(vm, this, "default", default.clone())?;
    py::set(vm, this, "content", default.clone())?;
    for (n, i) in [
        ("length", 1),
        ("allow", 3),
        ("exclude", 4),
        ("prefix", 5),
        ("suffix", 6),
        ("copypaste", 13),
        ("changed", 7),
        ("editable", 10),
        ("pixel_width", 11),
        ("multiline", 15),
        ("action", 16),
    ] {
        py::set(vm, this, n, v[i].clone())?;
    }
    let mut caretprops: Vec<(Rc<str>, Value)> = vec![("color".into(), Value::None)];
    for (k, val) in &props {
        if k.ends_with("color") {
            caretprops.push((k.clone(), val.clone()));
        }
    }
    // The caret's size: xmaximum=1 through 7.4.6, xsize=1 from 7.4.7,
    // one line of `preferences.font_size` from 7.5 / 8.0.
    let version = super::state::rt(vm).profile.version;
    if version >= [7, 5, 0, 0] {
        let prefs = py::import_attr(vm, "renpy.store", "preferences")?;
        let fs = py::get_or_none(vm, &prefs, "font_size").unwrap_or(Value::None);
        let fs = if py::is_none(&fs) { Value::Int(1) } else { fs };
        caretprops.push(("xysize".into(), vm.types.new_tuple(vec![Value::Int(1), fs])));
    } else if version >= [7, 4, 7, 0] {
        caretprops.push(("xsize".into(), Value::Int(1)));
    } else {
        caretprops.push(("xmaximum".into(), Value::Int(1)));
    }
    caretprops.push(("style".into(), v[2].clone()));
    let solid = py::import_attr(vm, "renpy.display.image", "Solid")?;
    let mut caret = vm.call(&solid, Vec::new(), caretprops)?;
    if ops::truthy(vm, &v[14])? {
        let cb = mget(vm, "CaretBlink")?;
        caret = vm.call(&cb, vec![caret, v[14].clone()], Vec::new())?;
    }
    py::set(vm, this, "caret", caret)?;
    let n = ops::len(vm, &default)? as i64;
    py::set(vm, this, "caret_pos", Value::Int(n))?;
    py::set(vm, this, "old_caret_pos", Value::Int(n))?;
    if ops::truthy(vm, &v[8])? {
        let button = v[8].clone();
        py::set(vm, this, "editable", Value::Bool(false))?;
        let hp = mget(vm, "HoveredProxy")?;
        let en = py::get(vm, this, "enable")?;
        let bh = py::get_or_none(vm, &button, "hovered")?;
        let h = vm.call(&hp, vec![en, bh], Vec::new())?;
        py::set(vm, &button, "hovered", h)?;
        let dis = py::get(vm, this, "disable")?;
        let bu = py::get_or_none(vm, &button, "unhovered")?;
        let u = vm.call(&hp, vec![dis, bu], Vec::new())?;
        py::set(vm, &button, "unhovered", u)?;
    }
    let input_cls = mget(vm, "Input")?;
    if let Some(c) = input_cls.obj().cloned() {
        if ops::isinstance(vm, &v[9], &c) {
            for n in ["content", "editable", "caret_pos", "shown"] {
                let x = py::get_or_none(vm, &v[9], n)?;
                py::set(vm, this, n, x)?;
            }
        }
    }
    let content = py::get(vm, this, "content")?;
    let ed = py::get(vm, this, "editable")?;
    vm.call_method(this, "update_text", vec![content, ed])?;
    Ok(Value::None)
}

fn escape(s: &str) -> String {
    s.replace('{', "{{")
}

fn input_set_content(
    vm: &mut Vm,
    this: &Value,
    content: &str,
    editable: bool,
    edit_text: &str,
    caret: &Value,
) -> PyResult<()> {
    let prefix = {
        let v = py::get_or_none(vm, this, "prefix")?;
        py::string(vm, &v).unwrap_or_default()
    };
    let suffix = {
        let v = py::get_or_none(vm, this, "suffix")?;
        py::string(vm, &v).unwrap_or_default()
    };
    let mut parts: Vec<Value> = Vec::new();
    if editable {
        let c = chars(content);
        let pos = {
            let v = py::get_or_none(vm, this, "caret_pos")?;
            py::int(vm, &v).unwrap_or(0)
        }
        .clamp(0, c.len() as i64) as usize;
        parts.push(vm.types.new_string(prefix));
        parts.push(vm.types.new_string(escape(&cs(&c[..pos]))));
        parts.push(vm.types.new_string(edit_text.to_owned()));
        parts.push(caret.clone());
        parts.push(vm.types.new_string(escape(&cs(&c[pos..]))));
        parts.push(vm.types.new_string(suffix));
    } else {
        parts.push(vm.types.new_string(prefix));
        parts.push(vm.types.new_string(escape(content)));
        parts.push(vm.types.new_string(suffix));
    }
    if content.is_empty() {
        parts.push(vm.types.new_str("{space=1}"));
    }
    let l = vm.types.new_list(parts);
    vm.call_method(this, "set_text", vec![l])?;
    let own_caret = py::get_or_none(vm, this, "caret")?;
    let cb = mget(vm, "CaretBlink")?;
    if let Some(c) = cb.obj().cloned() {
        if ops::isinstance(vm, &own_caret, &c) {
            let st = py::get_or_none(vm, this, "st")?;
            py::set(vm, &own_caret, "st_base", st)?;
            render::redraw(vm, &own_caret, 0.);
        }
    }
    Ok(())
}

fn input_update_text(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let new_content = py::string(vm, &arg_or(args, kw, 1, "new_content", Value::None))?;
    let editable = {
        let v = arg_or(args, kw, 2, "editable", Value::Bool(false));
        ops::truthy(vm, &v)?
    };
    let check_size = {
        let v = arg_or(args, kw, 3, "check_size", Value::Bool(false));
        ops::truthy(vm, &v)?
    };
    let disp_iface = py::import_attr(vm, "renpy.display", "interface")?;
    let edit = if py::is_none(&disp_iface) {
        Value::None
    } else {
        py::get_or_none(vm, &disp_iface, "text_editing")?
    };
    let old_content = {
        let v = py::get_or_none(vm, this, "content")?;
        py::string(vm, &v).unwrap_or_default()
    };
    let cur_ed = {
        let v = py::get_or_none(vm, this, "editable")?;
        ops::truthy(vm, &v)?
    };
    let has_edit = ops::truthy(vm, &edit)?;
    if new_content != old_content || editable != cur_ed || has_edit {
        render::redraw(vm, this, 0.);
    }
    py::set(vm, this, "editable", Value::Bool(editable))?;
    let mut caret = sty(vm, this, "caret")?;
    if py::is_none(&caret) {
        caret = py::get(vm, this, "caret")?;
    }
    let edit_text = if has_edit {
        let t = {
            let v = py::get(vm, &edit, "text")?;
            py::string(vm, &v)?
        };
        let s = {
            let v = py::get(vm, &edit, "start")?;
            py::int(vm, &v)?
        }
        .max(0) as usize;
        let l = {
            let v = py::get(vm, &edit, "length")?;
            py::int(vm, &v)?
        }
        .max(0) as usize;
        py::set(vm, this, "edit_text", vm.types.new_string(t.clone()))?;
        let c = chars(&t);
        let s = s.min(c.len());
        let e = (s + l).min(c.len());
        let mut out = String::new();
        for (part, u) in [(&c[..s], 1), (&c[s..e], 2), (&c[e..], 1)] {
            if !part.is_empty() {
                out += &format!("{{u={u}}}{}{{/u}}", escape(&cs(part)));
            }
        }
        out
    } else {
        py::set(vm, this, "edit_text", vm.types.new_str(""))?;
        String::new()
    };
    input_set_content(vm, this, &new_content, editable, &edit_text, &caret)?;
    let pw = py::get_or_none(vm, this, "pixel_width")?;
    if check_size && ops::truthy(vm, &pw)? {
        let sz = vm.call_method(this, "size", Vec::new())?;
        let w = {
            let s = py::items(vm, &sz)?;
            py::f64_of(vm, &s[0])?
        };
        if w > py::f64_of(vm, &pw)? {
            let ocp = py::get_or_none(vm, this, "old_caret_pos")?;
            py::set(vm, this, "caret_pos", ocp)?;
            input_set_content(vm, this, &old_content, editable, &edit_text, &caret)?;
            return Ok(Value::None);
        }
    }
    if new_content != old_content {
        let nc = vm.types.new_string(new_content);
        py::set(vm, this, "content", nc.clone())?;
        let ch = py::get_or_none(vm, this, "changed")?;
        if ops::truthy(vm, &ch)? {
            vm.call(&ch, vec![nc], Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn input_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::get(vm, this, "style")?;
    let p = py::get_or_none(vm, &s, "prefix")?;
    if !ops::eq(vm, &args[1], &p)? {
        let c = py::get_or_none(vm, this, "content")?;
        let e = py::get_or_none(vm, this, "editable")?;
        vm.call_method(this, "update_text", vec![c, e])?;
    }
    let text = py::import_attr(vm, "renpy.text.text", "Text")?;
    let f = py::get(vm, &text, "set_style_prefix")?;
    vm.call(&f, args.to_vec(), Vec::new())
}

fn input_enable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "content")?;
    vm.call_method(&args[0], "update_text", vec![c, Value::Bool(true)])
}

fn input_disable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "content")?;
    vm.call_method(&args[0], "update_text", vec![c, Value::Bool(false)])
}

fn input_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let value = py::get_or_none(vm, this, "value")?;
    if !py::is_none(&value) {
        let inputs = mget(vm, "inputs")?;
        vm.call_method(&inputs, "append", vec![this.clone()])?;
        let iv = mget(vm, "input_values")?;
        vm.call_method(&iv, "append", vec![value.clone()])?;
        let d = py::get_or_none(vm, &value, "default")?;
        let div = mget(vm, "default_input_value")?;
        if ops::truthy(vm, &d)? && py::is_none(&div) {
            mset(vm, "default_input_value", value.clone())?;
        }
    }
    let shown = {
        let v = py::get_or_none(vm, this, "shown")?;
        ops::truthy(vm, &v)?
    };
    if !shown {
        if !py::is_none(&value) {
            let d = vm.call_method(&value, "get_text", Vec::new())?;
            let ds = py::string(vm, &d)?;
            let s = vm.types.new_string(ds);
            py::set(vm, this, "default", s)?;
        }
        let d = py::get(vm, this, "default")?;
        py::set(vm, this, "content", d.clone())?;
        let n = ops::len(vm, &d)? as i64;
        py::set(vm, this, "caret_pos", Value::Int(n))?;
        let e = py::get_or_none(vm, this, "editable")?;
        vm.call_method(this, "update_text", vec![d, e])?;
        py::set(vm, this, "shown", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn update(vm: &mut Vm, this: &Value, content: &str, check: bool) -> PyResult<()> {
    let e = py::get_or_none(vm, this, "editable")?;
    let c = vm.types.new_string(content.to_owned());
    vm.call_method(this, "update_text", vec![c, e, Value::Bool(check)])?;
    Ok(())
}

fn set_caret(vm: &mut Vm, this: &Value, p: usize) -> PyResult<()> {
    py::set(vm, this, "caret_pos", Value::Int(p as i64))
}

/// End of the last whitespace run before `pos` that does not end at `pos`.
fn word_left(c: &[char], pos: usize) -> usize {
    let mut space = 0;
    let mut i = 0;
    while i < pos {
        if c[i].is_whitespace() {
            let mut j = i;
            while j < pos && c[j].is_whitespace() {
                j += 1;
            }
            if j != pos {
                space = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    space
}

fn accept(vm: &mut Vm, filt: &Value, c: char) -> PyResult<Option<bool>> {
    if !ops::truthy(vm, filt)? {
        return Ok(None);
    }
    let cv = vm.types.new_string(c.to_string());
    if filt.as_str().is_none() {
        if let Some(search) = py::get_opt(vm, filt, "search")? {
            let r = vm.call(&search, vec![cv], Vec::new())?;
            return Ok(Some(!py::is_none(&r)));
        }
    }
    Ok(Some(ops::contains(vm, filt, &cv)?))
}

fn input_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, _, _, st) = evargs(vm, args)?;
    py::set(vm, this, "st", Value::Float(st))?;
    let cp = py::get_or_none(vm, this, "caret_pos")?;
    py::set(vm, this, "old_caret_pos", cp.clone())?;
    let editable = {
        let v = py::get_or_none(vm, this, "editable")?;
        ops::truthy(vm, &v)?
    };
    if !editable {
        return Ok(Value::None);
    }
    let mut edit_controls = false;
    for n in [
        "input_jump_word_left",
        "input_jump_word_right",
        "input_delete_word",
        "input_delete_full",
    ] {
        edit_controls |= map_name(vm, &ev, n)?;
    }
    let ty = {
        let v = py::get_or_none(vm, &ev, "type")?;
        py::int(vm, &v).unwrap_or(0)
    };
    let mods = events::input(vm).borrow().mods;
    let unicode = {
        let v = py::get_or_none(vm, &ev, "unicode")?;
        if py::is_none(&v) {
            String::new()
        } else {
            py::string(vm, &v)?
        }
    };
    if ty == events::KEYDOWN && mods & 0x100 != 0 && unicode.is_empty() && !edit_controls {
        return Ok(Value::None);
    }
    let content = {
        let v = py::get_or_none(vm, this, "content")?;
        py::string(vm, &v)?
    };
    let c = chars(&content);
    let l = c.len();
    let pos = (py::int(vm, &cp).unwrap_or(0).max(0) as usize).min(l);
    let multiline = {
        let v = py::get_or_none(vm, this, "multiline")?;
        ops::truthy(vm, &v)?
    };
    let copypaste = {
        let v = py::get_or_none(vm, this, "copypaste")?;
        ops::truthy(vm, &v)?
    };
    let mut raw_text: Option<String> = None;
    macro_rules! done {
        () => {{
            render::redraw(vm, this, 0.);
            return Err(ignore_event(vm));
        }};
    }
    if map_name(vm, &ev, "input_backspace")? {
        if !c.is_empty() && pos > 0 {
            let nc = cs(&c[..pos - 1]) + &cs(&c[pos..]);
            set_caret(vm, this, pos - 1)?;
            update(vm, this, &nc, false)?;
        }
        done!();
    } else if multiline && map_name(vm, &ev, "input_next_line")? {
        let nc = cs(&c[..pos]) + "\n" + &cs(&c[pos..]);
        set_caret(vm, this, pos + 1)?;
        update(vm, this, &nc, false)?;
        done!();
    } else if map_name(vm, &ev, "input_enter")? {
        let et = {
            let v = py::get_or_none(vm, this, "edit_text")?;
            py::string(vm, &v).unwrap_or_default()
        };
        let out = if et.is_empty() {
            content.clone()
        } else {
            cs(&c[..pos]) + &et + &cs(&c[pos..])
        };
        let action = py::get_or_none(vm, this, "action")?;
        if !py::is_none(&action) {
            let rv = run(vm, &action, Vec::new(), Vec::new())?;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
            return Err(ignore_event(vm));
        }
        let value = py::get_or_none(vm, this, "value")?;
        if ops::truthy(vm, &value)? {
            let rv = vm.call_method(&value, "enter", Vec::new())?;
            // Before 7.8 / 8.3 a None result passes the event on (to a
            // screen's `key "K_RETURN"`, for one).
            if !py::is_none(&rv) || !super::state::rt(vm).profile.since_pair(8, 3) {
                return Ok(rv);
            }
            return Err(ignore_event(vm));
        }
        let ch = py::get_or_none(vm, this, "changed")?;
        if !ops::truthy(vm, &ch)? {
            return Ok(vm.types.new_string(out));
        }
    } else if map_name(vm, &ev, "input_left")? {
        if pos > 0 {
            set_caret(vm, this, pos - 1)?;
            update(vm, this, &content, false)?;
        }
        done!();
    } else if map_name(vm, &ev, "input_jump_word_left")? {
        if pos > 0 {
            set_caret(vm, this, word_left(&c, pos))?;
            update(vm, this, &content, false)?;
        }
        done!();
    } else if map_name(vm, &ev, "input_right")? {
        if pos < l {
            set_caret(vm, this, pos + 1)?;
            update(vm, this, &content, false)?;
        }
        done!();
    } else if map_name(vm, &ev, "input_jump_word_right")? {
        if pos < l {
            let mut space = l;
            let mut i = pos + 1;
            while i < l {
                if c[i].is_whitespace() {
                    let mut j = i;
                    while j < l && c[j].is_whitespace() {
                        j += 1;
                    }
                    space = j - (pos + 1);
                    break;
                }
                i += 1;
            }
            set_caret(vm, this, (space + pos + 1).min(l))?;
            update(vm, this, &content, false)?;
        }
        done!();
    } else if map_name(vm, &ev, "input_delete")? {
        if pos < l {
            let nc = cs(&c[..pos]) + &cs(&c[pos + 1..]);
            update(vm, this, &nc, false)?;
        }
        done!();
    } else if map_name(vm, &ev, "input_delete_word")? {
        let sp = word_left(&c, pos);
        let nc = cs(&c[..sp]) + &cs(&c[pos..]);
        set_caret(vm, this, sp)?;
        update(vm, this, &nc, false)?;
        done!();
    } else if map_name(vm, &ev, "input_delete_full")? {
        let nc = cs(&c[pos..]);
        set_caret(vm, this, 0)?;
        update(vm, this, &nc, false)?;
        done!();
    } else if map_name(vm, &ev, "input_home")? {
        set_caret(vm, this, 0)?;
        update(vm, this, &content, false)?;
        done!();
    } else if map_name(vm, &ev, "input_end")? {
        set_caret(vm, this, l)?;
        update(vm, this, &content, false)?;
        done!();
    } else if copypaste && map_name(vm, &ev, "input_copy")? {
        super::set_clipboard_text(vm, content.clone());
        return Err(ignore_event(vm));
    } else if copypaste && map_name(vm, &ev, "input_paste")? {
        let t = super::clipboard_text(vm);
        raw_text = Some(t.chars().filter(|c| *c as u32 >= 32).collect());
    } else if ty == events::TEXTEDITING {
        update(vm, this, &content, true)?;
        return Err(ignore_event(vm));
    } else if ty == events::TEXTINPUT {
        py::set(vm, this, "edit_text", vm.types.new_str(""))?;
        let t = py::get_or_none(vm, &ev, "text")?;
        raw_text = Some(py::string(vm, &t)?);
    } else if ty == events::KEYDOWN {
        if unicode.chars().next().is_some_and(|c| c as u32 >= 32) {
            raw_text = Some(unicode.clone());
        } else if text_event_in_queue(vm)? {
            return Err(ignore_event(vm));
        } else {
            let key = {
                let v = py::get_or_none(vm, &ev, "key")?;
                py::int(vm, &v).unwrap_or(0)
            };
            let emod = {
                let v = py::get_or_none(vm, &ev, "mod")?;
                py::int(vm, &v).unwrap_or(0)
            };
            if (32..127).contains(&key) && emod & (0x300 | 0xC00) == 0 {
                return Err(ignore_event(vm));
            }
        }
    }
    if let Some(raw) = raw_text {
        let allow = py::get_or_none(vm, this, "allow")?;
        let exclude = py::get_or_none(vm, this, "exclude")?;
        let mut text = String::new();
        for ch in raw.chars() {
            if accept(vm, &allow, ch)? == Some(false) {
                continue;
            }
            if accept(vm, &exclude, ch)? == Some(true) {
                continue;
            }
            text.push(ch);
        }
        let length = py::get_or_none(vm, this, "length")?;
        if ops::truthy(vm, &length)? {
            let remaining = (py::int(vm, &length)? - l as i64).max(0) as usize;
            text = text.chars().take(remaining).collect();
        }
        if !text.is_empty() {
            let n = text.chars().count();
            let nc = cs(&c[..pos]) + &text + &cs(&c[pos..]);
            set_caret(vm, this, pos + n)?;
            update(vm, this, &nc, true)?;
        }
        return Err(ignore_event(vm));
    }
    Ok(Value::None)
}

fn text_event_in_queue(vm: &mut Vm) -> PyResult<bool> {
    let queued: Vec<Value> = events::input(vm).borrow().queue.clone();
    for ev in queued {
        if events::is_type(vm, &ev, events::TEXTINPUT)?
            || events::is_type(vm, &ev, events::TEXTEDITING)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn input_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "st", args[3].clone())?;
    let text = py::import_attr(vm, "renpy.text.text", "Text")?;
    let f = py::get(vm, &text, "render")?;
    let rv = vm.call(&f, args.to_vec(), Vec::new())?;
    let ed = py::get_or_none(vm, this, "editable")?;
    if ops::truthy(vm, &ed)? {
        py::set(vm, &rv, "text_input", Value::Bool(true))?;
    }
    Ok(rv)
}

// Adjustment -------------------------------------------------------------------------------------------

fn is_float(v: &Value) -> bool {
    matches!(v, Value::Float(_))
}

/// `type(like)(x)`: int truncation when `like` is an int.
fn same_type(vm: &Vm, like: &Value, x: f64) -> Value {
    let _ = vm;
    if is_float(like) {
        Value::Float(x)
    } else {
        Value::Int(x.trunc() as i64)
    }
}

fn adj_registered(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let reg = mget(vm, "adj_registered")?;
    let l = vm.call_method(
        &reg,
        "setdefault",
        vec![this.clone(), vm.types.new_list(Vec::new())],
    )?;
    py::items(vm, &l)
}

fn adj_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "range",
        "value",
        "step",
        "page",
        "changed",
        "adjustable",
        "ranged",
        "force_step",
        "raw_changed",
    ];
    let (v, _) = split(
        args,
        kw,
        &names,
        vec![
            Value::Int(1),
            Value::Int(0),
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::Bool(false),
            Value::None,
        ],
    );
    let mut adjustable = v[5].clone();
    if py::is_none(&adjustable) && ops::truthy(vm, &v[4])? {
        adjustable = Value::Bool(true);
    }
    py::set(vm, this, "_range", v[0].clone())?;
    let val = py::f64_of(vm, &v[1])?;
    py::set(vm, this, "_value", same_type(vm, &v[0], val))?;
    py::set(vm, this, "_page", v[3].clone())?;
    py::set(vm, this, "_step", v[2].clone())?;
    py::set(vm, this, "changed", v[4].clone())?;
    py::set(vm, this, "adjustable", adjustable)?;
    py::set(vm, this, "ranged", v[6].clone())?;
    py::set(vm, this, "force_step", v[7].clone())?;
    py::set(vm, this, "raw_changed", v[8].clone())?;
    Ok(Value::None)
}

fn adj_viewport_replaces(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, r) = (&args[0], &args[1]);
    if py::same(this, r) {
        return Ok(Value::None);
    }
    for n in [
        "range",
        "value",
        "animation_amplitude",
        "animation_target",
        "animation_start",
        "animation_delay",
        "animation_warper",
    ] {
        let v = py::get_or_none(vm, r, n)?;
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn fget(vm: &mut Vm, this: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, this, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn adj_round_value(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let value = arg_or(args, kw, 1, "value", Value::Int(0));
    let release = {
        let v = arg_or(args, kw, 2, "release", Value::Bool(false));
        ops::truthy(vm, &v)?
    };
    let x = py::f64_of(vm, &value)?;
    let cur = py::get(vm, this, "_value")?;
    let range = py::get(vm, this, "_range")?;
    if x <= 0. {
        return Ok(same_type(vm, &cur, 0.));
    }
    if x >= py::f64_of(vm, &range)? {
        return Ok(range);
    }
    let fs = py::get_or_none(vm, this, "force_step")?;
    if matches!(fs, Value::Bool(false)) {
        return Ok(value);
    }
    if !release && fs.as_str() == Some("release") {
        return Ok(value);
    }
    let step = {
        let s = py::get(vm, this, "step")?;
        py::f64_of(vm, &s)?
    };
    Ok(same_type(vm, &cur, step * (x / step).round()))
}

fn adj_get_value(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v = py::get(vm, this, "_value")?;
    let r = py::get(vm, this, "_range")?;
    let x = py::f64_of(vm, &v)?;
    if x <= 0. {
        return Ok(same_type(vm, &v, 0.));
    }
    if x >= py::f64_of(vm, &r)? {
        return Ok(r);
    }
    Ok(v)
}

fn adj_set_value(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_value", args[1].clone())?;
    Ok(Value::None)
}

fn adj_get_range(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_range")
}

fn adj_set_range(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "_range", args[1].clone())?;
    let ranged = py::get_or_none(vm, this, "ranged")?;
    if ops::truthy(vm, &ranged)? {
        vm.call(&ranged, vec![this.clone()], Vec::new())?;
    }
    let ri = py::get_or_none(vm, this, "restart_interaction_at_range")?;
    if ops::truthy(vm, &ri)? {
        let f = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
        vm.call(&f, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn adj_get_page(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::get_or_none(vm, &args[0], "_page")?;
    if !py::is_none(&p) {
        return Ok(p);
    }
    let r = py::get(vm, &args[0], "_range")?;
    Ok(Value::Float(py::f64_of(vm, &r)? / 10.))
}

fn adj_set_page(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_page", args[1].clone())?;
    Ok(Value::None)
}

fn adj_get_step(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::get_or_none(vm, this, "_step")?;
    if !py::is_none(&s) {
        return Ok(s);
    }
    let p = py::get_or_none(vm, this, "_page")?;
    if !py::is_none(&p) {
        let page = {
            let v = py::get(vm, this, "page")?;
            py::f64_of(vm, &v)?
        };
        if page > 0. {
            return Ok(Value::Float(py::f64_of(vm, &p)? / 10.));
        }
    }
    let r = py::get(vm, this, "_range")?;
    if is_float(&r) {
        return Ok(Value::Float(py::f64_of(vm, &r)? / 10.));
    }
    Ok(Value::Int(1))
}

fn adj_set_step(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_step", args[1].clone())?;
    Ok(Value::None)
}

fn adj_register(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let reg = mget(vm, "adj_registered")?;
    let l = vm.call_method(
        &reg,
        "setdefault",
        vec![args[0].clone(), vm.types.new_list(Vec::new())],
    )?;
    vm.call_method(&l, "append", vec![args[1].clone()])?;
    Ok(Value::None)
}

fn adj_change(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut value = arg_or(args, kw, 1, "value", Value::Int(0));
    let end = {
        let v = arg_or(args, kw, 2, "end_animation", Value::Bool(true));
        ops::truthy(vm, &v)?
    };
    if end {
        vm.call_method(this, "end_animation", Vec::new())?;
    }
    let raw = py::get_or_none(vm, this, "raw_changed")?;
    if ops::truthy(vm, &raw)? {
        let rv = vm.call(&raw, vec![this.clone(), value.clone()], Vec::new())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    let range = py::get(vm, this, "_range")?;
    let rf = py::f64_of(vm, &range)?;
    if py::f64_of(vm, &value)? < 0. {
        value = Value::Int(0);
    }
    if py::f64_of(vm, &value)? > rf {
        value = range.clone();
    }
    let cur = py::get(vm, this, "_value")?;
    if !ops::eq(vm, &value, &cur)? {
        let ril = py::get_or_none(vm, this, "restart_interaction_at_limit")?;
        if ops::truthy(vm, &ril)? {
            let (nv, cv) = (py::f64_of(vm, &value)?, py::f64_of(vm, &cur)?);
            if (nv == 0.) != (cv == 0.) || (nv == rf) != (cv == rf) {
                let f = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
                vm.call(&f, Vec::new(), Vec::new())?;
            }
        }
        py::set(vm, this, "_value", value.clone())?;
        for d in adj_registered(vm, this)? {
            render::redraw(vm, &d, 0.);
        }
        let ch = py::get_or_none(vm, this, "changed")?;
        if ops::truthy(vm, &ch)? {
            return vm.call(&ch, vec![value], Vec::new());
        }
    }
    Ok(Value::None)
}

fn adj_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for d in adj_registered(vm, &args[0])? {
        render::invalidate(vm, &d);
    }
    Ok(Value::None)
}

fn adj_inertia_warper(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::f64_of(vm, &args[1])?.clamp(0., 1.);
    Ok(Value::Float(1. - (-d * 6.).exp()))
}

fn adj_animate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let amp = arg_or(args, kw, 1, "amplitude", Value::Int(0));
    let delay = arg_or(args, kw, 2, "delay", Value::Int(0));
    let warper = arg_or(args, kw, 3, "warper", Value::None);
    let range = py::get(vm, this, "_range")?;
    if !ops::truthy(vm, &amp)? || !ops::truthy(vm, &range)? {
        vm.call_method(this, "end_animation", Vec::new())?;
    } else {
        let cur = {
            let v = py::get(vm, this, "_value")?;
            py::f64_of(vm, &v)?
        };
        let a = py::f64_of(vm, &amp)?;
        py::set(vm, this, "animation_amplitude", amp)?;
        py::set(vm, this, "animation_target", Value::Float(cur + a))?;
        py::set(vm, this, "animation_delay", delay)?;
        py::set(vm, this, "animation_start", Value::None)?;
        py::set(vm, this, "animation_warper", warper)?;
        vm.call_method(this, "update", Vec::new())?;
    }
    Ok(Value::None)
}

fn adj_inertia(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let amp = arg_or(args, kw, 1, "amplitude", Value::Int(0));
    let tc = {
        let v = arg_or(args, kw, 2, "time_constant", Value::Int(0));
        py::f64_of(vm, &v)?
    };
    let st = arg_or(args, kw, 3, "st", Value::Int(0));
    let w = py::get(vm, this, "inertia_warper")?;
    vm.call_method(this, "animate", vec![amp, Value::Float(tc * 6.), w])?;
    vm.call_method(this, "periodic", vec![st])?;
    Ok(Value::None)
}

fn adj_end_animation(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let instantly = {
        let v = arg_or(args, kw, 1, "instantly", Value::Bool(false));
        ops::truthy(vm, &v)?
    };
    let target = py::get_or_none(vm, this, "animation_target")?;
    if !py::is_none(&target) || instantly {
        for n in [
            "animation_amplitude",
            "animation_target",
            "animation_start",
            "animation_delay",
            "animation_warper",
        ] {
            py::set(vm, this, n, Value::None)?;
        }
        if !instantly {
            py::method_kw(
                vm,
                this,
                "change",
                vec![target],
                vec![("end_animation".into(), Value::Bool(false))],
            )?;
        }
    }
    Ok(Value::None)
}

fn adj_periodic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = py::f64_of(vm, &args[1])?;
    let target = py::get_or_none(vm, this, "animation_target")?;
    if py::is_none(&target) {
        return Ok(Value::None);
    }
    let start = py::get_or_none(vm, this, "animation_start")?;
    let start = if py::is_none(&start) {
        py::set(vm, this, "animation_start", Value::Float(st))?;
        st
    } else {
        py::f64_of(vm, &start)?
    };
    let end_inst = |vm: &mut Vm| {
        py::method_kw(
            vm,
            this,
            "end_animation",
            vec![],
            vec![("instantly".into(), Value::Bool(true))],
        )
    };
    if st < start {
        end_inst(vm)?;
        return Ok(Value::Int(0));
    }
    let delay = fget(vm, this, "animation_delay")?;
    if delay == 0. {
        vm.call_method(this, "end_animation", Vec::new())?;
        return Ok(Value::Int(0));
    }
    let w = py::get(vm, this, "animation_warper")?;
    let d = vm.call(&w, vec![Value::Float((st - start) / delay)], Vec::new())?;
    let done = py::f64_of(vm, &d)?;
    let amp = fget(vm, this, "animation_amplitude")?;
    let value = py::f64_of(vm, &target)? - amp * (1. - done);
    py::method_kw(
        vm,
        this,
        "change",
        vec![Value::Float(value)],
        vec![("end_animation".into(), Value::Bool(false))],
    )?;
    let range = fget(vm, this, "_range")?;
    if value < 0. || value > range {
        end_inst(vm)?;
        return Ok(Value::Int(0));
    }
    if st > start + delay {
        vm.call_method(this, "end_animation", Vec::new())?;
        return Ok(Value::None);
    }
    Ok(Value::Int(0))
}

// Bar -----------------------------------------------------------------------------------------------

fn bar_draggable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get_or_none(vm, &args[0], "focusable")
}

fn bar_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version < 1 {
        let adj = mget(vm, "Adjustment")?;
        let r = py::get_or_none(vm, this, "range")?;
        let v = py::get_or_none(vm, this, "value")?;
        let c = py::get_or_none(vm, this, "changed")?;
        let a = vm.call(&adj, vec![r, v], vec![("changed".into(), c)])?;
        vm.call_method(&a, "register", vec![this.clone()])?;
        py::set(vm, this, "adjustment", a)?;
        for n in ["range", "value", "changed"] {
            let nv = vm.types.new_str(n);
            let _ = ops::delattr(vm, this, &nv);
        }
    }
    if version < 2 {
        py::set(vm, this, "value", Value::None)?;
    }
    Ok(Value::None)
}

fn bar_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "range",
        "value",
        "width",
        "height",
        "changed",
        "adjustment",
        "step",
        "page",
        "bar",
        "style",
        "vertical",
        "replaces",
        "hovered",
        "unhovered",
        "released",
    ];
    let mut defaults = vec![Value::None; names.len()];
    defaults[10] = Value::Bool(false);
    let (v, mut props) = split(args, kw, &names, defaults);
    py::set(vm, this, "value", Value::None)?;
    let mut adjustment = v[5].clone();
    if py::is_none(&adjustment) {
        let bv = py::import_attr(vm, "renpy.ui", "BarValue")
            .ok()
            .and_then(|c| c.obj().cloned());
        if bv.is_some_and(|c| ops::isinstance(vm, &v[1], &c)) {
            let bar = mget(vm, "Bar")?;
            if bar.obj().is_some_and(|c| ops::isinstance(vm, &v[11], c)) {
                let rv = py::get_or_none(vm, &v[11], "value")?;
                vm.call_method(&v[1], "replaces", vec![rv])?;
            }
            py::set(vm, this, "value", v[1].clone())?;
            adjustment = vm.call_method(&v[1], "get_adjustment", Vec::new())?;
            timeout(vm, 0.)?;
            let tt = vm.call_method(&v[1], "get_tooltip", Vec::new())?;
            if !py::is_none(&tt) && !props.iter().any(|(k, _)| &**k == "tooltip") {
                props.push(("tooltip".into(), tt));
            }
        } else {
            let adj = mget(vm, "Adjustment")?;
            adjustment = vm.call(
                &adj,
                vec![v[0].clone(), v[1].clone()],
                vec![
                    ("step".into(), v[6].clone()),
                    ("page".into(), v[7].clone()),
                    ("changed".into(), v[4].clone()),
                ],
            )?;
        }
    }
    let mut style = v[9].clone();
    if py::is_none(&style) {
        let value = py::get_or_none(vm, this, "value")?;
        let vertical = ops::truthy(vm, &v[10])?;
        style = if !py::is_none(&value) {
            let s = vm.call_method(&value, "get_style", Vec::new())?;
            ops::getitem(vm, &s, Value::Int(if vertical { 1 } else { 0 }))?
        } else {
            vm.types.new_str(if vertical { "vbar" } else { "bar" })
        };
    }
    if !py::is_none(&v[2]) {
        props.push(("xmaximum".into(), v[2].clone()));
    }
    if !py::is_none(&v[3]) {
        props.push(("ymaximum".into(), v[3].clone()));
    }
    props.push(("style".into(), style));
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "adjustment", adjustment)?;
    py::set(vm, this, "focusable", Value::Bool(true))?;
    for n in ["thumb_dim", "height", "width"] {
        py::set(vm, this, n, Value::Int(0))?;
    }
    py::set(vm, this, "hidden", Value::Bool(false))?;
    py::set(vm, this, "hovered", v[12].clone())?;
    py::set(vm, this, "unhovered", v[13].clone())?;
    py::set(vm, this, "released", v[14].clone())?;
    Ok(Value::None)
}

fn bar_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let value = py::get_or_none(vm, this, "value")?;
    if !py::is_none(&value) {
        let adj = vm.call_method(&value, "get_adjustment", Vec::new())?;
        let av = py::get(vm, &adj, "value")?;
        if !ops::eq(vm, &av, &value)? {
            render::invalidate(vm, this);
        }
        py::set(vm, this, "adjustment", adj)?;
    }
    let adj = py::get(vm, this, "adjustment")?;
    let a = py::get_or_none(vm, &adj, "adjustable")?;
    py::set(vm, this, "focusable", a)?;
    vm.call_method(&adj, "register", vec![this.clone()])?;
    Ok(Value::None)
}

fn bar_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rv = vm.types.new_list(Vec::new());
    let s = py::get(vm, &args[0], "style")?;
    let app = py::get(vm, &rv, "append")?;
    vm.call_method(&s, "_visit_bar", vec![app])?;
    Ok(rv)
}

fn bar_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (mut w, mut h, st, at) = rargs(vm, args)?;
    let value_obj = py::get_or_none(vm, this, "value")?;
    if !py::is_none(&value_obj) {
        let r = vm.call_method(&value_obj, "periodic", vec![Value::Float(st)])?;
        if !py::is_none(&r) {
            let t = py::f64_of(vm, &r)?;
            render::redraw(vm, this, t);
        }
    }
    let (xmin, ymin) = super::layout::xyminimums(vm, this, w, h)?;
    w = w.max(xmin);
    h = h.max(ymin);
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    let adj = py::get(vm, this, "adjustment")?;
    let range = {
        let v = py::get(vm, &adj, "range")?;
        py::f64_of(vm, &v)?
    };
    let mut value = {
        let v = py::get(vm, &adj, "value")?;
        py::f64_of(vm, &v)?
    };
    let page = {
        let v = py::get(vm, &adj, "page")?;
        py::f64_of(vm, &v)?
    };
    if range <= 0. {
        let u = sty(vm, this, "unscrollable")?;
        match u.as_str() {
            Some("hide") => {
                py::set(vm, this, "hidden", Value::Bool(true))?;
                return Ok(super::layout::new(vm, w, h));
            }
            Some("insensitive") => {
                vm.call_method(
                    this,
                    "set_style_prefix",
                    vec![vm.types.new_str("insensitive_"), Value::Bool(true)],
                )?;
            }
            _ => {}
        }
    } else {
        let s = py::get(vm, this, "style")?;
        let p = py::get_or_none(vm, &s, "prefix")?;
        if p.as_str() == Some("insensitive_") {
            vm.call_method(
                this,
                "set_style_prefix",
                vec![vm.types.new_str("idle_"), Value::Bool(true)],
            )?;
        }
    }
    py::set(vm, this, "hidden", Value::Bool(false))?;
    let vertical = super::layout::style_b(vm, this, "bar_vertical")?;
    if super::layout::style_b(vm, this, "bar_invert")? ^ vertical {
        value = range - value;
    }
    let dimension = if vertical { h } else { w };
    let fore_gutter = super::layout::style_f(vm, this, "fore_gutter", dimension)?;
    let aft_gutter = super::layout::style_f(vm, this, "aft_gutter", dimension)?;
    let mut active = dimension - fore_gutter - aft_gutter;
    let thumb_dim0 = if range != 0. {
        (active * page / (range + page)).floor()
    } else {
        active
    };
    let thumb_offset = super::layout::style_f(vm, this, "thumb_offset", dimension)?.abs();
    let thumb_s = sty(vm, this, "thumb")?;
    let shadow_s = sty(vm, this, "thumb_shadow")?;
    let r = |vm: &mut Vm, d: &Value, w: f64, h: f64| -> PyResult<Value> {
        if py::is_none(d) {
            Ok(super::layout::new(vm, 0., 0.))
        } else {
            render::render(vm, d, w, h, st, at)
        }
    };
    let (thumb, shadow, mut thumb_dim) = if vertical {
        let t = r(vm, &thumb_s, w, thumb_dim0)?;
        let s = r(vm, &shadow_s, w, thumb_dim0)?;
        let d = super::layout::size(&t).1;
        (t, s, d)
    } else {
        let t = r(vm, &thumb_s, thumb_dim0, h)?;
        let s = r(vm, &shadow_s, thumb_dim0, h)?;
        let d = super::layout::size(&t).0;
        (t, s, d)
    };
    thumb_dim -= thumb_offset * 2.;
    py::set(vm, this, "thumb_dim", Value::Float(thumb_dim))?;
    active -= thumb_dim;
    let fore = if range != 0. {
        (active * value / range).floor()
    } else {
        active
    };
    let fore = fore.trunc();
    let aft = active - fore + aft_gutter;
    let fore = fore + fore_gutter;
    let rv = super::layout::new(vm, w, h);
    let resizing = super::layout::style_b(vm, this, "bar_resizing")?;
    let fore_bar = sty(vm, this, "fore_bar")?;
    let aft_bar = sty(vm, this, "aft_bar")?;
    let blit = |vm: &mut Vm, src: &Value, x: f64, y: f64, main: bool| -> PyResult<()> {
        let pos = vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]);
        py::method_kw(
            vm,
            &rv,
            "blit",
            vec![src.clone(), pos],
            vec![("main".into(), Value::Bool(main))],
        )?;
        Ok(())
    };
    if vertical {
        if resizing {
            let fs = r(vm, &fore_bar, w, fore)?;
            let as_ = r(vm, &aft_bar, w, aft)?;
            blit(vm, &shadow, 0., fore - thumb_offset, true)?;
            blit(vm, &fs, 0., 0., false)?;
            blit(vm, &as_, 0., h - aft, false)?;
        } else {
            let fs = r(vm, &fore_bar, w, h)?;
            let as_ = r(vm, &aft_bar, w, h)?;
            blit(vm, &shadow, 0., fore - thumb_offset, true)?;
            let fsub = render::subsurface(vm, &fs, [0., 0., w, fore], false, false)?;
            let asub = render::subsurface(vm, &as_, [0., h - aft, w, aft], false, false)?;
            blit(vm, &fsub, 0., 0., false)?;
            blit(vm, &asub, 0., h - aft, false)?;
        }
        blit(vm, &thumb, 0., fore - thumb_offset, true)?;
    } else {
        if resizing {
            let fs = r(vm, &fore_bar, fore, h)?;
            let as_ = r(vm, &aft_bar, aft, h)?;
            blit(vm, &shadow, fore - thumb_offset, 0., true)?;
            blit(vm, &fs, 0., 0., false)?;
            blit(vm, &as_, w - aft, 0., false)?;
        } else {
            let fs = r(vm, &fore_bar, w, h)?;
            let as_ = r(vm, &aft_bar, w, h)?;
            blit(vm, &shadow, fore - thumb_offset, 0., true)?;
            let fsub = render::subsurface(vm, &fs, [0., 0., fore, h], false, false)?;
            let asub = render::subsurface(vm, &as_, [w - aft, 0., aft, h], false, false)?;
            blit(vm, &fsub, 0., 0., false)?;
            blit(vm, &asub, w - aft, 0., false)?;
        }
        blit(vm, &thumb, fore - thumb_offset, 0., true)?;
    }
    let f = py::get_or_none(vm, this, "focusable")?;
    if ops::truthy(vm, &f)? {
        vm.call_method(
            &rv,
            "add_focus",
            vec![
                this.clone(),
                Value::None,
                Value::Int(0),
                Value::Int(0),
                Value::Float(w),
                Value::Float(h),
            ],
        )?;
    }
    Ok(rv)
}

fn bar_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "focus")?;
    vm.call(&f, vec![this.clone(), Value::Bool(default)], Vec::new())?;
    vm.call_method(this, "set_transform_event", vec![vm.types.new_str("hover")])?;
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        run(vm, &h, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn bar_unfocus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "unfocus")?;
    vm.call(&f, vec![this.clone()], Vec::new())?;
    vm.call_method(this, "set_transform_event", vec![vm.types.new_str("idle")])?;
    if !default {
        let h = py::get_or_none(vm, this, "hovered")?;
        let ru = mget(vm, "run_unhovered")?;
        vm.call(&ru, vec![h], Vec::new())?;
        let u = py::get_or_none(vm, this, "unhovered")?;
        run(vm, &u, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn speak(vm: &mut Vm, what: &str, translate: bool) -> PyResult<()> {
    let mut s = vm.types.new_string(what.to_owned());
    if translate {
        let tr = py::import_attr(vm, "renpy.minstore", "__")?;
        s = vm.call(&tr, vec![s], Vec::new())?;
    }
    if let Ok(sp) = py::import_attr(vm, "renpy.display.tts", "speak") {
        vm.call(&sp, vec![s], Vec::new())?;
    }
    Ok(())
}

fn get_grab(vm: &mut Vm) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.display.focus", "get_grab")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn set_grab(vm: &mut Vm, v: Value) -> PyResult<()> {
    let f = py::import_attr(vm, "renpy.display.focus", "set_grab")?;
    vm.call(&f, vec![v], Vec::new())?;
    Ok(())
}

fn bar_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, x, y, _) = evargs(vm, args)?;
    for (n, want) in [("focusable", true), ("hidden", false)] {
        let v = py::get_or_none(vm, this, n)?;
        if ops::truthy(vm, &v)? != want {
            return Ok(Value::None);
        }
    }
    let f = vm.call_method(this, "is_focused", Vec::new())?;
    if !ops::truthy(vm, &f)? {
        return Ok(Value::None);
    }
    let adj = py::get(vm, this, "adjustment")?;
    let range_v = py::get(vm, &adj, "range")?;
    let range = py::f64_of(vm, &range_v)?;
    let old_v = py::get(vm, &adj, "value")?;
    let old_value = py::f64_of(vm, &old_v)?;
    let mut value = old_value;
    let vertical = super::layout::style_b(vm, this, "bar_vertical")?;
    let invert = super::layout::style_b(vm, this, "bar_invert")? ^ vertical;
    let mut old_inverted = 0.;
    if invert {
        value = range - value;
        old_inverted = value;
    }
    let mut grabbed = py::same(&get_grab(vm)?, this);
    let mut just_grabbed = false;
    let mut ignore = false;
    if !grabbed && map_name(vm, &ev, "bar_activate")? {
        speak(vm, "activate", true)?;
        set_grab(vm, this.clone())?;
        vm.call_method(
            this,
            "set_style_prefix",
            vec![vm.types.new_str("hover_"), Value::Bool(true)],
        )?;
        just_grabbed = true;
        grabbed = true;
        ignore = true;
        let snd = sty(vm, this, "activate_sound")?;
        if !py::is_none(&snd) {
            let p = py::import_attr(vm, "renpy.exports", "play")?;
            vm.call(&p, vec![snd], Vec::new())?;
        }
    }
    if grabbed {
        let (inc, dec) = if vertical {
            ("bar_down", "bar_up")
        } else {
            ("bar_right", "bar_left")
        };
        let step = {
            let v = py::get(vm, &adj, "step")?;
            py::f64_of(vm, &v)?
        };
        if map_name(vm, &ev, dec)? {
            speak(vm, "decrease", true)?;
            value -= step;
            ignore = true;
        }
        if map_name(vm, &ev, inc)? {
            speak(vm, "increase", true)?;
            value += step;
            ignore = true;
        }
        let ty = {
            let v = py::get_or_none(vm, &ev, "type")?;
            py::int(vm, &v).unwrap_or(0)
        };
        if matches!(
            ty,
            events::MOUSEMOTION | events::MOUSEBUTTONUP | events::MOUSEBUTTONDOWN
        ) {
            let tdim = fget(vm, this, "thumb_dim")?;
            let (pos, size) = if vertical {
                (y, fget(vm, this, "height")?)
            } else {
                (x, fget(vm, this, "width")?)
            };
            let fg = super::layout::style_f(vm, this, "fore_gutter", size)?;
            let ag = super::layout::style_f(vm, this, "aft_gutter", size)?;
            let zone = size - fg - ag - tdim;
            value = if zone != 0. {
                (pos - fg - tdim / 2.) * range / zone
            } else {
                0.
            };
            ignore = true;
        }
        if !is_float(&range_v) {
            value = value.trunc();
        }
        if value < 0. {
            speak(vm, "", false)?;
            value = 0.;
        }
        if value > range {
            speak(vm, "", false)?;
            value = range;
        }
    }
    if invert {
        value = if value == old_inverted {
            old_value
        } else {
            range - value
        };
    }
    let num = |v: f64| {
        if is_float(&range_v) || is_float(&old_v) {
            Value::Float(v)
        } else {
            Value::Int(v as i64)
        }
    };
    if grabbed && !just_grabbed && map_name(vm, &ev, "bar_deactivate")? {
        speak(vm, "deactivate", true)?;
        set_grab(vm, Value::None)?;
        vm.call_method(
            this,
            "set_style_prefix",
            vec![vm.types.new_str("hover_"), Value::Bool(true)],
        )?;
        let v = py::method_kw(
            vm,
            &adj,
            "round_value",
            vec![num(value)],
            vec![("release".into(), Value::Bool(true))],
        )?;
        if !ops::eq(vm, &v, &old_v)? {
            let rv = vm.call_method(&adj, "change", vec![v])?;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
        }
        let rel = py::get_or_none(vm, this, "released")?;
        let rv = run(vm, &rel, Vec::new(), Vec::new())?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
        return Err(ignore_event(vm));
    }
    if value != old_value {
        let v = py::method_kw(
            vm,
            &adj,
            "round_value",
            vec![num(value)],
            vec![("release".into(), Value::Bool(false))],
        )?;
        let rv = vm.call_method(&adj, "change", vec![v])?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    if ignore {
        return Err(ignore_event(vm));
    }
    Ok(Value::None)
}

fn bar_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let root = args.get(2).cloned().unwrap_or(Value::Bool(true));
    if ops::truthy(vm, &root)? {
        let mut prefix = args[1].clone();
        if py::same(&get_grab(vm)?, this) {
            let p = py::string(vm, &prefix)?;
            prefix = vm.types.new_string(format!("selected_{p}"));
        }
        let d = super::core::displayable_class(vm);
        let f = py::get(vm, &Value::Obj(d), "set_style_prefix")?;
        vm.call(&f, vec![this.clone(), prefix, root], Vec::new())?;
    }
    Ok(Value::None)
}

fn bar_tts_all(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let value = py::get_or_none(vm, this, "value")?;
    let alt = if py::is_none(&value) {
        vm.types.new_str("")
    } else {
        py::get_or_none(vm, &value, "alt")?
    };
    let c = vm.call_method(this, "_tts_common", vec![alt])?;
    let tr = py::import_attr(vm, "renpy.minstore", "__")?;
    let b = vm.call(&tr, vec![vm.types.new_str("bar")], Vec::new())?;
    ops::binary_op(vm, &c, &b, crysirror_python::ast::BinOp::Add, false, false)
}

fn empty_str(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_str(""))
}

// Conditional, Timer, MouseArea, OnEvent ----------------------------------------------------------------

fn store_eval(vm: &mut Vm, cond: &Value) -> PyResult<Value> {
    let ns = crate::engine::module_namespace(vm, "store")
        .ok_or_else(|| vm.runtime_error("store is not initialized"))?;
    let ev = vm.builtins_module.clone();
    let f = py::get(vm, &Value::Obj(ev), "eval")?;
    vm.call(&f, vec![cond.clone(), Value::Obj(ns)], Vec::new())
}

fn cond_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let condition = args.get(1).cloned().unwrap_or(Value::None);
    let mut cargs = vec![this.clone()];
    cargs.extend(args.iter().skip(2).cloned());
    super::layout::container_init(vm, &cargs, kw)?;
    py::set(vm, this, "condition", condition.clone())?;
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let n = vm.call(&null, Vec::new(), Vec::new())?;
    py::set(vm, this, "null", n)?;
    let s = store_eval(vm, &condition)?;
    py::set(vm, this, "state", s)?;
    Ok(Value::None)
}

fn cond_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let state = {
        let v = py::get_or_none(vm, this, "state")?;
        ops::truthy(vm, &v)?
    };
    let d = py::get(vm, this, if state { "child" } else { "null" })?;
    render::render(vm, &d, w, h, st, at)
}

fn cond_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = py::get(vm, this, "condition")?;
    let state = store_eval(vm, &c)?;
    let old = py::get_or_none(vm, this, "state")?;
    if !ops::eq(vm, &state, &old)? {
        render::redraw(vm, this, 0.);
    }
    py::set(vm, this, "state", state.clone())?;
    if ops::truthy(vm, &state)? {
        let child = py::get(vm, this, "child")?;
        return vm.call_method(&child, "event", args[1..].to_vec());
    }
    Ok(Value::None)
}

fn timer_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if py::int(vm, &args[1])? < 1 {
        let ts = mget(vm, "TimerState")?;
        let s = vm.call(&ts, Vec::new(), Vec::new())?;
        for n in ["started", "next_event"] {
            let v = py::get_or_none(vm, this, n)?;
            py::set(vm, &s, n, v)?;
        }
        py::set(vm, this, "state", s)?;
    }
    Ok(Value::None)
}

fn timer_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "delay", "action", "repeat", "args", "kwargs", "replaces", "modal",
    ];
    let (v, props) = split(
        args,
        kw,
        &names,
        vec![
            Value::None,
            Value::None,
            Value::Bool(false),
            vm.types.new_tuple(Vec::new()),
            vm.types.new_dict(),
            Value::None,
            Value::None,
        ],
    );
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let ni = py::get(vm, &null, "__init__")?;
    vm.call(&ni, vec![this.clone()], props)?;
    if py::f64_of(vm, &v[0])? <= 0. {
        return Err(py::error(vm, "A timer's delay must be > 0."));
    }
    py::set(vm, this, "delay", v[0].clone())?;
    py::set(vm, this, "repeat", v[2].clone())?;
    py::set(vm, this, "next_event", Value::None)?;
    py::set(vm, this, "function", v[1].clone())?;
    py::set(vm, this, "args", v[3].clone())?;
    py::set(vm, this, "kwargs", v[4].clone())?;
    py::set(vm, this, "started", Value::Bool(false))?;
    let modal = if py::is_none(&v[6]) {
        py::config(vm, "modal_blocks_timer").unwrap_or(Value::Bool(false))
    } else {
        v[6].clone()
    };
    py::set(vm, this, "modal", modal)?;
    let tc = mget(vm, "Timer")?;
    let state = if tc.obj().is_some_and(|c| ops::isinstance(vm, &v[5], c)) {
        py::get(vm, &v[5], "state")?
    } else {
        let ts = mget(vm, "TimerState")?;
        vm.call(&ts, Vec::new(), Vec::new())?
    };
    py::set(vm, this, "state", state)?;
    Ok(Value::None)
}

fn timer_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let state = py::get(vm, this, "state")?;
    let started = {
        let v = py::get_or_none(vm, &state, "started")?;
        ops::truthy(vm, &v)?
    };
    if !started {
        let delay = fget(vm, this, "delay")?;
        py::set(vm, &state, "started", Value::Bool(true))?;
        py::set(vm, &state, "next_event", Value::Float(st + delay))?;
        timeout(vm, st)?;
    }
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let f = py::get(vm, &null, "render")?;
    vm.call(
        &f,
        vec![
            this.clone(),
            Value::Float(w),
            Value::Float(h),
            Value::Float(st),
            Value::Float(at),
        ],
        Vec::new(),
    )
}

fn timer_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, _, _, st) = evargs(vm, args)?;
    if events::is_type(vm, &ev, events::TIMEEVENT)? {
        let m = py::get_or_none(vm, this, "modal")?;
        let em = py::get_or_none(vm, &ev, "modal")?;
        if ops::truthy(vm, &m)? && ops::truthy(vm, &em)? {
            return Ok(Value::None);
        }
    }
    let state = py::get(vm, this, "state")?;
    let delay = fget(vm, this, "delay")?;
    let started = {
        let v = py::get_or_none(vm, &state, "started")?;
        ops::truthy(vm, &v)?
    };
    if !started {
        py::set(vm, &state, "started", Value::Bool(true))?;
        py::set(vm, &state, "next_event", Value::Float(st + delay))?;
    }
    let ne = py::get_or_none(vm, &state, "next_event")?;
    if py::is_none(&ne) {
        return Ok(Value::None);
    }
    let ne = py::f64_of(vm, &ne)?;
    if st < ne {
        timeout(vm, ne - st)?;
        return Ok(Value::None);
    }
    let repeat = {
        let v = py::get_or_none(vm, this, "repeat")?;
        ops::truthy(vm, &v)?
    };
    if !repeat {
        py::set(vm, &state, "next_event", Value::None)?;
    } else {
        let mut n = ne + delay;
        if n < st {
            n = st + delay;
        }
        py::set(vm, &state, "next_event", Value::Float(n))?;
        timeout(vm, n - st)?;
    }
    let f = py::get_or_none(vm, this, "function")?;
    let a = py::get_items(vm, this, "args")?;
    let k = py::get(vm, this, "kwargs")?;
    let k = py::str_items(vm, &k)?;
    run(vm, &f, a, k)
}

fn mousearea_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (v, props) = split(
        args,
        kw,
        &["hovered", "unhovered", "replaces"],
        vec![Value::None; 3],
    );
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "hovered", v[0].clone())?;
    py::set(vm, this, "unhovered", v[1].clone())?;
    py::set(vm, this, "is_hovered", Value::Bool(false))?;
    let mc = mget(vm, "MouseArea")?;
    if mc.obj().is_some_and(|c| ops::isinstance(vm, &v[2], c)) {
        let h = py::get_or_none(vm, &v[2], "is_hovered")?;
        py::set(vm, this, "is_hovered", h)?;
    }
    py::set(vm, this, "width", Value::Int(0))?;
    py::set(vm, this, "height", Value::Int(0))?;
    Ok(Value::None)
}

fn mousearea_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    py::set(vm, this, "at_st_offset", Value::Float(at - st))?;
    Ok(super::layout::new(vm, w, h))
}

fn mousearea_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, x, y, st) = evargs(vm, args)?;
    let g = get_grab(vm)?;
    if ops::truthy(vm, &g)? {
        return Ok(Value::None);
    }
    let pft =
        py::import_attr(vm, "renpy.display.focus", "pending_focus_type").unwrap_or(Value::None);
    let (w, h) = (fget(vm, this, "width")?, fget(vm, this, "height")?);
    let fm = sty(vm, this, "focus_mask")?;
    let is_hovered = if pft.as_str() == Some("keyboard") {
        false
    } else if events::is_type(vm, &ev, events::TIMEEVENT)? && {
        let m = py::get_or_none(vm, &ev, "modal")?;
        ops::truthy(vm, &m)?
    } {
        false
    } else if !py::is_none(&fm) {
        let off = fget(vm, this, "at_st_offset")?;
        let cr = render::render(vm, &fm, w, h, st, off + st)?;
        let r = vm.call_method(
            &cr,
            "is_pixel_opaque",
            vec![Value::Float(x), Value::Float(y)],
        )?;
        ops::truthy(vm, &r)?
    } else {
        (0. ..w).contains(&x) && (0. ..h).contains(&y)
    };
    let was = {
        let v = py::get_or_none(vm, this, "is_hovered")?;
        ops::truthy(vm, &v)?
    };
    if is_hovered && !was {
        py::set(vm, this, "is_hovered", Value::Bool(true))?;
        let hv = py::get_or_none(vm, this, "hovered")?;
        return run(vm, &hv, Vec::new(), Vec::new());
    } else if !is_hovered && was {
        py::set(vm, this, "is_hovered", Value::Bool(false))?;
        let hv = py::get_or_none(vm, this, "hovered")?;
        let ru = mget(vm, "run_unhovered")?;
        vm.call(&ru, vec![hv], Vec::new())?;
        let u = py::get_or_none(vm, this, "unhovered")?;
        run(vm, &u, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn onevent_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let event = arg_or(args, kw, 1, "event", Value::None);
    let action = arg_or(args, kw, 2, "action", vm.types.new_list(Vec::new()));
    super::core::d_init(vm, &[this.clone()], &[])?;
    py::set(vm, this, "event_name", event)?;
    py::set(vm, this, "action", action)?;
    Ok(Value::None)
}

fn onevent_is_event(vm: &mut Vm, this: &Value, event: &Value) -> PyResult<bool> {
    let n = py::get(vm, this, "event_name")?;
    if n.as_str().is_some() {
        return ops::eq(vm, &n, event);
    }
    ops::contains(vm, &n, event)
}

fn onevent_is(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(onevent_is_event(vm, &args[0], &args[1])?))
}

fn onevent_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if onevent_is_event(vm, this, &args[1])? {
        let a = py::get_or_none(vm, this, "action")?;
        let rv = run(vm, &a, Vec::new(), Vec::new())?;
        if !py::is_none(&rv) {
            let ei = py::import_attr(vm, "renpy.display.core", "EndInteraction")?;
            let e = vm.call(&ei, vec![rv], Vec::new())?;
            return Err(crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()));
        }
    }
    Ok(Value::None)
}

fn zero_render(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(super::layout::new(vm, 0., 0.))
}

// AreaPicker ---------------------------------------------------------------------------------------

fn ap_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["rows", "cols", "position", "changed", "finished", "persist"];
    let (v, props) = split(
        args,
        kw,
        &names,
        vec![
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::Bool(false),
        ],
    );
    super::layout::container_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "rows", v[0].clone())?;
    py::set(vm, this, "cols", v[1].clone())?;
    py::set(vm, this, "rect0", Value::None)?;
    py::set(vm, this, "rect1", Value::None)?;
    py::set(vm, this, "width", Value::Int(1))?;
    py::set(vm, this, "height", Value::Int(1))?;
    for (n, i) in [
        ("position", 2),
        ("changed", 3),
        ("finished", 4),
        ("persist", 5),
    ] {
        py::set(vm, this, n, v[i].clone())?;
    }
    Ok(Value::None)
}

fn ap_round(vm: &mut Vm, this: &Value, x: f64, y: f64, current: &Value) -> PyResult<Value> {
    if x == -1. && y == -1. {
        return Ok(current.clone());
    }
    let rows = py::get_or_none(vm, this, "rows")?;
    let cols = py::get_or_none(vm, this, "cols")?;
    let (w, h) = (fget(vm, this, "width")?, fget(vm, this, "height")?);
    let (xg, gw) = if py::is_none(&rows) {
        (1., 0.)
    } else {
        let g = w / py::f64_of(vm, &rows)?;
        (g, g)
    };
    let (yg, gh) = if py::is_none(&cols) {
        (1., 0.)
    } else {
        let g = h / py::f64_of(vm, &cols)?;
        (g, g)
    };
    let mut px = xg * (x / xg).round();
    let mut pyy = yg * (y / yg).round();
    if x < px {
        px -= xg;
    }
    if y < pyy {
        pyy -= yg;
    }
    Ok(vm.types.new_tuple(vec![
        Value::Float(px),
        Value::Float(pyy),
        Value::Float(gw),
        Value::Float(gh),
    ]))
}

fn ap_round_to_grid(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &args[1])?;
    let y = py::f64_of(vm, &args[2])?;
    ap_round(vm, &args[0], x, y, &args[3])
}

fn ap_rect(vm: &mut Vm, this: &Value) -> PyResult<Option<[i64; 4]>> {
    let r0 = py::get_or_none(vm, this, "rect0")?;
    let r1 = py::get_or_none(vm, this, "rect1")?;
    if py::is_none(&r0) || py::is_none(&r1) {
        return Ok(None);
    }
    let a = render::rect4(vm, &r0)?;
    let b = render::rect4(vm, &r1)?;
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    let w = (a[0] + a[2]).max(b[0] + b[2]) - x;
    let h = (a[1] + a[3]).max(b[1] + b[3]) - y;
    Ok(Some([x as i64, y as i64, w as i64, h as i64]))
}

fn rect_value(vm: &Vm, r: Option<[i64; 4]>) -> Value {
    match r {
        Some(r) => vm
            .types
            .new_tuple(r.iter().map(|v| Value::Int(*v)).collect()),
        None => Value::None,
    }
}

fn ap_get_rect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = ap_rect(vm, &args[0])?;
    Ok(rect_value(vm, r))
}

fn ap_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, _, _) = rargs(vm, args)?;
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    let rv = super::layout::new(vm, w, h);
    vm.call_method(
        &rv,
        "add_focus",
        vec![
            this.clone(),
            Value::None,
            Value::Int(0),
            Value::Int(0),
            Value::Float(w),
            Value::Float(h),
        ],
    )?;
    if let Some(r) = ap_rect(vm, this)? {
        let c = py::get(vm, this, "child")?;
        vm.call_method(
            &rv,
            "place",
            vec![
                c,
                Value::Int(r[0]),
                Value::Int(r[1]),
                Value::Int(r[2]),
                Value::Int(r[3]),
            ],
        )?;
    }
    Ok(rv)
}

fn ap_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (ev, x, y, _) = evargs(vm, args)?;
    let old = ap_rect(vm, this)?;
    let grabbed = {
        let g = get_grab(vm)?;
        ops::truthy(vm, &g)?
    };
    let persist = {
        let v = py::get_or_none(vm, this, "persist")?;
        ops::truthy(vm, &v)?
    };
    let mut finished = false;
    let focused = vm.call_method(this, "is_focused", Vec::new())?;
    if ops::truthy(vm, &focused)? {
        let r0 = py::get_or_none(vm, this, "rect0")?;
        if map_name(vm, &ev, "drag_activate")? {
            let r = ap_round(vm, this, x, y, &r0)?;
            py::set(vm, this, "rect0", r.clone())?;
            py::set(vm, this, "rect1", r)?;
            set_grab(vm, this.clone())?;
        } else if map_name(vm, &ev, "drag_deactivate")? {
            set_grab(vm, Value::None)?;
            finished = true;
        } else if !grabbed && !persist {
            let r = ap_round(vm, this, x, y, &r0)?;
            py::set(vm, this, "rect0", r)?;
        }
        if grabbed || !persist {
            let r1 = py::get_or_none(vm, this, "rect1")?;
            let r = ap_round(vm, this, x, y, &r1)?;
            py::set(vm, this, "rect1", r)?;
        }
        let pos = py::get_or_none(vm, this, "position")?;
        let xy = vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]);
        run(vm, &pos, vec![xy], Vec::new())?;
        let rect = ap_rect(vm, this)?;
        let showing = |r: Option<[i64; 4]>| r.is_some_and(|r| r[2] != 0 && r[3] != 0);
        if rect != old {
            if showing(old) || showing(rect) {
                render::redraw(vm, this, 0.);
            }
            let ch = py::get_or_none(vm, this, "changed")?;
            let rv = rect_value(vm, rect);
            run(vm, &ch, vec![rv], Vec::new())?;
        }
        if finished && rect.is_some() {
            render::redraw(vm, this, 0.);
            let f = py::get_or_none(vm, this, "finished")?;
            let rv = rect_value(vm, rect);
            run(vm, &f, vec![rv], Vec::new())?;
            if !persist {
                let r1 = py::get_or_none(vm, this, "rect1")?;
                py::set(vm, this, "rect0", r1)?;
            }
        }
    } else {
        py::set(vm, this, "rect0", Value::None)?;
        py::set(vm, this, "rect1", Value::None)?;
        if py::same(&get_grab(vm)?, this) {
            set_grab(vm, Value::None)?;
        }
    }
    Ok(Value::None)
}

// WebInput: browser-only; outside emscripten it never produces a value, as upstream. -----------------

fn webinput_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["prompt", "default", "allow", "exclude", "mask"];
    let (v, props) = split(
        args,
        kw,
        &names,
        vec![
            Value::None,
            vm.types.new_str(""),
            Value::None,
            vm.types.new_str("{}"),
            Value::Bool(false),
        ],
    );
    super::core::d_init(vm, &[this.clone()], &props)?;
    for (i, n) in names.iter().enumerate() {
        py::set(vm, this, n, v[i].clone())?;
    }
    py::set(vm, this, "value", v[1].clone())?;
    Ok(Value::None)
}

fn webinput_pre(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = mget(vm, "WebInput")?;
    py::set(vm, &c, "active", Value::None)?;
    Ok(Value::None)
}

fn webinput_find_focusable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = mget(vm, "WebInput")?;
    py::set(vm, &c, "active", args[0].clone())?;
    Ok(Value::None)
}

fn none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

pub fn create_part2(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let d = Value::Obj(super::core::displayable_class(vm));
    let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let text = py::import_attr(vm, "renpy.text.text", "Text")?;
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let always = py::import_attr(vm, "renpy.python", "AlwaysRollback")?;
    let c = |vm: &mut Vm,
             name: &str,
             bases: &[Value],
             methods: Methods,
             attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, m, name, v.clone());
        Ok(v)
    };
    for (n, v) in [
        ("current_input_value", Value::None),
        ("input_value_active", Value::Bool(false)),
        ("default_input_value", Value::None),
    ] {
        crysirror_python::modules::set(vm, m, n, v);
    }
    let l1 = vm.types.new_list(Vec::new());
    let l2 = vm.types.new_list(Vec::new());
    crysirror_python::modules::set(vm, m, "input_values", l1);
    crysirror_python::modules::set(vm, m, "inputs", l2);
    let reg = vm.types.new_dict();
    crysirror_python::modules::set(vm, m, "adj_registered", reg);
    for (n, f) in [
        (
            "input_pre_per_interact",
            f_input_pre as crysirror_python::object::NativeFn,
        ),
        ("input_post_per_interact", f_input_post),
    ] {
        let b = vm.types.new_builtin(n, f);
        crysirror_python::modules::set(vm, m, n, b);
    }
    c(
        vm,
        "CaretBlink",
        &[d.clone()],
        &[
            ("__init__", caret_init),
            ("get_placement", caret_get_placement),
            ("visit", caret_visit),
            ("render", caret_render),
        ],
        &[],
    )?;
    let e = vm.types.new_str("");
    c(
        vm,
        "Input",
        &[text],
        &[
            ("__init__", input_init),
            ("update_text", input_update_text),
            ("set_style_prefix", input_set_style_prefix),
            ("enable", input_enable),
            ("disable", input_disable),
            ("per_interact", input_per_interact),
            ("event", input_event),
            ("render", input_render),
        ],
        &[
            ("changed", Value::None),
            ("prefix", e.clone()),
            ("suffix", e.clone()),
            ("caret_pos", Value::Int(0)),
            ("old_caret_pos", Value::Int(0)),
            ("pixel_width", Value::None),
            ("default", e.clone()),
            ("edit_text", e.clone()),
            ("value", Value::None),
            ("shown", Value::Bool(false)),
            ("multiline", Value::Bool(false)),
            ("action", Value::None),
            ("st", Value::Int(0)),
        ],
    )?;
    let adj = c(
        vm,
        "Adjustment",
        &[object],
        &[
            ("__init__", adj_init),
            ("viewport_replaces", adj_viewport_replaces),
            ("round_value", adj_round_value),
            ("get_value", adj_get_value),
            ("set_value", adj_set_value),
            ("get_range", adj_get_range),
            ("set_range", adj_set_range),
            ("get_page", adj_get_page),
            ("set_page", adj_set_page),
            ("get_step", adj_get_step),
            ("set_step", adj_set_step),
            ("register", adj_register),
            ("change", adj_change),
            ("update", adj_update),
            ("inertia_warper", adj_inertia_warper),
            ("animate", adj_animate),
            ("inertia", adj_inertia),
            ("end_animation", adj_end_animation),
            ("periodic", adj_periodic),
        ],
        &[
            ("force_step", Value::Bool(false)),
            ("animation_amplitude", Value::None),
            ("animation_target", Value::None),
            ("animation_start", Value::None),
            ("animation_delay", Value::None),
            ("animation_warper", Value::None),
            ("restart_interaction_at_limit", Value::Bool(false)),
            ("restart_interaction_at_range", Value::Bool(false)),
            ("raw_changed", Value::None),
        ],
    )?;
    for n in ["value", "range", "page", "step"] {
        let g = py::get(vm, &adj, &format!("get_{n}"))?;
        let s = py::get(vm, &adj, &format!("set_{n}"))?;
        let p = py::property(vm, g, Some(s))?;
        py::set(vm, &adj, n, p)?;
    }
    let bar = c(
        vm,
        "Bar",
        &[d.clone()],
        &[
            ("after_upgrade", bar_after_upgrade),
            ("__init__", bar_init),
            ("per_interact", bar_per_interact),
            ("visit", bar_visit),
            ("render", bar_render),
            ("focus", bar_focus),
            ("unfocus", bar_unfocus),
            ("event", bar_event),
            ("set_style_prefix", bar_set_style_prefix),
            ("_tts", empty_str),
            ("_tts_all", bar_tts_all),
        ],
        &[
            ("_store_transform_event", Value::Bool(true)),
            ("__version__", Value::Int(2)),
        ],
    )?;
    let g = vm.types.new_builtin("_draggable", bar_draggable);
    let p = py::property(vm, g, None)?;
    py::set(vm, &bar, "_draggable", p)?;
    c(
        vm,
        "Conditional",
        &[container],
        &[
            ("__init__", cond_init),
            ("render", cond_render),
            ("event", cond_event),
        ],
        &[],
    )?;
    c(
        vm,
        "TimerState",
        &[always],
        &[],
        &[("started", Value::Bool(false)), ("next_event", Value::None)],
    )?;
    c(
        vm,
        "Timer",
        &[null],
        &[
            ("after_upgrade", timer_after_upgrade),
            ("__init__", timer_init),
            ("render", timer_render),
            ("event", timer_event),
        ],
        &[
            ("__version__", Value::Int(1)),
            ("started", Value::Bool(false)),
            ("_box_skip", Value::Bool(true)),
            ("modal", Value::Bool(false)),
        ],
    )?;
    c(
        vm,
        "MouseArea",
        &[d.clone()],
        &[
            ("__init__", mousearea_init),
            ("render", mousearea_render),
            ("event", mousearea_event),
        ],
        &[("at_st_offset", Value::Int(0))],
    )?;
    c(
        vm,
        "OnEvent",
        &[d.clone()],
        &[
            ("__init__", onevent_init),
            ("is_event", onevent_is),
            ("_handles_event", onevent_is),
            ("set_transform_event", onevent_set),
            ("render", zero_render),
        ],
        &[],
    )?;
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    c(
        vm,
        "AreaPicker",
        &[container],
        &[
            ("__init__", ap_init),
            ("round_to_grid", ap_round_to_grid),
            ("get_rect", ap_get_rect),
            ("render", ap_render),
            ("event", ap_event),
        ],
        &[],
    )?;
    let wi = c(
        vm,
        "WebInput",
        &[d],
        &[
            ("__init__", webinput_init),
            ("find_focusable", webinput_find_focusable),
            ("activate", none),
            ("event", none),
            ("render", zero_render),
        ],
        &[("active", Value::None)],
    )?;
    let pre = vm.types.new_builtin("pre_find_focusable", webinput_pre);
    let post = vm.types.new_builtin("post_find_focusable", none);
    let pre = py::staticmethod(vm, pre);
    let post = py::staticmethod(vm, post);
    py::set(vm, &wi, "pre_find_focusable", pre)?;
    py::set(vm, &wi, "post_find_focusable", post)?;
    Ok(())
}
