// SPDX-License-Identifier: MPL-2.0
//! Color parsing shared by `renpy.color.Color`, `renpy.easy.color` and
//! render fills: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, 3/4-tuples of
//! 0-255 ints and `Color` objects (tuple subclasses).
use super::py;
use crysirror_python::{Payload, PyResult, Value, Vm};

pub fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let t = text.strip_prefix('#').unwrap_or(text);
    if !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut rgba = [255u8; 4];
    match t.len() {
        3 | 4 => {
            for (i, o) in rgba.iter_mut().take(t.len()).enumerate() {
                *o = 17 * u8::from_str_radix(&t[i..i + 1], 16).ok()?;
            }
        }
        6 | 8 => {
            for (i, o) in rgba.iter_mut().take(t.len() / 2).enumerate() {
                *o = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
            }
        }
        _ => return None,
    }
    Some(rgba)
}

pub fn rgba(vm: &mut Vm, v: &Value) -> PyResult<[u8; 4]> {
    if let Some(s) = v.as_str() {
        return parse_hex(s).ok_or_else(|| vm.value_error(format!("Color string {s:?} must be in hex format.")));
    }
    let items = match v.payload() {
        Some(Payload::Tuple(_)) | Some(Payload::List(_)) => py::seq(v).unwrap_or_default(),
        _ => {
            // Color objects keep their channels in a tuple payload; other
            // sequences are iterated.
            py::items(vm, v)?
        }
    };
    if !matches!(items.len(), 3 | 4) {
        return Err(vm.value_error("color must have 3 or 4 components"));
    }
    let mut out = [255u8; 4];
    for (o, c) in out.iter_mut().zip(items.iter()) {
        let x = match c {
            Value::Float(f) => (f * 255.).round(),
            _ => py::f64_of(vm, c)?,
        };
        *o = x.clamp(0., 255.) as u8;
    }
    Ok(out)
}

pub fn rgba_opt(vm: &mut Vm, v: &Value) -> PyResult<Option<[u8; 4]>> {
    if py::is_none(v) { Ok(None) } else { rgba(vm, v).map(Some) }
}

fn f_rgba(vm: &mut Vm, args: &[Value], _: crysirror_python::builtins::Kw) -> PyResult<Value> {
    let c = rgba(vm, &args.first().cloned().unwrap_or(Value::None))?;
    Ok(vm.types.new_tuple(c.iter().map(|x| Value::Int(*x as i64)).collect()))
}

pub fn functions() -> &'static [(&'static str, crysirror_python::object::NativeFn)] {
    &[("rgba", f_rgba)]
}

// `renpy.color.Color` ---------------------------------------------------------------

use super::py::{self as p, arg, arg_or};
use crysirror_python::{ObjRef, builtins::Kw, ops};

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    if s == 0. {
        return [v, v, v];
    }
    let i = (h * 6.).floor();
    let f = h * 6. - i;
    let (pp, q, t) = (v * (1. - s), v * (1. - s * f), v * (1. - s * (1. - f)));
    match (i as i64).rem_euclid(6) {
        0 => [v, t, pp],
        1 => [q, v, pp],
        2 => [pp, v, t],
        3 => [pp, q, v],
        4 => [t, pp, v],
        _ => [v, pp, q],
    }
}

fn rgb_to_hsv(r: f64, g: f64, b: f64) -> [f64; 3] {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let v = maxc;
    if minc == maxc {
        return [0., 0., v];
    }
    let s = (maxc - minc) / maxc;
    let rc = (maxc - r) / (maxc - minc);
    let gc = (maxc - g) / (maxc - minc);
    let bc = (maxc - b) / (maxc - minc);
    let h = if r == maxc { bc - gc } else if g == maxc { 2. + rc - bc } else { 4. + gc - rc };
    [(h / 6.).rem_euclid(1.), s, v]
}

fn rgb_to_hls(r: f64, g: f64, b: f64) -> [f64; 3] {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let l = (minc + maxc) / 2.;
    if minc == maxc {
        return [0., l, 0.];
    }
    let s = if l <= 0.5 { (maxc - minc) / (maxc + minc) } else { (maxc - minc) / (2. - maxc - minc) };
    let rc = (maxc - r) / (maxc - minc);
    let gc = (maxc - g) / (maxc - minc);
    let bc = (maxc - b) / (maxc - minc);
    let h = if r == maxc { bc - gc } else if g == maxc { 2. + rc - bc } else { 4. + gc - rc };
    [(h / 6.).rem_euclid(1.), l, s]
}

fn hls_to_rgb(h: f64, l: f64, s: f64) -> [f64; 3] {
    if s == 0. {
        return [l, l, l];
    }
    let m2 = if l <= 0.5 { l * (1. + s) } else { l + s - l * s };
    let m1 = 2. * l - m2;
    let v = |hue: f64| {
        let hue = hue.rem_euclid(1.);
        if hue < 1. / 6. {
            m1 + (m2 - m1) * hue * 6.
        } else if hue < 0.5 {
            m2
        } else if hue < 2. / 3. {
            m1 + (m2 - m1) * (2. / 3. - hue) * 6.
        } else {
            m1
        }
    };
    [v(h + 1. / 3.), v(h), v(h - 1. / 3.)]
}

fn color_class(vm: &mut Vm) -> PyResult<ObjRef> {
    let c = p::import_attr(vm, "renpy.color", "Color")?;
    c.obj().cloned().ok_or_else(|| vm.type_error("Color class"))
}

fn tuple_new(vm: &mut Vm, cls: &ObjRef, items: Vec<Value>) -> PyResult<Value> {
    let tn = p::get(vm, &Value::Obj(vm.types.tuple.clone()), "__new__")?;
    let t = vm.types.new_tuple(items);
    vm.call(&tn, vec![Value::Obj(cls.clone()), t], Vec::new())
}

pub fn make_color(vm: &mut Vm, c: [i64; 4]) -> PyResult<Value> {
    let cls = color_class(vm)?;
    tuple_new(vm, &cls, c.iter().map(|v| Value::Int(*v)).collect())
}

fn from_unit(vm: &mut Vm, cls: &ObjRef, rgb: [f64; 3], alpha: f64, hls: Value, hsv: Value) -> PyResult<Value> {
    let items = vec![Value::Int((rgb[0] * 255.) as i64), Value::Int((rgb[1] * 255.) as i64), Value::Int((rgb[2] * 255.) as i64), Value::Int((alpha * 255.) as i64)];
    let rv = tuple_new(vm, cls, items)?;
    let rgbt = vm.types.new_tuple(rgb.iter().map(|v| Value::Float(*v)).collect());
    p::set(vm, &rv, "_rgb", rgbt)?;
    p::set(vm, &rv, "_hls", hls)?;
    p::set(vm, &rv, "_hsv", hsv)?;
    p::set(vm, &rv, "_alpha", Value::Float(alpha))?;
    let rgba = vm.types.new_tuple(vec![Value::Float(rgb[0]), Value::Float(rgb[1]), Value::Float(rgb[2]), Value::Float(alpha)]);
    p::set(vm, &rv, "_rgba", rgba)?;
    Ok(rv)
}

fn triple(vm: &mut Vm, v: &Value) -> PyResult<[f64; 3]> {
    let i = p::items(vm, v)?;
    if i.len() < 3 {
        return Err(vm.value_error("expected three components"));
    }
    Ok([p::f64_of(vm, &i[0])?, p::f64_of(vm, &i[1])?, p::f64_of(vm, &i[2])?])
}

fn c_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cls = args[0].obj().cloned().ok_or_else(|| vm.type_error("Color.__new__"))?;
    let a = &args[1..];
    let color = arg_or(a, kw, 0, "color", Value::None);
    let hls = arg_or(a, kw, 1, "hls", Value::None);
    let hsv = arg_or(a, kw, 2, "hsv", Value::None);
    let rgb = arg_or(a, kw, 3, "rgb", Value::None);
    let alpha = match arg(a, kw, 4, "alpha") { Some(v) => p::f64_of(vm, v)?, None => 1. };
    if !p::is_none(&color) {
        if let Some(s) = color.as_str() {
            let t = s.strip_prefix('#').unwrap_or(s);
            if !matches!(t.len(), 3 | 4 | 6 | 8) || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(p::error(vm, format!("Color string {t:?} must be 3, 4, 6, or 8 hex digits long.")));
            }
            let mut c = parse_hex(t).unwrap_or([0; 4]);
            if matches!(t.len(), 3 | 6) {
                c[3] = (alpha * 255.) as u8;
            }
            return tuple_new(vm, &cls, c.iter().map(|v| Value::Int(*v as i64)).collect());
        }
        if color.obj().is_some_and(|o| ops::isinstance(vm, &Value::Obj(o.clone()), &cls)) {
            return Ok(color);
        }
        let mut c = p::items(vm, &color)?;
        if c.len() == 3 {
            c.push(Value::Int((255. * alpha) as i64));
        }
        if c.len() == 4 {
            return tuple_new(vm, &cls, c);
        }
    }
    let mut rgbv = None;
    let mut hsv_keep = hsv.clone();
    if !p::is_none(&hsv) {
        let [h, s, v] = triple(vm, &hsv)?;
        rgbv = Some(hsv_to_rgb(h, s, v));
    }
    if !p::is_none(&hls) {
        hsv_keep = Value::None;
        let [h, l, s] = triple(vm, &hls)?;
        rgbv = Some(hls_to_rgb(h, l, s));
    }
    if !p::is_none(&rgb) {
        rgbv = Some(triple(vm, &rgb)?);
    }
    if let Some(c) = rgbv {
        return from_unit(vm, &cls, c, alpha, hls, hsv_keep);
    }
    if p::is_none(&color) {
        return Ok(Value::None);
    }
    let r = ops::repr(vm, &color)?;
    Err(p::error(vm, format!("Not a color: {r}")))
}

fn comps(vm: &mut Vm, v: &Value) -> PyResult<[f64; 4]> {
    let i = p::items(vm, v)?;
    let mut c = [0.; 4];
    for (k, x) in i.iter().take(4).enumerate() {
        c[k] = p::f64_of(vm, x)?;
    }
    Ok(c)
}

fn c_hexcode(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = comps(vm, &args[0])?;
    let [r, g, b, a] = c.map(|v| v as i64);
    Ok(vm.types.new_string(if a != 255 { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}") }))
}

fn c_repr(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let h = c_hexcode(vm, args, kw)?;
    Ok(vm.types.new_string(format!("<Color {}>", h.as_str().unwrap_or(""))))
}

fn c_getnewargs(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = p::items(vm, &args[0])?;
    let t = vm.types.new_tuple(c);
    Ok(vm.types.new_tuple(vec![t]))
}

fn unit(vm: &mut Vm, this: &Value) -> PyResult<[f64; 4]> {
    let c = comps(vm, this)?;
    Ok(c.map(|v| v / 255.))
}

fn ftuple(vm: &Vm, v: &[f64]) -> Value {
    vm.types.new_tuple(v.iter().map(|x| Value::Float(*x)).collect())
}

fn c_rgb(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = p::get_or_none(vm, &args[0], "_rgb")?;
    if !p::is_none(&c) {
        return Ok(c);
    }
    let u = unit(vm, &args[0])?;
    Ok(ftuple(vm, &u[..3]))
}
fn c_rgba(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = p::get_or_none(vm, &args[0], "_rgba")?;
    if !p::is_none(&c) {
        return Ok(c);
    }
    let u = unit(vm, &args[0])?;
    Ok(ftuple(vm, &u))
}
fn c_premultiplied(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let u = unit(vm, &args[0])?;
    Ok(ftuple(vm, &[u[0] * u[3], u[1] * u[3], u[2] * u[3], u[3]]))
}
fn hls_of(vm: &mut Vm, this: &Value) -> PyResult<[f64; 3]> {
    let c = p::get_or_none(vm, this, "_hls")?;
    if !p::is_none(&c) {
        return triple(vm, &c);
    }
    let u = unit(vm, this)?;
    Ok(rgb_to_hls(u[0], u[1], u[2]))
}
fn hsv_of(vm: &mut Vm, this: &Value) -> PyResult<[f64; 3]> {
    let c = p::get_or_none(vm, this, "_hsv")?;
    if !p::is_none(&c) {
        return triple(vm, &c);
    }
    let u = unit(vm, this)?;
    Ok(rgb_to_hsv(u[0], u[1], u[2]))
}
fn alpha_of(vm: &mut Vm, this: &Value) -> PyResult<f64> {
    let c = p::get_or_none(vm, this, "_alpha")?;
    if !p::is_none(&c) {
        return p::f64_of(vm, &c);
    }
    Ok(unit(vm, this)?[3])
}
fn c_hls(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = hls_of(vm, &args[0])?;
    Ok(ftuple(vm, &v))
}
fn c_hsv(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = hsv_of(vm, &args[0])?;
    Ok(ftuple(vm, &v))
}
fn c_alpha(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Float(alpha_of(vm, &args[0])?))
}

fn coerce(vm: &mut Vm, v: &Value) -> PyResult<[f64; 4]> {
    let cls = color_class(vm)?;
    let c = vm.call(&Value::Obj(cls), vec![v.clone()], Vec::new())?;
    comps(vm, &c)
}

fn icolor(vm: &mut Vm, c: [f64; 4]) -> PyResult<Value> {
    make_color(vm, c.map(|v| v as i64))
}

fn c_normalize(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = comps(vm, &args[0])?;
    icolor(vm, c.map(|v| v.clamp(0., 255.)))
}
fn c_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = comps(vm, &args[0])?;
    let b = coerce(vm, &args[1])?;
    icolor(vm, [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]])
}
fn c_sub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = comps(vm, &args[0])?;
    let b = coerce(vm, &args[1])?;
    icolor(vm, [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]])
}
fn c_rsub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = comps(vm, &args[0])?;
    let b = coerce(vm, &args[1])?;
    icolor(vm, [b[0] - a[0], b[1] - a[1], b[2] - a[2], b[3] - a[3]])
}
fn c_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = comps(vm, &args[0])?;
    let other = &args[1];
    if other.obj().is_some_and(|o| o.class_name() == "matrix") {
        let r = vm.call_method(other, "vector_mul", vec![args[0].clone()])?;
        let r = comps(vm, &r)?;
        return icolor(vm, r);
    }
    let b = coerce(vm, other)?;
    icolor(vm, [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]])
}

fn lerp_value(vm: &mut Vm, a: &Value, b: &Value, f: f64) -> PyResult<Value> {
    if let (Some(x), Some(y)) = (p::seq(a), p::seq(b)) {
        let mut out = Vec::new();
        for (x, y) in x.iter().zip(y.iter()) {
            out.push(lerp_value(vm, x, y, f)?);
        }
        let ty = ops::type_of(vm, a);
        if py_is_plain_tuple(vm, &ty) {
            return Ok(vm.types.new_tuple(out));
        }
        let t = vm.types.new_tuple(out);
        return vm.call(&Value::Obj(ty), vec![t], Vec::new());
    }
    let x = p::f64_of(vm, a)?;
    let y = p::f64_of(vm, b)?;
    let r = x + (y - x) * f;
    Ok(match a { Value::Int(_) => Value::Int(r as i64), _ => Value::Float(r) })
}

fn py_is_plain_tuple(vm: &Vm, t: &ObjRef) -> bool {
    std::rc::Rc::ptr_eq(t, &vm.types.tuple)
}

fn c_interpolate_core(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = p::f64_of(vm, &args[3])?;
    lerp_value(vm, &args[1], &args[2], f)
}
fn c_interpolate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = color_class(vm)?;
    let other = vm.call(&Value::Obj(cls), vec![args[1].clone()], Vec::new())?;
    let f = p::f64_of(vm, &args[2])?;
    lerp_value(vm, &args[0], &other, f)
}
fn interp_space(vm: &mut Vm, args: &[Value], hsv: bool) -> PyResult<Value> {
    let this = &args[0];
    let other = &args[1];
    let f = p::f64_of(vm, &args[2])?;
    let a = alpha_of(vm, this)?;
    let cls = color_class(vm)?;
    let key = if hsv { "hsv" } else { "hls" };
    let o = if other.as_str().is_some() {
        vm.call(&Value::Obj(cls.clone()), vec![other.clone()], vec![("alpha".into(), Value::Float(a))])?
    } else if other.obj().is_some_and(|o| ops::isinstance(vm, &Value::Obj(o.clone()), &cls)) {
        other.clone()
    } else {
        vm.call(&Value::Obj(cls.clone()), Vec::new(), vec![(key.into(), other.clone()), ("alpha".into(), Value::Float(a))])?
    };
    let (s, t) = if hsv { (hsv_of(vm, this)?, hsv_of(vm, &o)?) } else { (hls_of(vm, this)?, hls_of(vm, &o)?) };
    let mix = [s[0] + (t[0] - s[0]) * f, s[1] + (t[1] - s[1]) * f, s[2] + (t[2] - s[2]) * f];
    let oa = alpha_of(vm, &o)?;
    let alpha = a + (oa - a) * f;
    let m = ftuple(vm, &mix);
    vm.call(&Value::Obj(cls), Vec::new(), vec![(key.into(), m), ("alpha".into(), Value::Float(alpha))])
}
fn c_interpolate_hsv(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    interp_space(vm, args, true)
}
fn c_interpolate_hls(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    interp_space(vm, args, false)
}
fn towards(vm: &mut Vm, args: &[Value], target: f64) -> PyResult<Value> {
    let c = comps(vm, &args[0])?;
    let f = 1. - p::f64_of(vm, &args[1])?;
    let t = [target, target, target, c[3]];
    icolor(vm, [0, 1, 2, 3].map(|i| c[i] + (t[i] - c[i]) * f))
}
fn c_tint(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    towards(vm, args, 255.)
}
fn c_shade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    towards(vm, args, 0.)
}
fn c_opacity(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = comps(vm, &args[0])?;
    let o = p::f64_of(vm, &args[1])?;
    icolor(vm, [c[0], c[1], c[2], (c[3] * o).trunc()])
}
fn from_space(vm: &mut Vm, key: &str, v: [f64; 3], alpha: f64) -> PyResult<Value> {
    let cls = color_class(vm)?;
    let t = ftuple(vm, &v);
    vm.call(&Value::Obj(cls), Vec::new(), vec![(key.into(), t), ("alpha".into(), Value::Float(alpha))])
}
fn hls_edit(vm: &mut Vm, args: &[Value], f: impl Fn([f64; 3], f64) -> [f64; 3]) -> PyResult<Value> {
    let h = hls_of(vm, &args[0])?;
    let x = p::f64_of(vm, &args[1])?;
    let a = alpha_of(vm, &args[0])?;
    from_space(vm, "hls", f(h, x), a)
}
fn hsv_edit(vm: &mut Vm, args: &[Value], f: impl Fn([f64; 3], f64) -> [f64; 3]) -> PyResult<Value> {
    let h = hsv_of(vm, &args[0])?;
    let x = p::f64_of(vm, &args[1])?;
    let a = alpha_of(vm, &args[0])?;
    from_space(vm, "hsv", f(h, x), a)
}
fn c_rotate_hue(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hls_edit(vm, a, |[h, l, s], x| [(h + x).rem_euclid(1.), l, s])
}
fn c_replace_hue(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hls_edit(vm, a, |[_, l, s], x| [x, l, s])
}
fn c_mul_hls_sat(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hls_edit(vm, a, |[h, l, s], x| [h, l, (s * x).min(1.)])
}
fn c_mul_hsv_sat(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hsv_edit(vm, a, |[h, s, v], x| [h, (s * x).min(1.), v])
}
fn c_mul_value(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hsv_edit(vm, a, |[h, s, v], x| [h, s, (v * x).min(1.)])
}
fn c_rep_hls_sat(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hls_edit(vm, a, |[h, l, _], x| [h, l, x])
}
fn c_rep_hsv_sat(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hsv_edit(vm, a, |[h, _, v], x| [h, x, v])
}
fn c_rep_value(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hsv_edit(vm, a, |[h, s, _], x| [h, s, x])
}
fn c_rep_lightness(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    hls_edit(vm, a, |[h, _, s], x| [h, x, s])
}
fn c_rep_opacity(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = comps(vm, &args[0])?;
    let o = p::f64_of(vm, &args[1])?.clamp(0., 1.);
    let cls = color_class(vm)?;
    let t = vm.types.new_tuple(vec![Value::Int(c[0] as i64), Value::Int(c[1] as i64), Value::Int(c[2] as i64)]);
    vm.call(&Value::Obj(cls), vec![t], vec![("alpha".into(), Value::Float(o))])
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.color", functions());
    let tuple = Value::Obj(vm.types.tuple.clone());
    let cls = p::class(vm, "renpy.color", "Color", &[tuple], &[
        ("__repr__", c_repr),
        ("__getnewargs__", c_getnewargs),
        ("normalize", c_normalize),
        ("__add__", c_add),
        ("__radd__", c_add),
        ("__sub__", c_sub),
        ("__rsub__", c_rsub),
        ("__mul__", c_mul),
        ("__rmul__", c_mul),
        ("interpolate_core", c_interpolate_core),
        ("interpolate", c_interpolate),
        ("interpolate_hsv", c_interpolate_hsv),
        ("interpolate_hls", c_interpolate_hls),
        ("tint", c_tint),
        ("shade", c_shade),
        ("opacity", c_opacity),
        ("rotate_hue", c_rotate_hue),
        ("replace_hue", c_replace_hue),
        ("multiply_hls_saturation", c_mul_hls_sat),
        ("multiply_hsv_saturation", c_mul_hsv_sat),
        ("multiply_value", c_mul_value),
        ("replace_hls_saturation", c_rep_hls_sat),
        ("replace_hsv_saturation", c_rep_hsv_sat),
        ("replace_value", c_rep_value),
        ("replace_lightness", c_rep_lightness),
        ("replace_opacity", c_rep_opacity),
    ], &[("_rgb", Value::None), ("_hls", Value::None), ("_hsv", Value::None), ("_alpha", Value::None), ("_rgba", Value::None)])?;
    let new = vm.types.new_builtin("__new__", c_new);
    let new = p::staticmethod(vm, new);
    vm.types.set_attr(&cls, "__new__", new);
    for (n, f) in [("hexcode", c_hexcode as crysirror_python::object::NativeFn), ("rgb", c_rgb), ("rgba", c_rgba), ("premultiplied", c_premultiplied), ("hls", c_hls), ("hsv", c_hsv), ("alpha", c_alpha)] {
        let g = vm.types.new_builtin(n, f);
        let prop = p::property(vm, g, None)?;
        vm.types.set_attr(&cls, n, prop);
    }
    crysirror_python::modules::set(vm, &m, "Color", Value::Obj(cls.clone()));
    crysirror_python::modules::set(vm, &m, "color", Value::Obj(cls));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module("renpy.color", init_module);
}
