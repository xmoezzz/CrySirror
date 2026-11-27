// SPDX-License-Identifier: MPL-2.0
//! `binascii`: hex, base64 and crc32 conversions used by `base64`.

use super::module_with;
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn arg(vm: &Vm, v: &Value) -> PyResult<Vec<u8>> {
    if let Some(s) = v.as_str() {
        if s.is_ascii() {
            return Ok(s.as_bytes().to_vec());
        }
    }
    ops::bytes_of(v).ok_or_else(|| vm.type_error("a bytes-like object is required"))
}

pub fn b64encode(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63]);
        out.push(B64[(n >> 12) as usize & 63]);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] } else { b'=' });
    }
    out
}

pub fn b64decode(d: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(d.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    let mut pad = 0;
    for &c in d {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            _ => continue,
        };
        if pad > 0 {
            return Err("Incorrect padding");
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn hexlify(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = arg(vm, &a[0])?;
    let s: String = d.iter().map(|b| format!("{b:02x}")).collect();
    Ok(vm.types.new_bytes(s.into_bytes()))
}

fn unhexlify(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = arg(vm, &a[0])?;
    if d.len() % 2 != 0 {
        return Err(vm.value_error("Odd-length string"));
    }
    let s = String::from_utf8_lossy(&d).to_string();
    let mut out = Vec::new();
    for i in (0..s.len()).step_by(2) {
        out.push(u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| vm.value_error("Non-hexadecimal digit found"))?);
    }
    Ok(vm.types.new_bytes(out))
}

fn b2a_base64(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let d = arg(vm, &a[0])?;
    let mut out = b64encode(&d);
    let newline = k.iter().find(|(n, _)| &**n == "newline").map(|(_, v)| v.clone());
    let nl = match newline {
        Some(v) => ops::truthy(vm, &v)?,
        None => true,
    };
    if nl {
        out.push(b'\n');
    }
    Ok(vm.types.new_bytes(out))
}

fn a2b_base64(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = arg(vm, &a[0])?;
    let out = b64decode(&d).map_err(|e| vm.value_error(e))?;
    Ok(vm.types.new_bytes(out))
}

fn crc32(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = arg(vm, &a[0])?;
    let start = match a.get(1) {
        Some(v) => ops::index_of(vm, v)? as u32,
        None => 0,
    };
    Ok(Value::Int(super::zlib::crc32_of(&d, start) as i64))
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "binascii", &[("hexlify", hexlify), ("b2a_hex", hexlify), ("unhexlify", unhexlify), ("a2b_hex", unhexlify), ("b2a_base64", b2a_base64), ("a2b_base64", a2b_base64), ("crc32", crc32)]);
    let e = vm.types.new_builtin_type("Error", &vm.types.exc("ValueError"), BaseKind::Exception, false);
    super::set(vm, &m, "Error", Value::Obj(e));
    Ok(m)
}
