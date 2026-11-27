// SPDX-License-Identifier: MPL-2.0
//! `_json`: the encoder/decoder used by the `json` Python module
//! (`dumps`/`loads` keyword options follow CPython).

use super::module_with;
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

struct Enc {
    indent: Option<String>,
    item_sep: String,
    key_sep: String,
    sort_keys: bool,
    ensure_ascii: bool,
    skipkeys: bool,
    allow_nan: bool,
    default: Option<Value>,
}

fn esc(s: &str, ascii: bool, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if ascii && (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn float_json(vm: &Vm, f: f64, allow_nan: bool) -> PyResult<String> {
    if f.is_nan() || f.is_infinite() {
        if !allow_nan {
            return Err(vm.value_error("Out of range float values are not JSON compliant"));
        }
        return Ok(if f.is_nan() { "NaN".into() } else if f > 0.0 { "Infinity".into() } else { "-Infinity".into() });
    }
    Ok(crate::format::float_repr(f))
}

fn encode(vm: &mut Vm, e: &Enc, v: &Value, depth: usize, out: &mut String) -> PyResult<()> {
    if depth > 500 {
        return Err(vm.value_error("Circular reference detected"));
    }
    match v {
        Value::None => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&float_json(vm, *f, e.allow_nan)?),
        Value::Obj(o) => match &o.payload {
            Payload::Str(s) => esc(s.as_str(), e.ensure_ascii, out),
            Payload::Int(b) => out.push_str(&b.to_string()),
            Payload::Float(f) => out.push_str(&float_json(vm, *f, e.allow_nan)?),
            Payload::List(_) | Payload::Tuple(_) => {
                let items = ops::collect(vm, v)?;
                if items.is_empty() {
                    out.push_str("[]");
                    return Ok(());
                }
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&e.item_sep);
                    }
                    newline(e, depth + 1, out);
                    encode(vm, e, it, depth + 1, out)?;
                }
                newline(e, depth, out);
                out.push(']');
            }
            Payload::Dict(_) => {
                let mut items = ops::mapping_items(vm, v)?;
                if e.sort_keys {
                    let mut err = None;
                    items.sort_by(|(a, _), (b, _)| {
                        let (sa, sb) = (a.as_str().unwrap_or(""), b.as_str().unwrap_or(""));
                        if a.as_str().is_none() || b.as_str().is_none() {
                            err = Some(());
                        }
                        sa.cmp(sb)
                    });
                    let _ = err;
                }
                if items.is_empty() {
                    out.push_str("{}");
                    return Ok(());
                }
                out.push('{');
                let mut first = true;
                for (k, val) in items {
                    let ks = match &k {
                        Value::Obj(ko) if matches!(ko.payload, Payload::Str(_)) => ko.as_str().unwrap().as_str().to_string(),
                        Value::None => "null".into(),
                        Value::Bool(b) => if *b { "true".into() } else { "false".into() },
                        Value::Int(i) => i.to_string(),
                        Value::Float(f) => float_json(vm, *f, e.allow_nan)?,
                        _ => {
                            if e.skipkeys {
                                continue;
                            }
                            return Err(vm.type_error(format!("keys must be str, int, float, bool or None, not {}", ops::type_name(vm, &k))));
                        }
                    };
                    if !first {
                        out.push_str(&e.item_sep);
                    }
                    first = false;
                    newline(e, depth + 1, out);
                    esc(&ks, e.ensure_ascii, out);
                    out.push_str(&e.key_sep);
                    encode(vm, e, &val, depth + 1, out)?;
                }
                newline(e, depth, out);
                out.push('}');
            }
            _ => match &e.default {
                Some(d) => {
                    let r = vm.call(d, vec![v.clone()], Vec::new())?;
                    encode(vm, e, &r, depth + 1, out)?;
                }
                None => return Err(vm.type_error(format!("Object of type {} is not JSON serializable", o.class_name()))),
            },
        },
    }
    Ok(())
}

fn newline(e: &Enc, depth: usize, out: &mut String) {
    if let Some(ind) = &e.indent {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(ind);
        }
    }
}

fn dumps(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut e = Enc { indent: None, item_sep: ", ".into(), key_sep: ": ".into(), sort_keys: false, ensure_ascii: true, skipkeys: false, allow_nan: true, default: None };
    let mut seps_given = false;
    for (n, v) in k {
        match &**n {
            "indent" => {
                e.indent = match v {
                    Value::None => None,
                    Value::Int(i) => Some(" ".repeat((*i).max(0) as usize)),
                    other => other.as_str().map(|s| s.to_string()),
                }
            }
            "separators" if !v.is_none() => {
                let p = ops::collect(vm, v)?;
                e.item_sep = p.first().and_then(|x| x.as_str()).unwrap_or(", ").to_string();
                e.key_sep = p.get(1).and_then(|x| x.as_str()).unwrap_or(": ").to_string();
                seps_given = true;
            }
            "sort_keys" => e.sort_keys = ops::truthy(vm, v)?,
            "ensure_ascii" => e.ensure_ascii = ops::truthy(vm, v)?,
            "skipkeys" => e.skipkeys = ops::truthy(vm, v)?,
            "allow_nan" => e.allow_nan = ops::truthy(vm, v)?,
            "default" if !v.is_none() => e.default = Some(v.clone()),
            _ => {}
        }
    }
    if e.indent.is_some() && !seps_given {
        e.item_sep = ",".into();
    }
    let mut out = String::new();
    encode(vm, &e, &a[0], 0, &mut out)?;
    Ok(vm.types.new_string(out))
}

struct Dec<'a> {
    s: &'a [char],
    p: usize,
    object_hook: Option<Value>,
    pairs_hook: Option<Value>,
}

fn decode_error(vm: &Vm, msg: &str, pos: usize) -> crate::vm::PyErr {
    let cls = match crate::import::get_module(vm, "json.decoder").or_else(|| crate::import::get_module(vm, "_json")).and_then(|m| m.obj().and_then(|o| o.dict_get_str_raw("JSONDecodeError"))) {
        Some(Value::Obj(c)) => c,
        _ => vm.types.exc("ValueError"),
    };
    let m = vm.types.new_string(format!("{msg}: char {pos}"));
    crate::vm::PyErr::Exc(vm.new_exc(&cls, vec![m]))
}

impl<'a> Dec<'a> {
    fn ws(&mut self) {
        while self.p < self.s.len() && self.s[self.p].is_whitespace() {
            self.p += 1;
        }
    }
    fn value(&mut self, vm: &mut Vm) -> PyResult<Value> {
        self.ws();
        let Some(&c) = self.s.get(self.p) else { return Err(decode_error(vm, "Expecting value", self.p)) };
        match c {
            '{' => {
                self.p += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.s.get(self.p) == Some(&'}') {
                    self.p += 1;
                } else {
                    loop {
                        self.ws();
                        if self.s.get(self.p) != Some(&'"') {
                            return Err(decode_error(vm, "Expecting property name enclosed in double quotes", self.p));
                        }
                        let k = self.string(vm)?;
                        self.ws();
                        if self.s.get(self.p) != Some(&':') {
                            return Err(decode_error(vm, "Expecting ':' delimiter", self.p));
                        }
                        self.p += 1;
                        let v = self.value(vm)?;
                        pairs.push((vm.types.new_string(k), v));
                        self.ws();
                        match self.s.get(self.p) {
                            Some(',') => self.p += 1,
                            Some('}') => {
                                self.p += 1;
                                break;
                            }
                            _ => return Err(decode_error(vm, "Expecting ',' delimiter", self.p)),
                        }
                    }
                }
                if let Some(h) = self.pairs_hook.clone() {
                    let items = pairs.into_iter().map(|(k, v)| vm.types.new_tuple(vec![k, v])).collect();
                    let l = vm.types.new_list(items);
                    return vm.call(&h, vec![l], Vec::new());
                }
                let d = vm.types.new_dict_obj(Dict::new());
                for (k, v) in pairs {
                    ops::dict_set(vm, &d, k, v)?;
                }
                if let Some(h) = self.object_hook.clone() {
                    return vm.call(&h, vec![Value::Obj(d)], Vec::new());
                }
                Ok(Value::Obj(d))
            }
            '[' => {
                self.p += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.p) == Some(&']') {
                    self.p += 1;
                    return Ok(vm.types.new_list(items));
                }
                loop {
                    items.push(self.value(vm)?);
                    self.ws();
                    match self.s.get(self.p) {
                        Some(',') => self.p += 1,
                        Some(']') => {
                            self.p += 1;
                            break;
                        }
                        _ => return Err(decode_error(vm, "Expecting ',' delimiter", self.p)),
                    }
                }
                Ok(vm.types.new_list(items))
            }
            '"' => {
                let s = self.string(vm)?;
                Ok(vm.types.new_string(s))
            }
            't' if self.lit("true") => Ok(Value::Bool(true)),
            'f' if self.lit("false") => Ok(Value::Bool(false)),
            'n' if self.lit("null") => Ok(Value::None),
            'N' if self.lit("NaN") => Ok(Value::Float(f64::NAN)),
            'I' if self.lit("Infinity") => Ok(Value::Float(f64::INFINITY)),
            '-' if self.s.get(self.p + 1) == Some(&'I') && self.lit("-Infinity") => Ok(Value::Float(f64::NEG_INFINITY)),
            _ => self.number(vm),
        }
    }
    fn lit(&mut self, w: &str) -> bool {
        let n = w.chars().count();
        if self.p + n <= self.s.len() && self.s[self.p..self.p + n].iter().copied().eq(w.chars()) {
            self.p += n;
            return true;
        }
        false
    }
    fn number(&mut self, vm: &mut Vm) -> PyResult<Value> {
        let st = self.p;
        if self.s.get(self.p) == Some(&'-') {
            self.p += 1;
        }
        let mut float = false;
        while self.p < self.s.len() {
            let c = self.s[self.p];
            if c.is_ascii_digit() {
                self.p += 1;
            } else if matches!(c, '.' | 'e' | 'E' | '+' | '-') && self.p > st {
                float = true;
                self.p += 1;
            } else {
                break;
            }
        }
        let t: String = self.s[st..self.p].iter().collect();
        if t.is_empty() || t == "-" {
            return Err(decode_error(vm, "Expecting value", st));
        }
        if float {
            return t.parse().map(Value::Float).map_err(|_| decode_error(vm, "Expecting value", st));
        }
        let b: num_bigint::BigInt = t.parse().map_err(|_| decode_error(vm, "Expecting value", st))?;
        Ok(vm.types.new_bigint(b))
    }
    fn string(&mut self, vm: &mut Vm) -> PyResult<String> {
        self.p += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.p) else { return Err(decode_error(vm, "Unterminated string starting at", self.p)) };
            self.p += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(&e) = self.s.get(self.p) else { return Err(decode_error(vm, "Invalid \\escape", self.p)) };
                    self.p += 1;
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\x08'),
                        'f' => out.push('\x0c'),
                        '/' => out.push('/'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        'u' => {
                            let hex: String = self.s.get(self.p..self.p + 4).ok_or_else(|| decode_error(vm, "Invalid \\uXXXX escape", self.p))?.iter().collect();
                            self.p += 4;
                            let u = u32::from_str_radix(&hex, 16).map_err(|_| decode_error(vm, "Invalid \\uXXXX escape", self.p))?;
                            if (0xd800..0xdc00).contains(&u) && self.s.get(self.p) == Some(&'\\') && self.s.get(self.p + 1) == Some(&'u') {
                                let hex2: String = self.s.get(self.p + 2..self.p + 6).unwrap_or(&[]).iter().collect();
                                if let Ok(lo) = u32::from_str_radix(&hex2, 16) {
                                    if (0xdc00..0xe000).contains(&lo) {
                                        self.p += 6;
                                        out.push(char::from_u32(0x10000 + ((u - 0xd800) << 10) + (lo - 0xdc00)).unwrap_or('\u{fffd}'));
                                        continue;
                                    }
                                }
                            }
                            out.push(char::from_u32(u).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(decode_error(vm, "Invalid \\escape", self.p)),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

fn loads(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let src = match a[0].payload() {
        Some(Payload::Str(s)) => s.as_str().to_string(),
        Some(Payload::Bytes(b)) => String::from_utf8_lossy(b).trim_start_matches('\u{feff}').to_string(),
        _ => return Err(vm.type_error("the JSON object must be str, bytes or bytearray")),
    };
    let chars: Vec<char> = src.chars().collect();
    let mut d = Dec { s: &chars, p: 0, object_hook: None, pairs_hook: None };
    for (n, v) in k {
        match &**n {
            "object_hook" if !v.is_none() => d.object_hook = Some(v.clone()),
            "object_pairs_hook" if !v.is_none() => d.pairs_hook = Some(v.clone()),
            _ => {}
        }
    }
    let v = d.value(vm)?;
    d.ws();
    if d.p != chars.len() {
        return Err(decode_error(vm, "Extra data", d.p));
    }
    Ok(v)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_json", &[("dumps", dumps), ("loads", loads)]);
    let e = vm.types.new_builtin_type("JSONDecodeError", &vm.types.exc("ValueError"), BaseKind::Exception, false);
    super::set(vm, &m, "JSONDecodeError", Value::Obj(e));
    Ok(m)
}
