// SPDX-License-Identifier: MPL-2.0
//! Object model. Small immutable scalars are immediate `Value`s; everything
//! else is a reference-counted `PyObject` carrying its class, an optional
//! instance `__dict__` and a payload for builtin storage. Instances of user
//! subclasses of builtins (`class L(list)`) carry the builtin payload too, so
//! `isinstance` and native operations agree.

use crate::code::Code;
use crate::dict::Dict;
use crate::vm::Frame;
use num_bigint::BigInt;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};

pub type ObjRef = Rc<PyObject>;

#[derive(Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Obj(ObjRef),
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => write!(f, "None"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Obj(o) => match &o.payload {
                Payload::Str(s) => write!(f, "{:?}", s.as_str()),
                _ => write!(f, "<{} object>", o.class_name()),
            },
        }
    }
}

/// Collector fields come first (`repr(C)`) so they share a cache line with
/// the `Rc` counts the collector reads alongside them.
#[repr(C)]
pub struct PyObject {
    /// Collector state (`gc::TRACKED`, `gc::FINALIZED`, `gc::WEAKREFED`).
    pub(crate) gc_flags: Cell<u8>,
    /// Scratch slot of the collector while the object is a candidate.
    pub(crate) gc_index: Cell<u32>,
    /// Always set after bootstrap (`type` is its own class).
    pub class: RefCell<Option<ObjRef>>,
    pub dict: Option<RefCell<Dict>>,
    pub payload: Payload,
}

/// Python text: code points stored as UTF-8 with a cached character count.
pub struct PyStr {
    s: Box<str>,
    nchars: usize,
    hash: Cell<Option<i64>>,
}

impl PyStr {
    pub fn new(s: impl Into<Box<str>>) -> PyStr {
        let s = s.into();
        let nchars = if s.is_ascii() { s.len() } else { s.chars().count() };
        PyStr { s, nchars, hash: Cell::new(None) }
    }
    pub fn as_str(&self) -> &str {
        &self.s
    }
    pub fn char_len(&self) -> usize {
        self.nchars
    }
    pub fn is_ascii(&self) -> bool {
        self.nchars == self.s.len()
    }
    /// Byte offset of character index `i` (clamped to the end).
    pub fn byte_offset(&self, i: usize) -> usize {
        if self.is_ascii() {
            return i.min(self.s.len());
        }
        self.s.char_indices().nth(i).map(|(b, _)| b).unwrap_or(self.s.len())
    }
    pub fn char_at(&self, i: usize) -> Option<char> {
        if self.is_ascii() {
            return self.s.as_bytes().get(i).map(|b| *b as char);
        }
        self.s.chars().nth(i)
    }
    pub fn slice_chars(&self, start: usize, end: usize) -> &str {
        let a = self.byte_offset(start);
        let b = self.byte_offset(end);
        &self.s[a..b.max(a)]
    }
    pub fn hash(&self) -> i64 {
        if let Some(h) = self.hash.get() {
            return h;
        }
        let h = crate::hash::hash_str(&self.s);
        self.hash.set(Some(h));
        h
    }
}

pub type NativeFn = fn(&mut crate::vm::Vm, &[Value], &[(Rc<str>, Value)]) -> crate::PyResult<Value>;

pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    /// Bound `self` (for methods of builtin types resolved on an instance) or
    /// a captured value for closures created by native code; passed as the
    /// first positional argument.
    pub bound: Option<Value>,
}

#[derive(Clone)]
pub struct Function {
    pub code: Rc<Code>,
    pub globals: ObjRef,
    pub defaults: Vec<Value>,
    pub kwdefaults: Option<ObjRef>,
    pub closure: Vec<ObjRef>,
    pub name: RefCell<Rc<str>>,
    pub qualname: RefCell<Rc<str>>,
    pub module: RefCell<Value>,
    pub doc: RefCell<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseKind {
    Object,
    Int,
    Bool,
    Float,
    Complex,
    Str,
    Bytes,
    ByteArray,
    Tuple,
    List,
    Dict,
    Set,
    FrozenSet,
    Type,
    Exception,
    Module,
    Property,
    Native,
}

pub struct TypeData {
    pub name: RefCell<Rc<str>>,
    pub qualname: RefCell<Rc<str>>,
    pub bases: RefCell<Vec<ObjRef>>,
    pub mro: RefCell<Vec<ObjRef>>,
    pub base_kind: BaseKind,
    /// True for types implemented by the interpreter (not class statements).
    pub builtin: bool,
    /// Instances have no `__dict__` (builtin immutable types, `__slots__`).
    pub no_dict: bool,
    /// The class (or a base) defines `__del__`; checked when instances die.
    pub finalizer: Cell<bool>,
}

pub enum IterState {
    Seq { seq: Value, idx: usize },
    Range { cur: i64, stop: i64, step: i64 },
    BigRange { cur: BigInt, stop: BigInt, step: BigInt },
    StrChars { s: ObjRef, byte: usize },
    DictKeys { dict: ObjRef, idx: usize, len: usize },
    DictValues { dict: ObjRef, idx: usize, len: usize },
    DictItems { dict: ObjRef, idx: usize, len: usize },
    Reversed { seq: Value, idx: isize },
    Enumerate { it: Value, n: Value },
    Zip { its: Vec<Value>, strict: bool },
    Map { func: Value, its: Vec<Value> },
    Filter { func: Value, it: Value, invert: bool },
    Callable { func: Value, sentinel: Value },
    Values { items: Vec<Value>, idx: usize },
    Exhausted,
    /// Iterators of the itertools/natives family store their state here.
    Native(Box<dyn Any>),
}

pub enum GenStatus {
    Created,
    Suspended,
    Running,
    Finished,
}

pub struct Generator {
    pub frame: Option<Box<Frame>>,
    pub status: GenStatus,
    pub name: Rc<str>,
    pub qualname: Rc<str>,
    /// Python 3 `yield from` delegate currently being iterated.
    pub yield_from: Option<Value>,
}

/// A weak reference (`weakref.ref`, proxies). The target is reset when the
/// referent dies or is collected; the callback is taken when it is queued.
pub struct WeakRefData {
    pub target: RefCell<Weak<PyObject>>,
    pub callback: RefCell<Value>,
    pub hash: Cell<Option<i64>>,
}

impl WeakRefData {
    pub fn upgrade(&self) -> Option<ObjRef> {
        self.target.borrow().upgrade()
    }
}

pub enum Payload {
    None,
    Str(PyStr),
    Bytes(Box<[u8]>),
    ByteArray(RefCell<Vec<u8>>),
    /// Large integers and instances of int subclasses.
    Int(BigInt),
    Float(f64),
    Complex(f64, f64),
    Tuple(Box<[Value]>),
    List(RefCell<Vec<Value>>),
    Dict(RefCell<Dict>),
    Set(RefCell<Dict>),
    FrozenSet(Dict),
    Type(TypeData),
    Function(Function),
    Builtin(Builtin),
    Method { func: Value, slf: Value },
    Cell(RefCell<Option<Value>>),
    Generator(RefCell<Generator>),
    Property { fget: Value, fset: Value, fdel: Value, doc: Value },
    StaticMethod(Value),
    ClassMethod(Value),
    Slice(Value, Value, Value),
    Range(i64, i64, i64),
    BigRange(Box<(BigInt, BigInt, BigInt)>),
    Iter(RefCell<IterState>),
    DictView { dict: ObjRef, kind: u8 },
    Super { typ: ObjRef, obj: Value },
    Code(Rc<Code>),
    WeakRef(WeakRefData),
    Native(RefCell<Box<dyn Any>>),
    /// Module name and its namespace (a dict-payload object used as the
    /// globals of code defined in the module).
    Module(Rc<str>, ObjRef),
    NotImplemented,
    Ellipsis,
}

impl PyObject {
    /// `object` and `type` are created before any class exists.
    pub(crate) fn bootstrap_type(payload: Payload) -> ObjRef {
        let o = Rc::new(PyObject { class: RefCell::new(None), dict: Some(RefCell::new(Dict::new())), payload, gc_flags: Cell::new(crate::gc::TRACKED), gc_index: Cell::new(0) });
        crate::gc::track(&o);
        o
    }
    pub fn class(&self) -> ObjRef {
        self.class.borrow().clone().expect("class")
    }
    pub fn set_class(&self, c: ObjRef) {
        *self.class.borrow_mut() = Some(c);
    }
    pub fn class_name(&self) -> String {
        match &self.class().payload {
            Payload::Type(t) => t.name.borrow().to_string(),
            _ => "?".into(),
        }
    }
    pub fn type_data(&self) -> Option<&TypeData> {
        match &self.payload {
            Payload::Type(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&PyStr> {
        match &self.payload {
            Payload::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The attribute namespace: a module's globals dict, else `__dict__`.
    pub fn attr_table(&self) -> Option<&RefCell<Dict>> {
        match &self.payload {
            Payload::Module(_, ns) => match &ns.payload {
                Payload::Dict(d) => Some(d),
                _ => None,
            },
            _ => self.dict.as_ref(),
        }
    }
}

impl Value {
    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }
    pub fn obj(&self) -> Option<&ObjRef> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn payload(&self) -> Option<&Payload> {
        self.obj().map(|o| &o.payload)
    }
    pub fn as_pystr(&self) -> Option<&PyStr> {
        match self {
            Value::Obj(o) => o.as_str(),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        self.as_pystr().map(|s| s.as_str())
    }
    /// Identity comparison (`is`).
    pub fn is(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            // Small immediate ints/floats behave as cached singletons.
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Obj(a), Value::Obj(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn id(&self) -> i64 {
        match self {
            Value::None => 0x1000,
            Value::Bool(b) => 0x1010 + *b as i64,
            Value::Int(i) => (*i).wrapping_mul(2).wrapping_add(1),
            Value::Float(f) => f.to_bits() as i64 | 1,
            Value::Obj(o) => Rc::as_ptr(o) as *const u8 as usize as i64,
        }
    }
}

pub fn new_obj(class: ObjRef, dict: Option<Dict>, payload: Payload) -> ObjRef {
    let tracked = crate::gc::needs_tracking(&class, dict.is_some(), &payload);
    let o = Rc::new(PyObject { class: RefCell::new(Some(class)), dict: dict.map(RefCell::new), payload, gc_flags: Cell::new(if tracked { crate::gc::TRACKED } else { 0 }), gc_index: Cell::new(0) });
    if tracked {
        crate::gc::track(&o);
    }
    o
}
