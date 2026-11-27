// SPDX-License-Identifier: MPL-2.0
//! Private name mangling (`__x` inside `class C` becomes `_C__x`), applied to
//! a copy of the tree before symbol analysis, as CPython does while compiling.

use crate::ast::*;

fn mangle(name: &mut String, private: Option<&str>) {
    let Some(p) = private else { return };
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return;
    }
    let p = p.trim_start_matches('_');
    if p.is_empty() {
        return;
    }
    *name = format!("_{p}{name}");
}

pub fn mangle_module(body: &mut [Stmt]) {
    for s in body {
        stmt(s, None);
    }
}

fn stmts(body: &mut [Stmt], p: Option<&str>) {
    for s in body {
        stmt(s, p);
    }
}

fn args(a: &mut Arguments, p: Option<&str>) {
    for d in a.defaults.iter_mut() {
        mangle_expr(d, p);
    }
    for d in a.kw_defaults.iter_mut().flatten() {
        mangle_expr(d, p);
    }
    for arg in a.posonly.iter_mut().chain(a.args.iter_mut()).chain(a.kwonly.iter_mut()).chain(a.vararg.iter_mut()).chain(a.kwarg.iter_mut()) {
        mangle(&mut arg.name, p);
        if let Some(an) = arg.annotation.as_mut() {
            mangle_expr(an, p);
        }
        if let Some(u) = arg.unpack.as_mut() {
            mangle_expr(u, p);
        }
    }
}

fn stmt(s: &mut Stmt, p: Option<&str>) {
    match &mut s.kind {
        StmtKind::FunctionDef { name, args: a, body, decorators, returns, .. } => {
            for d in decorators.iter_mut() {
                mangle_expr(d, p);
            }
            mangle(name, p);
            args(a, p);
            if let Some(r) = returns {
                mangle_expr(r, p);
            }
            stmts(body, p);
        }
        StmtKind::ClassDef { name, bases, keywords, body, decorators } => {
            for d in decorators.iter_mut() {
                mangle_expr(d, p);
            }
            for b in bases.iter_mut() {
                mangle_expr(b, p);
            }
            for k in keywords.iter_mut() {
                mangle_expr(&mut k.value, p);
            }
            let inner = name.clone();
            mangle(name, p);
            stmts(body, Some(&inner));
        }
        StmtKind::Return(v) => {
            if let Some(v) = v {
                mangle_expr(v, p);
            }
        }
        StmtKind::Delete(v) => {
            for e in v.iter_mut() {
                mangle_expr(e, p);
            }
        }
        StmtKind::Assign { targets, value } => {
            for t in targets.iter_mut() {
                mangle_expr(t, p);
            }
            mangle_expr(value, p);
        }
        StmtKind::AugAssign { target, value, .. } => {
            mangle_expr(target, p);
            mangle_expr(value, p);
        }
        StmtKind::AnnAssign { target, annotation, value, .. } => {
            mangle_expr(target, p);
            mangle_expr(annotation, p);
            if let Some(v) = value {
                mangle_expr(v, p);
            }
        }
        StmtKind::For { target, iter, body, orelse, .. } => {
            mangle_expr(target, p);
            mangle_expr(iter, p);
            stmts(body, p);
            stmts(orelse, p);
        }
        StmtKind::While { test, body, orelse } | StmtKind::If { test, body, orelse } => {
            mangle_expr(test, p);
            stmts(body, p);
            stmts(orelse, p);
        }
        StmtKind::With { items, body, .. } => {
            for it in items.iter_mut() {
                mangle_expr(&mut it.context, p);
                if let Some(v) = it.vars.as_mut() {
                    mangle_expr(v, p);
                }
            }
            stmts(body, p);
        }
        StmtKind::Match { subject, cases } => {
            mangle_expr(subject, p);
            for c in cases.iter_mut() {
                if let Some(g) = c.guard.as_mut() {
                    mangle_expr(g, p);
                }
                stmts(&mut c.body, p);
            }
        }
        StmtKind::Raise { exc, cause, py2_value, py2_tb } => {
            for e in [exc, cause, py2_value, py2_tb].into_iter().flatten() {
                mangle_expr(e, p);
            }
        }
        StmtKind::Try { body, handlers, orelse, finalbody } => {
            stmts(body, p);
            for h in handlers.iter_mut() {
                if let Some(t) = h.typ.as_mut() {
                    mangle_expr(t, p);
                }
                if let Some(n) = h.name.as_mut() {
                    mangle_expr(n, p);
                }
                stmts(&mut h.body, p);
            }
            stmts(orelse, p);
            stmts(finalbody, p);
        }
        StmtKind::Assert { test, msg } => {
            mangle_expr(test, p);
            if let Some(m) = msg {
                mangle_expr(m, p);
            }
        }
        StmtKind::Import(names) | StmtKind::ImportFrom { names, .. } => {
            for a in names.iter_mut() {
                if let Some(asn) = a.asname.as_mut() {
                    mangle(asn, p);
                }
            }
        }
        StmtKind::Global(names) | StmtKind::Nonlocal(names) => {
            for n in names.iter_mut() {
                mangle(n, p);
            }
        }
        StmtKind::Expr(e) => mangle_expr(e, p),
        StmtKind::Print { dest, values, .. } => {
            if let Some(d) = dest {
                mangle_expr(d, p);
            }
            for v in values.iter_mut() {
                mangle_expr(v, p);
            }
        }
        StmtKind::Exec { body, globals, locals } => {
            mangle_expr(body, p);
            for e in [globals, locals].into_iter().flatten() {
                mangle_expr(e, p);
            }
        }
        StmtKind::Pass | StmtKind::Break | StmtKind::Continue => {}
    }
}

pub fn mangle_expr(e: &mut Expr, p: Option<&str>) {
    if p.is_none() && !has_class(e) {
        return;
    }
    match &mut e.kind {
        ExprKind::Name { id, .. } => mangle(id, p),
        ExprKind::Attribute { value, attr, .. } => {
            mangle_expr(value, p);
            mangle(attr, p);
        }
        ExprKind::BoolOp { values, .. } => {
            for v in values.iter_mut() {
                mangle_expr(v, p);
            }
        }
        ExprKind::NamedExpr { target, value } => {
            mangle_expr(target, p);
            mangle_expr(value, p);
        }
        ExprKind::BinOp { left, right, .. } => {
            mangle_expr(left, p);
            mangle_expr(right, p);
        }
        ExprKind::UnaryOp { operand, .. } => mangle_expr(operand, p),
        ExprKind::Lambda { args: a, body } => {
            args(a, p);
            mangle_expr(body, p);
        }
        ExprKind::IfExp { test, body, orelse } => {
            mangle_expr(test, p);
            mangle_expr(body, p);
            mangle_expr(orelse, p);
        }
        ExprKind::Dict { keys, values } => {
            for k in keys.iter_mut().flatten() {
                mangle_expr(k, p);
            }
            for v in values.iter_mut() {
                mangle_expr(v, p);
            }
        }
        ExprKind::Set(v) | ExprKind::List { elts: v, .. } | ExprKind::Tuple { elts: v, .. } => {
            for x in v.iter_mut() {
                mangle_expr(x, p);
            }
        }
        ExprKind::Comp { elt, generators, .. } => {
            mangle_expr(elt, p);
            gens(generators, p);
        }
        ExprKind::DictComp { key, value, generators } => {
            mangle_expr(key, p);
            mangle_expr(value, p);
            gens(generators, p);
        }
        ExprKind::Await(v) | ExprKind::YieldFrom(v) | ExprKind::Repr(v) => mangle_expr(v, p),
        ExprKind::Yield(v) => {
            if let Some(v) = v {
                mangle_expr(v, p);
            }
        }
        ExprKind::Compare { left, comparators, .. } => {
            mangle_expr(left, p);
            for c in comparators.iter_mut() {
                mangle_expr(c, p);
            }
        }
        ExprKind::Call { func, args: a, keywords } => {
            mangle_expr(func, p);
            for x in a.iter_mut() {
                mangle_expr(x, p);
            }
            for k in keywords.iter_mut() {
                mangle_expr(&mut k.value, p);
            }
        }
        ExprKind::FString(parts) => fparts(parts, p),
        ExprKind::Constant(_) => {}
        ExprKind::Subscript { value, slice, .. } => {
            mangle_expr(value, p);
            mangle_expr(slice, p);
        }
        ExprKind::Starred { value, .. } => mangle_expr(value, p),
        ExprKind::Slice { lower, upper, step } => {
            for x in [lower, upper, step].into_iter().flatten() {
                mangle_expr(x, p);
            }
        }
    }
}

fn fparts(parts: &mut [FPart], p: Option<&str>) {
    for part in parts {
        if let FPart::Field { value, spec, .. } = part {
            mangle_expr(value, p);
            if let Some(s) = spec {
                fparts(s, p);
            }
        }
    }
}

fn gens(g: &mut [Comprehension], p: Option<&str>) {
    for c in g {
        mangle_expr(&mut c.target, p);
        mangle_expr(&mut c.iter, p);
        for i in c.ifs.iter_mut() {
            mangle_expr(i, p);
        }
    }
}

/// Expressions cannot contain class statements; mangling only applies under one.
fn has_class(_e: &Expr) -> bool {
    false
}
