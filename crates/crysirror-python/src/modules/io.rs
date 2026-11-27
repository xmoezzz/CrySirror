// SPDX-License-Identifier: MPL-2.0
//! `io.StringIO`, `io.BytesIO` and `open()`. Files go through the host's
//! scoped file hooks (`Host::read_file`/`write_file`); there is no direct
//! filesystem access from scripts.

use super::{module_with, set};
use crate::builtins::{parse, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

pub struct Buf {
    pub text: Option<Vec<char>>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub closed: bool,
    pub path: Option<String>,
    pub writable: bool,
    pub dirty: bool,
    lock: Option<Box<dyn std::any::Any>>,
}

impl Buf {
    fn len(&self) -> usize {
        match &self.text {
            Some(t) => t.len(),
            None => self.bytes.len(),
        }
    }
}

fn with_buf<R>(vm: &Vm, v: &Value, f: impl FnOnce(&mut Buf) -> R) -> PyResult<R> {
    match v.payload() {
        Some(Payload::Native(n)) => {
            let mut b = n.borrow_mut();
            match b.downcast_mut::<Buf>() {
                Some(buf) => {
                    if buf.closed {
                        drop(b);
                        return Err(vm.value_error("I/O operation on closed file."));
                    }
                    Ok(f(buf))
                }
                None => Err(vm.type_error("not a file object")),
            }
        }
        _ => Err(vm.type_error("not a file object")),
    }
}

fn mk(vm: &Vm, cls: &ObjRef, buf: Buf) -> Value {
    Value::Obj(new_obj(cls.clone(), Some(Dict::new()), Payload::Native(std::cell::RefCell::new(Box::new(buf)))))
}

fn stringio_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let [init, _nl] = parse(vm, "StringIO", &a[1..], k, ["initial_value", "newline"], 0)?;
    let text: Vec<char> = match init {
        Some(v) if !v.is_none() => crate::builtins::str_arg(vm, &v, "initial_value")?.chars().collect(),
        _ => Vec::new(),
    };
    Ok(mk(vm, &cls, Buf { text: Some(text), bytes: Vec::new(), pos: 0, closed: false, path: None, writable: true, dirty: false, lock: None }))
}

fn bytesio_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let [init] = parse(vm, "BytesIO", &a[1..], k, ["initial_bytes"], 0)?;
    let bytes = match init {
        Some(v) if !v.is_none() => ops::bytes_of(&v).ok_or_else(|| vm.type_error("a bytes-like object is required"))?,
        _ => Vec::new(),
    };
    Ok(mk(vm, &cls, Buf { text: None, bytes, pos: 0, closed: false, path: None, writable: true, dirty: false, lock: None }))
}

fn noinit(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn out_value(vm: &Vm, text: bool, chars: Vec<char>, bytes: Vec<u8>) -> Value {
    if text { vm.types.new_string(chars.into_iter().collect()) } else { vm.types.new_bytes(bytes) }
}

fn read(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = match a.get(1) {
        Some(v) if !v.is_none() => ops::index_of(vm, v)?,
        _ => -1,
    };
    let (text, c, b) = with_buf(vm, &a[0], |buf| {
        let len = buf.len();
        let end = if n < 0 { len } else { (buf.pos + n as usize).min(len) };
        let start = buf.pos.min(len);
        buf.pos = end;
        match &buf.text {
            Some(t) => (true, t[start..end].to_vec(), Vec::new()),
            None => (false, Vec::new(), buf.bytes[start..end].to_vec()),
        }
    })?;
    Ok(out_value(vm, text, c, b))
}

fn readline(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let limit = match a.get(1) {
        Some(v) if !v.is_none() => ops::index_of(vm, v)?,
        _ => -1,
    };
    let (text, c, b) = with_buf(vm, &a[0], |buf| {
        let len = buf.len();
        let start = buf.pos.min(len);
        let mut end = start;
        match &buf.text {
            Some(t) => {
                while end < len && (limit < 0 || ((end - start) as i64) < limit) {
                    end += 1;
                    if t[end - 1] == '\n' {
                        break;
                    }
                }
                buf.pos = end;
                (true, t[start..end].to_vec(), Vec::new())
            }
            None => {
                while end < len && (limit < 0 || ((end - start) as i64) < limit) {
                    end += 1;
                    if buf.bytes[end - 1] == b'\n' {
                        break;
                    }
                }
                buf.pos = end;
                (false, Vec::new(), buf.bytes[start..end].to_vec())
            }
        }
    })?;
    Ok(out_value(vm, text, c, b))
}

fn readlines(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    loop {
        let l = readline(vm, &a[..1], &[])?;
        if !ops::truthy(vm, &l)? {
            break;
        }
        out.push(l);
    }
    Ok(vm.types.new_list(out))
}

fn write(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = a.get(1).cloned().unwrap_or(Value::None);
    let s = v.as_str().map(|s| s.to_string());
    let b = ops::bytes_of(&v);
    let r = with_buf(vm, &a[0], |buf| -> Result<usize, &'static str> {
        if !buf.writable {
            return Err("not writable");
        }
        buf.dirty = true;
        match &mut buf.text {
            Some(t) => {
                let Some(s) = &s else { return Err("string argument expected") };
                let chars: Vec<char> = s.chars().collect();
                let n = chars.len();
                if buf.pos > t.len() {
                    t.resize(buf.pos, '\0');
                }
                let end = (buf.pos + n).min(t.len());
                t.splice(buf.pos..end, chars);
                buf.pos += n;
                Ok(n)
            }
            None => {
                let Some(b) = &b else { return Err("a bytes-like object is required") };
                if buf.pos > buf.bytes.len() {
                    buf.bytes.resize(buf.pos, 0);
                }
                let end = (buf.pos + b.len()).min(buf.bytes.len());
                buf.bytes.splice(buf.pos..end, b.iter().copied());
                buf.pos += b.len();
                Ok(b.len())
            }
        }
    })?;
    match r {
        Ok(n) => Ok(Value::Int(n as i64)),
        Err(m) => Err(vm.type_error(m)),
    }
}

fn writelines(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    for l in ops::collect(vm, &a[1])? {
        write(vm, &[a[0].clone(), l], &[])?;
    }
    Ok(Value::None)
}

fn getvalue(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (text, c, b) = with_buf(vm, &a[0], |buf| match &buf.text {
        Some(t) => (true, t.clone(), Vec::new()),
        None => (false, Vec::new(), buf.bytes.clone()),
    })?;
    Ok(out_value(vm, text, c, b))
}

fn seek(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let off = ops::index_of(vm, &a[1])?;
    let whence = match a.get(2) {
        Some(v) => ops::index_of(vm, v)?,
        None => 0,
    };
    let p = with_buf(vm, &a[0], |buf| {
        let base = match whence {
            1 => buf.pos as i64,
            2 => buf.len() as i64,
            _ => 0,
        };
        buf.pos = (base + off).max(0) as usize;
        buf.pos
    })?;
    Ok(Value::Int(p as i64))
}

fn tell(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(with_buf(vm, &a[0], |b| b.pos)? as i64))
}

fn truncate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let size = match a.get(1) {
        Some(v) if !v.is_none() => Some(ops::index_of(vm, v)?.max(0) as usize),
        _ => None,
    };
    let n = with_buf(vm, &a[0], |buf| {
        let n = size.unwrap_or(buf.pos);
        match &mut buf.text {
            Some(t) => t.truncate(n),
            None => buf.bytes.truncate(n),
        }
        buf.dirty = true;
        n
    })?;
    Ok(Value::Int(n as i64))
}

fn close(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let closed = closed(vm, a, k)?;
    if matches!(closed, Value::Bool(true)) { return Ok(Value::None); }
    flush(vm, a, k)?;
    with_buf(vm, &a[0], |buffer| {
        buffer.lock = None;
        buffer.closed = true;
    })?;
    Ok(Value::None)
}

fn flush(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let pending = with_buf(vm, &a[0], |buffer| {
        if !buffer.dirty || !buffer.writable { return None; }
        let path = buffer.path.clone()?;
        let data = match &buffer.text {
            Some(text) => text.iter().collect::<String>().into_bytes(),
            None => buffer.bytes.clone(),
        };
        Some((path, data))
    })?;
    if let Some((path, data)) = pending {
        if !vm.host.write_file(&path, &data) {
            return Err(vm.exc_msg("PermissionError", format!("cannot write '{path}'")));
        }
        with_buf(vm, &a[0], |buffer| buffer.dirty = false)?;
    }
    Ok(Value::None)
}

pub(super) fn file_lock(vm: &mut Vm, file: &Value, acquire: bool) -> PyResult<Value> {
    if !acquire {
        with_buf(vm, file, |buffer| buffer.lock = None)?;
        return Ok(Value::None);
    }
    let (path, held) = with_buf(vm, file, |buffer| {
        (buffer.path.as_ref().filter(|_| buffer.writable).cloned(), buffer.lock.is_some())
    })?;
    let path = path.ok_or_else(|| vm.exc_msg("OSError", "exclusive lock requires a writable scoped file"))?;
    if !held {
        let lock = vm.host.try_lock_file(&path).map_err(|error| vm.exc_msg("OSError", error))?;
        with_buf(vm, file, |buffer| buffer.lock = Some(lock))?;
    }
    Ok(Value::None)
}

fn enter(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(a[0].clone())
}

fn exit(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    close(vm, &a[..1], &[])?;
    Ok(Value::Bool(false))
}

fn next(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let l = readline(vm, &a[..1], &[])?;
    if !ops::truthy(vm, &l)? {
        return Err(vm.stop_iteration(Value::None));
    }
    Ok(l)
}

fn closed(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(match a[0].payload() {
        Some(Payload::Native(n)) => n.borrow().downcast_ref::<Buf>().map(|b| b.closed).unwrap_or(true),
        _ => {
            let _ = vm;
            true
        }
    }))
}

fn truthy(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn falsy(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn file_class(vm: &mut Vm, name: &str, new: NativeFn) -> ObjRef {
    let c = vm.types.new_builtin_type(name, &vm.types.object.clone(), BaseKind::Native, false);
    let n = vm.types.new_builtin("__new__", new);
    let n = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(n)));
    vm.types.set_attr(&c, "__new__", n);
    for (nm, f) in [
        ("__init__", noinit as NativeFn), ("read", read), ("readline", readline), ("readlines", readlines), ("write", write),
        ("writelines", writelines), ("getvalue", getvalue), ("seek", seek), ("tell", tell), ("truncate", truncate),
        ("close", close), ("flush", flush), ("__enter__", enter), ("__exit__", exit), ("__next__", next), ("next", next),
        ("readable", truthy), ("writable", truthy), ("seekable", truthy), ("isatty", falsy), ("read1", read),
    ] {
        vm.types.add_method(&c, nm, f);
    }
    let it = vm.types.new_builtin("__iter__", enter);
    vm.types.set_attr(&c, "__iter__", it);
    let cl = vm.types.new_builtin("closed", closed);
    let p = Value::Obj(new_obj(vm.types.property.clone(), None, Payload::Property { fget: cl, fset: Value::None, fdel: Value::None, doc: Value::None }));
    vm.types.set_attr(&c, "closed", p);
    let mn = vm.types.new_str("io");
    vm.types.set_attr(&c, "__module__", mn);
    c
}

pub fn open(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [file, mode, _buffering, encoding, errors, _newline] = parse(vm, "open", a, k, ["file", "mode", "buffering", "encoding", "errors", "newline"], 1)?;
    let path = crate::builtins::str_arg(vm, file.as_ref().unwrap(), "file")?;
    let mode = match mode {
        Some(m) => crate::builtins::str_arg(vm, &m, "mode")?,
        None => "r".into(),
    };
    let binary = mode.contains('b');
    let writing = mode.contains('w') || mode.contains('a') || mode.contains('x') || mode.contains('+');
    let mut data = if mode.contains('w') && !mode.contains('+') {
        Vec::new()
    } else {
        match vm.host.read_file(&path) {
            Some(d) => d,
            None if writing => Vec::new(),
            None => return Err(vm.exc_msg("FileNotFoundError", format!("[Errno 2] No such file or directory: '{path}'"))),
        }
    };
    let enc = match encoding {
        Some(e) if !e.is_none() => crate::builtins::str_arg(vm, &e, "encoding")?,
        _ => "utf-8".into(),
    };
    let err = match errors {
        Some(e) if !e.is_none() => crate::builtins::str_arg(vm, &e, "errors")?,
        _ => "strict".into(),
    };
    let text = if binary {
        None
    } else {
        if data.starts_with(&[0xef, 0xbb, 0xbf]) && crate::methods_str::normalize_encoding(&enc) == "utf8sig" {
            data.drain(..3);
        }
        let s = crate::methods_str::decode(vm, &data, &enc, &err)?;
        Some(s.replace("\r\n", "\n").chars().collect::<Vec<char>>())
    };
    let pos = if mode.contains('a') { text.as_ref().map(|t| t.len()).unwrap_or(data.len()) } else { 0 };
    let cls = io_class(vm, if binary { "BytesIO" } else { "StringIO" })?;
    let v = mk(vm, &cls, Buf { text, bytes: if binary { data } else { Vec::new() }, pos, closed: false, path: Some(path.clone()), writable: writing, dirty: mode.contains('w'), lock: None });
    if let Value::Obj(o) = &v {
        let pv = vm.types.new_string(path);
        ops::set_raw_attr(vm, o, "name", pv);
        let mv = vm.types.new_string(mode);
        ops::set_raw_attr(vm, o, "mode", mv);
    }
    Ok(v)
}

fn io_class(vm: &mut Vm, name: &str) -> PyResult<ObjRef> {
    let m = crate::import::import_module(vm, "_io")?;
    match ops::getattr(vm, &m, name)? {
        Value::Obj(c) => Ok(c),
        _ => Err(vm.type_error("io class missing")),
    }
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_io", &[("open", open)]);
    let s = file_class(vm, "StringIO", stringio_new);
    vm.classes.string_io = Some(s.clone());
    set(vm, &m, "StringIO", Value::Obj(s));
    let b = file_class(vm, "BytesIO", bytesio_new);
    vm.classes.bytes_io = Some(b.clone());
    set(vm, &m, "BytesIO", Value::Obj(b));
    for n in ["IOBase", "RawIOBase", "BufferedIOBase", "TextIOBase", "TextIOWrapper", "BufferedReader", "BufferedWriter", "FileIO"] {
        let c = vm.types.new_builtin_type(n, &vm.types.object.clone(), BaseKind::Object, false);
        set(vm, &m, n, Value::Obj(c));
    }
    let uo = vm.types.exc("OSError");
    set(vm, &m, "UnsupportedOperation", Value::Obj(uo));
    set(vm, &m, "DEFAULT_BUFFER_SIZE", Value::Int(8192));
    Ok(m)
}
