// SPDX-License-Identifier: MPL-2.0
//! `_struct`: pack/unpack with byte order prefixes and the standard codes.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

#[derive(Clone, Copy)]
struct Item {
    code: char,
    count: usize,
}

fn parse_fmt(vm: &Vm, fmt: &str) -> PyResult<(bool, bool, Vec<Item>)> {
    let mut chars = fmt.chars().peekable();
    let (little, native) = match chars.peek() {
        Some('<') => {
            chars.next();
            (true, false)
        }
        Some('>') | Some('!') => {
            chars.next();
            (false, false)
        }
        Some('=') => {
            chars.next();
            (true, false)
        }
        Some('@') => {
            chars.next();
            (true, true)
        }
        _ => (true, true),
    };
    let mut items = Vec::new();
    let mut num = String::new();
    for c in chars {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if c.is_whitespace() {
            continue;
        }
        let count = if num.is_empty() { 1 } else { num.parse().unwrap_or(1) };
        num.clear();
        if !"xcbB?hHiIlLqQnNefdspP".contains(c) {
            return Err(vm.exc_msg("ValueError", format!("bad char in struct format: {c}")));
        }
        items.push(Item { code: c, count });
    }
    Ok((little, native, items))
}

fn size_of(c: char, native: bool) -> usize {
    match c {
        'x' | 'c' | 'b' | 'B' | '?' | 's' | 'p' => 1,
        'h' | 'H' | 'e' => 2,
        'i' | 'I' | 'f' => 4,
        'l' | 'L' => if native { 8 } else { 4 },
        'q' | 'Q' | 'd' | 'n' | 'N' | 'P' => 8,
        _ => 0,
    }
}

fn calc(items: &[Item], native: bool) -> usize {
    let mut n = 0usize;
    for it in items {
        let s = size_of(it.code, native);
        if native && s > 1 && !matches!(it.code, 's' | 'p') {
            n = n.div_ceil(s) * s;
        }
        n += if matches!(it.code, 's' | 'p') { it.count } else { s * it.count };
    }
    n
}

pub fn pack_impl(vm: &mut Vm, fmt: &str, args: &[Value]) -> PyResult<Vec<u8>> {
    let (little, native, items) = parse_fmt(vm, fmt)?;
    let mut out = Vec::new();
    let mut ai = 0;
    let mut next = |vm: &Vm| -> PyResult<Value> {
        let v = args.get(ai).cloned().ok_or_else(|| vm.exc_msg("ValueError", "pack expected more items for packing"))?;
        ai += 1;
        Ok(v)
    };
    for it in &items {
        let s = size_of(it.code, native);
        if native && s > 1 && !matches!(it.code, 's' | 'p') {
            while out.len() % s != 0 {
                out.push(0);
            }
        }
        if matches!(it.code, 's' | 'p') {
            let v = next(vm)?;
            let mut b = ops::bytes_of(&v).ok_or_else(|| vm.exc_msg("ValueError", "argument for 's' must be a bytes object"))?;
            b.resize(it.count, 0);
            out.extend(b);
            continue;
        }
        for _ in 0..it.count {
            match it.code {
                'x' => out.push(0),
                'c' => {
                    let v = next(vm)?;
                    let b = ops::bytes_of(&v).filter(|b| b.len() == 1).ok_or_else(|| vm.exc_msg("ValueError", "char format requires a bytes object of length 1"))?;
                    out.push(b[0]);
                }
                '?' => {
                    let v = next(vm)?;
                    out.push(ops::truthy(vm, &v)? as u8);
                }
                'e' | 'f' | 'd' => {
                    let v = next(vm)?;
                    let f = ops::to_f64(vm, &v)?;
                    let bytes = match it.code {
                        'f' => if little { (f as f32).to_le_bytes().to_vec() } else { (f as f32).to_be_bytes().to_vec() },
                        'd' => if little { f.to_le_bytes().to_vec() } else { f.to_be_bytes().to_vec() },
                        _ => {
                            let h = f16_bits(f);
                            if little { h.to_le_bytes().to_vec() } else { h.to_be_bytes().to_vec() }
                        }
                    };
                    out.extend(bytes);
                }
                c => {
                    let v = next(vm)?;
                    let n = ops::to_bigint(&v).ok_or_else(|| vm.exc_msg("ValueError", "required argument is not an integer"))?;
                    let signed = c.is_ascii_lowercase() || c == 'n';
                    let bits = (s * 8) as u32;
                    let (lo, hi) = if signed { (-(BigInt::from(1) << (bits - 1)), (BigInt::from(1) << (bits - 1)) - 1) } else { (BigInt::from(0), (BigInt::from(1) << bits) - 1) };
                    if n < lo || n > hi {
                        return Err(vm.exc_msg("ValueError", format!("'{c}' format requires {lo} <= number <= {hi}")));
                    }
                    let mut b = n.to_signed_bytes_le();
                    let fill = if n < BigInt::from(0) { 0xff } else { 0 };
                    b.resize(s, fill);
                    if !little {
                        b.reverse();
                    }
                    out.extend(b);
                }
            }
        }
    }
    if ai != args.len() {
        return Err(vm.exc_msg("ValueError", format!("pack expected {} items for packing (got {})", ai, args.len())));
    }
    Ok(out)
}

fn f16_bits(f: f64) -> u16 {
    let x = f as f32;
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7fffff;
    if exp <= 0 {
        sign
    } else if exp >= 31 {
        sign | 0x7c00
    } else {
        sign | ((exp as u16) << 10) | (mant >> 13) as u16
    }
}

fn f16_value(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x3ff) as f64;
    sign * match exp {
        0 => mant * 2f64.powi(-24),
        31 => if mant == 0.0 { f64::INFINITY } else { f64::NAN },
        e => (1.0 + mant / 1024.0) * 2f64.powi(e - 15),
    }
}

pub fn unpack_impl(vm: &mut Vm, fmt: &str, data: &[u8], offset: usize) -> PyResult<Vec<Value>> {
    let (little, native, items) = parse_fmt(vm, fmt)?;
    let need = calc(&items, native);
    if data.len() < offset + need {
        return Err(vm.exc_msg("ValueError", format!("unpack requires a buffer of {need} bytes")));
    }
    let mut p = offset;
    let mut out = Vec::new();
    for it in &items {
        let s = size_of(it.code, native);
        if native && s > 1 && !matches!(it.code, 's' | 'p') {
            p = offset + (p - offset).div_ceil(s) * s;
        }
        if matches!(it.code, 's' | 'p') {
            out.push(vm.types.new_bytes(data[p..p + it.count].to_vec()));
            p += it.count;
            continue;
        }
        for _ in 0..it.count {
            let mut b = data[p..p + s].to_vec();
            p += s;
            if !little {
                b.reverse();
            }
            match it.code {
                'x' => {}
                'c' => out.push(vm.types.new_bytes(b)),
                '?' => out.push(Value::Bool(b[0] != 0)),
                'e' => out.push(Value::Float(f16_value(u16::from_le_bytes([b[0], b[1]])))),
                'f' => out.push(Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64)),
                'd' => out.push(Value::Float(f64::from_le_bytes(b.try_into().unwrap()))),
                c => {
                    let signed = c.is_ascii_lowercase() || c == 'n';
                    let n = if signed { BigInt::from_signed_bytes_le(&b) } else { BigInt::from_bytes_le(num_bigint::Sign::Plus, &b) };
                    out.push(match n.to_i64() {
                        Some(i) => Value::Int(i),
                        None => vm.types.new_bigint(n),
                    });
                }
            }
        }
    }
    Ok(out)
}

fn fmt_arg(vm: &Vm, v: &Value) -> PyResult<String> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_string());
    }
    if let Some(b) = ops::bytes_of(v) {
        return Ok(String::from_utf8_lossy(&b).to_string());
    }
    if let Value::Obj(o) = v {
        if let Some(f) = ops::get_raw_attr(o, "format") {
            return fmt_arg(vm, &f);
        }
    }
    Err(vm.type_error("Struct() argument 1 must be a str or bytes object"))
}

fn pack(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = fmt_arg(vm, &a[0])?;
    let b = pack_impl(vm, &f, &a[1..])?;
    Ok(vm.types.new_bytes(b))
}

fn unpack(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = fmt_arg(vm, &a[0])?;
    let d = ops::bytes_of(&a[1]).ok_or_else(|| vm.type_error("a bytes-like object is required"))?;
    let (_, native, items) = parse_fmt(vm, &f)?;
    if d.len() != calc(&items, native) {
        return Err(vm.exc_msg("ValueError", format!("unpack requires a buffer of {} bytes", calc(&items, native))));
    }
    let v = unpack_impl(vm, &f, &d, 0)?;
    Ok(vm.types.new_tuple(v))
}

fn unpack_from(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let f = fmt_arg(vm, &a[0])?;
    let d = ops::bytes_of(&a[1]).ok_or_else(|| vm.type_error("a bytes-like object is required"))?;
    let off = match a.get(2).or_else(|| k.iter().find(|(n, _)| &**n == "offset").map(|(_, v)| v)) {
        Some(v) => ops::index_of(vm, v)?.max(0) as usize,
        None => 0,
    };
    let v = unpack_impl(vm, &f, &d, off)?;
    Ok(vm.types.new_tuple(v))
}

fn calcsize(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = fmt_arg(vm, &a[0])?;
    let (_, native, items) = parse_fmt(vm, &f)?;
    Ok(Value::Int(calc(&items, native) as i64))
}

fn iter_unpack(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = fmt_arg(vm, &a[0])?;
    let d = ops::bytes_of(&a[1]).ok_or_else(|| vm.type_error("a bytes-like object is required"))?;
    let (_, native, items) = parse_fmt(vm, &f)?;
    let sz = calc(&items, native).max(1);
    let mut out = Vec::new();
    let mut off = 0;
    while off + sz <= d.len() {
        let v = unpack_impl(vm, &f, &d, off)?;
        out.push(vm.types.new_tuple(v));
        off += sz;
    }
    Ok(vm.types.new_iter(IterState::Values { items: out, idx: 0 }))
}

fn struct_init(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let f = fmt_arg(vm, &a[1])?;
    let (_, native, items) = parse_fmt(vm, &f)?;
    let fv = vm.types.new_string(f);
    ops::set_raw_attr(vm, o, "format", fv);
    ops::set_raw_attr(vm, o, "size", Value::Int(calc(&items, native) as i64));
    Ok(Value::None)
}

fn s_pack(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    pack(vm, a, k)
}
fn s_unpack(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    unpack(vm, a, k)
}
fn s_unpack_from(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    unpack_from(vm, a, k)
}
fn s_iter_unpack(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    iter_unpack(vm, a, k)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_struct", &[("pack", pack), ("unpack", unpack), ("unpack_from", unpack_from), ("calcsize", calcsize), ("iter_unpack", iter_unpack)]);
    let c = vm.types.new_builtin_type("Struct", &vm.types.object.clone(), BaseKind::Object, false);
    for (n, f) in [("__init__", struct_init as NativeFn), ("pack", s_pack), ("unpack", s_unpack), ("unpack_from", s_unpack_from), ("iter_unpack", s_iter_unpack)] {
        vm.types.add_method(&c, n, f);
    }
    set(vm, &m, "Struct", Value::Obj(c));
    let e = vm.types.new_builtin_type("error", &vm.types.exc("Exception"), BaseKind::Exception, false);
    set(vm, &m, "error", Value::Obj(e));
    Ok(m)
}
