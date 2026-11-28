// SPDX-License-Identifier: MPL-2.0
//! The layout-level names of `renpy.text.text`, after Ren'Py 6.18 - 8.6
//! `text.py` (MIT): `Blit`/`outline_blits`, `DrawInfo`, the segment classes
//! (`TextSegment`, `SpaceSegment`, `DisplayableSegment`, `FlagSegment`),
//! `Layout`, `TextMeshDisplayable`, the layout caches and module constants.
//!
//! Segments produce `textsupport.Glyph` lists through `renpy.text.font`
//! (FTFont/HBFont/image fonts over the shared font service) and the
//! textsupport passes; `Layout` runs the same layout as the `Text`
//! displayable. Unavailable host: drawing glyphs into a pixel surface
//! (`TextSegment.draw`) and `TextMeshDisplayable.render`, which needs the
//! GL2 mesh/text-shader pipeline, raise an exception naming that; the
//! renderer receives text as retained glyph runs instead.
use super::{
    py::{self, *},
    textsupport::Rel,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.text.text";

// Blit -------------------------------------------------------------------------------------------

fn blit_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = &args[1..];
    let names = [
        "x", "y", "w", "h", "alpha", "left", "right", "top", "bottom",
    ];
    let defaults = [
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Float(1.0),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
    ];
    let n = if Rel::of(vm).at_least(6, 99) { 9 } else { 5 };
    for i in 0..n {
        let v = arg_or(a, kw, i, names[i], defaults[i].clone());
        py::set(vm, this, names[i], v)?;
    }
    Ok(Value::None)
}

fn blit_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut p = Vec::new();
    for n in ["x", "y", "w", "h", "alpha"] {
        let v = py::get(vm, &args[0], n)?;
        p.push(ops::str(vm, &v)?);
    }
    Ok(vm.types.new_string(format!(
        "<Blit ({}, {}, {}, {}) {}>",
        p[0], p[1], p[2], p[3], p[4]
    )))
}

/// `outline_blits(blits, outline)`: blits widened by the outline on each
/// side, rows kept from overlapping.
fn f_outline_blits(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let blits = arg_or(args, kw, 0, "blits", Value::None);
    let outline = py::f64_of(vm, &arg_or(args, kw, 1, "outline", Value::Int(0)))?;
    let new_rows = Rel::of(vm).at_least(6, 99);
    let mut items = Vec::new();
    for b in py::items(vm, &blits)? {
        let x = {
            let v = py::get(vm, &b, "x")?;
            py::f64_of(vm, &v)?
        };
        let y = {
            let v = py::get(vm, &b, "y")?;
            py::f64_of(vm, &v)?
        };
        items.push((y, x, b));
    }
    items.sort_by(|a, b| {
        (a.0, a.1)
            .partial_cmp(&(b.0, b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // The list is sorted in place, as upstream's blits.sort().
    let sorted: Vec<Value> = items.iter().map(|i| i.2.clone()).collect();
    if let Some(crysirror_python::Payload::List(l)) = blits.payload() {
        *l.borrow_mut() = sorted;
    }
    let cls = py::import_attr(vm, MODULE, "Blit")?;
    let (mut line_y, mut top_y, mut bottom_y, mut max_x) = (0f64, 0f64, 0f64, 0f64);
    let mut rv = Vec::new();
    let num = |v: f64| {
        if v.fract() == 0. {
            Value::Int(v as i64)
        } else {
            Value::Float(v)
        }
    };
    for (y, x, b) in items {
        let w = {
            let v = py::get(vm, &b, "w")?;
            py::f64_of(vm, &v)?
        };
        let h = {
            let v = py::get(vm, &b, "h")?;
            py::f64_of(vm, &v)?
        };
        let mut x0 = x;
        let x1 = x + w + outline * 2.;
        let mut y0 = y;
        let mut y1 = y + h + outline * 2.;
        if new_rows {
            y1 -= 1.;
        }
        if line_y != y0 {
            line_y = y0;
            top_y = bottom_y;
            max_x = 0.;
        }
        y0 = top_y;
        if y1 > bottom_y {
            bottom_y = y1;
        }
        if max_x > x0 {
            x0 = max_x;
        }
        max_x = x1;
        let alpha = py::get(vm, &b, "alpha")?;
        let mut kwv: Vec<(std::rc::Rc<str>, Value)> = Vec::new();
        if new_rows {
            for n in ["left", "right", "top", "bottom"] {
                let v = py::get_or_none(vm, &b, n)?;
                kwv.push((n.into(), v));
            }
        }
        let o = vm.call(
            &cls,
            vec![num(x0), num(y0), num(x1 - x0), num(y1 - y0), alpha],
            kwv,
        )?;
        rv.push(o);
    }
    Ok(py::list(vm, rv))
}

// Segments ---------------------------------------------------------------------------------------

const TS_FIELDS: &[&str] = &[
    "antialias",
    "vertical",
    "font",
    "size",
    "bold",
    "italic",
    "underline",
    "strikethrough",
    "color",
    "black_color",
    "hyperlink",
    "kerning",
    "cps",
    "ruby_top",
    "ruby_bottom",
    "hinting",
    "outline_color",
    "ignore",
    "default_font",
    "shaper",
    "instance",
    "axis",
    "shader",
    "features",
];

fn ts_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let source = arg_or(&args[1..], kw, 0, "source", Value::None);
    if !py::is_none(&source) {
        for n in TS_FIELDS {
            if let Some(v) = py::get_opt(vm, &source, n)? {
                py::set(vm, this, n, v)?;
            }
        }
        let font = py::get_or_none(vm, &source, "font")?;
        let map = py::config(vm, "font_name_map").unwrap_or(Value::None);
        if !py::is_none(&map)
            && !py::is_none(&font)
            && crysirror_python::ops::hash(vm, &font).is_ok()
        {
            let f = vm.call_method(&map, "get", vec![font.clone(), font])?;
            py::set(vm, this, "font", f)?;
        }
    } else {
        for (n, v) in [
            ("antialias", Value::Bool(true)),
            ("vertical", Value::Bool(false)),
            ("font", Value::None),
            ("hyperlink", Value::Int(0)),
            ("cps", Value::Int(0)),
            ("ruby_top", Value::Bool(false)),
            ("ruby_bottom", Value::Bool(false)),
            ("ignore", Value::Bool(false)),
            ("default_font", Value::Bool(true)),
            ("shader", Value::None),
            ("axis", Value::None),
            ("instance", Value::None),
            ("features", Value::None),
        ] {
            py::set(vm, this, n, v)?;
        }
    }
    Ok(Value::None)
}

fn ts_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut parts = Vec::new();
    for n in [
        "font",
        "size",
        "bold",
        "italic",
        "underline",
        "color",
        "black_color",
        "hyperlink",
        "vertical",
    ] {
        let v = py::get_or_none(vm, &args[0], n)?;
        parts.push(format!("{n}={}", ops::str(vm, &v)?));
    }
    Ok(vm
        .types
        .new_string(format!("<TextSegment {}>", parts.join(", "))))
}

fn layout_call(vm: &mut Vm, layout: &Value, method: &str, v: Value) -> PyResult<Value> {
    if py::is_none(layout) {
        return Ok(v);
    }
    vm.call_method(layout, method, vec![v])
}

fn ts_take_style(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let style = arg_or(&args[1..], kw, 0, "style", Value::None);
    let layout = arg_or(&args[1..], kw, 1, "layout", Value::None);
    let pick = |vm: &mut Vm, attr: &str, field: &str| -> PyResult<()> {
        let v = py::get_or_none(vm, &style, attr)?;
        if !py::is_none(&v) {
            py::set(vm, this, field, v)?;
        } else if py::get_opt(vm, this, field)?.is_none() {
            py::set(vm, this, field, Value::None)?;
        }
        Ok(())
    };
    for (a, f) in [
        ("antialias", "antialias"),
        ("font", "font"),
        ("size", "size"),
        ("bold", "bold"),
        ("italic", "italic"),
        ("hinting", "hinting"),
        ("color", "color"),
        ("black_color", "black_color"),
    ] {
        pick(vm, a, f)?;
    }
    let font = py::get_or_none(vm, this, "font")?;
    if !py::is_none(&font) {
        let map = py::config(vm, "font_name_map").unwrap_or(Value::None);
        if !py::is_none(&map) {
            let f = vm.call_method(&map, "get", vec![font.clone(), font])?;
            py::set(vm, this, "font", f)?;
        }
    }
    let ul = py::get_or_none(vm, &style, "underline")?;
    let ul = if matches!(ul, Value::Int(_)) {
        layout_call(vm, &layout, "scale_int", ul)?
    } else if ops::truthy(vm, &ul)? {
        Value::Int(1)
    } else {
        Value::Int(0)
    };
    py::set(vm, this, "underline", ul)?;
    let st = py::get_or_none(vm, &style, "strikethrough")?;
    let st = layout_call(vm, &layout, "scale_int", st)?;
    py::set(vm, this, "strikethrough", st)?;
    py::set(vm, this, "hyperlink", Value::None)?;
    let k = py::get_or_none(vm, &style, "kerning")?;
    let k = layout_call(vm, &layout, "scale", k)?;
    py::set(vm, this, "kerning", k)?;
    py::set(vm, this, "outline_color", Value::None)?;
    let scps = py::get_or_none(vm, &style, "slow_cps")?;
    if matches!(scps, Value::Bool(true)) {
        let p = py::import_attr(vm, "renpy.game", "preferences")?;
        let c = py::get(vm, &p, "text_cps")?;
        py::set(vm, this, "cps", c)?;
    }
    let mul = py::get_or_none(vm, &style, "slow_cps_multiplier")?;
    if !py::is_none(&mul) {
        let cps = py::get_or_none(vm, this, "cps")?;
        let cps = if py::is_none(&cps) {
            Value::Int(0)
        } else {
            cps
        };
        let v = ops::binary_op(
            vm,
            &cps,
            &mul,
            crysirror_python::ast::BinOp::Mult,
            false,
            false,
        )?;
        py::set(vm, this, "cps", v)?;
    }
    let rel = Rel::of(vm);
    if rel.since_pair(7, 2) {
        let sh = py::get_or_none(vm, &style, "shaper")?;
        py::set(vm, this, "shaper", sh)?;
        pick(vm, "axis", "axis")?;
        pick(vm, "instance", "instance")?;
    }
    if rel.at_least(8, 4) {
        pick(vm, "font_features", "features")?;
    }
    if rel.since_pair(8, 3) {
        let ts = py::get_or_none(vm, &style, "textshader")?;
        let g = py::import_attr(vm, "renpy.text.shader", "get_textshader")?;
        let s = vm.call(&g, vec![ts], Vec::new())?;
        py::set(vm, this, "shader", s)?;
    }
    Ok(Value::None)
}

fn get_font_for(vm: &mut Vm, this: &Value, layout: &Value, outline: Value) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let gf = py::import_attr(vm, "renpy.text.font", "get_font")?;
    let mut a = Vec::new();
    for n in ["font", "size", "bold", "italic"] {
        a.push(py::get_or_none(vm, this, n)?);
    }
    a.push(outline);
    a.push(py::get_or_none(vm, this, "antialias")?);
    a.push(py::get_or_none(vm, this, "vertical")?);
    if rel.at_least(6, 99) {
        a.push(py::get_or_none(vm, this, "hinting")?);
        let os = if py::is_none(layout) {
            Value::Float(1.)
        } else {
            py::get_or_none(vm, layout, "oversample")?
        };
        a.push(if py::is_none(&os) {
            Value::Float(1.)
        } else {
            os
        });
    }
    if rel.since_pair(7, 2) {
        for n in ["shaper", "instance", "axis"] {
            a.push(py::get_or_none(vm, this, n)?);
        }
    }
    if rel.at_least(8, 4) {
        a.push(py::get_or_none(vm, this, "features")?);
    }
    vm.call(&gf, a, Vec::new())
}

fn ts_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = arg_or(&args[1..], kw, 0, "s", Value::None);
    let layout = arg_or(&args[1..], kw, 1, "layout", Value::None);
    let level = arg_or(&args[1..], kw, 2, "level", Value::Int(0));
    let ign = py::get_or_none(vm, this, "ignore")?;
    if ops::truthy(vm, &ign)? {
        return Ok(py::list(vm, Vec::new()));
    }
    let fo = get_font_for(vm, this, &layout, Value::Int(0))?;
    let rel = Rel::of(vm);
    let mut ga = vec![s];
    if rel.at_least(8, 4) || rel.since_pair(7, 2) {
        ga.push(level);
    }
    let rv = vm.call_method(&fo, "glyphs", ga)?;
    let ts = py::import(vm, "renpy.text.textsupport")?;
    let mut kern = py::get_or_none(vm, this, "kerning")?;
    if py::is_none(&kern) {
        kern = Value::Int(0);
    }
    let mut k = py::f64_of(vm, &kern)?;
    if rel.at_least(7, 4) {
        let size = py::get_or_none(vm, this, "size")?;
        let size = if py::is_none(&size) {
            0.
        } else {
            py::f64_of(vm, &size)?
        };
        let p = py::import_attr(vm, "renpy.game", "preferences")?;
        if let Some(fk) = py::get_opt(vm, &p, "font_kerning")? {
            k += size / 30. * py::f64_of(vm, &fk).unwrap_or(0.);
        }
    }
    if k != 0. {
        vm.call_method(&ts, "kerning", vec![rv.clone(), Value::Float(k)])?;
    }
    let h = py::get_or_none(vm, this, "hyperlink")?;
    if ops::truthy(vm, &h)? {
        for g in py::items(vm, &rv)? {
            py::set(vm, &g, "hyperlink", h.clone())?;
        }
    }
    mark_ruby(vm, this, &rv)?;
    let sh = py::get_or_none(vm, this, "shader")?;
    if !py::is_none(&sh) {
        for g in py::items(vm, &rv)? {
            py::set(vm, &g, "shader", sh.clone())?;
        }
    }
    Ok(rv)
}

fn mark_ruby(vm: &mut Vm, this: &Value, rv: &Value) -> PyResult<()> {
    let ts = py::import(vm, "renpy.text.textsupport")?;
    let rb = py::get_or_none(vm, this, "ruby_bottom")?;
    let rt = py::get_or_none(vm, this, "ruby_top")?;
    if ops::truthy(vm, &rb)? {
        vm.call_method(&ts, "mark_ruby_bottom", vec![rv.clone()])?;
    } else if rt.as_str() == Some("alt") {
        vm.call_method(&ts, "mark_altruby_top", vec![rv.clone()])?;
    } else if ops::truthy(vm, &rt)? {
        vm.call_method(&ts, "mark_ruby_top", vec![rv.clone()])?;
    }
    Ok(())
}

fn ts_draw(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "TextSegment.draw: CrySirror renders text as retained glyph runs; drawing glyphs into a pixel surface is not available.",
    ))
}

fn ts_assign_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let gt = arg_or(&args[1..], kw, 0, "gt", Value::Float(0.));
    let glyphs = arg_or(&args[1..], kw, 1, "glyphs", Value::None);
    let cps = py::get_or_none(vm, &args[0], "cps")?;
    let cps = if py::is_none(&cps) {
        Value::Int(0)
    } else {
        cps
    };
    let at = py::import_attr(vm, "renpy.text.textsupport", "assign_times")?;
    vm.call(&at, vec![gt, cps, glyphs], Vec::new())
}

fn ts_subsegment(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = arg_or(&args[1..], kw, 0, "s", Value::None);
    let mut tf = py::get_or_none(vm, this, "font")?;
    if Rel::of(vm).at_least(7, 0) {
        let p = py::import_attr(vm, "renpy.game", "preferences")?;
        let ft = py::get_or_none(vm, &p, "font_transform")?;
        if !py::is_none(&ft) {
            let fts = py::config(vm, "font_transforms")?;
            let f = vm.call_method(&fts, "get", vec![ft, Value::None])?;
            if !py::is_none(&f) {
                tf = vm.call(&f, vec![tf], Vec::new())?;
            }
        }
    }
    let cls = py::import_attr(vm, MODULE, "TextSegment")?;
    let font = py::get_or_none(vm, this, "font")?;
    let is_group = tf.obj().is_some_and(|o| o.class_name() == "FontGroup");
    let mut rv = Vec::new();
    if !is_group {
        if py::same(&font, &tf) {
            rv.push(py::tuple(vm, vec![this.clone(), s]));
        } else {
            let seg = vm.call(&cls, vec![this.clone()], Vec::new())?;
            py::set(vm, &seg, "font", tf)?;
            rv.push(py::tuple(vm, vec![seg, s]));
        }
        return Ok(py::list(vm, rv));
    }
    let s = py::string(vm, &s)?;
    let mut segs: Vec<(Value, Value)> = Vec::new();
    for (f, ss) in super::font::fontgroup_segment(vm, &tf, &s)? {
        let mut found = None;
        for (k, v) in &segs {
            if ops::eq(vm, k, &f)? {
                found = Some(v.clone());
            }
        }
        let seg = match found {
            Some(v) => v,
            None => {
                let seg = vm.call(&cls, vec![this.clone()], Vec::new())?;
                py::set(vm, &seg, "font", f.clone())?;
                segs.push((f, seg.clone()));
                seg
            }
        };
        let ss = vm.types.new_string(ss);
        rv.push(py::tuple(vm, vec![seg, ss]));
    }
    Ok(py::list(vm, rv))
}

fn ts_bounds(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let glyphs = arg_or(&args[1..], kw, 0, "glyphs", Value::None);
    let bounds = arg_or(&args[1..], kw, 1, "bounds", Value::None);
    let layout = arg_or(&args[1..], kw, 2, "layout", Value::None);
    let fo = get_font_for(vm, this, &layout, Value::Int(0))?;
    vm.call_method(&fo, "bounds", vec![glyphs, bounds])
}

fn new_glyph(vm: &mut Vm) -> PyResult<Value> {
    let cls = py::import_attr(vm, "renpy.text.textsupport", "Glyph")?;
    vm.call(&cls, Vec::new(), Vec::new())
}

fn scale_int(vm: &mut Vm, layout: &Value, v: Value) -> PyResult<Value> {
    if py::is_none(layout) {
        return Ok(Value::Int(py::f64_of(vm, &v)?.round() as i64));
    }
    vm.call_method(layout, "scale_int", vec![v])
}

fn sp_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ts = arg_or(&args[1..], kw, 0, "ts", Value::None);
    let w = arg_or(&args[1..], kw, 1, "width", Value::Int(0));
    let h = arg_or(&args[1..], kw, 2, "height", Value::Int(0));
    py::set(vm, this, "width", w)?;
    py::set(vm, this, "height", h)?;
    py::set(vm, this, "ts", ts)?;
    Ok(Value::None)
}

fn sp_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layout = arg_or(&args[1..], kw, 1, "layout", Value::None);
    let g = new_glyph(vm)?;
    py::set(vm, &g, "character", Value::Int(0))?;
    py::set(vm, &g, "ascent", Value::Int(1))?;
    let h = py::get(vm, this, "height")?;
    let h = scale_int(vm, &layout, h)?;
    py::set(vm, &g, "line_spacing", h)?;
    let w = py::get(vm, this, "width")?;
    let w = scale_int(vm, &layout, w)?;
    py::set(vm, &g, "advance", w.clone())?;
    py::set(vm, &g, "width", w)?;
    let ts = py::get(vm, this, "ts")?;
    let hl = py::get_or_none(vm, &ts, "hyperlink")?;
    if ops::truthy(vm, &hl)? {
        py::set(vm, &g, "hyperlink", hl)?;
    }
    if Rel::of(vm).since_pair(8, 3) {
        let sh = py::get_or_none(vm, &ts, "shader")?;
        py::set(vm, &g, "shader", sh)?;
    }
    Ok(py::list(vm, vec![g]))
}

fn passthrough_bounds(_: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    Ok(arg_or(&args[1..], kw, 1, "bounds", Value::None))
}

fn no_draw(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// One glyph shown after `1/cps` more seconds.
fn one_glyph_time(vm: &mut Vm, cps: &Value, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut gt = py::f64_of(vm, &arg_or(&args[1..], kw, 0, "gt", Value::Float(0.)))?;
    let glyphs = arg_or(&args[1..], kw, 1, "glyphs", Value::None);
    let items = py::items(vm, &glyphs)?;
    let Some(g) = items.first() else {
        return Ok(Value::Float(gt));
    };
    let cps = if py::is_none(cps) {
        0.
    } else {
        py::f64_of(vm, cps)?
    };
    if cps != 0. {
        gt += 1.0 / cps;
    }
    py::set(vm, g, "time", Value::Float(gt))?;
    Ok(Value::Float(gt))
}

fn sp_assign_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ts = py::get(vm, &args[0], "ts")?;
    let cps = py::get_or_none(vm, &ts, "cps")?;
    one_glyph_time(vm, &cps, args, kw)
}

fn ds_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ts = arg_or(&args[1..], kw, 0, "ts", Value::None);
    let d = arg_or(&args[1..], kw, 1, "d", Value::None);
    let renders = arg_or(&args[1..], kw, 2, "renders", Value::None);
    py::set(vm, this, "d", d.clone())?;
    let rend = ops::getitem(vm, &renders, d)?;
    let size = vm.call_method(&rend, "get_size", Vec::new())?;
    let size = py::items(vm, &size)?;
    py::set(vm, this, "width", size[0].clone())?;
    py::set(vm, this, "height", size[1].clone())?;
    for n in ["hyperlink", "cps", "ruby_top", "ruby_bottom", "shader"] {
        let v = py::get_or_none(vm, &ts, n)?;
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn ds_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let layout = arg_or(&args[1..], kw, 1, "layout", Value::None);
    let d = py::get(vm, this, "d")?;
    let cls = d.obj().map(|o| o.class_name()).unwrap_or_default();
    if cls == "Null" {
        let w = py::get_or_none(vm, &d, "width")?;
        let h = py::get_or_none(vm, &d, "height")?;
        let zero = |vm: &mut Vm, v: &Value| -> PyResult<bool> {
            Ok(!py::is_none(v) && py::f64_of(vm, v)? == 0.)
        };
        if zero(vm, &w)? && zero(vm, &h)? {
            return Ok(py::list(vm, Vec::new()));
        }
    }
    let g = new_glyph(vm)?;
    let w = py::get(vm, this, "width")?;
    let w = scale_int(vm, &layout, w)?;
    let h = py::get(vm, this, "height")?;
    let h = scale_int(vm, &layout, h)?;
    py::set(vm, &g, "character", Value::Int(0xfffc))?;
    py::set(vm, &g, "ascent", Value::Int(0))?;
    py::set(vm, &g, "line_spacing", h)?;
    let adv = if cls == "CaretBlink" {
        Value::Int(0)
    } else {
        w.clone()
    };
    py::set(vm, &g, "advance", adv)?;
    py::set(vm, &g, "width", w)?;
    if Rel::of(vm).since_pair(8, 3) {
        let sh = py::get_or_none(vm, this, "shader")?;
        py::set(vm, &g, "shader", sh)?;
    }
    let hl = py::get_or_none(vm, this, "hyperlink")?;
    if ops::truthy(vm, &hl)? {
        py::set(vm, &g, "hyperlink", hl)?;
    }
    let rv = py::list(vm, vec![g]);
    mark_ruby(vm, this, &rv)?;
    Ok(rv)
}

/// Adds `(d, x, y, width, ascent, line_spacing, time)` to the draw info's
/// displayable blits (the layout places the displayable there).
fn ds_draw(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let glyphs = arg_or(&args[1..], kw, 0, "glyphs", Value::None);
    let di = arg_or(&args[1..], kw, 1, "di", Value::None);
    let items = py::items(vm, &glyphs)?;
    let Some(g) = items.first() else {
        return Ok(Value::None);
    };
    let blits = py::get_or_none(vm, &di, "displayable_blits")?;
    if py::is_none(&blits) {
        return Ok(Value::None);
    }
    let d = py::get(vm, &args[0], "d")?;
    let rel = Rel::of(vm);
    let mut t = vec![d];
    let fields: &[&str] = if rel.at_least(7, 0) {
        &["x", "y", "width", "ascent", "line_spacing", "time"]
    } else {
        &["x", "y", "time"]
    };
    for n in fields {
        t.push(py::get(vm, g, n)?);
    }
    let t = py::tuple(vm, t);
    vm.call_method(&blits, "append", vec![t])?;
    Ok(Value::None)
}

fn ds_assign_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cps = py::get_or_none(vm, &args[0], "cps")?;
    one_glyph_time(vm, &cps, args, kw)
}

fn fs_glyphs(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::list(vm, Vec::new()))
}

fn fs_assign_times(_: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    Ok(arg_or(&args[1..], kw, 0, "gt", Value::Float(0.)))
}

// Layout -----------------------------------------------------------------------------------------

fn layout_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = &args[1..];
    let text = arg_or(a, kw, 0, "text", Value::None);
    let width = py::f64_of(vm, &arg_or(a, kw, 1, "width", Value::Int(4096)))?;
    let height = py::f64_of(vm, &arg_or(a, kw, 2, "height", Value::Int(4096)))?;
    let renders = arg_or(a, kw, 3, "renders", Value::None);
    let size_only = arg_or(a, kw, 4, "size_only", Value::Bool(false));
    let splits_from = arg_or(a, kw, 5, "splits_from", Value::None);
    py::set(vm, this, "oversample", Value::Float(1.0))?;
    py::set(vm, this, "reverse", Value::None)?;
    py::set(vm, this, "forward", Value::None)?;
    py::set(vm, this, "outline_step", Value::Bool(false))?;
    py::set(vm, this, "size_only", size_only)?;
    super::text::fill_layout_object(vm, this, &text, width, height, &renders, &splits_from)?;
    // Python layouts join the current layout cache generation.
    let cache = py::import_attr(vm, MODULE, "layout_cache_new")?;
    ops::setitem(vm, &cache, Value::Int(text.id()), this.clone())?;
    Ok(Value::None)
}

fn layout_scale(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let n = arg_or(&args[1..], kw, 0, "n", Value::None);
    if py::is_none(&n) {
        return Ok(n);
    }
    let os = py::get(vm, &args[0], "oversample")?;
    ops::binary_op(
        vm,
        &n,
        &os,
        crysirror_python::ast::BinOp::Mult,
        false,
        false,
    )
}

fn layout_scale_int(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let n = arg_or(&args[1..], kw, 0, "n", Value::None);
    if py::is_none(&n) {
        return Ok(n);
    }
    if n.obj().is_some_and(|o| o.class_name() == "absolute") {
        return Ok(Value::Int(py::f64_of(vm, &n)? as i64));
    }
    let os = py::get(vm, &args[0], "oversample")?;
    let os = py::f64_of(vm, &os)?;
    let v = py::f64_of(vm, &n)? * os;
    Ok(Value::Int(v.round_ties_even() as i64))
}

fn layout_scale_outline(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let n = arg_or(&args[1..], kw, 0, "n", Value::None);
    if py::is_none(&n) {
        return Ok(n);
    }
    let os = py::get(vm, &args[0], "oversample")?;
    let os = py::f64_of(vm, &os)?;
    let v = py::f64_of(vm, &n)?;
    if v == 0. {
        return Ok(Value::Int(0));
    }
    let mut rv = (v * os).round_ties_even();
    if v < 0. && rv > -1. {
        rv = -1.;
    }
    if v > 0. && rv < 1. {
        rv = 1.;
    }
    Ok(Value::Int(rv as i64))
}

fn layout_unscale_pair(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &arg_or(&args[1..], kw, 0, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(&args[1..], kw, 1, "y", Value::Int(0)))?;
    let os = py::get(vm, &args[0], "oversample")?;
    let os = py::f64_of(vm, &os)?;
    Ok(py::tuple(
        vm,
        vec![Value::Float(x / os), Value::Float(y / os)],
    ))
}

// TextMeshDisplayable ----------------------------------------------------------------------------

fn tm_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = Value::Obj(super::core::displayable_class(vm));
    let init = py::get(vm, &d, "__init__")?;
    vm.call(&init, vec![this.clone()], Vec::new())?;
    for (i, n) in ["width", "height", "tex", "shader", "mesh", "uniforms"]
        .iter()
        .enumerate()
    {
        let v = arg_or(&args[1..], kw, i, n, Value::None);
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn tm_render(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "TextMeshDisplayable.render: text meshes with GL2 text shaders are not available; \
         CrySirror renders text as retained glyph runs.",
    ))
}

// Module -----------------------------------------------------------------------------------------

/// Rebinds the module's cache generations, as upstream's `text_tick`.
pub fn tick_module(vm: &mut Vm, clear: bool) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    let rel = Rel::of(vm);
    let pairs: &[(&str, &str)] = if rel.at_least(6, 99) {
        &[
            ("layout_cache_old", "layout_cache_new"),
            ("virtual_layout_cache_old", "virtual_layout_cache_new"),
        ]
    } else {
        &[("layout_cache_old", "layout_cache_new")]
    };
    for (old, new) in pairs {
        if clear {
            py::set(vm, &m, old, vm.types.new_dict())?;
        } else {
            let n = py::get(vm, &m, new)?;
            py::set(vm, &m, old, n)?;
        }
        py::set(vm, &m, new, vm.types.new_dict())?;
    }
    if !clear && rel.at_least(7, 0) {
        py::set(vm, &m, "slow_text", py::list(vm, Vec::new()))?;
    }
    Ok(())
}

pub fn install_names(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let rel = Rel::of(vm);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let blit = py::class(
        vm,
        MODULE,
        "Blit",
        &[],
        &[("__init__", blit_init), ("__repr__", blit_repr)],
        &[],
    )?;
    set(vm, "Blit", Value::Obj(blit));
    let ob = vm.types.new_builtin("outline_blits", f_outline_blits);
    set(vm, "outline_blits", ob);
    let di = py::class(
        vm,
        MODULE,
        "DrawInfo",
        &[],
        &[],
        &[
            ("surface", Value::None),
            ("override_color", Value::None),
            ("outline", Value::Int(0)),
            ("displayable_blits", Value::None),
        ],
    )?;
    set(vm, "DrawInfo", Value::Obj(di));
    let ts = py::class(
        vm,
        MODULE,
        "TextSegment",
        &[],
        &[
            ("__init__", ts_init),
            ("__repr__", ts_repr),
            ("take_style", ts_take_style),
            ("glyphs", ts_glyphs),
            ("draw", ts_draw),
            ("assign_times", ts_assign_times),
            ("subsegment", ts_subsegment),
            ("bounds", ts_bounds),
        ],
        &[],
    )?;
    set(vm, "TextSegment", Value::Obj(ts));
    let sp = py::class(
        vm,
        MODULE,
        "SpaceSegment",
        &[],
        &[
            ("__init__", sp_init),
            ("glyphs", sp_glyphs),
            ("bounds", passthrough_bounds),
            ("draw", no_draw),
            ("assign_times", sp_assign_times),
        ],
        &[],
    )?;
    set(vm, "SpaceSegment", Value::Obj(sp));
    let ds = py::class(
        vm,
        MODULE,
        "DisplayableSegment",
        &[],
        &[
            ("__init__", ds_init),
            ("glyphs", ds_glyphs),
            ("draw", ds_draw),
            ("assign_times", ds_assign_times),
            ("bounds", passthrough_bounds),
        ],
        &[],
    )?;
    set(vm, "DisplayableSegment", Value::Obj(ds));
    let fs = py::class(
        vm,
        MODULE,
        "FlagSegment",
        &[],
        &[
            ("glyphs", fs_glyphs),
            ("draw", no_draw),
            ("assign_times", fs_assign_times),
            ("bounds", passthrough_bounds),
        ],
        &[],
    )?;
    set(vm, "FlagSegment", Value::Obj(fs));
    let lay = py::class(
        vm,
        MODULE,
        "Layout",
        &[],
        &[
            ("__init__", layout_init),
            ("scale", layout_scale),
            ("scale_int", layout_scale_int),
            ("scale_outline", layout_scale_outline),
            ("unscale_pair", layout_unscale_pair),
        ],
        &[],
    )?;
    set(vm, "Layout", Value::Obj(lay));
    if rel.since_pair(8, 3) {
        let d = Value::Obj(super::core::displayable_class(vm));
        let tm = py::class(
            vm,
            MODULE,
            "TextMeshDisplayable",
            &[d],
            &[("__init__", tm_init), ("render", tm_render)],
            &[],
        )?;
        set(vm, "TextMeshDisplayable", Value::Obj(tm));
        // Re-exported from the GL2 polygon module when that is present.
        if let Ok(p) = py::import_attr(vm, "renpy.gl2.gl2polygon", "Polygon") {
            set(vm, "Polygon", p);
        }
    }
    set(vm, "LAYOUT_CACHE_SIZE", Value::Int(50));
    set(vm, "layout_cache_old", vm.types.new_dict());
    set(vm, "layout_cache_new", vm.types.new_dict());
    if rel.at_least(6, 99) {
        set(vm, "virtual_layout_cache_old", vm.types.new_dict());
        set(vm, "virtual_layout_cache_new", vm.types.new_dict());
    } else {
        let f = vm.types.new_builtin("layout_cache_tick", |vm, _, _| {
            super::text::text_tick();
            tick_module(vm, false)?;
            Ok(Value::None)
        });
        set(vm, "layout_cache_tick", f);
    }
    if rel.at_least(7, 0) {
        set(vm, "slow_text", py::list(vm, Vec::new()));
    }
    let m2 = py::import_attr(vm, "renpy.display.matrix", "Matrix2D")?;
    let rv = vm.call(
        &m2,
        vec![Value::Int(0), Value::Int(-1), Value::Int(1), Value::Int(0)],
        Vec::new(),
    )?;
    set(vm, "VERT_REVERSE", rv);
    let fw = vm.call(
        &m2,
        vec![Value::Int(0), Value::Int(1), Value::Int(-1), Value::Int(0)],
        Vec::new(),
    )?;
    set(vm, "VERT_FORWARD", fw);
    if rel.since_pair(7, 2) {
        let e = super::textunicode::emoji_root(vm)?;
        set(vm, "emoji", e);
        set(
            vm,
            "UNQUALIFIED",
            Value::Int(crysirror_text::emoji::UNQUALIFIED as i64),
        );
    }
    if rel.at_least(8, 5) {
        for (n, v) in super::textunicode::BIDI_NAMES {
            set(vm, n, Value::Int(*v));
        }
        let l = vm
            .types
            .new_builtin("log2vis", super::textunicode::f_log2vis);
        set(vm, "log2vis", l);
        let g = vm.types.new_builtin(
            "get_embedding_levels",
            super::textunicode::f_get_embedding_levels,
        );
        set(vm, "get_embedding_levels", g);
    }
    if rel.at_least(8, 4) {
        use super::textunicode::{LTR, ON, RTL, WLTR, WRTL};
        let d = vm.types.new_dict();
        ops::setitem(vm, &d, Value::None, Value::Int(ON))?;
        for (k, v) in [("ltr", LTR), ("rtl", RTL), ("wltr", WLTR), ("wrtl", WRTL)] {
            let k = vm.types.new_str(k);
            ops::setitem(vm, &d, k, Value::Int(v))?;
        }
        set(vm, "READING_ORDER", d);
    }
    if rel.at_least(8, 6) {
        if let Ok(a) = py::import_attr(vm, "renpy.display.position", "absolute") {
            set(vm, "absolute", a);
        }
    }
    if !rel.at_least(7, 3) {
        // BASELINE is a 7.3+ name.
        let n = vm.types.new_str("BASELINE");
        let _ = ops::delattr(vm, &Value::Obj(m.clone()), &n);
    }
    Ok(())
}
