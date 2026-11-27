// SPDX-License-Identifier: MPL-2.0
//! Builtin type objects and value constructors. Method tables are attached
//! by `builtins::install`; this module only creates the type graph.

use crate::code::Code;
use crate::dict::Dict;
use crate::object::*;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Types {
    pub object: ObjRef,
    pub type_: ObjRef,
    pub none_type: ObjRef,
    pub bool_: ObjRef,
    pub int: ObjRef,
    pub float: ObjRef,
    pub complex: ObjRef,
    pub str_: ObjRef,
    pub bytes: ObjRef,
    pub bytearray: ObjRef,
    pub tuple: ObjRef,
    pub list: ObjRef,
    pub dict: ObjRef,
    pub set: ObjRef,
    pub frozenset: ObjRef,
    pub function: ObjRef,
    pub builtin_function: ObjRef,
    pub method: ObjRef,
    pub module: ObjRef,
    pub cell: ObjRef,
    pub generator: ObjRef,
    pub property: ObjRef,
    pub staticmethod: ObjRef,
    pub classmethod: ObjRef,
    pub slice: ObjRef,
    pub range: ObjRef,
    pub iterator: ObjRef,
    pub dict_keys: ObjRef,
    pub dict_values: ObjRef,
    pub dict_items: ObjRef,
    pub super_: ObjRef,
    pub code: ObjRef,
    pub weakref: ObjRef,
    pub native: ObjRef,
    pub not_implemented_type: ObjRef,
    pub ellipsis_type: ObjRef,
    pub not_implemented: Value,
    pub ellipsis: Value,
    pub empty_tuple: Value,
    pub exc: HashMap<&'static str, ObjRef>,
    interned: RefCell<HashMap<Box<str>, Value>>,
    /// `__doc__` of native functions, by function pointer.
    pub builtin_docs: RefCell<HashMap<usize, Value>>,
}

pub const EXCEPTIONS: &[(&str, &str)] = &[
    ("BaseException", ""),
    ("SystemExit", "BaseException"),
    ("KeyboardInterrupt", "BaseException"),
    ("GeneratorExit", "BaseException"),
    ("Exception", "BaseException"),
    ("StopIteration", "Exception"),
    ("StopAsyncIteration", "Exception"),
    ("ArithmeticError", "Exception"),
    ("FloatingPointError", "ArithmeticError"),
    ("OverflowError", "ArithmeticError"),
    ("ZeroDivisionError", "ArithmeticError"),
    ("AssertionError", "Exception"),
    ("AttributeError", "Exception"),
    ("BufferError", "Exception"),
    ("EOFError", "Exception"),
    ("ImportError", "Exception"),
    ("ModuleNotFoundError", "ImportError"),
    ("LookupError", "Exception"),
    ("IndexError", "LookupError"),
    ("KeyError", "LookupError"),
    ("MemoryError", "Exception"),
    ("NameError", "Exception"),
    ("UnboundLocalError", "NameError"),
    ("OSError", "Exception"),
    ("BlockingIOError", "OSError"),
    ("ChildProcessError", "OSError"),
    ("ConnectionError", "OSError"),
    ("BrokenPipeError", "ConnectionError"),
    ("ConnectionAbortedError", "ConnectionError"),
    ("ConnectionRefusedError", "ConnectionError"),
    ("ConnectionResetError", "ConnectionError"),
    ("FileExistsError", "OSError"),
    ("FileNotFoundError", "OSError"),
    ("InterruptedError", "OSError"),
    ("IsADirectoryError", "OSError"),
    ("NotADirectoryError", "OSError"),
    ("PermissionError", "OSError"),
    ("ProcessLookupError", "OSError"),
    ("TimeoutError", "OSError"),
    ("ReferenceError", "Exception"),
    ("RuntimeError", "Exception"),
    ("NotImplementedError", "RuntimeError"),
    ("RecursionError", "RuntimeError"),
    ("SyntaxError", "Exception"),
    ("IndentationError", "SyntaxError"),
    ("TabError", "IndentationError"),
    ("SystemError", "Exception"),
    ("TypeError", "Exception"),
    ("ValueError", "Exception"),
    ("UnicodeError", "ValueError"),
    ("UnicodeDecodeError", "UnicodeError"),
    ("UnicodeEncodeError", "UnicodeError"),
    ("UnicodeTranslateError", "UnicodeError"),
    ("Warning", "Exception"),
    ("DeprecationWarning", "Warning"),
    ("PendingDeprecationWarning", "Warning"),
    ("RuntimeWarning", "Warning"),
    ("SyntaxWarning", "Warning"),
    ("UserWarning", "Warning"),
    ("FutureWarning", "Warning"),
    ("ImportWarning", "Warning"),
    ("UnicodeWarning", "Warning"),
    ("BytesWarning", "Warning"),
    ("ResourceWarning", "Warning"),
    ("EncodingWarning", "Warning"),
];

fn type_payload(name: &str, bases: Vec<ObjRef>, kind: BaseKind, no_dict: bool) -> Payload {
    Payload::Type(TypeData {
        name: RefCell::new(name.into()),
        qualname: RefCell::new(name.into()),
        bases: RefCell::new(bases),
        mro: RefCell::new(Vec::new()),
        base_kind: kind,
        builtin: true,
        no_dict,
    })
}

impl Types {
    pub fn new() -> Types {
        let object = Rc::new(PyObject { class: RefCell::new(None), dict: Some(RefCell::new(Dict::new())), payload: type_payload("object", Vec::new(), BaseKind::Object, false) });
        let type_ = Rc::new(PyObject { class: RefCell::new(None), dict: Some(RefCell::new(Dict::new())), payload: type_payload("type", vec![object.clone()], BaseKind::Type, false) });
        type_.set_class(type_.clone());
        object.set_class(type_.clone());
        set_mro(&object, vec![object.clone()]);
        set_mro(&type_, vec![type_.clone(), object.clone()]);
        let mk = |name: &str, base: &ObjRef, kind: BaseKind, no_dict: bool| -> ObjRef {
            let t = new_obj(type_.clone(), Some(Dict::new()), type_payload(name, vec![base.clone()], kind, no_dict));
            let mut mro = vec![t.clone()];
            mro.extend(base.type_data().unwrap().mro.borrow().iter().cloned());
            set_mro(&t, mro);
            t
        };
        let int = mk("int", &object, BaseKind::Int, true);
        let bool_ = mk("bool", &int, BaseKind::Bool, true);
        let exc_base = mk("BaseException", &object, BaseKind::Exception, false);
        let mut exc = HashMap::new();
        exc.insert("BaseException", exc_base);
        for (name, base) in &EXCEPTIONS[1..] {
            let b = exc[base].clone();
            let t = mk(name, &b, BaseKind::Exception, false);
            exc.insert(*name, t);
        }
        let not_implemented_type = mk("NotImplementedType", &object, BaseKind::Object, true);
        let ellipsis_type = mk("ellipsis", &object, BaseKind::Object, true);
        let tuple = mk("tuple", &object, BaseKind::Tuple, true);
        let empty_tuple = Value::Obj(new_obj(tuple.clone(), None, Payload::Tuple(Box::new([]))));
        let t = Types {
            none_type: mk("NoneType", &object, BaseKind::Object, true),
            float: mk("float", &object, BaseKind::Float, true),
            complex: mk("complex", &object, BaseKind::Complex, true),
            str_: mk("str", &object, BaseKind::Str, true),
            bytes: mk("bytes", &object, BaseKind::Bytes, true),
            bytearray: mk("bytearray", &object, BaseKind::ByteArray, true),
            list: mk("list", &object, BaseKind::List, true),
            dict: mk("dict", &object, BaseKind::Dict, true),
            set: mk("set", &object, BaseKind::Set, true),
            frozenset: mk("frozenset", &object, BaseKind::FrozenSet, true),
            function: mk("function", &object, BaseKind::Object, false),
            builtin_function: mk("builtin_function_or_method", &object, BaseKind::Object, true),
            method: mk("method", &object, BaseKind::Object, true),
            module: mk("module", &object, BaseKind::Module, false),
            cell: mk("cell", &object, BaseKind::Object, true),
            generator: mk("generator", &object, BaseKind::Object, true),
            property: mk("property", &object, BaseKind::Property, true),
            staticmethod: mk("staticmethod", &object, BaseKind::Object, false),
            classmethod: mk("classmethod", &object, BaseKind::Object, false),
            slice: mk("slice", &object, BaseKind::Object, true),
            range: mk("range", &object, BaseKind::Object, true),
            iterator: mk("iterator", &object, BaseKind::Object, true),
            dict_keys: mk("dict_keys", &object, BaseKind::Object, true),
            dict_values: mk("dict_values", &object, BaseKind::Object, true),
            dict_items: mk("dict_items", &object, BaseKind::Object, true),
            super_: mk("super", &object, BaseKind::Object, true),
            code: mk("code", &object, BaseKind::Object, true),
            weakref: mk("weakref", &object, BaseKind::Object, true),
            native: mk("native", &object, BaseKind::Native, false),
            not_implemented: Value::Obj(new_obj(not_implemented_type.clone(), None, Payload::NotImplemented)),
            ellipsis: Value::Obj(new_obj(ellipsis_type.clone(), None, Payload::Ellipsis)),
            not_implemented_type,
            ellipsis_type,
            empty_tuple,
            tuple,
            object,
            type_,
            int,
            bool_,
            exc,
            interned: RefCell::new(HashMap::new()),
            builtin_docs: RefCell::new(HashMap::new()),
        };
        t
    }

    /// Gives every native function `f` the docstring `doc`.
    pub fn set_builtin_doc(&self, f: NativeFn, doc: &str) {
        let v = self.new_str(doc);
        self.builtin_docs.borrow_mut().insert(f as usize, v);
    }

    pub fn builtin_doc(&self, f: NativeFn) -> Value {
        self.builtin_docs.borrow().get(&(f as usize)).cloned().unwrap_or(Value::None)
    }

    pub fn exc(&self, name: &str) -> ObjRef {
        self.exc.get(name).cloned().unwrap_or_else(|| self.exc["Exception"].clone())
    }

    pub fn new_str(&self, s: &str) -> Value {
        if s.len() <= 1 || (s.len() < 24 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')) {
            return self.intern(s);
        }
        Value::Obj(new_obj(self.str_.clone(), None, Payload::Str(PyStr::new(s))))
    }
    pub fn new_string(&self, s: String) -> Value {
        if s.len() <= 1 {
            return self.intern(&s);
        }
        Value::Obj(new_obj(self.str_.clone(), None, Payload::Str(PyStr::new(s))))
    }
    pub fn intern(&self, s: &str) -> Value {
        if let Some(v) = self.interned.borrow().get(s) {
            return v.clone();
        }
        let v = Value::Obj(new_obj(self.str_.clone(), None, Payload::Str(PyStr::new(s))));
        self.interned.borrow_mut().insert(s.into(), v.clone());
        v
    }
    pub fn new_bytes(&self, b: Vec<u8>) -> Value {
        Value::Obj(new_obj(self.bytes.clone(), None, Payload::Bytes(b.into_boxed_slice())))
    }
    pub fn new_bigint(&self, b: BigInt) -> Value {
        match b.to_i64() {
            Some(i) => Value::Int(i),
            None => Value::Obj(new_obj(self.int.clone(), None, Payload::Int(b))),
        }
    }
    pub fn new_complex(&self, r: f64, i: f64) -> Value {
        Value::Obj(new_obj(self.complex.clone(), None, Payload::Complex(r, i)))
    }
    pub fn new_tuple(&self, items: Vec<Value>) -> Value {
        if items.is_empty() {
            return self.empty_tuple.clone();
        }
        Value::Obj(new_obj(self.tuple.clone(), None, Payload::Tuple(items.into_boxed_slice())))
    }
    pub fn new_list(&self, items: Vec<Value>) -> Value {
        Value::Obj(new_obj(self.list.clone(), None, Payload::List(RefCell::new(items))))
    }
    pub fn new_dict_obj(&self, d: Dict) -> ObjRef {
        new_obj(self.dict.clone(), None, Payload::Dict(RefCell::new(d)))
    }
    pub fn new_dict(&self) -> Value {
        Value::Obj(self.new_dict_obj(Dict::new()))
    }
    pub fn new_set(&self, d: Dict) -> Value {
        Value::Obj(new_obj(self.set.clone(), None, Payload::Set(RefCell::new(d))))
    }
    pub fn new_frozenset(&self, d: Dict) -> Value {
        Value::Obj(new_obj(self.frozenset.clone(), None, Payload::FrozenSet(d)))
    }
    pub fn new_code(&self, c: Rc<Code>) -> Value {
        Value::Obj(new_obj(self.code.clone(), None, Payload::Code(c)))
    }
    pub fn new_iter(&self, st: IterState) -> Value {
        Value::Obj(new_obj(self.iterator.clone(), None, Payload::Iter(RefCell::new(st))))
    }
    pub fn new_builtin(&self, name: &'static str, func: NativeFn) -> Value {
        Value::Obj(new_obj(self.builtin_function.clone(), None, Payload::Builtin(Builtin { name, func, bound: None })))
    }
    pub fn new_builtin_data(&self, name: &'static str, func: NativeFn, data: Value) -> Value {
        Value::Obj(new_obj(self.builtin_function.clone(), None, Payload::Builtin(Builtin { name, func, bound: Some(data) })))
    }
    pub fn new_native(&self, class: &ObjRef, data: Box<dyn std::any::Any>) -> Value {
        Value::Obj(new_obj(class.clone(), Some(Dict::new()), Payload::Native(RefCell::new(data))))
    }
    pub fn new_cell(&self, v: Option<Value>) -> ObjRef {
        new_obj(self.cell.clone(), None, Payload::Cell(RefCell::new(v)))
    }

    /// Creates a builtin (interpreter-implemented) type deriving from `base`.
    pub fn new_builtin_type(&self, name: &str, base: &ObjRef, kind: BaseKind, no_dict: bool) -> ObjRef {
        let t = new_obj(self.type_.clone(), Some(Dict::new()), type_payload(name, vec![base.clone()], kind, no_dict));
        let mut mro = vec![t.clone()];
        mro.extend(base.type_data().unwrap().mro.borrow().iter().cloned());
        set_mro(&t, mro);
        t
    }

    /// Adds a method to a type's namespace.
    pub fn add_method(&self, t: &ObjRef, name: &'static str, func: NativeFn) {
        let v = self.new_builtin(name, func);
        t.dict.as_ref().unwrap().borrow_mut().set_str(self.intern(name), v);
    }
    pub fn set_attr(&self, t: &ObjRef, name: &str, v: Value) {
        t.dict.as_ref().unwrap().borrow_mut().set_str(self.intern(name), v);
    }
}

impl Default for Types {
    fn default() -> Self {
        Self::new()
    }
}

pub fn set_mro(t: &ObjRef, mro: Vec<ObjRef>) {
    if let Payload::Type(td) = &t.payload {
        *td.mro.borrow_mut() = mro;
    }
}

/// C3 linearisation.
pub fn c3_mro(cls: &ObjRef, bases: &[ObjRef]) -> Result<Vec<ObjRef>, String> {
    let mut seqs: Vec<Vec<ObjRef>> = bases.iter().map(|b| b.type_data().map(|t| t.mro.borrow().clone()).unwrap_or_default()).collect();
    seqs.push(bases.to_vec());
    let mut out = vec![cls.clone()];
    loop {
        seqs.retain(|s| !s.is_empty());
        if seqs.is_empty() {
            return Ok(out);
        }
        let mut cand = None;
        for s in &seqs {
            let head = &s[0];
            if !seqs.iter().any(|t| t[1..].iter().any(|x| Rc::ptr_eq(x, head))) {
                cand = Some(head.clone());
                break;
            }
        }
        let Some(c) = cand else {
            return Err("Cannot create a consistent method resolution order (MRO)".into());
        };
        for s in seqs.iter_mut() {
            if Rc::ptr_eq(&s[0], &c) {
                s.remove(0);
            }
        }
        out.push(c);
    }
}
