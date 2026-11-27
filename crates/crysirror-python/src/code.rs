// SPDX-License-Identifier: MPL-2.0
//! Bytecode and code objects. The instruction set is a compact stack machine
//! modelled on CPython 3.9 (block-stack exception handling, compile-time
//! unwinding of `finally`/`with` for return/break/continue).

use crate::ast::{BinOp, CmpOp, UnaryOp};
use crate::object::Value;
use crate::Dialect;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Nop,
    LoadConst(u32),
    LoadName(u32),
    StoreName(u32),
    DelName(u32),
    LoadGlobal(u32),
    StoreGlobal(u32),
    DelGlobal(u32),
    LoadFast(u32),
    StoreFast(u32),
    DelFast(u32),
    /// Index into cellvars ++ freevars.
    LoadDeref(u32),
    StoreDeref(u32),
    DelDeref(u32),
    LoadClassDeref(u32),
    LoadClosure(u32),
    LoadAttr(u32),
    StoreAttr(u32),
    DelAttr(u32),
    Subscr,
    StoreSubscr,
    DelSubscr,
    BuildSlice(u8),
    Binary(BinOp),
    Inplace(BinOp),
    Unary(UnaryOp),
    Compare(CmpOp),
    Pop,
    Dup,
    Dup2,
    Rot2,
    Rot3,
    Rot4,
    Jump(u32),
    PopJumpIfFalse(u32),
    PopJumpIfTrue(u32),
    JumpIfFalseOrPop(u32),
    JumpIfTrueOrPop(u32),
    GetIter,
    /// Push next item or pop the iterator and jump when exhausted.
    ForIter(u32),
    BuildTuple(u32),
    BuildList(u32),
    BuildSet(u32),
    /// `n` key/value pairs, keys pushed before values.
    BuildMap(u32),
    BuildString(u32),
    /// conversion: 0 none, b's', b'r', b'a'; then whether a spec is on the stack.
    FormatValue(u8, bool),
    ListAppend(u32),
    SetAdd(u32),
    MapAdd(u32),
    ListExtend(u32),
    SetUpdate(u32),
    DictUpdate(u32),
    /// Like DictUpdate but rejects duplicate keyword names.
    DictMerge(u32),
    ListToTuple,
    UnpackSequence(u32),
    UnpackEx(u16, u16),
    /// Positional count; callable below the arguments.
    Call(u32),
    /// Total arg count; a tuple of keyword names on top.
    CallKw(u32),
    /// Callable, args tuple and optionally a kwargs dict.
    CallEx(bool),
    /// Flags: 1 defaults tuple, 2 kwdefaults dict, 4 annotations, 8 closure.
    /// Stack (bottom→top): [defaults][kwdefaults][annotations][closure] code qualname.
    MakeFunction(u8),
    Return,
    Yield,
    /// Delegate to the sub-iterator below TOS (TOS is the value to send).
    YieldFrom,
    GetYieldFromIter,
    SetupFinally(u32),
    SetupWith(u32),
    PopBlock,
    PopExcept,
    /// 0: re-raise active, 1: raise TOS, 2: raise TOS1 from TOS.
    Raise(u8),
    /// Raise TOS (the exception pushed for a handler) preserving its traceback.
    Reraise,
    /// TOS1 exception, TOS class/tuple → pushes bool, keeps the exception.
    CheckExcMatch,
    /// Stack [exit, exc] → [exit, exc, exit(type, exc, tb)].
    WithExceptStart,
    /// Stack [level, fromlist] → module.
    ImportName(u32),
    ImportFrom(u32),
    ImportStar,
    LoadBuildClass,
    SetupAnnotations,
    /// Python 2 print statement: item, item with file, newline, newline to file.
    PrintItem,
    PrintItemTo,
    PrintNewline,
    PrintNewlineTo,
    /// Python 2 exec statement: [code, globals, locals] (None for omitted).
    ExecStmt,
    /// Interactive expression statement (unused by scripts).
    PrintExpr,
    /// Pattern matching helpers.
    MatchSequence,
    MatchMapping,
    GetLen,
    /// [subject, keys tuple] → [subject, keys, values tuple or None].
    MatchKeys,
    /// [subject, cls, names tuple] → [attrs tuple or None]; arg = positional count.
    MatchClass(u32),
    /// Pops TOS, pushes TOS[i] for an index from the start (or end when negative).
    IndexConst(i32),
    /// Copy of the mapping minus keys tuple: [subject, keys] → [subject, keys, rest].
    CopyDictWithoutKeys,
    /// Python 2 backtick / `repr()` without a builtins lookup.
    Repr,
}

pub const CO_VARARGS: u32 = 0x4;
pub const CO_VARKEYWORDS: u32 = 0x8;
pub const CO_GENERATOR: u32 = 0x20;
pub const CO_COROUTINE: u32 = 0x80;
/// Function scope (fast locals) as opposed to module/class/exec code.
pub const CO_NEWLOCALS: u32 = 0x2;

pub struct Code {
    pub name: Rc<str>,
    pub qualname: Rc<str>,
    pub filename: Rc<str>,
    pub firstlineno: u32,
    pub ops: Vec<Op>,
    pub lines: Vec<u32>,
    pub consts: Vec<Value>,
    /// Names as interned string Values for namespace lookups.
    pub names: Vec<Value>,
    pub varnames: Vec<Rc<str>>,
    pub cellvars: Vec<Rc<str>>,
    pub freevars: Vec<Rc<str>>,
    /// For each cellvar, the argument it shadows.
    pub cell2arg: Vec<Option<usize>>,
    pub argcount: usize,
    pub posonlyargcount: usize,
    pub kwonlyargcount: usize,
    pub flags: u32,
    pub dialect: Dialect,
    /// Python 2 code without `from __future__ import division`.
    pub classic_division: bool,
    pub doc: Option<Value>,
}

impl Code {
    pub fn is_generator(&self) -> bool {
        self.flags & CO_GENERATOR != 0
    }
    pub fn line_at(&self, ip: usize) -> u32 {
        self.lines.get(ip).copied().unwrap_or(self.firstlineno)
    }
    pub fn deref_name(&self, i: usize) -> &str {
        if i < self.cellvars.len() { &self.cellvars[i] } else { &self.freevars[i - self.cellvars.len()] }
    }
}
