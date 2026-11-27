// SPDX-License-Identifier: MPL-2.0
//! AST → bytecode. Scope resolution comes from `symtable`; control flow
//! uses labels patched at the end of each code unit. `finally` bodies and
//! `with` exits are inlined on return/break/continue (CPython 3.9 scheme).

use crate::ast::*;
use crate::code::*;
use crate::object::Value;
use crate::symtable::{self, ScopeKind, Sym, SymTable};
use crate::types::Types;
use crate::Dialect;
use num_traits::ToPrimitive;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct CompileError {
    pub msg: String,
    pub line: u32,
}

#[derive(Clone)]
pub struct CompileOptions {
    pub dialect: Dialect,
    /// Python 2 `/` on ints floors (no `division` future).
    pub classic_division: bool,
    pub filename: Rc<str>,
    /// Added to every line number (Ren'Py embeds code at a script line).
    pub line_offset: u32,
}

impl CompileOptions {
    pub fn new(dialect: Dialect, filename: &str) -> Self {
        CompileOptions { dialect, classic_division: dialect == Dialect::Py2, filename: filename.into(), line_offset: 0 }
    }
}

type Label = u32;

enum FBlock<'a> {
    Loop { cont: Label, brk: Label, is_for: bool },
    TryFinally(&'a [Stmt]),
    /// Running a `finally` body on the exception path ([exc] on the stack).
    FinallyEnd,
    /// Inside an `except` body; the handler name is cleaned up on exit.
    ExceptHandler(Option<String>),
    With,
}

struct Unit<'a> {
    scope: usize,
    kind: ScopeKind,
    ops: Vec<Op>,
    lines: Vec<u32>,
    consts: Vec<Value>,
    const_keys: HashMap<ConstKey, u32>,
    names: Vec<String>,
    name_idx: HashMap<String, u32>,
    varnames: Vec<String>,
    var_idx: HashMap<String, u32>,
    cellvars: Vec<String>,
    freevars: Vec<String>,
    labels: Vec<u32>,
    fblocks: Vec<FBlock<'a>>,
    line: u32,
    qualname: String,
    name: String,
    argcount: usize,
    posonly: usize,
    kwonly: usize,
    flags: u32,
    doc: Option<Value>,
    firstline: u32,
}

#[derive(Hash, PartialEq, Eq)]
enum ConstKey {
    None,
    Bool(bool),
    Int(i64),
    Float(u64),
    Str(String),
    Bytes(Vec<u8>),
    Ellipsis,
}

pub struct Compiler<'a> {
    types: &'a Types,
    st: SymTable,
    opts: CompileOptions,
    units: Vec<Unit<'a>>,
}

fn err<T>(msg: impl Into<String>, line: u32) -> Result<T, CompileError> {
    Err(CompileError { msg: msg.into(), line })
}

pub fn compile_module(types: &Types, body: &[Stmt], opts: CompileOptions) -> Result<Rc<Code>, CompileError> {
    let mut body = body.to_vec();
    crate::mangle::mangle_module(&mut body);
    let st = symtable::build_module(&body, opts.dialect).map_err(|e| CompileError { msg: e.0, line: e.1 + opts.line_offset })?;
    // Leak-free: the compiler borrows `body` only within this function.
    let body_ref: &[Stmt] = &body;
    let mut c = Compiler { types, st, opts, units: Vec::new() };
    c.enter(0, ScopeKind::Module, "<module>", "<module>", 1);
    c.docstring(body_ref, true);
    c.stmts(body_ref)?;
    c.load_const(Value::None);
    c.emit(Op::Return);
    Ok(c.finish())
}

pub fn compile_eval(types: &Types, expr: &Expr, opts: CompileOptions) -> Result<Rc<Code>, CompileError> {
    let mut expr = expr.clone();
    crate::mangle::mangle_expr(&mut expr, None);
    let st = symtable::build_expr(&expr, opts.dialect).map_err(|e| CompileError { msg: e.0, line: e.1 + opts.line_offset })?;
    let mut c = Compiler { types, st, opts, units: Vec::new() };
    c.enter(0, ScopeKind::Module, "<module>", "<module>", 1);
    c.expr(&expr)?;
    c.emit(Op::Return);
    Ok(c.finish())
}

impl<'a> Compiler<'a> {
    fn u(&mut self) -> &mut Unit<'a> {
        self.units.last_mut().unwrap()
    }

    fn enter(&mut self, scope: usize, kind: ScopeKind, name: &str, qualname: &str, firstline: u32) {
        let sc = &self.st.scopes[scope];
        let mut u = Unit {
            scope,
            kind,
            ops: Vec::new(),
            lines: Vec::new(),
            consts: Vec::new(),
            const_keys: HashMap::new(),
            names: Vec::new(),
            name_idx: HashMap::new(),
            varnames: Vec::new(),
            var_idx: HashMap::new(),
            cellvars: sc.cells.clone(),
            freevars: sc.frees.clone(),
            labels: Vec::new(),
            fblocks: Vec::new(),
            line: firstline,
            qualname: qualname.to_string(),
            name: name.to_string(),
            argcount: 0,
            posonly: 0,
            kwonly: 0,
            flags: 0,
            doc: None,
            firstline,
        };
        if matches!(kind, ScopeKind::Function | ScopeKind::Comprehension) {
            u.flags |= CO_NEWLOCALS;
            for v in &sc.varnames {
                let i = u.varnames.len() as u32;
                u.var_idx.insert(v.clone(), i);
                u.varnames.push(v.clone());
            }
        }
        if sc.generator {
            u.flags |= CO_GENERATOR;
        }
        self.units.push(u);
    }

    fn finish(&mut self) -> Rc<Code> {
        let u = self.units.pop().unwrap();
        let mut ops = u.ops;
        for op in ops.iter_mut() {
            let fix = |l: &mut u32| *l = u.labels[*l as usize];
            match op {
                Op::Jump(l)
                | Op::PopJumpIfFalse(l)
                | Op::PopJumpIfTrue(l)
                | Op::JumpIfFalseOrPop(l)
                | Op::JumpIfTrueOrPop(l)
                | Op::ForIter(l)
                | Op::SetupFinally(l)
                | Op::SetupWith(l) => fix(l),
                _ => {}
            }
        }
        let sc = &self.st.scopes[u.scope];
        let cell2arg = u.cellvars.iter().map(|c| u.varnames.iter().position(|v| v == c).filter(|i| *i < sc.varnames.len())).collect();
        let names = u.names.iter().map(|n| self.types.intern(n)).collect();
        Rc::new(Code {
            name: u.name.into(),
            qualname: u.qualname.into(),
            filename: self.opts.filename.clone(),
            firstlineno: u.firstline + self.opts.line_offset,
            lines: u.lines.iter().map(|l| l + self.opts.line_offset).collect(),
            ops,
            consts: u.consts,
            names,
            varnames: u.varnames.into_iter().map(Rc::from).collect(),
            cellvars: u.cellvars.into_iter().map(Rc::from).collect(),
            freevars: u.freevars.into_iter().map(Rc::from).collect(),
            cell2arg,
            argcount: u.argcount,
            posonlyargcount: u.posonly,
            kwonlyargcount: u.kwonly,
            flags: u.flags,
            dialect: self.opts.dialect,
            classic_division: self.opts.classic_division,
            doc: u.doc,
        })
    }

    fn emit(&mut self, op: Op) -> usize {
        let u = self.u();
        u.ops.push(op);
        let l = u.line;
        u.lines.push(l);
        u.ops.len() - 1
    }
    fn label(&mut self) -> Label {
        let u = self.u();
        u.labels.push(u32::MAX);
        (u.labels.len() - 1) as Label
    }
    fn bind(&mut self, l: Label) {
        let u = self.u();
        u.labels[l as usize] = u.ops.len() as u32;
    }

    fn load_const(&mut self, v: Value) {
        let key = match &v {
            Value::None => Some(ConstKey::None),
            Value::Bool(b) => Some(ConstKey::Bool(*b)),
            Value::Int(i) => Some(ConstKey::Int(*i)),
            Value::Float(f) => Some(ConstKey::Float(f.to_bits())),
            Value::Obj(o) => match &o.payload {
                crate::object::Payload::Str(s) => Some(ConstKey::Str(s.as_str().to_string())),
                crate::object::Payload::Bytes(b) => Some(ConstKey::Bytes(b.to_vec())),
                crate::object::Payload::Ellipsis => Some(ConstKey::Ellipsis),
                _ => None,
            },
        };
        let u = self.u();
        if let Some(k) = &key {
            if let Some(i) = u.const_keys.get(k) {
                let i = *i;
                self.emit(Op::LoadConst(i));
                return;
            }
        }
        let i = u.consts.len() as u32;
        u.consts.push(v);
        if let Some(k) = key {
            u.const_keys.insert(k, i);
        }
        self.emit(Op::LoadConst(i));
    }

    fn name_index(&mut self, n: &str) -> u32 {
        let u = self.u();
        if let Some(i) = u.name_idx.get(n) {
            return *i;
        }
        let i = u.names.len() as u32;
        u.names.push(n.to_string());
        u.name_idx.insert(n.to_string(), i);
        i
    }

    fn var_index(&mut self, n: &str) -> u32 {
        let u = self.u();
        if let Some(i) = u.var_idx.get(n) {
            return *i;
        }
        let i = u.varnames.len() as u32;
        u.varnames.push(n.to_string());
        u.var_idx.insert(n.to_string(), i);
        i
    }

    fn deref_index(&mut self, n: &str) -> Option<u32> {
        let u = self.u();
        if let Some(i) = u.cellvars.iter().position(|c| c == n) {
            return Some(i as u32);
        }
        u.freevars.iter().position(|c| c == n).map(|i| (i + u.cellvars.len()) as u32)
    }

    fn str_const(&mut self, s: &str) {
        let v = self.types.new_str(s);
        self.load_const(v);
    }

    fn name_op(&mut self, name: &str, ctx: Ctx) -> Result<(), CompileError> {
        let scope = self.u().scope;
        let kind = self.u().kind;
        let sym = self.st.scopes[scope].resolved.get(name).copied().unwrap_or(Sym::GlobalImplicit);
        let fast = matches!(kind, ScopeKind::Function | ScopeKind::Comprehension);
        match sym {
            Sym::Free | Sym::Cell => {
                let Some(i) = self.deref_index(name) else {
                    return err(format!("internal: no cell for '{name}'"), self.u().line);
                };
                self.emit(match ctx {
                    Ctx::Load if kind == ScopeKind::Class => Op::LoadClassDeref(i),
                    Ctx::Load => Op::LoadDeref(i),
                    Ctx::Store => Op::StoreDeref(i),
                    Ctx::Del => Op::DelDeref(i),
                });
            }
            Sym::Local if fast => {
                let i = self.var_index(name);
                self.emit(match ctx {
                    Ctx::Load => Op::LoadFast(i),
                    Ctx::Store => Op::StoreFast(i),
                    Ctx::Del => Op::DelFast(i),
                });
            }
            Sym::GlobalExplicit | Sym::GlobalImplicit if fast || sym == Sym::GlobalExplicit => {
                let i = self.name_index(name);
                self.emit(match ctx {
                    Ctx::Load => Op::LoadGlobal(i),
                    Ctx::Store => Op::StoreGlobal(i),
                    Ctx::Del => Op::DelGlobal(i),
                });
            }
            _ => {
                let i = self.name_index(name);
                self.emit(match ctx {
                    Ctx::Load => Op::LoadName(i),
                    Ctx::Store => Op::StoreName(i),
                    Ctx::Del => Op::DelName(i),
                });
            }
        }
        Ok(())
    }

    fn docstring(&mut self, body: &[Stmt], store: bool) -> bool {
        if let Some(Stmt { kind: StmtKind::Expr(Expr { kind: ExprKind::Constant(Constant::Str(s)), .. }), .. }) = body.first() {
            let v = self.types.new_str(s);
            if store {
                self.load_const(v);
                let _ = self.name_op("__doc__", Ctx::Store);
            } else {
                self.u().doc = Some(v);
            }
            return true;
        }
        false
    }

    fn stmts(&mut self, body: &'a [Stmt]) -> Result<(), CompileError> {
        for s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &'a Stmt) -> Result<(), CompileError> {
        self.u().line = s.loc.line;
        match &s.kind {
            StmtKind::Expr(e) => {
                if matches!(e.kind, ExprKind::Constant(_)) {
                    return Ok(());
                }
                self.expr(e)?;
                self.emit(Op::Pop);
            }
            StmtKind::Pass => {}
            StmtKind::Assign { targets, value } => {
                self.expr(value)?;
                for (i, t) in targets.iter().enumerate() {
                    if i + 1 < targets.len() {
                        self.emit(Op::Dup);
                    }
                    self.store(t)?;
                }
            }
            StmtKind::AugAssign { target, op, value } => self.aug_assign(target, *op, value)?,
            StmtKind::AnnAssign { target, annotation, value, simple } => {
                if let Some(v) = value {
                    self.expr(v)?;
                    self.store(target)?;
                }
                if *simple && matches!(self.u().kind, ScopeKind::Module | ScopeKind::Class) {
                    if let ExprKind::Name { id, .. } = &target.kind {
                        self.expr(annotation)?;
                        self.name_op("__annotations__", Ctx::Load)?;
                        self.str_const(id);
                        self.emit(Op::StoreSubscr);
                    }
                }
            }
            StmtKind::Delete(targets) => {
                for t in targets {
                    self.delete(t)?;
                }
            }
            StmtKind::If { test, body, orelse } => {
                let else_l = self.label();
                let end = self.label();
                self.expr(test)?;
                self.emit(Op::PopJumpIfFalse(else_l));
                self.stmts(body)?;
                if !orelse.is_empty() {
                    self.emit(Op::Jump(end));
                }
                self.bind(else_l);
                self.stmts(orelse)?;
                self.bind(end);
            }
            StmtKind::While { test, body, orelse } => {
                let start = self.label();
                let else_l = self.label();
                let end = self.label();
                self.bind(start);
                self.expr(test)?;
                self.emit(Op::PopJumpIfFalse(else_l));
                self.u().fblocks.push(FBlock::Loop { cont: start, brk: end, is_for: false });
                self.stmts(body)?;
                self.u().fblocks.pop();
                self.emit(Op::Jump(start));
                self.bind(else_l);
                self.stmts(orelse)?;
                self.bind(end);
            }
            StmtKind::For { target, iter, body, orelse, .. } => {
                let start = self.label();
                let cleanup = self.label();
                let end = self.label();
                self.expr(iter)?;
                self.emit(Op::GetIter);
                self.bind(start);
                self.emit(Op::ForIter(cleanup));
                self.store(target)?;
                self.u().fblocks.push(FBlock::Loop { cont: start, brk: end, is_for: true });
                self.stmts(body)?;
                self.u().fblocks.pop();
                self.emit(Op::Jump(start));
                self.bind(cleanup);
                self.stmts(orelse)?;
                self.bind(end);
            }
            StmtKind::Break | StmtKind::Continue => {
                let is_break = matches!(s.kind, StmtKind::Break);
                let target = self.unwind(false, true)?;
                let Some((cont, brk, is_for)) = target else {
                    return err(if is_break { "'break' outside loop" } else { "'continue' not properly in loop" }, s.loc.line);
                };
                if is_break {
                    if is_for {
                        self.emit(Op::Pop);
                    }
                    self.emit(Op::Jump(brk));
                } else {
                    self.emit(Op::Jump(cont));
                }
            }
            StmtKind::Return(v) => {
                if !matches!(self.u().kind, ScopeKind::Function | ScopeKind::Comprehension) {
                    return err("'return' outside function", s.loc.line);
                }
                match v {
                    Some(v) => self.expr(v)?,
                    None => self.load_const(Value::None),
                }
                self.unwind(true, false)?;
                self.emit(Op::Return);
            }
            StmtKind::Raise { exc, cause, py2_value, .. } => match (exc, cause, py2_value) {
                (None, _, _) => {
                    self.emit(Op::Raise(0));
                }
                (Some(e), Some(c), _) => {
                    self.expr(e)?;
                    self.expr(c)?;
                    self.emit(Op::Raise(2));
                }
                (Some(e), None, Some(v)) => {
                    self.expr(e)?;
                    self.expr(v)?;
                    self.emit(Op::Raise(3));
                }
                (Some(e), None, None) => {
                    self.expr(e)?;
                    self.emit(Op::Raise(1));
                }
            },
            StmtKind::Assert { test, msg } => {
                let end = self.label();
                self.expr(test)?;
                self.emit(Op::PopJumpIfTrue(end));
                self.name_op("AssertionError", Ctx::Load)?;
                if let Some(m) = msg {
                    self.expr(m)?;
                    self.emit(Op::Call(1));
                }
                self.emit(Op::Raise(1));
                self.bind(end);
            }
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                if finalbody.is_empty() {
                    self.try_except(body, handlers, orelse)?;
                } else {
                    self.try_finally(body, handlers, orelse, finalbody)?;
                }
            }
            StmtKind::With { items, body, .. } => self.with(items, body)?,
            StmtKind::FunctionDef { name, args, body, decorators, .. } => {
                for d in decorators {
                    self.expr(d)?;
                }
                self.function(s, name, args, FuncBody::Stmts(body))?;
                for _ in decorators {
                    self.emit(Op::Call(1));
                }
                self.name_op(name, Ctx::Store)?;
            }
            StmtKind::ClassDef { name, bases, keywords, body, decorators } => {
                for d in decorators {
                    self.expr(d)?;
                }
                self.class(s, name, bases, keywords, body)?;
                for _ in decorators {
                    self.emit(Op::Call(1));
                }
                self.name_op(name, Ctx::Store)?;
            }
            StmtKind::Import(names) => {
                for a in names {
                    self.load_const(Value::Int(if self.opts.dialect == Dialect::Py2 { -1 } else { 0 }));
                    self.load_const(Value::None);
                    let i = self.name_index(&a.name);
                    self.emit(Op::ImportName(i));
                    match &a.asname {
                        Some(asn) => {
                            for part in a.name.split('.').skip(1) {
                                let i = self.name_index(part);
                                self.emit(Op::LoadAttr(i));
                            }
                            self.name_op(asn, Ctx::Store)?;
                        }
                        None => {
                            let top = a.name.split('.').next().unwrap().to_string();
                            self.name_op(&top, Ctx::Store)?;
                        }
                    }
                }
            }
            StmtKind::ImportFrom { module, names, level } => {
                let lvl = if *level == 0 && self.opts.dialect == Dialect::Py2 { -1 } else { *level as i64 };
                self.load_const(Value::Int(lvl));
                let fl: Vec<Value> = names.iter().map(|a| self.types.new_str(&a.name)).collect();
                let t = self.types.new_tuple(fl);
                self.load_const(t);
                let i = self.name_index(module.as_deref().unwrap_or(""));
                self.emit(Op::ImportName(i));
                if names.len() == 1 && names[0].name == "*" {
                    self.emit(Op::ImportStar);
                } else {
                    for a in names {
                        let i = self.name_index(&a.name);
                        self.emit(Op::ImportFrom(i));
                        self.name_op(a.asname.as_ref().unwrap_or(&a.name), Ctx::Store)?;
                    }
                    self.emit(Op::Pop);
                }
            }
            StmtKind::Global(_) | StmtKind::Nonlocal(_) => {}
            StmtKind::Print { dest, values, nl } => {
                for v in values {
                    if let Some(d) = dest {
                        self.expr(d)?;
                        self.expr(v)?;
                        self.emit(Op::PrintItemTo);
                    } else {
                        self.expr(v)?;
                        self.emit(Op::PrintItem);
                    }
                }
                if *nl {
                    if let Some(d) = dest {
                        self.expr(d)?;
                        self.emit(Op::PrintNewlineTo);
                    } else {
                        self.emit(Op::PrintNewline);
                    }
                }
            }
            StmtKind::Exec { body, globals, locals } => {
                self.expr(body)?;
                match globals {
                    Some(g) => self.expr(g)?,
                    None => self.load_const(Value::None),
                }
                match locals {
                    Some(l) => self.expr(l)?,
                    None => self.load_const(Value::None),
                }
                self.emit(Op::ExecStmt);
            }
            StmtKind::Match { subject, cases } => self.match_stmt(subject, cases)?,
        }
        Ok(())
    }

    /// Emits unwinding code for the fblock stack. With `to_loop`, stops at the
    /// innermost loop and returns its labels.
    fn unwind(&mut self, preserve_tos: bool, to_loop: bool) -> Result<Option<(Label, Label, bool)>, CompileError> {
        let mut popped: Vec<FBlock<'a>> = Vec::new();
        let mut result = None;
        loop {
            let Some(fb) = self.u().fblocks.pop() else { break };
            match &fb {
                FBlock::Loop { cont, brk, is_for } => {
                    if to_loop {
                        result = Some((*cont, *brk, *is_for));
                        popped.push(fb);
                        break;
                    }
                    if *is_for {
                        if preserve_tos {
                            self.emit(Op::Rot2);
                        }
                        self.emit(Op::Pop);
                    }
                }
                FBlock::TryFinally(body) => {
                    let body: &'a [Stmt] = body;
                    self.emit(Op::PopBlock);
                    self.stmts(body)?;
                }
                FBlock::FinallyEnd => {
                    if preserve_tos {
                        self.emit(Op::Rot2);
                    }
                    self.emit(Op::Pop);
                    self.emit(Op::PopExcept);
                }
                FBlock::ExceptHandler(name) => {
                    if let Some(n) = name.clone() {
                        // The cleanup try/finally block around a named handler.
                        self.emit(Op::PopBlock);
                        self.emit(Op::PopExcept);
                        self.load_const(Value::None);
                        self.name_op(&n, Ctx::Store)?;
                        self.name_op(&n, Ctx::Del)?;
                    } else {
                        self.emit(Op::PopExcept);
                    }
                }
                FBlock::With => {
                    self.emit(Op::PopBlock);
                    if preserve_tos {
                        self.emit(Op::Rot2);
                    }
                    self.load_const(Value::None);
                    self.emit(Op::Dup);
                    self.emit(Op::Dup);
                    self.emit(Op::Call(3));
                    self.emit(Op::Pop);
                }
            }
            popped.push(fb);
        }
        while let Some(fb) = popped.pop() {
            self.u().fblocks.push(fb);
        }
        Ok(result)
    }

    fn try_except(&mut self, body: &'a [Stmt], handlers: &'a [Handler], orelse: &'a [Stmt]) -> Result<(), CompileError> {
        let handler_l = self.label();
        let end = self.label();
        self.emit(Op::SetupFinally(handler_l));
        // The body is not inside an fblock that needs inline unwinding:
        // the runtime block is popped by unwinding on exception, and by
        // PopBlock when leaving normally. Return/break inside need PopBlock.
        self.u().fblocks.push(FBlock::TryFinally(&[]));
        self.stmts(body)?;
        self.u().fblocks.pop();
        self.emit(Op::PopBlock);
        self.stmts(orelse)?;
        self.emit(Op::Jump(end));
        self.bind(handler_l);
        self.handlers(handlers, end)?;
        self.bind(end);
        Ok(())
    }

    fn handlers(&mut self, handlers: &'a [Handler], end: Label) -> Result<(), CompileError> {
        for h in handlers {
            self.u().line = h.loc.line;
            let next = self.label();
            if let Some(t) = &h.typ {
                // CheckExcMatch pops the type and keeps the exception.
                self.expr(t)?;
                self.emit(Op::CheckExcMatch);
                self.emit(Op::PopJumpIfFalse(next));
            }
            let py3_cleanup = self.opts.dialect == Dialect::Py3;
            match &h.name {
                Some(Expr { kind: ExprKind::Name { id, .. }, .. }) if py3_cleanup => {
                    let id = id.clone();
                    self.name_op(&id, Ctx::Store)?;
                    let cleanup = self.label();
                    self.emit(Op::SetupFinally(cleanup));
                    self.u().fblocks.push(FBlock::ExceptHandler(Some(id.clone())));
                    self.stmts(&h.body)?;
                    self.u().fblocks.pop();
                    self.emit(Op::PopBlock);
                    self.emit(Op::PopExcept);
                    self.load_const(Value::None);
                    self.name_op(&id, Ctx::Store)?;
                    self.name_op(&id, Ctx::Del)?;
                    self.emit(Op::Jump(end));
                    // Exception inside the handler body: clear the name, re-raise.
                    self.bind(cleanup);
                    self.load_const(Value::None);
                    self.name_op(&id, Ctx::Store)?;
                    self.name_op(&id, Ctx::Del)?;
                    self.emit(Op::Reraise);
                }
                other => {
                    match other {
                        Some(target) => self.store(target)?,
                        None => {
                            self.emit(Op::Pop);
                        }
                    }
                    self.u().fblocks.push(FBlock::ExceptHandler(None));
                    self.stmts(&h.body)?;
                    self.u().fblocks.pop();
                    self.emit(Op::PopExcept);
                    self.emit(Op::Jump(end));
                }
            }
            self.bind(next);
            if h.typ.is_none() {
                break;
            }
        }
        self.emit(Op::Reraise);
        Ok(())
    }

    fn try_finally(&mut self, body: &'a [Stmt], handlers: &'a [Handler], orelse: &'a [Stmt], finalbody: &'a [Stmt]) -> Result<(), CompileError> {
        let final_l = self.label();
        let end = self.label();
        self.emit(Op::SetupFinally(final_l));
        self.u().fblocks.push(FBlock::TryFinally(finalbody));
        if handlers.is_empty() {
            self.stmts(body)?;
            self.stmts(orelse)?;
        } else {
            self.try_except(body, handlers, orelse)?;
        }
        self.u().fblocks.pop();
        self.emit(Op::PopBlock);
        self.stmts(finalbody)?;
        self.emit(Op::Jump(end));
        self.bind(final_l);
        self.u().fblocks.push(FBlock::FinallyEnd);
        self.stmts(finalbody)?;
        self.u().fblocks.pop();
        self.emit(Op::Reraise);
        self.bind(end);
        Ok(())
    }

    fn with(&mut self, items: &'a [WithItem], body: &'a [Stmt]) -> Result<(), CompileError> {
        let Some((first, rest)) = items.split_first() else {
            return self.stmts(body);
        };
        let exit_l = self.label();
        let end = self.label();
        self.expr(&first.context)?;
        self.emit(Op::SetupWith(exit_l));
        match &first.vars {
            Some(v) => self.store(v)?,
            None => {
                self.emit(Op::Pop);
            }
        }
        self.u().fblocks.push(FBlock::With);
        if rest.is_empty() {
            self.stmts(body)?;
        } else {
            self.with(rest, body)?;
        }
        self.u().fblocks.pop();
        self.emit(Op::PopBlock);
        self.load_const(Value::None);
        self.emit(Op::Dup);
        self.emit(Op::Dup);
        self.emit(Op::Call(3));
        self.emit(Op::Pop);
        self.emit(Op::Jump(end));
        self.bind(exit_l);
        let suppress = self.label();
        self.emit(Op::WithExceptStart);
        self.emit(Op::PopJumpIfTrue(suppress));
        self.emit(Op::Reraise);
        self.bind(suppress);
        self.emit(Op::Pop);
        self.emit(Op::PopExcept);
        self.emit(Op::Pop);
        self.bind(end);
        Ok(())
    }

    fn match_stmt(&mut self, subject: &'a Expr, cases: &'a [MatchCase]) -> Result<(), CompileError> {
        let end = self.label();
        self.expr(subject)?;
        for c in cases {
            let next = self.label();
            self.emit(Op::Dup);
            self.pattern(&c.pattern, next)?;
            if let Some(g) = &c.guard {
                self.expr(g)?;
                self.emit(Op::PopJumpIfFalse(next));
            }
            self.emit(Op::Pop);
            self.stmts(&c.body)?;
            self.emit(Op::Jump(end));
            self.bind(next);
        }
        self.emit(Op::Pop);
        self.bind(end);
        Ok(())
    }

    /// Subject on TOS; consumed on success and on failure (jump to `fail`).
    fn pattern(&mut self, p: &'a Pattern, fail: Label) -> Result<(), CompileError> {
        match p {
            Pattern::Value(e) => {
                self.expr(e)?;
                self.emit(Op::Compare(CmpOp::Eq));
                self.emit(Op::PopJumpIfFalse(fail));
            }
            Pattern::Singleton(c) => {
                let v = self.constant(c);
                self.load_const(v);
                self.emit(Op::Compare(CmpOp::Is));
                self.emit(Op::PopJumpIfFalse(fail));
            }
            Pattern::As { pattern: None, name } => match name {
                Some(n) => self.name_op(n, Ctx::Store)?,
                None => {
                    self.emit(Op::Pop);
                }
            },
            Pattern::As { pattern: Some(inner), name } => {
                let pop = self.label();
                let ok = self.label();
                self.emit(Op::Dup);
                self.pattern(inner, pop)?;
                match name {
                    Some(n) => self.name_op(n, Ctx::Store)?,
                    None => {
                        self.emit(Op::Pop);
                    }
                }
                self.emit(Op::Jump(ok));
                self.bind(pop);
                self.emit(Op::Pop);
                self.emit(Op::Jump(fail));
                self.bind(ok);
            }
            Pattern::Or(alts) => {
                let ok = self.label();
                for alt in alts {
                    let next = self.label();
                    self.emit(Op::Dup);
                    self.pattern(alt, next)?;
                    self.emit(Op::Pop);
                    self.emit(Op::Jump(ok));
                    self.bind(next);
                }
                self.emit(Op::Pop);
                self.emit(Op::Jump(fail));
                self.bind(ok);
            }
            Pattern::Star(_) => return err("starred pattern outside a sequence pattern", self.u().line),
            Pattern::Sequence(items) => {
                let pop = self.label();
                let ok = self.label();
                let star = items.iter().position(|x| matches!(x, Pattern::Star(_)));
                self.emit(Op::MatchSequence);
                self.emit(Op::PopJumpIfFalse(pop));
                self.emit(Op::GetLen);
                match star {
                    None => {
                        self.load_const(Value::Int(items.len() as i64));
                        self.emit(Op::Compare(CmpOp::Eq));
                    }
                    Some(_) => {
                        self.load_const(Value::Int(items.len() as i64 - 1));
                        self.emit(Op::Compare(CmpOp::GtE));
                    }
                }
                self.emit(Op::PopJumpIfFalse(pop));
                let n = items.len();
                for (i, sub) in items.iter().enumerate() {
                    self.emit(Op::Dup);
                    match star {
                        Some(s) if i == s => {
                            self.load_const(Value::Int(s as i64));
                            let after = n - s - 1;
                            if after == 0 {
                                self.load_const(Value::None);
                            } else {
                                self.load_const(Value::Int(-(after as i64)));
                            }
                            self.emit(Op::BuildSlice(2));
                            self.emit(Op::Subscr);
                            match sub {
                                Pattern::Star(Some(name)) => self.name_op(name, Ctx::Store)?,
                                _ => {
                                    self.emit(Op::Pop);
                                }
                            }
                            continue;
                        }
                        Some(s) if i > s => {
                            self.emit(Op::IndexConst(-((n - i) as i32)));
                        }
                        _ => {
                            self.emit(Op::IndexConst(i as i32));
                        }
                    }
                    self.pattern(sub, pop)?;
                }
                self.emit(Op::Pop);
                self.emit(Op::Jump(ok));
                self.bind(pop);
                self.emit(Op::Pop);
                self.emit(Op::Jump(fail));
                self.bind(ok);
            }
            Pattern::Mapping { keys, patterns, rest } => {
                let pop1 = self.label();
                let pop3 = self.label();
                let ok = self.label();
                self.emit(Op::MatchMapping);
                self.emit(Op::PopJumpIfFalse(pop1));
                for k in keys {
                    self.expr(k)?;
                }
                self.emit(Op::BuildTuple(keys.len() as u32));
                self.emit(Op::MatchKeys);
                self.emit(Op::Dup);
                self.load_const(Value::None);
                self.emit(Op::Compare(CmpOp::Is));
                self.emit(Op::PopJumpIfTrue(pop3));
                for (i, sub) in patterns.iter().enumerate() {
                    self.emit(Op::Dup);
                    self.emit(Op::IndexConst(i as i32));
                    self.pattern(sub, pop3)?;
                }
                self.emit(Op::Pop);
                if let Some(r) = rest {
                    self.emit(Op::CopyDictWithoutKeys);
                    self.name_op(r, Ctx::Store)?;
                }
                self.emit(Op::Pop);
                self.emit(Op::Pop);
                self.emit(Op::Jump(ok));
                self.bind(pop3);
                self.emit(Op::Pop);
                self.emit(Op::Pop);
                self.bind(pop1);
                self.emit(Op::Pop);
                self.emit(Op::Jump(fail));
                self.bind(ok);
            }
            Pattern::Class { cls, patterns, kwd_attrs, kwd_patterns } => {
                let pop2 = self.label();
                let ok = self.label();
                self.emit(Op::Dup);
                self.expr(cls)?;
                let names: Vec<Value> = kwd_attrs.iter().map(|a| self.types.new_str(a)).collect();
                let t = self.types.new_tuple(names);
                self.load_const(t);
                self.emit(Op::MatchClass(patterns.len() as u32));
                self.emit(Op::Dup);
                self.load_const(Value::None);
                self.emit(Op::Compare(CmpOp::Is));
                self.emit(Op::PopJumpIfTrue(pop2));
                for (i, sub) in patterns.iter().chain(kwd_patterns).enumerate() {
                    self.emit(Op::Dup);
                    self.emit(Op::IndexConst(i as i32));
                    self.pattern(sub, pop2)?;
                }
                self.emit(Op::Pop);
                self.emit(Op::Pop);
                self.emit(Op::Jump(ok));
                self.bind(pop2);
                self.emit(Op::Pop);
                self.emit(Op::Pop);
                self.emit(Op::Jump(fail));
                self.bind(ok);
            }
        }
        Ok(())
    }

    fn aug_assign(&mut self, target: &'a Expr, op: BinOp, value: &'a Expr) -> Result<(), CompileError> {
        match &target.kind {
            ExprKind::Name { id, .. } => {
                self.name_op(id, Ctx::Load)?;
                self.expr(value)?;
                self.emit(Op::Inplace(op));
                self.name_op(id, Ctx::Store)?;
            }
            ExprKind::Attribute { value: obj, attr, .. } => {
                self.expr(obj)?;
                self.emit(Op::Dup);
                let i = self.name_index(attr);
                self.emit(Op::LoadAttr(i));
                self.expr(value)?;
                self.emit(Op::Inplace(op));
                self.emit(Op::Rot2);
                self.emit(Op::StoreAttr(i));
            }
            ExprKind::Subscript { value: obj, slice, .. } => {
                self.expr(obj)?;
                self.expr(slice)?;
                self.emit(Op::Dup2);
                self.emit(Op::Subscr);
                self.expr(value)?;
                self.emit(Op::Inplace(op));
                self.emit(Op::Rot3);
                self.emit(Op::StoreSubscr);
            }
            _ => return err("illegal expression for augmented assignment", target.loc.line),
        }
        Ok(())
    }

    fn store(&mut self, t: &'a Expr) -> Result<(), CompileError> {
        match &t.kind {
            ExprKind::Name { id, .. } => self.name_op(id, Ctx::Store)?,
            ExprKind::Attribute { value, attr, .. } => {
                self.expr(value)?;
                let i = self.name_index(attr);
                self.emit(Op::StoreAttr(i));
            }
            ExprKind::Subscript { value, slice, .. } => {
                self.expr(value)?;
                self.expr(slice)?;
                self.emit(Op::StoreSubscr);
            }
            ExprKind::Tuple { elts, .. } | ExprKind::List { elts, .. } => {
                let star = elts.iter().position(|e| matches!(e.kind, ExprKind::Starred { .. }));
                match star {
                    None => {
                        self.emit(Op::UnpackSequence(elts.len() as u32));
                    }
                    Some(s) => {
                        self.emit(Op::UnpackEx(s as u16, (elts.len() - s - 1) as u16));
                    }
                }
                for e in elts {
                    match &e.kind {
                        ExprKind::Starred { value, .. } => self.store(value)?,
                        _ => self.store(e)?,
                    }
                }
            }
            ExprKind::Starred { value, .. } => self.store(value)?,
            _ => return err("cannot assign to expression", t.loc.line),
        }
        Ok(())
    }

    fn delete(&mut self, t: &'a Expr) -> Result<(), CompileError> {
        match &t.kind {
            ExprKind::Name { id, .. } => self.name_op(id, Ctx::Del)?,
            ExprKind::Attribute { value, attr, .. } => {
                self.expr(value)?;
                let i = self.name_index(attr);
                self.emit(Op::DelAttr(i));
            }
            ExprKind::Subscript { value, slice, .. } => {
                self.expr(value)?;
                self.expr(slice)?;
                self.emit(Op::DelSubscr);
            }
            ExprKind::Tuple { elts, .. } | ExprKind::List { elts, .. } => {
                for e in elts {
                    self.delete(e)?;
                }
            }
            _ => return err("cannot delete expression", t.loc.line),
        }
        Ok(())
    }

    fn constant(&self, c: &Constant) -> Value {
        match c {
            Constant::None => Value::None,
            Constant::Bool(b) => Value::Bool(*b),
            Constant::Int(i) => match i.to_i64() {
                Some(v) => Value::Int(v),
                None => self.types.new_bigint(i.clone()),
            },
            Constant::Float(f) => Value::Float(*f),
            Constant::Complex(r, i) => self.types.new_complex(*r, *i),
            Constant::Str(s) => self.types.new_str(s),
            Constant::Bytes(b) => self.types.new_bytes(b.clone()),
            Constant::Ellipsis => self.types.ellipsis.clone(),
        }
    }

    fn function(&mut self, node: &'a Stmt, name: &str, args: &'a Arguments, body: FuncBody<'a>) -> Result<(), CompileError> {
        let scope = self.st.scope_of(node);
        self.make_function(scope, name, args, body, node.loc.line)
    }

    fn make_function(&mut self, scope: usize, name: &str, args: &'a Arguments, body: FuncBody<'a>, line: u32) -> Result<(), CompileError> {
        let mut flags = 0u8;
        if !args.defaults.is_empty() {
            for d in &args.defaults {
                self.expr(d)?;
            }
            self.emit(Op::BuildTuple(args.defaults.len() as u32));
            flags |= 1;
        }
        if args.kw_defaults.iter().any(|d| d.is_some()) {
            let mut n = 0;
            for (a, d) in args.kwonly.iter().zip(&args.kw_defaults) {
                if let Some(d) = d {
                    self.str_const(&a.name);
                    self.expr(d)?;
                    n += 1;
                }
            }
            self.emit(Op::BuildMap(n));
            flags |= 2;
        }
        let qual = self.child_qualname(name);
        self.enter(scope, ScopeKind::Function, name, &qual, line);
        {
            let u = self.u();
            u.argcount = args.posonly.len() + args.args.len();
            u.posonly = args.posonly.len();
            u.kwonly = args.kwonly.len();
            if args.vararg.is_some() {
                u.flags |= CO_VARARGS;
            }
            if args.kwarg.is_some() {
                u.flags |= CO_VARKEYWORDS;
            }
        }
        // Python 2 tuple parameters unpack on entry.
        for a in args.posonly.iter().chain(&args.args) {
            if let Some(t) = &a.unpack {
                self.name_op(&a.name, Ctx::Load)?;
                self.store(t)?;
            }
        }
        match body {
            FuncBody::Stmts(b) => {
                self.docstring(b, false);
                self.stmts(b)?;
                self.load_const(Value::None);
                self.emit(Op::Return);
            }
            FuncBody::Expr(e) => {
                self.expr(e)?;
                if self.u().flags & CO_GENERATOR != 0 {
                    self.emit(Op::Pop);
                    self.load_const(Value::None);
                }
                self.emit(Op::Return);
            }
        }
        let code = self.finish();
        self.emit_closure_and_make(code, flags, &qual)
    }

    fn emit_closure_and_make(&mut self, code: Rc<Code>, mut flags: u8, qual: &str) -> Result<(), CompileError> {
        if !code.freevars.is_empty() {
            for f in code.freevars.clone().iter() {
                let Some(i) = self.deref_index(f) else {
                    return err(format!("internal: free variable '{f}' has no enclosing cell"), self.u().line);
                };
                self.emit(Op::LoadClosure(i));
            }
            self.emit(Op::BuildTuple(code.freevars.len() as u32));
            flags |= 8;
        }
        let cv = self.types.new_code(code);
        self.load_const(cv);
        self.str_const(qual);
        self.emit(Op::MakeFunction(flags));
        Ok(())
    }

    fn child_qualname(&self, name: &str) -> String {
        let u = self.units.last().unwrap();
        match u.kind {
            ScopeKind::Module => name.to_string(),
            ScopeKind::Class => format!("{}.{}", u.qualname, name),
            _ => format!("{}.<locals>.{}", u.qualname, name),
        }
    }

    fn class(&mut self, node: &'a Stmt, name: &str, bases: &'a [Expr], keywords: &'a [Keyword], body: &'a [Stmt]) -> Result<(), CompileError> {
        let scope = self.st.scope_of(node);
        self.emit(Op::LoadBuildClass);
        let qual = self.child_qualname(name);
        self.enter(scope, ScopeKind::Class, name, &qual, node.loc.line);
        self.name_op("__name__", Ctx::Load)?;
        self.name_op("__module__", Ctx::Store)?;
        self.str_const(&qual);
        self.name_op("__qualname__", Ctx::Store)?;
        self.docstring(body, true);
        self.stmts(body)?;
        if let Some(i) = self.deref_index("__class__") {
            self.emit(Op::LoadClosure(i));
            let n = self.name_index("__classcell__");
            self.emit(Op::StoreName(n));
        }
        self.load_const(Value::None);
        self.emit(Op::Return);
        let code = self.finish();
        self.emit_closure_and_make(code, 0, &qual)?;
        self.str_const(name);
        self.call_tail(bases, keywords, 2)?;
        Ok(())
    }

    fn expr(&mut self, e: &'a Expr) -> Result<(), CompileError> {
        if e.loc.line != 0 {
            self.u().line = e.loc.line;
        }
        match &e.kind {
            ExprKind::Constant(c) => {
                let v = self.constant(c);
                self.load_const(v);
            }
            ExprKind::Name { id, ctx } => self.name_op(id, *ctx)?,
            ExprKind::Attribute { value, attr, .. } => {
                self.expr(value)?;
                let i = self.name_index(attr);
                self.emit(Op::LoadAttr(i));
            }
            ExprKind::Subscript { value, slice, .. } => {
                self.expr(value)?;
                self.expr(slice)?;
                self.emit(Op::Subscr);
            }
            ExprKind::Slice { lower, upper, step } => {
                for part in [lower, upper] {
                    match part {
                        Some(x) => self.expr(x)?,
                        None => self.load_const(Value::None),
                    }
                }
                if let Some(s) = step {
                    self.expr(s)?;
                    self.emit(Op::BuildSlice(3));
                } else {
                    self.emit(Op::BuildSlice(2));
                }
            }
            ExprKind::BinOp { left, op, right } => {
                self.expr(left)?;
                self.expr(right)?;
                self.emit(Op::Binary(*op));
            }
            ExprKind::UnaryOp { op, operand } => {
                // Fold negative numeric literals so -9223372036854775808 fits.
                if *op == UnaryOp::USub {
                    if let ExprKind::Constant(Constant::Int(i)) = &operand.kind {
                        let v = self.constant(&Constant::Int(-i.clone()));
                        self.load_const(v);
                        return Ok(());
                    }
                    if let ExprKind::Constant(Constant::Float(f)) = &operand.kind {
                        self.load_const(Value::Float(-f));
                        return Ok(());
                    }
                }
                self.expr(operand)?;
                self.emit(Op::Unary(*op));
            }
            ExprKind::BoolOp { op, values } => {
                let end = self.label();
                for (i, v) in values.iter().enumerate() {
                    self.expr(v)?;
                    if i + 1 < values.len() {
                        self.emit(match op {
                            BoolOp::And => Op::JumpIfFalseOrPop(end),
                            BoolOp::Or => Op::JumpIfTrueOrPop(end),
                        });
                    }
                }
                self.bind(end);
            }
            ExprKind::Compare { left, ops, comparators } => {
                self.expr(left)?;
                if ops.len() == 1 {
                    self.expr(&comparators[0])?;
                    self.emit(Op::Compare(ops[0]));
                } else {
                    let cleanup = self.label();
                    let end = self.label();
                    for (i, (op, c)) in ops.iter().zip(comparators).enumerate() {
                        self.expr(c)?;
                        if i + 1 < ops.len() {
                            self.emit(Op::Dup);
                            self.emit(Op::Rot3);
                            self.emit(Op::Compare(*op));
                            self.emit(Op::JumpIfFalseOrPop(cleanup));
                        } else {
                            self.emit(Op::Compare(*op));
                        }
                    }
                    self.emit(Op::Jump(end));
                    self.bind(cleanup);
                    self.emit(Op::Rot2);
                    self.emit(Op::Pop);
                    self.bind(end);
                }
            }
            ExprKind::IfExp { test, body, orelse } => {
                let else_l = self.label();
                let end = self.label();
                self.expr(test)?;
                self.emit(Op::PopJumpIfFalse(else_l));
                self.expr(body)?;
                self.emit(Op::Jump(end));
                self.bind(else_l);
                self.expr(orelse)?;
                self.bind(end);
            }
            ExprKind::NamedExpr { target, value } => {
                self.expr(value)?;
                self.emit(Op::Dup);
                self.store(target)?;
            }
            ExprKind::Lambda { args, body } => {
                let scope = self.st.scope_of(e);
                self.make_function(scope, "<lambda>", args, FuncBody::Expr(body), e.loc.line)?;
            }
            ExprKind::Call { func, args, keywords } => {
                self.expr(func)?;
                self.call_tail(args, keywords, 0)?;
            }
            ExprKind::Tuple { elts, .. } => self.sequence(elts, SeqKind::Tuple)?,
            ExprKind::List { elts, .. } => self.sequence(elts, SeqKind::List)?,
            ExprKind::Set(elts) => self.sequence(elts, SeqKind::Set)?,
            ExprKind::Dict { keys, values } => {
                if keys.iter().all(|k| k.is_some()) {
                    for (k, v) in keys.iter().zip(values) {
                        self.expr(k.as_ref().unwrap())?;
                        self.expr(v)?;
                    }
                    self.emit(Op::BuildMap(keys.len() as u32));
                } else {
                    self.emit(Op::BuildMap(0));
                    for (k, v) in keys.iter().zip(values) {
                        match k {
                            Some(k) => {
                                self.expr(k)?;
                                self.expr(v)?;
                                self.emit(Op::MapAdd(0));
                            }
                            None => {
                                self.expr(v)?;
                                self.emit(Op::DictUpdate(0));
                            }
                        }
                    }
                }
            }
            ExprKind::Comp { kind, elt, generators } => {
                if *kind == CompKind::List && self.opts.dialect == Dialect::Py2 {
                    self.emit(Op::BuildList(0));
                    self.comp_loop(generators, 0, &CompBody::List(elt), true)?;
                } else {
                    self.comprehension(e, generators, match kind {
                        CompKind::List => CompBody::List(elt),
                        CompKind::Set => CompBody::Set(elt),
                        CompKind::Gen => CompBody::Gen(elt),
                    })?;
                }
            }
            ExprKind::DictComp { key, value, generators } => self.comprehension(e, generators, CompBody::Dict(key, value))?,
            ExprKind::Yield(v) => {
                match v {
                    Some(v) => self.expr(v)?,
                    None => self.load_const(Value::None),
                }
                self.emit(Op::Yield);
            }
            ExprKind::YieldFrom(v) | ExprKind::Await(v) => {
                self.expr(v)?;
                self.emit(Op::GetYieldFromIter);
                self.load_const(Value::None);
                self.emit(Op::YieldFrom);
            }
            ExprKind::FString(parts) => {
                let n = self.fparts(parts)?;
                if n != 1 {
                    self.emit(Op::BuildString(n));
                }
            }
            ExprKind::Starred { .. } => return err("can't use starred expression here", e.loc.line),
            ExprKind::Repr(v) => {
                self.expr(v)?;
                self.emit(Op::Repr);
            }
        }
        Ok(())
    }

    fn fparts(&mut self, parts: &'a [FPart]) -> Result<u32, CompileError> {
        let mut n = 0;
        for p in parts {
            match p {
                FPart::Lit(s) => {
                    self.str_const(s);
                }
                FPart::Field { value, conversion, spec, debug_text } => {
                    if let Some(t) = debug_text {
                        self.str_const(t);
                        n += 1;
                    }
                    self.expr(value)?;
                    let has_spec = spec.is_some();
                    if let Some(sp) = spec {
                        let k = self.fparts(sp)?;
                        if k != 1 {
                            self.emit(Op::BuildString(k));
                        }
                    }
                    let conv = match conversion {
                        Some('s') => b's',
                        Some('r') => b'r',
                        Some('a') => b'a',
                        _ => 0,
                    };
                    self.emit(Op::FormatValue(conv, has_spec));
                }
            }
            n += 1;
        }
        if n == 0 {
            self.str_const("");
            n = 1;
        }
        Ok(n)
    }

    /// Emits arguments and the call; `extra` positional values are already
    /// on the stack after the callable (used by class creation).
    fn call_tail(&mut self, args: &'a [Expr], keywords: &'a [Keyword], extra: u32) -> Result<(), CompileError> {
        let star = args.iter().any(|a| matches!(a.kind, ExprKind::Starred { .. })) || keywords.iter().any(|k| k.arg.is_none());
        if !star {
            for a in args {
                self.expr(a)?;
            }
            if keywords.is_empty() {
                self.emit(Op::Call(args.len() as u32 + extra));
            } else {
                let mut names = Vec::new();
                for k in keywords {
                    self.expr(&k.value)?;
                    names.push(self.types.new_str(k.arg.as_ref().unwrap()));
                }
                let t = self.types.new_tuple(names);
                self.load_const(t);
                self.emit(Op::CallKw(args.len() as u32 + keywords.len() as u32 + extra));
            }
            return Ok(());
        }
        // Generic path: build a positional tuple and a keyword dict.
        self.emit(Op::BuildList(extra));
        for a in args {
            match &a.kind {
                ExprKind::Starred { value, .. } => {
                    self.expr(value)?;
                    self.emit(Op::ListExtend(0));
                }
                _ => {
                    self.expr(a)?;
                    self.emit(Op::ListAppend(0));
                }
            }
        }
        self.emit(Op::ListToTuple);
        if keywords.is_empty() {
            self.emit(Op::CallEx(false));
            return Ok(());
        }
        self.emit(Op::BuildMap(0));
        for k in keywords {
            match &k.arg {
                Some(n) => {
                    self.str_const(n);
                    self.expr(&k.value)?;
                    self.emit(Op::MapAdd(0));
                }
                None => {
                    self.expr(&k.value)?;
                    self.emit(Op::DictMerge(0));
                }
            }
        }
        self.emit(Op::CallEx(true));
        Ok(())
    }

    fn sequence(&mut self, elts: &'a [Expr], kind: SeqKind) -> Result<(), CompileError> {
        let star = elts.iter().any(|a| matches!(a.kind, ExprKind::Starred { .. }));
        if !star {
            for x in elts {
                self.expr(x)?;
            }
            let n = elts.len() as u32;
            self.emit(match kind {
                SeqKind::Tuple => Op::BuildTuple(n),
                SeqKind::List => Op::BuildList(n),
                SeqKind::Set => Op::BuildSet(n),
            });
            return Ok(());
        }
        if kind == SeqKind::Set {
            self.emit(Op::BuildSet(0));
        } else {
            self.emit(Op::BuildList(0));
        }
        for x in elts {
            match &x.kind {
                ExprKind::Starred { value, .. } => {
                    self.expr(value)?;
                    self.emit(if kind == SeqKind::Set { Op::SetUpdate(0) } else { Op::ListExtend(0) });
                }
                _ => {
                    self.expr(x)?;
                    self.emit(if kind == SeqKind::Set { Op::SetAdd(0) } else { Op::ListAppend(0) });
                }
            }
        }
        if kind == SeqKind::Tuple {
            self.emit(Op::ListToTuple);
        }
        Ok(())
    }

    fn comprehension(&mut self, e: &'a Expr, gens: &'a [Comprehension], body: CompBody<'a>) -> Result<(), CompileError> {
        let scope = self.st.scope_of(e);
        let name = match body {
            CompBody::List(_) => "<listcomp>",
            CompBody::Set(_) => "<setcomp>",
            CompBody::Gen(_) => "<genexpr>",
            CompBody::Dict(..) => "<dictcomp>",
        };
        let qual = self.child_qualname(name);
        self.enter(scope, ScopeKind::Comprehension, name, &qual, e.loc.line);
        self.u().argcount = 1;
        match body {
            CompBody::List(_) => {
                self.emit(Op::BuildList(0));
            }
            CompBody::Set(_) => {
                self.emit(Op::BuildSet(0));
            }
            CompBody::Dict(..) => {
                self.emit(Op::BuildMap(0));
            }
            CompBody::Gen(_) => {}
        }
        self.comp_loop(gens, 0, &body, false)?;
        if matches!(body, CompBody::Gen(_)) {
            self.load_const(Value::None);
        }
        self.emit(Op::Return);
        let code = self.finish();
        self.emit_closure_and_make(code, 0, &qual)?;
        self.expr(&gens[0].iter)?;
        self.emit(Op::GetIter);
        self.emit(Op::Call(1));
        Ok(())
    }

    fn comp_loop(&mut self, gens: &'a [Comprehension], i: usize, body: &CompBody<'a>, inline: bool) -> Result<(), CompileError> {
        let g = &gens[i];
        let start = self.label();
        let end = self.label();
        if i == 0 && !inline {
            let idx = self.var_index(".0");
            self.emit(Op::LoadFast(idx));
        } else {
            self.expr(&g.iter)?;
            self.emit(Op::GetIter);
        }
        self.bind(start);
        self.emit(Op::ForIter(end));
        self.store(&g.target)?;
        for c in &g.ifs {
            self.expr(c)?;
            self.emit(Op::PopJumpIfFalse(start));
        }
        if i + 1 < gens.len() {
            self.comp_loop(gens, i + 1, body, inline)?;
        } else {
            let depth = (i + 1) as u32;
            match body {
                CompBody::List(elt) => {
                    self.expr(elt)?;
                    self.emit(Op::ListAppend(depth));
                }
                CompBody::Set(elt) => {
                    self.expr(elt)?;
                    self.emit(Op::SetAdd(depth));
                }
                CompBody::Dict(k, v) => {
                    self.expr(k)?;
                    self.expr(v)?;
                    self.emit(Op::MapAdd(depth));
                }
                CompBody::Gen(elt) => {
                    self.expr(elt)?;
                    self.emit(Op::Yield);
                    self.emit(Op::Pop);
                }
            }
        }
        self.emit(Op::Jump(start));
        self.bind(end);
        Ok(())
    }
}

enum FuncBody<'a> {
    Stmts(&'a [Stmt]),
    Expr(&'a Expr),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SeqKind {
    Tuple,
    List,
    Set,
}

enum CompBody<'a> {
    List(&'a Expr),
    Set(&'a Expr),
    Gen(&'a Expr),
    Dict(&'a Expr, &'a Expr),
}
