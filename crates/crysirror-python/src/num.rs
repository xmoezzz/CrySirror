// SPDX-License-Identifier: MPL-2.0
//! Numeric tower: bool ⊂ int (i64 with BigInt overflow) ⊂ float ⊂ complex,
//! with Python floor division/modulo semantics and the Python 2 classic
//! division used by Ren'Py 7 scripts.

use crate::ast::{BinOp, UnaryOp};
use crate::object::{Payload, Value};
use crate::vm::{PyResult, Vm};
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Pow, Signed, ToPrimitive, Zero};

#[derive(Clone, Debug)]
pub enum Num {
    I(i64),
    B(BigInt),
    F(f64),
    C(f64, f64),
}

pub fn num_of(v: &Value) -> Option<Num> {
    match v {
        Value::Bool(b) => Some(Num::I(*b as i64)),
        Value::Int(i) => Some(Num::I(*i)),
        Value::Float(f) => Some(Num::F(*f)),
        Value::Obj(o) => match &o.payload {
            Payload::Int(b) => Some(match b.to_i64() {
                Some(i) => Num::I(i),
                None => Num::B(b.clone()),
            }),
            Payload::Float(f) => Some(Num::F(*f)),
            Payload::Complex(r, i) => Some(Num::C(*r, *i)),
            _ => None,
        },
        Value::None => None,
    }
}

fn big(n: &Num) -> BigInt {
    match n {
        Num::I(i) => BigInt::from(*i),
        Num::B(b) => b.clone(),
        _ => BigInt::zero(),
    }
}

fn to_float(vm: &Vm, n: &Num) -> PyResult<f64> {
    match n {
        Num::I(i) => Ok(*i as f64),
        Num::B(b) => b.to_f64().filter(|f| f.is_finite()).ok_or_else(|| vm.exc_msg("OverflowError", "int too large to convert to float")),
        Num::F(f) => Ok(*f),
        Num::C(..) => Err(vm.type_error("can't convert complex to float")),
    }
}

pub fn int_value(vm: &Vm, b: BigInt) -> Value {
    vm.types.new_bigint(b)
}

fn zero_div(vm: &Vm, msg: &str) -> crate::vm::PyErr {
    vm.exc_msg("ZeroDivisionError", msg)
}

/// i64 fast path; None means "fall back to the general path".
pub fn int_fast(vm: &Vm, x: i64, y: i64, op: BinOp, classic: bool) -> PyResult<Option<Value>> {
    Ok(match op {
        BinOp::Add => x.checked_add(y).map(Value::Int),
        BinOp::Sub => x.checked_sub(y).map(Value::Int),
        BinOp::Mult => x.checked_mul(y).map(Value::Int),
        BinOp::FloorDiv => {
            if y == 0 {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            x.checked_div_euclid(y).map(|_| Value::Int(x.div_floor(&y)))
        }
        BinOp::Div if classic => {
            if y == 0 {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            x.checked_div(y).map(|_| Value::Int(x.div_floor(&y)))
        }
        BinOp::Div => {
            if y == 0 {
                return Err(zero_div(vm, "division by zero"));
            }
            Some(Value::Float(true_div_int(x, y)))
        }
        BinOp::Mod => {
            if y == 0 {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            x.checked_rem(y).map(|_| Value::Int(x.mod_floor(&y)))
        }
        BinOp::BitAnd => Some(Value::Int(x & y)),
        BinOp::BitOr => Some(Value::Int(x | y)),
        BinOp::BitXor => Some(Value::Int(x ^ y)),
        BinOp::LShift => {
            if y < 0 {
                return Err(vm.value_error("negative shift count"));
            }
            if y < 63 && (x.abs() as u64) < (1u64 << (62 - y as u32).min(62)) {
                Some(Value::Int(x << y))
            } else {
                None
            }
        }
        BinOp::RShift => {
            if y < 0 {
                return Err(vm.value_error("negative shift count"));
            }
            Some(Value::Int(if y >= 64 { if x < 0 { -1 } else { 0 } } else { x >> y }))
        }
        BinOp::Pow => {
            if y >= 0 {
                if y <= u32::MAX as i64 {
                    x.checked_pow(y as u32).map(Value::Int)
                } else {
                    None
                }
            } else {
                if x == 0 {
                    return Err(zero_div(vm, "0.0 cannot be raised to a negative power"));
                }
                Some(Value::Float((x as f64).powf(y as f64)))
            }
        }
        BinOp::MatMult => None,
    })
}

fn true_div_int(x: i64, y: i64) -> f64 {
    // Exact when both fit in 53 bits; otherwise go through BigInt rational.
    if x.unsigned_abs() < (1 << 53) && y.unsigned_abs() < (1 << 53) {
        return x as f64 / y as f64;
    }
    big_true_div(&BigInt::from(x), &BigInt::from(y))
}

fn big_true_div(a: &BigInt, b: &BigInt) -> f64 {
    let (q, r) = a.div_rem(b);
    match (q.to_f64(), r.to_f64(), b.to_f64()) {
        (Some(q), Some(r), Some(bf)) if bf.is_finite() => q + r / bf,
        _ => a.to_f64().unwrap_or(f64::NAN) / b.to_f64().unwrap_or(f64::NAN),
    }
}

pub fn float_mod(x: f64, y: f64) -> f64 {
    let m = x % y;
    if m != 0.0 && ((m < 0.0) != (y < 0.0)) { m + y } else if m == 0.0 { 0.0f64.copysign(y) } else { m }
}

pub fn float_floordiv(x: f64, y: f64) -> f64 {
    let m = float_mod(x, y);
    let d = (x - m) / y;
    if d == 0.0 { 0.0f64.copysign(x / y) } else { d.round_ties_even_like() }
}

trait RoundLike {
    fn round_ties_even_like(self) -> f64;
}
impl RoundLike for f64 {
    fn round_ties_even_like(self) -> f64 {
        let f = self.floor();
        if self - f > 0.5 { f + 1.0 } else { f }
    }
}

pub fn num_binary(vm: &mut Vm, x: Num, y: Num, op: BinOp, classic: bool) -> PyResult<Value> {
    if matches!(x, Num::C(..)) || matches!(y, Num::C(..)) {
        let (a, b) = complex_of(vm, &x)?;
        let (c, d) = complex_of(vm, &y)?;
        let (r, i) = match op {
            BinOp::Add => (a + c, b + d),
            BinOp::Sub => (a - c, b - d),
            BinOp::Mult => (a * c - b * d, a * d + b * c),
            BinOp::Div => {
                let den = c * c + d * d;
                if den == 0.0 {
                    return Err(zero_div(vm, "complex division by zero"));
                }
                ((a * c + b * d) / den, (b * c - a * d) / den)
            }
            BinOp::Pow => {
                if a == 0.0 && b == 0.0 {
                    if c == 0.0 && d == 0.0 {
                        (1.0, 0.0)
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    let (mag, arg) = ((a * a + b * b).sqrt(), b.atan2(a));
                    let lm = mag.ln();
                    let nm = (c * lm - d * arg).exp();
                    let na = d * lm + c * arg;
                    (nm * na.cos(), nm * na.sin())
                }
            }
            _ => return Err(vm.type_error(format!("unsupported operand type(s) for {}: 'complex'", crate::ops::op_symbol(op)))),
        };
        return Ok(vm.types.new_complex(r, i));
    }
    if matches!(x, Num::F(_)) || matches!(y, Num::F(_)) {
        let a = to_float(vm, &x)?;
        let b = to_float(vm, &y)?;
        let r = match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mult => a * b,
            BinOp::Div => {
                if b == 0.0 {
                    return Err(zero_div(vm, "float division by zero"));
                }
                a / b
            }
            BinOp::FloorDiv => {
                if b == 0.0 {
                    return Err(zero_div(vm, "float divmod()"));
                }
                float_floordiv(a, b)
            }
            BinOp::Mod => {
                if b == 0.0 {
                    return Err(zero_div(vm, "float modulo"));
                }
                float_mod(a, b)
            }
            BinOp::Pow => {
                if a == 0.0 && b < 0.0 {
                    return Err(zero_div(vm, "0.0 cannot be raised to a negative power"));
                }
                if a < 0.0 && b.fract() != 0.0 {
                    if crate::ops::vm_dialect(vm) == crate::Dialect::Py2 {
                        return Err(vm.value_error("negative number cannot be raised to a fractional power"));
                    }
                    return num_binary(vm, Num::C(a, 0.0), Num::C(b, 0.0), BinOp::Pow, false);
                }
                let r = a.powf(b);
                if r.is_infinite() && a.is_finite() && b.is_finite() {
                    return Err(vm.exc_msg("OverflowError", "(34, 'Numerical result out of range')"));
                }
                r
            }
            _ => return Err(vm.type_error(format!("unsupported operand type(s) for {}: 'float'", crate::ops::op_symbol(op)))),
        };
        return Ok(Value::Float(r));
    }
    let a = big(&x);
    let b = big(&y);
    Ok(match op {
        BinOp::Add => int_value(vm, a + b),
        BinOp::Sub => int_value(vm, a - b),
        BinOp::Mult => int_value(vm, a * b),
        BinOp::FloorDiv => {
            if b.is_zero() {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            int_value(vm, a.div_floor(&b))
        }
        BinOp::Div if classic => {
            if b.is_zero() {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            int_value(vm, a.div_floor(&b))
        }
        BinOp::Div => {
            if b.is_zero() {
                return Err(zero_div(vm, "division by zero"));
            }
            Value::Float(big_true_div(&a, &b))
        }
        BinOp::Mod => {
            if b.is_zero() {
                return Err(zero_div(vm, "integer division or modulo by zero"));
            }
            int_value(vm, a.mod_floor(&b))
        }
        BinOp::BitAnd => int_value(vm, a & b),
        BinOp::BitOr => int_value(vm, a | b),
        BinOp::BitXor => int_value(vm, a ^ b),
        BinOp::LShift => {
            let n = b.to_i64().ok_or_else(|| vm.exc_msg("OverflowError", "shift count too large"))?;
            if n < 0 {
                return Err(vm.value_error("negative shift count"));
            }
            int_value(vm, a << n as usize)
        }
        BinOp::RShift => {
            let n = b.to_i64().unwrap_or(i64::MAX);
            if n < 0 {
                return Err(vm.value_error("negative shift count"));
            }
            int_value(vm, a.div_floor(&(BigInt::one() << n.min(1 << 20) as usize)))
        }
        BinOp::Pow => {
            if b.is_negative() {
                let af = a.to_f64().unwrap_or(f64::INFINITY);
                let bf = b.to_f64().unwrap_or(f64::NEG_INFINITY);
                if af == 0.0 {
                    return Err(zero_div(vm, "0.0 cannot be raised to a negative power"));
                }
                Value::Float(af.powf(bf))
            } else {
                let e = b.to_u32().ok_or_else(|| vm.exc_msg("OverflowError", "exponent too large"))?;
                int_value(vm, Pow::pow(a, e))
            }
        }
        BinOp::MatMult => return Err(vm.type_error("unsupported operand type(s) for @: 'int' and 'int'")),
    })
}

fn complex_of(vm: &Vm, n: &Num) -> PyResult<(f64, f64)> {
    match n {
        Num::C(r, i) => Ok((*r, *i)),
        other => Ok((to_float(vm, other)?, 0.0)),
    }
}

pub fn num_unary(vm: &mut Vm, n: Num, op: UnaryOp) -> PyResult<Value> {
    Ok(match (n, op) {
        (Num::I(i), UnaryOp::USub) => match i.checked_neg() {
            Some(v) => Value::Int(v),
            None => int_value(vm, -BigInt::from(i)),
        },
        (Num::I(i), UnaryOp::UAdd) => Value::Int(i),
        (Num::I(i), UnaryOp::Invert) => Value::Int(!i),
        (Num::B(b), UnaryOp::USub) => int_value(vm, -b),
        (Num::B(b), UnaryOp::UAdd) => int_value(vm, b),
        (Num::B(b), UnaryOp::Invert) => int_value(vm, -b - 1),
        (Num::F(f), UnaryOp::USub) => Value::Float(-f),
        (Num::F(f), UnaryOp::UAdd) => Value::Float(f),
        (Num::C(r, i), UnaryOp::USub) => vm.types.new_complex(-r, -i),
        (Num::C(r, i), UnaryOp::UAdd) => vm.types.new_complex(r, i),
        _ => return Err(vm.type_error("bad operand type for unary ~")),
    })
}

pub fn num_eq(a: &Num, b: &Num) -> bool {
    match (a, b) {
        (Num::I(x), Num::I(y)) => x == y,
        (Num::B(x), Num::B(y)) => x == y,
        (Num::I(_), Num::B(_)) | (Num::B(_), Num::I(_)) => false,
        (Num::C(r, i), o) | (o, Num::C(r, i)) => match o {
            Num::C(r2, i2) => r == r2 && i == i2,
            _ => *i == 0.0 && num_eq(&Num::F(*r), o),
        },
        (Num::F(x), Num::F(y)) => x == y,
        (Num::F(f), other) | (other, Num::F(f)) => float_int_cmp(*f, other) == Some(0),
    }
}

/// Exact float/int comparison (CPython semantics, no rounding of large ints).
fn float_int_cmp(f: f64, other: &Num) -> Option<i32> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 { 1 } else { -1 });
    }
    let i = big(other);
    let fl = f.floor();
    let fb = BigInt::from_f64_exact(fl)?;
    if fb < i {
        Some(-1)
    } else if fb > i {
        Some(1)
    } else if f > fl {
        Some(1)
    } else {
        Some(0)
    }
}

trait FromF64Exact {
    fn from_f64_exact(f: f64) -> Option<BigInt>;
}
impl FromF64Exact for BigInt {
    fn from_f64_exact(f: f64) -> Option<BigInt> {
        num_traits::FromPrimitive::from_f64(f)
    }
}

/// None when unordered (NaN or complex).
pub fn num_cmp(a: &Num, b: &Num) -> Option<i32> {
    match (a, b) {
        (Num::I(x), Num::I(y)) => Some(x.cmp(y) as i32),
        (Num::C(..), _) | (_, Num::C(..)) => None,
        (Num::F(x), Num::F(y)) => x.partial_cmp(y).map(|o| o as i32),
        (Num::F(f), o) => float_int_cmp(*f, o),
        (o, Num::F(f)) => float_int_cmp(*f, o).map(|c| -c),
        _ => Some(big(a).cmp(&big(b)) as i32),
    }
}
