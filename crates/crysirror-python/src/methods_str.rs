// SPDX-License-Identifier: MPL-2.0
//! `str`, `bytes` and `bytearray` methods, codecs and `str.format`.

use crate::builtins::{arg_count, parse, str_arg, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::cell::RefCell;
use std::rc::Rc;

fn m(vm: &Vm, t: &ObjRef, name: &'static str, f: NativeFn) {
    vm.types.add_method(t, name, f);
}

fn s_of<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a str> {
    v.as_str().ok_or_else(|| vm.type_error(format!("descriptor requires a 'str' object but received a '{}'", ops::type_name(vm, v))))
}

fn ret(vm: &Vm, s: String) -> PyResult<Value> {
    Ok(vm.types.new_string(s))
}

pub fn install(vm: &mut Vm) {
    let t = vm.types.str_.clone();
    let sm = vm.types.new_builtin("__new__", str_new);
    let sm = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(sm)));
    vm.types.set_attr(&t, "__new__", sm);
    for (n, f) in [
        ("capitalize", s_capitalize as NativeFn),
        ("casefold", s_lower),
        ("center", s_center),
        ("count", s_count),
        ("encode", s_encode),
        ("decode", s_decode_py2),
        ("endswith", s_endswith),
        ("expandtabs", s_expandtabs),
        ("find", s_find),
        ("format", s_format),
        ("format_map", s_format_map),
        ("index", s_index),
        ("isalnum", s_isalnum),
        ("isalpha", s_isalpha),
        ("isascii", s_isascii),
        ("isdecimal", s_isdigit),
        ("isdigit", s_isdigit),
        ("isidentifier", s_isidentifier),
        ("islower", s_islower),
        ("isnumeric", s_isnumeric),
        ("isprintable", s_isprintable),
        ("isspace", s_isspace),
        ("istitle", s_istitle),
        ("isupper", s_isupper),
        ("join", s_join),
        ("ljust", s_ljust),
        ("lower", s_lower),
        ("lstrip", s_lstrip),
        ("partition", s_partition),
        ("removeprefix", s_removeprefix),
        ("removesuffix", s_removesuffix),
        ("replace", s_replace),
        ("rfind", s_rfind),
        ("rindex", s_rindex),
        ("rjust", s_rjust),
        ("rpartition", s_rpartition),
        ("rsplit", s_rsplit),
        ("rstrip", s_rstrip),
        ("split", s_split),
        ("splitlines", s_splitlines),
        ("startswith", s_startswith),
        ("strip", s_strip),
        ("swapcase", s_swapcase),
        ("title", s_title),
        ("translate", s_translate),
        ("upper", s_upper),
        ("zfill", s_zfill),
        ("__getnewargs__", s_getnewargs),
        ("__mod__", s_mod),
        ("__rmod__", s_rmod),
    ] {
        m(vm, &t, n, f);
    }
    let mk = vm.types.new_builtin("maketrans", s_maketrans);
    let mk = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(mk)));
    vm.types.set_attr(&t, "maketrans", mk);
    copy_seq_dunders(vm, &t);

    let t = vm.types.bytes.clone();
    let bn = vm.types.new_builtin("__new__", bytes_new);
    let bn = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(bn)));
    vm.types.set_attr(&t, "__new__", bn);
    copy_seq_dunders(vm, &t);
    let t2 = vm.types.bytearray.clone();
    let bn = vm.types.new_builtin("__new__", bytearray_new);
    let bn = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(bn)));
    vm.types.set_attr(&t2, "__new__", bn);
    m(vm, &t2, "__init__", bytearray_init);
    m(vm, &t2, "append", ba_append);
    m(vm, &t2, "extend", ba_extend);
    m(vm, &t2, "clear", ba_clear);
    copy_seq_dunders(vm, &t2);
    let (bytes_t, bytearray_t) = (t.clone(), t2.clone());
    for t in [t, t2] {
        for (n, f) in [
            ("decode", b_decode as NativeFn),
            ("hex", b_hex),
            ("join", b_join),
            ("startswith", b_startswith),
            ("endswith", b_endswith),
            ("find", b_find),
            ("index", b_index),
            ("count", b_count),
            ("replace", b_replace),
            ("split", b_split),
            ("strip", b_strip),
            ("lstrip", b_lstrip),
            ("rstrip", b_rstrip),
            ("lower", b_lower),
            ("upper", b_upper),
            ("isdigit", b_isdigit),
            ("isalpha", b_isalpha),
            ("isspace", b_isspace),
        ] {
            m(vm, &t, n, f);
        }
        let fh = vm.types.new_builtin("fromhex", b_fromhex);
        let fh = Value::Obj(new_obj(vm.types.classmethod.clone(), Some(Dict::new()), Payload::ClassMethod(fh)));
        vm.types.set_attr(&t, "fromhex", fh);
    }
    crate::bytes_ext::install(vm, &bytes_t, &bytearray_t);
}

fn copy_seq_dunders(vm: &Vm, t: &ObjRef) {
    // Reuse the generic dunders installed on tuple.
    for n in ["__eq__", "__ne__", "__lt__", "__le__", "__gt__", "__ge__", "__hash__", "__repr__", "__str__", "__format__", "__getitem__", "__contains__", "__len__", "__iter__", "__add__", "__mul__", "__rmul__"] {
        if let Some(v) = vm.types.tuple.dict.as_ref().unwrap().borrow().get_str(n).cloned() {
            vm.types.set_attr(t, n, v);
        }
    }
}

fn str_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let [obj, encoding, errors] = parse(vm, "str", &a[1..], k, ["object", "encoding", "errors"], 0)?;
    let s = match obj {
        None => String::new(),
        Some(o) => {
            if encoding.is_some() || errors.is_some() {
                let b = ops::bytes_of(&o).ok_or_else(|| vm.type_error("decoding str is not supported"))?;
                let enc = match &encoding {
                    Some(e) => str_arg(vm, e, "encoding")?,
                    None => "utf-8".into(),
                };
                let err = match &errors {
                    Some(e) => str_arg(vm, e, "errors")?,
                    None => "strict".into(),
                };
                decode(vm, &b, &enc, &err)?
            } else {
                ops::str(vm, &o)?
            }
        }
    };
    if Rc::ptr_eq(&cls, &vm.types.str_) {
        return ret(vm, s);
    }
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::Str(PyStr::new(s)))))
}

fn s_getnewargs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_string();
    let v = vm.types.new_string(s);
    Ok(vm.types.new_tuple(vec![v]))
}

fn s_mod(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = s_of(vm, &a[0])?.to_string();
    let s = crate::format::percent_format(vm, &f, &a[1])?;
    ret(vm, s)
}

fn s_rmod(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.not_implemented.clone())
}

// ---- codecs ----

pub fn normalize_encoding(e: &str) -> String {
    e.to_ascii_lowercase().replace(['-', '_', ' '], "")
}

pub fn encode(vm: &Vm, s: &str, enc: &str, errors: &str) -> PyResult<Vec<u8>> {
    match normalize_encoding(enc).as_str() {
        "utf8" | "u8" | "utf" | "cp65001" => Ok(s.as_bytes().to_vec()),
        "utf8sig" => {
            let mut v = vec![0xef, 0xbb, 0xbf];
            v.extend_from_slice(s.as_bytes());
            Ok(v)
        }
        "ascii" | "usascii" | "latin1" | "iso88591" | "l1" | "cp1252" | "windows1252" => {
            let limit = if normalize_encoding(enc).contains("ascii") { 0x80 } else { 0x100 };
            let mut out = Vec::with_capacity(s.len());
            for c in s.chars() {
                let n = c as u32;
                if n < limit {
                    out.push(n as u8);
                } else {
                    match errors {
                        "ignore" => {}
                        "replace" => out.push(b'?'),
                        "xmlcharrefreplace" => out.extend(format!("&#{n};").bytes()),
                        "backslashreplace" => out.extend(format!("\\u{n:04x}").bytes()),
                        _ => return Err(vm.exc_msg("UnicodeEncodeError", format!("'{enc}' codec can't encode character '\\u{n:04x}'"))),
                    }
                }
            }
            Ok(out)
        }
        "utf16" | "utf16le" | "utf16be" => {
            let mut out = Vec::new();
            let n = normalize_encoding(enc);
            let be = n == "utf16be";
            if n == "utf16" {
                out.extend([0xff, 0xfe]);
            }
            for u in s.encode_utf16() {
                if be {
                    out.extend(u.to_be_bytes());
                } else {
                    out.extend(u.to_le_bytes());
                }
            }
            Ok(out)
        }
        "utf32" | "utf32le" => {
            let mut out = Vec::new();
            if normalize_encoding(enc) == "utf32" {
                out.extend([0xff, 0xfe, 0, 0]);
            }
            for c in s.chars() {
                out.extend((c as u32).to_le_bytes());
            }
            Ok(out)
        }
        "unicodeescape" => Ok(crate::format::quote_str(s).trim_matches('\'').as_bytes().to_vec()),
        "rawunicodeescape" => Ok(s.as_bytes().to_vec()),
        "shiftjis" | "sjis" | "cp932" | "ms932" | "shiftjis2004" | "gbk" | "gb2312" | "gb18030" | "cp936" | "big5" | "eucjp" | "euckr" | "cp949" => {
            Err(vm.exc_msg("LookupError", format!("codec '{enc}' encoding is provided by the host")))
        }
        _ => Err(vm.exc_msg("LookupError", format!("unknown encoding: {enc}"))),
    }
}

pub fn decode(vm: &Vm, b: &[u8], enc: &str, errors: &str) -> PyResult<String> {
    match normalize_encoding(enc).as_str() {
        "utf8" | "u8" | "utf" | "cp65001" | "utf8sig" => {
            let b = if normalize_encoding(enc) == "utf8sig" { b.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(b) } else { b };
            match std::str::from_utf8(b) {
                Ok(s) => Ok(s.to_string()),
                Err(e) => match errors {
                    "ignore" => Ok(String::from_utf8_lossy(b).replace('\u{fffd}', "")),
                    "replace" => Ok(String::from_utf8_lossy(b).to_string()),
                    "surrogateescape" => Ok(String::from_utf8_lossy(b).to_string()),
                    _ => Err(vm.exc_msg("UnicodeDecodeError", format!("'utf-8' codec can't decode byte 0x{:02x} in position {}: invalid start byte", b[e.valid_up_to()], e.valid_up_to()))),
                },
            }
        }
        "ascii" | "usascii" => {
            let mut out = String::with_capacity(b.len());
            for (i, &c) in b.iter().enumerate() {
                if c < 0x80 {
                    out.push(c as char);
                } else {
                    match errors {
                        "ignore" => {}
                        "replace" => out.push('\u{fffd}'),
                        _ => return Err(vm.exc_msg("UnicodeDecodeError", format!("'ascii' codec can't decode byte 0x{c:02x} in position {i}: ordinal not in range(128)"))),
                    }
                }
            }
            Ok(out)
        }
        "latin1" | "iso88591" | "l1" => Ok(b.iter().map(|c| *c as char).collect()),
        "cp1252" | "windows1252" => Ok(b.iter().map(|c| cp1252(*c)).collect()),
        "utf16" | "utf16le" | "utf16be" => {
            let n = normalize_encoding(enc);
            let (mut be, mut data) = (n == "utf16be", b);
            if n == "utf16" {
                if data.starts_with(&[0xfe, 0xff]) {
                    be = true;
                    data = &data[2..];
                } else if data.starts_with(&[0xff, 0xfe]) {
                    data = &data[2..];
                }
            }
            let units: Vec<u16> = data.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
            Ok(String::from_utf16_lossy(&units))
        }
        "unicodeescape" => {
            let s: String = b.iter().map(|c| *c as char).collect();
            crate::parser::decode_str(&s, false, false).map_err(|e| vm.exc_msg("UnicodeDecodeError", e))
        }
        _ => Err(vm.exc_msg("LookupError", format!("unknown encoding: {enc}"))),
    }
}

fn cp1252(c: u8) -> char {
    const HI: [u16; 32] = [
        0x20ac, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160, 0x2039, 0x152, 0x8d, 0x17d, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x17e, 0x178,
    ];
    if (0x80..0xa0).contains(&c) {
        char::from_u32(HI[(c - 0x80) as usize] as u32).unwrap()
    } else {
        c as char
    }
}

fn s_encode(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [enc, err] = parse(vm, "encode", &a[1..], k, ["encoding", "errors"], 0)?;
    let s = s_of(vm, &a[0])?.to_string();
    let enc = match enc {
        Some(e) => str_arg(vm, &e, "encoding")?,
        None => "utf-8".into(),
    };
    let err = match err {
        Some(e) => str_arg(vm, &e, "errors")?,
        None => "strict".into(),
    };
    let b = encode(vm, &s, &enc, &err)?;
    Ok(vm.types.new_bytes(b))
}

/// Python 2 `str.decode`: one text type, so decoding text returns itself.
fn s_decode_py2(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_string();
    ret(vm, s)
}

fn b_decode(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [enc, err] = parse(vm, "decode", &a[1..], k, ["encoding", "errors"], 0)?;
    let b = ops::bytes_of(&a[0]).unwrap_or_default();
    let enc = match enc {
        Some(e) => str_arg(vm, &e, "encoding")?,
        None => "utf-8".into(),
    };
    let err = match err {
        Some(e) => str_arg(vm, &e, "errors")?,
        None => "strict".into(),
    };
    let s = decode(vm, &b, &enc, &err)?;
    ret(vm, s)
}

// ---- str methods ----

fn s_capitalize(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    let mut c = s.chars();
    let r = match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    };
    ret(vm, r)
}

fn s_lower(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_lowercase();
    ret(vm, s)
}

fn s_upper(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_uppercase();
    ret(vm, s)
}

fn s_swapcase(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s: String = s_of(vm, &a[0])?.chars().flat_map(|c| if c.is_uppercase() { c.to_lowercase().collect::<Vec<_>>() } else { c.to_uppercase().collect() }).collect();
    ret(vm, s)
}

fn s_title(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut out = String::new();
    let mut prev = false;
    for c in s_of(vm, &a[0])?.chars() {
        if c.is_alphabetic() {
            if prev {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev = true;
        } else {
            out.push(c);
            prev = false;
        }
    }
    ret(vm, out)
}

fn fill_char(vm: &Vm, v: Option<&Value>) -> PyResult<char> {
    match v {
        None => Ok(' '),
        Some(f) => {
            let s = str_arg(vm, f, "fillchar")?;
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => Ok(c),
                _ => Err(vm.type_error("The fill character must be exactly one character long")),
            }
        }
    }
}

fn justify(vm: &mut Vm, a: &[Value], mode: u8) -> PyResult<Value> {
    arg_count(vm, "justify", a, 2, 3)?;
    let s = s_of(vm, &a[0])?.to_string();
    let w = ops::index_of(vm, &a[1])?.max(0) as usize;
    let f = fill_char(vm, a.get(2))?;
    let n = s.chars().count();
    if n >= w {
        return ret(vm, s);
    }
    let pad = w - n;
    let fs = |k: usize| f.to_string().repeat(k);
    let r = match mode {
        0 => s + &fs(pad),
        1 => fs(pad) + &s,
        _ => {
            let left = pad / 2 + (pad & w & 1);
            fs(left) + &s + &fs(pad - left)
        }
    };
    ret(vm, r)
}

fn s_ljust(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    justify(vm, a, 0)
}
fn s_rjust(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    justify(vm, a, 1)
}
fn s_center(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    justify(vm, a, 2)
}

fn s_zfill(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "zfill", a, 2, 2)?;
    let s = s_of(vm, &a[0])?.to_string();
    let w = ops::index_of(vm, &a[1])?.max(0) as usize;
    let n = s.chars().count();
    if n >= w {
        return ret(vm, s);
    }
    let (sign, body) = if s.starts_with(['+', '-']) { (&s[..1], &s[1..]) } else { ("", &s[..]) };
    ret(vm, format!("{sign}{}{body}", "0".repeat(w - n)))
}

/// Character-index window for find/count/startswith arguments.
fn window(vm: &mut Vm, s: &PyStr, start: Option<&Value>, end: Option<&Value>) -> PyResult<(usize, usize)> {
    let n = s.char_len() as i64;
    let norm = |i: i64| if i < 0 { (i + n).max(0) } else { i.min(n) };
    let a = match start {
        Some(v) if !v.is_none() => norm(ops::index_of(vm, v)?),
        _ => 0,
    };
    let b = match end {
        Some(v) if !v.is_none() => norm(ops::index_of(vm, v)?),
        _ => n,
    };
    Ok((a as usize, b as usize))
}

fn char_index_of_byte(s: &str, byte: usize) -> usize {
    if s.is_ascii() { byte } else { s[..byte].chars().count() }
}

fn find_impl(vm: &mut Vm, a: &[Value], rev: bool) -> PyResult<i64> {
    arg_count(vm, "find", a, 2, 4)?;
    let ps = a[0].as_pystr().ok_or_else(|| vm.type_error("str expected"))?;
    let sub = str_arg(vm, &a[1], "substring")?;
    let (st, en) = window(vm, ps, a.get(2), a.get(3))?;
    if st > en {
        return Ok(-1);
    }
    let hay = ps.slice_chars(st, en);
    let r = if rev { hay.rfind(&sub) } else { hay.find(&sub) };
    Ok(match r {
        Some(b) => (st + char_index_of_byte(hay, b)) as i64,
        None => -1,
    })
}

fn s_find(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(find_impl(vm, a, false)?))
}
fn s_rfind(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(find_impl(vm, a, true)?))
}
fn s_index(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match find_impl(vm, a, false)? {
        -1 => Err(vm.value_error("substring not found")),
        i => Ok(Value::Int(i)),
    }
}
fn s_rindex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match find_impl(vm, a, true)? {
        -1 => Err(vm.value_error("substring not found")),
        i => Ok(Value::Int(i)),
    }
}

fn s_count(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "count", a, 2, 4)?;
    let ps = a[0].as_pystr().ok_or_else(|| vm.type_error("str expected"))?;
    let sub = str_arg(vm, &a[1], "substring")?;
    let (st, en) = window(vm, ps, a.get(2), a.get(3))?;
    if st > en {
        return Ok(Value::Int(0));
    }
    let hay = ps.slice_chars(st, en);
    let n = if sub.is_empty() { hay.chars().count() + 1 } else { hay.matches(&sub).count() };
    Ok(Value::Int(n as i64))
}

fn affix_impl(vm: &mut Vm, a: &[Value], ends: bool) -> PyResult<Value> {
    arg_count(vm, "startswith", a, 2, 4)?;
    let ps = a[0].as_pystr().ok_or_else(|| vm.type_error("str expected"))?;
    let (st, en) = window(vm, ps, a.get(2), a.get(3))?;
    if st > en {
        return Ok(Value::Bool(false));
    }
    let hay = ps.slice_chars(st, en).to_string();
    let cands: Vec<Value> = match a[1].payload() {
        Some(Payload::Tuple(t)) => t.to_vec(),
        _ => vec![a[1].clone()],
    };
    for c in cands {
        let p = c.as_str().ok_or_else(|| vm.type_error("startswith first arg must be str or a tuple of str"))?;
        if if ends { hay.ends_with(p) } else { hay.starts_with(p) } {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn s_startswith(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    affix_impl(vm, a, false)
}
fn s_endswith(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    affix_impl(vm, a, true)
}

fn s_expandtabs(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [ts] = parse(vm, "expandtabs", &a[1..], k, ["tabsize"], 0)?;
    let ts = crate::builtins::opt_i64(vm, &ts, 8)?.max(0) as usize;
    let mut out = String::new();
    let mut col = 0;
    for c in s_of(vm, &a[0])?.chars() {
        match c {
            '\t' => {
                if ts > 0 {
                    let n = ts - col % ts;
                    out.push_str(&" ".repeat(n));
                    col += n;
                }
            }
            '\n' | '\r' => {
                out.push(c);
                col = 0;
            }
            _ => {
                out.push(c);
                col += 1;
            }
        }
    }
    ret(vm, out)
}

fn pred(vm: &mut Vm, a: &[Value], f: fn(char) -> bool) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    Ok(Value::Bool(!s.is_empty() && s.chars().all(f)))
}

fn s_isalnum(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    pred(vm, a, |c| c.is_alphanumeric())
}
fn s_isalpha(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    pred(vm, a, |c| c.is_alphabetic())
}
fn s_isascii(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    Ok(Value::Bool(s.is_ascii()))
}
fn s_isdigit(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    pred(vm, a, |c| c.is_ascii_digit() || (c.is_numeric() && c.to_digit(10).is_some()) || ('０'..='９').contains(&c))
}
fn s_isnumeric(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    pred(vm, a, |c| c.is_numeric())
}
fn s_isspace(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    pred(vm, a, |c| c.is_whitespace())
}
fn s_isprintable(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    Ok(Value::Bool(s.chars().all(|c| !c.is_control())))
}
fn s_isidentifier(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    let mut it = s.chars();
    Ok(Value::Bool(match it.next() {
        Some(c) if c == '_' || c.is_alphabetic() => it.all(|c| c == '_' || c.is_alphanumeric()),
        _ => false,
    }))
}
fn s_islower(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    Ok(Value::Bool(s.chars().any(|c| c.is_lowercase()) && !s.chars().any(|c| c.is_uppercase())))
}
fn s_isupper(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    Ok(Value::Bool(s.chars().any(|c| c.is_uppercase()) && !s.chars().any(|c| c.is_lowercase())))
}
fn s_istitle(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?;
    let mut prev = false;
    let mut any = false;
    for c in s.chars() {
        if c.is_uppercase() {
            if prev {
                return Ok(Value::Bool(false));
            }
            prev = true;
            any = true;
        } else if c.is_lowercase() {
            if !prev {
                return Ok(Value::Bool(false));
            }
            prev = true;
            any = true;
        } else {
            prev = false;
        }
    }
    Ok(Value::Bool(any))
}

fn s_join(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "join", a, 2, 2)?;
    let sep = s_of(vm, &a[0])?.to_string();
    let items = ops::collect(vm, &a[1])?;
    let mut out = String::new();
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        match it.as_str() {
            Some(s) => out.push_str(s),
            None => return Err(vm.type_error(format!("sequence item {i}: expected str instance, {} found", ops::type_name(vm, it)))),
        }
    }
    ret(vm, out)
}

fn strip_chars(vm: &Vm, a: &[Value]) -> PyResult<Option<Vec<char>>> {
    match a.get(1) {
        None => Ok(None),
        Some(v) if v.is_none() => Ok(None),
        Some(v) => Ok(Some(str_arg(vm, v, "chars")?.chars().collect())),
    }
}

fn strip_impl(vm: &mut Vm, a: &[Value], left: bool, right: bool) -> PyResult<Value> {
    arg_count(vm, "strip", a, 1, 2)?;
    let chars = strip_chars(vm, a)?;
    let s = s_of(vm, &a[0])?;
    let f = |c: char| match &chars {
        Some(cs) => cs.contains(&c),
        None => c.is_whitespace(),
    };
    let mut r = s;
    if left {
        r = r.trim_start_matches(f);
    }
    if right {
        r = r.trim_end_matches(f);
    }
    let r = r.to_string();
    ret(vm, r)
}

fn s_strip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    strip_impl(vm, a, true, true)
}
fn s_lstrip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    strip_impl(vm, a, true, false)
}
fn s_rstrip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    strip_impl(vm, a, false, true)
}

fn partition_impl(vm: &mut Vm, a: &[Value], rev: bool) -> PyResult<Value> {
    arg_count(vm, "partition", a, 2, 2)?;
    let s = s_of(vm, &a[0])?.to_string();
    let sep = str_arg(vm, &a[1], "sep")?;
    if sep.is_empty() {
        return Err(vm.value_error("empty separator"));
    }
    let found = if rev { s.rfind(&sep) } else { s.find(&sep) };
    let parts = match found {
        Some(i) => (s[..i].to_string(), sep.clone(), s[i + sep.len()..].to_string()),
        None => {
            if rev {
                (String::new(), String::new(), s)
            } else {
                (s, String::new(), String::new())
            }
        }
    };
    let t = vec![vm.types.new_string(parts.0), vm.types.new_string(parts.1), vm.types.new_string(parts.2)];
    Ok(vm.types.new_tuple(t))
}

fn s_partition(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    partition_impl(vm, a, false)
}
fn s_rpartition(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    partition_impl(vm, a, true)
}

fn s_removeprefix(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_string();
    let p = str_arg(vm, &a[1], "prefix")?;
    ret(vm, s.strip_prefix(&p).map(|x| x.to_string()).unwrap_or(s))
}
fn s_removesuffix(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = s_of(vm, &a[0])?.to_string();
    let p = str_arg(vm, &a[1], "suffix")?;
    ret(vm, s.strip_suffix(&p).map(|x| x.to_string()).unwrap_or(s))
}

fn s_replace(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "replace", a, 3, 4)?;
    let s = s_of(vm, &a[0])?.to_string();
    let old = str_arg(vm, &a[1], "old")?;
    let new = str_arg(vm, &a[2], "new")?;
    let count = crate::builtins::opt_i64(vm, &a.get(3).cloned(), -1)?;
    let r = if count < 0 {
        if old.is_empty() {
            let mut out = new.clone();
            for c in s.chars() {
                out.push(c);
                out.push_str(&new);
            }
            out
        } else {
            s.replace(&old, &new)
        }
    } else {
        s.replacen(&old, &new, count as usize)
    };
    ret(vm, r)
}

fn split_whitespace(s: &str, max: i64, rev: bool) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !rev {
        let mut rest = s.trim_start();
        while !rest.is_empty() {
            if max >= 0 && parts.len() as i64 == max {
                parts.push(rest.to_string());
                return parts;
            }
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            parts.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
        }
    } else {
        let mut rest = s.trim_end();
        while !rest.is_empty() {
            if max >= 0 && parts.len() as i64 == max {
                parts.push(rest.to_string());
                break;
            }
            let start = rest.rfind(char::is_whitespace).map(|i| i + rest[i..].chars().next().unwrap().len_utf8()).unwrap_or(0);
            parts.push(rest[start..].to_string());
            rest = rest[..start].trim_end();
        }
        parts.reverse();
    }
    parts
}

fn split_impl(vm: &mut Vm, a: &[Value], k: Kw, rev: bool) -> PyResult<Value> {
    let [sep, max] = parse(vm, "split", &a[1..], k, ["sep", "maxsplit"], 0)?;
    let s = s_of(vm, &a[0])?.to_string();
    let max = crate::builtins::opt_i64(vm, &max, -1)?;
    let parts: Vec<String> = match sep.filter(|v| !v.is_none()) {
        None => split_whitespace(&s, max, rev),
        Some(sep) => {
            let sep = str_arg(vm, &sep, "sep")?;
            if sep.is_empty() {
                return Err(vm.value_error("empty separator"));
            }
            if max < 0 {
                s.split(sep.as_str()).map(|x| x.to_string()).collect()
            } else if rev {
                let mut v: Vec<String> = s.rsplitn(max as usize + 1, sep.as_str()).map(|x| x.to_string()).collect();
                v.reverse();
                v
            } else {
                s.splitn(max as usize + 1, sep.as_str()).map(|x| x.to_string()).collect()
            }
        }
    };
    let items = parts.into_iter().map(|p| vm.types.new_string(p)).collect();
    Ok(vm.types.new_list(items))
}

fn s_split(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    split_impl(vm, a, k, false)
}
fn s_rsplit(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    split_impl(vm, a, k, true)
}

fn s_splitlines(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [keep] = parse(vm, "splitlines", &a[1..], k, ["keepends"], 0)?;
    let keep = match keep {
        Some(v) => ops::truthy(vm, &v)?,
        None => false,
    };
    let s = s_of(vm, &a[0])?.to_string();
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let is_break = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if is_break {
            let mut brk = c.to_string();
            if c == '\r' && chars.get(i + 1) == Some(&'\n') {
                brk.push('\n');
                i += 1;
            }
            if keep {
                cur.push_str(&brk);
            }
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    let items = out.into_iter().map(|p| vm.types.new_string(p)).collect();
    Ok(vm.types.new_list(items))
}

fn s_maketrans(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "maketrans", a, 1, 3)?;
    let d = vm.types.new_dict_obj(Dict::new());
    if a.len() == 1 {
        for (k, v) in ops::mapping_items(vm, &a[0])? {
            let key = match k.as_str() {
                Some(s) if s.chars().count() == 1 => Value::Int(s.chars().next().unwrap() as i64),
                _ => k,
            };
            ops::dict_set(vm, &d, key, v)?;
        }
        return Ok(Value::Obj(d));
    }
    let x: Vec<char> = str_arg(vm, &a[0], "x")?.chars().collect();
    let y: Vec<char> = str_arg(vm, &a[1], "y")?.chars().collect();
    if x.len() != y.len() {
        return Err(vm.value_error("the first two maketrans arguments must have equal length"));
    }
    for (c, r) in x.iter().zip(&y) {
        let mut b = [0u8; 4];
        let rv = vm.types.new_str(r.encode_utf8(&mut b));
        ops::dict_set(vm, &d, Value::Int(*c as i64), rv)?;
    }
    if let Some(z) = a.get(2) {
        for c in str_arg(vm, z, "z")?.chars() {
            ops::dict_set(vm, &d, Value::Int(c as i64), Value::None)?;
        }
    }
    Ok(Value::Obj(d))
}

fn s_translate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "translate", a, 2, 2)?;
    let s = s_of(vm, &a[0])?.to_string();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match ops::getitem(vm, &a[1], Value::Int(c as i64)) {
            Ok(Value::None) => {}
            Ok(Value::Int(n)) => out.push(char::from_u32(n as u32).unwrap_or('\u{fffd}')),
            Ok(v) => match v.as_str() {
                Some(r) => out.push_str(r),
                None => return Err(vm.type_error("character mapping must return integer, None or str")),
            },
            Err(e) if vm.is_exc(&e, "LookupError") => out.push(c),
            Err(e) => return Err(e),
        }
    }
    ret(vm, out)
}

// ---- str.format ----

fn s_format(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let fmt = s_of(vm, &a[0])?.to_string();
    let kwargs = vm.types.new_dict_obj(Dict::new());
    for (key, v) in k {
        let kv = vm.types.new_str(key);
        ops::dict_set(vm, &kwargs, kv, v.clone())?;
    }
    let s = format_str(vm, &fmt, &a[1..], &Value::Obj(kwargs), 2)?;
    ret(vm, s)
}

fn s_format_map(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "format_map", a, 2, 2)?;
    let fmt = s_of(vm, &a[0])?.to_string();
    let s = format_str(vm, &fmt, &[], &a[1], 2)?;
    ret(vm, s)
}

/// Resolves `field_name` (`0`, `name`, `0.attr`, `a[key]`).
pub fn resolve_field(vm: &mut Vm, field: &str, args: &[Value], kwargs: &Value, auto: &mut usize) -> PyResult<Value> {
    let first_end = field.find(['.', '[']).unwrap_or(field.len());
    let first = &field[..first_end];
    let mut v = if first.is_empty() {
        let i = *auto;
        *auto += 1;
        args.get(i).cloned().ok_or_else(|| vm.index_error(format!("Replacement index {i} out of range for positional args tuple")))?
    } else if let Ok(i) = first.parse::<usize>() {
        args.get(i).cloned().ok_or_else(|| vm.index_error(format!("Replacement index {i} out of range for positional args tuple")))?
    } else {
        let kv = vm.types.new_str(first);
        ops::getitem(vm, kwargs, kv)?
    };
    let mut rest = &field[first_end..];
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '[']).unwrap_or(r.len());
            v = ops::getattr(vm, &v, &r[..end])?;
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').ok_or_else(|| vm.value_error("Missing ']' in format string"))?;
            let key = &r[..end];
            let kv = match key.parse::<i64>() {
                Ok(i) => Value::Int(i),
                Err(_) => vm.types.new_str(key),
            };
            v = ops::getitem(vm, &v, kv)?;
            rest = &r[end + 1..];
        } else {
            return Err(vm.value_error("Only '.' or '[' may follow ']' in format field specifier"));
        }
    }
    Ok(v)
}

pub fn format_str(vm: &mut Vm, fmt: &str, args: &[Value], kwargs: &Value, depth: u32) -> PyResult<String> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut auto = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                out.push('{');
                i += 2;
                continue;
            }
            let mut d = 1;
            let st = i + 1;
            let mut j = st;
            while j < chars.len() && d > 0 {
                if chars[j] == '{' {
                    d += 1;
                } else if chars[j] == '}' {
                    d -= 1;
                }
                j += 1;
            }
            if d != 0 {
                return Err(vm.value_error("Single '{' encountered in format string"));
            }
            let field: String = chars[st..j - 1].iter().collect();
            i = j;
            // Split name!conv:spec, minding brackets in the name.
            let mut name_end = field.len();
            let mut br = 0;
            for (p, ch) in field.char_indices() {
                match ch {
                    '[' => br += 1,
                    ']' => br -= 1,
                    '!' | ':' if br == 0 => {
                        name_end = p;
                        break;
                    }
                    _ => {}
                }
            }
            let name = &field[..name_end];
            let rest = &field[name_end..];
            let (conv, spec) = if let Some(r) = rest.strip_prefix('!') {
                let mut it = r.chars();
                let cv = it.next();
                let after = it.as_str();
                (cv, after.strip_prefix(':').unwrap_or(""))
            } else {
                (None, rest.strip_prefix(':').unwrap_or(""))
            };
            let mut v = resolve_field(vm, name, args, kwargs, &mut auto)?;
            if let Some(cv) = conv {
                let s = match cv {
                    's' => ops::str(vm, &v)?,
                    'r' => ops::repr(vm, &v)?,
                    'a' => ops::ascii(vm, &v)?,
                    _ => return Err(vm.value_error(format!("Unknown conversion specifier {cv}"))),
                };
                v = vm.types.new_string(s);
            }
            let spec = if spec.contains('{') && depth > 0 { format_str(vm, spec, args, kwargs, depth - 1)? } else { spec.to_string() };
            out.push_str(&ops::format(vm, &v, &spec)?);
            continue;
        }
        if c == '}' {
            if chars.get(i + 1) == Some(&'}') {
                out.push('}');
                i += 2;
                continue;
            }
            return Err(vm.value_error("Single '}' encountered in format string"));
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

// ---- bytes ----

fn bytes_from(vm: &mut Vm, src: Option<&Value>, enc: Option<&Value>, err: Option<&Value>) -> PyResult<Vec<u8>> {
    let Some(src) = src else { return Ok(Vec::new()) };
    if let Some(s) = src.as_str() {
        let enc = match enc {
            Some(e) => str_arg(vm, e, "encoding")?,
            None => {
                if crate::ops::vm_dialect(vm) == crate::Dialect::Py2 {
                    "utf-8".to_string()
                } else {
                    return Err(vm.type_error("string argument without an encoding"));
                }
            }
        };
        let err = match err {
            Some(e) => str_arg(vm, e, "errors")?,
            None => "strict".into(),
        };
        return encode(vm, s, &enc, &err);
    }
    if let Some(b) = ops::bytes_of(src) {
        return Ok(b);
    }
    if let Value::Int(n) = src {
        if *n < 0 {
            return Err(vm.value_error("negative count"));
        }
        return Ok(vec![0; *n as usize]);
    }
    if let Some(r) = ops::call_special(vm, src, "__bytes__", Vec::new())? {
        return ops::bytes_of(&r).ok_or_else(|| vm.type_error("__bytes__ returned non-bytes"));
    }
    let mut out = Vec::new();
    for it in ops::collect(vm, src)? {
        let n = ops::index_of(vm, &it)?;
        if !(0..256).contains(&n) {
            return Err(vm.value_error("bytes must be in range(0, 256)"));
        }
        out.push(n as u8);
    }
    Ok(out)
}

fn bytes_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let [src, enc, err] = parse(vm, "bytes", &a[1..], k, ["source", "encoding", "errors"], 0)?;
    let b = bytes_from(vm, src.as_ref(), enc.as_ref(), err.as_ref())?;
    if Rc::ptr_eq(&cls, &vm.types.bytes) {
        return Ok(vm.types.new_bytes(b));
    }
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::Bytes(b.into_boxed_slice()))))
}

fn bytearray_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(new_obj(cls, None, Payload::ByteArray(RefCell::new(Vec::new())))))
}

fn bytearray_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [src, enc, err] = parse(vm, "bytearray", &a[1..], k, ["source", "encoding", "errors"], 0)?;
    let b = bytes_from(vm, src.as_ref(), enc.as_ref(), err.as_ref())?;
    if let Some(Payload::ByteArray(x)) = a[0].payload() {
        *x.borrow_mut() = b;
    }
    Ok(Value::None)
}

fn ba_append(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[1])?;
    if let Some(Payload::ByteArray(x)) = a[0].payload() {
        x.borrow_mut().push(n as u8);
    }
    Ok(Value::None)
}

fn ba_extend(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bytes_from(vm, Some(&a[1]), None, None)?;
    if let Some(Payload::ByteArray(x)) = a[0].payload() {
        x.borrow_mut().extend(b);
    }
    Ok(Value::None)
}

fn ba_clear(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(Payload::ByteArray(x)) = a[0].payload() {
        x.borrow_mut().clear();
    }
    Ok(Value::None)
}

fn same_bytes_type(vm: &Vm, model: &Value, b: Vec<u8>) -> Value {
    if matches!(model.payload(), Some(Payload::ByteArray(_))) {
        return Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(b))));
    }
    vm.types.new_bytes(b)
}

fn bb(vm: &Vm, v: &Value) -> PyResult<Vec<u8>> {
    ops::bytes_of(v).ok_or_else(|| vm.type_error(format!("a bytes-like object is required, not '{}'", ops::type_name(vm, v))))
}

fn b_hex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let s: String = b.iter().map(|x| format!("{x:02x}")).collect();
    ret(vm, s)
}

fn b_fromhex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = str_arg(vm, &a[1], "fromhex")?;
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() % 2 != 0 {
        return Err(vm.value_error("non-hexadecimal number found in fromhex() arg"));
    }
    let mut out = Vec::new();
    for i in (0..clean.len()).step_by(2) {
        out.push(u8::from_str_radix(&clean[i..i + 2], 16).map_err(|_| vm.value_error("non-hexadecimal number found in fromhex() arg"))?);
    }
    let cls = a[0].clone();
    if cls.obj().map(|c| Rc::ptr_eq(c, &vm.types.bytearray)).unwrap_or(false) {
        return Ok(Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(out)))));
    }
    Ok(vm.types.new_bytes(out))
}

fn b_join(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let sep = bb(vm, &a[0])?;
    let mut out = Vec::new();
    for (i, it) in ops::collect(vm, &a[1])?.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&sep);
        }
        out.extend(bb(vm, it)?);
    }
    Ok(same_bytes_type(vm, &a[0], out))
}

fn b_affix(vm: &mut Vm, a: &[Value], ends: bool) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let cands: Vec<Value> = match a[1].payload() {
        Some(Payload::Tuple(t)) => t.to_vec(),
        _ => vec![a[1].clone()],
    };
    for c in cands {
        let p = bb(vm, &c)?;
        if if ends { b.ends_with(&p) } else { b.starts_with(&p) } {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}
fn b_startswith(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    b_affix(vm, a, false)
}
fn b_endswith(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    b_affix(vm, a, true)
}

fn find_bytes(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    h.windows(n.len()).position(|w| w == n)
}

fn b_find(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let n = match &a[1] {
        Value::Int(i) => vec![*i as u8],
        v => bb(vm, v)?,
    };
    Ok(Value::Int(find_bytes(&b, &n).map(|i| i as i64).unwrap_or(-1)))
}

fn b_index(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    match b_find(vm, a, k)? {
        Value::Int(-1) => Err(vm.value_error("subsection not found")),
        v => Ok(v),
    }
}

fn b_count(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let n = bb(vm, &a[1])?;
    if n.is_empty() {
        return Ok(Value::Int(b.len() as i64 + 1));
    }
    let mut c = 0;
    let mut i = 0;
    while i + n.len() <= b.len() {
        if b[i..i + n.len()] == n[..] {
            c += 1;
            i += n.len();
        } else {
            i += 1;
        }
    }
    Ok(Value::Int(c))
}

fn b_replace(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let old = bb(vm, &a[1])?;
    let new = bb(vm, &a[2])?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !old.is_empty() && b[i..].starts_with(&old) {
            out.extend_from_slice(&new);
            i += old.len();
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(same_bytes_type(vm, &a[0], out))
}

fn b_split(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let parts: Vec<Vec<u8>> = match a.get(1).filter(|v| !v.is_none()) {
        None => b.split(|c| c.is_ascii_whitespace()).filter(|p| !p.is_empty()).map(|p| p.to_vec()).collect(),
        Some(sep) => {
            let sep = bb(vm, sep)?;
            let mut out = Vec::new();
            let mut start = 0;
            let mut i = 0;
            while i + sep.len() <= b.len() && !sep.is_empty() {
                if b[i..i + sep.len()] == sep[..] {
                    out.push(b[start..i].to_vec());
                    i += sep.len();
                    start = i;
                } else {
                    i += 1;
                }
            }
            out.push(b[start..].to_vec());
            out
        }
    };
    let items = parts.into_iter().map(|p| same_bytes_type(vm, &a[0], p)).collect();
    Ok(vm.types.new_list(items))
}

fn b_strip_impl(vm: &mut Vm, a: &[Value], l: bool, r: bool) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    let chars = match a.get(1).filter(|v| !v.is_none()) {
        Some(c) => bb(vm, c)?,
        None => b" \t\n\r\x0b\x0c".to_vec(),
    };
    let mut s = 0;
    let mut e = b.len();
    if l {
        while s < e && chars.contains(&b[s]) {
            s += 1;
        }
    }
    if r {
        while e > s && chars.contains(&b[e - 1]) {
            e -= 1;
        }
    }
    Ok(same_bytes_type(vm, &a[0], b[s..e].to_vec()))
}
fn b_strip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    b_strip_impl(vm, a, true, true)
}
fn b_lstrip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    b_strip_impl(vm, a, true, false)
}
fn b_rstrip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    b_strip_impl(vm, a, false, true)
}
fn b_lower(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?.to_ascii_lowercase();
    Ok(same_bytes_type(vm, &a[0], b))
}
fn b_upper(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?.to_ascii_uppercase();
    Ok(same_bytes_type(vm, &a[0], b))
}
fn b_isdigit(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    Ok(Value::Bool(!b.is_empty() && b.iter().all(|c| c.is_ascii_digit())))
}
fn b_isalpha(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    Ok(Value::Bool(!b.is_empty() && b.iter().all(|c| c.is_ascii_alphabetic())))
}
fn b_isspace(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = bb(vm, &a[0])?;
    Ok(Value::Bool(!b.is_empty() && b.iter().all(|c| c.is_ascii_whitespace())))
}
