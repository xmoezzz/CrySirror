// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.ftfont` (all releases) and `renpy.text.hbfont` (7.7/8.2+),
//! after Ren'Py `ftfont.pyx`/`hbfont.pyx` (MIT), over the shared client
//! font library instead of a private FreeType/HarfBuzz binding.
//!
//! `FTFace`/`HBFace` name a game font file (`index@file` selects a face of a
//! collection) that the font service loads asynchronously through the
//! game's asset capability. `FTFont.glyphs` measures per character with
//! pair kerning, as FreeType does; `HBFont.glyphs` shapes the run
//! (OpenType GSUB/GPOS, features, variation coordinates of a named
//! instance and per-axis overrides). `HBFace.variations` reads the face's
//! `fvar` axes and named instances (axes after the sixteenth are ignored,
//! the first named instance is skipped, both as upstream).
//!
//! Unavailable host: CrySirror draws text as retained glyph runs keyed by
//! character, so the `draw(surface, ...)` methods, which rasterize into a
//! pixel surface, raise an exception naming that; and a face still loading
//! in the font service raises instead of blocking the script thread.
//! `bounds` unions the glyph cells (advance width by line spacing), since
//! ink bounds come from the asynchronous glyph-bounds worker.
use super::{
    font::{self, Font, Lookup},
    py::{self, *},
    textsupport::{self, Glyph, Rel},
};
use crysirror_core::service::{FontVariations, ShapeDirection, ShapeRequest};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const FT: &str = "renpy.text.ftfont";
const HB: &str = "renpy.text.hbfont";

fn unavailable(vm: &Vm, what: &str) -> crysirror_python::PyErr {
    py::error(
        vm,
        format!(
            "{what}: CrySirror renders text as retained glyph runs through its font service; \
             rasterizing glyphs into a pixel surface is not available."
        ),
    )
}

/// The `index@file` name of a face object.
fn face_name(vm: &mut Vm, face: &Value) -> PyResult<String> {
    let fname = py::get(vm, face, "fn")?;
    let fname = py::string(vm, &fname)?;
    if fname.contains('@') {
        return Ok(fname);
    }
    let index = py::get_or_none(vm, face, "index")?;
    let index = if py::is_none(&index) { 0 } else { py::int(vm, &index)? };
    Ok(if index != 0 { format!("{index}@{fname}") } else { fname })
}

fn resolve(vm: &mut Vm, name: &str, size: f64, bold: bool, italic: bool) -> PyResult<Font> {
    match font::get_font(vm, name, size, bold, italic)? {
        Lookup::Ready(f) => Ok(f),
        Lookup::Pending => Err(py::error(
            vm,
            format!("Font {name:?} is still loading in the font service; it is available after the next interaction."),
        )),
    }
}

fn face_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = arg_or(&args[1..], kw, 0, "f", Value::None);
    let index = arg_or(&args[1..], kw, 1, "index", Value::Int(0));
    let mut fname = arg_or(&args[1..], kw, 2, "fn", Value::None);
    if py::is_none(&fname) {
        // 6.18 - 7.0 pass only the file; its name identifies the face.
        fname = py::get_or_none(vm, &f, "name")?;
    }
    py::set(vm, this, "f", f)?;
    py::set(vm, this, "index", index)?;
    py::set(vm, this, "fn", fname)?;
    py::set(vm, this, "size", Value::Int(-1))?;
    Ok(Value::None)
}

fn font_init(vm: &mut Vm, args: &[Value], kw: Kw, hb: bool) -> PyResult<Value> {
    let this = &args[0];
    let rel = Rel::of(vm);
    let mut names = vec!["face", "size", "bold", "italic", "outline", "antialias", "vertical"];
    if rel.at_least(6, 99) {
        names.push("hinting");
    }
    let has_features = hb && rel.at_least(8, 4);
    if hb {
        names.extend(["instance", "axis"]);
        if has_features {
            names.push("features");
        }
    }
    let mut vals: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| arg_or(&args[1..], kw, i, n, Value::None))
        .collect();
    let face = vals[0].clone();
    let mut size = py::f64_of(vm, &vals[1])?;
    if hb {
        if size < 1. {
            size = 1.;
        }
        // A bold request selects a bold named instance when there is one.
        let variations = hbface_variations(vm, &face)?;
        if py::is_none(&vals[8]) && !py::is_none(&variations) {
            let inst = py::get(vm, &variations, "instance")?;
            let bold = py::f64_of(vm, &vals[2])?;
            let pick = |vm: &mut Vm, n: &str| -> PyResult<bool> {
                let k = vm.types.new_str(n);
                ops::contains(vm, &inst, &k)
            };
            let chosen = if bold >= 1. {
                if pick(vm, "bold")? {
                    Some("bold")
                } else if pick(vm, "bold italic")? {
                    Some("bold italic")
                } else {
                    None
                }
            } else if pick(vm, "regular")? {
                Some("regular")
            } else if pick(vm, "italic")? {
                Some("italic")
            } else {
                None
            };
            if let Some(c) = chosen {
                if bold >= 1. {
                    vals[2] = Value::Float(0.);
                }
                vals[8] = vm.types.new_str(c);
            }
        }
        if !has_features {
        } else if ops::truthy(vm, &vals[10])? {
            let fc = py::import_attr(vm, HB, "Features")?;
            let get = py::get(vm, &fc, "get")?;
            vals[10] = vm.call(&get, vec![vals[10].clone()], Vec::new())?;
        } else {
            vals[10] = Value::None;
        }
        let fname = py::get(vm, &face, "fn")?;
        let scale = py::config(vm, "ftfont_scale")
            .and_then(|m| vm.call_method(&m, "get", vec![fname, Value::Float(1.)]))
            .and_then(|v| py::f64_of(vm, &v))
            .unwrap_or(1.);
        let pref = py::import_attr(vm, "renpy.game", "preferences")
            .and_then(|p| py::get(vm, &p, "font_size"))
            .and_then(|v| py::f64_of(vm, &v))
            .unwrap_or(1.);
        size *= scale * pref;
    }
    vals[1] = Value::Float(size);
    for (n, v) in names.iter().zip(vals) {
        py::set(vm, this, n, v)?;
    }
    py::set(vm, this, "face_object", face)?;
    Ok(Value::None)
}

fn ft_font_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    font_init(vm, args, kw, false)
}

fn hb_font_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    font_init(vm, args, kw, true)
}

struct FontArgs {
    name: String,
    size: f64,
    bold: bool,
    italic: bool,
    vertical: bool,
}

fn font_args(vm: &mut Vm, this: &Value) -> PyResult<FontArgs> {
    let face = py::get(vm, this, "face_object")?;
    let name = face_name(vm, &face)?;
    let size = py::get(vm, this, "size")?;
    let size = py::f64_of(vm, &size)?;
    let b = py::get(vm, this, "bold")?;
    let bold = ops::truthy(vm, &b)?;
    let i = py::get(vm, this, "italic")?;
    let italic = ops::truthy(vm, &i)?;
    let v = py::get_or_none(vm, this, "vertical")?;
    let vertical = ops::truthy(vm, &v)?;
    Ok(FontArgs { name, size, bold, italic, vertical })
}

/// Sets `ascent`/`descent`/`height`/`lineskip` from the resolved face.
fn set_line_metrics(vm: &mut Vm, this: &Value, f: &Font) -> PyResult<(f64, f64, f64)> {
    let (ascent, descent, line) = match f {
        Font::Outline { ascent, descent, line_height, .. } => {
            (ascent.ceil(), -descent.ceil(), line_height.round())
        }
        Font::Image { data, scale } => {
            ((data.baseline * scale).round(), 0., (data.height * scale).round())
        }
    };
    py::set(vm, this, "ascent", Value::Int(ascent as i64))?;
    py::set(vm, this, "descent", Value::Int(descent as i64))?;
    py::set(vm, this, "height", Value::Int((ascent - descent) as i64))?;
    py::set(vm, this, "lineskip", Value::Int(line as i64))?;
    Ok((ascent, descent, line))
}

fn glyph_objects(vm: &mut Vm, gs: &[Glyph]) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let cls = py::import_attr(vm, "renpy.text.textsupport", "Glyph")?;
    let mut out = Vec::with_capacity(gs.len());
    for g in gs {
        let o = vm.call(&cls, Vec::new(), Vec::new())?;
        textsupport::write_glyph(vm, rel, &o, g)?;
        out.push(o);
    }
    Ok(py::list(vm, out))
}

fn ft_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::string(vm, &arg_or(&args[1..], kw, 0, "s", Value::None))?;
    let a = font_args(vm, this)?;
    let f = resolve(vm, &a.name, a.size, a.bold, a.italic)?;
    let (ascent, descent, line) = set_line_metrics(vm, this, &f)?;
    let gs: Vec<Glyph> = f
        .glyphs(vm, &s)?
        .into_iter()
        .map(|(c, m)| {
            let zw = font::is_zerowidth(c);
            Glyph {
                ch: c,
                ascent,
                descent: -descent,
                line_spacing: line,
                width: if zw { 0. } else { m.width },
                advance: if zw { 0. } else { m.advance },
                draw: !zw,
                ..Glyph::default()
            }
        })
        .collect();
    glyph_objects(vm, &gs)
}

fn tag(vm: &Vm, s: &str) -> PyResult<[u8; 4]> {
    let b = s.as_bytes();
    if b.len() < 4 {
        return Err(vm.value_error(format!("OpenType tag {s:?} needs four characters")));
    }
    Ok([b[0], b[1], b[2], b[3]])
}

/// Variation coordinates for an `HBFont` (named instance, else axis
/// defaults, then per-axis overrides clamped to the axis range).
fn coordinates(vm: &mut Vm, this: &Value, info: &FontVariations) -> PyResult<Vec<([u8; 4], f32)>> {
    let axes: Vec<_> = info.axes.iter().take(16).collect();
    let mut coords: Vec<f32> = axes.iter().map(|a| a.default).collect();
    let instance = py::get_or_none(vm, this, "instance")?;
    if !py::is_none(&instance) {
        let name = py::string(vm, &instance)?.to_lowercase();
        if let Some(inst) = info.instances.iter().skip(1).find(|i| i.name.to_lowercase() == name) {
            for (c, v) in coords.iter_mut().zip(&inst.coordinates) {
                *c = *v;
            }
        }
    }
    let axis = py::get_or_none(vm, this, "axis")?;
    if !py::is_none(&axis) {
        for (k, v) in ops::mapping_items(vm, &axis)? {
            let k = py::string(vm, &k)?.to_lowercase();
            let v = py::f64_of(vm, &v)? as f32;
            if let Some(i) = axes.iter().position(|a| a.name.to_lowercase() == k) {
                coords[i] = v.clamp(axes[i].minimum, axes[i].maximum);
            }
        }
    }
    Ok(axes.iter().zip(coords).map(|(a, c)| (a.tag, c)).collect())
}

fn hb_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::string(vm, &arg_or(&args[1..], kw, 0, "s", Value::None))?;
    let level = py::int(vm, &arg_or(&args[1..], kw, 1, "level", Value::Int(0)))?;
    let a = font_args(vm, this)?;
    let f = resolve(vm, &a.name, a.size, a.bold, a.italic)?;
    let (ascent, descent, line) = set_line_metrics(vm, this, &f)?;
    let Font::Outline { alias, .. } = &f else {
        return Err(py::error(vm, format!("{:?} is an image font, not an OpenType face.", a.name)));
    };
    let fonts = super::host(vm)
        .borrow()
        .fonts
        .clone()
        .ok_or_else(|| py::error(vm, "font service unavailable"))?;
    let info = fonts.variations(alias).map_err(|e| py::error(vm, e.to_string()))?;
    let variations = if info.axes.is_empty() { Vec::new() } else { coordinates(vm, this, &info)? };
    let mut features = Vec::new();
    let fv = py::get_or_none(vm, this, "features")?;
    if !py::is_none(&fv) {
        let list = py::get(vm, &fv, "features")?;
        for item in py::items(vm, &list)? {
            let p = py::items(vm, &item)?;
            let t = py::string(vm, &p[0])?;
            let v = py::int(vm, &p[1])?;
            features.push((tag(vm, &t)?, v as u32));
        }
    }
    let direction = match (a.vertical, level & 1 == 1) {
        (true, _) => ShapeDirection::TopToBottom,
        (false, true) => ShapeDirection::RightToLeft,
        (false, false) => ShapeDirection::LeftToRight,
    };
    let shaped = fonts
        .shape(&ShapeRequest {
            font: alias.clone(),
            size: a.size as f32,
            text: s.clone(),
            direction,
            variations,
            features,
        })
        .map_err(|e| py::error(vm, e.to_string()))?;
    let gs: Vec<Glyph> = shaped
        .iter()
        .map(|g| {
            let ch = s[g.cluster as usize..].chars().next().unwrap_or('\0');
            let (xo, yo, adv) = if a.vertical {
                (-g.y_offset, -g.x_offset, -g.y_advance)
            } else {
                (g.x_offset, -g.y_offset, g.x_advance)
            };
            Glyph {
                ch,
                glyph: g.glyph_id,
                ascent,
                descent: -descent,
                line_spacing: line,
                x_offset: xo as f64,
                y_offset: yo as f64,
                advance: adv as f64,
                width: adv as f64,
                draw: true,
                ..Glyph::default()
            }
        })
        .collect();
    glyph_objects(vm, &gs)
}

fn bounds(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let glyphs = arg_or(&args[1..], kw, 0, "glyphs", Value::None);
    let b = arg_or(&args[1..], kw, 1, "bounds", Value::None);
    let b = py::items(vm, &b)?;
    let mut r = [0f64; 4];
    for (i, v) in b.iter().take(4).enumerate() {
        r[i] = py::f64_of(vm, v)?;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (r[0], r[1], r[0] + r[2], r[1] + r[3]);
    let rel = Rel::of(vm);
    for o in py::items(vm, &glyphs)? {
        let g = textsupport::read_glyph(vm, rel, &o)?;
        if g.width == 0. {
            continue;
        }
        let gy = g.y - g.ascent;
        x0 = x0.min(g.x);
        y0 = y0.min(gy);
        x1 = x1.max(g.x + g.width);
        y1 = y1.max(gy + g.line_spacing);
    }
    let v = [x0, y0, x1 - x0, y1 - y0].map(|v| Value::Int(v as i64)).to_vec();
    Ok(py::tuple(vm, v))
}

fn ft_draw(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(unavailable(vm, "FTFont.draw"))
}

fn hb_draw(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(unavailable(vm, "HBFont.draw"))
}

fn ft_error_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let code = arg_or(&args[1..], kw, 0, "code", Value::Int(0));
    let c = py::int(vm, &code)?;
    let msg = vm.types.new_string(format!("{c}: FreeType error {c}"));
    let exc = Value::Obj(vm.types.exc("Exception"));
    let init = py::get(vm, &exc, "__init__")?;
    vm.call(&init, vec![args[0].clone(), msg], Vec::new())?;
    py::set(vm, &args[0], "code", code)?;
    Ok(Value::None)
}

/// `HBFace.variations`: None for static faces, else a `Variations`.
fn hbface_variations(vm: &mut Vm, face: &Value) -> PyResult<Value> {
    if let Some(v) = py::get_opt(vm, face, "_variations")? {
        return Ok(v);
    }
    let name = face_name(vm, face)?;
    let f = resolve(vm, &name, 16., false, false)?;
    let Font::Outline { alias, .. } = &f else {
        return Ok(Value::None);
    };
    let fonts = super::host(vm)
        .borrow()
        .fonts
        .clone()
        .ok_or_else(|| py::error(vm, "font service unavailable"))?;
    let info = fonts.variations(alias).map_err(|e| py::error(vm, e.to_string()))?;
    let rv = if info.axes.is_empty() {
        Value::None
    } else {
        let vc = py::import_attr(vm, HB, "Variations")?;
        let ac = py::import_attr(vm, HB, "Axis")?;
        let v = vm.call(&vc, Vec::new(), Vec::new())?;
        let axis = py::get(vm, &v, "axis")?;
        for (i, a) in info.axes.iter().enumerate().take(16) {
            let o = vm.call(
                &ac,
                vec![
                    Value::Int(i as i64),
                    Value::Float(a.minimum as f64),
                    Value::Float(a.default as f64),
                    Value::Float(a.maximum as f64),
                ],
                Vec::new(),
            )?;
            let k = vm.types.new_string(a.name.to_lowercase());
            ops::setitem(vm, &axis, k, o)?;
        }
        let inst = py::get(vm, &v, "instance")?;
        for (i, n) in info.instances.iter().enumerate().skip(1) {
            if n.name.is_empty() {
                continue;
            }
            let k = vm.types.new_string(n.name.to_lowercase());
            ops::setitem(vm, &inst, k, Value::Int(i as i64))?;
        }
        v
    };
    py::set(vm, face, "_variations", rv.clone())?;
    Ok(rv)
}

fn f_hbface_variations(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    hbface_variations(vm, &args[0])
}

fn axis_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    for (i, n) in ["index", "minimum", "default", "maximum"].iter().enumerate() {
        let v = arg_or(&args[1..], kw, i, n, Value::None);
        py::set(vm, &args[0], n, v)?;
    }
    Ok(Value::None)
}

fn axis_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut parts = Vec::new();
    for n in ["index", "minimum", "default", "maximum"] {
        let v = py::get(vm, &args[0], n)?;
        parts.push(format!("{n}={}", ops::str(vm, &v)?));
    }
    Ok(vm.types.new_string(format!("<Axis {}>", parts.join(" "))))
}

fn variations_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "instance", vm.types.new_dict())?;
    py::set(vm, &args[0], "axis", vm.types.new_dict())?;
    Ok(Value::None)
}

fn variations_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut rv = Vec::new();
    let inst = py::get(vm, &args[0], "instance")?;
    for (k, _) in ops::mapping_items(vm, &inst)? {
        rv.push(format!("  Named Instance: {}", ops::repr(vm, &k)?));
    }
    let axis = py::get(vm, &args[0], "axis")?;
    for (k, v) in ops::mapping_items(vm, &axis)? {
        let mut f = Vec::new();
        for n in ["minimum", "default", "maximum"] {
            let x = py::get(vm, &v, n)?;
            f.push(format!("{n}={}", ops::str(vm, &x)?));
        }
        rv.push(format!("  Axis: {} ({})", ops::repr(vm, &k)?, f.join(", ")));
    }
    Ok(vm.types.new_string(rv.join("\n")))
}

fn features_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(&args[1..], kw, 0, "features", Value::None);
    for item in py::items(vm, &f)? {
        let p = py::items(vm, &item)?;
        let t = py::string(vm, &p[0])?;
        tag(vm, &t)?;
        py::int(vm, &p[1])?;
    }
    py::set(vm, &args[0], "features", f)?;
    Ok(Value::None)
}

fn features_get(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(args, kw, 0, "features", Value::None);
    let cls = py::import_attr(vm, HB, "Features")?;
    let cache = py::get(vm, &cls, "cache")?;
    if ops::contains(vm, &cache, &f)? {
        return ops::getitem(vm, &cache, f);
    }
    let rv = vm.call(&cls, vec![f.clone()], Vec::new())?;
    ops::setitem(vm, &cache, f, rv.clone())?;
    Ok(rv)
}

fn module(vm: &mut Vm, name: &str, hb: bool) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, name, &[("init", |_, _, _| Ok(Value::None))]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let exc = Value::Obj(vm.types.exc("Exception"));
    let err = py::class(vm, name, "FreetypeError", &[exc], &[("__init__", ft_error_init)], &[])?;
    set(vm, "FreetypeError", Value::Obj(err));
    let face_name = if hb { "HBFace" } else { "FTFace" };
    let face = py::class(vm, name, face_name, &[], &[("__init__", face_init)], &[])?;
    if hb {
        let g = vm.types.new_builtin("variations", f_hbface_variations);
        let p = py::property(vm, g, None)?;
        vm.types.set_attr(&face, "variations", p);
    }
    set(vm, face_name, Value::Obj(face));
    let font_name = if hb { "HBFont" } else { "FTFont" };
    let methods: Methods = if hb {
        &[("__init__", hb_font_init), ("glyphs", hb_glyphs), ("bounds", bounds), ("draw", hb_draw)]
    } else {
        &[("__init__", ft_font_init), ("glyphs", ft_glyphs), ("bounds", bounds), ("draw", ft_draw)]
    };
    let font = py::class(vm, name, font_name, &[], methods, &[])?;
    set(vm, font_name, Value::Obj(font));
    if hb {
        let axis = py::class(vm, name, "Axis", &[], &[("__init__", axis_init), ("__repr__", axis_repr)], &[])?;
        set(vm, "Axis", Value::Obj(axis));
        let var = py::class(
            vm,
            name,
            "Variations",
            &[],
            &[("__init__", variations_init), ("__repr__", variations_repr)],
            &[("_console_always_long", Value::Bool(true))],
        )?;
        set(vm, "Variations", Value::Obj(var));
        if Rel::of(vm).at_least(8, 4) {
            let cache = vm.types.new_dict();
            let feat = py::class(vm, name, "Features", &[], &[("__init__", features_init)], &[("cache", cache)])?;
            let g = vm.types.new_builtin("get", features_get);
            let g = py::staticmethod(vm, g);
            vm.types.set_attr(&feat, "get", g);
            set(vm, "Features", Value::Obj(feat));
        }
    }
    Ok(m)
}

fn init_ft(vm: &mut Vm) -> PyResult<ObjRef> {
    module(vm, FT, false)
}

fn init_hb(vm: &mut Vm) -> PyResult<ObjRef> {
    if !Rel::of(vm).since_pair(7, 2) {
        return Err(vm.exc_msg("ModuleNotFoundError", "No module named 'renpy.text.hbfont'"));
    }
    module(vm, HB, true)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(FT, init_ft);
    vm.register_native_module(HB, init_hb);
}
