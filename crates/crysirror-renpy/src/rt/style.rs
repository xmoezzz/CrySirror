// SPDX-License-Identifier: MPL-2.0
//! `renpy.style` and `renpy.styledata.*`, after Ren'Py 8.3 `style.pyx`,
//! `styledata/styleutil.py` and the tables of `module/generate_styles.py`
//! (MIT; 8.6 moved the generator to `scripts/`).
//!
//! The property table is the union over 6.18–8.6: releases only add
//! properties, and a game never names one its release lacks. The one
//! value difference is gated on the release: `xminimum`/`yminimum` map
//! `None` to 0 from 7.4.
//!
//! A style is a Python object (subclassable, picklable through
//! `__getstate__`) whose build cache — PREFIX_COUNT × property slots with
//! priorities, the parent and the indexed-style base — is a native value.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::OnceLock};

const MODULE: &str = "renpy.style";
const CORE: &str = "_crysirror_core";
const PRIORITY_LEVELS: i32 = 8;
pub const PREFIX_COUNT: usize = 6;

/// Value conversion of a style property (`styleutil` / `renpy.easy`).
#[derive(Clone, Copy, PartialEq)]
enum Conv {
    None,
    NoneIsNull,
    Displayable,
    Color,
    FocusMask,
    Outlines,
    Anchor,
    NoneIs0,
}

use Conv as C;

const STYLE_PROPERTIES: &[(&str, Conv)] = &[
    ("activate_sound", C::None),
    ("adjust_spacing", C::None),
    ("aft_bar", C::NoneIsNull),
    ("aft_gutter", C::None),
    ("alt", C::None),
    ("altruby_style", C::None),
    ("antialias", C::None),
    ("axis", C::None),
    ("background", C::Displayable),
    ("bar_invert", C::None),
    ("bar_resizing", C::None),
    ("bar_vertical", C::None),
    ("black_color", C::Color),
    ("bold", C::None),
    ("bottom_margin", C::None),
    ("bottom_padding", C::None),
    ("box_align", C::None),
    ("box_justify", C::None),
    ("box_layout", C::None),
    ("box_reverse", C::None),
    ("box_wrap", C::None),
    ("box_wrap_spacing", C::None),
    ("caret", C::Displayable),
    ("child", C::Displayable),
    ("clipping", C::None),
    ("color", C::Color),
    ("debug", C::None),
    ("drop_shadow", C::None),
    ("drop_shadow_color", C::Color),
    ("emoji_font", C::None),
    ("extra_alt", C::None),
    ("first_indent", C::None),
    ("first_spacing", C::None),
    ("fit_first", C::None),
    ("focus_mask", C::FocusMask),
    ("focus_rect", C::None),
    ("font", C::None),
    ("font_features", C::None),
    ("fore_bar", C::NoneIsNull),
    ("fore_gutter", C::None),
    ("foreground", C::Displayable),
    ("group_alt", C::None),
    ("hinting", C::None),
    ("hover_sound", C::None),
    ("hyperlink_functions", C::None),
    ("instance", C::None),
    ("italic", C::None),
    ("justify", C::None),
    ("kerning", C::None),
    ("key_events", C::None),
    ("keyboard_focus", C::None),
    ("keyboard_focus_insets", C::None),
    ("language", C::None),
    ("layout", C::None),
    ("left_margin", C::None),
    ("left_padding", C::None),
    ("line_leading", C::None),
    ("line_overlap_split", C::None),
    ("line_spacing", C::None),
    ("min_width", C::None),
    ("mipmap", C::None),
    ("modal", C::None),
    ("mouse", C::None),
    ("newline_indent", C::None),
    ("order_reverse", C::None),
    ("outline_scaling", C::None),
    ("outlines", C::Outlines),
    ("prefer_emoji", C::None),
    ("reading_order", C::None),
    ("rest_indent", C::None),
    ("right_margin", C::None),
    ("right_padding", C::None),
    ("ruby_line_leading", C::None),
    ("ruby_style", C::None),
    ("shaper", C::None),
    ("size", C::None),
    ("size_group", C::None),
    ("slow_abortable", C::None),
    ("slow_cps", C::None),
    ("slow_cps_multiplier", C::None),
    ("spacing", C::None),
    ("strikethrough", C::None),
    ("subpixel", C::None),
    ("subtitle_width", C::None),
    ("text_align", C::None),
    ("text_y_fudge", C::None),
    ("textshader", C::None),
    ("thumb", C::NoneIsNull),
    ("thumb_align", C::None),
    ("thumb_offset", C::None),
    ("thumb_shadow", C::NoneIsNull),
    ("time_policy", C::None),
    ("top_margin", C::None),
    ("top_padding", C::None),
    ("underline", C::None),
    ("unscrollable", C::None),
    ("vertical", C::None),
    ("xanchor", C::Anchor),
    ("xfill", C::None),
    ("xfit", C::None),
    ("xmaximum", C::None),
    ("xminimum", C::NoneIs0),
    ("xoffset", C::None),
    ("xpos", C::None),
    ("xspacing", C::None),
    ("yanchor", C::Anchor),
    ("yfill", C::None),
    ("yfit", C::None),
    ("ymaximum", C::None),
    ("yminimum", C::NoneIs0),
    ("yoffset", C::None),
    ("ypos", C::None),
    ("yspacing", C::None),
];

const DISPLAYABLE_PROPERTIES: &[&str] = &[
    "background",
    "foreground",
    "child",
    "fore_bar",
    "aft_bar",
    "thumb",
    "thumb_shadow",
];

const PROPERTY_PRIORITY: &[&str] = &[
    "left_margin",
    "top_margin",
    "right_margin",
    "bottom_margin",
    "xpos",
    "xanchor",
    "ypos",
    "yanchor",
    "left_padding",
    "top_padding",
    "right_padding",
    "bottom_padding",
    "xoffset",
    "yoffset",
    "xminimum",
    "yminimum",
    "xmaximum",
    "ymaximum",
    "xfill",
    "yfill",
];

/// How a synthetic property derives each target's value.
#[derive(Clone, Copy)]
enum Pick {
    Same,
    Index(usize),
    /// `index_2_or_0` / `index_3_or_1`.
    IndexOr(usize, usize),
    Int(i64),
    Float(f64),
    True,
}

use Pick as P;

const SYNTHETIC: &[(&str, &[(&str, Pick)])] = &[
    (
        "margin",
        &[
            ("left_margin", P::Index(0)),
            ("top_margin", P::Index(1)),
            ("right_margin", P::IndexOr(2, 0)),
            ("bottom_margin", P::IndexOr(3, 1)),
        ],
    ),
    (
        "xmargin",
        &[("left_margin", P::Same), ("right_margin", P::Same)],
    ),
    (
        "ymargin",
        &[("top_margin", P::Same), ("bottom_margin", P::Same)],
    ),
    ("xalign", &[("xpos", P::Same), ("xanchor", P::Same)]),
    ("yalign", &[("ypos", P::Same), ("yanchor", P::Same)]),
    (
        "padding",
        &[
            ("left_padding", P::Index(0)),
            ("top_padding", P::Index(1)),
            ("right_padding", P::IndexOr(2, 0)),
            ("bottom_padding", P::IndexOr(3, 1)),
        ],
    ),
    (
        "xpadding",
        &[("left_padding", P::Same), ("right_padding", P::Same)],
    ),
    (
        "ypadding",
        &[("top_padding", P::Same), ("bottom_padding", P::Same)],
    ),
    ("minwidth", &[("min_width", P::Same)]),
    // Through 6.16 `sound` is the base property whose hover_ and activate_
    // forms the button behaviour plays (6.17 made those two properties).
    ("sound", &[("hover_sound", P::Same), ("activate_sound", P::Same)]),
    ("textalign", &[("text_align", P::Same)]),
    ("slow_speed", &[("slow_cps", P::Same)]),
    ("enable_hover", &[]),
    ("left_gutter", &[("fore_gutter", P::Same)]),
    ("right_gutter", &[("aft_gutter", P::Same)]),
    ("top_gutter", &[("fore_gutter", P::Same)]),
    ("bottom_gutter", &[("aft_gutter", P::Same)]),
    ("left_bar", &[("fore_bar", P::Same)]),
    ("right_bar", &[("aft_bar", P::Same)]),
    ("top_bar", &[("fore_bar", P::Same)]),
    ("bottom_bar", &[("aft_bar", P::Same)]),
    ("base_bar", &[("fore_bar", P::Same), ("aft_bar", P::Same)]),
    ("box_spacing", &[("spacing", P::Same)]),
    ("box_first_spacing", &[("first_spacing", P::Same)]),
    ("pos", &[("xpos", P::Index(0)), ("ypos", P::Index(1))]),
    (
        "anchor",
        &[("xanchor", P::Index(0)), ("yanchor", P::Index(1))],
    ),
    (
        "offset",
        &[("xoffset", P::Index(0)), ("yoffset", P::Index(1))],
    ),
    (
        "align",
        &[
            ("xpos", P::Index(0)),
            ("ypos", P::Index(1)),
            ("xanchor", P::Index(0)),
            ("yanchor", P::Index(1)),
        ],
    ),
    (
        "maximum",
        &[("xmaximum", P::Index(0)), ("ymaximum", P::Index(1))],
    ),
    (
        "minimum",
        &[("xminimum", P::Index(0)), ("yminimum", P::Index(1))],
    ),
    ("xsize", &[("xminimum", P::Same), ("xmaximum", P::Same)]),
    ("ysize", &[("yminimum", P::Same), ("ymaximum", P::Same)]),
    (
        "xysize",
        &[
            ("xminimum", P::Index(0)),
            ("xmaximum", P::Index(0)),
            ("yminimum", P::Index(1)),
            ("ymaximum", P::Index(1)),
        ],
    ),
    (
        "area",
        &[
            ("xpos", P::Index(0)),
            ("ypos", P::Index(1)),
            ("xanchor", P::Int(0)),
            ("yanchor", P::Int(0)),
            ("xfill", P::True),
            ("yfill", P::True),
            ("xmaximum", P::Index(2)),
            ("ymaximum", P::Index(3)),
            ("xminimum", P::Index(2)),
            ("yminimum", P::Index(3)),
        ],
    ),
    ("xcenter", &[("xpos", P::Same), ("xanchor", P::Float(0.5))]),
    ("ycenter", &[("ypos", P::Same), ("yanchor", P::Float(0.5))]),
    (
        "xycenter",
        &[
            ("xpos", P::Index(0)),
            ("ypos", P::Index(1)),
            ("xanchor", P::Float(0.5)),
            ("yanchor", P::Float(0.5)),
        ],
    ),
];

/// (name, cache index or negative, priority level, alternate names).
const PREFIXES: &[(&str, i32, i32, &[&str])] = &[
    ("selected_hover_", 5, 3, &[]),
    ("selected_idle_", 4, 3, &[]),
    ("selected_insensitive_", 3, 3, &[]),
    (
        "selected_",
        -3,
        2,
        &["selected_hover_", "selected_idle_", "selected_insensitive_"],
    ),
    ("hover_", 2, 1, &["selected_hover_"]),
    ("idle_", 1, 1, &["selected_idle_"]),
    ("insensitive_", 0, 1, &["selected_insensitive_"]),
    (
        "",
        -4,
        0,
        &[
            "selected_hover_",
            "selected_idle_",
            "selected_insensitive_",
            "idle_",
            "hover_",
            "insensitive_",
        ],
    ),
    ("activate_", -2, 0, &[]),
    ("selected_activate_", -1, 0, &[]),
];

const PREFIX_SEARCH: &[(&str, &[&str])] = &[
    ("idle_", &["idle_", ""]),
    ("hover_", &["hover_", ""]),
    ("insensitive_", &["insensitive_", "", "idle_"]),
    (
        "selected_idle_",
        &["selected_idle_", "selected_", "", "idle_"],
    ),
    (
        "selected_hover_",
        &["selected_hover_", "hover_", "selected_", ""],
    ),
    (
        "selected_insensitive_",
        &[
            "selected_insensitive_",
            "insensitive_",
            "selected_",
            "",
            "selected_idle_",
            "idle_",
        ],
    ),
    ("", &[""]),
];

pub const COUNT: usize = STYLE_PROPERTIES.len();

/// A prefix: its priority (doubled, as upstream), and the cache slot
/// offsets / names it assigns.
struct Prefix {
    name: &'static str,
    priority: i32,
    alts: Vec<(usize, &'static str)>,
}

struct Tables {
    index: HashMap<&'static str, usize>,
    prefixes: Vec<Prefix>,
    /// Every (prefixed) property name -> (prefix, base property).
    full: HashMap<String, (usize, &'static str)>,
    /// Base property (style or synthetic) -> targets.
    all: Vec<(&'static str, Vec<(&'static str, Pick)>)>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let index: HashMap<_, _> = STYLE_PROPERTIES
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (*n, i))
            .collect();
        let slot = |n: &str| PREFIXES.iter().find(|p| p.0 == n).map(|p| p.1).unwrap();
        let prefixes: Vec<Prefix> = PREFIXES
            .iter()
            .map(|(name, idx, pri, alts)| {
                let mut a = Vec::new();
                if *idx >= 0 {
                    a.push((*idx as usize, *name));
                }
                for n in *alts {
                    a.push((slot(n) as usize, *n));
                }
                Prefix {
                    name,
                    priority: pri * 2,
                    alts: a,
                }
            })
            .collect();
        let mut all: Vec<(&'static str, Vec<(&'static str, Pick)>)> = STYLE_PROPERTIES
            .iter()
            .map(|(n, _)| (*n, vec![(*n, P::Same)]))
            .collect();
        for (n, l) in SYNTHETIC {
            all.push((n, l.to_vec()));
        }
        let mut full = HashMap::new();
        // Unprefixed names first: a property such as `hover_sound` is not
        // the hover_ form of the pre-6.17 `sound`.
        let bare = prefixes.iter().position(|p| p.name.is_empty()).unwrap();
        for (n, _) in &all {
            full.insert(n.to_string(), (bare, *n));
        }
        for (pi, p) in prefixes.iter().enumerate() {
            for (n, _) in &all {
                full.entry(format!("{}{}", p.name, n)).or_insert((pi, *n));
            }
        }
        Tables {
            index,
            prefixes,
            full,
            all,
        }
    })
}

/// Index of a style property (the `*_INDEX` constants).
pub fn index(name: &str) -> Option<usize> {
    tables().index.get(name).copied()
}

pub fn prefix_offset(prefix: &str) -> Option<usize> {
    PREFIXES
        .iter()
        .find(|p| p.0 == prefix && p.1 >= 0)
        .map(|p| p.1 as usize * COUNT)
}

fn targets(name: &str) -> &'static [(&'static str, Pick)] {
    tables()
        .all
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[])
}

// ---------------------------------------------------------------------------
// Native build state.

#[derive(Default)]
struct Core {
    built: bool,
    building: bool,
    down: Option<Value>,
    left: Option<Value>,
    cache: Option<Rc<Vec<Option<Value>>>>,
    prefix_offset: usize,
}

type CoreRef = Rc<RefCell<Core>>;

struct Classes {
    core: ObjRef,
    style: ObjRef,
    holder: ObjRef,
    styles: Value,
}

fn classes(vm: &Vm) -> Rc<Classes> {
    vm.ext_get::<Rc<Classes>>().expect("renpy.style").clone()
}

fn ensure(vm: &mut Vm) -> PyResult<Rc<Classes>> {
    if let Some(c) = vm.ext_get::<Rc<Classes>>() {
        return Ok(c.clone());
    }
    py::import(vm, MODULE)?;
    Ok(classes(vm))
}

fn core(vm: &Vm, s: &Value) -> PyResult<CoreRef> {
    let Some(o) = s.obj() else {
        return Err(vm.type_error("not a style"));
    };
    if let Some(c) = ops::get_raw_attr(o, CORE) {
        if let Some(r) = py::native::<CoreRef, _>(&c, |r| r.clone()) {
            return Ok(r);
        }
    }
    let r: CoreRef = Rc::default();
    let h = classes(vm).holder.clone();
    let v = vm.types.new_native(&h, Box::new(r.clone()));
    ops::set_raw_attr(vm, o, CORE, v);
    Ok(r)
}

pub fn is_style(vm: &mut Vm, v: &Value) -> bool {
    match ensure(vm) {
        Ok(c) => ops::isinstance(vm, v, &c.core),
        Err(_) => false,
    }
}

fn field(s: &Value, name: &str) -> Value {
    s.obj()
        .and_then(|o| ops::get_raw_attr(o, name))
        .unwrap_or(Value::None)
}

fn set_field(vm: &Vm, s: &Value, name: &str, v: Value) {
    if let Some(o) = s.obj() {
        ops::set_raw_attr(vm, o, name, v);
    }
}

fn properties_list(vm: &mut Vm, s: &Value) -> PyResult<Value> {
    let p = field(s, "properties");
    if py::is_none(&p) {
        let l = vm.types.new_list(Vec::new());
        set_field(vm, s, "properties", l.clone());
        return Ok(l);
    }
    Ok(p)
}

// ---------------------------------------------------------------------------
// Registry.

fn styles_get(vm: &mut Vm, key: &Value) -> PyResult<Option<Value>> {
    let d = classes(vm).styles.clone();
    let r = vm.call_method(&d, "get", vec![key.clone(), Value::None])?;
    Ok((!py::is_none(&r)).then_some(r))
}

fn styles_set(vm: &mut Vm, key: Value, v: Value) -> PyResult<()> {
    let d = classes(vm).styles.clone();
    ops::setitem(vm, &d, key, v)
}

fn name1(vm: &Vm, n: &str) -> Value {
    let s = vm.types.new_str(n);
    vm.types.new_tuple(vec![s])
}

fn new_style(vm: &mut Vm, parent: Value, name: Value) -> PyResult<Value> {
    let c = classes(vm).style.clone();
    vm.call(&Value::Obj(c), vec![parent], vec![("name".into(), name)])
}

/// `get_style(name)`.
pub fn get_style(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let key = name1(vm, name);
    if let Some(rv) = styles_get(vm, &key)? {
        return Ok(rv);
    }
    if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some() {
        let d = classes(vm).styles.clone();
        let n = ops::len(vm, &d).unwrap_or(0);
        let k = name1(vm, "main_menu_text");
        let has = styles_get(vm, &k)?.is_some();
        super::link::diagnostic(
            vm,
            format!("trace: get_style({name}) missing; {n} styles; main_menu_text={has}"),
        );
    }
    let missing = |vm: &Vm| py::error(vm, format!("Style {} does not exist.", quote(name)));
    let (start, end) = name.split_once('_').unwrap_or((name, ""));
    let end = if start.is_empty() {
        let rest = &name[1..];
        match rest.split_once('_') {
            Some((_, e)) if !e.is_empty() => format!("_{e}"),
            _ => return Err(missing(vm)),
        }
    } else if end.is_empty() {
        return Err(missing(vm));
    } else {
        end.to_owned()
    };
    let parent = match get_style(vm, &end) {
        Ok(p) => p,
        Err(_) => return Err(missing(vm)),
    };
    let rv = new_style(vm, parent, key.clone())?;
    styles_set(vm, key, rv.clone())?;
    Ok(rv)
}

fn quote(s: &str) -> String {
    format!("'{s}'")
}

/// `get_or_create_style(name)`.
pub fn get_or_create_style(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let key = name1(vm, name);
    if let Some(rv) = styles_get(vm, &key)? {
        return Ok(rv);
    }
    let parent = match name.split_once('_') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => get_or_create_style(vm, e)?,
        _ => vm.types.new_str("default"),
    };
    let rv = new_style(vm, parent, key.clone())?;
    styles_set(vm, key, rv.clone())?;
    Ok(rv)
}

/// `get_full_style(name)` for a tuple name.
pub fn get_full_style(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    if let Some(rv) = styles_get(vm, name)? {
        return Ok(rv);
    }
    let parts = py::items(vm, name)?;
    let Some(first) = parts.first() else {
        return Err(py::error(vm, "Empty style name."));
    };
    let first = py::string(vm, first)?;
    let mut rv = get_style(vm, &first)?;
    for i in &parts[1..] {
        rv = ops::getitem(vm, &rv, i.clone())?;
    }
    Ok(rv)
}

/// `get_tuple_name(s)`.
pub fn get_tuple_name(vm: &mut Vm, s: &Value) -> PyResult<Value> {
    if is_style(vm, s) {
        return Ok(field(s, "name"));
    }
    if matches!(s.payload(), Some(crysirror_python::Payload::Tuple(_))) || py::is_none(s) {
        return Ok(s.clone());
    }
    Ok(vm.types.new_tuple(vec![s.clone()]))
}

fn style_name_to_string(vm: &mut Vm, name: &Value) -> PyResult<String> {
    if py::is_none(name) {
        return Ok("anonymous style".into());
    }
    let parts = py::items(vm, name)?;
    let mut rv = String::from("style ");
    for (i, p) in parts.iter().enumerate() {
        let p = py::string(vm, p)?;
        if i == 0 {
            rv.push_str(&p);
        } else {
            rv.push_str(&format!("['{p}']"));
        }
    }
    Ok(rv)
}

// ---------------------------------------------------------------------------
// Value conversion (`styleutil`).

fn easy(vm: &mut Vm, name: &str, v: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.easy", name)?;
    vm.call(&f, vec![v.clone()], Vec::new())
}

fn none_is_null(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    if py::is_none(v) {
        let n = py::import_attr(vm, "renpy.display.layout", "Null")?;
        return vm.call(&n, Vec::new(), Vec::new());
    }
    easy(vm, "displayable", v)
}

fn expand_focus_mask(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    if matches!(v, Value::None | Value::Bool(_)) || crysirror_python::builtins::is_callable(vm, v) {
        return Ok(v.clone());
    }
    easy(vm, "displayable", v)
}

fn expand_outlines(vm: &mut Vm, l: &Value) -> PyResult<Value> {
    let color = if profile_at_least(vm, 8, 6) {
        py::import_attr(vm, "renpy.color", "Color")?
    } else {
        py::import_attr(vm, "renpy.easy", "color")?
    };
    let mut rv = Vec::new();
    for i in py::items(vm, l)? {
        let parts = py::items(vm, &i)?;
        if parts.len() < 2 {
            return Err(vm.type_error("outline must have at least two elements"));
        }
        let c = vm.call(&color, vec![parts[1].clone()], Vec::new())?;
        if parts.len() == 2 {
            rv.push(
                vm.types
                    .new_tuple(vec![parts[0].clone(), c, Value::Int(0), Value::Int(0)]),
            );
        } else {
            rv.push(vm.types.new_tuple(vec![
                parts[0].clone(),
                c,
                parts[2].clone(),
                parts[3].clone(),
            ]));
        }
    }
    Ok(vm.types.new_list(rv))
}

const ANCHORS: &[(&str, f64)] = &[
    ("left", 0.0),
    ("right", 1.0),
    ("center", 0.5),
    ("top", 0.0),
    ("bottom", 1.0),
];

fn expand_anchor(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    if let Some(s) = v.as_str() {
        return Ok(ANCHORS
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, f)| Value::Float(*f))
            .unwrap_or_else(|| v.clone()));
    }
    // Unhashable values: compare against the store's position objects.
    if matches!(v, Value::Obj(_)) && ops::hash(vm, v).is_err() {
        for (n, f) in ANCHORS {
            let o = py::import_attr(vm, "store", n).unwrap_or(Value::None);
            if !py::is_none(&o) && py::same(&o, v) {
                return Ok(Value::Float(*f));
            }
        }
    }
    Ok(v.clone())
}

fn profile_at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm)
        .map(|r| r.profile.at_least(major, minor))
        .unwrap_or(true)
}

fn convert(vm: &mut Vm, c: Conv, v: Value) -> PyResult<Value> {
    Ok(match c {
        C::None => v,
        C::NoneIsNull => none_is_null(vm, &v)?,
        C::Displayable => easy(vm, "displayable_or_none", &v)?,
        C::Color => easy(vm, "color", &v)?,
        C::FocusMask => expand_focus_mask(vm, &v)?,
        C::Outlines => expand_outlines(vm, &v)?,
        C::Anchor => expand_anchor(vm, &v)?,
        C::NoneIs0 if py::is_none(&v) && profile_at_least(vm, 7, 4) => Value::Int(0),
        C::NoneIs0 => v,
    })
}

fn pick(vm: &mut Vm, v: &Value, p: Pick) -> PyResult<Value> {
    Ok(match p {
        P::Same => v.clone(),
        P::Index(i) => ops::getitem(vm, v, Value::Int(i as i64))?,
        P::IndexOr(i, j) => {
            let n = ops::len(vm, v)?;
            ops::getitem(vm, v, Value::Int(if n > i { i } else { j } as i64))?
        }
        P::Int(i) => Value::Int(i),
        P::Float(f) => Value::Float(f),
        P::True => Value::Bool(true),
    })
}

// ---------------------------------------------------------------------------
// Building.

struct Cache {
    values: Vec<Option<Value>>,
    priorities: Vec<i32>,
}

fn assign(c: &mut Cache, index: usize, priority: i32, v: Value) {
    if priority < c.priorities[index] {
        return;
    }
    c.values[index] = Some(v);
    c.priorities[index] = priority;
}

fn assign_prefixed(
    vm: &mut Vm,
    c: &mut Cache,
    index: usize,
    priority: i32,
    d: &Value,
    prefix: &str,
) -> PyResult<()> {
    if priority < c.priorities[index] {
        return Ok(());
    }
    let mut dd = d.clone();
    if !py::is_none(d) {
        let dup = py::get_opt(vm, d, "_duplicatable")?.unwrap_or(Value::Bool(false));
        if py::truthy(vm, &dup)? {
            let a = py::get(vm, d, "_args")?;
            let p = vm.types.new_str(prefix);
            let args = py::method_kw(vm, &a, "copy", Vec::new(), vec![("prefix".into(), p)])?;
            dd = vm.call_method(d, "_duplicate", vec![args])?;
            vm.call_method(&dd, "_unique", Vec::new())?;
        }
    }
    c.values[index] = Some(dd);
    c.priorities[index] = priority;
    Ok(())
}

fn apply_property(
    vm: &mut Vm,
    c: &mut Cache,
    priority: i32,
    key: &str,
    value: &Value,
) -> PyResult<()> {
    let t = tables();
    let Some(&(pi, prop)) = t.full.get(key) else {
        return Ok(());
    };
    let prefix = &t.prefixes[pi];
    let priority = priority
        + prefix.priority
        + if PROPERTY_PRIORITY.contains(&prop) {
            1
        } else {
            0
        };
    for (target, p) in targets(prop) {
        let v = pick(vm, value, *p)?;
        let i = t.index[target];
        let v = convert(vm, STYLE_PROPERTIES[i].1, v)?;
        let displayable = DISPLAYABLE_PROPERTIES.contains(target);
        for (alt, alt_name) in &prefix.alts {
            let slot = alt * COUNT + i;
            if displayable {
                assign_prefixed(vm, c, slot, priority, &v, alt_name)?;
            } else {
                assign(c, slot, priority, v.clone());
            }
        }
    }
    Ok(())
}

/// `build_style(s)`.
pub fn build_style(vm: &mut Vm, s: &Value) -> PyResult<()> {
    let core = core(vm, s)?;
    if core.borrow().built {
        return Ok(());
    }
    let name = field(s, "name");
    if core.borrow().building && !py::is_none(&name) {
        let n = style_name_to_string(vm, &name)?;
        return Err(py::error(
            vm,
            format!("{n} is part of a loop of recursive styles (is one of its own parents)."),
        ));
    }
    core.borrow_mut().building = true;
    let r = build_inner(vm, s, &core, &name);
    core.borrow_mut().building = false;
    r
}

fn build_inner(vm: &mut Vm, s: &Value, core: &CoreRef, name: &Value) -> PyResult<()> {
    let parent = field(s, "parent");
    if !py::is_none(&parent) {
        let d = get_full_style(vm, &parent)?;
        core.borrow_mut().down = Some(d.clone());
        build_style(vm, &d)?;
    }
    if !py::is_none(name) && ops::len(vm, name)? > 1 {
        let n = ops::len(vm, name)?;
        let parts = py::items(vm, name)?;
        let base = vm.types.new_tuple(parts[..n - 1].to_vec());
        let l = get_full_style(vm, &base)?;
        core.borrow_mut().left = Some(l.clone());
        build_style(vm, &l)?;
    }
    let props = properties_list(vm, s)?;
    let dicts = py::items(vm, &props)?;
    if dicts.is_empty() {
        let mut c = core.borrow_mut();
        c.cache = None;
        c.built = true;
        return Ok(());
    }
    let mut cache = Cache {
        values: vec![None; PREFIX_COUNT * COUNT],
        priorities: vec![0; PREFIX_COUNT * COUNT],
    };
    let mut priority = 1;
    for d in dicts {
        for (k, v) in ops::mapping_items(vm, &d)? {
            let Some(k) = k.as_str().map(str::to_owned) else {
                continue;
            };
            if let Err(e) = apply_property(vm, &mut cache, priority, &k, &v) {
                let n = style_name_to_string(vm, name)?;
                let info = vm
                    .types
                    .new_str(&format!("While processing the {k} property of {n}:"));
                if let Ok(g) = py::import(vm, "renpy.game") {
                    let _ = py::set(vm, &g, "exception_info", info);
                }
                return Err(e);
            }
        }
        priority += PRIORITY_LEVELS;
    }
    let mut c = core.borrow_mut();
    c.cache = Some(Rc::new(cache.values));
    c.built = true;
    Ok(())
}

/// `unbuild_style(s)`.
pub fn unbuild_style(vm: &mut Vm, s: &Value) -> PyResult<()> {
    let core = core(vm, s)?;
    let mut c = core.borrow_mut();
    if !c.built {
        return Ok(());
    }
    c.cache = None;
    c.left = None;
    c.down = None;
    c.built = false;
    c.building = false;
    Ok(())
}

/// `StyleCore._get(index)`: the property at `index` (unoffset) under the
/// style's current prefix, from the style or its parents.
pub fn get(vm: &mut Vm, s: &Value, index: usize) -> PyResult<Value> {
    let c = core(vm, s)?;
    let offset = c.borrow().prefix_offset;
    get_unoffset(vm, s, index + offset)
}

pub fn get_unoffset(vm: &mut Vm, s: &Value, index: usize) -> PyResult<Value> {
    if index >= PREFIX_COUNT * COUNT {
        return Err(vm.exc_msg("IndexError", "style property index out of range"));
    }
    if !core(vm, s)?.borrow().built {
        build_style(vm, s)?;
    }
    let mut cur = s.clone();
    let mut left: Option<Value> = None;
    for _ in 0..100 {
        let c = core(vm, &cur)?;
        let (next, l) = {
            let c = c.borrow();
            if let Some(v) = c.cache.as_ref().and_then(|v| v[index].clone()) {
                return Ok(v);
            }
            (c.down.clone(), c.left.clone())
        };
        if left.is_none() && l.is_some() {
            left = l;
        }
        cur = match next.or_else(|| left.take()) {
            Some(n) => n,
            None => return Ok(Value::None),
        };
    }
    let r = ops::repr(vm, s)?;
    Err(py::error(
        vm,
        format!("{r} is too complex. Check for loops in style inheritance."),
    ))
}

/// Reads a named (unprefixed) property of a style.
pub fn get_named(vm: &mut Vm, s: &Value, name: &str) -> PyResult<Value> {
    match index(name) {
        Some(i) => get(vm, s, i),
        None => Err(vm.attr_error(format!("'Style' object has no attribute '{name}'"))),
    }
}

// ---------------------------------------------------------------------------
// StyleCore methods.

fn s_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let parent = arg_or(&args[1..], kw, 0, "parent", Value::None);
    let properties = arg_or(&args[1..], kw, 1, "properties", Value::None);
    let name = arg_or(&args[1..], kw, 2, "name", Value::None);
    let help = arg_or(&args[1..], kw, 3, "help", Value::None);
    set_field(vm, this, "prefix", py::s(vm, "insensitive_"));
    core(vm, this)?.borrow_mut().prefix_offset = 0;
    let mut list = Vec::new();
    if py::truthy(vm, &properties)? {
        let p = if matches!(
            properties.payload(),
            Some(crysirror_python::Payload::Dict(_))
        ) && Rc::ptr_eq(&ops::type_of(vm, &properties), &vm.types.dict)
        {
            properties
        } else {
            let d = Value::Obj(vm.types.dict.clone());
            vm.call(&d, vec![properties], Vec::new())?
        };
        list.push(p);
    }
    let l = vm.types.new_list(list);
    set_field(vm, this, "properties", l);
    let p = get_tuple_name(vm, &parent)?;
    set_field(vm, this, "parent", p);
    set_field(vm, this, "name", name);
    set_field(vm, this, "help", help);
    Ok(Value::None)
}

fn s_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = classes(vm).style.clone();
    let parent = field(&args[0], "parent");
    let rv = vm.call(&Value::Obj(c), vec![parent], Vec::new())?;
    let props = properties_list(vm, &args[0])?;
    let items = py::items(vm, &props)?;
    let l = vm.types.new_list(items);
    set_field(vm, &rv, "properties", l);
    Ok(rv)
}

fn style_eq(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    if py::same(a, b) {
        return Ok(true);
    }
    let (ta, tb) = (ops::type_of(vm, a), ops::type_of(vm, b));
    if !Rc::ptr_eq(&ta, &tb) {
        return Ok(false);
    }
    for f in ["parent", "name", "properties"] {
        let (x, y) = (field(a, f), field(b, f));
        if !ops::eq(vm, &x, &y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn s_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(style_eq(vm, &args[0], &args[1])?))
}

fn s_ne(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(!style_eq(vm, &args[0], &args[1])?))
}

fn s_hash(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(
        args[0]
            .obj()
            .map(|o| Rc::as_ptr(o) as usize as i64 >> 4)
            .unwrap_or(0),
    ))
}

fn s_getstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::dict(vm);
    for f in ["properties", "prefix", "name", "parent"] {
        let v = field(&args[0], f);
        py::dict_set(vm, &d, f, v)?;
    }
    Ok(d)
}

fn s_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, state) = (&args[0], &args[1]);
    let k = |vm: &Vm, n: &str| vm.types.new_str(n);
    let props = ops::getitem(vm, state, k(vm, "properties"))?;
    set_field(vm, this, "properties", props);
    let name = ops::getitem(vm, state, k(vm, "name"))?;
    set_field(vm, this, "name", name);
    let parent = ops::getitem(vm, state, k(vm, "parent"))?;
    let p = get_tuple_name(vm, &parent)?;
    set_field(vm, this, "parent", p);
    let prefix = ops::getitem(vm, state, k(vm, "prefix"))?;
    set_prefix(vm, this, &prefix)?;
    Ok(Value::None)
}

fn s_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = field(this, "name");
    let n = style_name_to_string(vm, &name)?;
    let parent = field(this, "parent");
    let id = this.obj().map(|o| Rc::as_ptr(o) as usize).unwrap_or(0);
    let r = if py::truthy(vm, &parent)? {
        let p = style_name_to_string(vm, &parent)?;
        format!("<{n} is {p} @ {id:#x}>")
    } else {
        format!("<{n} @ {id:#x}>")
    };
    Ok(vm.types.new_str(&r))
}

fn s_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = field(this, "name");
    let tname = concat(vm, &name, &args[1])?;
    if let Some(rv) = styles_get(vm, &tname)? {
        return Ok(rv);
    }
    let parent = field(this, "parent");
    let parent = if py::is_none(&parent) {
        Value::None
    } else {
        concat(vm, &parent, &args[1])?
    };
    let rv = new_style(vm, parent, tname.clone())?;
    styles_set(vm, tname, rv.clone())?;
    Ok(rv)
}

/// `name + (item,)` for a tuple name.
fn concat(vm: &mut Vm, name: &Value, item: &Value) -> PyResult<Value> {
    if py::is_none(name) {
        return Err(vm.type_error("unsupported operand type(s) for +: 'NoneType' and 'tuple'"));
    }
    let mut parts = py::items(vm, name)?;
    parts.push(item.clone());
    Ok(vm.types.new_tuple(parts))
}

fn append_property(vm: &mut Vm, this: &Value, name: &Value, value: Value) -> PyResult<()> {
    let d = py::dict(vm);
    ops::setitem(vm, &d, name.clone(), value)?;
    let props = properties_list(vm, this)?;
    vm.call_method(&props, "append", vec![d])?;
    Ok(())
}

fn s_setattr_method(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    append_property(vm, &args[0], &args[1], args[2].clone())?;
    Ok(Value::None)
}

fn delattr(vm: &mut Vm, this: &Value, name: &Value) -> PyResult<()> {
    let props = properties_list(vm, this)?;
    for d in py::items(vm, &props)? {
        if ops::contains(vm, &d, name)? {
            ops::delitem(vm, &d, name.clone())?;
        }
    }
    Ok(())
}

fn s_delattr_method(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    delattr(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

const PUBLIC_FIELDS: &[&str] = &["name", "parent", "properties", "prefix", "help"];

fn s_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, name, value) = (&args[0], &args[1], args[2].clone());
    let n = py::string(vm, name)?;
    if PUBLIC_FIELDS.contains(&n.as_str()) {
        set_field(vm, this, &n, value);
        return Ok(Value::None);
    }
    let old_only = n.ends_with("sound")
        && tables().full.get(n.as_str()).is_some_and(|(_, base)| *base == "sound")
        && profile_at_least(vm, 6, 17);
    if !tables().full.contains_key(n.as_str()) || old_only {
        return Err(py::error(vm, format!("Style property {n} is not known.")));
    }
    append_property(vm, this, name, value)?;
    Ok(Value::None)
}

fn s_delattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    delattr(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn s_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    get_named(vm, &args[0], &n)
}

fn s_set_parent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = get_tuple_name(vm, &args[1])?;
    set_field(vm, &args[0], "parent", p);
    Ok(Value::None)
}

fn s_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = vm.types.new_list(Vec::new());
    set_field(vm, &args[0], "properties", l);
    Ok(Value::None)
}

fn copy_properties(vm: &mut Vm, p: &Value) -> PyResult<Value> {
    let d = Value::Obj(vm.types.dict.clone());
    let mut out = Vec::new();
    for i in py::items(vm, p)? {
        out.push(vm.call(&d, vec![i], Vec::new())?);
    }
    Ok(vm.types.new_list(out))
}

fn s_take(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let other = if is_style(vm, &args[1]) {
        args[1].clone()
    } else {
        let n = py::string(vm, &args[1])?;
        get_style(vm, &n)?
    };
    let p = properties_list(vm, &other)?;
    let c = copy_properties(vm, &p)?;
    set_field(vm, &args[0], "properties", c);
    Ok(Value::None)
}

fn s_setdefault(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let props = py::kwargs_dict(vm, kw)?;
    let list = properties_list(vm, &args[0])?;
    for p in py::items(vm, &list)? {
        for k in py::items(vm, &p)? {
            if ops::contains(vm, &props, &k)? {
                ops::delitem(vm, &props, k)?;
            }
        }
    }
    if py::truthy(vm, &props)? {
        vm.call_method(&list, "append", vec![props])?;
    }
    Ok(Value::None)
}

fn s_add_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = Value::Obj(vm.types.dict.clone());
    let p = vm.call(&d, vec![args[1].clone()], Vec::new())?;
    let list = properties_list(vm, &args[0])?;
    vm.call_method(&list, "append", vec![p])?;
    Ok(Value::None)
}

pub fn set_prefix(vm: &mut Vm, this: &Value, prefix: &Value) -> PyResult<()> {
    let current = field(this, "prefix");
    if ops::eq(vm, prefix, &current)? {
        return Ok(());
    }
    set_field(vm, this, "prefix", prefix.clone());
    if let Some(off) = prefix.as_str().and_then(prefix_offset) {
        core(vm, this)?.borrow_mut().prefix_offset = off;
    }
    Ok(())
}

fn s_set_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    set_prefix(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn s_get_offset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(
        core(vm, &args[0])?.borrow().prefix_offset as i64,
    ))
}

fn s_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    for n in [
        "xpos", "ypos", "xanchor", "yanchor", "xoffset", "yoffset", "subpixel",
    ] {
        out.push(get(vm, &args[0], index(n).unwrap())?);
    }
    Ok(vm.types.new_tuple(out))
}

fn s_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::int(vm, &args[1])?;
    get(vm, &args[0], i.max(0) as usize)
}

fn s_get_unoffset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::int(vm, &args[1])?;
    get_unoffset(vm, &args[0], i.max(0) as usize)
}

fn visit(vm: &mut Vm, args: &[Value], props: &[&str]) -> PyResult<Value> {
    for p in 0..PREFIX_COUNT {
        for n in props {
            let v = get_unoffset(vm, &args[0], p * COUNT + index(n).unwrap())?;
            if !py::is_none(&v) {
                vm.call(&args[1], vec![v], Vec::new())?;
            }
        }
    }
    Ok(Value::None)
}

fn s_visit_window(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    visit(vm, args, &["background", "child", "foreground"])
}

fn s_visit_bar(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    visit(vm, args, &["fore_bar", "aft_bar", "thumb", "thumb_shadow"])
}

fn s_visit_frame(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    visit(vm, args, &["child"])
}

fn s_hover_alt(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_unoffset(vm, &args[0], 2 * COUNT + index("alt").unwrap())
}

/// Priority of a prefixed property name and the cache slots it affects
/// (`init_inspect`).
fn inspect_info(name: &str) -> (i32, Vec<String>) {
    let t = tables();
    let Some(&(pi, prop)) = t.full.get(name) else {
        return (100, Vec::new());
    };
    let p = &t.prefixes[pi];
    let pri = p.priority
        + if PROPERTY_PRIORITY.contains(&prop) {
            1
        } else {
            0
        };
    let mut affects = Vec::new();
    for (_, alt) in &p.alts {
        for (target, _) in targets(prop) {
            affects.push(format!("{alt}{target}"));
        }
    }
    (pri, affects)
}

fn s_inspect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    build_style(vm, this)?;
    let mut rv = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = this.clone();
    let mut left: Option<Value> = None;
    loop {
        let my = py::dict(vm);
        let props = properties_list(vm, &cur)?;
        let mut dicts = py::items(vm, &props)?;
        dicts.reverse();
        for d in dicts {
            let mut names = Vec::new();
            for k in py::items(vm, &d)? {
                let n = py::string(vm, &k)?;
                names.push((inspect_info(&n), n, k));
            }
            names.sort_by_key(|((p, _), _, _)| *p);
            names.reverse();
            for ((_, affects), _, k) in names {
                if affects.iter().all(|a| seen.contains(a)) {
                    continue;
                }
                seen.extend(affects);
                let v = ops::getitem(vm, &d, k.clone())?;
                ops::setitem(vm, &my, k, v)?;
            }
        }
        let name = field(&cur, "name");
        rv.push(vm.types.new_tuple(vec![name, my]));
        let c = core(vm, &cur)?;
        let (down, l) = {
            let c = c.borrow();
            (c.down.clone(), c.left.clone())
        };
        if left.is_none() && l.is_some() {
            left = l;
        }
        match down.or_else(|| left.take()) {
            Some(n) => cur = n,
            None => break,
        }
    }
    Ok(vm.types.new_list(rv))
}

const CORE_METHODS: py::Methods = &[
    ("__init__", s_init),
    ("copy", s_copy),
    ("__eq__", s_eq),
    ("__ne__", s_ne),
    ("__hash__", s_hash),
    ("__getstate__", s_getstate),
    ("__setstate__", s_setstate),
    ("__repr__", s_repr),
    ("__getitem__", s_getitem),
    ("setattr", s_setattr_method),
    ("delattr", s_delattr_method),
    ("__setattr__", s_setattr),
    ("__delattr__", s_delattr),
    ("set_parent", s_set_parent),
    ("clear", s_clear),
    ("take", s_take),
    ("setdefault", s_setdefault),
    ("add_properties", s_add_properties),
    ("set_prefix", s_set_prefix),
    ("get_offset", s_get_offset),
    ("get_placement", s_get_placement),
    ("_get", s_get),
    ("_get_unoffset", s_get_unoffset),
    ("_visit_window", s_visit_window),
    ("_visit_bar", s_visit_bar),
    ("_visit_frame", s_visit_frame),
    ("_hover_alt", s_hover_alt),
    ("inspect", s_inspect),
];

// ---------------------------------------------------------------------------
// StyleManager.

fn m_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, name, value) = (&args[0], &args[1], &args[2]);
    if !is_style(vm, value) {
        let compat = py::get_opt(vm, value, "_is_style_compat")?.unwrap_or(Value::Bool(false));
        if py::truthy(vm, &compat)? {
            let d = py::namespace_of(vm, this)?;
            ops::setitem(vm, &d, name.clone(), value.clone())?;
            return Ok(Value::None);
        }
        return Err(py::error(vm, "Value is not a style."));
    }
    let key = vm.types.new_tuple(vec![name.clone()]);
    if py::is_none(&field(value, "name")) {
        set_field(vm, value, "name", key.clone());
    }
    styles_set(vm, key, value.clone())?;
    Ok(Value::None)
}

fn m_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    if n.starts_with("__") && n.ends_with("__") {
        return Err(vm.attr_error(n));
    }
    get_style(vm, &n)
}

fn m_create(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(&args[1..], kw, 0, "name", Value::None);
    let parent = arg_or(&args[1..], kw, 1, "parent", Value::None);
    let help = arg_or(&args[1..], kw, 2, "description", Value::None);
    let c = classes(vm).style.clone();
    let s = vm.call(&Value::Obj(c), vec![parent], vec![("help".into(), help)])?;
    m_setattr(vm, &[args[0].clone(), name, s], &[])
}

fn m_rebuild(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    rebuild(vm, true)?;
    Ok(Value::None)
}

fn m_exists(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = classes(vm).styles.clone();
    if ops::contains(vm, &d, &args[1])? {
        return Ok(Value::Bool(true));
    }
    let t = vm.types.new_tuple(vec![args[1].clone()]);
    Ok(Value::Bool(ops::contains(vm, &d, &t)?))
}

fn m_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if matches!(args[1].payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return get_full_style(vm, &args[1]);
    }
    let n = py::string(vm, &args[1])?;
    get_style(vm, &n)
}

// ---------------------------------------------------------------------------
// Module functions.

fn f_get_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    get_style(vm, &n)
}

fn f_get_or_create_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    get_or_create_style(vm, &n)
}

fn f_get_full_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_full_style(vm, &args[0])
}

fn f_get_tuple_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_tuple_name(vm, &args[0])
}

/// `get_text_style(style, default)`.
pub fn get_text_style(vm: &mut Vm, style: &Value, default: &Value) -> PyResult<Value> {
    let style = if py::is_none(style) {
        default.clone()
    } else {
        style.clone()
    };
    let style = get_tuple_name(vm, &style)?;
    let parts = py::items(vm, &style)?;
    let start = py::string(vm, &parts[0])?;
    let key = name1(vm, &format!("{start}_text"));
    let mut rv = get_full_style(vm, &key)?;
    for i in &parts[1..] {
        rv = ops::getitem(vm, &rv, i.clone())?;
    }
    Ok(rv)
}

fn f_get_text_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_text_style(vm, &args[0], &args[1])
}

fn f_style_name_to_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = style_name_to_string(vm, &args[0])?;
    Ok(vm.types.new_str(&s))
}

fn f_build_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    build_style(vm, &args[0])?;
    Ok(Value::None)
}

fn f_unbuild_style(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    unbuild_style(vm, &args[0])?;
    Ok(Value::None)
}

fn f_init_inspect(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, MODULE)?;
    if !py::is_none(&py::get(vm, &m, "priority")?) {
        return Ok(Value::None);
    }
    let (pri, aff) = (py::dict(vm), py::dict(vm));
    for name in tables().full.keys() {
        let (p, a) = inspect_info(name);
        py::dict_set(vm, &pri, name, Value::Int(p as i64))?;
        let a = a.iter().map(|s| vm.types.new_str(s)).collect();
        let l = vm.types.new_list(a);
        py::dict_set(vm, &aff, name, l)?;
    }
    py::set(vm, &m, "priority", pri)?;
    py::set(vm, &m, "affects", aff)?;
    Ok(Value::None)
}

fn f_reset(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = ensure(vm)?.styles.clone();
    vm.call_method(&d, "clear", Vec::new())?;
    Ok(Value::None)
}

/// `build_styles()`.
pub fn build_styles(vm: &mut Vm) -> PyResult<()> {
    let cbs = py::config(vm, "build_styles_callbacks").unwrap_or(Value::None);
    if !py::is_none(&cbs) {
        for f in py::items(vm, &cbs)? {
            vm.call(&f, Vec::new(), Vec::new())?;
        }
    }
    let d = ensure(vm)?.styles.clone();
    let vals = vm.call_method(&d, "values", Vec::new())?;
    let all = py::items(vm, &vals)?;
    for s in &all {
        unbuild_style(vm, s)?;
    }
    for s in &all {
        build_style(vm, s)?;
    }
    Ok(())
}

fn f_build_styles(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    build_styles(vm)?;
    Ok(Value::None)
}

/// `rebuild(prepare_screens=True)`. The argument is honoured from 8.4.
pub fn rebuild(vm: &mut Vm, prepare_screens: bool) -> PyResult<()> {
    build_styles(vm)?;
    let screen = py::import(vm, "renpy.display.screen")?;
    py::set(vm, &screen, "prepared", Value::Bool(false))?;
    let ctx = py::import_attr(vm, "renpy.game", "context")?;
    let ctx = vm.call(&ctx, Vec::new(), Vec::new())?;
    let init = py::get(vm, &ctx, "init_phase")?;
    if (prepare_screens || !profile_at_least(vm, 8, 4)) && !py::truthy(vm, &init)? {
        vm.call_method(&screen, "prepare_screens", Vec::new())?;
    }
    let ri = py::import_attr(vm, "renpy.exports", "restart_interaction")?;
    vm.call(&ri, Vec::new(), Vec::new())?;
    Ok(())
}

fn f_rebuild(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = arg_or(args, kw, 0, "prepare_screens", Value::Bool(true));
    let p = py::truthy(vm, &p)?;
    rebuild(vm, p)?;
    Ok(Value::None)
}

fn f_copy_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    copy_properties(vm, &args[0])
}

fn f_backup(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let d = ensure(vm)?.styles.clone();
    let rv = py::dict(vm);
    for (k, v) in ops::mapping_items(vm, &d)? {
        let p = properties_list(vm, &v)?;
        let c = copy_properties(vm, &p)?;
        let t = vm.types.new_tuple(vec![field(&v, "parent"), c]);
        ops::setitem(vm, &rv, k, t)?;
    }
    Ok(rv)
}

fn f_restore(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[0];
    let d = ensure(vm)?.styles.clone();
    let keys = vm.call_method(&d, "keys", Vec::new())?;
    for k in py::items(vm, &keys)? {
        if !ops::contains(vm, o, &k)? {
            ops::delitem(vm, &d, k)?;
        }
    }
    for (k, v) in ops::mapping_items(vm, o)? {
        let parts = py::items(vm, &k)?;
        let first = py::string(vm, &parts[0])?;
        let mut s = get_or_create_style(vm, &first)?;
        for i in &parts[1..] {
            s = ops::getitem(vm, &s, i.clone())?;
        }
        let pv = py::items(vm, &v)?;
        let l = vm.types.new_list(Vec::new());
        set_field(vm, &s, "properties", l);
        let p = get_tuple_name(vm, &pv[0])?;
        set_field(vm, &s, "parent", p);
        let c = copy_properties(vm, &pv[1])?;
        set_field(vm, &s, "properties", c);
    }
    Ok(Value::None)
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("cannot create style cache objects"))
}

fn str_list(vm: &Vm, l: &[&str]) -> Value {
    let v = l.iter().map(|s| vm.types.new_str(s)).collect();
    vm.types.new_list(v)
}

/// The `stylesets` tables as Python values.
fn stylesets(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let t = tables();
    let all = py::dict(vm);
    for (n, l) in &t.all {
        let names: Vec<&str> = l.iter().map(|(n, _)| *n).collect();
        let v = str_list(vm, &names);
        py::dict_set(vm, &all, n, v)?;
    }
    let proxy = py::dict(vm);
    let fs = Value::Obj(vm.types.frozenset.clone());
    for (n, l) in SYNTHETIC {
        let names: Vec<&str> = l.iter().map(|(n, _)| *n).collect();
        let v = str_list(vm, &names);
        let v = vm.call(&fs, vec![v], Vec::new())?;
        py::dict_set(vm, &proxy, n, v)?;
    }
    let (pri, alts) = (py::dict(vm), py::dict(vm));
    for p in &t.prefixes {
        py::dict_set(vm, &pri, p.name, Value::Int(p.priority as i64))?;
        let names: Vec<&str> = p.alts.iter().map(|(_, n)| *n).collect();
        let v = str_list(vm, &names);
        py::dict_set(vm, &alts, p.name, v)?;
    }
    let search = py::dict(vm);
    for (n, l) in PREFIX_SEARCH {
        let v = str_list(vm, l);
        py::dict_set(vm, &search, n, v)?;
    }
    let pp = py::dict(vm);
    for n in PROPERTY_PRIORITY {
        py::dict_set(vm, &pp, n, Value::Int(1))?;
    }
    for (n, v) in [
        ("all_properties", all),
        ("proxy_properties", proxy),
        ("prefix_priority", pri),
        ("prefix_alts", alts),
        ("prefix_search", search),
        ("property_priority", pp),
    ] {
        crysirror_python::modules::set(vm, m, n, v);
    }
    Ok(())
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("get_style", f_get_style),
            ("get_or_create_style", f_get_or_create_style),
            ("get_full_style", f_get_full_style),
            ("get_tuple_name", f_get_tuple_name),
            ("get_text_style", f_get_text_style),
            ("style_name_to_string", f_style_name_to_string),
            ("build_style", f_build_style),
            ("unbuild_style", f_unbuild_style),
            ("init_inspect", f_init_inspect),
            ("reset", f_reset),
            ("build_styles", f_build_styles),
            ("rebuild", f_rebuild),
            ("copy_properties", f_copy_properties),
            ("backup", f_backup),
            ("restore", f_restore),
        ],
    );
    let holder = vm.types.new_builtin_type(
        "StyleCache",
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Native,
        true,
    );
    let n = vm.types.new_builtin("__new__", no_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&holder, "__new__", n);
    let core = py::class(vm, MODULE, "StyleCore", &[], CORE_METHODS, &[])?;
    let style = py::class(
        vm,
        "renpy.styledata.styleclass",
        "Style",
        &[Value::Obj(core.clone())],
        &[("__getattr__", s_getattr)],
        &[],
    )?;
    let manager = py::class(
        vm,
        MODULE,
        "StyleManager",
        &[],
        &[
            ("__setattr__", m_setattr),
            ("__setitem__", m_setattr),
            ("__getattr__", m_getattr),
            ("__getitem__", m_getattr),
            ("create", m_create),
            ("rebuild", m_rebuild),
            ("exists", m_exists),
            ("get", m_get),
        ],
        &[],
    )?;
    let styles = py::dict(vm);
    vm.ext_set(Rc::new(Classes {
        core: core.clone(),
        style: style.clone(),
        holder,
        styles: styles.clone(),
    }));
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "StyleCore", Value::Obj(core));
    set(vm, "Style", Value::Obj(style));
    set(vm, "StyleManager", Value::Obj(manager));
    set(vm, "styles", styles);
    set(vm, "priority", Value::None);
    set(vm, "affects", Value::None);
    stylesets(vm, &m)?;
    let prefixed = vm.types.new_set(Default::default());
    for n in tables().full.keys() {
        let s = vm.types.new_str(n);
        vm.call_method(&prefixed, "add", vec![s])?;
    }
    set(vm, "prefixed_all_properties", prefixed);
    Ok(m)
}

fn stylesets_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.styledata.stylesets", &[]);
    stylesets(vm, &m)?;
    Ok(m)
}

fn styleclass_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.styledata.styleclass", &[]);
    let s = py::import_attr(vm, MODULE, "Style")?;
    crysirror_python::modules::set(vm, &m, "Style", s);
    Ok(m)
}

fn u_none_is_null(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    none_is_null(vm, &args[0])
}

fn u_none_is_0(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(if py::is_none(&args[0]) {
        Value::Int(0)
    } else {
        args[0].clone()
    })
}

fn u_expand_focus_mask(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    expand_focus_mask(vm, &args[0])
}

fn u_expand_outlines(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    expand_outlines(vm, &args[0])
}

fn u_expand_anchor(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    expand_anchor(vm, &args[0])
}

fn styleutil_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        "renpy.styledata.styleutil",
        &[
            ("none_is_null", u_none_is_null),
            ("none_is_0", u_none_is_0),
            ("expand_focus_mask", u_expand_focus_mask),
            ("expand_outlines", u_expand_outlines),
            ("expand_anchor", u_expand_anchor),
        ],
    );
    let d = py::dict(vm);
    for (n, f) in ANCHORS {
        py::dict_set(vm, &d, n, Value::Float(*f))?;
    }
    crysirror_python::modules::set(vm, &m, "ANCHORS", d);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
    vm.register_native_module("renpy.styledata.stylesets", stylesets_module);
    vm.register_native_module("renpy.styledata.styleclass", styleclass_module);
    vm.register_native_module("renpy.styledata.styleutil", styleutil_module);
}
