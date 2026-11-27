// SPDX-License-Identifier: MPL-2.0
//! `math` and a minimal `cmath`.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::num::{self, Num};
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};

fn f(vm: &mut Vm, a: &[Value], i: usize) -> PyResult<f64> {
    let v = a.get(i).ok_or_else(|| vm.type_error("math function: missing argument"))?;
    ops::to_f64(vm, v)
}

fn domain(vm: &Vm) -> crate::vm::PyErr {
    vm.value_error("math domain error")
}

macro_rules! unary {
    ($($name:ident => $e:expr),* $(,)?) => {
        $(
            fn $name(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                let x = f(vm, a, 0)?;
                let r: f64 = $e(x);
                if r.is_nan() && !x.is_nan() {
                    return Err(domain(vm));
                }
                Ok(Value::Float(r))
            }
        )*
    };
}

unary! {
    sin => f64::sin, cos => f64::cos, tan => f64::tan, asin => f64::asin, acos => f64::acos, atan => f64::atan,
    sinh => f64::sinh, cosh => f64::cosh, tanh => f64::tanh, asinh => f64::asinh, acosh => f64::acosh, atanh => f64::atanh,
    exp => f64::exp, expm1 => f64::exp_m1, fabs => f64::abs, degrees => f64::to_degrees, radians => f64::to_radians,
    erf => erf_impl, erfc => |x| 1.0 - erf_impl(x), gamma => gamma_impl, lgamma => |x: f64| gamma_impl(x).abs().ln(),
}

fn sqrt(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    if x < 0.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x.sqrt()))
}

fn log(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = match a.first().and_then(|v| ops::to_bigint(v)) {
        Some(b) if b.bits() > 1000 => {
            // log of huge ints: bits * ln2 approximation through f64 mantissa.
            let shift = b.bits() - 60;
            let top = (&b >> shift as usize).to_f64().unwrap();
            top.ln() + shift as f64 * std::f64::consts::LN_2
        }
        _ => {
            let v = f(vm, a, 0)?;
            if v <= 0.0 {
                return Err(domain(vm));
            }
            v.ln()
        }
    };
    match a.get(1) {
        Some(b) => {
            let b = ops::to_f64(vm, b)?;
            if b <= 0.0 || b == 1.0 {
                return Err(domain(vm));
            }
            Ok(Value::Float(x / b.ln()))
        }
        None => Ok(Value::Float(x)),
    }
}

fn log10(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    if x <= 0.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x.log10()))
}

fn log2(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    if x <= 0.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x.log2()))
}

fn log1p(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    if x <= -1.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x.ln_1p()))
}

fn pow(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let y = f(vm, a, 1)?;
    if x < 0.0 && y.fract() != 0.0 && y.is_finite() {
        return Err(domain(vm));
    }
    if x == 0.0 && y < 0.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x.powf(y)))
}

fn atan2(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let y = f(vm, a, 0)?;
    let x = f(vm, a, 1)?;
    Ok(Value::Float(y.atan2(x)))
}

fn hypot(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut s = 0.0f64;
    for i in 0..a.len() {
        let x = f(vm, a, i)?;
        s = s.hypot(x);
    }
    Ok(Value::Float(s))
}

fn copysign(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let y = f(vm, a, 1)?;
    Ok(Value::Float(x.copysign(y)))
}

fn fmod(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let y = f(vm, a, 1)?;
    if y == 0.0 {
        return Err(domain(vm));
    }
    Ok(Value::Float(x % y))
}

fn int_result(vm: &mut Vm, x: f64) -> PyResult<Value> {
    if x.is_nan() {
        return Err(vm.value_error("cannot convert float NaN to integer"));
    }
    if x.is_infinite() {
        return Err(vm.exc_msg("OverflowError", "cannot convert float infinity to integer"));
    }
    let b: BigInt = num_traits::FromPrimitive::from_f64(x).unwrap_or_default();
    Ok(vm.types.new_bigint(b))
}

fn floor(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(v) = a.first() {
        if let Some(Num::I(_) | Num::B(_)) = num::num_of(v) {
            return Ok(v.clone());
        }
        if !matches!(num::num_of(v), Some(_)) {
            if let Some(r) = ops::call_special(vm, v, "__floor__", Vec::new())? {
                return Ok(r);
            }
        }
    }
    let x = f(vm, a, 0)?;
    if crate::ops::vm_dialect(vm) == crate::Dialect::Py2 {
        return Ok(Value::Float(x.floor()));
    }
    int_result(vm, x.floor())
}

fn ceil(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(v) = a.first() {
        if let Some(Num::I(_) | Num::B(_)) = num::num_of(v) {
            return Ok(v.clone());
        }
        if !matches!(num::num_of(v), Some(_)) {
            if let Some(r) = ops::call_special(vm, v, "__ceil__", Vec::new())? {
                return Ok(r);
            }
        }
    }
    let x = f(vm, a, 0)?;
    if crate::ops::vm_dialect(vm) == crate::Dialect::Py2 {
        return Ok(Value::Float(x.ceil()));
    }
    int_result(vm, x.ceil())
}

fn trunc(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(v) = a.first() {
        if let Some(Num::I(_) | Num::B(_)) = num::num_of(v) {
            return Ok(v.clone());
        }
    }
    let x = f(vm, a, 0)?;
    int_result(vm, x.trunc())
}

fn modf(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    Ok(vm.types.new_tuple(vec![Value::Float(x.fract()), Value::Float(x.trunc())]))
}

fn frexp(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let (m, e) = crate::hash::frexp(x);
    Ok(vm.types.new_tuple(vec![Value::Float(m), Value::Int(e as i64)]))
}

fn ldexp(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let e = ops::index_of(vm, &a[1])?;
    Ok(Value::Float(x * 2f64.powi(e.clamp(-2000, 2000) as i32)))
}

fn isnan(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(f(vm, a, 0)?.is_nan()))
}
fn isinf(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(f(vm, a, 0)?.is_infinite()))
}
fn isfinite(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(f(vm, a, 0)?.is_finite()))
}

fn isclose(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let x = f(vm, a, 0)?;
    let y = f(vm, a, 1)?;
    let mut rel = 1e-9;
    let mut abs = 0.0;
    for (n, v) in k {
        match &**n {
            "rel_tol" => rel = ops::to_f64(vm, v)?,
            "abs_tol" => abs = ops::to_f64(vm, v)?,
            _ => {}
        }
    }
    if x == y {
        return Ok(Value::Bool(true));
    }
    let d = (x - y).abs();
    Ok(Value::Bool(d <= (rel * y.abs()).max(rel * x.abs()) || d <= abs))
}

fn factorial(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[0])?;
    if n < 0 {
        return Err(vm.value_error("factorial() not defined for negative values"));
    }
    let mut r = BigInt::from(1);
    for i in 2..=n {
        r *= i;
    }
    Ok(vm.types.new_bigint(r))
}

fn gcd(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut g = BigInt::zero();
    for v in a {
        let b = ops::to_bigint(v).ok_or_else(|| vm.type_error("integer argument expected"))?;
        g = g.gcd(&b);
    }
    Ok(vm.types.new_bigint(g.abs()))
}

fn lcm(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut l = BigInt::from(1);
    for v in a {
        let b = ops::to_bigint(v).ok_or_else(|| vm.type_error("integer argument expected"))?;
        if b.is_zero() {
            return Ok(Value::Int(0));
        }
        l = l.lcm(&b);
    }
    Ok(vm.types.new_bigint(l.abs()))
}

fn isqrt(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = ops::to_bigint(&a[0]).ok_or_else(|| vm.type_error("integer argument expected"))?;
    if b.is_negative() {
        return Err(vm.value_error("isqrt() argument must be nonnegative"));
    }
    Ok(vm.types.new_bigint(b.sqrt()))
}

fn fsum(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = ops::collect(vm, &a[0])?;
    // Neumaier summation.
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for it in items {
        let x = ops::to_f64(vm, &it)?;
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    Ok(Value::Float(s + c))
}

fn prod(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut acc = Value::Int(1);
    for (n, v) in k {
        if &**n == "start" {
            acc = v.clone();
        }
    }
    for it in ops::collect(vm, &a[0])? {
        acc = ops::binary_op(vm, &acc, &it, crate::ast::BinOp::Mult, false, false)?;
    }
    Ok(acc)
}

fn comb(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[0])?;
    let k = ops::index_of(vm, &a[1])?;
    if n < 0 || k < 0 {
        return Err(vm.value_error("must be non-negative integer"));
    }
    if k > n {
        return Ok(Value::Int(0));
    }
    let k = k.min(n - k);
    let mut r = BigInt::from(1);
    for i in 0..k {
        r = r * (n - i) / (i + 1);
    }
    Ok(vm.types.new_bigint(r))
}

fn perm(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[0])?;
    let k = match a.get(1) {
        Some(v) if !v.is_none() => ops::index_of(vm, v)?,
        _ => n,
    };
    if k > n {
        return Ok(Value::Int(0));
    }
    let mut r = BigInt::from(1);
    for i in 0..k {
        r *= n - i;
    }
    Ok(vm.types.new_bigint(r))
}

fn dist(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = ops::collect(vm, &a[0])?;
    let q = ops::collect(vm, &a[1])?;
    let mut s = 0.0f64;
    for (x, y) in p.iter().zip(&q) {
        let d = ops::to_f64(vm, x)? - ops::to_f64(vm, y)?;
        s += d * d;
    }
    Ok(Value::Float(s.sqrt()))
}

fn erf_impl(x: f64) -> f64 {
    // Abramowitz-Stegun 7.1.26 refined with a series for small |x|.
    if x.abs() < 0.5 {
        let x2 = x * x;
        let mut term = x;
        let mut sum = x;
        for n in 1..30 {
            term *= -x2 / n as f64;
            sum += term / (2 * n + 1) as f64;
        }
        return sum * 2.0 / std::f64::consts::PI.sqrt();
    }
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let y = 1.0 - t * (-x * x - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418 + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277))))))))).exp();
    if x >= 0.0 { y } else { -y }
}

fn gamma_impl(x: f64) -> f64 {
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma_impl(1.0 - x));
    }
    let g = 7.0;
    let c = [0.999_999_999_999_809_9, 676.5203681218851, -1259.1392167224028, 771.323_428_777_653_1, -176.615_029_162_140_6, 12.507343278686905, -0.13857109526572012, 9.984_369_578_019_572e-6, 1.5056327351493116e-7];
    let x = x - 1.0;
    let mut a = c[0];
    let t = x + g + 0.5;
    for (i, ci) in c.iter().enumerate().skip(1) {
        a += ci / (x + i as f64);
    }
    (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "math", &[
        ("sin", sin), ("cos", cos), ("tan", tan), ("asin", asin), ("acos", acos), ("atan", atan), ("atan2", atan2),
        ("sinh", sinh), ("cosh", cosh), ("tanh", tanh), ("asinh", asinh), ("acosh", acosh), ("atanh", atanh),
        ("exp", exp), ("expm1", expm1), ("log", log), ("log10", log10), ("log2", log2), ("log1p", log1p),
        ("sqrt", sqrt), ("pow", pow), ("fabs", fabs), ("floor", floor), ("ceil", ceil), ("trunc", trunc),
        ("modf", modf), ("frexp", frexp), ("ldexp", ldexp), ("fmod", fmod), ("hypot", hypot), ("copysign", copysign),
        ("degrees", degrees), ("radians", radians), ("isnan", isnan), ("isinf", isinf), ("isfinite", isfinite),
        ("isclose", isclose), ("factorial", factorial), ("gcd", gcd), ("lcm", lcm), ("isqrt", isqrt), ("fsum", fsum),
        ("prod", prod), ("comb", comb), ("perm", perm), ("dist", dist), ("erf", erf), ("erfc", erfc), ("gamma", gamma), ("lgamma", lgamma),
    ]);
    set(vm, &m, "pi", Value::Float(std::f64::consts::PI));
    set(vm, &m, "e", Value::Float(std::f64::consts::E));
    set(vm, &m, "tau", Value::Float(std::f64::consts::TAU));
    set(vm, &m, "inf", Value::Float(f64::INFINITY));
    set(vm, &m, "nan", Value::Float(f64::NAN));
    Ok(m)
}

fn c_of(vm: &mut Vm, v: &Value) -> PyResult<(f64, f64)> {
    match num::num_of(v) {
        Some(Num::C(r, i)) => Ok((r, i)),
        _ => Ok((ops::to_f64(vm, v)?, 0.0)),
    }
}

fn c_sqrt(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (r, i) = c_of(vm, &a[0])?;
    let m = (r * r + i * i).sqrt();
    let re = ((m + r) / 2.0).sqrt();
    let im = ((m - r) / 2.0).sqrt().copysign(i);
    Ok(vm.types.new_complex(re, im))
}

fn c_phase(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (r, i) = c_of(vm, &a[0])?;
    Ok(Value::Float(i.atan2(r)))
}

fn c_polar(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (r, i) = c_of(vm, &a[0])?;
    Ok(vm.types.new_tuple(vec![Value::Float(r.hypot(i)), Value::Float(i.atan2(r))]))
}

fn c_rect(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let m = ops::to_f64(vm, &a[0])?;
    let p = ops::to_f64(vm, &a[1])?;
    Ok(vm.types.new_complex(m * p.cos(), m * p.sin()))
}

pub fn init_cmath(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "cmath", &[("sqrt", c_sqrt), ("phase", c_phase), ("polar", c_polar), ("rect", c_rect)]);
    set(vm, &m, "pi", Value::Float(std::f64::consts::PI));
    set(vm, &m, "e", Value::Float(std::f64::consts::E));
    Ok(m)
}
