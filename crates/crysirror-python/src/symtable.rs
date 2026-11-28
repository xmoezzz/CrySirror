// SPDX-License-Identifier: MPL-2.0
//! Symbol tables: CPython's two-pass scope analysis (collect bindings and
//! uses per block, then resolve local/global/free/cell top-down). Scopes are
//! keyed by the address of their AST node, which stays fixed while the
//! compiler borrows the tree.

use crate::ast::*;
use crate::Dialect;
use std::collections::{HashMap, HashSet};

pub const DEF_LOCAL: u32 = 1;
pub const DEF_PARAM: u32 = 2;
pub const DEF_GLOBAL: u32 = 4;
pub const DEF_NONLOCAL: u32 = 8;
pub const USE: u32 = 16;
pub const DEF_IMPORT: u32 = 32;
/// Bound by a walrus inside a comprehension: belongs to the enclosing scope.
pub const DEF_COMP_ENCLOSING: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Module,
    Class,
    Function,
    Comprehension,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sym {
    Local,
    GlobalExplicit,
    GlobalImplicit,
    Free,
    Cell,
}

#[derive(Debug)]
pub struct Scope {
    pub kind: ScopeKind,
    pub name: String,
    pub flags: HashMap<String, u32>,
    /// Ordered parameter names, then other locals as first seen.
    pub varnames: Vec<String>,
    pub order: Vec<String>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    pub generator: bool,
    pub needs_class_cell: bool,
    pub resolved: HashMap<String, Sym>,
    /// Names that become cells here (sorted for determinism).
    pub cells: Vec<String>,
    pub frees: Vec<String>,
    /// Module-level and `exec` code without fast locals.
    pub has_star_import: bool,
}

pub struct SymTable {
    pub scopes: Vec<Scope>,
    pub by_node: HashMap<usize, usize>,
    pub dialect: Dialect,
}

impl SymTable {
    pub fn scope_of<T>(&self, node: &T) -> usize {
        self.by_node[&(node as *const T as usize)]
    }
}

#[derive(Debug)]
pub struct SymError(pub String, pub u32);

pub fn build_module(body: &[Stmt], dialect: Dialect) -> Result<SymTable, SymError> {
    let mut b = Builder { st: SymTable { scopes: Vec::new(), by_node: HashMap::new(), dialect }, cur: 0 };
    b.new_scope(ScopeKind::Module, "<module>", None, 0);
    for s in body {
        b.stmt(s)?;
    }
    b.analyze_root()?;
    Ok(b.st)
}

pub fn build_expr(expr: &Expr, dialect: Dialect) -> Result<SymTable, SymError> {
    let mut b = Builder { st: SymTable { scopes: Vec::new(), by_node: HashMap::new(), dialect }, cur: 0 };
    b.new_scope(ScopeKind::Module, "<module>", None, 0);
    b.expr(expr)?;
    b.analyze_root()?;
    Ok(b.st)
}

struct Builder {
    st: SymTable,
    cur: usize,
}

impl Builder {
    fn new_scope(&mut self, kind: ScopeKind, name: &str, parent: Option<usize>, key: usize) -> usize {
        let id = self.st.scopes.len();
        self.st.scopes.push(Scope {
            kind,
            name: name.to_string(),
            flags: HashMap::new(),
            varnames: Vec::new(),
            order: Vec::new(),
            children: Vec::new(),
            parent,
            generator: false,
            needs_class_cell: false,
            resolved: HashMap::new(),
            cells: Vec::new(),
            frees: Vec::new(),
            has_star_import: false,
        });
        if let Some(p) = parent {
            self.st.scopes[p].children.push(id);
        }
        if key != 0 {
            self.st.by_node.insert(key, id);
        }
        id
    }

    fn add(&mut self, name: &str, flag: u32) -> Result<(), SymError> {
        let mangled = name.to_string();
        let sc = &mut self.st.scopes[self.cur];
        let e = sc.flags.entry(mangled.clone()).or_insert(0);
        // Python 2 code compiled through the pure-Python `compiler` package
        // (Ren'Py through 6.10) accepts a repeated parameter name; the
        // later parameter keeps its slot and the name reads the first.
        if flag & DEF_PARAM != 0 && *e & DEF_PARAM != 0 && self.st.dialect != Dialect::Py2 {
            return Err(SymError(format!("duplicate argument '{name}' in function definition"), 0));
        }
        if *e == 0 {
            sc.order.push(mangled.clone());
        }
        *e |= flag;
        if flag & DEF_PARAM != 0 {
            sc.varnames.push(mangled);
        }
        Ok(())
    }

    fn walrus_target(&mut self, name: &str) -> Result<(), SymError> {
        if self.st.scopes[self.cur].kind != ScopeKind::Comprehension {
            return self.add(name, DEF_LOCAL);
        }
        // Bind in the nearest enclosing non-comprehension scope.
        let mut s = self.cur;
        while self.st.scopes[s].kind == ScopeKind::Comprehension {
            let sc = &mut self.st.scopes[s];
            let e = sc.flags.entry(name.to_string()).or_insert(0);
            if *e == 0 {
                sc.order.push(name.to_string());
            }
            *e |= DEF_COMP_ENCLOSING | USE;
            s = sc.parent.unwrap();
        }
        let saved = self.cur;
        self.cur = s;
        let r = if self.st.scopes[s].kind == ScopeKind::Class {
            Err(SymError("assignment expression within a comprehension cannot be used in a class body".into(), 0))
        } else {
            self.add(name, DEF_LOCAL)
        };
        self.cur = saved;
        r
    }

    fn stmts(&mut self, body: &[Stmt]) -> Result<(), SymError> {
        for s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn args_defaults(&mut self, a: &Arguments) -> Result<(), SymError> {
        for d in &a.defaults {
            self.expr(d)?;
        }
        for d in a.kw_defaults.iter().flatten() {
            self.expr(d)?;
        }
        for arg in a.posonly.iter().chain(&a.args).chain(&a.kwonly).chain(a.vararg.iter()).chain(a.kwarg.iter()) {
            if let Some(an) = &arg.annotation {
                self.expr(an)?;
            }
        }
        Ok(())
    }

    fn params(&mut self, a: &Arguments) -> Result<(), SymError> {
        for arg in a.posonly.iter().chain(&a.args) {
            self.add(&arg.name, DEF_PARAM)?;
        }
        for arg in &a.kwonly {
            self.add(&arg.name, DEF_PARAM)?;
        }
        if let Some(v) = &a.vararg {
            self.add(&v.name, DEF_PARAM)?;
        }
        if let Some(v) = &a.kwarg {
            self.add(&v.name, DEF_PARAM)?;
        }
        for arg in a.posonly.iter().chain(&a.args) {
            if let Some(t) = &arg.unpack {
                self.expr(t)?;
            }
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), SymError> {
        let line = s.loc.line;
        let r = self.stmt_inner(s);
        r.map_err(|SymError(m, l)| SymError(m, if l == 0 { line } else { l }))
    }

    fn stmt_inner(&mut self, s: &Stmt) -> Result<(), SymError> {
        match &s.kind {
            StmtKind::FunctionDef { name, args, body, decorators, returns, .. } => {
                for d in decorators {
                    self.expr(d)?;
                }
                self.args_defaults(args)?;
                if let Some(r) = returns {
                    self.expr(r)?;
                }
                self.add(name, DEF_LOCAL)?;
                let parent = self.cur;
                self.cur = self.new_scope(ScopeKind::Function, name, Some(parent), s as *const Stmt as usize);
                self.params(args)?;
                self.stmts(body)?;
                self.cur = parent;
            }
            StmtKind::ClassDef { name, bases, keywords, body, decorators } => {
                for d in decorators {
                    self.expr(d)?;
                }
                for b in bases {
                    self.expr(b)?;
                }
                for k in keywords {
                    self.expr(&k.value)?;
                }
                self.add(name, DEF_LOCAL)?;
                let parent = self.cur;
                self.cur = self.new_scope(ScopeKind::Class, name, Some(parent), s as *const Stmt as usize);
                self.stmts(body)?;
                self.cur = parent;
            }
            StmtKind::Return(v) => {
                if let Some(v) = v {
                    self.expr(v)?;
                }
            }
            StmtKind::Delete(t) => {
                for e in t {
                    self.expr(e)?;
                }
            }
            StmtKind::Assign { targets, value } => {
                self.expr(value)?;
                for t in targets {
                    self.expr(t)?;
                }
            }
            StmtKind::AugAssign { target, value, .. } => {
                self.expr(value)?;
                self.expr(target)?;
                if let ExprKind::Name { id, .. } = &target.kind {
                    self.add(id, USE)?;
                }
            }
            StmtKind::AnnAssign { target, annotation, value, simple } => {
                if let Some(v) = value {
                    self.expr(v)?;
                }
                if let ExprKind::Name { id, .. } = &target.kind {
                    if value.is_some() || *simple {
                        if value.is_some() {
                            self.add(id, DEF_LOCAL)?;
                        }
                    }
                } else {
                    self.expr(target)?;
                }
                if self.st.scopes[self.cur].kind != ScopeKind::Function {
                    self.expr(annotation)?;
                }
            }
            StmtKind::For { target, iter, body, orelse, .. } => {
                self.expr(iter)?;
                self.expr(target)?;
                self.stmts(body)?;
                self.stmts(orelse)?;
            }
            StmtKind::While { test, body, orelse } | StmtKind::If { test, body, orelse } => {
                self.expr(test)?;
                self.stmts(body)?;
                self.stmts(orelse)?;
            }
            StmtKind::With { items, body, .. } => {
                for it in items {
                    self.expr(&it.context)?;
                    if let Some(v) = &it.vars {
                        self.expr(v)?;
                    }
                }
                self.stmts(body)?;
            }
            StmtKind::Match { subject, cases } => {
                self.expr(subject)?;
                for c in cases {
                    self.pattern(&c.pattern)?;
                    if let Some(g) = &c.guard {
                        self.expr(g)?;
                    }
                    self.stmts(&c.body)?;
                }
            }
            StmtKind::Raise { exc, cause, py2_value, py2_tb } => {
                for e in [exc, cause, py2_value, py2_tb].into_iter().flatten() {
                    self.expr(e)?;
                }
            }
            StmtKind::Try { body, handlers, orelse, finalbody } => {
                self.stmts(body)?;
                for h in handlers {
                    if let Some(t) = &h.typ {
                        self.expr(t)?;
                    }
                    if let Some(n) = &h.name {
                        self.expr(n)?;
                    }
                    self.stmts(&h.body)?;
                }
                self.stmts(orelse)?;
                self.stmts(finalbody)?;
            }
            StmtKind::Assert { test, msg } => {
                self.expr(test)?;
                if let Some(m) = msg {
                    self.expr(m)?;
                }
            }
            StmtKind::Import(names) => {
                for a in names {
                    let bound = a.asname.clone().unwrap_or_else(|| a.name.split('.').next().unwrap().to_string());
                    self.add(&bound, DEF_IMPORT | DEF_LOCAL)?;
                }
            }
            StmtKind::ImportFrom { names, .. } => {
                for a in names {
                    if a.name == "*" {
                        self.st.scopes[self.cur].has_star_import = true;
                        continue;
                    }
                    let bound = a.asname.clone().unwrap_or_else(|| a.name.clone());
                    self.add(&bound, DEF_IMPORT | DEF_LOCAL)?;
                }
            }
            StmtKind::Global(names) => {
                for n in names {
                    self.add(n, DEF_GLOBAL)?;
                }
            }
            StmtKind::Nonlocal(names) => {
                if self.st.scopes[self.cur].kind == ScopeKind::Module {
                    return Err(SymError("nonlocal declaration not allowed at module level".into(), 0));
                }
                for n in names {
                    self.add(n, DEF_NONLOCAL)?;
                }
            }
            StmtKind::Expr(e) => self.expr(e)?,
            StmtKind::Pass | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Print { dest, values, .. } => {
                if let Some(d) = dest {
                    self.expr(d)?;
                }
                for v in values {
                    self.expr(v)?;
                }
            }
            StmtKind::Exec { body, globals, locals } => {
                self.expr(body)?;
                for e in [globals, locals].into_iter().flatten() {
                    self.expr(e)?;
                }
            }
        }
        Ok(())
    }

    fn pattern(&mut self, p: &Pattern) -> Result<(), SymError> {
        match p {
            Pattern::Value(e) => self.expr(e)?,
            Pattern::Singleton(_) => {}
            Pattern::Sequence(v) | Pattern::Or(v) => {
                for x in v {
                    self.pattern(x)?;
                }
            }
            Pattern::Mapping { keys, patterns, rest } => {
                for k in keys {
                    self.expr(k)?;
                }
                for x in patterns {
                    self.pattern(x)?;
                }
                if let Some(r) = rest {
                    self.add(r, DEF_LOCAL)?;
                }
            }
            Pattern::Class { cls, patterns, kwd_patterns, .. } => {
                self.expr(cls)?;
                for x in patterns.iter().chain(kwd_patterns) {
                    self.pattern(x)?;
                }
            }
            Pattern::Star(n) => {
                if let Some(n) = n {
                    self.add(n, DEF_LOCAL)?;
                }
            }
            Pattern::As { pattern, name } => {
                if let Some(p) = pattern {
                    self.pattern(p)?;
                }
                if let Some(n) = name {
                    self.add(n, DEF_LOCAL)?;
                }
            }
        }
        Ok(())
    }

    fn comprehension(&mut self, e: &Expr, gens: &[Comprehension], elts: &[&Expr]) -> Result<(), SymError> {
        // The outermost iterable is evaluated in the enclosing scope.
        self.expr(&gens[0].iter)?;
        let parent = self.cur;
        let name = match &e.kind {
            ExprKind::Comp { kind: CompKind::List, .. } => "<listcomp>",
            ExprKind::Comp { kind: CompKind::Set, .. } => "<setcomp>",
            ExprKind::Comp { kind: CompKind::Gen, .. } => "<genexpr>",
            _ => "<dictcomp>",
        };
        self.cur = self.new_scope(ScopeKind::Comprehension, name, Some(parent), e as *const Expr as usize);
        self.add(".0", DEF_PARAM)?;
        if matches!(e.kind, ExprKind::Comp { kind: CompKind::Gen, .. }) {
            self.st.scopes[self.cur].generator = true;
        }
        for (i, g) in gens.iter().enumerate() {
            if i > 0 {
                self.expr(&g.iter)?;
            }
            self.expr(&g.target)?;
            for c in &g.ifs {
                self.expr(c)?;
            }
        }
        for x in elts {
            self.expr(x)?;
        }
        self.cur = parent;
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> Result<(), SymError> {
        match &e.kind {
            ExprKind::Name { id, ctx } => {
                let flag = if *ctx == Ctx::Load { USE } else { DEF_LOCAL };
                self.add(id, flag)?;
                if *ctx == Ctx::Load && id == "super" {
                    self.mark_super();
                }
            }
            ExprKind::BoolOp { values, .. } => {
                for v in values {
                    self.expr(v)?;
                }
            }
            ExprKind::NamedExpr { target, value } => {
                self.expr(value)?;
                if let ExprKind::Name { id, .. } = &target.kind {
                    self.walrus_target(id)?;
                }
            }
            ExprKind::BinOp { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)?;
            }
            ExprKind::UnaryOp { operand, .. } => self.expr(operand)?,
            ExprKind::Lambda { args, body } => {
                self.args_defaults(args)?;
                let parent = self.cur;
                self.cur = self.new_scope(ScopeKind::Function, "<lambda>", Some(parent), e as *const Expr as usize);
                self.params(args)?;
                self.expr(body)?;
                self.cur = parent;
            }
            ExprKind::IfExp { test, body, orelse } => {
                self.expr(test)?;
                self.expr(body)?;
                self.expr(orelse)?;
            }
            ExprKind::Dict { keys, values } => {
                for k in keys.iter().flatten() {
                    self.expr(k)?;
                }
                for v in values {
                    self.expr(v)?;
                }
            }
            ExprKind::Set(v) => {
                for x in v {
                    self.expr(x)?;
                }
            }
            ExprKind::Comp { kind, elt, generators } => {
                if *kind == CompKind::List && self.st.dialect == Dialect::Py2 {
                    // Python 2 list comprehensions run in the enclosing scope.
                    for g in generators {
                        self.expr(&g.iter)?;
                        self.expr(&g.target)?;
                        for c in &g.ifs {
                            self.expr(c)?;
                        }
                    }
                    self.expr(elt)?;
                } else {
                    self.comprehension(e, generators, &[elt])?;
                }
            }
            ExprKind::DictComp { key, value, generators } => self.comprehension(e, generators, &[key, value])?,
            ExprKind::Await(v) | ExprKind::YieldFrom(v) => {
                self.st.scopes[self.cur].generator = true;
                self.expr(v)?;
            }
            ExprKind::Yield(v) => {
                if self.st.scopes[self.cur].kind == ScopeKind::Module || self.st.scopes[self.cur].kind == ScopeKind::Class {
                    return Err(SymError("'yield' outside function".into(), e.loc.line));
                }
                self.st.scopes[self.cur].generator = true;
                if let Some(v) = v {
                    self.expr(v)?;
                }
            }
            ExprKind::Compare { left, comparators, .. } => {
                self.expr(left)?;
                for c in comparators {
                    self.expr(c)?;
                }
            }
            ExprKind::Call { func, args, keywords } => {
                self.expr(func)?;
                for a in args {
                    self.expr(a)?;
                }
                for k in keywords {
                    self.expr(&k.value)?;
                }
            }
            ExprKind::FString(parts) => self.fparts(parts)?,
            ExprKind::Constant(_) => {}
            ExprKind::Attribute { value, .. } => self.expr(value)?,
            ExprKind::Subscript { value, slice, .. } => {
                self.expr(value)?;
                self.expr(slice)?;
            }
            ExprKind::Starred { value, .. } => self.expr(value)?,
            ExprKind::List { elts, .. } | ExprKind::Tuple { elts, .. } => {
                for x in elts {
                    self.expr(x)?;
                }
            }
            ExprKind::Slice { lower, upper, step } => {
                for x in [lower, upper, step].into_iter().flatten() {
                    self.expr(x)?;
                }
            }
            ExprKind::Repr(v) => self.expr(v)?,
        }
        Ok(())
    }

    fn fparts(&mut self, parts: &[FPart]) -> Result<(), SymError> {
        for p in parts {
            if let FPart::Field { value, spec, .. } = p {
                self.expr(value)?;
                if let Some(s) = spec {
                    self.fparts(s)?;
                }
            }
        }
        Ok(())
    }

    fn mark_super(&mut self) {
        // Zero-argument super() needs the implicit __class__ cell.
        if self.st.scopes[self.cur].kind == ScopeKind::Function {
            let _ = self.add("__class__", USE);
        }
    }

    fn analyze_root(&mut self) -> Result<(), SymError> {
        self.analyze(0, &HashSet::new(), &HashSet::new())?;
        Ok(())
    }

    /// Returns the free variables of `id` (names it needs from outside).
    fn analyze(&mut self, id: usize, bound: &HashSet<String>, globals: &HashSet<String>) -> Result<HashSet<String>, SymError> {
        let kind = self.st.scopes[id].kind;
        let names: Vec<(String, u32)> = self.st.scopes[id].order.iter().map(|n| (n.clone(), self.st.scopes[id].flags[n])).collect();
        let mut local = HashSet::new();
        let mut new_globals = globals.clone();
        let mut free = HashSet::new();
        for (name, flags) in &names {
            let sym = if flags & DEF_GLOBAL != 0 {
                new_globals.insert(name.clone());
                Sym::GlobalExplicit
            } else if flags & DEF_NONLOCAL != 0 {
                if !bound.contains(name) {
                    return Err(SymError(format!("no binding for nonlocal '{name}' found"), 0));
                }
                free.insert(name.clone());
                Sym::Free
            } else if flags & DEF_COMP_ENCLOSING != 0 && flags & (DEF_LOCAL | DEF_PARAM) == 0 {
                if bound.contains(name) {
                    free.insert(name.clone());
                    Sym::Free
                } else {
                    Sym::GlobalImplicit
                }
            } else if flags & (DEF_LOCAL | DEF_PARAM | DEF_IMPORT) != 0 {
                local.insert(name.clone());
                new_globals.remove(name);
                Sym::Local
            } else if kind != ScopeKind::Module && bound.contains(name) {
                free.insert(name.clone());
                Sym::Free
            } else if globals.contains(name) {
                Sym::GlobalExplicit
            } else {
                Sym::GlobalImplicit
            };
            self.st.scopes[id].resolved.insert(name.clone(), sym);
        }
        // Names visible to nested function scopes.
        let mut child_bound = bound.clone();
        match kind {
            ScopeKind::Function | ScopeKind::Comprehension => {
                for n in &local {
                    child_bound.insert(n.clone());
                }
            }
            ScopeKind::Class => {
                child_bound.insert("__class__".into());
            }
            ScopeKind::Module => {}
        }
        for n in &new_globals {
            child_bound.remove(n);
        }
        let children = self.st.scopes[id].children.clone();
        let mut child_free = HashSet::new();
        for c in children {
            let f = self.analyze(c, &child_bound, &new_globals)?;
            child_free.extend(f);
        }
        let mut cells = Vec::new();
        for name in child_free {
            if kind == ScopeKind::Class && name == "__class__" {
                self.st.scopes[id].needs_class_cell = true;
                continue;
            }
            match self.st.scopes[id].resolved.get(&name).copied() {
                Some(Sym::Local) if kind != ScopeKind::Class && kind != ScopeKind::Module => {
                    self.st.scopes[id].resolved.insert(name.clone(), Sym::Cell);
                    cells.push(name);
                }
                Some(Sym::Cell) => {}
                Some(Sym::GlobalExplicit) | Some(Sym::GlobalImplicit) if kind != ScopeKind::Module && kind != ScopeKind::Class && !bound.contains(&name) => {}
                _ if kind == ScopeKind::Module => {}
                _ => {
                    // Pass through: implicit free variable of this scope.
                    if bound.contains(&name) || name == "__class__" {
                        if kind == ScopeKind::Class {
                            // A class body passes free variables to methods
                            // without reading them itself.
                            if !self.st.scopes[id].resolved.contains_key(&name) {
                                self.st.scopes[id].resolved.insert(name.clone(), Sym::Free);
                            } else if self.st.scopes[id].resolved[&name] != Sym::Local {
                                self.st.scopes[id].resolved.insert(name.clone(), Sym::Free);
                            } else {
                                // Class-local shadows: methods still see the outer cell.
                                self.st.scopes[id].frees.push(name.clone());
                                free.insert(name);
                                continue;
                            }
                        } else {
                            self.st.scopes[id].resolved.insert(name.clone(), Sym::Free);
                        }
                        free.insert(name);
                    }
                }
            }
        }
        cells.sort();
        let sc = &mut self.st.scopes[id];
        sc.cells = cells;
        let mut frees: Vec<String> = free.iter().cloned().collect();
        frees.extend(sc.frees.drain(..));
        frees.sort();
        frees.dedup();
        sc.frees = frees;
        if kind == ScopeKind::Class && sc.needs_class_cell && !sc.cells.contains(&"__class__".to_string()) {
            sc.cells.push("__class__".into());
        }
        Ok(free)
    }
}
