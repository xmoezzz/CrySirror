// SPDX-License-Identifier: MPL-2.0
//! `zlib`: deflate/inflate (zlib, raw and gzip wrappers by `wbits`),
//! streaming objects, crc32 and adler32.

use super::{module_with, set};
use crate::builtins::{parse, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use std::cell::RefCell;
use std::io::Read;

fn bytes_arg(vm: &Vm, v: &Value) -> PyResult<Vec<u8>> {
    ops::bytes_of(v).ok_or_else(|| vm.type_error(format!("a bytes-like object is required, not '{}'", ops::type_name(vm, v))))
}

fn error(vm: &Vm, msg: impl Into<String>) -> crate::vm::PyErr {
    let cls = match crate::import::get_module(vm, "zlib").and_then(|m| m.obj().and_then(|o| o.dict_get_str_raw("error"))) {
        Some(Value::Obj(c)) => c,
        _ => vm.types.exc("Exception"),
    };
    let m = vm.types.new_string(msg.into());
    crate::vm::PyErr::Exc(vm.new_exc(&cls, vec![m]))
}

pub fn inflate(data: &[u8], wbits: i64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let r = if wbits < 0 {
        flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)
    } else if wbits > 31 || (16..32).contains(&wbits) && wbits >= 24 {
        flate2::read::GzDecoder::new(data).read_to_end(&mut out)
    } else {
        flate2::read::ZlibDecoder::new(data).read_to_end(&mut out)
    };
    r.map(|_| out).map_err(|e| format!("Error -3 while decompressing data: {e}"))
}

pub fn deflate(data: &[u8], level: i64, wbits: i64) -> Vec<u8> {
    let lvl = if level < 0 { Compression::default() } else { Compression::new(level.clamp(0, 9) as u32) };
    use std::io::Write;
    if wbits < 0 {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), lvl);
        let _ = e.write_all(data);
        e.finish().unwrap_or_default()
    } else if wbits >= 24 {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), lvl);
        let _ = e.write_all(data);
        e.finish().unwrap_or_default()
    } else {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), lvl);
        let _ = e.write_all(data);
        e.finish().unwrap_or_default()
    }
}

fn compress(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [data, level, wbits] = parse(vm, "compress", a, k, ["data", "level", "wbits"], 1)?;
    let d = bytes_arg(vm, data.as_ref().unwrap())?;
    let level = crate::builtins::opt_i64(vm, &level, -1)?;
    let wbits = crate::builtins::opt_i64(vm, &wbits, 15)?;
    Ok(vm.types.new_bytes(deflate(&d, level, wbits)))
}

fn decompress(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [data, wbits, _bufsize] = parse(vm, "decompress", a, k, ["data", "wbits", "bufsize"], 1)?;
    let d = bytes_arg(vm, data.as_ref().unwrap())?;
    let wbits = crate::builtins::opt_i64(vm, &wbits, 15)?;
    let out = inflate(&d, wbits).map_err(|e| error(vm, e))?;
    Ok(vm.types.new_bytes(out))
}

pub fn crc32_of(data: &[u8], start: u32) -> u32 {
    let mut c = !start;
    for b in data {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn crc32(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &a[0])?;
    let start = match a.get(1) {
        Some(v) => ops::index_of(vm, v)? as u32,
        None => 0,
    };
    let c = crc32_of(&d, start);
    Ok(Value::Int(if crate::ops::vm_dialect(vm) == crate::Dialect::Py2 { c as i32 as i64 } else { c as i64 }))
}

fn adler32(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &a[0])?;
    let start = match a.get(1) {
        Some(v) => ops::index_of(vm, v)? as u32,
        None => 1,
    };
    let (mut s1, mut s2) = (start & 0xffff, start >> 16);
    for b in d {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    Ok(Value::Int(((s2 << 16) | s1) as i64))
}

enum Stream {
    C(Compress),
    D(Decompress, Vec<u8>, bool),
}

fn stream_class(vm: &mut Vm, name: &str) -> ObjRef {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "zlib") {
        if let Some(Value::Obj(c)) = m.dict_get_str_raw(name) {
            return c;
        }
    }
    vm.types.native.clone()
}

fn compressobj(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [level, _method, wbits] = parse(vm, "compressobj", a, k, ["level", "method", "wbits"], 0)?;
    let level = crate::builtins::opt_i64(vm, &level, 6)?;
    let wbits = crate::builtins::opt_i64(vm, &wbits, 15)?;
    let lvl = if level < 0 { Compression::default() } else { Compression::new(level.clamp(0, 9) as u32) };
    let c = Compress::new(lvl, wbits >= 0);
    let cls = stream_class(vm, "Compress");
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::Native(RefCell::new(Box::new(Stream::C(c)))))))
}

fn decompressobj(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [wbits] = parse(vm, "decompressobj", a, k, ["wbits"], 0)?;
    let wbits = crate::builtins::opt_i64(vm, &wbits, 15)?;
    let d = Decompress::new(wbits >= 0);
    let cls = stream_class(vm, "Decompress");
    let o = new_obj(cls, Some(Dict::new()), Payload::Native(RefCell::new(Box::new(Stream::D(d, Vec::new(), false)))));
    let e = vm.types.new_bytes(Vec::new());
    ops::set_raw_attr(vm, &o, "unused_data", e.clone());
    ops::set_raw_attr(vm, &o, "unconsumed_tail", e);
    ops::set_raw_attr(vm, &o, "eof", Value::Bool(false));
    Ok(Value::Obj(o))
}

fn run_stream(vm: &mut Vm, obj: &Value, input: &[u8], finish: bool) -> PyResult<Vec<u8>> {
    let Some(Payload::Native(n)) = obj.payload() else { return Err(vm.type_error("stream expected")) };
    let mut b = n.borrow_mut();
    let Some(s) = b.downcast_mut::<Stream>() else { return Err(vm.type_error("stream expected")) };
    let mut out = Vec::with_capacity(input.len() * 2 + 64);
    match s {
        Stream::C(c) => {
            let mut inp = input;
            loop {
                let before_in = c.total_in();
                out.reserve(4096);
                let st = c.compress_vec(inp, &mut out, if finish { FlushCompress::Finish } else { FlushCompress::None }).map_err(|e| error(vm, e.to_string()))?;
                let used = (c.total_in() - before_in) as usize;
                inp = &inp[used..];
                if matches!(st, Status::StreamEnd) || (inp.is_empty() && !finish) {
                    break;
                }
                if inp.is_empty() && finish && out.capacity() > out.len() {
                    continue;
                }
            }
        }
        Stream::D(d, _, eof) => {
            let mut inp = input;
            loop {
                let before_in = d.total_in();
                out.reserve(input.len() * 4 + 4096);
                let st = d.decompress_vec(inp, &mut out, if finish { FlushDecompress::Finish } else { FlushDecompress::None }).map_err(|e| error(vm, format!("Error -3 while decompressing data: {e}")))?;
                let used = (d.total_in() - before_in) as usize;
                inp = &inp[used..];
                if matches!(st, Status::StreamEnd) {
                    *eof = true;
                    break;
                }
                if inp.is_empty() || (used == 0 && out.len() < out.capacity()) {
                    break;
                }
            }
        }
    }
    Ok(out)
}

fn s_compress(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &a[1])?;
    let r = run_stream(vm, &a[0], &d, false)?;
    Ok(vm.types.new_bytes(r))
}

fn s_flush(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let r = run_stream(vm, &a[0], &[], true)?;
    Ok(vm.types.new_bytes(r))
}

fn s_decompress(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = bytes_arg(vm, &a[1])?;
    let r = run_stream(vm, &a[0], &d, false)?;
    let eof = match a[0].payload() {
        Some(Payload::Native(n)) => matches!(n.borrow().downcast_ref::<Stream>(), Some(Stream::D(_, _, true))),
        _ => false,
    };
    if let Value::Obj(o) = &a[0] {
        ops::set_raw_attr(vm, o, "eof", Value::Bool(eof));
    }
    Ok(vm.types.new_bytes(r))
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "zlib", &[("compress", compress), ("decompress", decompress), ("crc32", crc32), ("adler32", adler32), ("compressobj", compressobj), ("decompressobj", decompressobj)]);
    let err = vm.types.new_builtin_type("error", &vm.types.exc("Exception"), BaseKind::Exception, false);
    set(vm, &m, "error", Value::Obj(err));
    let c = vm.types.new_builtin_type("Compress", &vm.types.object.clone(), BaseKind::Native, false);
    vm.types.add_method(&c, "compress", s_compress);
    vm.types.add_method(&c, "flush", s_flush);
    set(vm, &m, "Compress", Value::Obj(c));
    let d = vm.types.new_builtin_type("Decompress", &vm.types.object.clone(), BaseKind::Native, false);
    vm.types.add_method(&d, "decompress", s_decompress);
    vm.types.add_method(&d, "flush", s_flush);
    set(vm, &m, "Decompress", Value::Obj(d));
    for (n, v) in [("MAX_WBITS", 15), ("DEFLATED", 8), ("DEF_MEM_LEVEL", 8), ("Z_BEST_COMPRESSION", 9), ("Z_BEST_SPEED", 1), ("Z_DEFAULT_COMPRESSION", -1), ("Z_FINISH", 4), ("Z_NO_FLUSH", 0), ("Z_SYNC_FLUSH", 2), ("Z_FULL_FLUSH", 3)] {
        set(vm, &m, n, Value::Int(v));
    }
    let v = vm.types.new_str("1.2.13");
    set(vm, &m, "ZLIB_VERSION", v);
    Ok(m)
}
