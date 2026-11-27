// SPDX-License-Identifier: MPL-2.0
//! The `bytes`/`bytearray` methods that operate byte for byte like their
//! `str` counterparts. Their arguments and results map through latin-1
//! (one char per byte), which is exact for searching, padding and
//! partitioning; methods whose meaning differs for bytes (ASCII-only case
//! and character classes, whitespace defaults, `translate`) are native.
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::cell::RefCell;

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn unlatin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

fn bytes_like(vm: &Vm, v: &Value) -> PyResult<Vec<u8>> {
    ops::bytes_of(v).ok_or_else(|| vm.type_error(format!("a bytes-like object is required, not '{}'", ops::type_name(vm, v))))
}

fn same_type(vm: &Vm, model: &Value, b: Vec<u8>) -> Value {
    if matches!(model.payload(), Some(Payload::ByteArray(_))) {
        return Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(b))));
    }
    vm.types.new_bytes(b)
}

/// Converts one argument: bytes-like -> latin-1 str, a tuple of them
/// (startswith) element-wise, an int byte (find/count) -> its char when
/// `int_is_byte`; other values (start, end, count) pass through.
fn arg(vm: &mut Vm, v: &Value, int_is_byte: bool) -> PyResult<Value> {
    if let Some(b) = ops::bytes_of(v)
        && !matches!(v, Value::Int(_) | Value::Bool(_))
    {
        return Ok(vm.types.new_str(&latin1(&b)));
    }
    if let Some(Payload::Tuple(t)) = v.payload() {
        let items = t.iter().map(|i| arg(vm, i, false)).collect::<PyResult<Vec<_>>>()?;
        return Ok(vm.types.new_tuple(items));
    }
    if int_is_byte && let Value::Int(i) = v {
        if !(0..256).contains(i) {
            return Err(vm.value_error("byte must be in range(0, 256)"));
        }
        return Ok(vm.types.new_str(&(*i as u8 as char).to_string()));
    }
    Ok(v.clone())
}

fn result(vm: &mut Vm, model: &Value, r: Value) -> PyResult<Value> {
    if let Some(s) = r.as_str() {
        return Ok(same_type(vm, model, unlatin1(s)));
    }
    match r.payload() {
        Some(Payload::Tuple(t)) => {
            let items = t.iter().map(|i| result(vm, model, i.clone())).collect::<PyResult<Vec<_>>>()?;
            Ok(vm.types.new_tuple(items))
        }
        Some(Payload::List(l)) => {
            let items = l.borrow().clone();
            let items = items.into_iter().map(|i| result(vm, model, i)).collect::<PyResult<Vec<_>>>()?;
            Ok(vm.types.new_list(items))
        }
        _ => Ok(r),
    }
}

fn via_str(vm: &mut Vm, a: &[Value], k: Kw, name: &str, int_first: bool) -> PyResult<Value> {
    let this = bytes_like(vm, &a[0])?;
    let s = vm.types.new_str(&latin1(&this));
    let mut args = Vec::with_capacity(a.len() - 1);
    for (i, v) in a[1..].iter().enumerate() {
        args.push(arg(vm, v, int_first && i == 0)?);
    }
    let mut kwargs = Vec::with_capacity(k.len());
    for (n, v) in k {
        kwargs.push((n.clone(), arg(vm, v, false)?));
    }
    let m = ops::getattr(vm, &s, name)?;
    let r = vm.call(&m, args, kwargs)?;
    result(vm, &a[0], r)
}

macro_rules! bridged {
    ($($f:ident => $name:literal, $int:expr;)*) => {
        $(fn $f(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> { via_str(vm, a, k, $name, $int) })*
    };
}

bridged! {
    b_find => "find", true;
    b_rfind => "rfind", true;
    b_index => "index", true;
    b_rindex => "rindex", true;
    b_count => "count", true;
    b_partition => "partition", false;
    b_rpartition => "rpartition", false;
    b_replace => "replace", false;
    b_startswith => "startswith", false;
    b_endswith => "endswith", false;
    b_center => "center", false;
    b_ljust => "ljust", false;
    b_rjust => "rjust", false;
    b_zfill => "zfill", false;
    b_expandtabs => "expandtabs", false;
    b_removeprefix => "removeprefix", false;
    b_removesuffix => "removesuffix", false;
}

const WS: &[u8] = b" \t\n\r\x0b\x0c";

fn opt_int(vm: &mut Vm, v: Option<&Value>, default: i64) -> PyResult<i64> {
    match v {
        None | Some(Value::None) => Ok(default),
        Some(v) => ops::index_of(vm, v),
    }
}

fn split_args<'a>(a: &'a [Value], k: Kw<'a>) -> (Option<&'a Value>, Option<&'a Value>) {
    let kw = |n: &str| k.iter().find(|(x, _)| &**x == n).map(|(_, v)| v);
    (a.get(1).or_else(|| kw("sep")), a.get(2).or_else(|| kw("maxsplit")))
}

/// `split(sep=None, maxsplit=-1)` / `rsplit`.
fn split_impl(vm: &mut Vm, a: &[Value], k: Kw, right: bool) -> PyResult<Value> {
    let b = bytes_like(vm, &a[0])?;
    let (sep, max) = split_args(a, k);
    let max = opt_int(vm, max, -1)?;
    let mut parts: Vec<Vec<u8>> = Vec::new();
    match sep.filter(|v| !v.is_none()) {
        None => {
            let mut words: Vec<(usize, usize)> = Vec::new();
            let mut i = 0;
            while i < b.len() {
                while i < b.len() && WS.contains(&b[i]) {
                    i += 1;
                }
                let s = i;
                while i < b.len() && !WS.contains(&b[i]) {
                    i += 1;
                }
                if s < i {
                    words.push((s, i));
                }
            }
            if max >= 0 && words.len() as i64 > max + 1 {
                let m = max as usize;
                if right {
                    let cut = words.len() - m;
                    // The unsplit remainder keeps its leading whitespace.
                    parts.push(b[..words[cut - 1].1].to_vec());
                    parts.extend(words[cut..].iter().map(|(s, e)| b[*s..*e].to_vec()));
                } else {
                    parts.extend(words[..m].iter().map(|(s, e)| b[*s..*e].to_vec()));
                    // ... and its trailing whitespace.
                    parts.push(b[words[m].0..].to_vec());
                }
            } else {
                parts.extend(words.iter().map(|(s, e)| b[*s..*e].to_vec()));
            }
        }
        Some(sep) => {
            let sep = bytes_like(vm, sep)?;
            if sep.is_empty() {
                return Err(vm.value_error("empty separator"));
            }
            let s = vm.types.new_str(&latin1(&b));
            let sv = vm.types.new_str(&latin1(&sep));
            let m = ops::getattr(vm, &s, if right { "rsplit" } else { "split" })?;
            let r = vm.call(&m, vec![sv, Value::Int(max)], Vec::new())?;
            return result(vm, &a[0], r);
        }
    }
    let items = parts.into_iter().map(|p| same_type(vm, &a[0], p)).collect();
    Ok(vm.types.new_list(items))
}

fn b_split(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    split_impl(vm, a, k, false)
}

fn b_rsplit(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    split_impl(vm, a, k, true)
}

/// `splitlines(keepends=False)`: `\n`, `\r` and `\r\n` only.
fn b_splitlines(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let b = bytes_like(vm, &a[0])?;
    let keep = match a.get(1).or_else(|| k.iter().find(|(n, _)| &**n == "keepends").map(|(_, v)| v)) {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let mut out = Vec::new();
    let mut i = 0;
    let mut start = 0;
    while i < b.len() {
        let eol = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => 0,
        };
        if eol > 0 {
            let end = if keep { i + eol } else { i };
            out.push(b[start..end].to_vec());
            i += eol;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < b.len() {
        out.push(b[start..].to_vec());
    }
    let items = out.into_iter().map(|p| same_type(vm, &a[0], p)).collect();
    Ok(vm.types.new_list(items))
}

fn map_bytes(vm: &mut Vm, a: &[Value], f: impl Fn(&[u8]) -> Vec<u8>) -> PyResult<Value> {
    let b = bytes_like(vm, &a[0])?;
    Ok(same_type(vm, &a[0], f(&b)))
}

fn b_capitalize(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    map_bytes(vm, a, |b| b.iter().enumerate().map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect())
}

fn b_swapcase(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    map_bytes(vm, a, |b| b.iter().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() }).collect())
}

fn b_title(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    map_bytes(vm, a, |b| {
        let mut prev_cased = false;
        b.iter()
            .map(|&c| {
                let r = if prev_cased { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() };
                prev_cased = c.is_ascii_alphabetic();
                r
            })
            .collect()
    })
}

fn pred(vm: &mut Vm, a: &[Value], f: impl Fn(&[u8]) -> bool) -> PyResult<Value> {
    let b = bytes_like(vm, &a[0])?;
    Ok(Value::Bool(f(&b)))
}

fn b_isalnum(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    pred(vm, a, |b| !b.is_empty() && b.iter().all(|c| c.is_ascii_alphanumeric()))
}

fn b_isascii(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    pred(vm, a, |b| b.is_ascii())
}

fn b_islower(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    pred(vm, a, |b| b.iter().any(|c| c.is_ascii_lowercase()) && !b.iter().any(|c| c.is_ascii_uppercase()))
}

fn b_isupper(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    pred(vm, a, |b| b.iter().any(|c| c.is_ascii_uppercase()) && !b.iter().any(|c| c.is_ascii_lowercase()))
}

fn b_istitle(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    pred(vm, a, |b| {
        let mut cased = false;
        let mut prev_cased = false;
        for &c in b {
            if c.is_ascii_uppercase() {
                if prev_cased {
                    return false;
                }
                prev_cased = true;
                cased = true;
            } else if c.is_ascii_lowercase() {
                if !prev_cased {
                    return false;
                }
                prev_cased = true;
                cased = true;
            } else {
                prev_cased = false;
            }
        }
        cased
    })
}

/// `bytes.maketrans(from, to)`: the 256-byte translation table.
fn b_maketrans(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    // Called as a static method: (from, to) or (cls, from, to).
    let off = a.len().saturating_sub(2);
    let from = bytes_like(vm, &a[off])?;
    let to = bytes_like(vm, &a[off + 1])?;
    if from.len() != to.len() {
        return Err(vm.value_error("maketrans arguments must have same length"));
    }
    let mut t: Vec<u8> = (0..=255u8).collect();
    for (f, x) in from.iter().zip(to) {
        t[*f as usize] = x;
    }
    Ok(vm.types.new_bytes(t))
}

/// `translate(table, /, delete=b'')`.
fn b_translate(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let b = bytes_like(vm, &a[0])?;
    let table = a.get(1).cloned().unwrap_or(Value::None);
    let table = if table.is_none() {
        None
    } else {
        let t = bytes_like(vm, &table)?;
        if t.len() != 256 {
            return Err(vm.value_error("translation table must be 256 characters long"));
        }
        Some(t)
    };
    let delete = match a.get(2).or_else(|| k.iter().find(|(n, _)| &**n == "delete").map(|(_, v)| v)) {
        Some(d) => bytes_like(vm, d)?,
        None => Vec::new(),
    };
    let out = b.iter().filter(|c| !delete.contains(c)).map(|&c| table.as_ref().map_or(c, |t| t[c as usize])).collect();
    Ok(same_type(vm, &a[0], out))
}

fn with_bytearray<R>(vm: &Vm, v: &Value, f: impl FnOnce(&mut Vec<u8>) -> R) -> PyResult<R> {
    match v.payload() {
        Some(Payload::ByteArray(x)) => Ok(f(&mut x.borrow_mut())),
        _ => Err(vm.type_error("descriptor requires a 'bytearray' object")),
    }
}

fn byte_value(vm: &mut Vm, v: &Value) -> PyResult<u8> {
    let n = ops::index_of(vm, v)?;
    if !(0..256).contains(&n) {
        return Err(vm.value_error("byte must be in range(0, 256)"));
    }
    Ok(n as u8)
}

fn ba_insert(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let i = ops::index_of(vm, &a[1])?;
    let c = byte_value(vm, &a[2])?;
    with_bytearray(vm, &a[0], |x| {
        let n = x.len() as i64;
        let p = if i < 0 { (i + n).max(0) } else { i.min(n) } as usize;
        x.insert(p, c);
    })?;
    Ok(Value::None)
}

fn ba_pop(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let i = opt_int(vm, a.get(1), -1)?;
    let r = with_bytearray(vm, &a[0], |x| {
        let n = x.len() as i64;
        let p = if i < 0 { i + n } else { i };
        if x.is_empty() || p < 0 || p >= n { None } else { Some(x.remove(p as usize)) }
    })?;
    match r {
        Some(c) => Ok(Value::Int(c as i64)),
        None => Err(vm.index_error("pop index out of range")),
    }
}

fn ba_remove(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = byte_value(vm, &a[1])?;
    let found = with_bytearray(vm, &a[0], |x| match x.iter().position(|b| *b == c) {
        Some(p) => {
            x.remove(p);
            true
        }
        None => false,
    })?;
    if found { Ok(Value::None) } else { Err(vm.value_error("value not found in bytearray")) }
}

fn ba_reverse(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    with_bytearray(vm, &a[0], |x| x.reverse())?;
    Ok(Value::None)
}

fn ba_copy(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let b = with_bytearray(vm, &a[0], |x| x.clone())?;
    Ok(same_type(vm, &a[0], b))
}

/// Installs the methods on `bytes` and `bytearray` (replacing the simpler
/// versions of `find`, `index`, `count`, `replace`, `split`, `startswith`
/// and `endswith`).
pub fn install(vm: &Vm, bytes: &ObjRef, bytearray: &ObjRef) {
    let methods: &[(&str, NativeFn)] = &[
        ("find", b_find),
        ("rfind", b_rfind),
        ("index", b_index),
        ("rindex", b_rindex),
        ("count", b_count),
        ("partition", b_partition),
        ("rpartition", b_rpartition),
        ("replace", b_replace),
        ("startswith", b_startswith),
        ("endswith", b_endswith),
        ("center", b_center),
        ("ljust", b_ljust),
        ("rjust", b_rjust),
        ("zfill", b_zfill),
        ("expandtabs", b_expandtabs),
        ("removeprefix", b_removeprefix),
        ("removesuffix", b_removesuffix),
        ("split", b_split),
        ("rsplit", b_rsplit),
        ("splitlines", b_splitlines),
        ("capitalize", b_capitalize),
        ("swapcase", b_swapcase),
        ("title", b_title),
        ("isalnum", b_isalnum),
        ("isascii", b_isascii),
        ("islower", b_islower),
        ("isupper", b_isupper),
        ("istitle", b_istitle),
        ("translate", b_translate),
    ];
    for t in [bytes, bytearray] {
        for (n, f) in methods {
            let v = vm.types.new_builtin(n, *f);
            vm.types.set_attr(t, n, v);
        }
        let mt = vm.types.new_builtin("maketrans", b_maketrans);
        let mt = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(crate::dict::Dict::new()), Payload::StaticMethod(mt)));
        vm.types.set_attr(t, "maketrans", mt);
    }
    for (n, f) in [("insert", ba_insert as NativeFn), ("pop", ba_pop), ("remove", ba_remove), ("reverse", ba_reverse), ("copy", ba_copy)] {
        let v = vm.types.new_builtin(n, f);
        vm.types.set_attr(bytearray, n, v);
    }
}
