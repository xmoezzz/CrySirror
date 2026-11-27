// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.transform` and the `RenderTransform` of
//! `renpy.display.accelerator`, after Ren'Py 8.3 (MIT).
//!
//! The transform's output is a Render whose reverse matrix carries zoom,
//! rotation, flips and 3D rotations, a child blit offset and clipping for
//! crops. Shader-only properties (mesh, blur, shader, perspective cameras,
//! matrixcolor and uniforms) are kept on the render's properties; the
//! layer backend reports the ones it cannot present.
use super::{
    core::compute_raw,
    py::{self, *},
    render::{self, Mat, RenderData, new_matrix, new_render, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::f64::consts::PI;

pub const MODULE: &str = "renpy.display.transform";

/// (name, default, diff kind: 2 = plain, 4 = with inherited value, 0 = none)
pub const PROPERTIES: &[(&str, Def, u8)] = &[
    ("additive", Def::F(0.), 2), ("alpha", Def::F(1.), 2), ("blend", Def::None, 2), ("blur", Def::None, 2),
    ("corner1", Def::None, 2), ("corner2", Def::None, 2), ("crop", Def::None, 2), ("crop_relative", Def::None, 2),
    ("debug", Def::None, 2), ("delay", Def::I(0), 2), ("events", Def::B(true), 2), ("fit", Def::None, 2), ("fps", Def::None, 2),
    ("matrixanchor", Def::None, 2), ("matrixcolor", Def::None, 2), ("matrixtransform", Def::None, 2), ("maxsize", Def::None, 2),
    ("mesh", Def::B(false), 0), ("mesh_pad", Def::None, 2), ("nearest", Def::None, 2), ("perspective", Def::None, 2),
    ("rotate", Def::None, 2), ("rotate_pad", Def::B(true), 2), ("point_to", Def::None, 2), ("orientation", Def::None, 2),
    ("xrotate", Def::None, 2), ("yrotate", Def::None, 2), ("zrotate", Def::None, 2), ("shader", Def::None, 0),
    ("show_cancels_hide", Def::B(true), 2), ("subpixel", Def::B(false), 2), ("transform_anchor", Def::B(false), 2),
    ("zoom", Def::F(1.), 2), ("xanchoraround", Def::F(0.5), 2), ("xanchor", Def::None, 4), ("xaround", Def::F(0.), 2),
    ("xoffset", Def::F(0.), 2), ("xpan", Def::None, 2), ("xpos", Def::None, 4), ("xsize", Def::None, 2), ("xtile", Def::I(1), 2),
    ("xzoom", Def::F(1.), 2), ("yanchoraround", Def::F(0.5), 2), ("yanchor", Def::None, 4), ("yaround", Def::F(0.), 2),
    ("yoffset", Def::F(0.), 2), ("ypan", Def::None, 2), ("ypos", Def::None, 4), ("ysize", Def::None, 2), ("ytile", Def::I(1), 2),
    ("yzoom", Def::F(1.), 2), ("zpos", Def::F(0.), 2), ("zzoom", Def::B(false), 2),
    ("gl_anisotropic", Def::None, 0), ("gl_blend_func", Def::None, 0), ("gl_color_mask", Def::None, 0), ("gl_depth", Def::None, 0),
    ("gl_drawable_resolution", Def::None, 0), ("gl_mipmap", Def::None, 0), ("gl_pixel_perfect", Def::None, 0),
    ("gl_texture_scaling", Def::None, 0), ("gl_texture_wrap", Def::None, 0),
];

pub const ALIASES: &[&str] = &[
    "alignaround", "align", "anchor", "anchorangle", "anchoraround", "anchorradius", "angle", "around", "offset", "pos", "radius",
    "size", "xalign", "xcenter", "xycenter", "xysize", "yalign", "ycenter",
];

#[derive(Clone, Copy)]
pub enum Def {
    None,
    F(f64),
    I(i64),
    B(bool),
}

fn def_value(d: Def) -> Value {
    match d {
        Def::None => Value::None,
        Def::F(f) => Value::Float(f),
        Def::I(i) => Value::Int(i),
        Def::B(b) => Value::Bool(b),
    }
}

pub fn is_property(name: &str) -> bool {
    PROPERTIES.iter().any(|p| p.0 == name) || ALIASES.contains(&name) || name.starts_with("u_") || name.starts_with("gl_")
}

fn absolute(vm: &mut Vm, v: f64) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.core", "absolute")?;
    vm.call(&c, vec![Value::Float(v)], Vec::new())
}

fn num(vm: &mut Vm, v: &Value) -> PyResult<f64> {
    if py::is_none(v) { Ok(0.) } else { py::f64_of(vm, v) }
}

fn first_not_none(vals: &[Value]) -> Value {
    vals.iter().find(|v| !py::is_none(v)).cloned().unwrap_or_else(|| vals.last().cloned().unwrap_or(Value::None))
}

fn g(vm: &mut Vm, s: &Value, n: &str) -> PyResult<Value> {
    py::get_or_none(vm, s, n)
}

fn st(vm: &mut Vm, s: &Value, n: &str, v: Value) -> PyResult<()> {
    py::set(vm, s, n, v)
}

fn limit_angle(n: f64) -> f64 {
    n.rem_euclid(360.)
}

// TransformState ---------------------------------------------------------------------------

fn ts_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = &args[0];
    for n in ["inherited_xpos", "inherited_ypos", "inherited_xanchor", "inherited_yanchor", "last_angle"] {
        st(vm, s, n, Value::None)?;
    }
    Ok(Value::None)
}

fn ts_take_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = &args[0];
    let ts = args.get(1).cloned().unwrap_or(Value::None);
    for (n, _, _) in PROPERTIES {
        let v = g(vm, &ts, n)?;
        st(vm, s, n, v)?;
    }
    for n in ["last_angle", "radius_sign", "relative_anchor_radius_sign", "absolute_anchor_radius_sign", "last_absolute_anchorangle", "last_relative_anchorangle", "last_events", "available_width", "available_height"] {
        let v = g(vm, &ts, n)?;
        st(vm, s, n, v)?;
    }
    let persp = g(vm, s, "perspective")?;
    if py::is_none(&persp) {
        for n in ["xpos", "ypos", "xanchor", "yanchor"] {
            st(vm, s, n, Value::None)?;
        }
    }
    let p = vm.call_method(&ts, "get_placement", Vec::new())?;
    let p = py::items(vm, &p)?;
    st(vm, s, "inherited_xpos", p[0].clone())?;
    st(vm, s, "inherited_ypos", p[1].clone())?;
    st(vm, s, "inherited_xanchor", p[2].clone())?;
    st(vm, s, "inherited_yanchor", p[3].clone())?;
    for n in ["xoffset", "yoffset", "subpixel"] {
        let v = g(vm, &ts, n)?;
        st(vm, s, n, v)?;
    }
    Ok(Value::None)
}

fn ts_diff(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = &args[0];
    let newts = args.get(1).cloned().unwrap_or(Value::None);
    let rv = vm.types.new_dict();
    for (n, _, kind) in PROPERTIES {
        if *kind == 0 {
            continue;
        }
        let mut new = g(vm, &newts, n)?;
        let mut old = g(vm, s, n)?;
        if *kind == 4 {
            if py::is_none(&new) {
                new = g(vm, &newts, &format!("inherited_{n}"))?;
            }
            if py::is_none(&old) {
                old = g(vm, s, &format!("inherited_{n}"))?;
            }
        }
        if !ops::eq(vm, &new, &old)? {
            let t = vm.types.new_tuple(vec![old, new]);
            py::dict_set(vm, &rv, n, t)?;
        }
    }
    // Uniforms and other dynamic properties set on either state.
    for state in [s, &newts] {
        let d = py::get(vm, state, "__dict__")?;
        for (k, _) in py::str_items(vm, &d)? {
            if k.starts_with("u_") && !ops::contains(vm, &rv, &vm.types.new_str(&k))? {
                let new = g(vm, &newts, &k)?;
                let old = g(vm, s, &k)?;
                if !ops::eq(vm, &new, &old)? {
                    let t = vm.types.new_tuple(vec![old, new]);
                    py::dict_set(vm, &rv, &k, t)?;
                }
            }
        }
    }
    Ok(rv)
}

fn ts_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = &args[0];
    let prop = py::string(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let saved: Vec<Value> = ["xpos", "ypos", "xanchor", "yanchor"].iter().map(|n| g(vm, s, n)).collect::<PyResult<_>>()?;
    for (i, n) in ["xpos", "ypos", "xanchor", "yanchor"].iter().enumerate() {
        if py::is_none(&saved[i]) {
            let v = g(vm, s, &format!("inherited_{n}"))?;
            st(vm, s, n, v)?;
        }
    }
    let r = py::get_or_none(vm, s, &prop);
    for (i, n) in ["xpos", "ypos", "xanchor", "yanchor"].iter().enumerate() {
        st(vm, s, n, saved[i].clone())?;
    }
    r
}

fn get_placement(vm: &mut Vm, s: &Value, cxo: f64, cyo: f64) -> PyResult<Vec<Value>> {
    let persp = g(vm, s, "perspective")?;
    if !py::is_none(&persp) && !matches!(persp, Value::Bool(false)) {
        return Ok(vec![Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(0), Value::Float(cxo), Value::Float(cyo), Value::Bool(false)]);
    }
    let mut out = Vec::new();
    for n in ["xpos", "ypos", "xanchor", "yanchor"] {
        let a = g(vm, s, n)?;
        let b = g(vm, s, &format!("inherited_{n}"))?;
        out.push(first_not_none(&[a, b]));
    }
    let xo = g(vm, s, "xoffset")?;
    let yo = g(vm, s, "yoffset")?;
    out.push(Value::Float(num(vm, &xo)? + cxo));
    out.push(Value::Float(num(vm, &yo)? + cyo));
    out.push(g(vm, s, "subpixel")?);
    Ok(out)
}

fn ts_get_placement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cx = py::f64_of(vm, &arg_or(args, kw, 1, "cxoffset", Value::Int(0)))?;
    let cy = py::f64_of(vm, &arg_or(args, kw, 2, "cyoffset", Value::Int(0)))?;
    let p = get_placement(vm, &args[0], cx, cy)?;
    Ok(vm.types.new_tuple(p))
}

fn scale(vm: &mut Vm, v: &Value, avail: f64) -> PyResult<f64> {
    compute_raw(vm, v, avail)
}

fn avail(vm: &mut Vm, s: &Value) -> PyResult<(f64, f64)> {
    let w = g(vm, s, "available_width")?;
    let h = g(vm, s, "available_height")?;
    Ok((num(vm, &w)?, num(vm, &h)?))
}

fn pos_polar_vector(vm: &mut Vm, s: &Value) -> PyResult<(f64, f64)> {
    let (aw, ah) = avail(vm, s)?;
    let xp = { let a = g(vm, s, "xpos")?; let b = g(vm, s, "inherited_xpos")?; first_not_none(&[a, b, Value::Int(0)]) };
    let yp = { let a = g(vm, s, "ypos")?; let b = g(vm, s, "inherited_ypos")?; first_not_none(&[a, b, Value::Int(0)]) };
    let xa = g(vm, s, "xaround")?;
    let ya = g(vm, s, "yaround")?;
    Ok((scale(vm, &xp, aw)? - scale(vm, &xa, aw)?, scale(vm, &yp, ah)? - scale(vm, &ya, ah)?))
}

fn get_angle(vm: &mut Vm, s: &Value) -> PyResult<f64> {
    let (vx, vy) = pos_polar_vector(vm, s)?;
    let radius = vx.hypot(vy);
    let mut angle = vx.atan2(-vy) / PI * 180.;
    if angle < 0. {
        angle += 360.;
    }
    let last = g(vm, s, "last_angle")?;
    let sign = { let v = g(vm, s, "radius_sign")?; if py::is_none(&v) { 1. } else { py::f64_of(vm, &v)? } };
    if radius < 0.001 && !py::is_none(&last) {
        angle = py::f64_of(vm, &last)?;
    } else if sign < 0. {
        angle = limit_angle(angle + 180.);
    }
    Ok(angle)
}

fn get_radius(vm: &mut Vm, s: &Value) -> PyResult<f64> {
    let (vx, vy) = pos_polar_vector(vm, s)?;
    let sign = { let v = g(vm, s, "radius_sign")?; if py::is_none(&v) { 1. } else { py::f64_of(vm, &v)? } };
    Ok(vx.hypot(vy) * sign)
}

fn set_pos_from_angle_radius(vm: &mut Vm, s: &Value, angle: f64, radius: f64) -> PyResult<()> {
    let (aw, ah) = avail(vm, s)?;
    let xa = g(vm, s, "xaround")?;
    let ya = g(vm, s, "yaround")?;
    let xa = scale(vm, &xa, aw)?;
    let ya = scale(vm, &ya, ah)?;
    let a = angle * PI / 180.;
    let x = absolute(vm, xa + radius * a.sin())?;
    let y = absolute(vm, ya - radius * a.cos())?;
    st(vm, s, "xpos", x)?;
    st(vm, s, "ypos", y)
}

fn pair(vm: &mut Vm, v: &Value) -> PyResult<(Value, Value)> {
    if py::is_none(v) {
        return Ok((Value::None, Value::None));
    }
    let i = py::items(vm, v)?;
    if i.len() != 2 {
        return Err(vm.value_error("expected a pair"));
    }
    Ok((i[0].clone(), i[1].clone()))
}

fn position_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.atl", "position")
}

/// (absolute, relative) of a position-like value.
pub fn pos_parts(vm: &mut Vm, v: &Value) -> PyResult<(f64, f64)> {
    match v {
        Value::None => Ok((0., 0.)),
        Value::Int(i) => Ok((*i as f64, 0.)),
        Value::Bool(b) => Ok((*b as i64 as f64, 0.)),
        Value::Float(f) => Ok((0., *f)),
        Value::Obj(o) => match o.class_name().as_str() {
            "position" => {
                let a = py::get(vm, v, "absolute")?;
                let r = py::get(vm, v, "relative")?;
                Ok((py::f64_of(vm, &a)?, py::f64_of(vm, &r)?))
            }
            "absolute" => Ok((py::f64_of(vm, v)?, 0.)),
            _ => {
                if matches!(o.payload, crysirror_python::Payload::Float(_)) {
                    Ok((0., py::f64_of(vm, v)?))
                } else {
                    Ok((py::f64_of(vm, v)?, 0.))
                }
            }
        },
    }
}

fn make_position(vm: &mut Vm, a: f64, r: f64) -> PyResult<Value> {
    let c = position_class(vm)?;
    vm.call(&c, vec![Value::Float(a), Value::Float(r)], Vec::new())
}

fn anchor_polar_vectors(vm: &mut Vm, s: &Value) -> PyResult<((f64, f64), (f64, f64))> {
    let xar = g(vm, s, "xanchoraround")?;
    let yar = g(vm, s, "yanchoraround")?;
    let xa = { let a = g(vm, s, "xanchor")?; let b = g(vm, s, "inherited_xanchor")?; first_not_none(&[a, b, Value::Int(0)]) };
    let ya = { let a = g(vm, s, "yanchor")?; let b = g(vm, s, "inherited_yanchor")?; first_not_none(&[a, b, Value::Int(0)]) };
    let (xra, xrr) = pos_parts(vm, &xar)?;
    let (yra, yrr) = pos_parts(vm, &yar)?;
    let (xaa, xar2) = pos_parts(vm, &xa)?;
    let (yaa, yar2) = pos_parts(vm, &ya)?;
    Ok(((xaa - xra, yaa - yra), (xar2 - xrr, yar2 - yrr)))
}

fn anchorangle(vm: &mut Vm, s: &Value) -> PyResult<(f64, f64)> {
    let ((ax, ay), (rx, ry)) = anchor_polar_vectors(vm, s)?;
    let mut aa = ax.atan2(-ay) / PI * 180.;
    let mut ra = rx.atan2(-ry) / PI * 180.;
    if aa < 0. {
        aa += 360.;
    }
    if ra < 0. {
        ra += 360.;
    }
    let la = g(vm, s, "last_absolute_anchorangle")?;
    let lr = g(vm, s, "last_relative_anchorangle")?;
    let asign_v = g(vm, s, "absolute_anchor_radius_sign")?;
    let asign = num(vm, &asign_v).unwrap_or(1.);
    let rsign_v = g(vm, s, "relative_anchor_radius_sign")?;
    let rsign = num(vm, &rsign_v).unwrap_or(1.);
    if ax.hypot(ay) < 0.001 && !py::is_none(&la) {
        aa = py::f64_of(vm, &la)?;
    } else if asign < 0. {
        aa += 180.;
    }
    if rx.hypot(ry) < 0.001 && !py::is_none(&lr) {
        ra = py::f64_of(vm, &lr)?;
    } else if rsign < 0. {
        ra += 180.;
    }
    Ok((limit_angle(aa), limit_angle(ra)))
}

fn anchorradius(vm: &mut Vm, s: &Value) -> PyResult<(f64, f64)> {
    let ((ax, ay), (rx, ry)) = anchor_polar_vectors(vm, s)?;
    let asign_v = g(vm, s, "absolute_anchor_radius_sign")?;
    let asign = num(vm, &asign_v).unwrap_or(1.);
    let rsign_v = g(vm, s, "relative_anchor_radius_sign")?;
    let rsign = num(vm, &rsign_v).unwrap_or(1.);
    Ok((ax.hypot(ay) * asign, rx.hypot(ry) * rsign))
}

fn set_anchor_from_polar(vm: &mut Vm, s: &Value, aa: f64, ra: f64, ar: f64, rr: f64) -> PyResult<()> {
    let xar = g(vm, s, "xanchoraround")?;
    let yar = g(vm, s, "yanchoraround")?;
    let (xra, xrr) = pos_parts(vm, &xar)?;
    let (yra, yrr) = pos_parts(vm, &yar)?;
    let (aa, ra) = (aa * PI / 180., ra * PI / 180.);
    let x = make_position(vm, xra + ar * aa.sin(), xrr + rr * ra.sin())?;
    let y = make_position(vm, yra - ar * aa.cos(), yrr - rr * ra.cos())?;
    st(vm, s, "xanchor", x)?;
    st(vm, s, "yanchor", y)
}

fn dual_angle(vm: &mut Vm, a: f64, r: f64) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.atl", "DualAngle")?;
    vm.call(&c, vec![Value::Float(a), Value::Float(r)], Vec::new())
}

/// Alias property getter/setter. `args[0]` is the state.
fn alias_get(vm: &mut Vm, s: &Value, name: &str) -> PyResult<Value> {
    let t2 = |vm: &mut Vm, a: &str, b: &str| -> PyResult<Value> {
        let x = g(vm, s, a)?;
        let y = g(vm, s, b)?;
        Ok(vm.types.new_tuple(vec![x, y]))
    };
    match name {
        "xalign" | "xcenter" => g(vm, s, "xpos"),
        "yalign" | "ycenter" => g(vm, s, "ypos"),
        "around" | "alignaround" => t2(vm, "xaround", "yaround"),
        "anchoraround" => t2(vm, "xanchoraround", "yanchoraround"),
        "pos" | "align" | "xycenter" => t2(vm, "xpos", "ypos"),
        "anchor" => t2(vm, "xanchor", "yanchor"),
        "offset" => t2(vm, "xoffset", "yoffset"),
        "xysize" | "size" => t2(vm, "xsize", "ysize"),
        "angle" => Ok(Value::Float(get_angle(vm, s)?)),
        "radius" => {
            let r = get_radius(vm, s)?;
            absolute(vm, r)
        }
        "anchorangle" => {
            let (a, r) = anchorangle(vm, s)?;
            dual_angle(vm, a, r)
        }
        "anchorradius" => {
            let (a, r) = anchorradius(vm, s)?;
            make_position(vm, a, r)
        }
        _ => Err(vm.attr_error(name.to_owned())),
    }
}

fn alias_set(vm: &mut Vm, s: &Value, name: &str, v: Value) -> PyResult<()> {
    match name {
        "xalign" => {
            st(vm, s, "xpos", v.clone())?;
            st(vm, s, "xanchor", v)
        }
        "yalign" => {
            st(vm, s, "ypos", v.clone())?;
            st(vm, s, "yanchor", v)
        }
        "xcenter" => {
            st(vm, s, "xpos", v)?;
            st(vm, s, "xanchor", Value::Float(0.5))
        }
        "ycenter" => {
            st(vm, s, "ypos", v)?;
            st(vm, s, "yanchor", Value::Float(0.5))
        }
        "xycenter" => {
            let (a, b) = pair(vm, &v)?;
            alias_set(vm, s, "xcenter", a)?;
            alias_set(vm, s, "ycenter", b)
        }
        "around" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xaround", a)?;
            st(vm, s, "yaround", b)
        }
        "alignaround" => {
            let (a, b) = pair(vm, &v)?;
            for (n, x) in [("xanchor", &a), ("yanchor", &b), ("xaround", &a), ("yaround", &b), ("xanchoraround", &a), ("yanchoraround", &b)] {
                st(vm, s, n, x.clone())?;
            }
            Ok(())
        }
        "anchoraround" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xanchoraround", a)?;
            st(vm, s, "yanchoraround", b)
        }
        "pos" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xpos", a)?;
            st(vm, s, "ypos", b)
        }
        "anchor" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xanchor", a)?;
            st(vm, s, "yanchor", b)
        }
        "align" => {
            let (a, b) = pair(vm, &v)?;
            for (n, x) in [("xanchor", &a), ("yanchor", &b), ("xpos", &a), ("ypos", &b)] {
                st(vm, s, n, x.clone())?;
            }
            Ok(())
        }
        "offset" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xoffset", a)?;
            st(vm, s, "yoffset", b)
        }
        "xysize" => {
            let (a, b) = pair(vm, &v)?;
            st(vm, s, "xsize", a)?;
            st(vm, s, "ysize", b)
        }
        "size" => {
            let (a, b) = pair(vm, &v)?;
            let fix = |v: Value| match v { Value::Float(f) => Value::Int(f as i64), o => o };
            st(vm, s, "xsize", fix(a))?;
            st(vm, s, "ysize", fix(b))
        }
        "angle" => {
            let angle = py::f64_of(vm, &v)?;
            st(vm, s, "last_angle", Value::Float(limit_angle(angle)))?;
            let mut radius = get_radius(vm, s)?;
            let mut a = angle;
            if radius < 0. {
                a = limit_angle(a + 180.);
                radius = -radius;
            }
            set_pos_from_angle_radius(vm, s, a, radius)
        }
        "radius" => {
            let (aw, ah) = avail(vm, s)?;
            let mut radius = compute_raw(vm, &v, aw.min(ah))?;
            let mut angle = get_angle(vm, s)?;
            if radius < 0. {
                angle = limit_angle(angle + 180.);
                radius = -radius;
                st(vm, s, "radius_sign", Value::Int(-1))?;
            } else if radius > 0. {
                st(vm, s, "radius_sign", Value::Int(1))?;
            }
            set_pos_from_angle_radius(vm, s, angle, radius)
        }
        "anchorangle" => {
            let (aa, ra) = if v.obj().is_some_and(|o| o.class_name() == "DualAngle") {
                let a = py::get(vm, &v, "absolute")?;
                let r = py::get(vm, &v, "relative")?;
                (py::f64_of(vm, &a)?, py::f64_of(vm, &r)?)
            } else {
                let x = py::f64_of(vm, &v)?;
                (x, x)
            };
            st(vm, s, "last_absolute_anchorangle", Value::Float(limit_angle(aa)))?;
            st(vm, s, "last_relative_anchorangle", Value::Float(limit_angle(ra)))?;
            let (mut ar, mut rr) = anchorradius(vm, s)?;
            let (mut aa2, mut ra2) = (aa, ra);
            if ar < 0. {
                aa2 = limit_angle(aa2 + 180.);
                ar = -ar;
            }
            if rr < 0. {
                ra2 = limit_angle(ra2 + 180.);
                rr = -rr;
            }
            set_anchor_from_polar(vm, s, aa2, ra2, ar, rr)
        }
        "anchorradius" => {
            let (ar, rr) = pos_parts(vm, &v)?;
            let (mut aa, mut ra) = anchorangle(vm, s)?;
            let (oa, or) = anchorradius(vm, s)?;
            let la = g(vm, s, "last_absolute_anchorangle")?;
            let lr = g(vm, s, "last_relative_anchorangle")?;
            if oa == 0. && !py::is_none(&la) {
                aa = py::f64_of(vm, &la)?;
            }
            if or == 0. && !py::is_none(&lr) {
                ra = py::f64_of(vm, &lr)?;
            }
            let (mut ar2, mut rr2) = (ar, rr);
            if ar < 0. {
                aa = limit_angle(aa + 180.);
                ar2 = -ar;
                st(vm, s, "absolute_anchor_radius_sign", Value::Int(-1))?;
            } else if ar > 0. {
                st(vm, s, "absolute_anchor_radius_sign", Value::Int(1))?;
            }
            if rr < 0. {
                ra = limit_angle(ra + 180.);
                rr2 = -rr;
                st(vm, s, "relative_anchor_radius_sign", Value::Int(-1))?;
            } else if rr > 0. {
                st(vm, s, "relative_anchor_radius_sign", Value::Int(1))?;
            }
            set_anchor_from_polar(vm, s, aa, ra, ar2, rr2)
        }
        _ => Err(vm.attr_error(name.to_owned())),
    }
}

fn alias_getter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    // closure data: name
    let name = py::string(vm, &args[0])?;
    alias_get(vm, &args[1], &name)
}

fn alias_setter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    alias_set(vm, &args[1], &name, args.get(2).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

fn f_scale(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let room = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    Ok(Value::Float(compute_raw(vm, &args.first().cloned().unwrap_or(Value::None), room)?))
}

// Proxy ------------------------------------------------------------------------------------

fn proxy_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    st(vm, &args[0], "name", args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

fn simplify(vm: &mut Vm, v: Value) -> PyResult<Value> {
    if let Some(items) = py::seq(&v).filter(|_| matches!(v.payload(), Some(crysirror_python::Payload::Tuple(_)))) {
        let mut out = Vec::new();
        for i in items {
            out.push(simplify(vm, i)?);
        }
        return Ok(vm.types.new_tuple(out));
    }
    if v.obj().is_some_and(|o| o.class_name() == "position") {
        return vm.call_method(&v, "simplify", Vec::new());
    }
    Ok(v)
}

fn proxy_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let inst = args.get(1).cloned().unwrap_or(Value::None);
    if py::is_none(&inst) {
        return Ok(this.clone());
    }
    let name = py::get(vm, this, "name")?;
    let name = py::string(vm, &name)?;
    let state = py::get(vm, &inst, "state")?;
    let v = py::get_or_none(vm, &state, &name)?;
    simplify(vm, v)
}

fn proxy_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::get(vm, &args[0], "name")?;
    let name = py::string(vm, &name)?;
    let state = py::get(vm, &args[1], "state")?;
    py::set(vm, &state, &name, args.get(2).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

// Transform --------------------------------------------------------------------------------

fn state_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "TransformState")
}

fn transform_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "Transform")
}

pub fn is_transform(vm: &mut Vm, v: &Value) -> bool {
    match transform_class(vm) {
        Ok(c) => c.obj().is_some_and(|c| ops::isinstance(vm, v, c)),
        Err(_) => false,
    }
}

const PREFIXES: &[&str] = &["selected_activate", "selected_hover", "selected_idle", "selected_insensitive", "activate", "hover", "idle", "insensitive", ""];

pub fn t_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["child", "function", "style", "focus", "default", "_args"];
    let mut vals = vec![Value::None, Value::None, vm.types.new_str("default"), Value::None, Value::Bool(false), Value::None];
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut style_props = Vec::new();
    let mut kwargs = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None if &**k == "alt" => style_props.push((k.clone(), v.clone())),
            None => kwargs.push((k.clone(), v.clone())),
        }
    }
    let kwd = py::kwargs_dict(vm, &kwargs)?;
    py::set(vm, this, "kwargs", kwd)?;
    py::set(vm, this, "style_arg", vals[2].clone())?;
    let mut init_kw = style_props;
    init_kw.push(("style".into(), vals[2].clone()));
    init_kw.push(("focus".into(), vals[3].clone()));
    init_kw.push(("default".into(), vals[4].clone()));
    init_kw.push(("_args".into(), vals[5].clone()));
    super::layout::container_init(vm, &[this.clone()], &init_kw)?;
    py::set(vm, this, "function", vals[1].clone())?;
    let child = if py::is_none(&vals[0]) { Value::None } else { super::easy::displayable(vm, &vals[0], &Value::None, true)? };
    if !py::is_none(&child) {
        vm.call_method(this, "add", vec![child])?;
    }
    let sc = state_class(vm)?;
    let state = vm.call(&sc, Vec::new(), Vec::new())?;
    py::set(vm, this, "state", state.clone())?;
    if !kwargs.is_empty() {
        let arguments = vm.types.new_dict();
        for (k, v) in &kwargs {
            let mut prefix = String::new();
            let mut prop = k.to_string();
            loop {
                if is_property(&prop) && (prefix.is_empty() || PREFIXES.contains(&prefix.as_str())) {
                    let key = vm.types.new_str(&prefix);
                    let mut d = vm.call_method(&arguments, "get", vec![key.clone(), Value::None])?;
                    if py::is_none(&d) {
                        d = vm.types.new_dict();
                        ops::setitem(vm, &arguments, key, d.clone())?;
                    }
                    py::dict_set(vm, &d, &prop, v.clone())?;
                    break;
                }
                let Some((np, rest)) = prop.split_once('_') else {
                    return Err(py::error(vm, format!("Unknown transform property: {k:?}")));
                };
                prefix = if prefix.is_empty() { np.to_owned() } else { format!("{prefix}_{np}") };
                prop = rest.to_owned();
            }
        }
        let empty = vm.types.new_str("");
        let base = vm.call_method(&arguments, "get", vec![empty, Value::None])?;
        if !py::is_none(&base) {
            for (k, v) in py::str_items(vm, &base)? {
                py::set(vm, &state, &k, v)?;
            }
        }
        py::set(vm, this, "arguments", arguments)?;
    } else {
        py::set(vm, this, "arguments", Value::None)?;
    }
    for n in ["forward", "reverse"] {
        py::set(vm, this, n, Value::None)?;
    }
    for (n, v) in [("active", false), ("hide_request", false), ("hide_response", true), ("replaced_request", false), ("replaced_response", true)] {
        py::set(vm, this, n, Value::Bool(v))?;
    }
    for n in ["st", "at", "st_offset", "at_offset", "child_st_base"] {
        py::set(vm, this, n, Value::Int(0))?;
    }
    let z = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    py::set(vm, this, "child_size", z.clone())?;
    py::set(vm, this, "render_size", z)?;
    Ok(Value::None)
}

fn t_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(5)))?;
    if v < 1 {
        py::set(vm, this, "active", Value::Bool(false))?;
        let sc = state_class(vm)?;
        let state = vm.call(&sc, Vec::new(), Vec::new())?;
        for n in ["xpos", "ypos", "xanchor", "yanchor"] {
            let x = py::get_or_none(vm, this, n)?;
            let x = if ops::truthy(vm, &x)? { x } else { Value::Int(0) };
            py::set(vm, &state, n, x)?;
        }
        for n in ["alpha", "rotate", "zoom", "xzoom", "yzoom"] {
            let x = py::get_or_none(vm, this, n)?;
            py::set(vm, &state, n, x)?;
        }
        py::set(vm, this, "state", state)?;
        py::set(vm, this, "hide_request", Value::Bool(false))?;
        py::set(vm, this, "hide_response", Value::Bool(true))?;
    }
    if v < 2 {
        py::set(vm, this, "st", Value::Int(0))?;
        py::set(vm, this, "at", Value::Int(0))?;
    }
    if v < 3 {
        py::set(vm, this, "st_offset", Value::Int(0))?;
        py::set(vm, this, "at_offset", Value::Int(0))?;
        py::set(vm, this, "child_st_base", Value::Int(0))?;
    }
    if v < 4 {
        py::set(vm, this, "style_arg", vm.types.new_str("transform"))?;
    }
    if v < 5 {
        py::set(vm, this, "replaced_request", Value::Bool(false))?;
        py::set(vm, this, "replaced_response", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn t_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    Ok(vm.types.new_list(if py::is_none(&c) { Vec::new() } else { vec![c] }))
}

fn t_default_function(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let state = args.get(1).cloned().unwrap_or(Value::None);
    let arguments = py::get_or_none(vm, this, "arguments")?;
    if py::is_none(&arguments) {
        return Ok(Value::None);
    }
    let style = py::get(vm, this, "style")?;
    let prefix = py::get(vm, &style, "prefix")?;
    let mut prefix = py::string(vm, &prefix)?.trim_matches('_').to_owned();
    let mut prefixes = Vec::new();
    while !prefix.is_empty() {
        prefixes.insert(0, prefix.clone());
        prefix = prefix.split_once('_').map(|(_, r)| r.to_owned()).unwrap_or_default();
    }
    prefixes.insert(0, String::new());
    // State is the Transform itself in upstream (properties proxy to state).
    let target = if py::is_none(&state) { this.clone() } else { state };
    for p in prefixes {
        let key = vm.types.new_string(p);
        let d = vm.call_method(&arguments, "get", vec![key, Value::None])?;
        if py::is_none(&d) {
            continue;
        }
        for (k, v) in py::str_items(vm, &d)? {
            py::set(vm, &target, &k, v)?;
        }
    }
    Ok(Value::None)
}

fn t_set_transform_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let event = args.get(1).cloned().unwrap_or(Value::None);
    let child = py::get_or_none(vm, this, "child")?;
    if !py::is_none(&child) {
        vm.call_method(&child, "set_transform_event", vec![event.clone()])?;
        py::set(vm, this, "last_child_transform_event", event.clone())?;
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "set_transform_event")?;
    vm.call(&f, vec![this.clone(), event], Vec::new())
}

fn t_take_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = args.get(1).cloned().unwrap_or(Value::None);
    if py::same(this, &t) || !is_transform(vm, &t) {
        return Ok(Value::None);
    }
    let s = py::get(vm, this, "state")?;
    let ts = py::get(vm, &t, "state")?;
    vm.call_method(&s, "take_state", vec![ts])?;
    let c = py::get_or_none(vm, this, "child")?;
    let tc = py::get_or_none(vm, &t, "child")?;
    if is_transform(vm, &c) && is_transform(vm, &tc) {
        vm.call_method(&c, "take_state", vec![tc.clone()])?;
    }
    if py::is_none(&c) && !py::is_none(&tc) {
        vm.call_method(this, "add", vec![tc])?;
        let b = py::get_or_none(vm, &t, "child_st_base")?;
        py::set(vm, this, "child_st_base", b)?;
    }
    Ok(Value::None)
}

fn t_take_execution_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = args.get(1).cloned().unwrap_or(Value::None);
    if py::same(this, &t) || !is_transform(vm, &t) {
        return Ok(Value::None);
    }
    for n in ["hide_request", "replaced_request", "child_st_base"] {
        let v = py::get_or_none(vm, &t, n)?;
        py::set(vm, this, n, v)?;
    }
    let s = py::get(vm, this, "state")?;
    let ts = py::get(vm, &t, "state")?;
    for n in ["xpos", "ypos", "xanchor", "yanchor"] {
        let v = py::get_or_none(vm, &ts, n)?;
        py::set(vm, &s, n, v)?;
    }
    let c = py::get_or_none(vm, this, "child")?;
    let tc = py::get_or_none(vm, &t, "child")?;
    if is_transform(vm, &c) && is_transform(vm, &tc) {
        vm.call_method(&c, "take_execution_state", vec![tc])?;
    }
    Ok(Value::None)
}

fn t_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = vm.call(this, Vec::new(), Vec::new())?;
    py::set(vm, &d, "kwargs", vm.types.new_dict())?;
    vm.call_method(&d, "take_state", vec![this.clone()])?;
    vm.call_method(&d, "take_execution_state", vec![this.clone()])?;
    for n in ["st", "at"] {
        let v = py::get_or_none(vm, this, n)?;
        py::set(vm, &d, n, v)?;
    }
    Ok(d)
}

fn t_change_transform_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = args.get(1).cloned().unwrap_or(Value::None);
    let rv = vm.call_method(this, "copy", Vec::new())?;
    let c = py::get_or_none(vm, this, "child")?;
    if !py::is_none(&c) {
        let nc = vm.call_method(&c, "_change_transform_child", vec![child])?;
        vm.call_method(&rv, "set_child", vec![nc])?;
    }
    Ok(rv)
}

fn t_handles_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let event = args.get(1).cloned().unwrap_or(Value::None);
    let active = py::get_or_none(vm, this, "active")?;
    if event.as_str() == Some("replaced") && !ops::truthy(vm, &active)? {
        return Ok(Value::Bool(true));
    }
    let f = py::get_or_none(vm, this, "function")?;
    if !py::is_none(&f) {
        return Ok(Value::Bool(true));
    }
    let c = py::get_or_none(vm, this, "child")?;
    if ops::truthy(vm, &c)? {
        return vm.call_method(&c, "_handles_event", vec![event]);
    }
    Ok(Value::Bool(false))
}

fn adjust_for_fps(vm: &mut Vm, this: &Value, s: f64, a: f64) -> PyResult<(f64, f64)> {
    let state = py::get(vm, this, "state")?;
    let fps = g(vm, &state, "fps")?;
    if ops::truthy(vm, &fps)? {
        let m = 1. / py::f64_of(vm, &fps)?;
        let fs = s + m / 2.;
        let fa = a + m / 2.;
        return Ok((fs - fs.rem_euclid(m), fa - fa.rem_euclid(m)));
    }
    Ok((s, a))
}

fn fget(vm: &mut Vm, this: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, this, n)?;
    num(vm, &v)
}

fn sync_times(vm: &mut Vm, this: &Value, s: f64, a: f64) -> PyResult<(f64, f64)> {
    let mut so = fget(vm, this, "st_offset")?;
    let mut ao = fget(vm, this, "at_offset")?;
    let cs = fget(vm, this, "st")?;
    let ca = fget(vm, this, "at")?;
    if s + so <= cs {
        so = cs - s;
        py::set(vm, this, "st_offset", Value::Float(so))?;
    }
    if a + ao <= ca {
        ao = ca - a;
        py::set(vm, this, "at_offset", Value::Float(ao))?;
    }
    let (ns, na) = (s + so, a + ao);
    py::set(vm, this, "st", Value::Float(ns))?;
    py::set(vm, this, "at", Value::Float(na))?;
    Ok((ns, na))
}

fn is_atl(vm: &mut Vm, v: &Value) -> bool {
    match py::import_attr(vm, "renpy.atl", "ATLTransformBase") {
        Ok(c) => c.obj().is_some_and(|c| ops::isinstance(vm, v, c)),
        Err(_) => false,
    }
}

fn t_hide(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s0 = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    let a0 = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let kind = args.get(3).cloned().unwrap_or(Value::None);
    let state = py::get(vm, this, "state")?;
    if kind.as_str() == Some("cancel") {
        let sch = g(vm, &state, "show_cancels_hide")?;
        return Ok(if ops::truthy(vm, &sch)? { Value::None } else { this.clone() });
    }
    let (s, a) = sync_times(vm, this, s0, a0)?;
    let active = py::get_or_none(vm, this, "active")?;
    if !ops::truthy(vm, &active)? {
        vm.call_method(this, "update_state", Vec::new())?;
    }
    let child = py::get_or_none(vm, this, "child")?;
    if !ops::truthy(vm, &child)? {
        return Ok(Value::None);
    }
    let hr = py::get_or_none(vm, this, "hide_request")?;
    let rr = py::get_or_none(vm, this, "replaced_request")?;
    let d = if !(ops::truthy(vm, &hr)? || ops::truthy(vm, &rr)?) { vm.call_method(this, "copy", Vec::new())? } else { this.clone() };
    for n in ["st_offset", "at_offset"] {
        let v = py::get_or_none(vm, this, n)?;
        py::set(vm, &d, n, v)?;
    }
    if is_atl(vm, this) {
        let aso = py::get_or_none(vm, this, "atl_st_offset")?;
        let v = if py::is_none(&aso) { py::get_or_none(vm, this, "st_offset")? } else { aso };
        py::set(vm, &d, "atl_st_offset", v)?;
    }
    if kind.as_str() == Some("hide") {
        py::set(vm, &d, "hide_request", Value::Bool(true))?;
    } else {
        py::set(vm, &d, "replaced_request", Value::Bool(true))?;
    }
    py::set(vm, &d, "hide_response", Value::Bool(true))?;
    py::set(vm, &d, "replaced_response", Value::Bool(true))?;
    let (fs, fa) = adjust_for_fps(vm, this, s, a)?;
    let f = py::get_or_none(vm, &d, "function")?;
    if !py::is_none(&f) {
        vm.call(&f, vec![d.clone(), Value::Float(fs), Value::Float(fa)], Vec::new())?;
    } else if is_atl(vm, &d) {
        vm.call_method(&d, "execute", vec![d.clone(), Value::Float(fs), Value::Float(fa)])?;
    }
    let so = fget(vm, this, "st_offset")?;
    let ao = fget(vm, this, "at_offset")?;
    let dchild = py::get(vm, &d, "child")?;
    let nc = vm.call_method(&dchild, "_hide", vec![Value::Float(s - so), Value::Float(a - ao), kind])?;
    if !py::is_none(&nc) {
        py::set(vm, &d, "child", nc)?;
        py::set(vm, &d, "hide_response", Value::Bool(false))?;
        py::set(vm, &d, "replaced_response", Value::Bool(false))?;
    }
    let hres = py::get_or_none(vm, &d, "hide_response")?;
    let rres = py::get_or_none(vm, &d, "replaced_response")?;
    if !ops::truthy(vm, &hres)? || !ops::truthy(vm, &rres)? {
        render::redraw(vm, &d, 0.);
        return Ok(d);
    }
    Ok(Value::None)
}

fn t_set_child(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let child = arg_or(args, kw, 1, "child", Value::None);
    let duplicate = match arg(args, kw, 2, "duplicate") { Some(v) => ops::truthy(vm, v)?, None => true };
    let mut child = super::easy::displayable(vm, &child, &Value::None, true)?;
    let dup = py::get_or_none(vm, &child, "_duplicatable")?;
    if duplicate && ops::truthy(vm, &dup)? {
        let a = py::get_or_none(vm, this, "_args")?;
        child = vm.call_method(&child, "_duplicate", vec![a])?;
        let own = py::get_or_none(vm, this, "_duplicatable")?;
        if !ops::truthy(vm, &own)? {
            vm.call_method(&child, "_unique", Vec::new())?;
        }
    }
    py::set(vm, this, "child", child.clone())?;
    py::set(vm, this, "children", vm.types.new_list(vec![child.clone()]))?;
    let stv = py::get_or_none(vm, this, "st")?;
    py::set(vm, this, "child_st_base", stv)?;
    vm.call_method(&child, "per_interact", Vec::new())?;
    render::invalidate(vm, this);
    Ok(Value::None)
}

fn t_update_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "hide_response", Value::Bool(true))?;
    py::set(vm, this, "replaced_response", Value::Bool(true))?;
    let s = fget(vm, this, "st")?;
    let a = fget(vm, this, "at")?;
    let (fs, fa) = adjust_for_fps(vm, this, s, a)?;
    let arguments = py::get_or_none(vm, this, "arguments")?;
    if !py::is_none(&arguments) {
        vm.call_method(this, "default_function", vec![this.clone(), Value::Float(fs), Value::Float(fa)])?;
    }
    let f = py::get_or_none(vm, this, "function")?;
    if !py::is_none(&f) {
        let fr = vm.call(&f, vec![this.clone(), Value::Float(fs), Value::Float(fa)], Vec::new())?;
        if !py::is_none(&fr) {
            let t = py::f64_of(vm, &fr)?;
            render::redraw(vm, this, t);
        }
    }
    py::set(vm, this, "active", Value::Bool(true))?;
    update_events(vm, this)
}

pub fn update_events(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let state = py::get(vm, this, "state")?;
    let le = g(vm, &state, "last_events")?;
    let ev = g(vm, &state, "events")?;
    if !ops::eq(vm, &le, &ev)? {
        if ops::truthy(vm, &ev)? {
            let iface = py::import_attr(vm, "renpy.game", "interface")?;
            if !py::is_none(&iface) {
                vm.call_method(&iface, "timeout", vec![Value::Int(0)])?;
            }
        }
        py::set(vm, &state, "last_events", ev)?;
    }
    Ok(Value::None)
}

fn t_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, s0, a0) = super::layout::rargs(vm, args)?;
    let (s, a) = sync_times(vm, this, s0, a0)?;
    let state = py::get(vm, this, "state")?;
    py::set(vm, &state, "available_width", Value::Float(w))?;
    py::set(vm, &state, "available_height", Value::Float(h))?;
    vm.call_method(this, "update_state", Vec::new())?;
    render_transform(vm, this, &state, w, h, s, a)
}

fn rot(vm: &mut Vm, state: &Value, n: &str) -> PyResult<f64> {
    let v = g(vm, state, n)?;
    num(vm, &v)
}

/// The RenderTransform pipeline.
pub fn render_transform(vm: &mut Vm, this: &Value, state: &Value, widtho: f64, heighto: f64, st: f64, at: f64) -> PyResult<Value> {
    let persp = g(vm, state, "perspective")?;
    if ops::truthy(vm, &persp)? {
        return Err(py::error(vm, "The perspective transform property needs a 3D camera projection, which the CrySirror layer presenter does not provide for Ren'Py transforms."));
    }
    // render_child (xsize/ysize).
    let mut child = py::get_or_none(vm, this, "child")?;
    if py::is_none(&child) {
        child = py::import_attr(vm, MODULE, "get_null").and_then(|f| vm.call(&f, Vec::new(), Vec::new()))?;
    }
    let rel = py::config_bool(vm, "relative_transform_size").unwrap_or(true);
    let (mut wo, mut ho) = (widtho, heighto);
    let xs = g(vm, state, "xsize")?;
    let ys = g(vm, state, "ysize")?;
    let xsize = if py::is_none(&xs) { None } else { let v = compute_raw(vm, &xs, if rel { wo } else { 1. })?; wo = v; Some(v) };
    let ysize = if py::is_none(&ys) { None } else { let v = compute_raw(vm, &ys, if rel { ho } else { 1. })?; ho = v; Some(v) };
    let base = fget(vm, this, "child_st_base")?;
    let mut cr = render::render(vm, &child, wo, ho, st - base, at)?;
    // tile_and_pan
    let xtile = { let v = g(vm, state, "xtile")?; if py::is_none(&v) { 1 } else { py::int(vm, &v)? } };
    let ytile = { let v = g(vm, state, "ytile")?; if py::is_none(&v) { 1 } else { py::int(vm, &v)? } };
    let (cw0, ch0) = super::layout::size(&cr);
    if xtile != 1 || ytile != 1 {
        let mut t = RenderData::new(cw0 * xtile as f64, ch0 * ytile as f64);
        let src = render::source_of(vm, &cr)?;
        for i in 0..xtile {
            for j in 0..ytile {
                t.children.push(render::Blit { source: src.clone(), x: i as f64 * cw0, y: j as f64 * ch0, focus: true, main: true });
            }
        }
        cr = new_render(vm, t);
    }
    let xpan = g(vm, state, "xpan")?;
    let ypan = g(vm, state, "ypan")?;
    if !py::is_none(&xpan) || !py::is_none(&ypan) {
        let (crw, crh) = super::layout::size(&cr);
        let (pan_x, pan_w) = if py::is_none(&xpan) { (0., crw) } else { (cw0 * (py::f64_of(vm, &xpan)?.rem_euclid(360.) / 360.), cw0) };
        let (pan_y, pan_h) = if py::is_none(&ypan) { (0., crh) } else { (ch0 * (py::f64_of(vm, &ypan)?.rem_euclid(360.) / 360.), ch0) };
        let mut t = RenderData::new(pan_w, pan_h);
        let src = render::source_of(vm, &cr)?;
        let xs: &[f64] = if py::is_none(&xpan) { &[0.] } else { &[0., 1.] };
        let ys: &[f64] = if py::is_none(&ypan) { &[0.] } else { &[0., 1.] };
        for xo in xs {
            for yo in ys {
                t.children.push(render::Blit { source: src.clone(), x: xo * cw0 - pan_x, y: yo * ch0 - pan_y, focus: true, main: true });
            }
        }
        t.xclipping = true;
        t.yclipping = true;
        cr = new_render(vm, t);
    }
    let mesh = g(vm, state, "mesh")?;
    let blur = g(vm, state, "blur")?;
    let has_blur = !py::is_none(&blur) && py::f64_of(vm, &blur)? > 0.;
    if ops::truthy(vm, &mesh)? && !has_blur {
        return Err(py::error(vm, "The mesh transform property needs a shader mesh pipeline, which the CrySirror layer presenter does not provide."));
    }
    let (mut width, mut height) = super::layout::size(&cr);
    let cs = vm.types.new_tuple(vec![Value::Float(width), Value::Float(height)]);
    py::set(vm, this, "child_size", cs)?;
    // cropping
    let (mut xo, mut yo) = (0f64, 0f64);
    let mut clipping = false;
    let crop_v = g(vm, state, "crop")?;
    let mut crop_rel = g(vm, state, "crop_relative")?;
    if py::is_none(&crop_rel) {
        crop_rel = py::config(vm, "crop_relative_default").unwrap_or(Value::Bool(true));
    }
    let crop_rel = ops::truthy(vm, &crop_rel)?;
    let ltc = py::config(vm, "limit_transform_crop").unwrap_or(Value::Bool(false));
    let rfc = |vm: &mut Vm, n: &Value, base: f64, limit: f64| -> PyResult<f64> {
        if ltc.as_str() == Some("only_float") && !matches!(n, Value::Float(_)) {
            return num(vm, n);
        }
        let rv = compute_raw(vm, n, base)?.trunc();
        Ok(if ops::truthy(vm, &ltc).unwrap_or(false) { rv.min(limit) } else { rv })
    };
    let mut crop: Option<[f64; 4]> = None;
    if !py::is_none(&crop_v) {
        let c = py::items(vm, &crop_v)?;
        let (bx, by) = if crop_rel { (width, height) } else { (1., 1.) };
        let x = rfc(vm, &c[0], bx, width)?;
        let y = rfc(vm, &c[1], by, height)?;
        let w = rfc(vm, &c[2], bx, width - x)?;
        let h = rfc(vm, &c[3], by, height - y)?;
        crop = Some([x, y, w, h]);
    }
    let c1 = g(vm, state, "corner1")?;
    let c2 = g(vm, state, "corner2")?;
    if crop.is_none() && !py::is_none(&c1) && !py::is_none(&c2) {
        let a = py::items(vm, &c1)?;
        let b = py::items(vm, &c2)?;
        let (bx, by) = if crop_rel { (width, height) } else { (1., 1.) };
        let mut x1 = rfc(vm, &a[0], bx, width)?;
        let mut y1 = rfc(vm, &a[1], by, height)?;
        let mut x2 = rfc(vm, &b[0], bx, width)?;
        let mut y2 = rfc(vm, &b[1], by, height)?;
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
        }
        if y1 > y2 {
            std::mem::swap(&mut y1, &mut y2);
        }
        crop = Some([x1, y1, x2 - x1, y2 - y1]);
    }
    let rotate = g(vm, state, "rotate")?;
    if let Some([nx, ny, w, h]) = crop {
        if !py::is_none(&rotate) {
            let mut c = RenderData::new(w, h);
            let src = render::source_of(vm, &cr)?;
            c.children.push(render::Blit { source: src, x: -nx, y: -ny, focus: true, main: true });
            c.xclipping = true;
            c.yclipping = true;
            cr = new_render(vm, c);
        } else {
            xo = -nx;
            yo = -ny;
            clipping = true;
        }
        width = w;
        height = h;
    }
    // size_zoom_rotate
    let (mut rxdx, mut rxdy, mut rydx, mut rydy) = (1f64, 0f64, 0f64, 1f64);
    let mut fit = g(vm, state, "fit")?;
    let (mut xsz, mut ysz) = (xsize, ysize);
    if width != 0. && height != 0. {
        let maxsize = g(vm, state, "maxsize")?;
        let mut mul: Option<f64> = None;
        if !py::is_none(&maxsize) {
            let m = py::items(vm, &maxsize)?;
            let mx = py::f64_of(vm, &m[0])?;
            let my = py::f64_of(vm, &m[1])?;
            mul = Some((mx / width).min(my / height));
        }
        let mut sc = Vec::new();
        if let Some(x) = xsz {
            sc.push(x / width);
        }
        if let Some(y) = ysz {
            sc.push(y / height);
        }
        if ops::truthy(vm, &fit)? && sc.is_empty() {
            sc = vec![wo / width, ho / height];
        }
        if py::is_none(&fit) {
            fit = vm.types.new_str("fill");
        }
        if !sc.is_empty() {
            let mx = sc.iter().cloned().fold(f64::MIN, f64::max);
            let mn = sc.iter().cloned().fold(f64::MAX, f64::min);
            match fit.as_str() {
                Some("scale-up") => mul = Some(mx.max(1.)),
                Some("scale-down") => mul = Some(mn.min(1.)),
                Some("contain") => mul = Some(mn),
                Some("cover") => mul = Some(mx),
                _ => {
                    xsz = Some(xsz.unwrap_or(width));
                    ysz = Some(ysz.unwrap_or(height));
                }
            }
        }
        if let Some(m) = mul {
            xsz = Some(m * width);
            ysz = Some(m * height);
        }
        if let (Some(nw), Some(nh)) = (xsz, ysz) {
            if (nw, nh) != (width, height) {
                let xz = nw / width;
                let yz = nh / height;
                rxdx = xz;
                rydy = yz;
                xo *= xz;
                yo *= yz;
                width = nw;
                height = nh;
            }
        }
    }
    let zoom = rot(vm, state, "zoom")?;
    let xzoom = zoom * { let v = g(vm, state, "xzoom")?; if py::is_none(&v) { 1. } else { py::f64_of(vm, &v)? } };
    let yzoom = zoom * { let v = g(vm, state, "yzoom")?; if py::is_none(&v) { 1. } else { py::f64_of(vm, &v)? } };
    if xzoom != 1. {
        rxdx *= xzoom;
        width *= xzoom.abs();
        xo *= xzoom;
        if xzoom < 0. {
            xo += width;
        }
    }
    if yzoom != 1. {
        rydy *= yzoom;
        height *= yzoom.abs();
        yo *= yzoom;
        if yzoom < 0. {
            yo += height;
        }
    }
    if !py::is_none(&rotate) {
        let (cw, ch) = (width, height);
        let angle = py::f64_of(vm, &rotate)? * PI / 180.;
        let (sina, cosa) = angle.sin_cos();
        rxdy = rydy * -sina;
        rydx = rxdx * sina;
        rxdx *= cosa;
        rydy *= cosa;
        let px = if xzoom < 0. { -cw / 2. } else { cw / 2. };
        let py_ = if yzoom < 0. { -ch / 2. } else { ch / 2. };
        let pad = { let v = g(vm, state, "rotate_pad")?; ops::truthy(vm, &v)? };
        xo = -px * cosa + py_ * sina;
        yo = -px * sina - py_ * cosa;
        if pad {
            width = cw.hypot(ch);
            height = width;
        } else {
            let pts = [(xo, yo), (-px * cosa - py_ * sina, -px * sina + py_ * cosa), (px * cosa - py_ * sina, px * sina + py_ * cosa), (px * cosa + py_ * sina, px * sina - py_ * cosa)];
            let minx = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let maxx = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let miny = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let maxy = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            width = maxx - minx;
            height = maxy - miny;
        }
        xo += width / 2.;
        yo += height / 2.;
    }
    // Matrix2D(xdx, ydx, xdy, ydy) with upstream's argument order.
    let mut reverse = if rxdx == 1. && rxdy == 0. && rydx == 0. && rydy == 1. {
        Mat::IDENTITY
    } else {
        let mut m = Mat::IDENTITY;
        m.0[0] = rxdx;
        m.0[1] = rxdy;
        m.0[4] = rydx;
        m.0[5] = rydy;
        m
    };
    // matrix_operations: x/y/z rotate, orientation, point_to, zpos.
    let (xr, yr, zr) = (rot(vm, state, "xrotate")?, rot(vm, state, "yrotate")?, rot(vm, state, "zrotate")?);
    let orient = g(vm, state, "orientation")?;
    let point_to = g(vm, state, "point_to")?;
    if !py::is_none(&point_to) {
        return Err(py::error(vm, "The point_to transform property needs a 3D camera, which the CrySirror layer presenter does not provide for Ren'Py transforms."));
    }
    let anchor = |vm: &mut Vm| -> PyResult<(f64, f64)> {
        let ma = g(vm, state, "matrixanchor")?;
        if py::is_none(&ma) {
            return Ok((width / 2., height / 2.));
        }
        let m = py::items(vm, &ma)?;
        Ok((compute_raw(vm, &m[0], width)?, compute_raw(vm, &m[1], height)?))
    };
    if xr != 0. || yr != 0. || zr != 0. || ops::truthy(vm, &orient)? {
        let (mx, my) = anchor(vm)?;
        let mut m = Mat::offset(-mx, -my, 0.);
        if ops::truthy(vm, &orient)? {
            let o = py::items(vm, &orient)?;
            m = Mat::rotate(py::f64_of(vm, &o[0])?, py::f64_of(vm, &o[1])?, py::f64_of(vm, &o[2])?).mul(&m);
        }
        if xr != 0. || yr != 0. || zr != 0. {
            m = Mat::rotate(xr, yr, zr).mul(&m);
        }
        m = Mat::offset(mx, my, 0.).mul(&m);
        reverse = m.mul(&reverse);
    }
    let zpos = rot(vm, state, "zpos")?;
    if zpos != 0. {
        reverse = Mat::offset(0., 0., zpos).mul(&reverse);
    }
    let mt = g(vm, state, "matrixtransform")?;
    if !py::is_none(&mt) {
        let mut mo = mt.clone();
        if crysirror_python::builtins::is_callable(vm, &mo) && render::matrix_data(&mo).is_none() {
            mo = vm.call(&mo, vec![Value::None, Value::Float(1.)], Vec::new())?;
        }
        let m = render::matrix_data(&mo).ok_or_else(|| py::error(vm, "matrixtransform requires a Matrix"))?;
        let (mx, my) = anchor(vm)?;
        let full = Mat::offset(mx, my, 0.).mul(&m).mul(&Mat::offset(-mx, -my, 0.));
        reverse = full.mul(&reverse);
    }
    let mut rv = RenderData::new(width, height);
    let fwd;
    if !reverse.is_identity() {
        fwd = reverse.inverse();
        rv.reverse = Some(reverse);
        rv.forward = Some(fwd);
        let rm = new_matrix(vm, reverse);
        let fm = new_matrix(vm, fwd);
        py::set(vm, this, "reverse", rm)?;
        py::set(vm, this, "forward", fm)?;
    } else {
        let id = new_matrix(vm, Mat::IDENTITY);
        py::set(vm, this, "reverse", id.clone())?;
        py::set(vm, this, "forward", id)?;
    }
    let alpha = rot(vm, state, "alpha")?.clamp(0., 1.);
    let subpixel = { let v = g(vm, state, "subpixel")?; ops::truthy(vm, &v)? };
    let src = render::source_of(vm, &cr)?;
    if alpha <= 0. {
        if let render::Source::Render(r, _) = &src {
            rv.pass_focuses.push(r.clone());
        }
    } else {
        let (bx, by) = if subpixel { (xo, yo) } else { (xo.trunc(), yo.trunc()) };
        rv.children.push(render::Blit { source: src, x: bx, y: by, focus: true, main: true });
    }
    // final_render
    let mc = g(vm, state, "matrixcolor")?;
    if ops::truthy(vm, &mc)? {
        let mut m = mc.clone();
        if crysirror_python::builtins::is_callable(vm, &m) && render::matrix_data(&m).is_none() {
            m = vm.call(&m, vec![Value::None, Value::Float(1.)], Vec::new())?;
        }
        if render::matrix_data(&m).is_none() {
            let r = ops::repr(vm, &m)?;
            return Err(py::error(vm, format!("matrixcolor requires a Matrix (not im.matrix, got {r})")));
        }
        let d = vm.types.new_dict();
        py::dict_set(vm, &d, "matrixcolor", m)?;
        rv.properties = d;
    }
    let nearest = g(vm, state, "nearest")?;
    rv.nearest = nearest;
    rv.alpha = alpha;
    rv.over = 1. - rot(vm, state, "additive")?;
    if has_blur {
        let d = if py::is_none(&rv.properties) { vm.types.new_dict() } else { rv.properties.clone() };
        py::dict_set(vm, &d, "blur", blur)?;
        rv.properties = d;
    }
    let blend = g(vm, state, "blend")?;
    if ops::truthy(vm, &blend)? {
        let d = if py::is_none(&rv.properties) { vm.types.new_dict() } else { rv.properties.clone() };
        py::dict_set(vm, &d, "blend", blend)?;
        rv.properties = d;
    }
    let shader = g(vm, state, "shader")?;
    if !py::is_none(&shader) {
        return Err(py::error(vm, "The shader transform property needs model-based GLSL shaders, which the CrySirror layer presenter does not run for Ren'Py."));
    }
    rv.xclipping = clipping;
    rv.yclipping = clipping;
    let off = vm.types.new_tuple(vec![Value::Float(xo), Value::Float(yo)]);
    py::set(vm, this, "offsets", vm.types.new_list(vec![off]))?;
    let rs = vm.types.new_tuple(vec![Value::Float(width), Value::Float(height)]);
    py::set(vm, this, "render_size", rs)?;
    Ok(new_render(vm, rv))
}

fn t_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let hr = py::get_or_none(vm, this, "hide_request")?;
    if ops::truthy(vm, &hr)? {
        return Ok(Value::None);
    }
    let state = py::get(vm, this, "state")?;
    let ev_ok = g(vm, &state, "events")?;
    if !ops::truthy(vm, &ev_ok)? {
        return Ok(Value::None);
    }
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let stv = args.get(4).cloned().unwrap_or(Value::Int(0));
    let children = py::get(vm, this, "children")?;
    let children = py::items(vm, &children)?;
    let offs = py::get(vm, this, "offsets")?;
    let offs = py::items(vm, &offs)?;
    if offs.is_empty() {
        return Ok(Value::None);
    }
    let fwd = py::get_or_none(vm, this, "forward")?;
    let fm = render::matrix_data(&fwd).unwrap_or(Mat::IDENTITY);
    for i in (0..children.len().min(offs.len())).rev() {
        let o = py::items(vm, &offs[i])?;
        let (cx, cy) = fm.transform2(x - py::f64_of(vm, &o[0])?, y - py::f64_of(vm, &o[1])?);
        let rv = vm.call_method(&children[i], "event", vec![ev.clone(), Value::Float(cx), Value::Float(cy), stv.clone()])?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    Ok(Value::None)
}

fn t_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut child = arg_or(args, kw, 1, "child", Value::None);
    let a = py::kw(kw, "_args").cloned().unwrap_or(Value::None);
    if py::is_none(&child) {
        child = py::get_or_none(vm, this, "child")?;
    }
    let dup = py::get_opt(vm, &child, "_duplicatable")?.unwrap_or(Value::Bool(false));
    if ops::truthy(vm, &dup)? {
        child = vm.call_method(&child, "_duplicate", vec![a.clone()])?;
    }
    let f = py::get_or_none(vm, this, "function")?;
    let style = py::get_or_none(vm, this, "style_arg")?;
    let kwargs = py::get(vm, this, "kwargs")?;
    let mut k = py::str_items(vm, &kwargs)?;
    k.push(("child".into(), child));
    k.push(("function".into(), f));
    k.push(("style".into(), style));
    k.push(("_args".into(), a));
    let tc = transform_class(vm)?;
    let rv = vm.call(&tc, Vec::new(), k)?;
    vm.call_method(&rv, "take_state", vec![this.clone()])?;
    Ok(rv)
}

fn t_unique(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = py::get_or_none(vm, this, "_duplicatable")?;
    if ops::truthy(vm, &d)? {
        let c = py::get_or_none(vm, this, "child")?;
        if !py::is_none(&c) {
            vm.call_method(&c, "_unique", Vec::new())?;
        }
        py::set(vm, this, "_duplicatable", Value::Bool(false))?;
    }
    Ok(Value::None)
}

fn t_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let active = py::get_or_none(vm, this, "active")?;
    if !ops::truthy(vm, &active)? {
        vm.call_method(this, "update_state", Vec::new())?;
    }
    let state = py::get(vm, this, "state")?;
    let child = py::get_or_none(vm, this, "child")?;
    let (mut cxo, mut cyo) = (0., 0.);
    if !py::is_none(&child) {
        let p = vm.call_method(&child, "get_placement", Vec::new())?;
        let p = py::items(vm, &p)?;
        if py::config_bool(vm, "transform_uses_child_position").unwrap_or(true) {
            for (i, n) in [(0, "inherited_xpos"), (2, "inherited_xanchor"), (1, "inherited_ypos"), (3, "inherited_yanchor")] {
                if !py::is_none(&p[i]) {
                    py::set(vm, &state, n, p[i].clone())?;
                }
            }
            let sp = g(vm, &state, "subpixel")?;
            let csp = p.get(6).cloned().unwrap_or(Value::Bool(false));
            let v = ops::truthy(vm, &sp)? || ops::truthy(vm, &csp)?;
            py::set(vm, &state, "subpixel", Value::Bool(v))?;
        }
        cxo = num(vm, &p[4])?;
        cyo = num(vm, &p[5])?;
    }
    let mut rv = get_placement(vm, &state, cxo, cyo)?;
    let ta = g(vm, &state, "transform_anchor")?;
    if ops::truthy(vm, &ta)? && !py::is_none(&rv[2]) && !py::is_none(&rv[3]) {
        let cs = py::get(vm, this, "child_size")?;
        let cs = py::items(vm, &cs)?;
        let rs = py::get(vm, this, "render_size")?;
        let rs = py::items(vm, &rs)?;
        let (cw, ch) = (py::f64_of(vm, &cs[0])?, py::f64_of(vm, &cs[1])?);
        let (rw, rh) = (py::f64_of(vm, &rs[0])?, py::f64_of(vm, &rs[1])?);
        let xa = compute_raw(vm, &rv[2], cw)? - cw / 2.;
        let ya = compute_raw(vm, &rv[3], ch)? - ch / 2.;
        let rev = py::get_or_none(vm, this, "reverse")?;
        let m = render::matrix_data(&rev).unwrap_or(Mat::IDENTITY);
        let (xa, ya) = m.transform2(xa, ya);
        rv[2] = absolute(vm, xa + rw / 2.)?;
        rv[3] = absolute(vm, ya + rh / 2.)?;
    }
    Ok(vm.types.new_tuple(rv))
}

fn t_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    render::invalidate(vm, &args[0]);
    Ok(Value::None)
}

fn t_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = arg_or(args, kw, 1, "args", Value::None);
    if !py::is_none(&a) {
        let x = py::get(vm, &a, "args")?;
        if ops::truthy(vm, &x)? {
            vm.call_method(&a, "extraneous", Vec::new())?;
        }
    }
    let d = py::get_or_none(vm, this, "_duplicatable")?;
    if !ops::truthy(vm, &d)? {
        return Ok(this.clone());
    }
    let rv = vm.call(this, Vec::new(), vec![("_args".into(), a)])?;
    vm.call_method(&rv, "take_execution_state", vec![this.clone()])?;
    Ok(rv)
}

fn t_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let c = py::get_or_none(vm, this, "child")?;
    if py::is_none(&c) {
        return Ok(this.clone());
    }
    let nc = vm.call_method(&c, "_in_current_store", Vec::new())?;
    if py::same(&nc, &c) {
        return Ok(this.clone());
    }
    let rv = vm.call(this, Vec::new(), Vec::new())?;
    vm.call_method(&rv, "take_execution_state", vec![this.clone()])?;
    py::set(vm, &rv, "child", nc)?;
    vm.call_method(&rv, "_unique", Vec::new())?;
    Ok(rv)
}

fn t_repr_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    let r = ops::repr(vm, &c)?;
    Ok(vm.types.new_string(r.to_string()))
}

fn f_get_null(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::import(vm, MODULE)?;
    let n = py::get_or_none(vm, &m, "null")?;
    if !py::is_none(&n) {
        return Ok(n);
    }
    let c = py::import_attr(vm, "renpy.display.layout", "Null")?;
    let n = vm.call(&c, Vec::new(), Vec::new())?;
    py::set(vm, &m, "null", n.clone())?;
    if let Ok(mo) = py::import(vm, "renpy.display.motion") {
        py::set(vm, &mo, "null", n.clone())?;
    }
    Ok(n)
}

fn camera_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let layer = arg_or(args, kw, 1, "layer", vm.types.new_str("master"));
    py::set(vm, &args[0], "layer", layer)?;
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("add_property", f_add_property),
        ("add_uniform", f_add_uniform),
        ("add_gl_property", f_add_gl_property),
        ("get_null", f_get_null),
        ("first_not_none", |_, a, _| Ok(first_not_none(a))),
        ("limit_angle", |vm, a, _| {
            let n = py::f64_of(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
            Ok(Value::Float(limit_angle(n)))
        }),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "null", Value::None);
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let cam = py::class(vm, MODULE, "Camera", &[object.clone()], &[("__init__", camera_init)], &[])?;
    set(vm, "Camera", Value::Obj(cam));
    // TransformState
    let absolute_c = py::import_attr(vm, "renpy.display.core", "absolute")?;
    let ts = py::class(vm, MODULE, "TransformState", &[object], &[
        ("__init__", ts_init),
        ("take_state", ts_take_state),
        ("diff", ts_diff),
        ("get", ts_get),
        ("get_placement", ts_get_placement),
    ], &[
        ("last_angle", Value::Float(0.)), ("last_relative_anchorangle", Value::Float(0.)), ("last_absolute_anchorangle", Value::Float(0.)),
        ("last_events", Value::Bool(true)), ("available_width", Value::Int(0)), ("available_height", Value::Int(0)), ("radius_type", absolute_c),
        ("radius_sign", Value::Int(1)), ("relative_anchor_radius_sign", Value::Int(1)), ("absolute_anchor_radius_sign", Value::Int(1)),
    ])?;
    for (n, d, _) in PROPERTIES {
        vm.types.set_attr(&ts, n, def_value(*d));
    }
    let scale_f = vm.types.new_builtin("scale", f_scale);
    let scale_f = py::staticmethod(vm, scale_f);
    vm.types.set_attr(&ts, "scale", scale_f);
    for n in ALIASES {
        let nv = vm.types.new_str(n);
        let getter = py::closure(vm, "alias_get", alias_getter, nv.clone());
        let setter = py::closure(vm, "alias_set", alias_setter, nv);
        let p = py::property(vm, getter, Some(setter))?;
        vm.types.set_attr(&ts, n, p);
    }
    set(vm, "TransformState", Value::Obj(ts));
    // Proxy
    let proxy = py::class(vm, MODULE, "Proxy", &[], &[("__init__", proxy_init), ("__get__", proxy_get), ("__set__", proxy_set)], &[])?;
    set(vm, "Proxy", Value::Obj(proxy.clone()));
    // Transform
    let container = py::import_attr(vm, "renpy.display.layout", "Container")?;
    let args_default = vm.types.new_dict();
    for p in PREFIXES {
        py::dict_set(vm, &args_default, p, vm.types.new_dict())?;
    }
    let empty_list = vm.types.new_list(Vec::new());
    let zero2 = vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]);
    let tcls = py::class(vm, MODULE, "Transform", &[container], &[
        ("__init__", t_init),
        ("after_upgrade", t_after_upgrade),
        ("visit", t_visit),
        ("default_function", t_default_function),
        ("set_transform_event", t_set_transform_event),
        ("take_state", t_take_state),
        ("take_execution_state", t_take_execution_state),
        ("copy", t_copy),
        ("_change_transform_child", t_change_transform_child),
        ("_handles_event", t_handles_event),
        ("_hide", t_hide),
        ("set_child", t_set_child),
        ("update_state", t_update_state),
        ("render", t_render),
        ("event", t_event),
        ("__call__", t_call),
        ("_unique", t_unique),
        ("get_placement", t_get_placement),
        ("update", t_update),
        ("_duplicate", t_duplicate),
        ("_in_current_store", t_in_current_store),
        ("_repr_info", t_repr_info),
    ], &[
        ("__version__", Value::Int(5)), ("transform_event_responder", Value::Bool(true)), ("DEFAULT_ARGUMENTS", args_default.clone()),
        ("active", Value::Bool(false)), ("children", empty_list), ("arguments", args_default), ("child_size", zero2),
        ("_duplicatable", Value::Bool(true)),
    ])?;
    for (n, _, _) in PROPERTIES.iter().map(|p| (p.0, p.1, p.2)).chain(ALIASES.iter().map(|a| (*a, Def::None, 0))) {
        let nv = vm.types.new_str(n);
        let p = vm.call(&Value::Obj(proxy.clone()), vec![nv], Vec::new())?;
        vm.types.set_attr(&tcls, n, p);
    }
    set(vm, "Transform", Value::Obj(tcls.clone()));
    super::atlrt::create_atl_transform(vm, &m, Value::Obj(tcls))?;
    let sp = vm.call(&Value::Obj(vm.types.set.clone()), vec![vm.types.new_list(vec![vm.types.new_str("alt")])], Vec::new())?;
    set(vm, "style_properties", sp);
    let names: Vec<Value> = PROPERTIES.iter().map(|p| vm.types.new_str(p.0)).collect();
    let all = vm.call(&Value::Obj(vm.types.set.clone()), vec![vm.types.new_list(names)], Vec::new())?;
    set(vm, "all_properties", all);
    for n in ["diff2_properties", "diff4_properties", "uniforms", "gl_properties"] {
        let s = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
        set(vm, n, s);
    }
    Ok(m)
}

/// `add_property(name, atl=any_object, default=None, diff=2)`.
fn add_property(vm: &mut Vm, name: &Value, atl: Option<Value>, diff: Option<i64>) -> PyResult<()> {
    let all = py::import_attr(vm, MODULE, "all_properties")?;
    if ops::contains(vm, &all, name)? {
        return Ok(());
    }
    ops::set_add(vm, &all, name.clone())?;
    let proxy = py::import_attr(vm, MODULE, "Proxy")?;
    let p = vm.call(&proxy, vec![name.clone()], Vec::new())?;
    let t = py::import_attr(vm, MODULE, "Transform")?;
    let n = py::string(vm, name)?;
    if let Some(c) = t.obj() {
        vm.types.set_attr(c, &n, p);
    }
    let atl = match atl {
        Some(a) => a,
        None => py::import_attr(vm, "renpy.atl", "any_object")?,
    };
    let props = py::import_attr(vm, "renpy.atl", "PROPERTIES")?;
    ops::setitem(vm, &props, name.clone(), atl)?;
    let set = match diff {
        Some(2) => Some("diff2_properties"),
        Some(4) => Some("diff4_properties"),
        _ => None,
    };
    if let Some(set) = set {
        let s = py::import_attr(vm, MODULE, set)?;
        ops::set_add(vm, &s, name.clone())?;
    }
    Ok(())
}

fn f_add_property(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(args, kw, 0, "name", Value::None);
    let atl = py::arg(args, kw, 1, "atl").cloned();
    let diff = match py::arg(args, kw, 3, "diff") {
        None => Some(2),
        Some(Value::None) => None,
        Some(v) => Some(py::int(vm, v)?),
    };
    add_property(vm, &name, atl, diff)?;
    Ok(Value::None)
}

/// `add_uniform(name)`: a `u_` uniform becomes a Transform/ATL property
/// (the standard uniforms of the GL2 renderer excepted).
fn f_add_uniform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    if !n.starts_with("u_") || ["u_transform", "u_time", "u_random", "u_drawable_size"].contains(&n.as_str()) {
        return Ok(Value::None);
    }
    add_property(vm, &args[0], None, Some(2))?;
    let u = py::import_attr(vm, MODULE, "uniforms")?;
    ops::set_add(vm, &u, args[0].clone())?;
    Ok(Value::None)
}

fn f_add_gl_property(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_property(vm, &args[0], None, None)?;
    let g = py::import_attr(vm, MODULE, "gl_properties")?;
    ops::set_add(vm, &g, args[0].clone())?;
    Ok(Value::None)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
