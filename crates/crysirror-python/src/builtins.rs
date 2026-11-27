// SPDX-License-Identifier: MPL-2.0
//! The `builtins` namespace, class creation, special attributes of builtin
//! objects and the shared argument helpers used by native methods.

use crate::dict::Dict;
use crate::num::{self, Num};
use crate::object::*;
use crate::ops;
use crate::vm::{Kwargs, PyErr, PyResult, Vm};
use crate::Dialect;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::rc::Rc;

/// Interpreter-created classes beyond the core type graph.
#[derive(Default)]
pub struct Classes {
    pub mappingproxy: Option<ObjRef>,
    pub defaultdict: Option<ObjRef>,
    pub ordereddict: Option<ObjRef>,
    pub deque: Option<ObjRef>,
    pub partial: Option<ObjRef>,
    pub string_io: Option<ObjRef>,
    pub bytes_io: Option<ObjRef>,
    pub stdout: Option<ObjRef>,
    pub pattern: Option<ObjRef>,
    pub match_: Option<ObjRef>,
    pub random: Option<ObjRef>,
    pub counter: Option<ObjRef>,
}

pub type Kw<'a> = &'a [(Rc<str>, Value)];

/// Binds positional/keyword arguments to `names`; names after `required`
/// are optional. A name starting with '*' is not bindable by keyword.
pub fn parse<const N: usize>(vm: &Vm, fname: &str, args: &[Value], kw: Kw, names: [&str; N], required: usize) -> PyResult<[Option<Value>; N]> {
    let mut out: [Option<Value>; N] = std::array::from_fn(|_| None);
    if args.len() > N {
        return Err(vm.type_error(format!("{fname}() takes at most {N} arguments ({} given)", args.len())));
    }
    for (i, a) in args.iter().enumerate() {
        out[i] = Some(a.clone());
    }
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(p) => {
                if out[p].is_some() {
                    return Err(vm.type_error(format!("{fname}() got multiple values for argument '{k}'")));
                }
                out[p] = Some(v.clone());
            }
            None => return Err(vm.type_error(format!("{fname}() got an unexpected keyword argument '{k}'"))),
        }
    }
    for (i, o) in out.iter().enumerate().take(required) {
        if o.is_none() {
            return Err(vm.type_error(format!("{fname}() missing required argument '{}' (pos {})", names[i].trim_start_matches('*'), i + 1)));
        }
    }
    Ok(out)
}

pub fn no_kw(vm: &Vm, fname: &str, kw: Kw) -> PyResult<()> {
    if !kw.is_empty() {
        return Err(vm.type_error(format!("{fname}() takes no keyword arguments")));
    }
    Ok(())
}

pub fn arg_count(vm: &Vm, fname: &str, args: &[Value], min: usize, max: usize) -> PyResult<()> {
    if args.len() < min || args.len() > max {
        return Err(vm.type_error(if min == max {
            format!("{fname}() takes exactly {min} argument{} ({} given)", if min == 1 { "" } else { "s" }, args.len())
        } else {
            format!("{fname}() takes {min} to {max} arguments ({} given)", args.len())
        }));
    }
    Ok(())
}

pub fn str_arg(vm: &Vm, v: &Value, what: &str) -> PyResult<String> {
    v.as_str().map(|s| s.to_string()).ok_or_else(|| vm.type_error(format!("{what} must be str, not {}", ops::type_name(vm, v))))
}

fn add(vm: &mut Vm, name: &'static str, f: NativeFn) {
    let v = vm.types.new_builtin(name, f);
    let k = vm.types.intern(name);
    vm.builtins.dict_set_raw(&vm.types, k, v);
}

fn add_value(vm: &mut Vm, name: &str, v: Value) {
    let k = vm.types.intern(name);
    vm.builtins.dict_set_raw(&vm.types, k, v);
}

pub fn install(vm: &mut Vm) {
    let t = &vm.types;
    let types: Vec<(&str, ObjRef)> = vec![
        ("object", t.object.clone()),
        ("type", t.type_.clone()),
        ("int", t.int.clone()),
        ("bool", t.bool_.clone()),
        ("float", t.float.clone()),
        ("complex", t.complex.clone()),
        ("str", t.str_.clone()),
        ("bytes", t.bytes.clone()),
        ("bytearray", t.bytearray.clone()),
        ("tuple", t.tuple.clone()),
        ("list", t.list.clone()),
        ("dict", t.dict.clone()),
        ("set", t.set.clone()),
        ("frozenset", t.frozenset.clone()),
        ("property", t.property.clone()),
        ("staticmethod", t.staticmethod.clone()),
        ("classmethod", t.classmethod.clone()),
        ("slice", t.slice.clone()),
        ("range", t.range.clone()),
        ("super", t.super_.clone()),
        // Python 2 aliases (one text type serves both str and unicode).
        ("unicode", t.str_.clone()),
        ("basestring", t.str_.clone()),
        ("long", t.int.clone()),
    ];
    for (n, ty) in types {
        add_value(vm, n, Value::Obj(ty));
    }
    let excs: Vec<(&'static str, ObjRef)> = vm.types.exc.iter().map(|(k, v)| (*k, v.clone())).collect();
    for (n, e) in excs {
        add_value(vm, n, Value::Obj(e));
    }
    let ioe = vm.types.exc("OSError");
    add_value(vm, "IOError", Value::Obj(ioe.clone()));
    add_value(vm, "EnvironmentError", Value::Obj(ioe));
    let std = vm.types.exc("Exception");
    add_value(vm, "StandardError", Value::Obj(std));
    add_value(vm, "None", Value::None);
    add_value(vm, "True", Value::Bool(true));
    add_value(vm, "False", Value::Bool(false));
    let ni = vm.types.not_implemented.clone();
    add_value(vm, "NotImplemented", ni);
    let el = vm.types.ellipsis.clone();
    add_value(vm, "Ellipsis", el);
    add_value(vm, "__debug__", Value::Bool(true));

    add(vm, "abs", b_abs);
    add(vm, "all", b_all);
    add(vm, "any", b_any);
    add(vm, "ascii", b_ascii);
    add(vm, "bin", b_bin);
    add(vm, "callable", b_callable);
    add(vm, "chr", b_chr);
    add(vm, "unichr", b_chr);
    add(vm, "cmp", b_cmp);
    add(vm, "compile", b_compile);
    add(vm, "delattr", b_delattr);
    add(vm, "dir", b_dir);
    add(vm, "divmod", b_divmod);
    add(vm, "enumerate", b_enumerate);
    add(vm, "eval", b_eval);
    add(vm, "exec", b_exec);
    add(vm, "execfile", b_execfile);
    add(vm, "filter", b_filter);
    add(vm, "format", b_format);
    add(vm, "getattr", b_getattr);
    add(vm, "globals", b_globals);
    add(vm, "hasattr", b_hasattr);
    add(vm, "hash", b_hash);
    add(vm, "hex", b_hex);
    add(vm, "id", b_id);
    add(vm, "intern", b_intern);
    add(vm, "isinstance", b_isinstance);
    add(vm, "issubclass", b_issubclass);
    add(vm, "iter", b_iter);
    add(vm, "len", b_len);
    add(vm, "locals", b_locals);
    add(vm, "map", b_map);
    add(vm, "max", b_max);
    add(vm, "min", b_min);
    add(vm, "next", b_next);
    add(vm, "oct", b_oct);
    add(vm, "ord", b_ord);
    add(vm, "pow", b_pow);
    add(vm, "print", b_print);
    add(vm, "repr", b_repr);
    add(vm, "reversed", b_reversed);
    add(vm, "round", b_round);
    add(vm, "setattr", b_setattr);
    add(vm, "sorted", b_sorted);
    add(vm, "sum", b_sum);
    add(vm, "vars", b_vars);
    add(vm, "zip", b_zip);
    add(vm, "xrange", b_xrange);
    add(vm, "reduce", crate::modules::functools::reduce);
    add(vm, "apply", b_apply);
    add(vm, "__import__", crate::import::b_import);
    add(vm, "__build_class__", build_class);
    add(vm, "open", b_open);
    add(vm, "input", b_input);
    add(vm, "raw_input", b_input);
    add(vm, "breakpoint", b_noop);

    let bm = vm.builtins_module.clone();
    add_value(vm, "__name__", vm.types.new_str("builtins"));
    crate::methods::install(vm);
    crate::modules::install(vm);
    let k = vm.types.new_str("builtins");
    vm.modules.dict_set_raw(&vm.types, k, Value::Obj(bm.clone()));
    let k = vm.types.new_str("__builtin__");
    vm.modules.dict_set_raw(&vm.types, k, Value::Obj(bm));
}

// ---- class creation ----

pub fn build_class(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if args.len() < 2 {
        return Err(vm.type_error("__build_class__: not enough arguments"));
    }
    let func = args[0].clone();
    let name = args[1].clone();
    let mut bases: Vec<Value> = args[2..].to_vec();
    // PEP 560 __mro_entries__.
    let mut resolved = Vec::with_capacity(bases.len());
    for b in bases.drain(..) {
        if !ops::is_type(&b) {
            if let Some(me) = ops::getattr_opt(vm, &b, "__mro_entries__")? {
                let tup = vm.types.new_tuple(resolved.clone());
                let r = vm.call(&me, vec![tup], Vec::new())?;
                resolved.extend(ops::collect(vm, &r)?);
                continue;
            }
        }
        resolved.push(b);
    }
    let bases = resolved;
    let mut meta: Option<Value> = None;
    let mut kwargs: Kwargs = Vec::new();
    for (k, v) in kw {
        if &**k == "metaclass" {
            meta = Some(v.clone());
        } else {
            kwargs.push((k.clone(), v.clone()));
        }
    }
    let explicit_meta = meta.is_some();
    let mut meta = match meta {
        Some(m) => m,
        None => match bases.first() {
            Some(b) => Value::Obj(ops::type_of(vm, b)),
            None => Value::Obj(vm.types.type_.clone()),
        },
    };
    if ops::is_type(&meta) {
        meta = Value::Obj(calculate_meta(vm, meta.obj().unwrap(), &bases)?);
    }
    // __prepare__
    let bases_t = vm.types.new_tuple(bases.clone());
    let ns = match ops::getattr_opt(vm, &meta, "__prepare__")? {
        Some(p) if !matches!(p.payload(), Some(Payload::Builtin(b)) if b.name == "type.__prepare__") => {
            vm.call(&p, vec![name.clone(), bases_t.clone()], kwargs.clone())?
        }
        _ => vm.types.new_dict(),
    };
    // Run the body with the namespace as locals.
    let Some(Payload::Function(f)) = func.payload() else {
        return Err(vm.type_error("__build_class__: func must be a function"));
    };
    let code = f.code.clone();
    let globals = f.globals.clone();
    let mut cells = Vec::new();
    for _ in code.cellvars.iter() {
        cells.push(vm.types.new_cell(None));
    }
    cells.extend(f.closure.iter().cloned());
    run_class_body(vm, code, globals, ns.clone(), cells)?;
    // Python 2 __metaclass__ in the class body.
    if !explicit_meta {
        if let Some(Value::Obj(nso)) = Some(&ns) {
            if let Some(m) = nso.dict_get_str_raw("__metaclass__") {
                meta = m;
            }
        }
    }
    let cls = vm.call(&meta, vec![name, bases_t, ns.clone()], kwargs)?;
    // Fill the implicit __class__ cell.
    if let Value::Obj(nso) = &ns {
        if let Some(Value::Obj(cell)) = nso.dict_get_str_raw("__classcell__") {
            if let Payload::Cell(c) = &cell.payload {
                *c.borrow_mut() = Some(cls.clone());
            }
        }
    }
    Ok(cls)
}

fn run_class_body(vm: &mut Vm, code: Rc<crate::code::Code>, globals: ObjRef, ns: Value, cells: Vec<ObjRef>) -> PyResult<Value> {
    let mut frame = Box::new(crate::vm::Frame {
        code: code.clone(),
        ip: 0,
        stack: Vec::new(),
        blocks: Vec::new(),
        fast: vec![None; code.varnames.len()],
        cells,
        globals,
        locals: Some(ns),
        builtins: vm.builtins.clone(),
        saved_exc: None,
        func_name: code.name.clone(),
    });
    frame.func_name = code.name.clone();
    vm.native_depth += 1;
    let r = vm.run_frame(frame);
    vm.native_depth -= 1;
    r
}

fn calculate_meta(vm: &Vm, meta: &ObjRef, bases: &[Value]) -> PyResult<ObjRef> {
    let mut winner = meta.clone();
    for b in bases {
        let bt = ops::type_of(vm, b);
        if ops::is_subclass(&winner, &bt) {
            continue;
        }
        if ops::is_subclass(&bt, &winner) {
            winner = bt;
            continue;
        }
        return Err(vm.type_error("metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases"));
    }
    Ok(winner)
}

/// `type.__new__(meta, name, bases, ns)`.
pub fn type_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if args.len() == 2 {
        return Ok(Value::Obj(ops::type_of(vm, &args[1])));
    }
    if args.len() != 4 {
        return Err(vm.type_error("type() takes 1 or 3 arguments"));
    }
    let meta = args[0].obj().cloned().ok_or_else(|| vm.type_error("type.__new__(X): X is not a type object"))?;
    let name = str_arg(vm, &args[1], "type name")?;
    let mut bases: Vec<ObjRef> = Vec::new();
    for b in ops::collect(vm, &args[2])? {
        match b {
            Value::Obj(o) if o.type_data().is_some() => bases.push(o),
            _ => return Err(vm.type_error("bases must be types")),
        }
    }
    if bases.is_empty() {
        bases.push(vm.types.object.clone());
    }
    // Builtin storage comes from the first base with a non-object payload.
    let mut kind = BaseKind::Object;
    for b in &bases {
        let k = b.type_data().unwrap().base_kind;
        if k != BaseKind::Object && kind == BaseKind::Object {
            kind = k;
        }
    }
    if kind == BaseKind::Bool {
        return Err(vm.type_error("type 'bool' is not an acceptable base type"));
    }
    let ns_items = ops::mapping_items(vm, &args[3])?;
    let mut d = Dict::new();
    for (k, v) in ns_items {
        let v = match (k.as_str(), v.payload()) {
            // __new__ is an implicit staticmethod; __init_subclass__ and
            // __class_getitem__ are implicit classmethods.
            (Some("__new__"), Some(Payload::Function(_))) => Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(v.clone()))),
            (Some("__init_subclass__" | "__class_getitem__"), Some(Payload::Function(_))) => Value::Obj(new_obj(vm.types.classmethod.clone(), Some(Dict::new()), Payload::ClassMethod(v.clone()))),
            _ => v,
        };
        let h = ops::hash(vm, &k)?;
        d.insert_new(h, k, v);
    }
    let slots = d.get_str("__slots__").is_some();
    let qual = d.get_str("__qualname__").and_then(|q| q.as_str().map(|s| s.to_string())).unwrap_or_else(|| name.clone());
    d.del_str("__qualname__");
    if d.get_str("__doc__").is_none() {
        d.set_str(vm.types.intern("__doc__"), Value::None);
    }
    let td = TypeData {
        name: RefCell::new(name.as_str().into()),
        qualname: RefCell::new(qual.as_str().into()),
        bases: RefCell::new(bases.clone()),
        mro: RefCell::new(Vec::new()),
        base_kind: kind,
        builtin: false,
        no_dict: slots && false,
    };
    let cls = new_obj(meta, Some(d), Payload::Type(td));
    let m = crate::types::c3_mro(&cls, &bases).map_err(|e| vm.type_error(e))?;
    crate::types::set_mro(&cls, m);
    // __set_name__ hooks.
    let entries = cls.dict.as_ref().unwrap().borrow().items();
    for (k, v) in entries {
        if let Value::Obj(vo) = &v {
            if ops::is_user_type(&vo.class()) {
                if let Some(sn) = ops::lookup_type(&vo.class(), "__set_name__") {
                    vm.call(&sn, vec![v.clone(), Value::Obj(cls.clone()), k], Vec::new())?;
                }
            }
        }
    }
    // __init_subclass__ of the parent.
    let parent_mro = ops::mro(&cls);
    if let Some(parent) = parent_mro.get(1) {
        if let Some(f) = ops::lookup_type(parent, "__init_subclass__") {
            let f = match f.payload() {
                Some(Payload::ClassMethod(inner)) => Some(inner.clone()),
                Some(Payload::Builtin(_)) => None,
                _ => Some(f.clone()),
            };
            if let Some(f) = f {
                vm.call(&f, vec![Value::Obj(cls.clone())], kw.to_vec())?;
            }
        }
    }
    Ok(Value::Obj(cls))
}

/// `object.__new__(cls, ...)`.
pub fn object_new(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    let Some(Value::Obj(cls)) = args.first() else {
        return Err(vm.type_error("object.__new__(): not enough arguments"));
    };
    if cls.type_data().is_none() {
        return Err(vm.type_error("object.__new__(X): X is not a type object"));
    }
    let o = ops::alloc_instance(vm, cls);
    if ops::is_subclass(cls, &vm.types.exc("BaseException")) {
        let t = vm.types.new_tuple(args[1..].to_vec());
        ops::set_raw_attr(vm, &o, "args", t);
    }
    Ok(Value::Obj(o))
}

// ---- special attributes of builtin objects ----

pub fn builtin_attr(vm: &mut Vm, obj: &Value, key: &str) -> PyResult<Option<Value>> {
    let types = &vm.types;
    if key == "__class__" {
        return Ok(Some(Value::Obj(ops::type_of(vm, obj))));
    }
    let Value::Obj(o) = obj else {
        return Ok(match (obj, key) {
            (Value::Int(i), "real" | "numerator") => Some(Value::Int(*i)),
            (Value::Int(_), "imag") => Some(Value::Int(0)),
            (Value::Int(_), "denominator") => Some(Value::Int(1)),
            (Value::Bool(b), "real" | "numerator") => Some(Value::Int(*b as i64)),
            (Value::Float(f), "real") => Some(Value::Float(*f)),
            (Value::Float(_), "imag") => Some(Value::Float(0.0)),
            _ => None,
        });
    };
    Ok(match (&o.payload, key) {
        (Payload::Type(td), "__name__") => Some(types.new_str(&td.name.borrow())),
        (Payload::Type(td), "__qualname__") => Some(types.new_str(&td.qualname.borrow())),
        (Payload::Type(td), "__bases__") => Some(types.new_tuple(td.bases.borrow().iter().map(|b| Value::Obj(b.clone())).collect())),
        (Payload::Type(td), "__base__") => Some(td.bases.borrow().first().map(|b| Value::Obj(b.clone())).unwrap_or(Value::None)),
        (Payload::Type(td), "__mro__") => Some(types.new_tuple(td.mro.borrow().iter().map(|b| Value::Obj(b.clone())).collect())),
        (Payload::Type(_), "__dict__") => Some(namespace_proxy(vm, o)),
        (Payload::Type(td), "__module__") if td.builtin => Some(types.new_str("builtins")),
        (Payload::Type(_), "__doc__") => Some(Value::None),
        (Payload::Function(f), "__name__") => Some(types.new_str(&f.name.borrow())),
        (Payload::Function(f), "__qualname__") => Some(types.new_str(&f.qualname.borrow())),
        (Payload::Function(f), "__module__") => Some(f.module.borrow().clone()),
        (Payload::Function(f), "__doc__") => Some(f.doc.borrow().clone()),
        (Payload::Function(f), "__defaults__" | "func_defaults") => Some(if f.defaults.is_empty() { Value::None } else { types.new_tuple(f.defaults.clone()) }),
        (Payload::Function(f), "__kwdefaults__") => Some(f.kwdefaults.clone().map(Value::Obj).unwrap_or(Value::None)),
        (Payload::Function(f), "__globals__" | "func_globals") => Some(Value::Obj(f.globals.clone())),
        (Payload::Function(f), "__code__" | "func_code") => Some(types.new_code(f.code.clone())),
        (Payload::Function(f), "__closure__") => Some(if f.closure.is_empty() { Value::None } else { types.new_tuple(f.closure.iter().map(|c| Value::Obj(c.clone())).collect()) }),
        (Payload::Function(_), "__dict__") => Some(namespace_proxy(vm, o)),
        (Payload::Function(_), "__annotations__") => Some(types.new_dict()),
        (Payload::Function(f), "func_name") => Some(types.new_str(&f.name.borrow())),
        (Payload::Builtin(b), "__name__") => Some(types.new_str(b.name.rsplit('.').next().unwrap_or(b.name))),
        (Payload::Builtin(b), "__qualname__") => Some(types.new_str(b.name)),
        (Payload::Builtin(b), "__self__") => Some(b.bound.clone().unwrap_or(Value::None)),
        (Payload::Builtin(_), "__module__") => Some(Value::None),
        (Payload::Builtin(b), "__doc__") => Some(types.builtin_doc(b.func)),
        (Payload::Method { func, .. }, "__func__" | "im_func") => Some(func.clone()),
        (Payload::Method { slf, .. }, "__self__" | "im_self") => Some(slf.clone()),
        (Payload::Method { func, .. }, "__name__" | "__qualname__" | "__doc__" | "__module__" | "__code__" | "__defaults__" | "__globals__") => {
            let f = func.clone();
            return ops::getattr(vm, &f, key).map(Some);
        }
        (Payload::Method { slf, .. }, "im_class") => Some(Value::Obj(ops::type_of(vm, slf))),
        (Payload::Module(_, ns), "__dict__") => Some(Value::Obj(ns.clone())),
        (Payload::StaticMethod(f) | Payload::ClassMethod(f), "__func__") => Some(f.clone()),
        (Payload::Property { fget, .. }, "fget") => Some(fget.clone()),
        (Payload::Property { fset, .. }, "fset") => Some(fset.clone()),
        (Payload::Property { fdel, .. }, "fdel") => Some(fdel.clone()),
        (Payload::Property { doc, .. }, "__doc__") => Some(doc.clone()),
        (Payload::Slice(a, _, _), "start") => Some(a.clone()),
        (Payload::Slice(_, b, _), "stop") => Some(b.clone()),
        (Payload::Slice(_, _, c), "step") => Some(c.clone()),
        (Payload::Range(a, _, _), "start") => Some(Value::Int(*a)),
        (Payload::Range(_, b, _), "stop") => Some(Value::Int(*b)),
        (Payload::Range(_, _, s), "step") => Some(Value::Int(*s)),
        (Payload::Cell(c), "cell_contents") => match c.borrow().clone() {
            Some(v) => Some(v),
            None => return Err(vm.value_error("Cell is empty")),
        },
        (Payload::Code(c), "co_name") => Some(types.new_str(&c.name)),
        (Payload::Code(c), "co_filename") => Some(types.new_str(&c.filename)),
        (Payload::Code(c), "co_firstlineno") => Some(Value::Int(c.firstlineno as i64)),
        (Payload::Code(c), "co_flags") => Some(Value::Int(c.flags as i64 | if c.classic_division { 0 } else { 0x2000 } | 0x8000)),
        (Payload::Code(c), "co_argcount") => Some(Value::Int(c.argcount as i64)),
        (Payload::Code(c), "co_varnames") => Some(types.new_tuple(c.varnames.iter().map(|n| types.new_str(n)).collect())),
        (Payload::Generator(g), "__name__") => Some(types.new_str(&g.borrow().name)),
        (Payload::Generator(_), "gi_running") => Some(Value::Bool(false)),
        (Payload::Int(b), "real" | "numerator") => Some(types.new_bigint(b.clone())),
        (Payload::Int(_), "imag") => Some(Value::Int(0)),
        (Payload::Int(_), "denominator") => Some(Value::Int(1)),
        (Payload::Float(f), "real") => Some(Value::Float(*f)),
        (Payload::Float(_), "imag") => Some(Value::Float(0.0)),
        (Payload::Complex(r, _), "real") => Some(Value::Float(*r)),
        (Payload::Complex(_, i), "imag") => Some(Value::Float(*i)),
        (Payload::Super { .. }, _) => return super_getattr(vm, o, key),
        (_, "__dict__") if o.dict.is_some() => Some(namespace_proxy(vm, o)),
        (_, "__doc__") => Some(Value::None),
        _ => {
            // Exception attributes.
            if ops::is_subclass(&o.class(), &vm.types.exc("BaseException")) {
                return Ok(match key {
                    "__cause__" | "__context__" | "__traceback__" => Some(Value::None),
                    "__suppress_context__" => Some(Value::Bool(false)),
                    "message" => {
                        let a = crate::format::exc_args(o);
                        Some(if a.len() == 1 { a[0].clone() } else { types.new_str("") })
                    }
                    "value" if ops::is_subclass(&o.class(), &vm.types.exc("StopIteration")) => {
                        let a = crate::format::exc_args(o);
                        Some(a.first().cloned().unwrap_or(Value::None))
                    }
                    "code" if ops::is_subclass(&o.class(), &vm.types.exc("SystemExit")) => {
                        let a = crate::format::exc_args(o);
                        Some(a.first().cloned().unwrap_or(Value::None))
                    }
                    _ => None,
                });
            }
            None
        }
    })
}

pub fn builtin_setattr(vm: &mut Vm, o: &ObjRef, key: &str, v: &Value) -> PyResult<bool> {
    match &o.payload {
        Payload::Function(f) => match key {
            "__name__" => {
                *f.name.borrow_mut() = Rc::from(str_arg(vm, v, "__name__")?.as_str());
                Ok(true)
            }
            "__qualname__" => {
                *f.qualname.borrow_mut() = Rc::from(str_arg(vm, v, "__qualname__")?.as_str());
                Ok(true)
            }
            "__doc__" => {
                *f.doc.borrow_mut() = v.clone();
                Ok(true)
            }
            "__module__" => {
                *f.module.borrow_mut() = v.clone();
                Ok(true)
            }
            _ => Ok(false),
        },
        Payload::Type(td) => match key {
            "__name__" => {
                *td.name.borrow_mut() = Rc::from(str_arg(vm, v, "__name__")?.as_str());
                Ok(true)
            }
            "__qualname__" => {
                *td.qualname.borrow_mut() = Rc::from(str_arg(vm, v, "__qualname__")?.as_str());
                Ok(true)
            }
            _ => {
                if td.builtin && !matches!(key, "__module__" | "__doc__") && o.dict.is_none() {
                    return Err(vm.type_error(format!("can't set attributes of built-in/extension type '{}'", td.name.borrow())));
                }
                Ok(false)
            }
        },
        _ => Ok(false),
    }
}

pub fn super_getattr(vm: &mut Vm, o: &ObjRef, key: &str) -> PyResult<Option<Value>> {
    let Payload::Super { typ, obj } = &o.payload else { unreachable!() };
    let start_cls = if ops::is_type(obj) && ops::is_subclass(obj.obj().unwrap(), typ) { obj.obj().unwrap().clone() } else { ops::type_of(vm, obj) };
    let m = ops::mro(&start_cls);
    let pos = m.iter().position(|c| Rc::ptr_eq(c, typ)).map(|p| p + 1).unwrap_or(m.len());
    for c in &m[pos..] {
        let found = c.dict.as_ref().and_then(|d| d.borrow().get_str(key).cloned());
        if let Some(a) = found {
            if obj.is_none() {
                return Ok(Some(a));
            }
            let inst = if ops::is_type(obj) && ops::is_subclass(obj.obj().unwrap(), typ) { None } else { Some(obj) };
            // Classmethods and __new__ bind to the class.
            if inst.is_none() {
                return match a.payload() {
                    Some(Payload::ClassMethod(f)) => Ok(Some(Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: f.clone(), slf: obj.clone() })))),
                    Some(Payload::StaticMethod(f)) => Ok(Some(f.clone())),
                    _ => Ok(Some(a)),
                };
            }
            return ops::descr_get(vm, &a, inst, &start_cls).map(Some);
        }
    }
    Ok(None)
}

/// `obj.__dict__` as a live mapping over the instance namespace.
pub fn namespace_proxy(vm: &mut Vm, o: &ObjRef) -> Value {
    let cls = vm.classes.mappingproxy.clone().unwrap();
    Value::Obj(new_obj(cls, None, Payload::Native(RefCell::new(Box::new(NsProxy(o.clone()))))))
}

pub struct NsProxy(pub ObjRef);

pub fn proxy_target(v: &Value) -> Option<ObjRef> {
    match v.payload()? {
        Payload::Native(n) => n.borrow().downcast_ref::<NsProxy>().map(|p| p.0.clone()),
        _ => None,
    }
}

/// `defaultdict.default_factory` lookup for missing keys.
pub fn dict_missing(vm: &Vm, d: &ObjRef) -> Option<Value> {
    let dd = vm.classes.defaultdict.as_ref()?;
    if !ops::is_subclass(&d.class(), dd) {
        return None;
    }
    let f = ops::get_raw_attr(d, "default_factory")?;
    if f.is_none() {
        return None;
    }
    let factory = f;
    let target = d.clone();
    let _ = target;
    // Bound helper: calls factory(), stores and returns it.
    Some(vm.types.new_builtin_data("defaultdict.__missing__", defaultdict_missing, vm.types.new_tuple(vec![Value::Obj(d.clone()), factory])))
}

fn defaultdict_missing(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    let Some(Payload::Tuple(t)) = args[0].payload() else { unreachable!() };
    let (d, f) = (t[0].clone(), t[1].clone());
    let v = vm.call0(&f)?;
    ops::dict_set(vm, d.obj().unwrap(), args[1].clone(), v.clone())?;
    Ok(v)
}

pub fn native_iter_next(vm: &mut Vm, o: &ObjRef) -> PyResult<Option<Value>> {
    crate::modules::itertools::next_native(vm, o)
}

// ---- I/O ----

pub fn print_write(vm: &mut Vm, dest: Option<&Value>, s: &str, _softspace: bool) -> PyResult<()> {
    match dest {
        Some(d) if !d.is_none() => {
            let sv = vm.types.new_str(s);
            vm.call_method(d, "write", vec![sv])?;
            Ok(())
        }
        _ => {
            let out = crate::modules::sys::stdout(vm);
            match out {
                Some(o) if !is_default_stdout(vm, &o) => {
                    let sv = vm.types.new_str(s);
                    vm.call_method(&o, "write", vec![sv])?;
                }
                _ => vm.host.write_stdout(s),
            }
            Ok(())
        }
    }
}

fn is_default_stdout(vm: &Vm, o: &Value) -> bool {
    matches!((o, &vm.classes.stdout), (Value::Obj(a), Some(c)) if Rc::ptr_eq(&a.class(), c))
}

fn b_print(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut sep = " ".to_string();
    let mut end = "\n".to_string();
    let mut file = None;
    for (k, v) in kw {
        match &**k {
            "sep" if !v.is_none() => sep = str_arg(vm, v, "sep")?,
            "end" if !v.is_none() => end = str_arg(vm, v, "end")?,
            "file" => file = Some(v.clone()),
            "flush" | "sep" | "end" => {}
            _ => return Err(vm.type_error(format!("'{k}' is an invalid keyword argument for print()"))),
        }
    }
    let mut s = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            s.push_str(&sep);
        }
        s.push_str(&ops::str(vm, a)?);
    }
    s.push_str(&end);
    print_write(vm, file.as_ref(), &s, false)?;
    Ok(Value::None)
}

fn b_open(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    crate::modules::io::open(vm, args, kw)
}

fn b_input(vm: &mut Vm, _args: &[Value], _kw: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("EOFError", "EOF when reading a line"))
}

fn b_noop(_vm: &mut Vm, _args: &[Value], _kw: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

// ---- eval / exec ----

fn caller_namespaces(vm: &Vm) -> (ObjRef, Value) {
    let f = vm.frames.last().unwrap();
    let g = f.globals.clone();
    let l = match &f.locals {
        Some(l) => l.clone(),
        None => {
            // Function frame: snapshot of fast locals.
            let d = vm.types.new_dict_obj(Dict::new());
            for (i, n) in f.code.varnames.iter().enumerate() {
                if let Some(v) = &f.fast[i] {
                    d.dict_set_raw(&vm.types, vm.types.new_str(n), v.clone());
                }
            }
            Value::Obj(d)
        }
    };
    (g, l)
}

pub fn exec_impl(vm: &mut Vm, code: &Value, globals: &Value, locals: Option<Value>, dialect: Dialect) -> PyResult<Value> {
    let g = globals.obj().cloned().ok_or_else(|| vm.type_error("exec() globals must be a dict"))?;
    if g.dict_get_str_raw("__builtins__").is_none() {
        if let Payload::Dict(_) = &g.payload {
            g.dict_set_raw(&vm.types, vm.types.intern("__builtins__"), Value::Obj(vm.builtins_module.clone()));
        }
    }
    let c = match code.payload() {
        Some(Payload::Code(c)) => c.clone(),
        Some(Payload::Str(s)) => {
            let src = s.as_str().to_string();
            vm.compile_exec(&src, "<string>", dialect).map_err(|e| vm.syntax_error(e))?
        }
        Some(Payload::Bytes(b)) => {
            let src = String::from_utf8_lossy(b).to_string();
            vm.compile_exec(&src, "<string>", dialect).map_err(|e| vm.syntax_error(e))?
        }
        _ => return Err(vm.type_error("exec() arg 1 must be a string, bytes or code object")),
    };
    vm.native_depth += 1;
    let r = vm.run_code(c, g, locals);
    vm.native_depth -= 1;
    r?;
    Ok(Value::None)
}

fn b_exec(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [code, g, l] = parse(vm, "exec", args, kw, ["source", "globals", "locals"], 1)?;
    let d = ops::vm_dialect(vm);
    let (cg, cl) = caller_namespaces(vm);
    let (g, l) = match (g.filter(|x| !x.is_none()), l.filter(|x| !x.is_none())) {
        (None, None) => (Value::Obj(cg), Some(cl)),
        (Some(g), None) => (g.clone(), Some(g)),
        (Some(g), Some(l)) => (g, Some(l)),
        (None, Some(l)) => (Value::Obj(cg), Some(l)),
    };
    exec_impl(vm, &code.unwrap(), &g, l, d)
}

fn b_execfile(vm: &mut Vm, _args: &[Value], _kw: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("NotImplementedError", "execfile() is not available"))
}

fn b_eval(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [src, g, l] = parse(vm, "eval", args, kw, ["source", "globals", "locals"], 1)?;
    let d = ops::vm_dialect(vm);
    let (cg, cl) = caller_namespaces(vm);
    let (g, l) = match (g.filter(|x| !x.is_none()), l.filter(|x| !x.is_none())) {
        (None, None) => (cg, Some(cl)),
        (Some(g), l) => (g.obj().cloned().ok_or_else(|| vm.type_error("globals must be a dict"))?, l.or(Some(g))),
        (None, Some(l)) => (cg, Some(l)),
    };
    if g.dict_get_str_raw("__builtins__").is_none() {
        if let Payload::Dict(_) = &g.payload {
            g.dict_set_raw(&vm.types, vm.types.intern("__builtins__"), Value::Obj(vm.builtins_module.clone()));
        }
    }
    let src = src.unwrap();
    let code = match src.payload() {
        Some(Payload::Code(c)) => c.clone(),
        Some(Payload::Str(s)) => {
            let text = s.as_str().trim_start_matches([' ', '\t']).to_string();
            vm.compile_eval(&text, "<string>", d).map_err(|e| vm.syntax_error(e))?
        }
        Some(Payload::Bytes(b)) => {
            let text = String::from_utf8_lossy(b).trim_start().to_string();
            vm.compile_eval(&text, "<string>", d).map_err(|e| vm.syntax_error(e))?
        }
        _ => return Err(vm.type_error("eval() arg 1 must be a string, bytes or code object")),
    };
    vm.native_depth += 1;
    let r = vm.run_code(code, g, l);
    vm.native_depth -= 1;
    r
}

fn b_compile(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [src, filename, mode, _flags, _dont, _opt] = parse(vm, "compile", args, kw, ["source", "filename", "mode", "flags", "dont_inherit", "optimize"], 3)?;
    let src = src.unwrap();
    let text = match src.payload() {
        Some(Payload::Str(s)) => s.as_str().to_string(),
        Some(Payload::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
        _ => return Err(vm.type_error("compile() arg 1 must be a string or bytes")),
    };
    let fname = str_arg(vm, filename.as_ref().unwrap(), "filename")?;
    let mode = str_arg(vm, mode.as_ref().unwrap(), "mode")?;
    let d = ops::vm_dialect(vm);
    let code = match mode.as_str() {
        "eval" => vm.compile_eval(text.trim(), &fname, d),
        "exec" | "single" => vm.compile_exec(&text, &fname, d),
        _ => return Err(vm.value_error("compile() mode must be 'exec', 'eval' or 'single'")),
    }
    .map_err(|e| vm.syntax_error(e))?;
    Ok(vm.types.new_code(code))
}

fn b_globals(vm: &mut Vm, _args: &[Value], _kw: Kw) -> PyResult<Value> {
    Ok(Value::Obj(vm.frames.last().unwrap().globals.clone()))
}

fn b_locals(vm: &mut Vm, _args: &[Value], _kw: Kw) -> PyResult<Value> {
    Ok(caller_namespaces(vm).1)
}

fn b_vars(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    match args.first() {
        None => Ok(caller_namespaces(vm).1),
        Some(o) => ops::getattr(vm, o, "__dict__"),
    }
}

// ---- builtin functions ----

fn b_abs(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "abs", args, 1, 1)?;
    match num::num_of(&args[0]) {
        Some(Num::I(i)) => Ok(match i.checked_abs() {
            Some(v) => Value::Int(v),
            None => vm.types.new_bigint(BigInt::from(i).abs()),
        }),
        Some(Num::B(b)) => Ok(vm.types.new_bigint(b.abs())),
        Some(Num::F(f)) => Ok(Value::Float(f.abs())),
        Some(Num::C(r, i)) => Ok(Value::Float(r.hypot(i))),
        None => match ops::call_special(vm, &args[0], "__abs__", Vec::new())? {
            Some(v) => Ok(v),
            None => Err(vm.type_error(format!("bad operand type for abs(): '{}'", ops::type_name(vm, &args[0])))),
        },
    }
}

fn b_all(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "all", args, 1, 1)?;
    let it = ops::iter(vm, &args[0])?;
    while let Some(v) = ops::next(vm, &it)? {
        if !ops::truthy(vm, &v)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn b_any(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "any", args, 1, 1)?;
    let it = ops::iter(vm, &args[0])?;
    while let Some(v) = ops::next(vm, &it)? {
        if ops::truthy(vm, &v)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn b_ascii(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "ascii", args, 1, 1)?;
    let s = ops::ascii(vm, &args[0])?;
    Ok(vm.types.new_string(s))
}

fn radix_str(vm: &mut Vm, v: &Value, radix: u32, prefix: &str) -> PyResult<Value> {
    let i = ops::index_of(vm, v).ok().map(BigInt::from).or_else(|| ops::to_bigint(v));
    let i = match i {
        Some(i) => i,
        None => {
            let r = ops::call_special(vm, v, "__index__", Vec::new())?.ok_or_else(|| vm.type_error("an integer is required"))?;
            ops::to_bigint(&r).unwrap()
        }
    };
    let s = format!("{}{}{}", if i.is_negative() { "-" } else { "" }, prefix, i.abs().to_str_radix(radix));
    Ok(vm.types.new_string(s))
}

fn b_bin(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "bin", args, 1, 1)?;
    radix_str(vm, &args[0], 2, "0b")
}
fn b_hex(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "hex", args, 1, 1)?;
    radix_str(vm, &args[0], 16, "0x")
}
fn b_oct(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "oct", args, 1, 1)?;
    let p = if ops::vm_dialect(vm) == Dialect::Py2 { "0" } else { "0o" };
    radix_str(vm, &args[0], 8, p)
}

pub fn is_callable(vm: &Vm, v: &Value) -> bool {
    match v.payload() {
        Some(Payload::Function(_) | Payload::Builtin(_) | Payload::Method { .. } | Payload::Type(_)) => true,
        Some(_) => ops::lookup_special(vm, v, "__call__").is_some(),
        None => false,
    }
}

fn b_callable(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "callable", args, 1, 1)?;
    Ok(Value::Bool(is_callable(vm, &args[0])))
}

fn b_chr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "chr", args, 1, 1)?;
    let i = ops::index_of(vm, &args[0])?;
    let c = u32::try_from(i).ok().and_then(char::from_u32).ok_or_else(|| vm.value_error("chr() arg not in range(0x110000)"))?;
    let mut b = [0u8; 4];
    Ok(vm.types.new_str(c.encode_utf8(&mut b)))
}

fn b_ord(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "ord", args, 1, 1)?;
    match args[0].payload() {
        Some(Payload::Str(s)) if s.char_len() == 1 => Ok(Value::Int(s.as_str().chars().next().unwrap() as i64)),
        Some(Payload::Bytes(b)) if b.len() == 1 => Ok(Value::Int(b[0] as i64)),
        Some(Payload::Str(s)) => Err(vm.type_error(format!("ord() expected a character, but string of length {} found", s.char_len()))),
        _ => Err(vm.type_error(format!("ord() expected string of length 1, but {} found", ops::type_name(vm, &args[0])))),
    }
}

fn b_cmp(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "cmp", args, 2, 2)?;
    Ok(Value::Int(three_way(vm, &args[0], &args[1])? as i64))
}

pub fn three_way(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<i32> {
    if ops::eq(vm, a, b)? {
        return Ok(0);
    }
    let lt = ops::compare(vm, a, b, crate::ast::CmpOp::Lt)?;
    Ok(if ops::truthy(vm, &lt)? { -1 } else { 1 })
}

fn b_delattr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "delattr", args, 2, 2)?;
    ops::delattr(vm, &args[0], &args[1])?;
    Ok(Value::None)
}

fn b_dir(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    let mut names: Vec<String> = Vec::new();
    match args.first() {
        None => {
            let (_, l) = caller_namespaces(vm);
            for (k, _) in ops::mapping_items(vm, &l)? {
                if let Some(s) = k.as_str() {
                    names.push(s.to_string());
                }
            }
        }
        Some(o) => {
            if let Some(f) = ops::lookup_user(&ops::type_of(vm, o), "__dir__") {
                let r = vm.call(&f, vec![o.clone()], Vec::new())?;
                let l = ops::collect(vm, &r)?;
                for k in l {
                    if let Some(s) = k.as_str() {
                        names.push(s.to_string());
                    }
                }
            } else {
                if let Value::Obj(ob) = o {
                    if let Some(d) = &ob.dict {
                        for k in d.borrow().keys() {
                            if let Some(s) = k.as_str() {
                                names.push(s.to_string());
                            }
                        }
                    }
                }
                let t = if ops::is_type(o) { o.obj().unwrap().clone() } else { ops::type_of(vm, o) };
                for c in ops::mro(&t) {
                    if let Some(d) = &c.dict {
                        for k in d.borrow().keys() {
                            if let Some(s) = k.as_str() {
                                names.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    let items = names.iter().map(|n| vm.types.new_str(n)).collect();
    Ok(vm.types.new_list(items))
}

fn b_divmod(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "divmod", args, 2, 2)?;
    if let Some(r) = ops::call_special(vm, &args[0], "__divmod__", vec![args[1].clone()])? {
        if !ops::is_not_implemented(&r) {
            return Ok(r);
        }
    }
    let q = ops::binary_op(vm, &args[0], &args[1], crate::ast::BinOp::FloorDiv, false, false)?;
    let m = ops::binary_op(vm, &args[0], &args[1], crate::ast::BinOp::Mod, false, false)?;
    Ok(vm.types.new_tuple(vec![q, m]))
}

fn b_enumerate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [it, start] = parse(vm, "enumerate", args, kw, ["iterable", "start"], 1)?;
    let it = ops::iter(vm, &it.unwrap())?;
    Ok(vm.types.new_iter(IterState::Enumerate { it, n: start.unwrap_or(Value::Int(0)) }))
}

fn b_filter(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "filter", args, 2, 2)?;
    let it = ops::iter(vm, &args[1])?;
    let r = vm.types.new_iter(IterState::Filter { func: args[0].clone(), it, invert: false });
    if ops::vm_dialect(vm) == Dialect::Py2 {
        let items = ops::collect(vm, &r)?;
        if let Some(Payload::Str(_)) = args[1].payload() {
            let s: String = items.iter().filter_map(|v| v.as_str()).collect();
            return Ok(vm.types.new_string(s));
        }
        if let Some(Payload::Tuple(_)) = args[1].payload() {
            return Ok(vm.types.new_tuple(items));
        }
        return Ok(vm.types.new_list(items));
    }
    Ok(r)
}

fn b_map(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    if args.len() < 2 {
        return Err(vm.type_error("map() must have at least two arguments."));
    }
    let mut its = Vec::new();
    for a in &args[1..] {
        its.push(ops::iter(vm, a)?);
    }
    if ops::vm_dialect(vm) == Dialect::Py2 {
        // Python 2: list result; None function builds tuples; pads with None.
        let mut out = Vec::new();
        loop {
            let mut vals = Vec::new();
            let mut any = false;
            for it in &its {
                match ops::next(vm, it)? {
                    Some(v) => {
                        any = true;
                        vals.push(v);
                    }
                    None => vals.push(Value::None),
                }
            }
            if !any {
                break;
            }
            if args[0].is_none() {
                out.push(if vals.len() == 1 { vals.pop().unwrap() } else { vm.types.new_tuple(vals) });
            } else {
                out.push(vm.call(&args[0], vals, Vec::new())?);
            }
        }
        return Ok(vm.types.new_list(out));
    }
    Ok(vm.types.new_iter(IterState::Map { func: args[0].clone(), its }))
}

fn b_zip(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut strict = false;
    for (k, v) in kw {
        if &**k == "strict" {
            strict = ops::truthy(vm, v)?;
        } else {
            return Err(vm.type_error(format!("zip() got an unexpected keyword argument '{k}'")));
        }
    }
    let mut its = Vec::new();
    for a in args {
        its.push(ops::iter(vm, a)?);
    }
    let r = vm.types.new_iter(IterState::Zip { its, strict });
    if ops::vm_dialect(vm) == Dialect::Py2 {
        let items = ops::collect(vm, &r)?;
        return Ok(vm.types.new_list(items));
    }
    Ok(r)
}

fn b_format(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "format", args, 1, 2)?;
    let spec = match args.get(1) {
        Some(s) => str_arg(vm, s, "format spec")?,
        None => String::new(),
    };
    let s = ops::format(vm, &args[0], &spec)?;
    Ok(vm.types.new_string(s))
}

fn b_getattr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "getattr", args, 2, 3)?;
    if args[1].as_str().is_none() {
        return Err(vm.type_error("getattr(): attribute name must be string"));
    }
    match ops::getattr_v(vm, &args[0], &args[1]) {
        Ok(v) => Ok(v),
        Err(e) if args.len() == 3 && vm.is_exc(&e, "AttributeError") => Ok(args[2].clone()),
        Err(e) => Err(e),
    }
}

fn b_hasattr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "hasattr", args, 2, 2)?;
    if args[1].as_str().is_none() {
        return Err(vm.type_error("hasattr(): attribute name must be string"));
    }
    match ops::getattr_v(vm, &args[0], &args[1]) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) if vm.is_exc(&e, "AttributeError") => Ok(Value::Bool(false)),
        Err(e) if ops::vm_dialect(vm) == Dialect::Py2 && matches!(e, PyErr::Exc(_)) => Ok(Value::Bool(false)),
        Err(e) => Err(e),
    }
}

fn b_setattr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "setattr", args, 3, 3)?;
    if args[1].as_str().is_none() {
        return Err(vm.type_error("setattr(): attribute name must be string"));
    }
    ops::setattr(vm, &args[0], &args[1], args[2].clone())?;
    Ok(Value::None)
}

fn b_hash(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "hash", args, 1, 1)?;
    Ok(Value::Int(ops::hash(vm, &args[0])?))
}

fn b_id(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "id", args, 1, 1)?;
    Ok(Value::Int(args[0].id()))
}

fn b_intern(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "intern", args, 1, 1)?;
    let s = str_arg(vm, &args[0], "intern() argument")?;
    Ok(vm.types.intern(&s))
}

pub fn isinstance_any(vm: &mut Vm, v: &Value, cls: &Value) -> PyResult<bool> {
    match cls.payload() {
        Some(Payload::Tuple(t)) => {
            for c in t.to_vec() {
                if isinstance_any(vm, v, &c)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Some(Payload::Type(_)) => {
            let c = cls.obj().unwrap();
            if ops::isinstance(vm, v, c) {
                return Ok(true);
            }
            let meta = c.class();
            if ops::is_user_type(&meta) {
                if let Some(ic) = ops::lookup_user(&meta, "__instancecheck__") {
                    let r = vm.call(&ic, vec![cls.clone(), v.clone()], Vec::new())?;
                    return ops::truthy(vm, &r);
                }
            }
            // Python 2 old-style class compatibility: obj.__class__.
            Ok(false)
        }
        _ => {
            if let Some(ic) = ops::lookup_special(vm, cls, "__instancecheck__") {
                let r = vm.call(&ic, vec![cls.clone(), v.clone()], Vec::new())?;
                return ops::truthy(vm, &r);
            }
            Err(vm.type_error("isinstance() arg 2 must be a type or tuple of types"))
        }
    }
}

fn b_isinstance(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "isinstance", args, 2, 2)?;
    Ok(Value::Bool(isinstance_any(vm, &args[0], &args[1])?))
}

pub fn issubclass_any(vm: &mut Vm, c: &Value, cls: &Value) -> PyResult<bool> {
    match cls.payload() {
        Some(Payload::Tuple(t)) => {
            for x in t.to_vec() {
                if issubclass_any(vm, c, &x)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Some(Payload::Type(_)) => {
            let Some(co) = c.obj().filter(|o| o.type_data().is_some()) else {
                return Err(vm.type_error("issubclass() arg 1 must be a class"));
            };
            if ops::is_subclass(co, cls.obj().unwrap()) {
                return Ok(true);
            }
            let meta = cls.obj().unwrap().class();
            if ops::is_user_type(&meta) {
                if let Some(sc) = ops::lookup_user(&meta, "__subclasscheck__") {
                    let r = vm.call(&sc, vec![cls.clone(), c.clone()], Vec::new())?;
                    return ops::truthy(vm, &r);
                }
            }
            Ok(false)
        }
        _ => Err(vm.type_error("issubclass() arg 2 must be a class or tuple of classes")),
    }
}

fn b_issubclass(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "issubclass", args, 2, 2)?;
    Ok(Value::Bool(issubclass_any(vm, &args[0], &args[1])?))
}

fn b_iter(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "iter", args, 1, 2)?;
    if args.len() == 2 {
        return Ok(vm.types.new_iter(IterState::Callable { func: args[0].clone(), sentinel: args[1].clone() }));
    }
    ops::iter(vm, &args[0])
}

fn b_len(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "len", args, 1, 1)?;
    Ok(Value::Int(ops::len(vm, &args[0])? as i64))
}

fn minmax(vm: &mut Vm, args: &[Value], kw: Kw, want_max: bool) -> PyResult<Value> {
    let name = if want_max { "max" } else { "min" };
    let mut key = None;
    let mut default = None;
    for (k, v) in kw {
        match &**k {
            "key" => key = Some(v.clone()).filter(|v| !v.is_none()),
            "default" => default = Some(v.clone()),
            _ => return Err(vm.type_error(format!("{name}() got an unexpected keyword argument '{k}'"))),
        }
    }
    let items = if args.len() == 1 { ops::collect(vm, &args[0])? } else { args.to_vec() };
    if args.is_empty() {
        return Err(vm.type_error(format!("{name} expected at least 1 argument, got 0")));
    }
    let mut best: Option<(Value, Value)> = None;
    for it in items {
        let k = match &key {
            Some(f) => vm.call(f, vec![it.clone()], Vec::new())?,
            None => it.clone(),
        };
        match &best {
            None => best = Some((k, it)),
            Some((bk, _)) => {
                let op = if want_max { crate::ast::CmpOp::Gt } else { crate::ast::CmpOp::Lt };
                let r = ops::compare(vm, &k, bk, op)?;
                if ops::truthy(vm, &r)? {
                    best = Some((k, it));
                }
            }
        }
    }
    match best {
        Some((_, v)) => Ok(v),
        None => default.ok_or_else(|| vm.value_error(format!("{name}() arg is an empty sequence"))),
    }
}

fn b_max(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    minmax(vm, args, kw, true)
}
fn b_min(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    minmax(vm, args, kw, false)
}

fn b_next(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "next", args, 1, 2)?;
    match ops::next(vm, &args[0])? {
        Some(v) => Ok(v),
        None => match args.get(1) {
            Some(d) => Ok(d.clone()),
            None => {
                if let Some(Payload::Generator(_)) = args[0].payload() {
                    return Err(vm.stop_iteration(Value::None));
                }
                Err(vm.stop_iteration(Value::None))
            }
        },
    }
}

fn b_pow(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [b, e, m] = parse(vm, "pow", args, kw, ["base", "exp", "mod"], 2)?;
    let (b, e) = (b.unwrap(), e.unwrap());
    match m.filter(|m| !m.is_none()) {
        None => ops::binary_op(vm, &b, &e, crate::ast::BinOp::Pow, false, false),
        Some(m) => {
            let (Some(bb), Some(eb), Some(mb)) = (ops::to_bigint(&b), ops::to_bigint(&e), ops::to_bigint(&m)) else {
                return Err(vm.type_error("pow() 3rd argument not allowed unless all arguments are integers"));
            };
            if mb.is_zero() {
                return Err(vm.value_error("pow() 3rd argument cannot be 0"));
            }
            if eb.is_negative() {
                return Err(vm.value_error("pow() 2nd argument cannot be negative when 3rd argument specified"));
            }
            let r = bb.modpow(&eb, &mb);
            let r = if r.is_negative() != mb.is_negative() && !r.is_zero() { r + &mb } else { r };
            Ok(vm.types.new_bigint(r))
        }
    }
}

fn b_repr(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "repr", args, 1, 1)?;
    let s = ops::repr(vm, &args[0])?;
    Ok(vm.types.new_string(s))
}

fn b_reversed(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "reversed", args, 1, 1)?;
    let v = &args[0];
    if let Some(f) = ops::lookup_user(&ops::type_of(vm, v), "__reversed__") {
        return vm.call(&f, vec![v.clone()], Vec::new());
    }
    match v.payload() {
        Some(Payload::Dict(d)) => {
            let mut keys = d.borrow().keys();
            keys.reverse();
            return Ok(vm.types.new_iter(IterState::Values { items: keys, idx: 0 }));
        }
        Some(Payload::Range(a, b, s)) => {
            let (a, b, s) = (*a, *b, *s);
            let n = ops::range_len(a, b, s);
            let last = a + (n - 1) * s;
            return Ok(vm.types.new_iter(IterState::Range { cur: last, stop: a - s, step: -s }));
        }
        _ => {}
    }
    let n = ops::len(vm, v)?;
    Ok(vm.types.new_iter(IterState::Reversed { seq: v.clone(), idx: n as isize - 1 }))
}

fn b_round(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [x, nd] = parse(vm, "round", args, kw, ["number", "ndigits"], 1)?;
    let x = x.unwrap();
    let nd = nd.filter(|v| !v.is_none());
    let py2 = ops::vm_dialect(vm) == Dialect::Py2;
    if let Some(r) = ops::lookup_user(&ops::type_of(vm, &x), "__round__") {
        let mut a = vec![x.clone()];
        if let Some(n) = nd {
            a.push(n);
        }
        return vm.call(&r, a, Vec::new());
    }
    match num::num_of(&x) {
        Some(Num::I(_)) | Some(Num::B(_)) => {
            let Some(n) = nd else { return Ok(if py2 { Value::Float(ops::to_f64(vm, &x)?) } else { x }) };
            let n = ops::index_of(vm, &n)?;
            if n >= 0 {
                return Ok(if py2 { Value::Float(ops::to_f64(vm, &x)?) } else { x });
            }
            let b = ops::to_bigint(&x).unwrap();
            let p = num_traits::Pow::pow(BigInt::from(10), (-n) as u32);
            let half = &p / 2;
            let (q, r) = num_integer::Integer::div_mod_floor(&b, &p);
            let q = if r > half || (r == half && num_integer::Integer::is_odd(&q)) { q + 1 } else { q };
            Ok(vm.types.new_bigint(q * p))
        }
        Some(Num::F(f)) => {
            let n = match nd {
                Some(n) => Some(ops::index_of(vm, &n)?),
                None => None,
            };
            match n {
                None => {
                    if !f.is_finite() {
                        return Err(if f.is_nan() { vm.value_error("cannot convert float NaN to integer") } else { vm.exc_msg("OverflowError", "cannot convert float infinity to integer") });
                    }
                    if py2 {
                        return Ok(Value::Float(round_half_away(f)));
                    }
                    let r = round_half_even(f);
                    Ok(vm.types.new_bigint(num_traits::FromPrimitive::from_f64(r).unwrap_or_default()))
                }
                Some(n) => {
                    if !f.is_finite() {
                        return Ok(Value::Float(f));
                    }
                    // Correctly rounded via decimal string conversion.
                    let r = if n >= 0 {
                        let s = format!("{:.*}", n.min(320) as usize, f);
                        let v: f64 = s.parse().unwrap_or(f);
                        if py2 {
                            let m = 10f64.powi(n as i32);
                            round_half_away(f * m) / m
                        } else {
                            v
                        }
                    } else {
                        let m = 10f64.powi((-n) as i32);
                        let q = f / m;
                        (if py2 { round_half_away(q) } else { round_half_even(q) }) * m
                    };
                    Ok(Value::Float(r))
                }
            }
        }
        _ => Err(vm.type_error(format!("type {} doesn't define __round__ method", ops::type_name(vm, &x)))),
    }
}

pub fn round_half_even(f: f64) -> f64 {
    let r = f.round();
    if (f - f.trunc()).abs() == 0.5 { 2.0 * (f / 2.0).round() } else { r }
}

fn round_half_away(f: f64) -> f64 {
    f.round()
}

fn b_sorted(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    arg_count(vm, "sorted", args, 1, 1)?;
    let items = ops::collect(vm, &args[0])?;
    let l = vm.types.new_list(items);
    crate::methods::list_sort(vm, &[l.clone()], kw)?;
    Ok(l)
}

fn b_sum(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [it, start] = parse(vm, "sum", args, kw, ["iterable", "start"], 1)?;
    let mut acc = start.unwrap_or(Value::Int(0));
    if acc.as_str().is_some() {
        return Err(vm.type_error("sum() can't sum strings [use ''.join(seq) instead]"));
    }
    let iter = ops::iter(vm, &it.unwrap())?;
    while let Some(v) = ops::next(vm, &iter)? {
        acc = ops::binary_op(vm, &acc, &v, crate::ast::BinOp::Add, false, false)?;
    }
    Ok(acc)
}

fn b_xrange(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    crate::methods::range_new_impl(vm, args, kw, false)
}

fn b_apply(vm: &mut Vm, args: &[Value], _kw: Kw) -> PyResult<Value> {
    arg_count(vm, "apply", args, 1, 3)?;
    let a = match args.get(1) {
        Some(v) => ops::collect(vm, v)?,
        None => Vec::new(),
    };
    let mut k = Vec::new();
    if let Some(m) = args.get(2) {
        for (kk, v) in ops::mapping_items(vm, m)? {
            k.push((Rc::from(kk.as_str().unwrap_or("")), v));
        }
    }
    vm.call(&args[0], a, k)
}

pub fn to_list_of_str(vm: &mut Vm, v: &Value) -> PyResult<Vec<String>> {
    let items = ops::collect(vm, v)?;
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        out.push(ops::str(vm, &it)?);
    }
    Ok(out)
}

pub fn opt_i64(vm: &mut Vm, v: &Option<Value>, default: i64) -> PyResult<i64> {
    match v {
        None => Ok(default),
        Some(x) if x.is_none() => Ok(default),
        Some(x) => ops::index_of(vm, x),
    }
}

pub fn big_to_value(vm: &Vm, b: BigInt) -> Value {
    match b.to_i64() {
        Some(i) => Value::Int(i),
        None => vm.types.new_bigint(b),
    }
}
