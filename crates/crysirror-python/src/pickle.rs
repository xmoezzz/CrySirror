// SPDX-License-Identifier: MPL-2.0
//! Pickle protocols 0–5: an unpickler with a pluggable `find_class`
//! (engines map pickled module/class names to their own classes) and a
//! pickler producing protocol 2+ streams through the reduce protocol.
//!
//! Python 2 byte strings (`STRING`/`BINSTRING`) decode to text with the
//! configured encoding (UTF-8, falling back to Latin-1), matching the single
//! text type this interpreter uses for both dialects.

use crate::builtins::{parse, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyErr, PyResult, Vm};
use num_bigint::BigInt;
use std::cell::RefCell;
use std::rc::Rc;

pub type FindClass<'a> = &'a mut dyn FnMut(&mut Vm, &str, &str) -> PyResult<Value>;

pub struct UnpickleOptions {
    /// Decode Python 2 `str` payloads as text (true) or keep bytes.
    pub py2_str_as_text: bool,
    pub persistent_load: Option<Value>,
}

impl Default for UnpickleOptions {
    fn default() -> Self {
        UnpickleOptions { py2_str_as_text: true, persistent_load: None }
    }
}

fn err(vm: &Vm, msg: impl Into<String>) -> PyErr {
    let cls = match crate::import::get_module(vm, "_pickle").and_then(|m| m.obj().and_then(|o| o.dict_get_str_raw("UnpicklingError"))) {
        Some(Value::Obj(c)) => c,
        _ => vm.types.exc("ValueError"),
    };
    let m = vm.types.new_string(msg.into());
    PyErr::Exc(vm.new_exc(&cls, vec![m]))
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let r = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(r)
    }
    fn byte(&mut self) -> Option<u8> {
        let b = *self.d.get(self.p)?;
        self.p += 1;
        Some(b)
    }
    fn line(&mut self) -> Option<&'a [u8]> {
        let st = self.p;
        let end = self.d[st..].iter().position(|b| *b == b'\n')? + st;
        self.p = end + 1;
        Some(&self.d[st..end])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
}

fn text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|c| *c as char).collect(),
    }
}

fn py2_unescape(b: &[u8]) -> Vec<u8> {
    // Protocol 0 STRING: a Python repr of a byte string.
    let s = text(b);
    let s = s.trim();
    let inner = if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) { &s[1..s.len() - 1] } else { s };
    crate::parser::decode_bytes(inner, false).unwrap_or_else(|_| inner.as_bytes().to_vec())
}

fn raw_unicode_unescape(b: &[u8]) -> String {
    let s = text(b);
    crate::parser::decode_str(&s, true, true).unwrap_or(s)
}

pub fn loads(vm: &mut Vm, data: &[u8], find_class: FindClass, opts: &UnpickleOptions) -> PyResult<Value> {
    let mut r = Reader { d: data, p: 0 };
    let mut stack: Vec<Value> = Vec::new();
    let mut marks: Vec<usize> = Vec::new();
    let mut memo: std::collections::HashMap<u64, Value> = std::collections::HashMap::new();
    let eof = |vm: &Vm| err(vm, "pickle data was truncated");
    macro_rules! pop {
        () => {
            stack.pop().ok_or_else(|| err(vm, "unpickling stack underflow"))?
        };
    }
    macro_rules! pop_mark {
        () => {{
            let m = marks.pop().ok_or_else(|| err(vm, "could not find MARK"))?;
            if m > stack.len() {
                return Err(err(vm, "could not find MARK"));
            }
            stack.split_off(m)
        }};
    }
    macro_rules! top {
        () => {
            stack.last().cloned().ok_or_else(|| err(vm, "unpickling stack underflow"))?
        };
    }
    let py2_str = |vm: &Vm, b: &[u8]| -> Value {
        if opts.py2_str_as_text {
            vm.types.new_string(text(b))
        } else {
            vm.types.new_bytes(b.to_vec())
        }
    };
    loop {
        let op = r.byte().ok_or_else(|| eof(vm))?;
        match op {
            b'(' => marks.push(stack.len()),
            b'.' => return stack.pop().ok_or_else(|| err(vm, "unpickling stack underflow")),
            b'0' => {
                if marks.last() == Some(&stack.len()) {
                    marks.pop();
                } else {
                    pop!();
                }
            }
            b'1' => {
                pop_mark!();
            }
            b'2' => {
                let v = top!();
                stack.push(v);
            }
            b'F' => {
                let l = r.line().ok_or_else(|| eof(vm))?;
                let f: f64 = text(l).trim().parse().map_err(|_| err(vm, "could not convert string to float"))?;
                stack.push(Value::Float(f));
            }
            b'I' => {
                let l = text(r.line().ok_or_else(|| eof(vm))?);
                let t = l.trim();
                let v = match t {
                    "00" => Value::Bool(false),
                    "01" => Value::Bool(true),
                    _ => {
                        let b: BigInt = t.trim_end_matches('L').parse().map_err(|_| err(vm, "invalid literal for int"))?;
                        vm.types.new_bigint(b)
                    }
                };
                stack.push(v);
            }
            b'J' => {
                let b = r.take(4).ok_or_else(|| eof(vm))?;
                stack.push(Value::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64));
            }
            b'K' => {
                let b = r.byte().ok_or_else(|| eof(vm))?;
                stack.push(Value::Int(b as i64));
            }
            b'M' => {
                let b = r.take(2).ok_or_else(|| eof(vm))?;
                stack.push(Value::Int(u16::from_le_bytes([b[0], b[1]]) as i64));
            }
            b'L' => {
                let l = text(r.line().ok_or_else(|| eof(vm))?);
                let b: BigInt = l.trim().trim_end_matches('L').parse().map_err(|_| err(vm, "invalid literal for long"))?;
                stack.push(vm.types.new_bigint(b));
            }
            0x8a | 0x8b => {
                let n = if op == 0x8a { r.byte().ok_or_else(|| eof(vm))? as usize } else { r.u32().ok_or_else(|| eof(vm))? as usize };
                let b = r.take(n).ok_or_else(|| eof(vm))?;
                stack.push(vm.types.new_bigint(BigInt::from_signed_bytes_le(b)));
            }
            b'N' => stack.push(Value::None),
            0x88 => stack.push(Value::Bool(true)),
            0x89 => stack.push(Value::Bool(false)),
            b'G' => {
                let b = r.take(8).ok_or_else(|| eof(vm))?;
                stack.push(Value::Float(f64::from_be_bytes(b.try_into().unwrap())));
            }
            b'S' => {
                let l = r.line().ok_or_else(|| eof(vm))?;
                let b = py2_unescape(l);
                stack.push(py2_str(vm, &b));
            }
            b'T' => {
                let n = r.u32().ok_or_else(|| eof(vm))? as usize;
                let b = r.take(n).ok_or_else(|| eof(vm))?;
                stack.push(py2_str(vm, b));
            }
            b'U' => {
                let n = r.byte().ok_or_else(|| eof(vm))? as usize;
                let b = r.take(n).ok_or_else(|| eof(vm))?;
                stack.push(py2_str(vm, b));
            }
            b'V' => {
                let l = r.line().ok_or_else(|| eof(vm))?;
                stack.push(vm.types.new_string(raw_unicode_unescape(l)));
            }
            b'X' | 0x8c | 0x8d => {
                let n = match op {
                    b'X' => r.u32().ok_or_else(|| eof(vm))? as usize,
                    0x8c => r.byte().ok_or_else(|| eof(vm))? as usize,
                    _ => r.u64().ok_or_else(|| eof(vm))? as usize,
                };
                let b = r.take(n).ok_or_else(|| eof(vm))?;
                stack.push(vm.types.new_string(String::from_utf8_lossy(b).to_string()));
            }
            b'B' | b'C' | 0x8e | 0x96 => {
                let n = match op {
                    b'B' => r.u32().ok_or_else(|| eof(vm))? as usize,
                    b'C' => r.byte().ok_or_else(|| eof(vm))? as usize,
                    _ => r.u64().ok_or_else(|| eof(vm))? as usize,
                };
                let b = r.take(n).ok_or_else(|| eof(vm))?.to_vec();
                if op == 0x96 {
                    stack.push(Value::Obj(new_obj(vm.types.bytearray.clone(), None, Payload::ByteArray(RefCell::new(b)))));
                } else {
                    stack.push(vm.types.new_bytes(b));
                }
            }
            b')' => stack.push(vm.types.new_tuple(Vec::new())),
            b't' => {
                let items = pop_mark!();
                stack.push(vm.types.new_tuple(items));
            }
            0x85 | 0x86 | 0x87 => {
                let n = (op - 0x84) as usize;
                if stack.len() < n {
                    return Err(err(vm, "unpickling stack underflow"));
                }
                let items = stack.split_off(stack.len() - n);
                stack.push(vm.types.new_tuple(items));
            }
            b']' => stack.push(vm.types.new_list(Vec::new())),
            b'l' => {
                let items = pop_mark!();
                stack.push(vm.types.new_list(items));
            }
            b'}' => stack.push(vm.types.new_dict()),
            b'd' => {
                let items = pop_mark!();
                let d = vm.types.new_dict_obj(Dict::new());
                let mut it = items.into_iter();
                while let (Some(k), Some(v)) = (it.next(), it.next()) {
                    ops::dict_set(vm, &d, k, v)?;
                }
                stack.push(Value::Obj(d));
            }
            0x8f => stack.push(vm.types.new_set(Dict::new())),
            0x90 => {
                let items = pop_mark!();
                let s = top!();
                if matches!(s.payload(), Some(Payload::Set(_))) {
                    for it in items {
                        ops::set_add(vm, &s, it)?;
                    }
                } else {
                    for it in items {
                        vm.call_method(&s, "add", vec![it])?;
                    }
                }
            }
            0x91 => {
                let items = pop_mark!();
                let t = ops::make_table(vm, items)?;
                stack.push(vm.types.new_frozenset(t));
            }
            b'a' => {
                let v = pop!();
                let l = top!();
                append(vm, &l, vec![v])?;
            }
            b'e' => {
                let items = pop_mark!();
                let l = top!();
                append(vm, &l, items)?;
            }
            b's' => {
                let v = pop!();
                let k = pop!();
                let d = top!();
                ops::setitem(vm, &d, k, v)?;
            }
            b'u' => {
                let items = pop_mark!();
                let d = top!();
                let mut it = items.into_iter();
                while let (Some(k), Some(v)) = (it.next(), it.next()) {
                    ops::setitem(vm, &d, k, v)?;
                }
            }
            b'g' => {
                let l = text(r.line().ok_or_else(|| eof(vm))?);
                let k: u64 = l.trim().parse().map_err(|_| err(vm, "bad memo key"))?;
                stack.push(memo.get(&k).cloned().ok_or_else(|| err(vm, format!("Memo value not found at index {k}")))?);
            }
            b'h' => {
                let k = r.byte().ok_or_else(|| eof(vm))? as u64;
                stack.push(memo.get(&k).cloned().ok_or_else(|| err(vm, format!("Memo value not found at index {k}")))?);
            }
            b'j' => {
                let k = r.u32().ok_or_else(|| eof(vm))? as u64;
                stack.push(memo.get(&k).cloned().ok_or_else(|| err(vm, format!("Memo value not found at index {k}")))?);
            }
            b'p' => {
                let l = text(r.line().ok_or_else(|| eof(vm))?);
                let k: u64 = l.trim().parse().map_err(|_| err(vm, "bad memo key"))?;
                let v = top!();
                memo.insert(k, v);
            }
            b'q' => {
                let k = r.byte().ok_or_else(|| eof(vm))? as u64;
                let v = top!();
                memo.insert(k, v);
            }
            b'r' => {
                let k = r.u32().ok_or_else(|| eof(vm))? as u64;
                let v = top!();
                memo.insert(k, v);
            }
            0x94 => {
                let v = top!();
                let k = memo.len() as u64;
                memo.insert(k, v);
            }
            b'c' => {
                let m = text(r.line().ok_or_else(|| eof(vm))?);
                let n = text(r.line().ok_or_else(|| eof(vm))?);
                let v = find_class(vm, m.trim(), n.trim())?;
                stack.push(v);
            }
            0x93 => {
                let n = pop!();
                let m = pop!();
                let (Some(ms), Some(ns)) = (m.as_str(), n.as_str()) else { return Err(err(vm, "STACK_GLOBAL requires str")) };
                let (ms, ns) = (ms.to_string(), ns.to_string());
                let v = find_class(vm, &ms, &ns)?;
                stack.push(v);
            }
            b'R' => {
                let args = pop!();
                let f = pop!();
                let a = ops::collect(vm, &args)?;
                let v = vm.call(&f, a, Vec::new())?;
                stack.push(v);
            }
            0x81 => {
                let args = pop!();
                let cls = pop!();
                let mut a = vec![cls.clone()];
                a.extend(ops::collect(vm, &args)?);
                let new = ops::getattr(vm, &cls, "__new__")?;
                let v = vm.call(&new, a, Vec::new())?;
                stack.push(v);
            }
            0x92 => {
                let kw = pop!();
                let args = pop!();
                let cls = pop!();
                let mut a = vec![cls.clone()];
                a.extend(ops::collect(vm, &args)?);
                let mut k = Vec::new();
                for (kk, v) in ops::mapping_items(vm, &kw)? {
                    k.push((Rc::from(kk.as_str().unwrap_or("")), v));
                }
                let new = ops::getattr(vm, &cls, "__new__")?;
                let v = vm.call(&new, a, k)?;
                stack.push(v);
            }
            b'i' => {
                let m = text(r.line().ok_or_else(|| eof(vm))?);
                let n = text(r.line().ok_or_else(|| eof(vm))?);
                let cls = find_class(vm, m.trim(), n.trim())?;
                let args = pop_mark!();
                let v = instantiate(vm, &cls, args)?;
                stack.push(v);
            }
            b'o' => {
                let mut items = pop_mark!();
                if items.is_empty() {
                    return Err(err(vm, "OBJ without class"));
                }
                let cls = items.remove(0);
                let v = instantiate(vm, &cls, items)?;
                stack.push(v);
            }
            b'b' => {
                let state = pop!();
                let inst = top!();
                build(vm, &inst, state)?;
            }
            b'P' => {
                let l = r.line().ok_or_else(|| eof(vm))?;
                let pid = vm.types.new_string(text(l));
                let pl = opts.persistent_load.clone().ok_or_else(|| err(vm, "A load persistent id instruction was encountered, but no persistent_load function was specified."))?;
                let v = vm.call(&pl, vec![pid], Vec::new())?;
                stack.push(v);
            }
            b'Q' => {
                let pid = pop!();
                let pl = opts.persistent_load.clone().ok_or_else(|| err(vm, "A load persistent id instruction was encountered, but no persistent_load function was specified."))?;
                let v = vm.call(&pl, vec![pid], Vec::new())?;
                stack.push(v);
            }
            0x80 => {
                r.byte().ok_or_else(|| eof(vm))?;
            }
            0x95 => {
                r.take(8).ok_or_else(|| eof(vm))?;
            }
            0x82 | 0x83 | 0x84 => {
                let n = match op {
                    0x82 => r.byte().map(|b| b as usize),
                    0x83 => r.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize),
                    _ => r.u32().map(|b| b as usize),
                }
                .ok_or_else(|| eof(vm))?;
                return Err(err(vm, format!("unregistered extension code {n}")));
            }
            0x97 | 0x98 => return Err(err(vm, "out-of-band buffers are not supported")),
            other => return Err(err(vm, format!("invalid load key, '\\x{other:02x}'."))),
        }
    }
}

fn append(vm: &mut Vm, l: &Value, items: Vec<Value>) -> PyResult<()> {
    if let Some(Payload::List(lv)) = l.payload() {
        if !ops::is_user_type(&ops::type_of(vm, l)) || ops::lookup_user(&ops::type_of(vm, l), "append").is_none() && ops::lookup_user(&ops::type_of(vm, l), "extend").is_none() {
            lv.borrow_mut().extend(items);
            return Ok(());
        }
    }
    if let Some(ext) = ops::getattr_opt(vm, l, "extend")? {
        let lst = vm.types.new_list(items);
        vm.call(&ext, vec![lst], Vec::new())?;
        return Ok(());
    }
    for it in items {
        vm.call_method(l, "append", vec![it])?;
    }
    Ok(())
}

fn instantiate(vm: &mut Vm, cls: &Value, args: Vec<Value>) -> PyResult<Value> {
    if args.is_empty() && ops::is_type(cls) && ops::lookup_user(cls.obj().unwrap(), "__getinitargs__").is_none() {
        // Old-style class instance creation without __init__.
        let new = ops::getattr(vm, cls, "__new__")?;
        return vm.call(&new, vec![cls.clone()], Vec::new());
    }
    vm.call(cls, args, Vec::new())
}

/// BUILD: `__setstate__` or dict/slot state update.
pub fn build(vm: &mut Vm, inst: &Value, state: Value) -> PyResult<()> {
    if let Some(ss) = ops::getattr_opt(vm, inst, "__setstate__")? {
        let is_default = matches!(ss.payload(), Some(Payload::Method { func, .. }) if matches!(func.payload(), Some(Payload::Builtin(b)) if b.name == "object.__setstate__"));
        if !is_default {
            vm.call(&ss, vec![state], Vec::new())?;
            return Ok(());
        }
    }
    let (dstate, slots) = match state.payload() {
        Some(Payload::Tuple(t)) if t.len() == 2 => (t[0].clone(), t[1].clone()),
        _ => (state, Value::None),
    };
    if !dstate.is_none() {
        let Value::Obj(o) = inst else { return Err(vm.type_error("BUILD target is not an object")) };
        let items = ops::mapping_items(vm, &dstate)?;
        match o.attr_table() {
            Some(d) => {
                let mut d = d.borrow_mut();
                for (k, v) in items {
                    let key = match k.as_str() {
                        Some(s) => vm.types.intern(s),
                        None => k,
                    };
                    if key.as_str().is_some() {
                        d.set_str(key, v);
                    }
                }
            }
            None => {
                for (k, v) in items {
                    ops::setattr(vm, inst, &k, v)?;
                }
            }
        }
    }
    if !slots.is_none() {
        for (k, v) in ops::mapping_items(vm, &slots)? {
            ops::setattr(vm, inst, &k, v)?;
        }
    }
    Ok(())
}

/// Default class resolution: import `module` and fetch `name` (dotted
/// names allowed for protocol 4 qualnames).
pub fn default_find_class(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    let module = match module {
        "__builtin__" | "exceptions" => "builtins",
        "copy_reg" => "copyreg",
        m => m,
    };
    let name = match (module, name) {
        ("builtins", "unicode") => "str",
        ("builtins", "long") => "int",
        ("builtins", "xrange") => "range",
        ("builtins", "basestring") => "str",
        ("builtins", "StandardError") => "Exception",
        (_, n) => n,
    };
    let m = crate::import::import_module(vm, module)?;
    let mut v = m;
    for part in name.split('.') {
        v = ops::getattr(vm, &v, part)?;
    }
    Ok(v)
}

// ---- pickler ----

pub struct Pickler<'a> {
    pub out: Vec<u8>,
    pub proto: u8,
    memo: std::collections::HashMap<i64, (u32, Value)>,
    pub persistent_id: Option<Value>,
    pub reducer_override: Option<&'a mut dyn FnMut(&mut Vm, &Value) -> PyResult<Option<Value>>>,
}

impl<'a> Pickler<'a> {
    pub fn new(proto: u8) -> Self {
        Pickler { out: Vec::new(), proto: proto.clamp(0, 5), memo: std::collections::HashMap::new(), persistent_id: None, reducer_override: None }
    }

    fn put_memo(&mut self, v: &Value) {
        let id = v.id();
        if self.memo.contains_key(&id) {
            return;
        }
        let n = self.memo.len() as u32;
        self.memo.insert(id, (n, v.clone()));
        if self.proto >= 4 {
            self.out.push(0x94);
        } else if n < 256 {
            self.out.extend([b'q', n as u8]);
        } else {
            self.out.push(b'r');
            self.out.extend(n.to_le_bytes());
        }
    }

    fn get_memo(&mut self, v: &Value) -> bool {
        if let Some((n, _)) = self.memo.get(&v.id()) {
            let n = *n;
            if n < 256 {
                self.out.extend([b'h', n as u8]);
            } else {
                self.out.push(b'j');
                self.out.extend(n.to_le_bytes());
            }
            return true;
        }
        false
    }

    pub fn dump(&mut self, vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
        if self.proto >= 2 {
            self.out.extend([0x80, self.proto]);
        }
        self.save(vm, v)?;
        self.out.push(b'.');
        Ok(std::mem::take(&mut self.out))
    }

    fn save_str(&mut self, s: &str) {
        let b = s.as_bytes();
        if self.proto >= 4 && b.len() < 256 {
            self.out.extend([0x8c, b.len() as u8]);
        } else {
            self.out.push(b'X');
            self.out.extend((b.len() as u32).to_le_bytes());
        }
        self.out.extend(b);
    }

    fn save_global(&mut self, vm: &mut Vm, v: &Value) -> PyResult<()> {
        let name = ops::getattr_opt(vm, v, "__qualname__")?.or(ops::getattr_opt(vm, v, "__name__")?).and_then(|m| m.as_str().map(|s| s.to_string())).ok_or_else(|| vm.exc_msg("PicklingError", "Can't pickle object: no __name__"))?;
        let module = match ops::getattr_opt(vm, v, "__module__")?.and_then(|m| m.as_str().map(|s| s.to_string())) {
            Some(m) => m,
            None => whichmodule(vm, v, &name).unwrap_or_else(|| "builtins".into()),
        };
        let module = if module == "builtins" && self.proto < 3 { "__builtin__".to_string() } else { module };
        if self.proto >= 4 {
            self.save_str(&module);
            self.save_str(&name);
            self.out.push(0x93);
        } else {
            self.out.push(b'c');
            self.out.extend(module.as_bytes());
            self.out.push(b'\n');
            self.out.extend(name.as_bytes());
            self.out.push(b'\n');
        }
        self.put_memo(v);
        Ok(())
    }

    pub fn save(&mut self, vm: &mut Vm, v: &Value) -> PyResult<()> {
        if let Some(pid) = self.persistent_id.clone() {
            let r = vm.call(&pid, vec![v.clone()], Vec::new())?;
            if !r.is_none() {
                self.save(vm, &r)?;
                self.out.push(b'Q');
                return Ok(());
            }
        }
        match v {
            Value::None => {
                self.out.push(b'N');
                return Ok(());
            }
            Value::Bool(b) => {
                if self.proto >= 2 {
                    self.out.push(if *b { 0x88 } else { 0x89 });
                } else {
                    self.out.extend(if *b { b"I01\n" } else { b"I00\n" });
                }
                return Ok(());
            }
            Value::Int(i) => {
                self.save_int(&BigInt::from(*i));
                return Ok(());
            }
            Value::Float(f) => {
                self.out.push(b'G');
                self.out.extend(f.to_be_bytes());
                return Ok(());
            }
            Value::Obj(_) => {}
        }
        let Value::Obj(o) = v else { unreachable!() };
        let t = o.class();
        let user = ops::is_user_type(&t);
        if !user {
            match &o.payload {
                Payload::Str(s) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    self.save_str(s.as_str());
                    self.put_memo(v);
                    return Ok(());
                }
                Payload::Int(b) => {
                    self.save_int(b);
                    return Ok(());
                }
                Payload::Bytes(b) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    if self.proto >= 3 {
                        if b.len() < 256 {
                            self.out.extend([b'C', b.len() as u8]);
                        } else {
                            self.out.push(b'B');
                            self.out.extend((b.len() as u32).to_le_bytes());
                        }
                        self.out.extend(b.iter());
                    } else {
                        // Python 2 str.
                        if b.len() < 256 {
                            self.out.extend([b'U', b.len() as u8]);
                        } else {
                            self.out.push(b'T');
                            self.out.extend((b.len() as u32).to_le_bytes());
                        }
                        self.out.extend(b.iter());
                    }
                    self.put_memo(v);
                    return Ok(());
                }
                Payload::Tuple(items) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    let items = items.to_vec();
                    if items.is_empty() {
                        self.out.push(b')');
                        return Ok(());
                    }
                    if self.proto >= 2 && items.len() <= 3 {
                        for it in &items {
                            self.save(vm, it)?;
                        }
                        if self.get_memo(v) {
                            // Recursive tuple: drop the built copy.
                            return Ok(());
                        }
                        self.out.push(0x84 + items.len() as u8);
                    } else {
                        self.out.push(b'(');
                        for it in &items {
                            self.save(vm, it)?;
                        }
                        self.out.push(b't');
                    }
                    self.put_memo(v);
                    return Ok(());
                }
                Payload::List(l) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    self.out.push(b']');
                    self.put_memo(v);
                    let items = l.borrow().clone();
                    self.batch_appends(vm, &items)?;
                    return Ok(());
                }
                Payload::Dict(d) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    self.out.push(b'}');
                    self.put_memo(v);
                    let items = d.borrow().items();
                    self.batch_setitems(vm, &items)?;
                    return Ok(());
                }
                Payload::Set(d) if self.proto >= 4 => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    self.out.push(0x8f);
                    self.put_memo(v);
                    let keys = d.borrow().keys();
                    for chunk in keys.chunks(1000) {
                        self.out.push(b'(');
                        for k in chunk {
                            self.save(vm, k)?;
                        }
                        self.out.push(0x90);
                    }
                    return Ok(());
                }
                Payload::Type(_) | Payload::Builtin(_) | Payload::Function(_) => {
                    if self.get_memo(v) {
                        return Ok(());
                    }
                    if matches!(o.payload, Payload::Type(_)) {
                        if Rc::ptr_eq(o, &vm.types.none_type) {
                            return self.save_reduce_global(vm, "builtins", "type", &[Value::None], v);
                        }
                    }
                    return self.save_global(vm, v);
                }
                _ => {}
            }
        } else if matches!(o.payload, Payload::Type(_)) {
            if self.get_memo(v) {
                return Ok(());
            }
            return self.save_global(vm, v);
        }
        if self.get_memo(v) {
            return Ok(());
        }
        // Reduce protocol.
        let rv = match &mut self.reducer_override {
            Some(f) => f(vm, v)?,
            None => None,
        };
        let rv = match rv {
            Some(r) => r,
            None => {
                let copyreg = crate::import::import_module(vm, "copyreg")?;
                let dt = ops::getattr(vm, &copyreg, "dispatch_table")?;
                let tv = Value::Obj(t.clone());
                match dt.obj().map(|d| ops::dict_get(vm, d, &tv)).transpose()?.flatten() {
                    Some(red) => vm.call(&red, vec![v.clone()], Vec::new())?,
                    None => {
                        let p = Value::Int(self.proto as i64);
                        match ops::getattr_opt(vm, v, "__reduce_ex__")? {
                            Some(r) => vm.call(&r, vec![p], Vec::new())?,
                            None => vm.call_method(v, "__reduce__", Vec::new())?,
                        }
                    }
                }
            }
        };
        if let Some(s) = rv.as_str() {
            let s = s.to_string();
            let module = ops::getattr_opt(vm, v, "__module__")?.and_then(|m| m.as_str().map(|x| x.to_string())).unwrap_or_default();
            if self.proto >= 4 {
                self.save_str(&module);
                self.save_str(&s);
                self.out.push(0x93);
            } else {
                self.out.push(b'c');
                self.out.extend(module.as_bytes());
                self.out.push(b'\n');
                self.out.extend(s.as_bytes());
                self.out.push(b'\n');
            }
            self.put_memo(v);
            return Ok(());
        }
        let parts = ops::collect(vm, &rv)?;
        if parts.len() < 2 || parts.len() > 6 {
            return Err(vm.exc_msg("PicklingError", "tuple returned by __reduce__ must contain 2 through 6 elements"));
        }
        let func = parts[0].clone();
        let args = ops::collect(vm, &parts[1])?;
        let state = parts.get(2).cloned().unwrap_or(Value::None);
        let listitems = parts.get(3).cloned().unwrap_or(Value::None);
        let dictitems = parts.get(4).cloned().unwrap_or(Value::None);
        let is_newobj = self.proto >= 2 && ops::getattr_opt(vm, &func, "__name__")?.and_then(|n| n.as_str().map(|s| s == "__newobj__")).unwrap_or(false);
        if is_newobj && !args.is_empty() {
            self.save(vm, &args[0])?;
            let rest = vm.types.new_tuple(args[1..].to_vec());
            self.save(vm, &rest)?;
            self.out.push(0x81);
        } else {
            self.save(vm, &func)?;
            let at = vm.types.new_tuple(args);
            self.save(vm, &at)?;
            self.out.push(b'R');
        }
        if self.memo.contains_key(&v.id()) {
            // Built recursively during argument saving.
            self.out.push(b'0');
            self.get_memo(v);
        } else {
            self.put_memo(v);
        }
        if !listitems.is_none() {
            let items = ops::collect(vm, &listitems)?;
            self.batch_appends(vm, &items)?;
        }
        if !dictitems.is_none() {
            let mut items = Vec::new();
            for kv in ops::collect(vm, &dictitems)? {
                let p = ops::collect(vm, &kv)?;
                if p.len() == 2 {
                    items.push((p[0].clone(), p[1].clone()));
                }
            }
            self.batch_setitems(vm, &items)?;
        }
        if !state.is_none() {
            self.save(vm, &state)?;
            self.out.push(b'b');
        }
        Ok(())
    }

    fn save_reduce_global(&mut self, vm: &mut Vm, module: &str, name: &str, args: &[Value], v: &Value) -> PyResult<()> {
        self.out.push(b'c');
        self.out.extend(module.as_bytes());
        self.out.push(b'\n');
        self.out.extend(name.as_bytes());
        self.out.push(b'\n');
        let t = vm.types.new_tuple(args.to_vec());
        self.save(vm, &t)?;
        self.out.push(b'R');
        self.put_memo(v);
        Ok(())
    }

    fn save_int(&mut self, b: &BigInt) {
        use num_traits::ToPrimitive;
        if let Some(i) = b.to_i64() {
            if (0..256).contains(&i) {
                self.out.extend([b'K', i as u8]);
                return;
            }
            if (0..65536).contains(&i) {
                self.out.push(b'M');
                self.out.extend((i as u16).to_le_bytes());
                return;
            }
            if i32::try_from(i).is_ok() {
                self.out.push(b'J');
                self.out.extend((i as i32).to_le_bytes());
                return;
            }
        }
        if self.proto >= 2 {
            let bytes = b.to_signed_bytes_le();
            if bytes.len() < 256 {
                self.out.extend([0x8a, bytes.len() as u8]);
            } else {
                self.out.push(0x8b);
                self.out.extend((bytes.len() as u32).to_le_bytes());
            }
            self.out.extend(bytes);
        } else {
            self.out.push(b'L');
            self.out.extend(b.to_string().bytes());
            self.out.extend(b"L\n");
        }
    }

    fn batch_appends(&mut self, vm: &mut Vm, items: &[Value]) -> PyResult<()> {
        for chunk in items.chunks(1000) {
            if chunk.len() == 1 {
                self.save(vm, &chunk[0])?;
                self.out.push(b'a');
            } else {
                self.out.push(b'(');
                for it in chunk {
                    self.save(vm, it)?;
                }
                self.out.push(b'e');
            }
        }
        Ok(())
    }

    fn batch_setitems(&mut self, vm: &mut Vm, items: &[(Value, Value)]) -> PyResult<()> {
        for chunk in items.chunks(1000) {
            if chunk.len() == 1 {
                self.save(vm, &chunk[0].0)?;
                self.save(vm, &chunk[0].1)?;
                self.out.push(b's');
            } else {
                self.out.push(b'(');
                for (k, v) in chunk {
                    self.save(vm, k)?;
                    self.save(vm, v)?;
                }
                self.out.push(b'u');
            }
        }
        Ok(())
    }
}

/// Finds the module that exposes `v` as `name` (CPython `whichmodule`).
fn whichmodule(vm: &mut Vm, v: &Value, name: &str) -> Option<String> {
    let mods = match &vm.modules.payload {
        Payload::Dict(d) => d.borrow().items(),
        _ => return None,
    };
    for (k, m) in mods {
        let Value::Obj(mo) = &m else { continue };
        if !matches!(mo.payload, Payload::Module(..)) {
            continue;
        }
        if let Some(x) = mo.dict_get_str_raw(name) {
            if x.is(v) {
                return k.as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

pub fn dumps(vm: &mut Vm, v: &Value, proto: u8) -> PyResult<Vec<u8>> {
    Pickler::new(proto).dump(vm, v)
}

// ---- Python module `_pickle` ----

fn data_arg(vm: &Vm, v: &Value) -> PyResult<Vec<u8>> {
    ops::bytes_of(v).or_else(|| v.as_str().map(|s| s.chars().map(|c| c as u32 as u8).collect())).ok_or_else(|| vm.type_error("a bytes-like object is required"))
}

fn py_loads(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [data, _fix, encoding, _errors, _buffers] = parse(vm, "loads", a, k, ["data", "fix_imports", "encoding", "errors", "buffers"], 1)?;
    let d = data_arg(vm, data.as_ref().unwrap())?;
    let as_text = !matches!(encoding.as_ref().and_then(|e| e.as_str()), Some("bytes"));
    let opts = UnpickleOptions { py2_str_as_text: as_text, persistent_load: None };
    loads(vm, &d, &mut default_find_class, &opts)
}

fn py_dumps(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [obj, protocol, _fix] = parse(vm, "dumps", a, k, ["obj", "protocol", "fix_imports"], 1)?;
    let proto = match protocol {
        Some(p) if !p.is_none() => {
            let n = ops::index_of(vm, &p)?;
            if n < 0 { 5 } else { n as u8 }
        }
        _ => 4,
    };
    let b = dumps(vm, obj.as_ref().unwrap(), proto)?;
    Ok(vm.types.new_bytes(b))
}

fn py_load(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let f = a.first().cloned().ok_or_else(|| vm.type_error("load() missing file"))?;
    let d = vm.call_method(&f, "read", Vec::new())?;
    let mut args = vec![d];
    args.extend(a[1..].iter().cloned());
    py_loads(vm, &args, k)
}

fn py_dump(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    if a.len() < 2 {
        return Err(vm.type_error("dump() requires obj and file"));
    }
    let mut args = vec![a[0].clone()];
    args.extend(a[2..].iter().cloned());
    let b = py_dumps(vm, &args, k)?;
    vm.call_method(&a[1], "write", vec![b])?;
    Ok(Value::None)
}

// Pickler / Unpickler classes (subclassable; find_class/persistent_id hooks).

fn unpickler_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let file = a.get(1).cloned().ok_or_else(|| vm.type_error("Unpickler() missing file"))?;
    ops::set_raw_attr(vm, o, "_file", file);
    for (n, v) in k {
        if &**n == "encoding" {
            ops::set_raw_attr(vm, o, "_encoding", v.clone());
        }
    }
    Ok(Value::None)
}

fn unpickler_load(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let slf = a[0].clone();
    let Value::Obj(o) = &slf else { unreachable!() };
    let file = ops::get_raw_attr(o, "_file").unwrap_or(Value::None);
    let d = vm.call_method(&file, "read", Vec::new())?;
    let data = data_arg(vm, &d)?;
    let t = o.class();
    let custom = ops::lookup_user(&t, "find_class");
    let pl = ops::getattr_opt(vm, &slf, "persistent_load")?;
    let enc = ops::get_raw_attr(o, "_encoding");
    let opts = UnpickleOptions { py2_str_as_text: !matches!(enc.as_ref().and_then(|e| e.as_str()), Some("bytes")), persistent_load: pl };
    match custom {
        Some(fc) => {
            let mut f = |vm: &mut Vm, m: &str, n: &str| -> PyResult<Value> {
                let mv = vm.types.new_str(m);
                let nv = vm.types.new_str(n);
                vm.call(&fc, vec![slf.clone(), mv, nv], Vec::new())
            };
            loads(vm, &data, &mut f, &opts)
        }
        None => loads(vm, &data, &mut default_find_class, &opts),
    }
}

fn unpickler_find_class(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let m = crate::builtins::str_arg(vm, &a[1], "module")?;
    let n = crate::builtins::str_arg(vm, &a[2], "name")?;
    default_find_class(vm, &m, &n)
}

fn pickler_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    let [file, protocol, _fix] = parse(vm, "Pickler", &a[1..], k, ["file", "protocol", "fix_imports"], 1)?;
    ops::set_raw_attr(vm, o, "_file", file.unwrap());
    ops::set_raw_attr(vm, o, "_protocol", protocol.unwrap_or(Value::Int(4)));
    Ok(Value::None)
}

fn pickler_dump(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let slf = a[0].clone();
    let Value::Obj(o) = &slf else { unreachable!() };
    let proto = match ops::get_raw_attr(o, "_protocol") {
        Some(Value::Int(n)) if n >= 0 => n as u8,
        Some(Value::Int(_)) => 5,
        _ => 4,
    };
    let mut p = Pickler::new(proto);
    if let Some(pid) = ops::lookup_user(&o.class(), "persistent_id") {
        let b = crate::ops::bind(vm, &pid, &slf);
        p.persistent_id = Some(b);
    }
    let bytes = p.dump(vm, &a[1])?;
    let file = ops::get_raw_attr(o, "_file").unwrap_or(Value::None);
    let bv = vm.types.new_bytes(bytes);
    vm.call_method(&file, "write", vec![bv])?;
    Ok(Value::None)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crate::modules::module_with(vm, "_pickle", &[("loads", py_loads), ("dumps", py_dumps), ("load", py_load), ("dump", py_dump)]);
    let pe = vm.types.new_builtin_type("PickleError", &vm.types.exc("Exception"), BaseKind::Exception, false);
    let pk = vm.types.new_builtin_type("PicklingError", &pe, BaseKind::Exception, false);
    let uk = vm.types.new_builtin_type("UnpicklingError", &pe, BaseKind::Exception, false);
    crate::modules::set(vm, &m, "PickleError", Value::Obj(pe));
    crate::modules::set(vm, &m, "PicklingError", Value::Obj(pk));
    crate::modules::set(vm, &m, "UnpicklingError", Value::Obj(uk));
    let up = vm.types.new_builtin_type("Unpickler", &vm.types.object.clone(), BaseKind::Object, false);
    vm.types.add_method(&up, "__init__", unpickler_init);
    vm.types.add_method(&up, "load", unpickler_load);
    vm.types.add_method(&up, "find_class", unpickler_find_class);
    crate::modules::set(vm, &m, "Unpickler", Value::Obj(up));
    let pp = vm.types.new_builtin_type("Pickler", &vm.types.object.clone(), BaseKind::Object, false);
    vm.types.add_method(&pp, "__init__", pickler_init);
    vm.types.add_method(&pp, "dump", pickler_dump);
    crate::modules::set(vm, &m, "Pickler", Value::Obj(pp));
    crate::modules::set(vm, &m, "HIGHEST_PROTOCOL", Value::Int(5));
    crate::modules::set(vm, &m, "DEFAULT_PROTOCOL", Value::Int(4));
    Ok(m)
}
