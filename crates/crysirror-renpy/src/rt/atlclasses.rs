// SPDX-License-Identifier: MPL-2.0
//! The statement classes of `renpy.atl`, after Ren'Py 6.18–8.6 `atl.py`
//! (MIT), as Python-visible views of the Rust ATL program.
//!
//! Raw statements (`RawMultipurpose`, `RawRepeat`, ... ) are what the
//! parser produces; compiled statements (`Interpolation`, `Repeat`,
//! `Child`, ...) are what `RawBlock.compile` produces. Both are views: an
//! object holds the Rust statement it describes and answers upstream's
//! field names (`warper`, `duration`, `properties`, `blocks`, ...), and
//! `RawBlock.statements` / `Block.statements` list them. `isinstance`
//! against the upstream class hierarchy (`RawStatement`, `Statement`)
//! holds. Execution stays in the Rust runtime (`rt::atlrt`).
//!
//! Also here: `parse_atl(lexer)` and the module helpers
//! (`correct_type`, `deep_compare`, `check_spline_types`,
//! `get_catmull_rom_value`, `incompatible_props`, `compatible_pairs`,
//! `tuple_or_list`, `late_imports`, `is_constant_expr`, `NOT_CONST`).
use super::{
    atlrt::{self, Block, Stmt},
    py::{self},
};
use crate::atl::{RawBlock, RawMultipurpose, RawStatement};
use crate::lexer::PyExpr;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.atl";

struct RawView(Rc<RawStatement>);
struct StmtView(Rc<Block>, usize);

fn loc_value(vm: &Vm, loc: &(Rc<str>, u32)) -> Value {
    let f = vm.types.new_str(&loc.0);
    vm.types.new_tuple(vec![f, Value::Int(loc.1 as i64)])
}

fn expr(vm: &Vm, e: &PyExpr) -> Value {
    vm.types.new_str(&e.text)
}

fn opt_expr(vm: &Vm, e: &Option<PyExpr>) -> Value {
    e.as_ref().map(|e| expr(vm, e)).unwrap_or(Value::None)
}

fn class_named(vm: &mut Vm, name: &str) -> PyResult<ObjRef> {
    let c = py::import_attr(vm, MODULE, name)?;
    c.obj().cloned().ok_or_else(|| vm.type_error(name.to_owned()))
}

fn raw_class_name(s: &RawStatement) -> &'static str {
    match s {
        RawStatement::Block(_) => "RawBlock",
        RawStatement::Multipurpose(_) => "RawMultipurpose",
        RawStatement::Repeat { .. } => "RawRepeat",
        RawStatement::Parallel { .. } => "RawParallel",
        RawStatement::Choice { .. } => "RawChoice",
        RawStatement::On { .. } => "RawOn",
        RawStatement::Time { .. } => "RawTime",
        RawStatement::Event { .. } => "RawEvent",
        RawStatement::Function { .. } => "RawFunction",
        RawStatement::ContainsExpr { .. } => "RawContainsExpr",
        RawStatement::Child { .. } => "RawChild",
    }
}

/// A Python view of one raw statement (blocks become `RawBlock`s).
pub fn raw_statement_value(vm: &mut Vm, s: &RawStatement) -> PyResult<Value> {
    if let RawStatement::Block(b) = s {
        return atlrt::raw_value(vm, Rc::new(b.clone()));
    }
    let c = class_named(vm, raw_class_name(s))?;
    Ok(vm.types.new_native(&c, Box::new(RawView(Rc::new(s.clone())))))
}

fn raw_block_value(vm: &mut Vm, b: &RawBlock) -> PyResult<Value> {
    atlrt::raw_value(vm, Rc::new(b.clone()))
}

/// `RawBlock.statements`.
pub fn raw_block_statements(vm: &mut Vm, b: &RawBlock) -> PyResult<Value> {
    let mut out = Vec::new();
    for s in &b.statements {
        out.push(raw_statement_value(vm, s)?);
    }
    Ok(vm.types.new_list(out))
}

fn multipurpose_attr(vm: &mut Vm, m: &RawMultipurpose, name: &str) -> PyResult<Option<Value>> {
    Ok(Some(match name {
        "warper" => m.warper.as_deref().map(|w| py::s(vm, w)).unwrap_or(Value::None),
        "duration" => opt_expr(vm, &m.duration),
        "warp_function" => opt_expr(vm, &m.warp_function),
        "properties" => {
            let items = m.properties.iter().map(|(n, e)| {
                let n = vm.types.new_str(n);
                vm.types.new_tuple(vec![n, expr(vm, e)])
            });
            let items = items.collect();
            vm.types.new_list(items)
        }
        "expressions" => {
            let items = m
                .expressions
                .iter()
                .map(|(e, w)| vm.types.new_tuple(vec![expr(vm, e), opt_expr(vm, w)]))
                .collect();
            vm.types.new_list(items)
        }
        "splines" => {
            let items = m
                .splines
                .iter()
                .map(|(n, es)| {
                    let n = vm.types.new_str(n);
                    let es = es.iter().map(|e| expr(vm, e)).collect();
                    let es = vm.types.new_list(es);
                    vm.types.new_tuple(vec![n, es])
                })
                .collect();
            vm.types.new_list(items)
        }
        "revolution" => m.revolution.as_deref().map(|w| py::s(vm, w)).unwrap_or(Value::None),
        "circles" => m.circles.as_ref().map(|e| expr(vm, e)).unwrap_or_else(|| py::s(vm, "0")),
        "loc" => m.loc.as_ref().map(|l| loc_value(vm, l)).unwrap_or(Value::None),
        _ => return Ok(None),
    }))
}

fn raw_view_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::native::<RawView, _>(&args[0], |v| v.0.clone()).ok_or_else(|| vm.type_error("raw ATL statement"))?;
    let name = args.get(1).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let found: Option<Value> = match &*s {
        RawStatement::Multipurpose(m) => multipurpose_attr(vm, m, &name)?,
        RawStatement::Repeat { loc, repeats } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "repeats" => Some(opt_expr(vm, repeats)),
            _ => None,
        },
        RawStatement::Parallel { loc, blocks } | RawStatement::Child { loc, children: blocks } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "blocks" | "children" => {
                let mut out = Vec::new();
                for b in blocks {
                    out.push(raw_block_value(vm, b)?);
                }
                Some(vm.types.new_list(out))
            }
            _ => None,
        },
        RawStatement::Choice { loc, choices } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "choices" => {
                let mut out = Vec::new();
                for (c, b) in choices {
                    let b = raw_block_value(vm, b)?;
                    out.push(vm.types.new_tuple(vec![expr(vm, c), b]));
                }
                Some(vm.types.new_list(out))
            }
            _ => None,
        },
        RawStatement::On { loc, handlers } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "handlers" => {
                let d = py::dict(vm);
                for (n, b) in handlers {
                    let b = raw_block_value(vm, b)?;
                    py::dict_set(vm, &d, n, b)?;
                }
                Some(d)
            }
            _ => None,
        },
        RawStatement::Time { loc, time } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "time" => Some(expr(vm, time)),
            _ => None,
        },
        RawStatement::Event { loc, name: n } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "name" => Some(py::s(vm, n)),
            _ => None,
        },
        RawStatement::Function { loc, expr: e } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "expr" => Some(expr(vm, e)),
            _ => None,
        },
        RawStatement::ContainsExpr { loc, expr: e } => match name.as_str() {
            "loc" => Some(loc_value(vm, loc)),
            "expression" => Some(expr(vm, e)),
            _ => None,
        },
        RawStatement::Block(_) => None,
    };
    match found {
        Some(v) => Ok(v),
        None if name == "constant" => Ok(Value::None),
        None => Err(vm.attr_error(format!("'{}' object has no attribute '{name}'", raw_class_name(&s)))),
    }
}

/// `RawStatement.compile(ctx)`: compiles the statement as a one-statement
/// block and returns its compiled statement.
fn raw_view_compile(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::native::<RawView, _>(&args[0], |v| v.0.clone()).ok_or_else(|| vm.type_error("raw ATL statement"))?;
    let loc = raw_loc(&s);
    let block = RawBlock { loc, statements: vec![(*s).clone()], animation: false };
    let raw = atlrt::raw_value(vm, Rc::new(block))?;
    let ctx = args.get(1).cloned().unwrap_or(Value::None);
    let compiled = py::method(vm, &raw, "compile", vec![ctx])?;
    let stmts = block_statements(vm, std::slice::from_ref(&compiled), &[])?;
    Ok(py::items(vm, &stmts)?.into_iter().next().unwrap_or(Value::None))
}

fn raw_loc(s: &RawStatement) -> (Rc<str>, u32) {
    match s {
        RawStatement::Block(b) => b.loc.clone(),
        RawStatement::Multipurpose(m) => m.loc.clone().unwrap_or_else(|| ("<atl>".into(), 0)),
        RawStatement::Repeat { loc, .. }
        | RawStatement::Parallel { loc, .. }
        | RawStatement::Choice { loc, .. }
        | RawStatement::On { loc, .. }
        | RawStatement::Time { loc, .. }
        | RawStatement::Event { loc, .. }
        | RawStatement::Function { loc, .. }
        | RawStatement::ContainsExpr { loc, .. }
        | RawStatement::Child { loc, .. } => loc.clone(),
    }
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn raw_view_eq(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::native::<RawView, _>(&args[0], |v| v.0.clone());
    let b = py::native::<RawView, _>(&args[1], |v| v.0.clone());
    Ok(Value::Bool(match (a, b) {
        (Some(a), Some(b)) => format!("{a:?}") == format!("{b:?}"),
        _ => false,
    }))
}

// Compiled statements ----------------------------------------------------------------------------

fn stmt_class_name(s: &Stmt) -> &'static str {
    match s {
        Stmt::Block(_) => "Block",
        Stmt::Interp(_) => "Interpolation",
        Stmt::Repeat(_) => "Repeat",
        Stmt::Parallel(_) => "Parallel",
        Stmt::Choice(_) => "Choice",
        Stmt::Time => "Time",
        Stmt::On(_) => "On",
        Stmt::Event(_) => "Event",
        Stmt::Function(_) => "Function",
        Stmt::Child { .. } => "Child",
    }
}

fn block_value(vm: &mut Vm, b: &Rc<Block>) -> PyResult<Value> {
    let c = class_named(vm, "Block")?;
    Ok(vm.types.new_native(&c, Box::new(atlrt::BlockRef(b.clone()))))
}

/// `Block.statements`.
fn block_statements(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = atlrt::block_of(&args[0]).ok_or_else(|| vm.type_error("Block"))?;
    let mut out = Vec::new();
    for (i, s) in b.stmts.iter().enumerate() {
        let v = match s {
            Stmt::Block(inner) => block_value(vm, inner)?,
            _ => {
                let c = class_named(vm, stmt_class_name(s))?;
                vm.types.new_native(&c, Box::new(StmtView(b.clone(), i)))
            }
        };
        out.push(v);
    }
    Ok(vm.types.new_list(out))
}

fn block_times(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = atlrt::block_of(&args[0]).ok_or_else(|| vm.type_error("Block"))?;
    let items = b
        .times
        .iter()
        .map(|(t, i)| vm.types.new_tuple(vec![py::f(*t), Value::Int(*i as i64)]))
        .collect();
    Ok(vm.types.new_list(items))
}

fn blocks_list(vm: &mut Vm, bs: &[Rc<Block>]) -> PyResult<Value> {
    let mut out = Vec::new();
    for b in bs {
        out.push(block_value(vm, b)?);
    }
    Ok(vm.types.new_list(out))
}

fn stmt_view_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (b, i) = py::native::<StmtView, _>(&args[0], |v| (v.0.clone(), v.1)).ok_or_else(|| vm.type_error("ATL statement"))?;
    let name = args.get(1).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let s = &b.stmts[i];
    let found = match (s, name.as_str()) {
        (Stmt::Interp(it), "warper") => Some(match &it.warper_name {
            Some(n) => py::s(vm, n),
            None => it.warper.clone(),
        }),
        (Stmt::Interp(it), "duration") => Some(py::f(it.duration)),
        (Stmt::Interp(it), "properties") => {
            let items = it
                .properties
                .iter()
                .map(|(n, v)| {
                    let n = vm.types.new_str(n);
                    vm.types.new_tuple(vec![n, v.clone()])
                })
                .collect();
            Some(vm.types.new_list(items))
        }
        (Stmt::Interp(it), "revolution") => Some(it.revolution.as_deref().map(|r| py::s(vm, r)).unwrap_or(Value::None)),
        (Stmt::Interp(it), "circles") => Some(py::f(it.circles)),
        (Stmt::Interp(it), "splines") => {
            let items = it
                .splines
                .iter()
                .map(|(n, vs)| {
                    let n = vm.types.new_str(n);
                    let l = vm.types.new_list(vs.clone());
                    vm.types.new_tuple(vec![n, l])
                })
                .collect();
            Some(vm.types.new_list(items))
        }
        (Stmt::Repeat(r), "repeats") => Some(r.map(Value::Int).unwrap_or(Value::None)),
        (Stmt::Parallel(bs), "blocks") => Some(blocks_list(vm, bs)?),
        (Stmt::Choice(cs), "choices") => {
            let mut out = Vec::new();
            for (c, blk) in cs {
                let blk = block_value(vm, blk)?;
                out.push(vm.types.new_tuple(vec![py::f(*c), blk]));
            }
            Some(vm.types.new_list(out))
        }
        (Stmt::On(hs), "handlers") => {
            let d = py::dict(vm);
            for (n, blk) in hs {
                let blk = block_value(vm, blk)?;
                py::dict_set(vm, &d, n, blk)?;
            }
            Some(d)
        }
        (Stmt::Time, "time") => b.times.iter().find(|(_, j)| *j == i).map(|(t, _)| py::f(*t)),
        (Stmt::Event(n), "name") => Some(py::s(vm, n)),
        (Stmt::Function(f), "function") => Some(f.clone()),
        (Stmt::Child { child, .. }, "child") => Some(child.clone()),
        (Stmt::Child { transition, .. }, "transition") => Some(transition.clone()),
        (_, "loc") => Some(Value::None),
        _ => None,
    };
    found.ok_or_else(|| vm.attr_error(format!("'{}' object has no attribute '{name}'", stmt_class_name(s))))
}

// parse_atl and helpers ------------------------------------------------------------------------

/// `parse_atl(l)`: parses the block of lexer `l` into a `RawBlock`.
fn f_parse_atl(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cx = super::statements::ParseCx::new(vm);
    let r = super::lexer::with(vm, &args[0], |l| crate::atl::parse_atl(l, &cx))?;
    match r {
        Ok(b) => atlrt::raw_value(vm, Rc::new(b)),
        Err(e) => Err(super::lexer::parse_error(vm, &e)),
    }
}

fn f_correct_type(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (v, b, ty) = (&args[0], &args[1], &args[2]);
    let position = py::import_attr(vm, MODULE, "position")?;
    if py::same(ty, &position) {
        if py::is_none(v) {
            return Ok(Value::None);
        }
        let t = Value::Obj(ops::type_of(vm, b));
        return vm.call(&t, vec![v.clone()], Vec::new());
    }
    vm.call(ty, vec![v.clone()], Vec::new())
}

fn f_check_spline_types(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = &args[0];
    if matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_)) {
        return Ok(Value::Bool(true));
    }
    let position = py::import_attr(vm, MODULE, "position")?;
    if let Some(c) = position.obj() {
        if ops::isinstance(vm, v, c) {
            return Ok(Value::Bool(true));
        }
    }
    if ops::isinstance(vm, v, &vm.types.int.clone()) || ops::isinstance(vm, v, &vm.types.float.clone()) {
        return Ok(Value::Bool(true));
    }
    if ops::isinstance(vm, v, &vm.types.tuple.clone()) {
        for i in py::items(vm, v)? {
            if !matches!(f_check_spline_types(vm, &[i], &[])?, Value::Bool(true)) {
                return Ok(Value::Bool(false));
            }
        }
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

/// Catmull-Rom with no alpha, as upstream (`type(p0)` of the result).
fn f_get_catmull_rom_value(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::f64_of(vm, &args[0])?.clamp(0.0, 1.0);
    let p: Vec<f64> = args[1..5].iter().map(|v| py::f64_of(vm, v)).collect::<PyResult<_>>()?;
    let r = (t * ((2.0 - t) * t - 1.0) * p[0]
        + (t * t * (3.0 * t - 5.0) + 2.0) * p[1]
        + t * ((4.0 - 3.0 * t) * t + 1.0) * p[2]
        + (t - 1.0) * t * t * p[3])
        / 2.0;
    let ty = Value::Obj(ops::type_of(vm, &args[2]));
    vm.call(&ty, vec![py::f(r)], Vec::new())
}

/// `deep_compare(a, b)`: structural equality of statement trees.
fn f_deep_compare(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(deep_compare(vm, &args[0], &args[1])?))
}

fn deep_compare(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<bool> {
    if !Rc::ptr_eq(&ops::type_of(vm, a), &ops::type_of(vm, b)) {
        return Ok(false);
    }
    let list_t = vm.types.list.clone();
    let tuple_t = vm.types.tuple.clone();
    if ops::isinstance(vm, a, &list_t) || ops::isinstance(vm, a, &tuple_t) {
        let (x, y) = (py::items(vm, a)?, py::items(vm, b)?);
        for (i, j) in x.iter().zip(&y) {
            if !deep_compare(vm, i, j)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let dict_t = vm.types.dict.clone();
    if ops::isinstance(vm, a, &dict_t) {
        let (ka, kb) = (py::items(vm, a)?, py::items(vm, b)?);
        if ka.len() != kb.len() {
            return Ok(false);
        }
        for k in ka {
            if !ops::contains(vm, b, &k)? {
                return Ok(false);
            }
            let x = ops::getitem(vm, a, k.clone())?;
            let y = ops::getitem(vm, b, k)?;
            if !deep_compare(vm, &x, &y)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let rv = py::native::<RawView, _>(a, |v| v.0.clone());
    let rw = py::native::<RawView, _>(b, |v| v.0.clone());
    if let (Some(x), Some(y)) = (rv, rw) {
        return Ok(format!("{x:?}") == format!("{y:?}"));
    }
    if let (Some(x), Some(y)) = (atlrt::raw_of(a), atlrt::raw_of(b)) {
        return Ok(format!("{x:?}") == format!("{y:?}"));
    }
    Ok(ops::eq(vm, a, b).unwrap_or(true))
}

fn f_late_imports(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `is_constant_expr(expr)` (≤ 7.4): the pyanalysis constness of `expr`.
fn f_is_constant_expr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::import_attr(vm, "renpy.pyanalysis", "Analysis")?;
    let a = vm.call(&a, Vec::new(), Vec::new())?;
    py::method(vm, &a, "is_constant_expr", vec![args[0].clone()])
}

fn incompatible_props(vm: &mut Vm) -> PyResult<Value> {
    const TABLE: &[(&str, &[&str])] = &[
        ("alignaround", &["xaround", "yaround", "xanchoraround", "yanchoraround"]),
        ("align", &["xanchor", "yanchor", "xpos", "ypos"]),
        ("anchor", &["xanchor", "yanchor"]),
        ("angle", &["xpos", "ypos"]),
        ("anchorangle", &["xangle", "yangle"]),
        ("around", &["xaround", "yaround", "xanchoraround", "yanchoraround"]),
        ("offset", &["xoffset", "yoffset"]),
        ("pos", &["xpos", "ypos"]),
        ("radius", &["xpos", "ypos"]),
        ("anchorradius", &["xanchor", "yanchor"]),
        ("size", &["xsize", "ysize"]),
        ("xalign", &["xpos", "xanchor"]),
        ("xcenter", &["xpos", "xanchor"]),
        ("xycenter", &["xpos", "ypos", "xanchor", "yanchor"]),
        ("xysize", &["xsize", "ysize"]),
        ("yalign", &["ypos", "yanchor"]),
        ("ycenter", &["ypos", "yanchor"]),
    ];
    let d = py::dict(vm);
    for (k, vs) in TABLE {
        let set = vm.types.new_set(Default::default());
        for v in *vs {
            ops::set_add(vm, &set, py::s(vm, v))?;
        }
        py::dict_set(vm, &d, k, set)?;
    }
    Ok(d)
}

fn str_set(vm: &mut Vm, items: &[&str]) -> PyResult<Value> {
    let set = vm.types.new_set(Default::default());
    for v in items {
        ops::set_add(vm, &set, py::s(vm, v))?;
    }
    Ok(set)
}

const RAW_CLASSES: &[&str] = &[
    "RawMultipurpose",
    "RawRepeat",
    "RawParallel",
    "RawChoice",
    "RawOn",
    "RawTime",
    "RawEvent",
    "RawFunction",
    "RawContainsExpr",
    "RawChild",
];

const STMT_CLASSES: &[&str] = &["Child", "Interpolation", "Repeat", "Parallel", "Choice", "Time", "On", "Event", "Function"];

/// Adds the statement classes and helpers to `renpy.atl` (called from
/// `atlrt` after `RawBlock`/`Block` exist).
pub fn add_to_module(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let profile = super::state::try_rt(vm).map(|r| r.profile.clone());
    let before = |a: u32, b: u32| profile.as_ref().is_some_and(|p| !p.at_least(a, b));
    let raw_base = py::class(vm, MODULE, "RawStatement", &[], &[("predict", noop), ("mark_constant", noop)], &[("constant", Value::None)])?;
    set(vm, "RawStatement", Value::Obj(raw_base.clone()));
    for n in RAW_CLASSES {
        let c = vm.types.new_builtin_type(n, &raw_base, crysirror_python::object::BaseKind::Native, true);
        vm.types.add_method(&c, "__getattr__", raw_view_getattr);
        vm.types.add_method(&c, "compile", raw_view_compile);
        vm.types.add_method(&c, "__eq__", raw_view_eq);
        let mn = vm.types.new_str(MODULE);
        vm.types.set_attr(&c, "__module__", mn);
        set(vm, n, Value::Obj(c));
    }
    let stmt_base = py::class(vm, MODULE, "Statement", &[], &[], &[])?;
    set(vm, "Statement", Value::Obj(stmt_base.clone()));
    for n in STMT_CLASSES {
        let c = vm.types.new_builtin_type(n, &stmt_base, crysirror_python::object::BaseKind::Native, true);
        vm.types.add_method(&c, "__getattr__", stmt_view_getattr);
        let mn = vm.types.new_str(MODULE);
        vm.types.set_attr(&c, "__module__", mn);
        set(vm, n, Value::Obj(c));
    }
    if let Ok(block) = py::import_attr(vm, MODULE, "Block") {
        if let Some(c) = block.obj() {
            vm.types.add_method(c, "statements", block_statements);
            let st = py::get(vm, &block, "statements")?;
            let st = py::property(vm, st, None)?;
            vm.types.set_attr(c, "statements", st);
            let t = vm.types.new_builtin("times", block_times);
            let t = py::property(vm, t, None)?;
            vm.types.set_attr(c, "times", t);
        }
    }
    let funcs: &[(&'static str, crysirror_python::object::NativeFn)] = &[
        ("parse_atl", f_parse_atl),
        ("correct_type", f_correct_type),
        ("get_catmull_rom_value", f_get_catmull_rom_value),
        ("deep_compare", f_deep_compare),
        ("check_spline_types", f_check_spline_types),
        ("late_imports", f_late_imports),
    ];
    for (n, f) in funcs {
        let v = py::builtin(vm, n, *f);
        set(vm, n, v);
    }
    if before(7, 5) {
        let v = py::builtin(vm, "is_constant_expr", f_is_constant_expr);
        set(vm, "is_constant_expr", v);
    }
    let ip = incompatible_props(vm)?;
    set(vm, "incompatible_props", ip);
    let a = str_set(vm, &["radius", "angle"])?;
    let b = str_set(vm, &["anchorradius", "anchorangle"])?;
    set(vm, "compatible_pairs", vm.types.new_list(vec![a, b]));
    let tl = vm.types.new_tuple(vec![Value::Obj(vm.types.tuple.clone()), Value::Obj(vm.types.list.clone())]);
    set(vm, "tuple_or_list", tl);
    set(vm, "NOT_CONST", Value::Int(0));
    Ok(())
}
