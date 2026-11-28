// SPDX-License-Identifier: MPL-2.0
//! The stream modules of the pygame layer: `rwobject` (pygame_sdl2 and
//! Ren'Py 8.5, SDL_RWops wrappers) and `iostream` (Ren'Py 8.6,
//! SDL_IOStream wrappers), after the vendored `rwobject.pyx` /
//! `iostream.pyx` (MIT/zlib).
//!
//! There is no SDL stream layer here: every stream is a native window over
//! an in-memory buffer, a Python file object (`builtins.open`, which the
//! runtime routes through the game's scoped storage) or the concatenation
//! of two streams (`RWopsIO.from_split`). Positions, `base`/`length`
//! windows, short reads at the end of a window and read-only splits behave
//! as SDL's implementations do. `fileno()` raises `OSError` (no OS
//! descriptors are exposed), as upstream does for non-file streams.
use super::py::{self, *};
use super::pygame::{self as pg, new_mod, set};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

enum Kind {
    Buffer { data: Vec<u8>, writable: bool },
    File { f: Value, base: i64, length: Option<i64>, close: bool, writable: bool },
    Split { a: Box<Stream>, b: Box<Stream> },
}

pub struct Stream {
    kind: Kind,
    pos: i64,
    closed: bool,
}

impl Stream {
    fn size(&self, vm: &mut Vm) -> PyResult<i64> {
        match &self.kind {
            Kind::Buffer { data, .. } => Ok(data.len() as i64),
            Kind::File { f, base, length, .. } => match length {
                Some(l) => Ok(*l),
                None => {
                    let cur = py::method(vm, f, "tell", Vec::new())?;
                    let end = py::method(vm, f, "seek", vec![Value::Int(0), Value::Int(2)])?;
                    py::method(vm, f, "seek", vec![cur])?;
                    Ok(py::int(vm, &end)? - base)
                }
            },
            Kind::Split { a, b } => Ok(a.size(vm)? + b.size(vm)?),
        }
    }

    fn read(&mut self, vm: &mut Vm, n: Option<i64>) -> PyResult<Vec<u8>> {
        let size = self.size(vm)?;
        let avail = (size - self.pos).max(0);
        let n = n.filter(|n| *n >= 0).map_or(avail, |n| n.min(avail));
        if n == 0 {
            return Ok(Vec::new());
        }
        let out = match &mut self.kind {
            Kind::Buffer { data, .. } => data[self.pos as usize..(self.pos + n) as usize].to_vec(),
            Kind::File { f, base, .. } => {
                py::method(vm, f, "seek", vec![Value::Int(*base + self.pos)])?;
                let r = py::method(vm, f, "read", vec![Value::Int(n)])?;
                ops::bytes_of(&r).map(|b| b.to_vec()).unwrap_or_default()
            }
            Kind::Split { a, b } => {
                let split = a.size(vm)?;
                let mut out = Vec::new();
                if self.pos < split {
                    a.pos = self.pos;
                    out.extend(a.read(vm, Some(n.min(split - self.pos)))?);
                }
                let rest = n - out.len() as i64;
                if rest > 0 {
                    b.pos = (self.pos + out.len() as i64 - split).max(0);
                    out.extend(b.read(vm, Some(rest))?);
                }
                out
            }
        };
        self.pos += out.len() as i64;
        Ok(out)
    }

    fn write(&mut self, vm: &mut Vm, b: &[u8]) -> PyResult<i64> {
        match &mut self.kind {
            Kind::Buffer { data, writable: true } => {
                let end = self.pos as usize + b.len();
                if end > data.len() {
                    return Err(vm.exc_msg("OSError", "write past the end of a fixed buffer"));
                }
                data[self.pos as usize..end].copy_from_slice(b);
            }
            Kind::File { f, base, writable: true, .. } => {
                py::method(vm, f, "seek", vec![Value::Int(*base + self.pos)])?;
                let v = vm.types.new_bytes(b.to_vec());
                py::method(vm, f, "write", vec![v])?;
            }
            _ => return Err(vm.exc_msg("OSError", "stream is not writable")),
        }
        self.pos += b.len() as i64;
        Ok(b.len() as i64)
    }

    fn seek(&mut self, vm: &mut Vm, off: i64, whence: i64) -> PyResult<i64> {
        let base = match whence {
            0 => 0,
            1 => self.pos,
            2 => self.size(vm)?,
            _ => return Err(vm.value_error(format!("invalid whence ({whence}, should be 0, 1 or 2)"))),
        };
        let p = base + off;
        if p < 0 {
            return Err(vm.exc_msg("OSError", "negative seek position"));
        }
        self.pos = p;
        Ok(p)
    }

    fn close(&mut self, vm: &mut Vm) -> PyResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        match &mut self.kind {
            Kind::File { f, close: true, .. } => {
                py::method(vm, f, "close", Vec::new())?;
            }
            Kind::Split { a, b } => {
                a.close(vm)?;
                b.close(vm)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        match &self.kind {
            Kind::Buffer { writable, .. } | Kind::File { writable, .. } => *writable,
            Kind::Split { .. } => false,
        }
    }
}

fn take(v: &Value) -> Option<Stream> {
    py::native::<Option<Stream>, _>(v, |s| s.take()).flatten()
}

fn put(v: &Value, s: Stream) {
    py::native::<Option<Stream>, _>(v, |o| *o = Some(s));
}

/// Runs `f` on the stream of `this` (the native slot is moved out while
/// Python callbacks may run).
fn on<R>(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut Vm, &mut Stream) -> PyResult<R>) -> PyResult<R> {
    let mut s = take(this).ok_or_else(|| vm.value_error("I/O operation on an uninitialized stream."))?;
    let r = if s.closed {
        Err(vm.value_error("I/O operation on closed file."))
    } else {
        f(vm, &mut s)
    };
    put(this, s);
    r
}

fn open_file(vm: &mut Vm, path: &Value, mode: &str) -> PyResult<Value> {
    let open = py::import_attr(vm, "builtins", "open")?;
    let m = vm.types.new_str(mode);
    vm.call(&open, vec![path.clone(), m], Vec::new())
}

fn stream_of(vm: &mut Vm, obj: &Value, mode: &str, base: Option<i64>, length: Option<i64>) -> PyResult<Stream> {
    let writable = mode.contains('w') || mode.contains('+') || mode.contains('a');
    let mut obj = obj.clone();
    while let Some(raw) = py::get_opt(vm, &obj, "raw")? {
        if take(&raw).map(|s| {
            put(&raw, Stream { kind: Kind::Buffer { data: Vec::new(), writable: false }, pos: 0, closed: true });
            s
        }).is_some() {
            obj = raw;
            break;
        }
        obj = raw;
    }
    if let Some(data) = ops::bytes_of(&obj) {
        let mut data = data.to_vec();
        if let (Some(b), Some(l)) = (base, length) {
            data = data[(b as usize).min(data.len())..((b + l) as usize).min(data.len())].to_vec();
        }
        return Ok(Stream { kind: Kind::Buffer { data, writable: false }, pos: 0, closed: false });
    }
    let (f, close) = if obj.as_str().is_some() {
        (open_file(vm, &obj, mode)?, true)
    } else {
        (obj, false)
    };
    let base = base.unwrap_or(0);
    if base != 0 {
        py::method(vm, &f, "seek", vec![Value::Int(base)])?;
    }
    Ok(Stream { kind: Kind::File { f, base, length, close, writable }, pos: 0, closed: false })
}

fn opt_i64(vm: &mut Vm, v: &Value) -> PyResult<Option<i64>> {
    if py::is_none(v) { Ok(None) } else { Ok(Some(py::int(vm, v)?)) }
}

fn name_of(vm: &mut Vm, obj: &Value, name: &Value) -> PyResult<Value> {
    if !py::is_none(name) {
        return Ok(name.clone());
    }
    if obj.as_str().is_some() {
        return Ok(obj.clone());
    }
    Ok(py::get_opt(vm, obj, "name")?.unwrap_or(Value::None))
}

fn new_native(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = a.first().and_then(|c| c.obj().cloned()).ok_or_else(|| vm.type_error("__new__(X): X is not a type"))?;
    Ok(vm.types.new_native(&c, Box::new(None::<Stream>)))
}

// Shared IOStream / RWopsIOImpl methods -------------------------------------------

fn m_close(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    if let Some(mut s) = take(&a[0]) {
        let r = s.close(vm);
        put(&a[0], s);
        r?;
    }
    Ok(Value::None)
}

fn m_closed(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(py::native::<Option<Stream>, _>(&a[0], |s| s.as_ref().is_none_or(|s| s.closed)).unwrap_or(true)))
}

fn m_read(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let n = match a.get(1) {
        Some(v) => opt_i64(vm, v)?,
        None => None,
    };
    let b = on(vm, &a[0], |vm, s| s.read(vm, n))?;
    Ok(vm.types.new_bytes(b))
}

fn m_readall(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let b = on(vm, &a[0], |vm, s| s.read(vm, None))?;
    Ok(vm.types.new_bytes(b))
}

fn m_readinto(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let target = a.get(1).cloned().unwrap_or(Value::None);
    let n = ops::len(vm, &target)? as i64;
    let b = on(vm, &a[0], |vm, s| s.read(vm, Some(n)))?;
    let l = b.len();
    let data = vm.types.new_bytes(b);
    let sl = vm.call(&Value::Obj(vm.types.slice.clone()), vec![Value::Int(0), Value::Int(l as i64)], Vec::new())?;
    ops::setitem(vm, &target, sl, data)?;
    Ok(Value::Int(l as i64))
}

fn m_readline(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let limit = match a.get(1) {
        Some(v) => opt_i64(vm, v)?,
        None => None,
    };
    let mut out = Vec::new();
    on(vm, &a[0], |vm, s| {
        while limit.is_none_or(|l| (out.len() as i64) < l) {
            let c = s.read(vm, Some(1))?;
            if c.is_empty() {
                break;
            }
            out.push(c[0]);
            if c[0] == b'\n' {
                break;
            }
        }
        Ok(())
    })?;
    Ok(vm.types.new_bytes(out))
}

fn m_readlines(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let mut lines = Vec::new();
    loop {
        let l = m_readline(vm, &a[..1], &[])?;
        if ops::len(vm, &l)? == 0 {
            break;
        }
        lines.push(l);
    }
    Ok(vm.types.new_list(lines))
}

fn m_write(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let data = a.get(1).cloned().unwrap_or(Value::None);
    let b = ops::bytes_of(&data).map(|b| b.to_vec()).ok_or_else(|| vm.type_error("a bytes-like object is required"))?;
    Ok(Value::Int(on(vm, &a[0], |vm, s| s.write(vm, &b))?))
}

fn m_writelines(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    for l in py::items(vm, a.get(1).unwrap_or(&Value::None))? {
        m_write(vm, &[a[0].clone(), l], &[])?;
    }
    Ok(Value::None)
}

fn m_seek(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let off = arg_or(&a[1..], kw, 0, "offset", Value::Int(0));
    let whence = arg_or(&a[1..], kw, 1, "whence", Value::Int(0));
    let (off, whence) = (py::int(vm, &off)?, py::int(vm, &whence)?);
    Ok(Value::Int(on(vm, &a[0], |vm, s| s.seek(vm, off, whence))?))
}

fn m_tell(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(on(vm, &a[0], |_, s| Ok(s.pos))?))
}

fn m_readable(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(on(vm, &a[0], |_, s| Ok(s.readable()))?))
}

fn m_writable(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(on(vm, &a[0], |_, s| Ok(s.writable()))?))
}

fn m_fileno(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("OSError", "fileno() is not available for this stream"))
}

fn m_truncate(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("OSError", "truncate() is not supported"))
}

fn m_next(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let l = m_readline(vm, &a[..1], &[])?;
    if ops::len(vm, &l)? == 0 {
        return Err(vm.exc_msg("StopIteration", ""));
    }
    Ok(l)
}

fn m_exit(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    m_close(vm, &a[..1], &[])?;
    Ok(Value::None)
}

fn m_repr(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let cls = ops::type_of(vm, &a[0]);
    let cname = py::get(vm, &Value::Obj(cls), "__name__")?;
    let cname = py::string(vm, &cname)?;
    let name = py::get_opt(vm, &a[0], "name")?.unwrap_or(Value::None);
    let name = ops::repr(vm, &name)?;
    Ok(vm.types.new_string(format!("<{cname} {name}>")))
}

const STREAM_METHODS: &[(&str, crysirror_python::object::NativeFn)] = &[
    ("close", m_close),
    ("read", m_read),
    ("readall", m_readall),
    ("readinto", m_readinto),
    ("readline", m_readline),
    ("readlines", m_readlines),
    ("write", m_write),
    ("writelines", m_writelines),
    ("seek", m_seek),
    ("tell", m_tell),
    ("readable", m_readable),
    ("writable", m_writable),
    ("seekable", |_, _, _| Ok(Value::Bool(true))),
    ("flush", |_, _, _| Ok(Value::None)),
    ("isatty", |_, _, _| Ok(Value::Bool(false))),
    ("fileno", m_fileno),
    ("truncate", m_truncate),
    ("__iter__", |_, a, _| Ok(a[0].clone())),
    ("__next__", m_next),
    ("__enter__", |_, a, _| Ok(a[0].clone())),
    ("__exit__", m_exit),
    ("__repr__", m_repr),
];

fn stream_class(
    vm: &mut Vm,
    module: &str,
    name: &str,
    bases: &[Value],
    init: crysirror_python::object::NativeFn,
) -> PyResult<ObjRef> {
    let mut methods = if bases.is_empty() { STREAM_METHODS.to_vec() } else { Vec::new() };
    methods.push(("__init__", init));
    let mut attrs = Vec::new();
    if bases.is_empty() {
        let new = py::builtin(vm, "__new__", new_native);
        attrs.push(("__new__", py::staticmethod(vm, new)));
        let g = py::builtin(vm, "closed", m_closed);
        attrs.push(("closed", py::property(vm, g, None)?));
    }
    py::class(vm, module, name, bases, &methods, &attrs)
}

// iostream (8.6) ------------------------------------------------------------------

fn io_base_init(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    put(&a[0], Stream { kind: Kind::Buffer { data: Vec::new(), writable: false }, pos: 0, closed: false });
    Ok(Value::None)
}

fn io_path_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let path = arg_or(&a[1..], kw, 0, "path", Value::None);
    let mode = arg_or(&a[1..], kw, 1, "mode", vm.types.new_str("rb"));
    let name = arg_or(&a[1..], kw, 2, "name", Value::None);
    let mode = py::string(vm, &mode)?;
    let s = stream_of(vm, &path, &mode, None, None)?;
    put(&a[0], s);
    let n = name_of(vm, &path, &name)?;
    py::set(vm, &a[0], "name", n)?;
    Ok(Value::None)
}

fn io_subfile_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let path = arg_or(&a[1..], kw, 0, "path", Value::None);
    let base = arg_or(&a[1..], kw, 1, "base", Value::Int(0));
    let length = arg_or(&a[1..], kw, 2, "length", Value::Int(0));
    let name = arg_or(&a[1..], kw, 3, "name", Value::None);
    let (b, l) = (py::int(vm, &base)?, py::int(vm, &length)?);
    let s = stream_of(vm, &path, "rb", Some(b), Some(l))?;
    put(&a[0], s);
    let n = name_of(vm, &path, &name)?;
    py::set(vm, &a[0], "name", n)?;
    Ok(Value::None)
}

fn io_buffer_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let buf = arg_or(&a[1..], kw, 0, "buffer", Value::None);
    let writable = arg_or(&a[1..], kw, 1, "writable", Value::Bool(false));
    let name = arg_or(&a[1..], kw, 2, "name", Value::None);
    let data = ops::bytes_of(&buf)
        .map(|b| b.to_vec())
        .ok_or_else(|| vm.value_error("Passed in object does not support buffer protocol"))?;
    let writable = ops::truthy(vm, &writable)?;
    put(&a[0], Stream { kind: Kind::Buffer { data, writable }, pos: 0, closed: false });
    py::set(vm, &a[0], "name", name)?;
    Ok(Value::None)
}

fn io_filelike_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(&a[1..], kw, 0, "filelike", Value::None);
    let close = arg_or(&a[1..], kw, 1, "close", Value::Bool(false));
    let name = arg_or(&a[1..], kw, 2, "name", Value::None);
    let close = ops::truthy(vm, &close)?;
    let writable = match py::get_opt(vm, &f, "writable")? {
        Some(w) => {
            let r = vm.call(&w, Vec::new(), Vec::new())?;
            ops::truthy(vm, &r)?
        }
        None => false,
    };
    let n = name_of(vm, &f, &name)?;
    put(&a[0], Stream { kind: Kind::File { f, base: 0, length: None, close, writable }, pos: 0, closed: false });
    py::set(vm, &a[0], "name", n)?;
    Ok(Value::None)
}

/// `open_io(obj, mode="rb", name=None)`.
fn f_open_io(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let mut obj = arg_or(a, kw, 0, "obj", Value::None);
    let mode = arg_or(a, kw, 1, "mode", vm.types.new_str("rb"));
    let name = arg_or(a, kw, 2, "name", Value::None);
    let m = py::string(vm, &mode)?;
    if m != "rb" && m != "wb" {
        let r = ops::repr(vm, &mode)?;
        return Err(vm.value_error(format!("invalid mode: {r}")));
    }
    while let Some(raw) = py::get_opt(vm, &obj, "raw")? {
        obj = raw;
    }
    let base = py::import_attr(vm, &pg::modname(vm, "iostream"), "IOStream")?;
    if let Some(b) = base.obj() {
        if ops::isinstance(vm, &obj, b) {
            return Ok(obj);
        }
    }
    let kw: Vec<(std::rc::Rc<str>, Value)> = vec![("name".into(), name)];
    if obj.as_str().is_some() {
        let c = py::import_attr(vm, &pg::modname(vm, "iostream"), "IOPath")?;
        return vm.call(&c, vec![obj, mode], kw);
    }
    if ops::bytes_of(&obj).is_some() {
        let c = py::import_attr(vm, &pg::modname(vm, "iostream"), "IOBuffer")?;
        return vm.call(&c, vec![obj, Value::Bool(m == "wb")], kw);
    }
    let c = py::import_attr(vm, &pg::modname(vm, "iostream"), "IOFileLike")?;
    vm.call(&c, vec![obj, Value::Bool(false)], kw)
}

pub fn init_iostream(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "iostream", &[("open_io", f_open_io)]);
    let mn = pg::modname(vm, "iostream");
    let base = stream_class(vm, &mn, "IOStream", &[], io_base_init)?;
    set(vm, &m, "IOStream", Value::Obj(base.clone()));
    let b = [Value::Obj(base)];
    for (n, init) in [
        ("IOPath", io_path_init as crysirror_python::object::NativeFn),
        ("IOSubFile", io_subfile_init),
        ("IOBuffer", io_buffer_init),
        ("IOFileLike", io_filelike_init),
    ] {
        let c = stream_class(vm, &mn, n, &b, init)?;
        set(vm, &m, n, Value::Obj(c));
    }
    set(vm, &m, "IOPATH_DEFAULT_BUFFERING", Value::Int(0));
    Ok(m)
}

// rwobject (pygame_sdl2, 8.5) -------------------------------------------------------

fn rw_impl_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(&a[1..], kw, 0, "filelike", Value::None);
    let mode = arg_or(&a[1..], kw, 1, "mode", vm.types.new_str("rb"));
    let base = arg_or(&a[1..], kw, 2, "base", Value::None);
    let length = arg_or(&a[1..], kw, 3, "length", Value::None);
    let name = arg_or(&a[1..], kw, 4, "name", Value::None);
    let n = name_of(vm, &f, &name)?;
    py::set(vm, &a[0], "name", n)?;
    py::set(vm, &a[0], "base", base.clone())?;
    py::set(vm, &a[0], "length", length.clone())?;
    if !py::is_none(&f) {
        let mode = py::string(vm, &mode)?;
        let (b, l) = (opt_i64(vm, &base)?, opt_i64(vm, &length)?);
        let s = stream_of(vm, &f, &mode, b, l)?;
        put(&a[0], s);
    }
    Ok(Value::None)
}

fn rw_is_closed(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    m_closed(vm, a, &[])
}

fn rw_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(&a[1..], kw, 0, "filelike", Value::None);
    let mode = arg_or(&a[1..], kw, 1, "mode", vm.types.new_str("rb"));
    let base = arg_or(&a[1..], kw, 2, "base", Value::None);
    let length = arg_or(&a[1..], kw, 3, "length", Value::None);
    let name = arg_or(&a[1..], kw, 4, "name", Value::None);
    let c = py::import_attr(vm, &pg::modname(vm, "rwobject"), "RWopsIOImpl")?;
    let raw = vm.call(
        &c,
        vec![f],
        vec![
            ("mode".into(), mode),
            ("base".into(), base),
            ("length".into(), length),
            ("name".into(), name),
        ],
    )?;
    py::set(vm, &a[0], "raw", raw.clone())?;
    for n in ["close", "seek", "tell", "write", "readinto"] {
        let f = py::get(vm, &raw, n)?;
        py::set(vm, &a[0], n, f)?;
    }
    Ok(Value::None)
}

fn rw_raw(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    py::get(vm, this, "raw")
}

fn rw_repr(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let raw = rw_raw(vm, &a[0])?;
    let name = py::get(vm, &raw, "name")?;
    let base = py::get(vm, &raw, "base")?;
    let name = ops::repr(vm, &name)?;
    if py::is_none(&base) {
        return Ok(vm.types.new_string(format!("<RWopsIO {name}>")));
    }
    let length = py::get(vm, &raw, "length")?;
    let (b, l) = (ops::repr(vm, &base)?, ops::repr(vm, &length)?);
    Ok(vm.types.new_string(format!("<RWopsIO {name} base={b} length={l}>")))
}

fn rw_delegate(vm: &mut Vm, a: &[Value], name: &str) -> PyResult<Value> {
    let raw = rw_raw(vm, &a[0])?;
    let mut args = vec![raw];
    args.extend_from_slice(&a[1..]);
    match name {
        "read" => m_read(vm, &args, &[]),
        "readall" => m_readall(vm, &args, &[]),
        "readline" => m_readline(vm, &args, &[]),
        "readlines" => m_readlines(vm, &args, &[]),
        "writelines" => m_writelines(vm, &args, &[]),
        "__next__" => m_next(vm, &args, &[]),
        _ => unreachable!(),
    }
}

fn rw_closed(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let raw = rw_raw(vm, &a[0])?;
    m_closed(vm, &[raw], &[])
}

fn rw_wrap(vm: &mut Vm, s: Stream, name: Value) -> PyResult<Value> {
    let c = py::import_attr(vm, &pg::modname(vm, "rwobject"), "RWopsIO")?;
    let rv = vm.call(&c, vec![Value::None], vec![("name".into(), name)])?;
    let raw = rw_raw(vm, &rv)?;
    put(&raw, s);
    Ok(rv)
}

fn rw_from_buffer(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let buf = arg_or(a, kw, 0, "buffer", Value::None);
    let name = arg_or(a, kw, 2, "name", Value::None);
    let data = ops::bytes_of(&buf)
        .map(|b| b.to_vec())
        .ok_or_else(|| vm.value_error("Passed in object does not support buffer protocol"))?;
    rw_wrap(vm, Stream { kind: Kind::Buffer { data, writable: false }, pos: 0, closed: false }, name)
}

fn rw_from_split(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let av = arg_or(a, kw, 0, "a", Value::None);
    let bv = arg_or(a, kw, 1, "b", Value::None);
    let name = arg_or(a, kw, 2, "name", Value::None);
    let sa = stream_of(vm, &av, "rb", None, None)?;
    let sb = stream_of(vm, &bv, "rb", None, None)?;
    rw_wrap(vm, Stream { kind: Kind::Split { a: Box::new(sa), b: Box::new(sb) }, pos: 0, closed: false }, name)
}

pub fn init_rwobject(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "rwobject", &[]);
    let mn = pg::modname(vm, "rwobject");
    let new = py::builtin(vm, "__new__", new_native);
    let new = py::staticmethod(vm, new);
    let impl_c = py::class(
        vm,
        &mn,
        "RWopsIOImpl",
        &[],
        &[
            ("__init__", rw_impl_init),
            ("close", m_close),
            ("is_closed", m_closed),
            ("seek", m_seek),
            ("tell", m_tell),
            ("readinto", m_readinto),
            ("write", m_write),
        ],
        &[("__new__", new)],
    )?;
    set(vm, &m, "RWopsIOImpl", Value::Obj(impl_c));
    let raw_base = py::import_attr(vm, "io", "RawIOBase")?;
    let closed = py::builtin(vm, "closed", rw_closed);
    let closed = py::property(vm, closed, None)?;
    let fb = py::builtin(vm, "from_buffer", rw_from_buffer);
    let fs = py::builtin(vm, "from_split", rw_from_split);
    let attrs = [
        ("closed", closed),
        ("from_buffer", py::staticmethod(vm, fb)),
        ("from_split", py::staticmethod(vm, fs)),
    ];
    let c = py::class(
        vm,
        &mn,
        "RWopsIO",
        &[raw_base],
        &[
            ("__init__", rw_init),
            ("__repr__", rw_repr),
            ("fileno", m_fileno),
            ("readable", |_, _, _| Ok(Value::Bool(true))),
            ("seekable", |_, _, _| Ok(Value::Bool(true))),
            ("writable", |_, _, _| Ok(Value::Bool(true))),
            ("truncate", m_truncate),
            ("read", |vm, a, _| rw_delegate(vm, a, "read")),
            ("readall", |vm, a, _| rw_delegate(vm, a, "readall")),
            ("readline", |vm, a, _| rw_delegate(vm, a, "readline")),
            ("readlines", |vm, a, _| rw_delegate(vm, a, "readlines")),
            ("writelines", |vm, a, _| rw_delegate(vm, a, "writelines")),
            ("__next__", |vm, a, _| rw_delegate(vm, a, "__next__")),
            ("__iter__", |_, a, _| Ok(a[0].clone())),
        ],
        &attrs,
    )?;
    set(vm, &m, "RWopsIO", Value::Obj(c));
    let wm = vm.types.new_dict();
    for i in 0..3 {
        ops::setitem(vm, &wm, Value::Int(i), Value::Int(i))?;
    }
    set(vm, &m, "whence_mapping", wm);
    for n in ["file_type", "bytes_", "unicode_"] {
        let v = pg::attr(vm, "compat", n)?;
        set(vm, &m, n, v);
    }
    let enc = vm.types.new_str("utf-8");
    set(vm, &m, "fsencoding", enc);
    Ok(m)
}
