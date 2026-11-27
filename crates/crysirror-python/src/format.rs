// SPDX-License-Identifier: MPL-2.0
//! Text conversion: `repr` of builtin values (shortest round-trip floats),
//! the format-specification mini-language and printf-style `%` formatting.

use crate::num::{self, Num};
use crate::object::*;
use crate::vm::{PyResult, Vm};
use crate::Dialect;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};

/// Shortest round-trip digits and decimal exponent: value = 0.d1d2… × 10^exp.
fn shortest(f: f64) -> (String, i32) {
    let s = format!("{:e}", f.abs());
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0').to_string();
    let digits = if digits.is_empty() { "0".to_string() } else { digits };
    (digits, exp + 1)
}

pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (digits, exp) = shortest(f);
    let sign = if f < 0.0 { "-" } else { "" };
    // CPython float_repr_style 'short': fixed notation for -4 <= exp10 < 16.
    let e10 = exp - 1;
    if (-4..16).contains(&e10) {
        let n = digits.len() as i32;
        let s = if exp <= 0 {
            format!("0.{}{}", "0".repeat((-exp) as usize), digits)
        } else if exp >= n {
            format!("{}{}.0", digits, "0".repeat((exp - n) as usize))
        } else {
            format!("{}.{}", &digits[..exp as usize], &digits[exp as usize..])
        };
        return format!("{sign}{s}");
    }
    let mant = if digits.len() == 1 { digits.clone() } else { format!("{}.{}", &digits[..1], &digits[1..]) };
    format!("{sign}{mant}e{}{:02}", if e10 < 0 { '-' } else { '+' }, e10.abs())
}

/// Python 2 `str(float)`: `%.12g`.
pub fn float_str_py2(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
        let s = format_g(f, 12, false, false);
        return if s.contains('.') || s.contains('e') { s } else { s + ".0" };
    }
    if !f.is_finite() {
        return float_repr(f);
    }
    format_g(f, 12, false, false)
}

pub fn quote_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if is_unprintable(c) => {
                let n = c as u32;
                if n < 0x100 {
                    out.push_str(&format!("\\x{n:02x}"));
                } else if n < 0x10000 {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    out.push_str(&format!("\\U{n:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn is_unprintable(c: char) -> bool {
    let n = c as u32;
    (0x80..0xa0).contains(&n) || n == 0xad || (0x2028..=0x2029).contains(&n) || (0xd800..0xe000).contains(&n) || n == 0xfeff || (0xe000..0xf900).contains(&n)
}

pub fn bytes_repr(b: &[u8]) -> String {
    let q = if b.contains(&b'\'') && !b.contains(&b'"') { b'"' } else { b'\'' };
    let mut out = String::from("b");
    out.push(q as char);
    for &c in b {
        match c {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c as char);
            }
            0x20..=0x7e => out.push(c as char),
            c => out.push_str(&format!("\\x{c:02x}")),
        }
    }
    out.push(q as char);
    out
}

pub fn native_repr(vm: &mut Vm, v: &Value) -> PyResult<String> {
    Ok(match v {
        Value::None => "None".into(),
        Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        Value::Int(i) => i.to_string(),
        Value::Float(f) => float_repr(*f),
        Value::Obj(o) => match &o.payload {
            Payload::Str(s) => quote_str(s.as_str()),
            Payload::Bytes(b) => bytes_repr(b),
            Payload::ByteArray(b) => format!("bytearray({})", bytes_repr(&b.borrow())),
            Payload::Int(b) => b.to_string(),
            Payload::Float(f) => float_repr(*f),
            Payload::Complex(r, i) => complex_repr(*r, *i),
            Payload::Tuple(t) => {
                let t = t.to_vec();
                if t.len() == 1 {
                    format!("({},)", crate::ops::repr(vm, &t[0])?)
                } else {
                    format!("({})", seq_repr(vm, &t, Some(v))?)
                }
            }
            Payload::List(l) => {
                let l = l.borrow().clone();
                if repr_enter(vm, v) {
                    return Ok("[...]".into());
                }
                let r = seq_repr(vm, &l, None);
                repr_leave(vm, v);
                format!("[{}]", r?)
            }
            Payload::Dict(d) => {
                let items = d.borrow().items();
                if repr_enter(vm, v) {
                    return Ok("{...}".into());
                }
                let mut parts = Vec::with_capacity(items.len());
                let mut res = Ok(());
                for (k, val) in items {
                    match (crate::ops::repr(vm, &k), crate::ops::repr(vm, &val)) {
                        (Ok(a), Ok(b)) => parts.push(format!("{a}: {b}")),
                        (Err(e), _) | (_, Err(e)) => {
                            res = Err(e);
                            break;
                        }
                    }
                }
                repr_leave(vm, v);
                res?;
                let body = format!("{{{}}}", parts.join(", "));
                if crate::ops::lookup_user(&o.class(), "__repr__").is_none() && o.class().type_data().map(|t| !t.builtin).unwrap_or(false) {
                    // dict subclass without __repr__: CPython shows plain dict repr.
                    body
                } else {
                    body
                }
            }
            Payload::Set(_) | Payload::FrozenSet(_) => {
                let keys = crate::ops::set_keys(v);
                let frozen = matches!(o.payload, Payload::FrozenSet(_));
                let name = o.class_name();
                if keys.is_empty() {
                    return Ok(format!("{name}()"));
                }
                let body = format!("{{{}}}", seq_repr(vm, &keys, None)?);
                if frozen || name != "set" { format!("{name}({body})") } else { body }
            }
            Payload::Type(t) => {
                let module = crate::ops::get_raw_attr(o, "__module__").and_then(|m| m.as_str().map(|s| s.to_string()));
                match module {
                    Some(m) if m != "builtins" && !t.builtin => format!("<class '{}.{}'>", m, t.qualname.borrow()),
                    _ => format!("<class '{}'>", t.name.borrow()),
                }
            }
            Payload::Function(f) => format!("<function {} at 0x{:x}>", f.qualname.borrow(), v.id()),
            Payload::Builtin(b) => match &b.bound {
                Some(s) => format!("<built-in method {} of {} object at 0x{:x}>", b.name, crate::ops::type_name(vm, s), s.id()),
                None => format!("<built-in function {}>", b.name),
            },
            Payload::Method { func, slf } => {
                let fname = match func.payload() {
                    Some(Payload::Function(f)) => f.qualname.borrow().to_string(),
                    Some(Payload::Builtin(b)) => b.name.to_string(),
                    _ => "?".into(),
                };
                let sr = crate::ops::repr(vm, slf)?;
                format!("<bound method {fname} of {sr}>")
            }
            Payload::Module(m, _) => format!("<module '{m}'>"),
            Payload::Range(a, b, s) => {
                if *s == 1 { format!("range({a}, {b})") } else { format!("range({a}, {b}, {s})") }
            }
            Payload::Slice(a, b, c) => {
                let (a, b, c) = (a.clone(), b.clone(), c.clone());
                format!("slice({}, {}, {})", crate::ops::repr(vm, &a)?, crate::ops::repr(vm, &b)?, crate::ops::repr(vm, &c)?)
            }
            Payload::NotImplemented => "NotImplemented".into(),
            Payload::Ellipsis => "Ellipsis".into(),
            Payload::Generator(g) => format!("<generator object {} at 0x{:x}>", g.borrow().qualname, v.id()),
            Payload::Cell(_) => format!("<cell at 0x{:x}>", v.id()),
            Payload::Property { .. } => format!("<property object at 0x{:x}>", v.id()),
            Payload::DictView { dict, kind } => {
                let name = ["dict_keys", "dict_values", "dict_items"][*kind as usize];
                let d = Value::Obj(dict.clone());
                let items = crate::ops::collect(vm, &Value::Obj(new_obj(vm.types.dict_keys.clone(), None, Payload::DictView { dict: dict.clone(), kind: *kind })))?;
                let _ = d;
                format!("{name}([{}])", seq_repr(vm, &items, None)?)
            }
            Payload::Code(c) => format!("<code object {} at 0x{:x}, file \"{}\", line {}>", c.name, v.id(), c.filename, c.firstlineno),
            _ => {
                if crate::ops::is_subclass(&o.class(), &vm.types.exc("BaseException")) {
                    let args = exc_args(o);
                    let name = o.class_name();
                    if args.len() == 1 {
                        return Ok(format!("{}({})", name, crate::ops::repr(vm, &args[0])?));
                    }
                    return Ok(format!("{}({})", name, seq_repr(vm, &args, None)?));
                }
                let module = crate::ops::lookup_type(&o.class(), "__module__").and_then(|m| m.as_str().map(|s| s.to_string()));
                let qual = o.class().type_data().map(|t| t.qualname.borrow().to_string()).unwrap_or_default();
                match module {
                    Some(m) if m != "builtins" => format!("<{}.{} object at 0x{:x}>", m, qual, v.id()),
                    _ => format!("<{} object at 0x{:x}>", qual, v.id()),
                }
            }
        },
    })
}

thread_local! {
    static REPR_STACK: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn repr_enter(_vm: &Vm, v: &Value) -> bool {
    let id = v.id();
    REPR_STACK.with(|s| {
        let mut s = s.borrow_mut();
        if s.contains(&id) {
            true
        } else {
            s.push(id);
            false
        }
    })
}

fn repr_leave(_vm: &Vm, v: &Value) {
    let id = v.id();
    REPR_STACK.with(|s| s.borrow_mut().retain(|x| *x != id));
}

fn seq_repr(vm: &mut Vm, items: &[Value], _owner: Option<&Value>) -> PyResult<String> {
    let mut parts = Vec::with_capacity(items.len());
    for it in items {
        parts.push(crate::ops::repr(vm, it)?);
    }
    Ok(parts.join(", "))
}

pub fn complex_repr(r: f64, i: f64) -> String {
    let fmt = |x: f64| {
        let s = float_repr(x);
        s.strip_suffix(".0").map(|s| s.to_string()).unwrap_or(s)
    };
    if r == 0.0 && !r.is_sign_negative() {
        return format!("{}j", fmt(i));
    }
    let sign = if i < 0.0 || (i == 0.0 && i.is_sign_negative()) { "-" } else { "+" };
    format!("({}{}{}j)", fmt(r), sign, fmt(i.abs()))
}

pub fn exc_args(o: &ObjRef) -> Vec<Value> {
    match crate::ops::get_raw_attr(o, "args") {
        Some(Value::Obj(t)) => match &t.payload {
            Payload::Tuple(t) => t.to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub fn exception_str(vm: &mut Vm, o: &ObjRef) -> PyResult<String> {
    let args = exc_args(o);
    let is_key = crate::ops::is_subclass(&o.class(), &vm.types.exc("KeyError"));
    match args.len() {
        0 => Ok(String::new()),
        1 if is_key => crate::ops::repr(vm, &args[0]),
        1 => crate::ops::str(vm, &args[0]),
        _ => native_repr(vm, &vm.types.new_tuple(args.clone())),
    }
}

// ---- format specification mini-language ----

#[derive(Default, Debug)]
struct Spec {
    fill: char,
    align: Option<char>,
    sign: Option<char>,
    alt: bool,
    zero: bool,
    width: Option<usize>,
    grouping: Option<char>,
    precision: Option<usize>,
    ty: Option<char>,
}

fn parse_spec(vm: &Vm, s: &str) -> PyResult<Spec> {
    let c: Vec<char> = s.chars().collect();
    let mut sp = Spec { fill: ' ', ..Default::default() };
    let mut i = 0;
    let is_align = |ch: char| matches!(ch, '<' | '>' | '^' | '=');
    if c.len() >= 2 && is_align(c[1]) {
        sp.fill = c[0];
        sp.align = Some(c[1]);
        i = 2;
    } else if !c.is_empty() && is_align(c[0]) {
        sp.align = Some(c[0]);
        i = 1;
    }
    if i < c.len() && matches!(c[i], '+' | '-' | ' ') {
        sp.sign = Some(c[i]);
        i += 1;
    }
    if i < c.len() && c[i] == 'z' {
        i += 1;
    }
    if i < c.len() && c[i] == '#' {
        sp.alt = true;
        i += 1;
    }
    if i < c.len() && c[i] == '0' {
        sp.zero = true;
        i += 1;
    }
    let st = i;
    while i < c.len() && c[i].is_ascii_digit() {
        i += 1;
    }
    if i > st {
        sp.width = c[st..i].iter().collect::<String>().parse().ok();
    }
    if i < c.len() && (c[i] == ',' || c[i] == '_') {
        sp.grouping = Some(c[i]);
        i += 1;
    }
    if i < c.len() && c[i] == '.' {
        i += 1;
        let st = i;
        while i < c.len() && c[i].is_ascii_digit() {
            i += 1;
        }
        if i == st {
            return Err(vm.value_error("Format specifier missing precision"));
        }
        sp.precision = c[st..i].iter().collect::<String>().parse().ok();
    }
    if i < c.len() {
        sp.ty = Some(c[i]);
        i += 1;
    }
    if i != c.len() {
        return Err(vm.value_error("Invalid format specifier"));
    }
    if sp.zero && sp.align.is_none() {
        sp.fill = '0';
        sp.align = Some('=');
    }
    Ok(sp)
}

fn pad(sp: &Spec, sign: &str, body: &str, default_align: char) -> String {
    let len = sign.chars().count() + body.chars().count();
    let w = sp.width.unwrap_or(0);
    if len >= w {
        return format!("{sign}{body}");
    }
    let n = w - len;
    let fill = |k: usize| sp.fill.to_string().repeat(k);
    match sp.align.unwrap_or(default_align) {
        '<' => format!("{sign}{body}{}", fill(n)),
        '>' => format!("{}{sign}{body}", fill(n)),
        '^' => format!("{}{sign}{body}{}", fill(n / 2), fill(n - n / 2)),
        _ => format!("{sign}{}{body}", fill(n)),
    }
}

fn group(digits: &str, sep: char, every: usize) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % every == 0 {
            out.push(sep);
        }
        out.push(*c);
    }
    out
}

pub fn format_value(vm: &mut Vm, v: &Value, spec: &str) -> PyResult<String> {
    if spec.is_empty() {
        return crate::ops::str(vm, v);
    }
    let sp = parse_spec(vm, spec)?;
    match (num::num_of(v), v) {
        (Some(n), _) if !matches!(v.payload(), Some(Payload::Str(_))) => format_number(vm, n, &sp, matches!(v, Value::Bool(_))),
        _ => {
            if !matches!(sp.ty, None | Some('s')) {
                return Err(vm.value_error(format!("Unknown format code '{}' for object of type '{}'", sp.ty.unwrap(), crate::ops::type_name(vm, v))));
            }
            let mut s = crate::ops::str(vm, v)?;
            if let Some(p) = sp.precision {
                s = s.chars().take(p).collect();
            }
            Ok(pad(&sp, "", &s, '<'))
        }
    }
}

fn format_number(vm: &mut Vm, n: Num, sp: &Spec, is_bool: bool) -> PyResult<String> {
    let ty = sp.ty;
    let int_types = [Some('b'), Some('c'), Some('d'), Some('o'), Some('x'), Some('X'), Some('n')];
    let as_int = match &n {
        Num::I(i) => Some(BigInt::from(*i)),
        Num::B(b) => Some(b.clone()),
        _ => None,
    };
    if let Some(i) = as_int.as_ref().filter(|_| int_types.contains(&ty) || (ty.is_none() && sp.precision.is_none())) {
        if is_bool && ty.is_none() {
            let s = if i.is_zero() { "False" } else { "True" };
            return Ok(pad(sp, "", s, '<'));
        }
        let neg = i.is_negative();
        let mag = i.abs();
        let (body, prefix) = match ty {
            Some('b') => (mag.to_str_radix(2), "0b"),
            Some('o') => (mag.to_str_radix(8), "0o"),
            Some('x') => (mag.to_str_radix(16), "0x"),
            Some('X') => (mag.to_str_radix(16).to_uppercase(), "0X"),
            Some('c') => {
                let c = char::from_u32(i.to_u32().unwrap_or(0xfffd)).unwrap_or('\u{fffd}');
                return Ok(pad(sp, "", &c.to_string(), '<'));
            }
            _ => (mag.to_string(), ""),
        };
        let body = match sp.grouping {
            Some(g) => group(&body, g, if matches!(ty, Some('b' | 'o' | 'x' | 'X')) { 4 } else { 3 }),
            None => body,
        };
        let sign = sign_str(neg, sp.sign);
        let sign = if sp.alt { format!("{sign}{prefix}") } else { sign.to_string() };
        return Ok(pad(sp, &sign, &body, '>'));
    }
    let f = match n {
        Num::I(i) => i as f64,
        Num::B(b) => b.to_f64().unwrap_or(f64::INFINITY),
        Num::F(f) => f,
        Num::C(r, i) => {
            let s = complex_repr(r, i);
            return Ok(pad(sp, "", &s, '>'));
        }
    };
    let neg = f.is_sign_negative() && !(f == 0.0 && false);
    let a = f.abs();
    let body = match ty {
        Some('f') | Some('F') => {
            let p = sp.precision.unwrap_or(6);
            let s = if a.is_finite() { format!("{a:.p$}") } else { nonfinite(a, ty == Some('F')) };
            if sp.alt && p == 0 { s + "." } else { s }
        }
        Some('e') | Some('E') => {
            let p = sp.precision.unwrap_or(6);
            let s = if a.is_finite() { format_e(a, p, sp.alt) } else { nonfinite(a, false) };
            if ty == Some('E') { s.to_uppercase() } else { s }
        }
        Some('g') | Some('G') | Some('n') => {
            let p = sp.precision.unwrap_or(6);
            let s = if a.is_finite() { format_g(a, p, sp.alt, false) } else { nonfinite(a, false) };
            if ty == Some('G') { s.to_uppercase() } else { s }
        }
        Some('%') => {
            let p = sp.precision.unwrap_or(6);
            let x = a * 100.0;
            if x.is_finite() { format!("{x:.p$}%") } else { nonfinite(x, false) + "%" }
        }
        None => {
            if !a.is_finite() {
                nonfinite(a, false)
            } else {
                match sp.precision {
                    Some(p) => {
                        let s = format_g(a, p, sp.alt, true);
                        s
                    }
                    None => float_repr(a),
                }
            }
        }
        Some(c) => return Err(vm.value_error(format!("Unknown format code '{c}' for object of type 'float'"))),
    };
    let body = match sp.grouping {
        Some(g) => {
            let (int_part, rest) = match body.find(|c: char| c == '.' || c == 'e' || c == '%') {
                Some(p) => (&body[..p], &body[p..]),
                None => (&body[..], ""),
            };
            format!("{}{}", group(int_part, g, 3), rest)
        }
        None => body,
    };
    let sign = sign_str(neg && !(f == 0.0 && !f.is_sign_negative()), sp.sign);
    Ok(pad(sp, sign, &body, '>'))
}

fn nonfinite(a: f64, upper: bool) -> String {
    let s = if a.is_nan() { "nan" } else { "inf" };
    if upper { s.to_uppercase() } else { s.to_string() }
}

fn sign_str(neg: bool, sign: Option<char>) -> &'static str {
    if neg {
        "-"
    } else {
        match sign {
            Some('+') => "+",
            Some(' ') => " ",
            _ => "",
        }
    }
}

pub fn format_e(a: f64, p: usize, alt: bool) -> String {
    let s = format!("{a:.p$e}");
    let (m, e) = s.split_once('e').unwrap();
    let e: i32 = e.parse().unwrap();
    let m = if alt && p == 0 { format!("{m}.") } else { m.to_string() };
    format!("{}e{}{:02}", m, if e < 0 { '-' } else { '+' }, e.abs())
}

/// `%g` with precision `p`. `repr_style` is the general format used when a
/// float spec has a precision but no type (at least one digit after '.').
pub fn format_g(a: f64, p: usize, alt: bool, repr_style: bool) -> String {
    let p = if p == 0 { 1 } else { p };
    if a == 0.0 {
        return if alt { format!("0.{}", "0".repeat(p - 1)) } else if repr_style { "0.0".into() } else { "0".into() };
    }
    let e = format!("{:.*e}", p - 1, a);
    let (_, ex) = e.split_once('e').unwrap();
    let x: i32 = ex.parse().unwrap();
    let mut s = if x < -4 || x >= p as i32 {
        let m = format_e(a, p - 1, alt);
        if alt {
            m
        } else {
            let (mm, ee) = m.split_once('e').unwrap();
            let mm = if mm.contains('.') { mm.trim_end_matches('0').trim_end_matches('.') } else { mm };
            format!("{mm}e{ee}")
        }
    } else {
        let prec = (p as i32 - 1 - x).max(0) as usize;
        let f = format!("{a:.prec$}");
        if alt || !f.contains('.') { f } else { f.trim_end_matches('0').trim_end_matches('.').to_string() }
    };
    if repr_style && !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("nan") {
        s.push_str(".0");
    }
    s
}

// ---- printf-style formatting ----

pub fn percent_format(vm: &mut Vm, fmt: &str, args: &Value) -> PyResult<String> {
    let chars: Vec<char> = fmt.chars().collect();
    // PyUnicode_Format: any non-tuple, non-str operand with __getitem__
    // counts as a mapping (so `"abc" % []` is "abc").
    let is_mapping = matches!(args.payload(), Some(Payload::Dict(_))) || (!matches!(args.payload(), Some(Payload::Tuple(_)) | Some(Payload::Str(_))) && crate::ops::lookup_special(vm, args, "__getitem__").is_some());
    let items: Vec<Value> = match args.payload() {
        Some(Payload::Tuple(t)) => t.to_vec(),
        _ => vec![args.clone()],
    };
    let mut ai = 0usize;
    let mut used_mapping = false;
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut i = 0;
    let next_arg = |vm: &Vm, ai: &mut usize| -> PyResult<Value> {
        let v = items.get(*ai).cloned().ok_or_else(|| vm.type_error("not enough arguments for format string"))?;
        *ai += 1;
        Ok(v)
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            return Err(vm.value_error("incomplete format"));
        }
        let mut key: Option<String> = None;
        if chars[i] == '(' {
            let mut depth = 1;
            let st = i + 1;
            i += 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                }
                i += 1;
            }
            key = Some(chars[st..i - 1].iter().collect());
        }
        let mut flags = String::new();
        while i < chars.len() && "-+ #0".contains(chars[i]) {
            flags.push(chars[i]);
            i += 1;
        }
        let mut width: Option<usize> = None;
        if i < chars.len() && chars[i] == '*' {
            let w = next_arg(vm, &mut ai)?;
            let wi = crate::ops::index_of(vm, &w)?;
            if wi < 0 {
                flags.push('-');
            }
            width = Some(wi.unsigned_abs() as usize);
            i += 1;
        } else {
            let st = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i > st {
                width = chars[st..i].iter().collect::<String>().parse().ok();
            }
        }
        let mut prec: Option<usize> = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            if i < chars.len() && chars[i] == '*' {
                let p = next_arg(vm, &mut ai)?;
                prec = Some(crate::ops::index_of(vm, &p)?.max(0) as usize);
                i += 1;
            } else {
                let st = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                prec = Some(chars[st..i].iter().collect::<String>().parse().unwrap_or(0));
            }
        }
        while i < chars.len() && "hlL".contains(chars[i]) {
            i += 1;
        }
        if i >= chars.len() {
            return Err(vm.value_error("incomplete format"));
        }
        let ty = chars[i];
        i += 1;
        if ty == '%' {
            out.push('%');
            continue;
        }
        let arg = match &key {
            Some(k) => {
                if !is_mapping {
                    return Err(vm.type_error("format requires a mapping"));
                }
                used_mapping = true;
                let kv = vm.types.new_str(k);
                crate::ops::getitem(vm, args, kv)?
            }
            None => next_arg(vm, &mut ai)?,
        };
        let left = flags.contains('-');
        let zero = flags.contains('0') && !left;
        let sign = if flags.contains('+') { Some('+') } else if flags.contains(' ') { Some(' ') } else { None };
        let alt = flags.contains('#');
        let body: String;
        let mut numeric = false;
        let mut sgn = String::new();
        match ty {
            's' | 'r' | 'a' => {
                let s = match ty {
                    's' => crate::ops::str(vm, &arg)?,
                    'r' => crate::ops::repr(vm, &arg)?,
                    _ => crate::ops::ascii(vm, &arg)?,
                };
                body = match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                };
            }
            'c' => {
                body = match &arg {
                    Value::Int(n) => char::from_u32(*n as u32).map(|c| c.to_string()).ok_or_else(|| vm.exc_msg("OverflowError", "%c arg not in range(0x110000)"))?,
                    _ => match arg.as_str() {
                        Some(s) if s.chars().count() == 1 => s.to_string(),
                        _ => return Err(vm.type_error("%c requires int or char")),
                    },
                };
            }
            'd' | 'i' | 'u' | 'o' | 'x' | 'X' => {
                numeric = true;
                let n = match num::num_of(&arg) {
                    Some(Num::I(i)) => BigInt::from(i),
                    Some(Num::B(b)) => b,
                    Some(Num::F(f)) if matches!(ty, 'd' | 'i' | 'u') => num_traits::FromPrimitive::from_f64(f.trunc()).unwrap_or_default(),
                    _ => {
                        if let Some(r) = crate::ops::call_special(vm, &arg, "__int__", Vec::new())? {
                            crate::ops::to_bigint(&r).unwrap_or_default()
                        } else {
                            return Err(vm.type_error(format!("%{ty} format: a number is required, not {}", crate::ops::type_name(vm, &arg))));
                        }
                    }
                };
                sgn = sign_str(n.is_negative(), sign).to_string();
                let m = n.abs();
                let mut digits = match ty {
                    'o' => m.to_str_radix(8),
                    'x' => m.to_str_radix(16),
                    'X' => m.to_str_radix(16).to_uppercase(),
                    _ => m.to_string(),
                };
                if let Some(p) = prec {
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                if alt {
                    let pre = match ty {
                        'o' => "0o",
                        'x' => "0x",
                        'X' => "0X",
                        _ => "",
                    };
                    sgn.push_str(pre);
                }
                body = digits;
            }
            'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
                numeric = true;
                let f = crate::ops::to_f64(vm, &arg)?;
                sgn = sign_str(f.is_sign_negative() && !(f == 0.0 && !f.is_sign_negative()) && !f.is_nan(), sign).to_string();
                let a = f.abs();
                let p = prec.unwrap_or(6);
                let s = if !a.is_finite() {
                    nonfinite(a, ty.is_uppercase())
                } else {
                    match ty {
                        'e' | 'E' => format_e(a, p, alt),
                        'f' | 'F' => format!("{a:.p$}"),
                        _ => format_g(a, p, alt, false),
                    }
                };
                body = if ty.is_uppercase() { s.to_uppercase() } else { s };
            }
            other => {
                return Err(vm.value_error(format!("unsupported format character '{}' (0x{:x}) at index {}", other, other as u32, i - 1)));
            }
        }
        let len = sgn.chars().count() + body.chars().count();
        let w = width.unwrap_or(0);
        if len >= w {
            out.push_str(&sgn);
            out.push_str(&body);
        } else if left {
            out.push_str(&sgn);
            out.push_str(&body);
            out.push_str(&" ".repeat(w - len));
        } else if zero && numeric {
            out.push_str(&sgn);
            out.push_str(&"0".repeat(w - len));
            out.push_str(&body);
        } else {
            out.push_str(&" ".repeat(w - len));
            out.push_str(&sgn);
            out.push_str(&body);
        }
    }
    if !used_mapping && ai < items.len() && !is_mapping {
        return Err(vm.type_error("not all arguments converted during string formatting"));
    }
    Ok(out)
}

pub fn str_of_float(vm: &Vm, f: f64) -> String {
    if crate::ops::vm_dialect(vm) == Dialect::Py2 { float_str_py2(f) } else { float_repr(f) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floats() {
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(123.0), "123.0");
        assert_eq!(float_repr(1.5e-5), "1.5e-05");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(1234567890123456.0), "1234567890123456.0");
        assert_eq!(format_g(0.0001234, 6, false, false), "0.0001234");
        assert_eq!(format_g(1234567.0, 6, false, false), "1.23457e+06");
        assert_eq!(format_g(100.0, 6, false, false), "100");
    }
}
