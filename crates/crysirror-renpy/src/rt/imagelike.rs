// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.imagelike`: Solid, Frame, Borders and
//! FileCurrentScreenshot, after Ren'Py 8.3 `imagelike.py` (MIT).
use super::{
    layout::{rargs, style, xyminimums},
    py::{self, *},
    render::{self, Mat, RenderData, TexKind, Texture, new_render, new_texture, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.imagelike";

fn solid_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let color = arg_or(args, kw, 1, "color", Value::None);
    let props: Vec<_> = kw.iter().filter(|(k, _)| &**k != "color").cloned().collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    let c = if py::is_none(&color) {
        Value::None
    } else {
        let f = py::import_attr(vm, "renpy.easy", "color")?;
        vm.call(&f, vec![color], Vec::new())?
    };
    py::set(vm, this, "color", c)?;
    Ok(Value::None)
}

fn solid_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "color")?;
    Ok(Value::Int(ops::hash(vm, &c)?))
}

fn solid_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = args.get(1).cloned().unwrap_or(Value::None);
    let eq = vm.call_method(&args[0], "_equals", vec![o.clone()])?;
    if !ops::truthy(vm, &eq)? {
        return Ok(Value::Bool(false));
    }
    let a = py::get_or_none(vm, &args[0], "color")?;
    let b = py::get_or_none(vm, &o, "color")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn empty_list(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_list(Vec::new()))
}

fn solid_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, _, _) = rargs(vm, args)?;
    let (xmin, ymin) = xyminimums(vm, this, w, h)?;
    let (w, h) = (w.max(xmin), h.max(ymin));
    let mut color = py::get_or_none(vm, this, "color")?;
    if !ops::truthy(vm, &color)? {
        color = style(vm, this, "color")?;
    }
    let mut rd = RenderData::new(w, h);
    if !py::is_none(&color) && w > 0. && h > 0. {
        let rgba = super::color::rgba(vm, &color)?;
        let t = new_texture(vm, Texture { kind: TexKind::Solid(rgba), width: w.max(1.), height: h.max(1.), crop: None, full: None, opaque: rgba[3] == 255 });
        let source = render::source_of(vm, &t)?;
        rd.children.push(render::Blit { source, x: 0., y: 0., focus: true, main: true });
    }
    Ok(new_render(vm, rd))
}

// Borders ------------------------------------------------------------------------------

fn borders_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["left", "top", "right", "bottom", "pad_left", "pad_top", "pad_right", "pad_bottom"].iter().enumerate() {
        let default = if i < 4 { None } else { Some(Value::Int(0)) };
        let v = match arg(args, kw, i + 1, n) {
            Some(v) => v.clone(),
            None => default.ok_or_else(|| vm.type_error(format!("Borders() missing {n}")))?,
        };
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn borders_padding(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut out = Vec::new();
    for (a, b) in [("left", "pad_left"), ("top", "pad_top"), ("right", "pad_right"), ("bottom", "pad_bottom")] {
        let x = py::get(vm, this, a)?;
        let y = py::get(vm, this, b)?;
        out.push(ops::binary_op(vm, &x, &y, crysirror_python::ast::BinOp::Add, false, false)?);
    }
    Ok(vm.types.new_tuple(out))
}

// Frame ---------------------------------------------------------------------------------

fn frame_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["image", "left", "top", "right", "bottom", "xborder", "yborder", "bilinear", "tile", "tile_ratio"];
    let mut vals = vec![Value::None, Value::None, Value::None, Value::None, Value::None, Value::Int(0), Value::Int(0), Value::Bool(true), Value::Bool(false), Value::Float(0.5)];
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    let image = super::easy::displayable(vm, &vals[0], &Value::None, true)?;
    let dup = py::get_or_none(vm, &image, "_duplicatable")?;
    py::set(vm, this, "image", image)?;
    py::set(vm, this, "_duplicatable", dup)?;
    let (mut left, mut top, mut right, mut bottom) = (vals[1].clone(), vals[2].clone(), vals[3].clone(), vals[4].clone());
    if left.obj().is_some_and(|o| o.class_name() == "Borders") {
        let b = left.clone();
        left = py::get(vm, &b, "left")?;
        top = py::get(vm, &b, "top")?;
        right = py::get(vm, &b, "right")?;
        bottom = py::get(vm, &b, "bottom")?;
    }
    py::set(vm, this, "tile", vals[8].clone())?;
    let tr = py::f64_of(vm, &vals[9])?;
    py::set(vm, this, "tile_ratio", Value::Float(tr))?;
    if py::is_none(&left) {
        left = vals[5].clone();
    }
    if py::is_none(&top) {
        top = vals[6].clone();
    }
    if py::is_none(&right) {
        right = left.clone();
    }
    if py::is_none(&bottom) {
        bottom = top.clone();
    }
    py::set(vm, this, "left", left)?;
    py::set(vm, this, "top", top)?;
    py::set(vm, this, "right", right)?;
    py::set(vm, this, "bottom", bottom)?;
    Ok(Value::None)
}

fn frame_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(2)))?;
    if v < 2 {
        let xb = py::get_or_none(vm, this, "xborder")?;
        let yb = py::get_or_none(vm, this, "yborder")?;
        py::set(vm, this, "left", xb.clone())?;
        py::set(vm, this, "right", xb)?;
        py::set(vm, this, "top", yb.clone())?;
        py::set(vm, this, "bottom", yb)?;
    }
    Ok(Value::None)
}

fn frame_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut parts = Vec::new();
    for n in ["image", "left", "top", "right", "bottom"] {
        let v = py::get_or_none(vm, this, n)?;
        parts.push(if n == "image" { ops::repr(vm, &v)?.to_string() } else { ops::str(vm, &v)?.to_string() });
    }
    let tile = py::get_or_none(vm, this, "tile")?;
    let t = if tile.as_str() == Some("integer") {
        let r = py::get_or_none(vm, this, "tile_ratio")?;
        format!(" tile ({})", ops::str(vm, &r)?)
    } else if ops::truthy(vm, &tile)? {
        " tile".into()
    } else {
        String::new()
    };
    Ok(vm.types.new_string(format!("<Frame {} ({},{},{},{}){}>", parts[0], parts[1], parts[2], parts[3], parts[4], t)))
}

fn frame_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = args.get(1).cloned().unwrap_or(Value::None);
    let eq = vm.call_method(&args[0], "_equals", vec![o.clone()])?;
    if !ops::truthy(vm, &eq)? {
        return Ok(Value::Bool(false));
    }
    for n in ["image", "left", "top", "right", "bottom", "tile", "tile_ratio"] {
        let a = py::get_or_none(vm, &args[0], n)?;
        let b = py::get_or_none(vm, &o, n)?;
        if !ops::eq(vm, &a, &b)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn num(vm: &mut Vm, this: &Value, n: &str) -> PyResult<i64> {
    let v = py::get_or_none(vm, this, n)?;
    if py::is_none(&v) { Ok(0) } else { py::int(vm, &v) }
}

fn frame_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let xmin = style(vm, this, "xminimum")?;
    let ymin = style(vm, this, "yminimum")?;
    let width = w.max(super::core::compute_raw(vm, &xmin, w)?);
    let height = h.max(super::core::compute_raw(vm, &ymin, h)?);
    let tile = py::get_or_none(vm, this, "tile")?;
    let tiled = ops::truthy(vm, &tile)?;
    let integer = tile.as_str() == Some("integer");
    let tile_ratio = { let v = py::get_or_none(vm, this, "tile_ratio")?; py::f64_of(vm, &v).unwrap_or(0.5) };
    let (dw, dh) = if tiled { (width.trunc(), height.trunc()) } else { (width, height) };
    let child = style(vm, this, "child")?;
    let image = if ops::truthy(vm, &child)? { child } else { py::get(vm, this, "image")? };
    let crend = render::render(vm, &image, width.max(1.), height.max(1.), st, at)?;
    let (sw, sh) = super::layout::size(&crend);
    let (sw, sh) = (sw.trunc() as i64, sh.trunc() as i64);
    let (l0, t0, r0, b0) = (num(vm, this, "left")?, num(vm, this, "top")?, num(vm, this, "right")?, num(vm, this, "bottom")?);
    let (bw, bh) = (l0 + r0, t0 + b0);
    let xborder = bw.min(sw - 2).min(dw as i64);
    let (left, right) = if xborder != 0 && bw != 0 { (l0 * xborder / bw, r0 * xborder / bw) } else { (0, 0) };
    let yborder = bh.min(sh - 2).min(dh as i64);
    let (top, bottom) = if yborder != 0 && bh != 0 { (t0 * yborder / bh, b0 * yborder / bh) } else { (0, 0) };
    let rv = new_render(vm, RenderData::new(dw, dh));
    let (swf, shf) = (sw as f64, sh as f64);
    let mut draw = |vm: &mut Vm, x0: i64, x1: i64, y0: i64, y1: i64| -> PyResult<()> {
        let (dx0, sx0) = if x0 >= 0 { (x0 as f64, x0 as f64) } else { (dw + x0 as f64, swf + x0 as f64) };
        let (dx1, sx1) = if x1 > 0 { (x1 as f64, x1 as f64) } else { (dw + x1 as f64, swf + x1 as f64) };
        let (dy0, sy0) = if y0 >= 0 { (y0 as f64, y0 as f64) } else { (dh + y0 as f64, shf + y0 as f64) };
        let (dy1, sy1) = if y1 > 0 { (y1 as f64, y1 as f64) } else { (dh + y1 as f64, shf + y1 as f64) };
        if sx0 == sx1 || sy0 == sy1 {
            return Ok(());
        }
        let (mut csw, mut csh) = (sx1 - sx0, sy1 - sy0);
        let (cdw, cdh) = (dx1 - dx0, dy1 - dy0);
        if csw <= 0. || csh <= 0. || cdh <= 0. || cdw <= 0. {
            return Ok(());
        }
        let mut cr = render::subsurface(vm, &crend, [sx0, sy0, csw, csh], false, false)?;
        if csw != cdw || csh != cdh {
            if tiled {
                let (mut ctw, mut cth) = (cdw, cdh);
                let tiles = |d: f64, s: f64| (((d / s).floor() as i64) + if d % s != 0. { 1 } else { 0 }).max(1);
                let mut xt = tiles(cdw, csw);
                let mut yt = tiles(cdh, csh);
                if (cdw % csw != 0. || cdh % csh != 0.) && integer {
                    if (cdw % csw) / csw < tile_ratio {
                        xt = (xt - 1).max(1);
                    }
                    if (cdh % csh) / csh < tile_ratio {
                        yt = (yt - 1).max(1);
                    }
                    ctw = csw * xt as f64;
                    cth = csh * yt as f64;
                }
                let mut nd = RenderData::new(ctw, cth);
                nd.xclipping = true;
                nd.yclipping = true;
                let src = render::source_of(vm, &cr)?;
                for x in 0..xt {
                    for y in 0..yt {
                        nd.children.push(render::Blit { source: src.clone(), x: x as f64 * csw, y: y as f64 * csh, focus: true, main: true });
                    }
                }
                csw = ctw;
                csh = cth;
                cr = new_render(vm, nd);
            }
            if csw != cdw || csh != cdh {
                let mut nd = RenderData::new(cdw, cdh);
                nd.forward = Some(Mat::scale(csw / cdw, csh / cdh, 1.));
                nd.reverse = Some(Mat::scale(cdw / csw, cdh / csh, 1.));
                let src = render::source_of(vm, &cr)?;
                nd.children.push(render::Blit { source: src, x: 0., y: 0., focus: true, main: true });
                cr = new_render(vm, nd);
            }
        }
        let src = render::source_of(vm, &cr)?;
        if let Some(r) = render_data(&rv) {
            r.borrow_mut().children.push(render::Blit { source: src, x: dx0.trunc(), y: dy0.trunc(), focus: true, main: true });
        }
        Ok(())
    };
    if top != 0 {
        if left != 0 {
            draw(vm, 0, left, 0, top)?;
        }
        draw(vm, left, -right, 0, top)?;
        if right != 0 {
            draw(vm, -right, 0, 0, top)?;
        }
    }
    if left != 0 {
        draw(vm, 0, left, top, -bottom)?;
    }
    draw(vm, left, -right, top, -bottom)?;
    if right != 0 {
        draw(vm, -right, 0, top, -bottom)?;
    }
    if bottom != 0 {
        if left != 0 {
            draw(vm, 0, left, -bottom, 0)?;
        }
        draw(vm, left, -right, -bottom, 0)?;
        if right != 0 {
            draw(vm, -right, 0, -bottom, 0)?;
        }
    }
    Ok(rv)
}

fn frame_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = arg_or(args, kw, 1, "args", Value::None);
    let image = py::get(vm, this, "image")?;
    let ni = vm.call_method(&image, "_duplicate", vec![a.clone()])?;
    if py::same(&ni, &image) {
        return Ok(this.clone());
    }
    vm.call_method(&ni, "_unique", Vec::new())?;
    let rv = vm.call_method(this, "_copy", vec![a])?;
    let dup = py::get_or_none(vm, &ni, "_duplicatable")?;
    py::set(vm, &rv, "image", ni)?;
    py::set(vm, &rv, "_duplicatable", dup)?;
    Ok(rv)
}

fn frame_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let image = py::get(vm, &args[0], "image")?;
    vm.call_method(&image, "_unique", Vec::new())?;
    py::set(vm, &args[0], "_duplicatable", Value::Bool(false))?;
    Ok(Value::None)
}

fn frame_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let image = py::get(vm, this, "image")?;
    let ni = vm.call_method(&image, "_in_current_store", Vec::new())?;
    if py::same(&ni, &image) {
        return Ok(this.clone());
    }
    let rv = vm.call_method(this, "_copy", Vec::new())?;
    py::set(vm, &rv, "image", ni)?;
    Ok(rv)
}

fn frame_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let image = py::get(vm, &args[0], "image")?;
    let mut out = vec![image];
    let c = style(vm, &args[0], "child")?;
    if !py::is_none(&c) && super::core::is_displayable(vm, &c) {
        out.push(c);
    }
    Ok(vm.types.new_list(out))
}

// FileCurrentScreenshot -------------------------------------------------------------------

fn fcs_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut empty = arg_or(args, kw, 1, "empty", Value::None);
    let props: Vec<_> = kw.iter().filter(|(k, _)| &**k != "empty").cloned().collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    if py::is_none(&empty) {
        let n = py::import_attr(vm, "renpy.display.layout", "Null")?;
        empty = vm.call(&n, Vec::new(), Vec::new())?;
    }
    py::set(vm, this, "empty", empty)?;
    Ok(Value::None)
}

fn fcs_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, at) = rargs(vm, args)?;
    let iface = py::import_attr(vm, "renpy.display", "interface")?;
    let ss = if py::is_none(&iface) { Value::None } else { py::get_or_none(vm, &iface, "screenshot_surface")? };
    if py::is_none(&ss) {
        let empty = py::get(vm, this, "empty")?;
        return render::render(vm, &empty, w, h, st, at);
    }
    let src = render::source_of(vm, &ss)?;
    let (tw, th) = src.size();
    let mut rd = RenderData::new(tw, th);
    rd.children.push(render::Blit { source: src, x: 0., y: 0., focus: true, main: true });
    Ok(new_render(vm, rd))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[]);
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods, attrs: &[(&str, Value)]| -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    c(vm, "Solid", &[d.clone()], &[("__init__", solid_init), ("__hash__", solid_hash), ("__eq__", solid_eq), ("visit", empty_list), ("render", solid_render)], &[])?;
    let b = c(vm, "Borders", &[], &[("__init__", borders_init)], &[])?;
    let g = vm.types.new_builtin("padding", borders_padding);
    let p = py::property(vm, g, None)?;
    if let Some(bo) = b.obj() {
        vm.types.set_attr(bo, "padding", p);
    }
    c(vm, "Frame", &[d.clone()], &[
        ("__init__", frame_init),
        ("after_upgrade", frame_after_upgrade),
        ("__repr__", frame_repr),
        ("__eq__", frame_eq),
        ("render", frame_render),
        ("_duplicate", frame_duplicate),
        ("_unique", frame_unique),
        ("_in_current_store", frame_in_current_store),
        ("visit", frame_visit),
    ], &[("__version__", Value::Int(1)), ("properties", vm.types.new_dict()), ("tile_ratio", Value::Float(0.5))])?;
    c(vm, "FileCurrentScreenshot", &[d], &[("__init__", fcs_init), ("render", fcs_render)], &[])?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
