// SPDX-License-Identifier: MPL-2.0
//! Ren'Py's software surface layer, 6.18 – 8.6 (MIT): the `_renpy`
//! extension (`module/_renpy.pyx` over `module/core.c`),
//! `renpy.display.module`, `renpy.display.pgrender`, `renpy.display.scale`
//! and `renpy.display.accelerator`.
//!
//! Pixel operations run on pygame surfaces (`pygame_surface.rs`) through
//! the shared fixed-point raster operations of
//! `crysirror_image::fixed_raster`, which keep core.c's integer
//! arithmetic. Surfaces store bytes in R, G, B, A order with masks
//! `0xff`, `0xff00`, `0xff0000`, `0xff000000`, so the byte-offset tables of
//! `module.py` resolve as on a little-endian host. A surface without alpha
//! takes upstream's 24-bit paths: its fourth byte stays 255.
//!
//! `accelerator.transform_render` / `RenderTransform` render through the
//! native Transform pipeline (`transform::render_transform`).
use super::py::{self, *};
use super::pygame_surface::{SurfRef, surf};
use crysirror_image::fixed_raster::{self as fr, Px, PxMut, ScaleRects};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const RENPY_C: &str = "_renpy";
const MODULE: &str = "renpy.display.module";
const PGRENDER: &str = "renpy.display.pgrender";
const SCALE: &str = "renpy.display.scale";
const ACCEL: &str = "renpy.display.accelerator";

fn rel(vm: &Vm) -> (u32, u32) {
    super::state::try_rt(vm).map(|r| (r.profile.version[0], r.profile.version[1])).unwrap_or((8, 6))
}

fn err(vm: &Vm, e: anyhow::Error) -> crysirror_python::PyErr {
    py::error(vm, e.to_string())
}

fn sz(s: &SurfRef) -> (usize, usize) {
    (s.w.max(0) as usize, s.h.max(0) as usize)
}

fn table(vm: &mut Vm, v: &Value) -> PyResult<[u8; 256]> {
    let b: Vec<u8> = match ops::bytes_of(v) {
        Some(b) => b.to_vec(),
        None => py::string(vm, v)?.chars().map(|c| c as u32 as u8).collect(),
    };
    if b.len() < 256 {
        return Err(vm.value_error("map tables must have 256 entries"));
    }
    Ok(b[..256].try_into().unwrap())
}

fn f32a(vm: &mut Vm, args: &[Value], i: usize) -> PyResult<f32> {
    Ok(py::f64_of(vm, &args[i])? as f32)
}

fn ia(vm: &mut Vm, args: &[Value], i: usize) -> PyResult<i64> {
    py::int(vm, &args[i])
}

fn need(vm: &Vm, args: &[Value], n: usize, name: &str) -> PyResult<()> {
    if args.len() < n {
        return Err(vm.type_error(format!("{name}() takes at least {n} arguments")));
    }
    Ok(())
}

// _renpy ------------------------------------------------------------------------------------

fn c_pixellate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 6, "pixellate")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let v: Vec<usize> = (2..6).map(|i| ia(vm, args, i).map(|v| v.max(1) as usize)).collect::<PyResult<_>>()?;
    let src = s.packed();
    let mut out = d.packed();
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    fr::pixellate(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, v[0], v[1], v[2], v[3])
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_map(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 6, "map")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    if sz(&s) != sz(&d) {
        return Err(py::error(vm, "map requires both surfaces to be the same size."));
    }
    let mut t = Vec::new();
    for i in 2..6 {
        t.push(table(vm, &args[i])?);
    }
    let src = s.packed();
    let mut out = d.packed();
    let (w, h) = sz(&s);
    let alpha = if s.alpha { Some(&t[3]) } else { None };
    fr::channel_map(&Px { data: &src, w, h }, &mut PxMut { data: &mut out, w, h }, [Some(&t[0]), Some(&t[1]), Some(&t[2]), alpha])
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_linmap(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 6, "linmap")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    if sz(&s) != sz(&d) {
        return Err(py::error(vm, "linmap requires both surfaces to be the same size."));
    }
    let m: Vec<i32> = (2..6).map(|i| ia(vm, args, i).map(|v| v as i32)).collect::<PyResult<_>>()?;
    let src = s.packed();
    let mut out = d.packed();
    let (w, h) = sz(&s);
    let alpha = if s.alpha { Some(m[3]) } else { None };
    fr::channel_linmap(&Px { data: &src, w, h }, &mut PxMut { data: &mut out, w, h }, [Some(m[0]), Some(m[1]), Some(m[2]), alpha])
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_blur(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 4, "blur")?;
    let (s, wk, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?, surf(vm, &args[2])?);
    if sz(&s) != sz(&wk) || sz(&s) != sz(&d) {
        return Err(py::error(vm, "blur requires all three surfaces to be the same size."));
    }
    let xrad = f32a(vm, args, 3)?;
    let yrad = match py::arg(args, kw, 4, "yrad") {
        Some(v) if !py::is_none(v) => py::f64_of(vm, v)? as f32,
        _ => xrad,
    };
    if xrad < 0.0 || yrad < 0.0 {
        return Err(py::error(vm, "blur radii must be non-negative."));
    }
    let (w, h) = sz(&s);
    let src = s.packed();
    let mut work = wk.packed();
    let mut out = d.packed();
    fr::gaussian_box_blur(&src, &mut work, &mut out, w, h, xrad, yrad).map_err(|e| err(vm, e))?;
    wk.store(&work);
    d.store(&out);
    Ok(Value::None)
}

fn c_linblur(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 3, "linblur")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    if sz(&s) != sz(&d) {
        return Err(py::error(vm, "linblur requires both surfaces to be the same size."));
    }
    let radius = ia(vm, args, 2)? as i32;
    if radius < 1 {
        return Err(py::error(vm, "linblur radius must be at least 1."));
    }
    let vertical = match py::arg(args, kw, 3, "vertical") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let (w, h) = sz(&s);
    let src = s.packed();
    let mut out = d.packed();
    fr::linear_blur(&src, &mut out, w, h, radius, vertical).map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_alpha_munge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "alpha_munge")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let (sc, dc) = (ia(vm, args, 2)? as usize & 3, ia(vm, args, 3)? as usize & 3);
    let amap = table(vm, &args[4])?;
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let src = s.packed();
    let mut out = d.packed();
    fr::alpha_munge(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, sc, dc, &amap)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

/// `bilinear(src, dst, source_xoff=0, ..., precise=0)`.
fn c_bilinear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 2, "bilinear")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let names = ["source_xoff", "source_yoff", "source_width", "source_height", "dest_xoff", "dest_yoff", "dest_width", "dest_height"];
    let mut v = [0.0f32, 0.0, sw as f32, sh as f32, 0.0, 0.0, dw as f32, dh as f32];
    for (i, n) in names.iter().enumerate() {
        if let Some(x) = py::arg(args, kw, 2 + i, n)
            && !py::is_none(x)
        {
            v[i] = py::f64_of(vm, x)? as f32;
        }
    }
    let precise = match py::kw(kw, "precise").or(args.get(10)) {
        Some(p) => ops::truthy(vm, p)?,
        None => false,
    };
    let src = s.packed();
    let mut out = d.packed();
    let r = ScaleRects { sx: v[0], sy: v[1], sw: v[2], sh: v[3], dx: v[4], dy: v[5], dw: v[6], dh: v[7] };
    fr::bilinear(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, r, precise)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

/// `transform(src, dst, corner_x, corner_y, xdx, ydx, xdy, ydy, a=1.0,
/// precise=0)`; the sampled alpha comes from the source's alpha byte.
fn c_transform(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 8, "transform")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let v: Vec<f32> = (2..8).map(|i| f32a(vm, args, i)).collect::<PyResult<_>>()?;
    let a = match py::arg(args, kw, 8, "a") {
        Some(x) => py::f64_of(vm, x)? as f32,
        None => 1.0,
    };
    let precise = match py::arg(args, kw, 9, "precise") {
        Some(p) => ops::truthy(vm, p)?,
        None => false,
    };
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let src = s.packed();
    let mut out = d.packed();
    fr::affine_blend(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, [v[0], v[1]], v[2], v[3], v[4], v[5], 24, a, precise)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_blend(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 4, "blend")?;
    let (a, b, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?, surf(vm, &args[2])?);
    let alpha = ia(vm, args, 3)?.clamp(0, 256) as u32;
    let (pa, pb) = (a.packed(), b.packed());
    let mut out = d.packed();
    let ((aw, ah), (bw, bh), (dw, dh)) = (sz(&a), sz(&b), sz(&d));
    fr::blend(&Px { data: &pa, w: aw, h: ah }, &Px { data: &pb, w: bw, h: bh }, &mut PxMut { data: &mut out, w: dw, h: dh }, alpha)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_imageblend(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 6, "imageblend")?;
    let (a, b, d, img) = (surf(vm, &args[0])?, surf(vm, &args[1])?, surf(vm, &args[2])?, surf(vm, &args[3])?);
    let off = ia(vm, args, 4)? as usize & 3;
    let amap = table(vm, &args[5])?;
    let (pa, pb, pi) = (a.packed(), b.packed(), img.packed());
    let mut out = d.packed();
    let ((aw, ah), (bw, bh), (dw, dh), (iw, ih)) = (sz(&a), sz(&b), sz(&d), sz(&img));
    fr::image_blend(
        &Px { data: &pa, w: aw, h: ah },
        &Px { data: &pb, w: bw, h: bh },
        &mut PxMut { data: &mut out, w: dw, h: dh },
        &Px { data: &pi, w: iw, h: ih },
        off,
        &amap,
    )
    .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

fn c_colormatrix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 22, "colormatrix")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let mut m = [[0.0f32; 5]; 4];
    for r in 0..4 {
        for c in 0..5 {
            m[r][c] = f32a(vm, args, 2 + r * 5 + c)?;
        }
    }
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let src = s.packed();
    let mut out = d.packed();
    fr::color_matrix(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, &m)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

/// `subpixel(src, dst, x, y, shift)`: draws `src` at a fractional
/// position with bilinear coverage (a unit-scale `transform`).
fn c_subpixel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "subpixel")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let (x, y) = (f32a(vm, args, 2)?, f32a(vm, args, 3)?);
    let shift = ia(vm, args, 4)?.clamp(0, 24) as u32;
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let src = s.packed();
    let mut out = d.packed();
    fr::affine_blend(&Px { data: &src, w: sw, h: sh }, &mut PxMut { data: &mut out, w: dw, h: dh }, [-x, -y], 1.0, 0.0, 0.0, 1.0, shift, 1.0, true)
        .map_err(|e| err(vm, e))?;
    d.store(&out);
    Ok(Value::None)
}

/// `save_png(surf, file, compress=-1)`.
fn c_save_png(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 2, "save_png")?;
    let s = surf(vm, &args[0])?;
    let (w, h) = sz(&s);
    let img = crysirror_image::ImageSurface { width: w as u32, height: h as u32, rgba: s.packed() };
    let png = crysirror_image::encode_png(&img).map_err(|e| err(vm, e))?;
    let _ = py::arg(args, kw, 2, "compress");
    let b = vm.types.new_bytes(png);
    let f = &args[1];
    if f.as_str().is_some() {
        let open = py::import_attr(vm, "builtins", "open")?;
        let mode = vm.types.new_str("wb");
        let fh = vm.call(&open, vec![f.clone(), mode], Vec::new())?;
        vm.call_method(&fh, "write", vec![b])?;
        vm.call_method(&fh, "close", Vec::new())?;
    } else {
        vm.call_method(f, "write", vec![b])?;
    }
    Ok(Value::None)
}

fn c_staticgray(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 8, "staticgray")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let m: Vec<i32> = (2..6).map(|i| ia(vm, args, i).map(|v| v as i32)).collect::<PyResult<_>>()?;
    let shift = ia(vm, args, 6)?.clamp(0, 31) as u32;
    let vmap: Vec<u8> = match ops::bytes_of(&args[7]) {
        Some(b) => b.to_vec(),
        None => py::string(vm, &args[7])?.chars().map(|c| c as u32 as u8).collect(),
    };
    let (w, h) = sz(&s);
    let src = s.packed();
    // The destination is an 8-bit surface; ours keep four bytes, so the
    // grey value fills the colour channels.
    let mut grey = vec![0u8; w * h];
    fr::static_gray(&Px { data: &src, w, h }, &mut grey, w, [m[0], m[1], m[2], m[3]], shift, &vmap).map_err(|e| err(vm, e))?;
    let mut out = d.packed();
    let (dw, dh) = sz(&d);
    for y in 0..h.min(dh) {
        for x in 0..w.min(dw) {
            let g = grey[y * w + x];
            let p = (y * dw + x) * 4;
            out[p..p + 3].copy_from_slice(&[g, g, g]);
        }
    }
    d.store(&out);
    Ok(Value::None)
}

fn init_c(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut funcs: Vec<(&'static str, NativeFn)> = vec![
        ("version", |_, _, _| Ok(Value::Int(6_99_00_00))),
        ("save_png", c_save_png),
        ("pixellate", c_pixellate),
        ("map", c_map),
        ("linmap", c_linmap),
        ("alpha_munge", c_alpha_munge),
        ("bilinear", c_bilinear),
        ("transform", c_transform),
        ("blend", c_blend),
        ("imageblend", c_imageblend),
        ("colormatrix", c_colormatrix),
        ("subpixel", c_subpixel),
        ("staticgray", c_staticgray),
    ];
    if rel(vm) >= (6, 99) {
        funcs.push(("blur", c_blur));
        funcs.push(("linblur", c_linblur));
    }
    Ok(crysirror_python::modules::module_with(vm, RENPY_C, &funcs))
}

type NativeFn = crysirror_python::object::NativeFn;

// renpy.display.module --------------------------------------------------------------------

fn masks(vm: &mut Vm, s: &Value) -> PyResult<Vec<i64>> {
    let m = vm.call_method(s, "get_masks", Vec::new())?;
    let mut out = Vec::new();
    for v in py::items(vm, &m)? {
        out.push(py::int(vm, &v)?);
    }
    Ok(out)
}

fn c(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, RENPY_C, name)?;
    vm.call(&f, args, kw)
}

fn m_convert_and_call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 3, "convert_and_call")?;
    let (ms, md) = (masks(vm, &args[1])?, masks(vm, &args[2])?);
    if (md[3] != 0) != (ms[3] != 0) {
        return Err(py::error(vm, "Surface alphas do not match."));
    }
    vm.call(&args[0], args[1..].to_vec(), Vec::new())
}

fn convert_and_call(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, RENPY_C, name)?;
    let mut a = vec![f];
    a.extend(args);
    m_convert_and_call(vm, &a, &[])
}

/// `byte_offset(src)`: the cached per-channel byte offsets.
fn byte_offset(vm: &mut Vm, src: &Value) -> PyResult<Value> {
    let m = py::import(vm, MODULE)?;
    let cache = py::get(vm, &m, "bo_cache")?;
    if !py::is_none(&cache) {
        return Ok(cache);
    }
    let bo32 = py::get(vm, &m, "bo32")?;
    let mut out = Vec::new();
    for mk in masks(vm, src)? {
        out.push(ops::getitem(vm, &bo32, Value::Int(mk))?);
    }
    let l = vm.types.new_list(out);
    py::set(vm, &m, "bo_cache", l.clone())?;
    Ok(l)
}

fn m_byte_offset(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    byte_offset(vm, &args[0])
}

fn endian_order(vm: &mut Vm, src: &Value, v: [Value; 4]) -> PyResult<Vec<Value>> {
    let bo = byte_offset(vm, src)?;
    let bo = py::items(vm, &bo)?;
    let mut rv = vec![v[3].clone(), v[3].clone(), v[3].clone(), v[3].clone()];
    for (val, idx) in v.iter().zip(bo) {
        let i = py::int(vm, &idx)? as usize;
        rv[i] = val.clone();
    }
    Ok(rv)
}

fn m_endian_order(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "endian_order")?;
    let v = endian_order(vm, &args[0], [args[1].clone(), args[2].clone(), args[3].clone(), args[4].clone()])?;
    Ok(vm.types.new_list(v))
}

fn m_pixellate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    convert_and_call(vm, "pixellate", args.to_vec())
}

fn m_scale(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "scale")?;
    let surface = py::import_attr(vm, PGRENDER, "surface")?;
    let d = vm.call(&surface, vec![args[1].clone(), Value::Bool(true)], Vec::new())?;
    m_bilinear_scale(vm, &[args[0].clone(), d.clone()], &[])?;
    Ok(d)
}

fn mapped(vm: &mut Vm, name: &str, args: &[Value]) -> PyResult<Value> {
    need(vm, args, 6, name)?;
    let o = endian_order(vm, &args[1], [args[2].clone(), args[3].clone(), args[4].clone(), args[5].clone()])?;
    let mut a = vec![args[0].clone(), args[1].clone()];
    a.extend(o);
    convert_and_call(vm, name, a)
}

fn m_linmap(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mapped(vm, "linmap", args)
}

fn m_map(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    mapped(vm, "map", args)
}

fn m_blur(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 4, "blur")?;
    let yrad = py::arg(args, kw, 4, "yrad").cloned().unwrap_or(Value::None);
    convert_and_call(vm, "blur", vec![args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone(), yrad])
}

fn m_twomap(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 4, "twomap")?;
    let w = py::items(vm, &args[2])?;
    let b = py::items(vm, &args[3])?;
    let wi: Vec<i64> = w.iter().take(4).map(|v| py::int(vm, v)).collect::<PyResult<_>>()?;
    let bi: Vec<i64> = b.iter().take(3).map(|v| py::int(vm, v)).collect::<PyResult<_>>()?;
    if bi == [0, 0, 0] {
        let a: Vec<Value> = wi.iter().map(|v| Value::Int(v + 1)).collect();
        return m_linmap(vm, &[args[0].clone(), args[1].clone(), a[0].clone(), a[1].clone(), a[2].clone(), a[3].clone()], &[]);
    }
    let ramp = py::import_attr(vm, "renpy.display.im", "ramp")?;
    let mut maps = Vec::new();
    for (lo, hi) in [(bi[0], wi[0]), (bi[1], wi[1]), (bi[2], wi[2]), (0, wi[3])] {
        maps.push(vm.call(&ramp, vec![Value::Int(lo), Value::Int(hi)], Vec::new())?);
    }
    m_map(vm, &[args[0].clone(), args[1].clone(), maps[0].clone(), maps[1].clone(), maps[2].clone(), maps[3].clone()], &[])
}

fn m_alpha_munge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 3, "alpha_munge")?;
    let a = vm.call_method(&args[0], "get_size", Vec::new())?;
    let b = vm.call_method(&args[1], "get_size", Vec::new())?;
    if !ops::eq(vm, &a, &b)? {
        return Ok(Value::None);
    }
    let so = byte_offset(vm, &args[0])?;
    let red = ops::getitem(vm, &so, Value::Int(0))?;
    let dobo = byte_offset(vm, &args[1])?;
    let alpha = ops::getitem(vm, &dobo, Value::Int(3))?;
    if !py::is_none(&red) && !py::is_none(&alpha) {
        c(vm, "alpha_munge", vec![args[0].clone(), args[1].clone(), red, alpha, args[2].clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn m_bilinear_scale(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 2, "bilinear_scale")?;
    let mut src = args[0].clone();
    let dst = args[1].clone();
    let names = ["sx", "sy", "sw", "sh", "dx", "dy", "dw", "dh"];
    let mut v: Vec<Value> = (0..8).map(|i| py::arg(args, kw, 2 + i, names[i]).cloned().unwrap_or(Value::None)).collect();
    for i in [0, 1, 4, 5] {
        if py::is_none(&v[i]) {
            v[i] = Value::Int(0);
        }
    }
    let precise = py::arg(args, kw, 10, "precise").cloned().unwrap_or(Value::Int(0));
    if py::is_none(&v[2]) {
        let s = vm.call_method(&src, "get_size", Vec::new())?;
        let s = py::items(vm, &s)?;
        v[2] = s[0].clone();
        v[3] = s[1].clone();
    }
    if py::is_none(&v[6]) {
        let s = vm.call_method(&dst, "get_size", Vec::new())?;
        let s = py::items(vm, &s)?;
        v[6] = s[0].clone();
        v[7] = s[1].clone();
    }
    let (mut sw, mut sh) = (py::int(vm, &v[2])?, py::int(vm, &v[3])?);
    let (dw, dh) = (py::int(vm, &v[6])?, py::int(vm, &v[7])?);
    loop {
        if sw <= dw * 2 && sh <= dh * 2 {
            break;
        }
        let (nsw, nsh) = ((sw / 2).max(dw), (sh / 2).max(dh));
        let surface = py::import_attr(vm, PGRENDER, "surface")?;
        let a = masks(vm, &src)?[3];
        let size = vm.types.new_tuple(vec![Value::Int(nsw), Value::Int(nsh)]);
        let nsrc = vm.call(&surface, vec![size, Value::Int(a)], Vec::new())?;
        c(
            vm,
            "bilinear",
            vec![src.clone(), nsrc.clone(), v[0].clone(), v[1].clone(), Value::Int(sw), Value::Int(sh)],
            vec![("precise".into(), precise.clone())],
        )?;
        v[0] = Value::Int(0);
        v[1] = Value::Int(0);
        sw = nsw;
        sh = nsh;
        src = nsrc;
    }
    c(
        vm,
        "bilinear",
        vec![src, dst, v[0].clone(), v[1].clone(), Value::Int(sw), Value::Int(sh), v[4].clone(), v[5].clone(), v[6].clone(), v[7].clone()],
        vec![("precise".into(), precise)],
    )
}

fn m_imageblend(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "imageblend")?;
    let bo = byte_offset(vm, &args[3])?;
    let alpha = ops::getitem(vm, &bo, Value::Int(3))?;
    c(vm, "imageblend", vec![args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone(), alpha, args[4].clone()], Vec::new())
}

fn m_colormatrix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 3, "colormatrix")?;
    let m = py::items(vm, &args[2])?;
    if m.len() < 20 {
        return Err(vm.value_error("colormatrix needs 20 values"));
    }
    let bo = byte_offset(vm, &args[0])?;
    let mut offs = Vec::new();
    for i in py::items(vm, &bo)? {
        offs.push(py::int(vm, &i)? as usize);
    }
    let mut o = [0usize; 4];
    for i in 0..4 {
        o[offs[i]] = i;
    }
    let mut a = vec![args[0].clone(), args[1].clone()];
    for r in 0..4 {
        for col in 0..4 {
            a.push(m[o[r] * 5 + o[col]].clone());
        }
        a.push(m[o[r] * 5 + 4].clone());
    }
    c(vm, "colormatrix", a, Vec::new())
}

fn m_subpixel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 4, "subpixel")?;
    let shifts = vm.call_method(&args[0], "get_shifts", Vec::new())?;
    let shift = ops::getitem(vm, &shifts, Value::Int(3))?;
    c(vm, "subpixel", vec![args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone(), shift], Vec::new())
}

fn m_premultiply_alpha(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 1, "premultiply_alpha")?;
    let s = surf(vm, &args[0])?;
    let mut px = s.packed();
    for p in px.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    s.store(&px);
    Ok(Value::None)
}

fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let r = rel(vm);
    let mut funcs: Vec<(&'static str, NativeFn)> = vec![
        ("convert_and_call", m_convert_and_call),
        ("pixellate", m_pixellate),
        ("scale", m_scale),
        ("byte_offset", m_byte_offset),
        ("endian_order", m_endian_order),
        ("linmap", m_linmap),
        ("map", m_map),
        ("twomap", m_twomap),
        ("alpha_munge", m_alpha_munge),
        ("bilinear_scale", m_bilinear_scale),
        ("imageblend", m_imageblend),
        ("colormatrix", m_colormatrix),
        ("subpixel", m_subpixel),
    ];
    if r >= (6, 99) {
        funcs.push(("blur", m_blur));
    }
    if r >= (8, 6) {
        funcs.push(("premultiply_alpha", m_premultiply_alpha));
    }
    let m = crysirror_python::modules::module_with(vm, MODULE, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let bo32 = vm.types.new_dict();
    for (k, v) in [(255i64, 0i64), (65280, 1), (16711680, 2), (4278190080, 3), (-16777216, 3)] {
        ops::setitem(vm, &bo32, Value::Int(k), Value::Int(v))?;
    }
    set(vm, "bo32", bo32);
    set(vm, "bo_cache", Value::None);
    for n in ["save_png", "blend"] {
        let v = py::import_attr(vm, RENPY_C, n)?;
        set(vm, n, v);
    }
    let t = py::import_attr(vm, RENPY_C, "transform")?;
    // 6.18 – 7.x name it `transform`; 8.x binds it as `self` too.
    if r < (8, 0) {
        set(vm, "transform", t.clone());
    }
    if r >= (8, 0) {
        set(vm, "self", t);
    }
    Ok(m)
}

// renpy.display.pgrender ------------------------------------------------------------------

fn pg(vm: &mut Vm, sub: &str, name: &str) -> PyResult<Value> {
    py::import_attr(vm, &format!("pygame_sdl2.{sub}"), name)
}

/// `surface((w, h), alpha)`: a subsurface of a surface with a 2-pixel
/// border.
fn p_surface(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let size = py::arg_or(args, kw, 0, "rect", Value::None);
    let mut alpha = py::arg_or(args, kw, 1, "alpha", Value::Bool(true));
    let cls = pg(vm, "surface", "Surface")?;
    let surface_t = cls.obj().cloned().ok_or_else(|| vm.type_error("Surface"))?;
    if ops::isinstance(vm, &alpha, &surface_t) {
        let m = masks(vm, &alpha)?;
        alpha = Value::Bool(m[3] != 0);
    }
    let wh = py::items(vm, &size)?;
    let (w, h) = (py::int(vm, &wh[0])?, py::int(vm, &wh[1])?);
    let flags = if ops::truthy(vm, &alpha)? { super::pygame_surface::SRCALPHA } else { 0 };
    let big = vm.types.new_tuple(vec![Value::Int(w + 4), Value::Int(h + 4)]);
    let s = vm.call(&cls, vec![big, Value::Int(flags), Value::Int(32)], Vec::new())?;
    let r = vm.types.new_tuple(vec![Value::Int(2), Value::Int(2), Value::Int(w), Value::Int(h)]);
    vm.call_method(&s, "subsurface", vec![r])
}

fn p_copy_surface(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::arg_or(args, kw, 0, "surf", Value::None);
    let alpha = py::arg_or(args, kw, 1, "alpha", Value::Bool(true));
    let size = vm.call_method(&s, "get_size", Vec::new())?;
    let rv = p_surface(vm, &[size, alpha], &[])?;
    a_nogil_copy(vm, &[s, rv.clone()], &[])?;
    Ok(rv)
}

fn p_load_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::arg_or(args, kw, 0, "f", Value::None);
    let filename = py::arg_or(args, kw, 1, "filename", Value::None);
    let size = py::arg_or(args, kw, 2, "size", Value::None);
    let load = pg(vm, "image", "load")?;
    let name = py::string(vm, &filename)?;
    let mut kwargs = Vec::new();
    if !py::is_none(&size) && rel(vm) >= (8, 1) {
        kwargs.push(("size".into(), size));
    }
    let surf = match vm.call(&load, vec![f, filename], kwargs) {
        Ok(s) => s,
        Err(e) => {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            let known = ["png", "jpg", "jpeg", "webp", "avif", "tga", "bmp", "ico", "svg"];
            let extra = if known.contains(&ext.as_str()) {
                String::new()
            } else {
                format!(" ({ext} files are not supported by Ren'Py)")
            };
            let msg = match &e {
                crysirror_python::PyErr::Exc(o) => {
                    ops::repr(vm, &Value::Obj(o.clone()))?
                }
                _ => String::new(),
            };
            return Err(py::error(vm, format!("Could not load image '{name}'{extra}: {msg}")));
        }
    };
    p_copy_surface(vm, &[surf], &[])
}

fn p_wrap(vm: &mut Vm, sub: &str, f: &str, args: Vec<Value>, alpha_from_result: bool) -> PyResult<Value> {
    let func = pg(vm, sub, f)?;
    let s = vm.call(&func, args, Vec::new())?;
    let alpha = if alpha_from_result { s.clone() } else { Value::Bool(true) };
    p_copy_surface(vm, &[s, alpha], &[])
}

fn p_flip(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_wrap(vm, "transform", "flip", args.to_vec(), false)
}

fn p_rotozoom(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_wrap(vm, "transform", "rotozoom", args.to_vec(), false)
}

fn p_transform_scale(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_wrap(vm, "transform", "scale", args.to_vec(), true)
}

fn p_transform_rotate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_wrap(vm, "transform", "rotate", args.to_vec(), false)
}

/// `set_rgba_masks()`: the sample surfaces of the running masks.
fn p_set_rgba_masks(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cls = pg(vm, "surface", "Surface")?;
    let m = py::import(vm, PGRENDER)?;
    for (n, flags) in [("sample_alpha", super::pygame_surface::SRCALPHA), ("sample_noalpha", 0)] {
        let size = vm.types.new_tuple(vec![Value::Int(10), Value::Int(10)]);
        let s = vm.call(&cls, vec![size, Value::Int(flags), Value::Int(32)], Vec::new())?;
        py::set(vm, &m, n, s)?;
    }
    if let Ok(f) = py::import_attr(vm, "renpy.audio.audio", "sample_surfaces") {
        let a = py::get(vm, &m, "sample_noalpha")?;
        let b = py::get(vm, &m, "sample_alpha")?;
        vm.call(&f, vec![a, b], Vec::new())?;
    }
    Ok(Value::None)
}

fn s_convert_alpha(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_copy_surface(vm, &[args[0].clone(), Value::Bool(true)], &[])
}

fn s_convert(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_copy_surface(vm, &[args[0].clone(), Value::Bool(false)], &[])
}

fn s_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_copy_surface(vm, &[args[0].clone(), args[0].clone()], &[])
}

fn init_pgrender(vm: &mut Vm) -> PyResult<ObjRef> {
    let r = rel(vm);
    let funcs: Vec<(&'static str, NativeFn)> = vec![
        ("set_rgba_masks", p_set_rgba_masks),
        ("surface", p_surface),
        ("surface_unscaled", p_surface),
        ("copy_surface", p_copy_surface),
        ("copy_surface_unscaled", p_copy_surface),
        ("load_image", p_load_image),
        ("load_image_unscaled", p_load_image),
        ("flip", p_flip),
        ("flip_unscaled", p_flip),
        ("rotozoom", p_rotozoom),
        ("rotozoom_unscaled", p_rotozoom),
        ("transform_scale", p_transform_scale),
        ("transform_scale_unscaled", p_transform_scale),
        ("transform_rotate", p_transform_rotate),
        ("transform_rotate_unscaled", p_transform_rotate),
    ];
    let m = crysirror_python::modules::module_with(vm, PGRENDER, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "sample_alpha", Value::None);
    set(vm, "sample_noalpha", Value::None);
    let base = pg(vm, "surface", "Surface")?;
    let cls = py::class(
        vm,
        PGRENDER,
        "Surface",
        &[base],
        &[("convert_alpha", s_convert_alpha), ("convert", s_convert), ("copy", s_copy)],
        &[],
    )?;
    set(vm, "Surface", Value::Obj(cls));
    let mut safe = vec!["png", "jpg", "jpeg"];
    if r >= (6, 99) {
        safe.push("webp");
    }
    let safe: Vec<Value> = safe.iter().map(|s| vm.types.new_str(s)).collect();
    let l = vm.types.new_list(safe);
    let set_t = Value::Obj(vm.types.set.clone());
    let safe = vm.call(&set_t, vec![l], Vec::new())?;
    set(vm, "safe_formats", safe);
    let rlock = py::import_attr(vm, "threading", "RLock")?;
    let lock = vm.call(&rlock, Vec::new(), Vec::new())?;
    set(vm, "image_load_lock", lock);
    if r >= (8, 0) {
        let d = vm.types.new_dict();
        for (k, init) in [("png", "INIT_PNG"), ("jpg", "INIT_JPG"), ("jpeg", "INIT_JPG"), ("webp", "INIT_WEBP"), ("avif", "INIT_AVIF")] {
            let v = pg(vm, "image", init).unwrap_or(Value::Int(0));
            let k = vm.types.new_str(k);
            ops::setitem(vm, &d, k, v)?;
        }
        for k in ["tga", "bmp", "ico", "svg"] {
            let k = vm.types.new_str(k);
            ops::setitem(vm, &d, k, Value::Int(0))?;
        }
        set(vm, "formats", d);
    }
    Ok(m)
}

// renpy.display.scale ---------------------------------------------------------------------

fn sc_identity(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args.first().cloned().unwrap_or(Value::None))
}

fn sc_real_bilinear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "real_bilinear")?;
    let rv = p_surface(vm, &[args[1].clone(), args[0].clone()], &[])?;
    m_bilinear_scale(vm, &[args[0].clone(), rv.clone()], &[])?;
    Ok(rv)
}

fn sc_real_transform_scale(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_transform_scale(vm, args, &[])
}

fn sc_image_load_unscaled(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "image_load_unscaled")?;
    p_load_image(vm, &args[..2], &[])
}

fn sc_image_save_unscaled(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "image_save_unscaled")?;
    let save = pg(vm, "image", "save")?;
    vm.call(&save, args[..2].to_vec(), Vec::new())
}

/// `real_smoothscale(src, size, dest=None)`: pixellate down by powers of
/// two, then a precise transform to the size.
fn sc_real_smoothscale(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    need(vm, args, 2, "real_smoothscale")?;
    let mut src = args[0].clone();
    let wh = py::items(vm, &args[1])?;
    let (width, height) = (py::int(vm, &wh[0])?, py::int(vm, &wh[1])?);
    let ssz = vm.call_method(&src, "get_size", Vec::new())?;
    let ssz = py::items(vm, &ssz)?;
    let (sw, sh) = (py::int(vm, &ssz[0])?, py::int(vm, &ssz[1])?);
    let (mut iw, mut ih) = (sw, sh);
    let dest = match py::arg(args, kw, 2, "dest") {
        Some(d) if !py::is_none(d) => d.clone(),
        _ => p_surface(vm, &[args[1].clone(), src.clone()], &[])?,
    };
    if width == 0 || height == 0 {
        return Ok(dest);
    }
    let (mut xs, mut ys) = (1, 1);
    while iw >= width * 2 {
        xs *= 2;
        iw /= 2;
    }
    while ih >= height * 2 {
        ys *= 2;
        ih /= 2;
    }
    if iw != sw || ih != sh {
        let size = vm.types.new_tuple(vec![Value::Int(iw), Value::Int(ih)]);
        let inter = p_surface(vm, &[size, src.clone()], &[])?;
        c(vm, "pixellate", vec![src, inter.clone(), Value::Int(xs), Value::Int(ys), Value::Int(1), Value::Int(1)], Vec::new())?;
        src = inter;
    }
    c(
        vm,
        "transform",
        vec![
            src,
            dest.clone(),
            Value::Int(0),
            Value::Int(0),
            Value::Float(iw as f64 / width as f64),
            Value::Int(0),
            Value::Int(0),
            Value::Float(ih as f64 / height as f64),
        ],
        vec![("precise".into(), Value::Int(1))],
    )?;
    Ok(dest)
}

fn init_scale(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        SCALE,
        &[
            ("real", sc_identity),
            ("scale", sc_identity),
            ("surface_scale", sc_identity),
            ("real_bilinear", sc_real_bilinear),
            ("real_transform_scale", sc_real_transform_scale),
            ("image_load_unscaled", sc_image_load_unscaled),
            ("image_save_unscaled", sc_image_save_unscaled),
            ("real_smoothscale", sc_real_smoothscale),
            ("smoothscale", sc_real_smoothscale),
        ],
    );
    for (n, src) in [("real_renpy_pixellate", "pixellate"), ("real_renpy_transform", "transform")] {
        let v = py::import_attr(vm, RENPY_C, src)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    Ok(m)
}

// renpy.display.accelerator ---------------------------------------------------------------

/// `nogil_copy(src, dest)`: a non-blending blit at the origin.
fn a_nogil_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "nogil_copy")?;
    let (s, d) = (surf(vm, &args[0])?, surf(vm, &args[1])?);
    let ((sw, sh), (dw, dh)) = (sz(&s), sz(&d));
    let src = s.packed();
    let mut out = d.packed();
    for y in 0..sh.min(dh) {
        let n = sw.min(dw) * 4;
        out[y * dw * 4..y * dw * 4 + n].copy_from_slice(&src[y * sw * 4..y * sw * 4 + n]);
    }
    d.store(&out);
    Ok(Value::None)
}

fn a_get_poi(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let state = &args[0];
    let point_to = py::get_or_none(vm, state, "point_to")?;
    if matches!(point_to.payload(), Some(crysirror_python::object::Payload::Tuple(t)) if t.len() == 3) {
        return Ok(point_to);
    }
    let camera = py::import_attr(vm, "renpy.display.transform", "Camera")?;
    if let Some(c) = camera.obj()
        && ops::isinstance(vm, &point_to, c)
    {
        let persp = py::get_or_none(vm, state, "perspective")?;
        if ops::truthy(vm, &persp)? {
            return Err(py::error(vm, "The point_to transform property for camera should not be True."));
        }
        let layer = py::get(vm, &point_to, "layer")?;
        let ctx = py::import_attr(vm, "renpy.game", "context")?;
        let ctx = vm.call(&ctx, Vec::new(), Vec::new())?;
        let sle = py::get(vm, &ctx, "scene_lists")?;
        let ct = py::get(vm, &sle, "camera_transform")?;
        let d = vm.call_method(&ct, "get", vec![layer, Value::None])?;
        let tcls = py::import_attr(vm, "renpy.display.motion", "Transform")?;
        if !tcls.obj().is_some_and(|t| ops::isinstance(vm, &d, t)) {
            return Ok(Value::None);
        }
        let mut perspective = py::get_or_none(vm, &d, "perspective")?;
        let cfg = py::config(vm, "perspective")?;
        if matches!(perspective, Value::Bool(true)) {
            perspective = cfg;
        } else if matches!(perspective, Value::Int(_) | Value::Float(_)) {
            let c = py::items(vm, &cfg)?;
            perspective = vm.types.new_tuple(vec![c[0].clone(), perspective, c[2].clone()]);
        }
        if !ops::truthy(vm, &perspective)? {
            return Ok(Value::None);
        }
        let z11 = py::items(vm, &perspective)?[1].clone();
        let (w, h) = (py::config(vm, "screen_width")?, py::config(vm, "screen_height")?);
        let mut pl = Vec::new();
        for n in ["xpos", "ypos", "xanchor", "yanchor", "xoffset", "yoffset"] {
            pl.push(py::get_or_none(vm, &d, n)?);
        }
        pl.push(Value::Bool(true));
        let placement = vm.types.new_tuple(pl);
        let place = py::import_attr(vm, "renpy.display.displayable", "place")
            .or_else(|_| py::import_attr(vm, "renpy.display.core", "place"))?;
        let xy = vm.call(&place, vec![w.clone(), h.clone(), w.clone(), h.clone(), placement], Vec::new())?;
        let xy = py::items(vm, &xy)?;
        let (wf, hf) = (py::f64_of(vm, &w)?, py::f64_of(vm, &h)?);
        let x = py::f64_of(vm, &xy[0])? + wf / 2.0;
        let y = py::f64_of(vm, &xy[1])? + hf / 2.0;
        let zpos = py::get_or_none(vm, &d, "zpos")?;
        let z = py::f64_of(vm, &zpos)? + py::f64_of(vm, &z11)?;
        return Ok(vm.types.new_tuple(vec![Value::Float(x), Value::Float(y), Value::Float(z)]));
    }
    Err(py::error(vm, "The point_to transform property should be None, a 3-tuple (x, y, z), or True."))
}

fn a_relative_for_crop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 3, "relative_for_crop")?;
    let (n, base, limit) = (&args[0], &args[1], &args[2]);
    let ltc = py::config(vm, "limit_transform_crop")?;
    let absolute = py::import_attr(vm, "renpy.display.core", "absolute")?;
    if ltc.as_str() == Some("only_float") {
        let is_abs = absolute.obj().is_some_and(|a| ops::isinstance(vm, n, a));
        if matches!(n, Value::Int(_)) || is_abs {
            return Ok(n.clone());
        }
    }
    let raw = py::get(vm, &absolute, "compute_raw")?;
    let v = vm.call(&raw, vec![n.clone(), base.clone()], Vec::new())?;
    let v = py::f64_of(vm, &v)? as i64;
    if ops::truthy(vm, &ltc)? {
        let l = py::int(vm, limit)?;
        return Ok(Value::Int(v.min(l)));
    }
    Ok(Value::Int(v))
}

/// `transform_render(self, widtho, heighto, st, at)` (through 7.x).
fn a_transform_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "transform_render")?;
    let this = &args[0];
    let state = py::get(vm, this, "state")?;
    let v: Vec<f64> = (1..5).map(|i| py::f64_of(vm, &args[i])).collect::<PyResult<_>>()?;
    super::transform::render_transform(vm, this, &state, v[0], v[1], v[2], v[3])
}

fn rt_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 2, "RenderTransform")?;
    py::set(vm, &args[0], "transform", args[1].clone())?;
    let state = py::get(vm, &args[1], "state")?;
    py::set(vm, &args[0], "state", state)?;
    Ok(Value::None)
}

fn rt_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    need(vm, args, 5, "render")?;
    let t = py::get(vm, &args[0], "transform")?;
    let state = py::get(vm, &args[0], "state")?;
    let v: Vec<f64> = (1..5).map(|i| py::f64_of(vm, &args[i])).collect::<PyResult<_>>()?;
    super::transform::render_transform(vm, &t, &state, v[0], v[1], v[2], v[3])
}

fn init_accelerator(vm: &mut Vm) -> PyResult<ObjRef> {
    let r = rel(vm);
    let mut funcs: Vec<(&'static str, NativeFn)> = vec![("nogil_copy", a_nogil_copy)];
    if r < (8, 0) {
        funcs.push(("transform_render", a_transform_render));
    }
    if r >= (8, 0) {
        funcs.push(("get_poi", a_get_poi));
        funcs.push(("relative_for_crop", a_relative_for_crop));
    }
    let m = crysirror_python::modules::module_with(vm, ACCEL, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let ident = py::import_attr(vm, "renpy.display.render", "IDENTITY")?;
    set(vm, "IDENTITY", ident);
    let absolute = py::import_attr(vm, "renpy.display.core", "absolute")?;
    set(vm, "absolute", absolute);
    if r >= (7, 4) {
        set(vm, "z11", Value::Float(0.0));
    }
    if r >= (8, 0) {
        let c = py::class(vm, ACCEL, "RenderTransform", &[], &[("__init__", rt_init), ("render", rt_render)], &[])?;
        set(vm, "RenderTransform", Value::Obj(c));
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(RENPY_C, init_c);
    vm.register_native_module(MODULE, init_module);
    vm.register_native_module(PGRENDER, init_pgrender);
    vm.register_native_module(SCALE, init_scale);
    vm.register_native_module(ACCEL, init_accelerator);
}
