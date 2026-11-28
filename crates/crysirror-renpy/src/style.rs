// SPDX-License-Identifier: MPL-2.0
//! The style system (`renpy/style.pyx`, `module/generate_styles.py`).
//!
//! A style has a parent (tuple name), an ordered list of property dicts and
//! an active state prefix. Building expands synthetic properties
//! (`align`, `padding`, `area`, …) and prefixed names (`hover_color`,
//! `selected_idle_background`, …) into a cache of PREFIX_COUNT ×
//! property slots with priorities; lookups walk the cache, the parent
//! (`down_parent`) and the indexed-style base (`left_parent`).

use crate::ast::NodeId;
use crate::engine::{Engine, ExecError};
use crysirror_python::{ObjRef, Payload, PyResult, Value, Vm, object::NativeFn, ops};
use std::collections::HashMap;
use std::rc::Rc;

pub const STYLE_PROPERTIES: &[&str] = &[
    "activate_sound",
    "adjust_spacing",
    "aft_bar",
    "aft_gutter",
    "alt",
    "antialias",
    "vertical",
    "background",
    "bar_invert",
    "bar_resizing",
    "unscrollable",
    "bar_vertical",
    "black_color",
    "bold",
    "bottom_margin",
    "bottom_padding",
    "box_layout",
    "box_reverse",
    "box_wrap",
    "caret",
    "child",
    "clipping",
    "color",
    "debug",
    "drop_shadow",
    "drop_shadow_color",
    "first_indent",
    "first_spacing",
    "fit_first",
    "focus_mask",
    "focus_rect",
    "font",
    "fore_bar",
    "fore_gutter",
    "foreground",
    "hinting",
    "hover_sound",
    "hyperlink_functions",
    "italic",
    "justify",
    "kerning",
    "key_events",
    "keyboard_focus",
    "language",
    "layout",
    "line_leading",
    "left_margin",
    "line_overlap_split",
    "left_padding",
    "line_spacing",
    "mouse",
    "min_width",
    "newline_indent",
    "order_reverse",
    "outlines",
    "rest_indent",
    "right_margin",
    "right_padding",
    "ruby_style",
    "size",
    "size_group",
    "slow_abortable",
    "slow_cps",
    "slow_cps_multiplier",
    "spacing",
    "strikethrough",
    "subtitle_width",
    "subpixel",
    "text_y_fudge",
    "text_align",
    "thumb",
    "thumb_offset",
    "thumb_shadow",
    "time_policy",
    "top_margin",
    "top_padding",
    "underline",
    "xanchor",
    "xfill",
    "xfit",
    "xmaximum",
    "xminimum",
    "xoffset",
    "xpos",
    "yanchor",
    "yfill",
    "yfit",
    "ymaximum",
    "yminimum",
    "yoffset",
    "ypos",
    // Later releases.
    "outline_scaling",
    "textshader",
    "emoji_font",
    "prefer_emoji",
    "shaper",
    "instance",
    "axis",
    "hyperlink_style",
    "xspacing",
    "yspacing",
    "box_align",
    "altruby_style",
    "mipmap",
    "bar_rel",
];

const DISPLAYABLE_PROPERTIES: &[&str] = &[
    "background",
    "foreground",
    "child",
    "fore_bar",
    "aft_bar",
    "thumb",
    "thumb_shadow",
    "caret",
];

/// Prefix index order: insensitive, idle, hover, selected_insensitive,
/// selected_idle, selected_hover.
pub const PREFIX_COUNT: usize = 6;
pub const PREFIX_NAMES: [&str; 6] = [
    "insensitive_",
    "idle_",
    "hover_",
    "selected_insensitive_",
    "selected_idle_",
    "selected_hover_",
];

/// (prefix, priority, target prefix indices).
fn prefix_table() -> Vec<(&'static str, i32, Vec<usize>)> {
    vec![
        ("selected_hover_", 3, vec![5]),
        ("selected_idle_", 3, vec![4]),
        ("selected_insensitive_", 3, vec![3]),
        ("selected_", 2, vec![5, 4, 3]),
        ("hover_", 1, vec![2, 5]),
        ("idle_", 1, vec![1, 4]),
        ("insensitive_", 1, vec![0, 3]),
        ("activate_", 0, vec![]),
        ("selected_activate_", 0, vec![]),
        ("", 0, vec![5, 4, 3, 1, 2, 0]),
    ]
}

/// Synthetic expansion: (target property, value transform).
#[derive(Clone, Copy)]
enum Pick {
    Same,
    Index(usize),
    /// `index_2_or_0`: element 2, or 0 for 2-tuples.
    IndexOr(usize, usize),
    Const(f64),
    True,
}

fn synthetic(name: &str) -> Option<Vec<(&'static str, Pick)>> {
    use Pick::*;
    Some(match name {
        "margin" => vec![
            ("left_margin", Index(0)),
            ("top_margin", Index(1)),
            ("right_margin", IndexOr(2, 0)),
            ("bottom_margin", IndexOr(3, 1)),
        ],
        "xmargin" => vec![("left_margin", Same), ("right_margin", Same)],
        "ymargin" => vec![("top_margin", Same), ("bottom_margin", Same)],
        "xalign" => vec![("xpos", Same), ("xanchor", Same)],
        "yalign" => vec![("ypos", Same), ("yanchor", Same)],
        "padding" => vec![
            ("left_padding", Index(0)),
            ("top_padding", Index(1)),
            ("right_padding", IndexOr(2, 0)),
            ("bottom_padding", IndexOr(3, 1)),
        ],
        "xpadding" => vec![("left_padding", Same), ("right_padding", Same)],
        "ypadding" => vec![("top_padding", Same), ("bottom_padding", Same)],
        "minwidth" => vec![("min_width", Same)],
        "textalign" => vec![("text_align", Same)],
        "slow_speed" => vec![("slow_cps", Same)],
        "enable_hover" => vec![],
        "left_gutter" | "top_gutter" => vec![("fore_gutter", Same)],
        "right_gutter" | "bottom_gutter" => vec![("aft_gutter", Same)],
        "left_bar" | "top_bar" => vec![("fore_bar", Same)],
        "right_bar" | "bottom_bar" => vec![("aft_bar", Same)],
        "base_bar" => vec![("fore_bar", Same), ("aft_bar", Same)],
        "box_spacing" => vec![("spacing", Same)],
        "box_first_spacing" => vec![("first_spacing", Same)],
        "pos" => vec![("xpos", Index(0)), ("ypos", Index(1))],
        "anchor" => vec![("xanchor", Index(0)), ("yanchor", Index(1))],
        "offset" => vec![("xoffset", Index(0)), ("yoffset", Index(1))],
        "align" => vec![
            ("xpos", Index(0)),
            ("ypos", Index(1)),
            ("xanchor", Index(0)),
            ("yanchor", Index(1)),
        ],
        "maximum" => vec![("xmaximum", Index(0)), ("ymaximum", Index(1))],
        "minimum" => vec![("xminimum", Index(0)), ("yminimum", Index(1))],
        "xsize" => vec![("xminimum", Same), ("xmaximum", Same)],
        "ysize" => vec![("yminimum", Same), ("ymaximum", Same)],
        "xysize" => vec![
            ("xminimum", Index(0)),
            ("xmaximum", Index(0)),
            ("yminimum", Index(1)),
            ("ymaximum", Index(1)),
        ],
        "area" => vec![
            ("xpos", Index(0)),
            ("ypos", Index(1)),
            ("xanchor", Const(0.0)),
            ("yanchor", Const(0.0)),
            ("xfill", True),
            ("yfill", True),
            ("xmaximum", Index(2)),
            ("ymaximum", Index(3)),
            ("xminimum", Index(2)),
            ("yminimum", Index(3)),
        ],
        "xcenter" => vec![("xpos", Same), ("xanchor", Const(0.5))],
        "ycenter" => vec![("ypos", Same), ("yanchor", Const(0.5))],
        "spacing" | "xspacing" | "yspacing" => return None,
        _ => return None,
    })
}

pub fn prop_index(name: &str) -> Option<usize> {
    STYLE_PROPERTIES.iter().position(|p| *p == name)
}

/// Splits a prefixed property name into (prefix, base).
pub fn split_prefix(name: &str) -> Option<(&'static str, &str)> {
    for (p, _, _) in prefix_table().iter().filter(|(p, _, _)| !p.is_empty()) {
        if let Some(rest) = name.strip_prefix(p) {
            if prop_index(rest).is_some() || synthetic(rest).is_some() {
                return Some((p, rest));
            }
        }
    }
    if prop_index(name).is_some() || synthetic(name).is_some() {
        return Some(("", name));
    }
    None
}

pub fn is_known(name: &str) -> bool {
    split_prefix(name).is_some()
}

pub struct StyleData {
    pub name: Option<Vec<String>>,
    pub parent: Option<Vec<String>>,
    pub properties: Vec<Vec<(String, Value)>>,
    pub prefix: usize,
    pub help: Value,
    cache: Option<Rc<Vec<Option<Value>>>>,
    built_version: u64,
}

impl Default for StyleData {
    fn default() -> Self {
        Self {
            name: None,
            parent: None,
            properties: Vec::new(),
            prefix: 0,
            help: Value::None,
            cache: None,
            built_version: 0,
        }
    }
}

struct Styles {
    values: HashMap<Vec<String>, Value>,
    version: u64,
    class: ObjRef,
}

fn styles(vm: &Vm) -> std::cell::RefMut<'_, Styles> {
    vm.ext_get::<Styles>()
        .expect("Ren'Py style module is not initialized")
}

fn bump(vm: &Vm) {
    styles(vm).version += 1;
}

pub fn with<R>(v: &Value, f: impl FnOnce(&mut StyleData) -> R) -> Option<R> {
    match v.payload() {
        Some(Payload::Native(n)) => n.borrow_mut().downcast_mut::<StyleData>().map(f),
        _ => None,
    }
}

pub fn is_style(v: &Value) -> bool {
    with(v, |_| ()).is_some()
}

pub fn new_style(vm: &mut Vm, parent: Option<Vec<String>>, name: Option<Vec<String>>) -> Value {
    let data = StyleData {
        name,
        parent,
        prefix: 0,
        help: Value::None,
        ..Default::default()
    };
    let class = styles(vm).class.clone();
    let v = vm.types.new_native(&class, Box::new(data));
    bump(vm);
    v
}

pub fn tuple_name(vm: &mut Vm, v: &Value) -> PyResult<Option<Vec<String>>> {
    if v.is_none() {
        return Ok(None);
    }
    if let Some(n) = with(v, |s| s.name.clone()) {
        return Ok(n);
    }
    if let Some(s) = v.as_str() {
        return Ok(Some(vec![s.to_string()]));
    }
    let items = ops::collect(vm, v)?;
    let mut out = Vec::new();
    for i in items {
        out.push(ops::str(vm, &i)?);
    }
    Ok(Some(out))
}

fn lookup(vm: &Vm, name: &[String]) -> Option<Value> {
    styles(vm).values.get(name).cloned()
}

fn register(vm: &Vm, name: Vec<String>, v: Value) {
    styles(vm).values.insert(name, v);
    bump(vm);
}

/// `get_style`: missing `a_b` styles inherit from `b`.
pub fn get_style(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let key = vec![name.to_string()];
    if let Some(v) = lookup(vm, &key) {
        return Ok(v);
    }
    let (start, end) = match name.split_once('_') {
        Some((a, b)) if !b.is_empty() => (a, b.to_string()),
        _ => return Err(vm.exc_msg("Exception", format!("Style {name:?} does not exist."))),
    };
    let end = if start.is_empty() {
        match name[1..].split_once('_') {
            Some((_, b)) if !b.is_empty() => format!("_{b}"),
            _ => return Err(vm.exc_msg("Exception", format!("Style {name:?} does not exist."))),
        }
    } else {
        end
    };
    let parent = get_style(vm, &end)
        .map_err(|_| vm.exc_msg("Exception", format!("Style {name:?} does not exist.")))?;
    let pn = with(&parent, |s| s.name.clone()).flatten();
    let rv = new_style(vm, pn, Some(key.clone()));
    register(vm, key, rv.clone());
    Ok(rv)
}

pub fn get_or_create_style(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let key = vec![name.to_string()];
    if let Some(v) = lookup(vm, &key) {
        return Ok(v);
    }
    let parent = match name.split_once('_') {
        Some((a, b)) if !a.is_empty() && !b.is_empty() => {
            let p = get_or_create_style(vm, b)?;
            with(&p, |s| s.name.clone()).flatten()
        }
        _ if name == "default" => None,
        _ => Some(vec!["default".to_string()]),
    };
    let rv = new_style(vm, parent, Some(key.clone()));
    register(vm, key, rv.clone());
    Ok(rv)
}

pub fn get_full_style(vm: &mut Vm, name: &[String]) -> PyResult<Value> {
    if let Some(v) = lookup(vm, name) {
        return Ok(v);
    }
    let mut rv = get_style(vm, &name[0])?;
    for i in &name[1..] {
        rv = index_style(vm, &rv, i)?;
    }
    Ok(rv)
}

fn index_style(vm: &mut Vm, st: &Value, key: &str) -> PyResult<Value> {
    let (name, parent) = with(st, |s| (s.name.clone(), s.parent.clone())).unwrap_or((None, None));
    let mut tname = name.unwrap_or_default();
    tname.push(key.to_string());
    if let Some(v) = lookup(vm, &tname) {
        return Ok(v);
    }
    let parent = parent.map(|mut p| {
        p.push(key.to_string());
        p
    });
    let rv = new_style(vm, parent, Some(tname.clone()));
    register(vm, tname, rv.clone());
    Ok(rv)
}

/// Converts a raw property value as the generated property functions do.
fn convert(vm: &mut Vm, prop: &str, prefix: &str, v: &Value) -> PyResult<Value> {
    let easy = |vm: &mut Vm, f: &str, v: Value| -> PyResult<Value> {
        let m = crysirror_python::import::import_module(vm, "renpy.easy")?;
        let func = ops::getattr(vm, &m, f)?;
        vm.call(&func, vec![v], Vec::new())
    };
    match prop {
        "xanchor" | "yanchor" => Ok(match v.as_str() {
            Some("left") | Some("top") => Value::Float(0.0),
            Some("right") | Some("bottom") => Value::Float(1.0),
            Some("center") => Value::Float(0.5),
            _ => v.clone(),
        }),
        "color" | "black_color" | "drop_shadow_color" => {
            if v.is_none() {
                return Ok(Value::None);
            }
            easy(vm, "color", v.clone())
        }
        "aft_bar" | "fore_bar" | "thumb" | "thumb_shadow" => {
            let v = substitute_prefix(vm, v, prefix);
            easy(vm, "none_is_null", v)
        }
        "background" | "foreground" | "child" | "caret" => {
            let v = substitute_prefix(vm, v, prefix);
            easy(vm, "displayable_or_none", v)
        }
        "focus_mask" => match v {
            Value::None | Value::Bool(_) => Ok(v.clone()),
            _ if crysirror_python::builtins::is_callable(vm, v) && v.as_str().is_none() => {
                Ok(v.clone())
            }
            _ => easy(vm, "displayable", v.clone()),
        },
        "outlines" => {
            let items = ops::collect(vm, v)?;
            let mut out = Vec::new();
            for i in items {
                let p = ops::collect(vm, &i)?;
                let c = easy(vm, "color", p.get(1).cloned().unwrap_or(Value::None))?;
                out.push(vm.types.new_tuple(vec![
                    p[0].clone(),
                    c,
                    p.get(2).cloned().unwrap_or(Value::Int(0)),
                    p.get(3).cloned().unwrap_or(Value::Int(0)),
                ]));
            }
            Ok(vm.types.new_list(out))
        }
        _ => Ok(v.clone()),
    }
}

/// `[prefix_]` in image names selects the state's image (`assign_prefixed`).
fn substitute_prefix(vm: &mut Vm, v: &Value, prefix: &str) -> Value {
    match v.as_str() {
        Some(s) if s.contains("[prefix_]") => {
            let p = if prefix.is_empty() { "idle_" } else { prefix };
            vm.types.new_string(s.replace("[prefix_]", p))
        }
        _ => v.clone(),
    }
}

fn pick(vm: &mut Vm, v: &Value, p: Pick) -> PyResult<Value> {
    Ok(match p {
        Pick::Same => v.clone(),
        Pick::Index(i) => ops::getitem(vm, v, Value::Int(i as i64))?,
        Pick::IndexOr(i, j) => {
            let n = ops::len(vm, v)?;
            ops::getitem(vm, v, Value::Int(if n > i { i } else { j } as i64))?
        }
        Pick::Const(f) => Value::Float(f),
        Pick::True => Value::Bool(true),
    })
}

fn build(vm: &mut Vm, st: &Value) -> PyResult<Rc<Vec<Option<Value>>>> {
    let ver = styles(vm).version;
    if let Some(Some(c)) = with(st, |s| {
        if s.built_version == ver {
            s.cache.clone()
        } else {
            None
        }
    }) {
        return Ok(c);
    }
    let props = with(st, |s| s.properties.clone()).unwrap_or_default();
    let n = STYLE_PROPERTIES.len();
    let mut cache: Vec<Option<Value>> = vec![None; n * PREFIX_COUNT];
    let mut prio = vec![0i32; n * PREFIX_COUNT];
    let table = prefix_table();
    let mut priority = 1;
    for d in &props {
        for (k, v) in d {
            let Some((pfx, base)) = split_prefix(k) else {
                continue;
            };
            let (_, ppri, alts) = table.iter().find(|(p, _, _)| *p == pfx).unwrap();
            let targets: Vec<(&str, Pick)> = match synthetic(base) {
                Some(t) => t,
                None => vec![(STYLE_PROPERTIES[prop_index(base).unwrap()], Pick::Same)],
            };
            let pr = priority + ppri;
            for (tprop, pk) in targets {
                let raw = pick(vm, v, pk)?;
                let idx = prop_index(tprop).unwrap();
                for alt in alts {
                    let cv = convert(vm, tprop, PREFIX_NAMES[*alt], &raw)?;
                    let slot = alt * n + idx;
                    if pr >= prio[slot] {
                        prio[slot] = pr;
                        cache[slot] = Some(cv);
                    }
                }
            }
        }
        priority += 4;
    }
    let rc = Rc::new(cache);
    with(st, |s| {
        s.cache = Some(rc.clone());
        s.built_version = ver;
    });
    Ok(rc)
}

/// Resolves property `idx` for prefix slot `prefix` through the hierarchy.
pub fn get_prop(vm: &mut Vm, st: &Value, idx: usize, prefix: usize) -> PyResult<Value> {
    let n = STYLE_PROPERTIES.len();
    let mut s = st.clone();
    let mut left: Option<Value> = None;
    for _ in 0..100 {
        let cache = build(vm, &s)?;
        if let Some(Some(v)) = cache.get(prefix * n + idx) {
            return Ok(v.clone());
        }
        let (parent, name) =
            with(&s, |d| (d.parent.clone(), d.name.clone())).unwrap_or((None, None));
        if left.is_none() {
            if let Some(nm) = name.filter(|n| n.len() > 1) {
                left = Some(get_full_style(vm, &nm[..nm.len() - 1])?);
            }
        }
        let down = match parent {
            Some(p) => Some(get_full_style(vm, &p)?),
            None => None,
        };
        match down.or_else(|| left.take()) {
            Some(d) => s = d,
            None => return Ok(Value::None),
        }
    }
    Err(vm.exc_msg(
        "Exception",
        "style is too complex (loop in style inheritance)",
    ))
}

pub fn get(vm: &mut Vm, st: &Value, name: &str) -> PyResult<Value> {
    let prefix = with(st, |s| s.prefix).unwrap_or(0);
    match prop_index(name) {
        Some(i) => get_prop(vm, st, i, prefix),
        None => Err(vm.attr_error(format!("style property {name} is not known"))),
    }
}

// ---- Python class ----

fn arg_style(vm: &Vm, v: &Value) -> PyResult<()> {
    if is_style(v) {
        Ok(())
    } else {
        Err(vm.type_error("Style expected"))
    }
}

fn s_new(vm: &mut Vm, a: &[Value], k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let [parent, properties, name, help, _heavy] = crysirror_python::builtins::parse(
        vm,
        "Style",
        &a[1..],
        k,
        ["parent", "properties", "name", "help", "heavy"],
        0,
    )?;
    let parent = match parent {
        Some(p) => tuple_name(vm, &p)?,
        None => None,
    };
    let name = match name {
        Some(n) if !n.is_none() => tuple_name(vm, &n)?,
        _ => None,
    };
    let v = new_style(vm, parent, name);
    if let Some(p) = properties.filter(|p| !p.is_none()) {
        let items = ops::mapping_items(vm, &p)?;
        let props = items
            .into_iter()
            .map(|(k, v)| (k.as_str().unwrap_or("").to_string(), v))
            .collect();
        with(&v, |s| s.properties.push(props));
    }
    if let Some(h) = help {
        with(&v, |s| s.help = h);
    }
    Ok(v)
}

fn s_getattr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    arg_style(vm, &a[0])?;
    let name = a[1].as_str().unwrap_or("").to_string();
    match name.as_str() {
        "name" => return Ok(name_value(vm, with(&a[0], |s| s.name.clone()).flatten())),
        "parent" => return Ok(name_value(vm, with(&a[0], |s| s.parent.clone()).flatten())),
        "prefix" => {
            let p = with(&a[0], |s| s.prefix).unwrap_or(0);
            return Ok(vm.types.new_str(PREFIX_NAMES[p]));
        }
        "properties" => {
            let props = with(&a[0], |s| s.properties.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for d in props {
                let dd = vm.types.new_dict();
                for (k, v) in d {
                    let kv = vm.types.new_string(k);
                    ops::setitem(vm, &dd, kv, v)?;
                }
                out.push(dd);
            }
            return Ok(vm.types.new_list(out));
        }
        "help" => return Ok(with(&a[0], |s| s.help.clone()).unwrap_or(Value::None)),
        _ => {}
    }
    if let Some((pfx, base)) = split_prefix(&name) {
        if let Some(i) = prop_index(base) {
            let slot = if pfx.is_empty() {
                with(&a[0], |s| s.prefix).unwrap_or(0)
            } else {
                PREFIX_NAMES.iter().position(|p| *p == pfx).unwrap_or(0)
            };
            return get_prop(vm, &a[0], i, slot);
        }
    }
    Err(vm.attr_error(format!("'Style' object has no attribute '{name}'")))
}

fn name_value(vm: &mut Vm, n: Option<Vec<String>>) -> Value {
    match n {
        Some(v) => {
            let items = v.iter().map(|s| vm.types.new_str(s)).collect();
            vm.types.new_tuple(items)
        }
        None => Value::None,
    }
}

fn s_setattr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    arg_style(vm, &a[0])?;
    let name = a[1].as_str().unwrap_or("").to_string();
    if !is_known(&name) {
        return Err(vm.exc_msg("Exception", format!("Style property {name} is not known.")));
    }
    with(&a[0], |s| s.properties.push(vec![(name, a[2].clone())]));
    bump(vm);
    Ok(Value::None)
}

fn s_delattr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let name = a[1].as_str().unwrap_or("").to_string();
    with(&a[0], |s| {
        for d in s.properties.iter_mut() {
            d.retain(|(k, _)| *k != name);
        }
    });
    bump(vm);
    let _ = vm;
    Ok(Value::None)
}

fn s_getitem(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let key = ops::str(vm, &a[1])?;
    index_style(vm, &a[0], &key)
}

fn s_set_parent(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let p = tuple_name(vm, &a[1])?;
    with(&a[0], |s| s.parent = p);
    bump(vm);
    Ok(Value::None)
}

fn s_clear(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    with(&a[0], |s| s.properties.clear());
    bump(vm);
    Ok(Value::None)
}

fn s_take(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let other = if is_style(&a[1]) {
        a[1].clone()
    } else {
        {
            let name = ops::str(vm, &a[1])?;
            get_style(vm, &name)?
        }
    };
    let props = with(&other, |s| s.properties.clone()).unwrap_or_default();
    with(&a[0], |s| s.properties = props);
    bump(vm);
    Ok(Value::None)
}

fn s_setdefault(vm: &mut Vm, a: &[Value], k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let set: Vec<String> = with(&a[0], |s| {
        s.properties
            .iter()
            .flat_map(|d| d.iter().map(|(k, _)| k.clone()))
            .collect()
    })
    .unwrap_or_default();
    let add: Vec<(String, Value)> = k
        .iter()
        .filter(|(n, _)| !set.iter().any(|s| s == &**n))
        .map(|(n, v)| (n.to_string(), v.clone()))
        .collect();
    if !add.is_empty() {
        with(&a[0], |s| s.properties.push(add));
        bump(vm);
    }
    let _ = vm;
    Ok(Value::None)
}

fn s_add_properties(
    vm: &mut Vm,
    a: &[Value],
    _k: crysirror_python::builtins::Kw,
) -> PyResult<Value> {
    let items = ops::mapping_items(vm, &a[1])?;
    let props = items
        .into_iter()
        .map(|(k, v)| (k.as_str().unwrap_or("").to_string(), v))
        .collect();
    with(&a[0], |s| s.properties.push(props));
    bump(vm);
    Ok(Value::None)
}

fn s_set_prefix(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let p = ops::str(vm, &a[1])?;
    if let Some(i) = PREFIX_NAMES.iter().position(|x| *x == p) {
        with(&a[0], |s| s.prefix = i);
    }
    Ok(Value::None)
}

fn s_copy(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let (parent, props) =
        with(&a[0], |s| (s.parent.clone(), s.properties.clone())).unwrap_or((None, Vec::new()));
    let v = new_style(vm, parent, None);
    with(&v, |s| s.properties = props);
    Ok(v)
}

fn s_get_placement(
    vm: &mut Vm,
    a: &[Value],
    _k: crysirror_python::builtins::Kw,
) -> PyResult<Value> {
    let mut out = Vec::new();
    for n in [
        "xpos", "ypos", "xanchor", "yanchor", "xoffset", "yoffset", "subpixel",
    ] {
        out.push(get(vm, &a[0], n)?);
    }
    Ok(vm.types.new_tuple(out))
}

fn s_repr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = with(&a[0], |s| s.name.clone()).flatten();
    let s = match n {
        Some(n) => format!("<style {}>", n.join(".")),
        None => "<anonymous style>".into(),
    };
    Ok(vm.types.new_string(s))
}

fn s_eq(_vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    Ok(Value::Bool(a[0].is(&a[1])))
}

fn s_getstate(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let d = vm.types.new_dict();
    for n in ["properties", "prefix", "name", "parent"] {
        let nv = vm.types.new_str(n);
        let v = s_getattr(vm, &[a[0].clone(), nv.clone()], &[])?;
        ops::setitem(vm, &d, nv, v)?;
    }
    Ok(d)
}

// StyleManager (`style` in the store).
fn m_getattr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = ops::str(vm, &a[1])?;
    get_style(vm, &n)
}

fn m_setattr(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    if !is_style(&a[2]) {
        return Err(vm.exc_msg("Exception", "Value is not a style."));
    }
    let n = ops::str(vm, &a[1])?;
    let key = vec![n];
    with(&a[2], |s| {
        if s.name.is_none() {
            s.name = Some(key.clone());
        }
    });
    register(vm, key, a[2].clone());
    Ok(Value::None)
}

fn m_create(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = ops::str(vm, &a[1])?;
    let p = tuple_name(vm, &a[2])?;
    let v = new_style(vm, p, Some(vec![n.clone()]));
    register(vm, vec![n], v);
    Ok(Value::None)
}

fn m_rebuild(vm: &mut Vm, _a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    bump(vm);
    Ok(Value::None)
}

fn m_exists(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = tuple_name(vm, &a[1])?.unwrap_or_default();
    Ok(Value::Bool(lookup(vm, &n).is_some()))
}

fn m_get(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = tuple_name(vm, &a[1])?.unwrap_or_default();
    get_full_style(vm, &n)
}

fn f_get_style(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = ops::str(vm, &a[0])?;
    get_style(vm, &n)
}

fn f_get_or_create(
    vm: &mut Vm,
    a: &[Value],
    _k: crysirror_python::builtins::Kw,
) -> PyResult<Value> {
    let n = ops::str(vm, &a[0])?;
    get_or_create_style(vm, &n)
}

fn f_get_full(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = tuple_name(vm, &a[0])?.unwrap_or_default();
    get_full_style(vm, &n)
}

fn f_get_text_style(
    vm: &mut Vm,
    a: &[Value],
    _k: crysirror_python::builtins::Kw,
) -> PyResult<Value> {
    let st = if a[0].is_none() {
        a[1].clone()
    } else {
        a[0].clone()
    };
    let n = tuple_name(vm, &st)?.unwrap_or_default();
    let mut rv = get_full_style(vm, &[format!("{}_text", n[0])])?;
    for i in &n[1..] {
        rv = index_style(vm, &rv, i)?;
    }
    Ok(rv)
}

fn f_is_style(_vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    Ok(Value::Bool(is_style(&a[0])))
}

fn f_known(vm: &mut Vm, a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let n = ops::str(vm, &a[0])?;
    Ok(Value::Bool(is_known(&n)))
}

fn f_reset(vm: &mut Vm, _a: &[Value], _k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    styles(vm).values.clear();
    bump(vm);
    Ok(Value::None)
}

/// Installs the native `_renpy_style` module.
pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        "_renpy_style",
        &[
            ("get_style", f_get_style),
            ("get_or_create_style", f_get_or_create),
            ("get_full_style", f_get_full),
            ("get_text_style", f_get_text_style),
            ("is_style", f_is_style),
            ("is_known_property", f_known),
            ("reset", f_reset),
        ],
    );
    let c = vm.types.new_builtin_type(
        "Style",
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Native,
        false,
    );
    let n = vm.types.new_builtin("__new__", s_new);
    let n = Value::Obj(crysirror_python::object::new_obj(
        vm.types.staticmethod.clone(),
        Some(Default::default()),
        Payload::StaticMethod(n),
    ));
    vm.types.set_attr(&c, "__new__", n);
    let methods: &[(&'static str, NativeFn)] = &[
        ("__init__", |_vm, _a, _k| Ok(Value::None)),
        ("__getattr__", s_getattr),
        ("__setattr__", s_setattr),
        ("__delattr__", s_delattr),
        ("__getitem__", s_getitem),
        ("setattr", s_setattr_m),
        ("delattr", s_delattr),
        ("set_parent", s_set_parent),
        ("clear", s_clear),
        ("take", s_take),
        ("setdefault", s_setdefault),
        ("add_properties", s_add_properties),
        ("set_prefix", s_set_prefix),
        ("copy", s_copy),
        ("get_placement", s_get_placement),
        ("__repr__", s_repr),
        ("__eq__", s_eq),
        ("__hash__", |_vm, a, _k| Ok(Value::Int(a[0].id()))),
        ("__getstate__", s_getstate),
    ];
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str("renpy.style");
    vm.types.set_attr(&c, "__module__", mn);
    vm.ext_set(Styles {
        values: HashMap::new(),
        version: 1,
        class: c.clone(),
    });
    crysirror_python::modules::set(vm, &m, "Style", Value::Obj(c));
    let mc = vm.types.new_builtin_type(
        "StyleManager",
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Object,
        false,
    );
    for (n, f) in [
        ("__getattr__", m_getattr as NativeFn),
        ("__setattr__", m_setattr),
        ("__getitem__", m_getattr),
        ("__setitem__", m_setattr),
        ("create", m_create),
        ("rebuild", m_rebuild),
        ("exists", m_exists),
        ("get", m_get),
    ] {
        vm.types.add_method(&mc, n, f);
    }
    crysirror_python::modules::set(vm, &m, "StyleManager", Value::Obj(mc));
    Ok(m)
}

fn s_setattr_m(vm: &mut Vm, a: &[Value], k: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let name = a[1].as_str().unwrap_or("").to_string();
    with(&a[0], |s| s.properties.push(vec![(name, a[2].clone())]));
    bump(vm);
    let _ = (vm, k);
    Ok(Value::None)
}

/// The `style` statement (`renpy.ast.Style.execute`).
pub fn apply_statement(
    e: &mut Engine,
    id: NodeId,
    st: &crate::ast::Style,
) -> Result<(), ExecError> {
    let vm = &mut e.vm;
    let s = get_or_create_style(vm, &st.name)?;
    if let Some(v) = &st.variant {
        let ok = e.eval_cached(id, 50, v)?;
        if !ops::truthy(&mut e.vm, &ok)? {
            return Ok(());
        }
    }
    let vm = &mut e.vm;
    if st.clear {
        with(&s, |d| d.properties.clear());
    }
    if let Some(p) = &st.parent {
        let pn = vec![p.clone()];
        with(&s, |d| d.parent = Some(pn));
    }
    if let Some(t) = &st.take {
        let other = get_style(vm, t)?;
        let props = with(&other, |d| d.properties.clone()).unwrap_or_default();
        with(&s, |d| d.properties = props);
    }
    for name in &st.delattr {
        with(&s, |d| {
            for p in d.properties.iter_mut() {
                p.retain(|(k, _)| k != name);
            }
        });
    }
    let mut props = Vec::new();
    for (i, (name, expr)) in st.properties.iter().enumerate() {
        let v = e.eval_cached(id, i as u8, expr)?;
        if name == "properties" {
            for (k, v) in ops::mapping_items(&mut e.vm, &v)? {
                props.push((ops::str(&mut e.vm, &k)?, v));
            }
        } else {
            props.push((name.clone(), v));
        }
    }
    if !props.is_empty() {
        with(&s, |d| d.properties.push(props));
    }
    bump(&e.vm);
    Ok(())
}
