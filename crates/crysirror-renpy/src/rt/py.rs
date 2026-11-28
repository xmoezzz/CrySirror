// SPDX-License-Identifier: MPL-2.0
//! Small helpers shared by the native Ren'Py runtime modules.
use crysirror_python::{
    ObjRef, Payload, PyResult, Value, Vm,
    builtins::Kw,
    object::{NativeFn, new_obj},
    ops,
};
use std::rc::Rc;

pub type Methods<'a> = &'a [(&'static str, NativeFn)];

/// Keyword lookup.
pub fn kw<'a>(kw: Kw<'a>, name: &str) -> Option<&'a Value> {
    kw.iter().find(|(k, _)| &**k == name).map(|(_, v)| v)
}

/// Positional `i` or keyword `name`.
pub fn arg<'a>(args: &'a [Value], kwargs: Kw<'a>, i: usize, name: &str) -> Option<&'a Value> {
    args.get(i).or_else(|| kw(kwargs, name))
}

pub fn arg_or(args: &[Value], kwargs: Kw, i: usize, name: &str, default: Value) -> Value {
    arg(args, kwargs, i, name).cloned().unwrap_or(default)
}

pub fn f64_of(vm: &mut Vm, v: &Value) -> PyResult<f64> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        Value::Bool(b) => Ok(*b as i64 as f64),
        _ => ops::to_f64(vm, v),
    }
}

pub fn opt_f64(vm: &mut Vm, v: Option<&Value>) -> PyResult<Option<f64>> {
    match v {
        None | Some(Value::None) => Ok(None),
        Some(v) => f64_of(vm, v).map(Some),
    }
}

pub fn get(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Value> {
    ops::getattr(vm, obj, name)
}

pub fn get_opt(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Option<Value>> {
    ops::getattr_opt(vm, obj, name)
}

/// Attribute or `None` when missing.
pub fn get_or_none(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Value> {
    Ok(ops::getattr_opt(vm, obj, name)?.unwrap_or(Value::None))
}

pub fn set(vm: &mut Vm, obj: &Value, name: &str, v: Value) -> PyResult<()> {
    let n = vm.types.intern(name);
    ops::setattr(vm, obj, &n, v)
}

pub fn truthy(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    ops::truthy(vm, v)
}

pub fn s(vm: &Vm, v: &str) -> Value {
    vm.types.new_str(v)
}

pub fn tuple(vm: &Vm, items: Vec<Value>) -> Value {
    vm.types.new_tuple(items)
}

pub fn list(vm: &Vm, items: Vec<Value>) -> Value {
    vm.types.new_list(items)
}

pub fn f(x: f64) -> Value {
    Value::Float(x)
}

/// Items of a tuple or list without calling Python.
pub fn seq(v: &Value) -> Option<Vec<Value>> {
    match v.payload()? {
        Payload::Tuple(t) => Some(t.to_vec()),
        Payload::List(l) => Some(l.borrow().clone()),
        _ => None,
    }
}

pub fn items(vm: &mut Vm, v: &Value) -> PyResult<Vec<Value>> {
    match seq(v) {
        Some(v) => Ok(v),
        None => ops::collect(vm, v),
    }
}

pub fn call(vm: &mut Vm, f: &Value, args: Vec<Value>) -> PyResult<Value> {
    vm.call(f, args, Vec::new())
}

pub fn call_kw(
    vm: &mut Vm,
    f: &Value,
    args: Vec<Value>,
    kwargs: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    vm.call(f, args, kwargs)
}

pub fn method(vm: &mut Vm, obj: &Value, name: &str, args: Vec<Value>) -> PyResult<Value> {
    vm.call_method(obj, name, args)
}

pub fn dict(vm: &Vm) -> Value {
    vm.types.new_dict()
}

pub fn dict_set(vm: &mut Vm, d: &Value, k: &str, v: Value) -> PyResult<()> {
    let key = vm.types.new_str(k);
    ops::setitem(vm, d, key, v)
}

pub fn kwargs_dict(vm: &mut Vm, kwargs: Kw) -> PyResult<Value> {
    let d = vm.types.new_dict();
    for (k, v) in kwargs {
        let key = vm.types.new_str(k);
        ops::setitem(vm, &d, key, v.clone())?;
    }
    Ok(d)
}

/// `(key, value)` pairs of a mapping with string keys.
pub fn str_items(vm: &mut Vm, d: &Value) -> PyResult<Vec<(Rc<str>, Value)>> {
    let mut out = Vec::new();
    for (k, v) in ops::mapping_items(vm, d)? {
        let k = ops::str(vm, &k)?;
        out.push((Rc::from(k.as_str()), v));
    }
    Ok(out)
}

pub fn builtin(vm: &Vm, name: &'static str, f: NativeFn) -> Value {
    vm.types.new_builtin(name, f)
}

/// A native closure carrying `data` (`Builtin.bound`): the callee receives
/// `data` as its first argument.
pub fn closure(vm: &Vm, name: &'static str, f: NativeFn, data: Value) -> Value {
    vm.types.new_builtin_data(name, f, data)
}

pub fn staticmethod(vm: &Vm, f: Value) -> Value {
    Value::Obj(new_obj(
        vm.types.staticmethod.clone(),
        Some(Default::default()),
        Payload::StaticMethod(f),
    ))
}

pub fn classmethod(vm: &Vm, f: Value) -> Value {
    Value::Obj(new_obj(
        vm.types.classmethod.clone(),
        Some(Default::default()),
        Payload::ClassMethod(f),
    ))
}

pub fn property(vm: &mut Vm, get: Value, set: Option<Value>) -> PyResult<Value> {
    let prop = Value::Obj(vm.types.property.clone());
    let mut args = vec![get];
    if let Some(s) = set {
        args.push(s);
    }
    vm.call(&prop, args, Vec::new())
}

/// Builds an ordinary (dict-backed, subclassable, picklable) Python class
/// whose methods are natives, as `type(name, bases, ns)` would.
pub fn class(
    vm: &mut Vm,
    module: &str,
    name: &str,
    bases: &[Value],
    methods: Methods,
    attrs: &[(&str, Value)],
) -> PyResult<ObjRef> {
    let ns = vm.types.new_dict();
    for (n, f) in methods {
        let v = vm.types.new_builtin(n, *f);
        dict_set(vm, &ns, n, v)?;
    }
    for (n, v) in attrs {
        dict_set(vm, &ns, n, v.clone())?;
    }
    let m = vm.types.new_str(module);
    dict_set(vm, &ns, "__module__", m)?;
    let q = vm.types.new_str(name);
    dict_set(vm, &ns, "__qualname__", q)?;
    let bases = if bases.is_empty() {
        vec![Value::Obj(vm.types.object.clone())]
    } else {
        bases.to_vec()
    };
    let ty = Value::Obj(vm.types.type_.clone());
    let n = vm.types.new_str(name);
    let b = vm.types.new_tuple(bases);
    let c = vm.call(&ty, vec![n, b, ns], Vec::new())?;
    c.obj()
        .cloned()
        .ok_or_else(|| vm.type_error("class construction failed"))
}

/// Native-payload access.
pub fn native<T: 'static, R>(v: &Value, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    match v.payload()? {
        Payload::Native(n) => n.borrow_mut().downcast_mut::<T>().map(f),
        _ => None,
    }
}

pub fn is_native<T: 'static>(v: &Value) -> bool {
    matches!(v.payload(), Some(Payload::Native(n)) if n.borrow().is::<T>())
}

/// `module.name`, importing the module.
pub fn import_attr(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    let m = crysirror_python::import::import_module(vm, module)?;
    ops::getattr(vm, &m, name)
}

pub fn import(vm: &mut Vm, module: &str) -> PyResult<Value> {
    crysirror_python::import::import_module(vm, module)
}

/// `renpy.config.<name>`.
pub fn config(vm: &mut Vm, name: &str) -> PyResult<Value> {
    import_attr(vm, "renpy.config", name)
}

pub fn config_f64(vm: &mut Vm, name: &str) -> PyResult<f64> {
    let v = config(vm, name)?;
    f64_of(vm, &v)
}

pub fn config_bool(vm: &mut Vm, name: &str) -> PyResult<bool> {
    let v = config(vm, name)?;
    ops::truthy(vm, &v)
}

pub fn is_none(v: &Value) -> bool {
    matches!(v, Value::None)
}

pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Obj(x), Value::Obj(y)) => Rc::ptr_eq(x, y),
        (Value::None, Value::None) => true,
        _ => false,
    }
}

pub fn int(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(*b as i64),
        Value::Float(f) => Ok(*f as i64),
        _ => Ok(f64_of(vm, v)? as i64),
    }
}

pub fn string(vm: &mut Vm, v: &Value) -> PyResult<String> {
    match v.as_str() {
        Some(s) => Ok(s.to_owned()),
        None => ops::str(vm, v).map(|s| s.to_string()),
    }
}

/// Shorthand for raising `Exception(msg)`.
pub fn error(vm: &Vm, msg: impl Into<String>) -> crysirror_python::PyErr {
    vm.exc_msg("Exception", &msg.into())
}

/// `items(getattr(obj, name))`.
pub fn get_items(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Vec<Value>> {
    let v = get(vm, obj, name)?;
    items(vm, &v)
}

pub fn method_kw(
    vm: &mut Vm,
    obj: &Value,
    name: &str,
    args: Vec<Value>,
    kwargs: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    let f = crysirror_python::ops::getattr(vm, obj, name)?;
    vm.call(&f, args, kwargs)
}

/// `a < b` with Python semantics.
pub fn lt(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    let r = ops::compare(vm, a, b, crysirror_python::ast::CmpOp::Lt)?;
    ops::truthy(vm, &r)
}

/// `a > b` with Python semantics.
pub fn gt(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    let r = ops::compare(vm, a, b, crysirror_python::ast::CmpOp::Gt)?;
    ops::truthy(vm, &r)
}

/// The attribute dict of a module (its namespace) or instance.
pub fn namespace_of(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let Some(o) = v.obj() else {
        return Err(vm.type_error("object has no namespace"));
    };
    if let Payload::Module(_, ns) = &o.payload {
        return Ok(Value::Obj(ns.clone()));
    }
    if o.dict.is_none() {
        return Err(vm.type_error("object has no __dict__"));
    }
    Ok(ops::instance_dict_value(vm, o))
}

/// A module attribute set earlier during that module's own init.
pub fn get_raw(m: &ObjRef, name: &str) -> Value {
    crysirror_python::ops::get_raw_attr(m, name).unwrap_or(Value::None)
}

/// A one-line description of a Python error for diagnostics.
pub fn error_text(vm: &mut Vm, e: &crysirror_python::PyErr) -> String {
    match e {
        crysirror_python::PyErr::Exc(o) => {
            let v = Value::Obj(o.clone());
            let s = ops::str(vm, &v).map(|s| s.to_string()).unwrap_or_default();
            format!("{}: {s}", o.class_name())
        }
        crysirror_python::PyErr::Suspend(_) => "suspended".into(),
    }
}
