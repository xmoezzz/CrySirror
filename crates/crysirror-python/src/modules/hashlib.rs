// SPDX-License-Identifier: MPL-2.0
//! `_hashlib`: md5, sha1, sha224/256/384/512 objects (`update`, `digest`,
//! `hexdigest`, `copy`) and `new(name, data)`.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use std::cell::RefCell;

pub fn sha512(data: &[u8]) -> Vec<u8> {
    Sha512::digest(data).to_vec()
}

#[derive(Clone)]
enum H {
    Md5(Md5),
    Sha1(Sha1),
    Sha224(Sha224),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}

impl H {
    fn new(name: &str) -> Option<H> {
        Some(match name.to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => H::Md5(Md5::new()),
            "sha1" => H::Sha1(Sha1::new()),
            "sha224" => H::Sha224(Sha224::new()),
            "sha256" => H::Sha256(Sha256::new()),
            "sha384" => H::Sha384(Sha384::new()),
            "sha512" => H::Sha512(Sha512::new()),
            _ => return None,
        })
    }
    fn update(&mut self, d: &[u8]) {
        match self {
            H::Md5(h) => h.update(d),
            H::Sha1(h) => h.update(d),
            H::Sha224(h) => h.update(d),
            H::Sha256(h) => h.update(d),
            H::Sha384(h) => h.update(d),
            H::Sha512(h) => h.update(d),
        }
    }
    fn digest(&self) -> Vec<u8> {
        match self.clone() {
            H::Md5(h) => h.finalize().to_vec(),
            H::Sha1(h) => h.finalize().to_vec(),
            H::Sha224(h) => h.finalize().to_vec(),
            H::Sha256(h) => h.finalize().to_vec(),
            H::Sha384(h) => h.finalize().to_vec(),
            H::Sha512(h) => h.finalize().to_vec(),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            H::Md5(_) => "md5",
            H::Sha1(_) => "sha1",
            H::Sha224(_) => "sha224",
            H::Sha256(_) => "sha256",
            H::Sha384(_) => "sha384",
            H::Sha512(_) => "sha512",
        }
    }
    fn sizes(&self) -> (i64, i64) {
        match self {
            H::Md5(_) => (16, 64),
            H::Sha1(_) => (20, 64),
            H::Sha224(_) => (28, 64),
            H::Sha256(_) => (32, 64),
            H::Sha384(_) => (48, 128),
            H::Sha512(_) => (64, 128),
        }
    }
}

fn hash_class(vm: &mut Vm) -> ObjRef {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "_hashlib") {
        if let Some(Value::Obj(c)) = m.dict_get_str_raw("HASH") {
            return c;
        }
    }
    vm.types.native.clone()
}

fn make(vm: &mut Vm, h: H, data: Option<&Value>) -> PyResult<Value> {
    let mut h = h;
    if let Some(d) = data {
        let b = ops::bytes_of(d).ok_or_else(|| vm.type_error("Strings must be encoded before hashing"))?;
        h.update(&b);
    }
    let cls = hash_class(vm);
    let (ds, bs) = h.sizes();
    let name = h.name();
    let o = new_obj(cls, Some(Dict::new()), Payload::Native(RefCell::new(Box::new(h))));
    let nv = vm.types.new_str(name);
    ops::set_raw_attr(vm, &o, "name", nv);
    ops::set_raw_attr(vm, &o, "digest_size", Value::Int(ds));
    ops::set_raw_attr(vm, &o, "block_size", Value::Int(bs));
    Ok(Value::Obj(o))
}

fn with_h<R>(vm: &Vm, v: &Value, f: impl FnOnce(&mut H) -> R) -> PyResult<R> {
    match v.payload() {
        Some(Payload::Native(n)) => n.borrow_mut().downcast_mut::<H>().map(f).ok_or_else(|| vm.type_error("hash object expected")),
        _ => Err(vm.type_error("hash object expected")),
    }
}

fn data_arg(a: &[Value], k: Kw) -> Option<Value> {
    a.first().cloned().or_else(|| k.iter().find(|(n, _)| &**n == "data" || &**n == "string").map(|(_, v)| v.clone()))
}

macro_rules! ctor {
    ($($fn:ident => $name:expr),*) => {
        $(
            fn $fn(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
                let d = data_arg(a, k);
                make(vm, H::new($name).unwrap(), d.as_ref())
            }
        )*
    };
}
ctor!(md5 => "md5", sha1 => "sha1", sha224 => "sha224", sha256 => "sha256", sha384 => "sha384", sha512_f => "sha512");

fn new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let name = crate::builtins::str_arg(vm, &a[0], "name")?;
    let h = H::new(&name).ok_or_else(|| vm.value_error(format!("unsupported hash type {name}")))?;
    let d = data_arg(&a[1..], k);
    make(vm, h, d.as_ref())
}

fn update(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = ops::bytes_of(&a[1]).ok_or_else(|| vm.type_error("Strings must be encoded before hashing"))?;
    with_h(vm, &a[0], |h| h.update(&b))?;
    Ok(Value::None)
}

fn digest(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = with_h(vm, &a[0], |h| h.digest())?;
    Ok(vm.types.new_bytes(d))
}

fn hexdigest(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = with_h(vm, &a[0], |h| h.digest())?;
    let s: String = d.iter().map(|b| format!("{b:02x}")).collect();
    Ok(vm.types.new_string(s))
}

fn copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let h = with_h(vm, &a[0], |h| h.clone())?;
    make(vm, h, None)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_hashlib", &[("new", new), ("md5", md5), ("sha1", sha1), ("sha224", sha224), ("sha256", sha256), ("sha384", sha384), ("sha512", sha512_f)]);
    let c = vm.types.new_builtin_type("HASH", &vm.types.object.clone(), BaseKind::Native, false);
    for (n, f) in [("update", update as NativeFn), ("digest", digest), ("hexdigest", hexdigest), ("copy", copy)] {
        vm.types.add_method(&c, n, f);
    }
    set(vm, &m, "HASH", Value::Obj(c));
    let algs: Vec<Value> = ["md5", "sha1", "sha224", "sha256", "sha384", "sha512"].iter().map(|s| vm.types.new_str(s)).collect();
    let t = vm.types.new_tuple(algs);
    set(vm, &m, "algorithms_guaranteed", t.clone());
    set(vm, &m, "algorithms_available", t);
    Ok(m)
}
