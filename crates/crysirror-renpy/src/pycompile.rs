// SPDX-License-Identifier: MPL-2.0
//! `renpy.python.py_compile`: Ren'Py compiles embedded Python with the
//! `print_function`/`unicode_literals` grammar first and the classic Python 2
//! grammar as fallback, wraps list/dict/set displays and comprehensions in
//! `__renpy__list__`/`__renpy__dict__`/`__renpy__set__` (revertable
//! containers for rollback) and offsets line numbers to the script line.

use crate::lexer::PyExpr;
use crysirror_python::ast::*;
use crysirror_python::compile::{CompileOptions, compile_eval, compile_module};
use crysirror_python::parser::{
    ParseOptions, parse_expression, parse_module, parse_module_py2_fallback,
};
use crysirror_python::{Dialect, Vm};
use std::rc::Rc;

fn wrap(name: &str, e: Expr) -> Expr {
    let loc = e.loc.clone();
    Expr::new(
        loc.clone(),
        ExprKind::Call {
            func: Box::new(Expr::new(
                loc,
                ExprKind::Name {
                    id: name.into(),
                    ctx: Ctx::Load,
                },
            )),
            args: vec![e],
            keywords: Vec::new(),
        },
    )
}

fn w_expr(e: &mut Expr) {
    // Children first.
    match &mut e.kind {
        ExprKind::BoolOp { values, .. } => values.iter_mut().for_each(w_expr),
        ExprKind::NamedExpr { value, .. } => w_expr(value),
        ExprKind::BinOp { left, right, .. } => {
            w_expr(left);
            w_expr(right);
        }
        ExprKind::UnaryOp { operand, .. } => w_expr(operand),
        ExprKind::Lambda { body, args } => {
            w_args(args);
            w_expr(body);
        }
        ExprKind::IfExp { test, body, orelse } => {
            w_expr(test);
            w_expr(body);
            w_expr(orelse);
        }
        ExprKind::Dict { keys, values } => {
            keys.iter_mut().flatten().for_each(w_expr);
            values.iter_mut().for_each(w_expr);
        }
        ExprKind::Set(v) => v.iter_mut().for_each(w_expr),
        ExprKind::Comp {
            elt, generators, ..
        } => {
            w_expr(elt);
            w_gens(generators);
        }
        ExprKind::DictComp {
            key,
            value,
            generators,
        } => {
            w_expr(key);
            w_expr(value);
            w_gens(generators);
        }
        ExprKind::Await(v) | ExprKind::YieldFrom(v) | ExprKind::Repr(v) => w_expr(v),
        ExprKind::Yield(v) => {
            if let Some(v) = v {
                w_expr(v);
            }
        }
        ExprKind::Compare {
            left, comparators, ..
        } => {
            w_expr(left);
            comparators.iter_mut().for_each(w_expr);
        }
        ExprKind::Call {
            func,
            args,
            keywords,
        } => {
            w_expr(func);
            args.iter_mut().for_each(w_expr);
            keywords.iter_mut().for_each(|k| w_expr(&mut k.value));
        }
        ExprKind::FString(parts) => w_fparts(parts),
        ExprKind::Attribute { value, .. } => w_expr(value),
        ExprKind::Subscript { value, slice, .. } => {
            w_expr(value);
            w_expr(slice);
        }
        ExprKind::Starred { value, .. } => w_expr(value),
        ExprKind::List { elts, .. } | ExprKind::Tuple { elts, .. } => {
            elts.iter_mut().for_each(w_expr)
        }
        ExprKind::Slice { lower, upper, step } => {
            for x in [lower, upper, step].into_iter().flatten() {
                w_expr(x);
            }
        }
        ExprKind::Constant(_) | ExprKind::Name { .. } => {}
    }
    let name = match &e.kind {
        ExprKind::List { ctx: Ctx::Load, .. }
        | ExprKind::Comp {
            kind: CompKind::List,
            ..
        } => Some("__renpy__list__"),
        ExprKind::Dict { .. } | ExprKind::DictComp { .. } => Some("__renpy__dict__"),
        ExprKind::Comp {
            kind: CompKind::Set,
            ..
        } => Some("__renpy__set__"),
        _ => None,
    };
    if let Some(n) = name {
        let placeholder = Expr::new(e.loc.clone(), ExprKind::Constant(Constant::None));
        let inner = std::mem::replace(e, placeholder);
        *e = wrap(n, inner);
    }
}

fn w_fparts(parts: &mut [FPart]) {
    for p in parts {
        if let FPart::Field { value, spec, .. } = p {
            w_expr(value);
            if let Some(s) = spec {
                w_fparts(s);
            }
        }
    }
}

fn w_gens(g: &mut [Comprehension]) {
    for c in g {
        w_expr(&mut c.iter);
        c.ifs.iter_mut().for_each(w_expr);
    }
}

fn w_args(a: &mut Arguments) {
    a.defaults.iter_mut().for_each(w_expr);
    a.kw_defaults.iter_mut().flatten().for_each(w_expr);
}

fn w_stmts(b: &mut [Stmt]) {
    for s in b {
        w_stmt(s);
    }
}

fn w_stmt(s: &mut Stmt) {
    match &mut s.kind {
        StmtKind::FunctionDef {
            args,
            body,
            decorators,
            ..
        } => {
            decorators.iter_mut().for_each(w_expr);
            w_args(args);
            w_stmts(body);
        }
        StmtKind::ClassDef {
            bases,
            keywords,
            body,
            decorators,
            ..
        } => {
            decorators.iter_mut().for_each(w_expr);
            bases.iter_mut().for_each(w_expr);
            keywords.iter_mut().for_each(|k| w_expr(&mut k.value));
            w_stmts(body);
        }
        StmtKind::Return(Some(v)) | StmtKind::Expr(v) => w_expr(v),
        StmtKind::Delete(v) => v.iter_mut().for_each(w_expr),
        StmtKind::Assign { targets, value } => {
            targets.iter_mut().for_each(w_expr);
            w_expr(value);
        }
        StmtKind::AugAssign { target, value, .. } => {
            w_expr(target);
            w_expr(value);
        }
        StmtKind::AnnAssign { value, .. } => {
            if let Some(v) = value {
                w_expr(v);
            }
        }
        StmtKind::For {
            target,
            iter,
            body,
            orelse,
            ..
        } => {
            w_expr(target);
            w_expr(iter);
            w_stmts(body);
            w_stmts(orelse);
        }
        StmtKind::While { test, body, orelse } | StmtKind::If { test, body, orelse } => {
            w_expr(test);
            w_stmts(body);
            w_stmts(orelse);
        }
        StmtKind::With { items, body, .. } => {
            for it in items.iter_mut() {
                w_expr(&mut it.context);
            }
            w_stmts(body);
        }
        StmtKind::Raise {
            exc,
            cause,
            py2_value,
            py2_tb,
        } => {
            for e in [exc, cause, py2_value, py2_tb].into_iter().flatten() {
                w_expr(e);
            }
        }
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            w_stmts(body);
            for h in handlers.iter_mut() {
                if let Some(t) = h.typ.as_mut() {
                    w_expr(t);
                }
                w_stmts(&mut h.body);
            }
            w_stmts(orelse);
            w_stmts(finalbody);
        }
        StmtKind::Assert { test, msg } => {
            w_expr(test);
            if let Some(m) = msg {
                w_expr(m);
            }
        }
        StmtKind::Print { dest, values, .. } => {
            if let Some(d) = dest {
                w_expr(d);
            }
            values.iter_mut().for_each(w_expr);
        }
        StmtKind::Exec {
            body,
            globals,
            locals,
        } => {
            w_expr(body);
            for e in [globals, locals].into_iter().flatten() {
                w_expr(e);
            }
        }
        StmtKind::Match { subject, cases } => {
            w_expr(subject);
            for c in cases.iter_mut() {
                w_stmts(&mut c.body);
            }
        }
        _ => {}
    }
}

/// Compiles `src` in mode `exec`/`hide` (statements) or `eval`.
pub fn compile(
    vm: &Vm,
    src: &PyExpr,
    mode: &str,
    dialect: Dialect,
) -> Result<Rc<crysirror_python::code::Code>, String> {
    let text = src.text.replace('\r', "");
    let filename = src.filename.to_string();
    let line = src.line.max(1);
    let err = |m: String, l: u32| format!("File \"{filename}\", line {}: {m}", l + line - 1);
    let mut opts = CompileOptions::new(dialect, &filename);
    opts.line_offset = line - 1;
    if mode == "eval" {
        let t = text.trim();
        let po = match dialect {
            Dialect::Py3 => ParseOptions::py3(),
            Dialect::Py2 => ParseOptions::py2(),
        };
        let mut e = parse_expression(t, po).map_err(|e| err(e.msg, e.line))?;
        w_expr(&mut e);
        return compile_eval(&vm.types, &e, opts).map_err(|e| err(e.msg, e.line));
    }
    let mut body = match dialect {
        Dialect::Py3 => parse_module(&text, ParseOptions::py3()),
        Dialect::Py2 => parse_module_py2_fallback(&text),
    }
    .map_err(|e| err(e.msg, e.line))?;
    w_stmts(&mut body);
    if mode == "hide" {
        body = wrap_hide(body);
    }
    compile_module(&vm.types, &body, opts).map_err(|e| err(e.msg, e.line))
}

/// `wrap_hide`: hidden Python runs as the body of `_execute_python_hide()`,
/// so its names are function locals (closures and comprehensions see them).
fn wrap_hide(body: Vec<crysirror_python::ast::Stmt>) -> Vec<crysirror_python::ast::Stmt> {
    use crysirror_python::ast::{Ctx, Expr, ExprKind, Stmt, StmtKind};
    let loc = body
        .first()
        .map(|s| s.loc.clone())
        .unwrap_or(crysirror_python::ast::Loc { line: 1, col: 0 });
    let name = "_execute_python_hide".to_string();
    let def = Stmt {
        loc: loc.clone(),
        kind: StmtKind::FunctionDef {
            name: name.clone(),
            args: Box::default(),
            body,
            decorators: Vec::new(),
            returns: None,
            is_async: false,
        },
    };
    let func = Expr {
        loc: loc.clone(),
        kind: ExprKind::Name {
            id: name,
            ctx: Ctx::Load,
        },
    };
    let call = Expr {
        loc: loc.clone(),
        kind: ExprKind::Call {
            func: Box::new(func),
            args: Vec::new(),
            keywords: Vec::new(),
        },
    };
    vec![
        def,
        Stmt {
            loc,
            kind: StmtKind::Expr(call),
        },
    ]
}

/// Bind the native ParameterInfo lists used by SL2 and ATL.
pub(crate) fn bind_parameters(
    vm: &mut Vm,
    signature: &crate::ast::Signature,
    globals: &crysirror_python::ObjRef,
    location: &crate::atl::Loc,
    args: &[crysirror_python::Value],
    kwargs: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    let scope = vm.types.new_dict();
    let mut args = args.iter().cloned();
    let mut kwargs = kwargs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>();
    for parameter in &signature.parameters {
        let value = match parameter.kind {
            0 => args.next(),
            1 => match (args.next(), kwargs.remove(parameter.name.as_str())) {
                (Some(_), Some(_)) => {
                    return Err(
                        vm.type_error(format!("parameter {} supplied twice", parameter.name))
                    );
                }
                (pos, kw) => pos.or(kw),
            },
            2 => Some(vm.types.new_tuple(args.by_ref().collect())),
            3 => kwargs.remove(parameter.name.as_str()),
            4 => {
                let rest = vm.types.new_dict();
                for (key, value) in std::mem::take(&mut kwargs) {
                    rest.obj()
                        .unwrap()
                        .dict_set_raw(&vm.types, vm.types.new_str(&key), value);
                }
                Some(rest)
            }
            _ => return Err(vm.type_error("invalid Ren'Py parameter")),
        };
        let value = match (value, &parameter.default) {
            (Some(value), _) => value,
            (None, Some(default)) => {
                let expr = PyExpr::new(default.as_str(), location.0.clone(), location.1);
                let code = compile(vm, &expr, "eval", crate::engine::state(vm).profile.dialect)
                    .map_err(|error| vm.syntax_error(crysirror_python::Error::Runtime(error)))?;
                vm.run_code(code, globals.clone(), Some(scope.clone()))?
            }
            _ => return Err(vm.type_error(format!("missing parameter {}", parameter.name))),
        };
        scope
            .obj()
            .unwrap()
            .dict_set_raw(&vm.types, vm.types.new_str(&parameter.name), value);
    }
    if args.next().is_some() || !kwargs.is_empty() {
        return Err(vm.type_error("unexpected Ren'Py arguments"));
    }
    Ok(scope)
}
