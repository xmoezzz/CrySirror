// SPDX-License-Identifier: MPL-2.0
//! The Python-visible node classes of `renpy.sl2.slast` (`SLNode`,
//! `SLBlock`, `SLDisplayable`, `SLIf`, `SLShowIf`, `SLFor`, `SLPython`,
//! `SLPass`, `SLDefault`, `SLUse`, `SLTransclude`, `SLCustomUse`, `SLBreak`,
//! `SLContinue`) over the Rust screen AST.
//!
//! Node objects are views: an `SLScreen` (or a node) hands out its
//! `children`, `entries` and `block` as objects naming a path inside the
//! screen's shared AST, so the attributes upstream's nodes carry
//! (`location`, `serial`, `positional`, `keyword`, `variable`,
//! `expression`, `target`, `args`, ...) read the parsed screen. The
//! behavioural methods run the screen executor on that node:
//! `execute(context)` and `keywords(context)` take an `SLContext` and
//! update its `children`/`keywords`/caches exactly as a screen update
//! does; `used_screens`, `has_transclude`, `has_python` and
//! `keyword_exist` walk the node. Constant analysis (`analyze`, `prepare`)
//! has no effect: the executor evaluates every node as upstream's
//! not-constant AST (see `slast.rs`), and `copy` returns the node, which is
//! immutable. Nodes are made by the screen parser; constructing one from
//! Python raises.
use super::py::{self, *};
use super::slast::{Ctx, ScreenAst, ast_of};
use crate::sl2::{Block, SlNode};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};
use std::{cell::Cell, rc::Rc};

pub const MODULE: &str = "renpy.sl2.slast";

/// Node classes that are not module names in the running release.
struct Hidden(Vec<(&'static str, ObjRef)>);

#[derive(Clone, Copy, Debug)]
pub enum Step {
    Child(usize),
    Entry(usize),
    Body,
}

#[derive(Clone)]
pub struct NodeView {
    pub ast: Rc<ScreenAst>,
    pub path: Vec<Step>,
}

pub enum Target<'a> {
    Block(&'a Block),
    Node(&'a SlNode),
}

fn node_block(n: &SlNode) -> Option<&Block> {
    match n {
        SlNode::Displayable(d) => Some(&d.block),
        SlNode::For { block, .. } => Some(block),
        SlNode::Use { block, .. } => block.as_ref(),
        SlNode::CustomUse { block, .. } => Some(block),
        _ => None,
    }
}

pub fn resolve<'a>(root: &'a Block, path: &[Step]) -> Option<Target<'a>> {
    let mut cur = Target::Block(root);
    for s in path {
        cur = match (cur, s) {
            (Target::Block(b), Step::Child(i)) => Target::Node(b.children.get(*i)?),
            (Target::Node(SlNode::If { entries, .. }), Step::Entry(j))
            | (Target::Node(SlNode::ShowIf { entries, .. }), Step::Entry(j)) => {
                Target::Block(&entries.get(*j)?.1)
            }
            (Target::Node(n), Step::Body) => Target::Block(node_block(n)?),
            _ => return None,
        };
    }
    Some(cur)
}

/// The view behind an `SLScreen` (the root block) or a node object.
pub fn view_of(v: &Value) -> Option<NodeView> {
    if let Some(ast) = ast_of(v) {
        return Some(NodeView {
            ast,
            path: Vec::new(),
        });
    }
    py::native::<NodeView, _>(v, |n| n.clone())
}

fn this(vm: &Vm, v: &Value) -> PyResult<NodeView> {
    view_of(v).ok_or_else(|| vm.type_error("screen-language node expected"))
}

fn class_name(t: &Target) -> &'static str {
    match t {
        Target::Block(_) => "SLBlock",
        Target::Node(n) => match n {
            SlNode::Displayable(_) => "SLDisplayable",
            SlNode::If { .. } => "SLIf",
            SlNode::ShowIf { .. } => "SLShowIf",
            SlNode::For { .. } => "SLFor",
            SlNode::Python { .. } => "SLPython",
            SlNode::Pass { .. } => "SLPass",
            SlNode::Default { .. } => "SLDefault",
            SlNode::Use { .. } => "SLUse",
            SlNode::Transclude { .. } => "SLTransclude",
            SlNode::CustomUse { .. } => "SLCustomUse",
            SlNode::Break { .. } => "SLBreak",
            SlNode::Continue { .. } => "SLContinue",
        },
    }
}

fn make(vm: &mut Vm, ast: &Rc<ScreenAst>, path: Vec<Step>) -> PyResult<Value> {
    let name = match resolve(&ast.screen.block, &path) {
        Some(t) => class_name(&t),
        None => return Err(py::error(vm, "stale screen-language node")),
    };
    let hidden = vm.ext_get::<Hidden>().and_then(|h| {
        h.0.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| c.clone())
    });
    let c = match hidden {
        Some(c) => c,
        None => {
            let c = py::import_attr(vm, MODULE, name)?;
            c.obj().cloned().ok_or_else(|| vm.type_error(name))?
        }
    };
    Ok(vm.types.new_native(
        &c,
        Box::new(NodeView {
            ast: ast.clone(),
            path,
        }),
    ))
}

fn loc_of(t: &Target, ast: &ScreenAst) -> (Rc<str>, u32) {
    match t {
        Target::Block(_) => ast.screen.loc.clone(),
        Target::Node(n) => match n {
            SlNode::Displayable(d) => d.loc.clone(),
            SlNode::If { loc, .. }
            | SlNode::ShowIf { loc, .. }
            | SlNode::For { loc, .. }
            | SlNode::Python { loc, .. }
            | SlNode::Pass { loc }
            | SlNode::Default { loc, .. }
            | SlNode::Use { loc, .. }
            | SlNode::Transclude { loc }
            | SlNode::CustomUse { loc, .. }
            | SlNode::Break { loc }
            | SlNode::Continue { loc } => loc.clone(),
        },
    }
}

fn expr(vm: &mut Vm, e: &crate::lexer::PyExpr) -> PyResult<Value> {
    super::lexer::pyexpr(vm, &e.text, &e.filename, e.line)
}

fn opt_expr(vm: &mut Vm, e: &Option<crate::lexer::PyExpr>) -> PyResult<Value> {
    match e {
        Some(e) => expr(vm, e),
        None => Ok(Value::None),
    }
}

fn block_path(t: &Target, path: &[Step]) -> Option<Vec<Step>> {
    match t {
        Target::Block(_) => Some(path.to_vec()),
        Target::Node(n) => node_block(n).map(|_| {
            let mut p = path.to_vec();
            p.push(Step::Body);
            p
        }),
    }
}

fn block_keyword(vm: &mut Vm, b: &Block) -> PyResult<Value> {
    let mut v = Vec::new();
    for (k, e) in &b.keyword {
        let e = expr(vm, e)?;
        v.push(vm.types.new_tuple(vec![py::s(vm, k), e]));
    }
    Ok(vm.types.new_list(v))
}

fn children_of(vm: &mut Vm, ast: &Rc<ScreenAst>, bp: &[Step], b: &Block) -> PyResult<Value> {
    let mut v = Vec::new();
    for i in 0..b.children.len() {
        let mut p = bp.to_vec();
        p.push(Step::Child(i));
        v.push(make(vm, ast, p)?);
    }
    Ok(vm.types.new_list(v))
}

/// `__getattr__`: the upstream node attributes.
fn n_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let name = py::string(vm, &args[1])?;
    let ast = view.ast.clone();
    let Some(t) = resolve(&ast.screen.block, &view.path) else {
        return Err(py::error(vm, "stale screen-language node"));
    };
    let missing = |vm: &mut Vm| -> PyResult<Value> {
        Err(vm.exc_msg(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", class_name(&t), name),
        ))
    };
    match name.as_str() {
        "location" => {
            let l = loc_of(&t, &ast);
            return Ok(vm.types.new_tuple(vec![py::s(vm, &l.0), Value::Int(l.1 as i64)]));
        }
        "serial" => {
            let s = match &t {
                Target::Block(b) => *b as *const Block as usize,
                Target::Node(n) => *n as *const SlNode as usize,
            };
            return Ok(Value::Int(s as i64));
        }
        "constant" => return super::slast::const_value(vm, "NOT_CONST"),
        "has_keyword" | "last_keyword" => {
            if let Some(bp) = block_path(&t, &view.path) {
                let b = match resolve(&ast.screen.block, &bp) {
                    Some(Target::Block(b)) => b,
                    _ => return missing(vm),
                };
                return Ok(Value::Bool(super::slast::block_has_keyword(b)));
            }
            return Ok(Value::Bool(false));
        }
        _ => {}
    }
    if let Some(bp) = block_path(&t, &view.path) {
        let Some(Target::Block(b)) = resolve(&ast.screen.block, &bp) else {
            return missing(vm);
        };
        match name.as_str() {
            "keyword" => return block_keyword(vm, b),
            "children" => return children_of(vm, &ast, &bp, b),
            "atl_transform" => {
                return match &b.atl_transform {
                    Some(_) => ast.atl_parent_value(vm, b),
                    None => Ok(Value::None),
                };
            }
            "transform" => {
                return match &b.atl_transform {
                    Some(_) => ast.atl_parent_value(vm, b),
                    None => Ok(Value::None),
                };
            }
            _ => {}
        }
    }
    let Target::Node(n) = t else {
        return missing(vm);
    };
    match (n, name.as_str()) {
        (SlNode::Displayable(d), _) => {
            let rt = ast.node_rt(vm, d)?;
            let s = &rt.spec;
            match name.as_str() {
                "displayable" => Ok(rt.factory.clone()),
                "positional" => {
                    let mut v = Vec::new();
                    for e in &d.positional {
                        v.push(expr(vm, e)?);
                    }
                    Ok(vm.types.new_list(v))
                }
                "style" => Ok(s.style.as_ref().map(|x| py::s(vm, x)).unwrap_or(Value::None)),
                "scope" => Ok(Value::Bool(s.scope)),
                "child_or_fixed" => Ok(Value::Bool(s.nchildren == crate::sl2::NChildren::One)),
                "pass_context" => Ok(Value::Bool(s.pass_context)),
                "imagemap" => Ok(Value::Bool(s.imagemap)),
                "hotspot" => Ok(Value::Bool(s.hotspot)),
                "replaces" => Ok(Value::Bool(s.replaces)),
                "unique" => Ok(Value::Bool(s.unique)),
                "name" => Ok(py::s(vm, &s.name)),
                "text_style" => Ok(Value::None),
                "variable" => Ok(d
                    .block
                    .variable
                    .as_ref()
                    .map(|x| py::s(vm, x))
                    .unwrap_or(Value::None)),
                "default_keywords" => {
                    let dk = vm.types.new_dict();
                    for (k, e) in &s.default_keywords {
                        let v = super::params::eval_text(vm, e, &d.loc, &Value::None)?;
                        py::dict_set(vm, &dk, k, v)?;
                    }
                    Ok(dk)
                }
                _ => missing(vm),
            }
        }
        (SlNode::If { entries, .. }, "entries") | (SlNode::ShowIf { entries, .. }, "entries") => {
            let mut v = Vec::new();
            for (j, (cond, _)) in entries.iter().enumerate() {
                let c = opt_expr(vm, cond)?;
                let mut p = view.path.clone();
                p.push(Step::Entry(j));
                let b = make(vm, &ast, p)?;
                v.push(vm.types.new_tuple(vec![c, b]));
            }
            Ok(vm.types.new_list(v))
        }
        (
            SlNode::For {
                variable,
                expression,
                index_expression,
                ..
            },
            _,
        ) => match name.as_str() {
            "variable" => Ok(py::s(vm, variable)),
            "expression" => expr(vm, expression),
            "index_expression" => opt_expr(vm, index_expression),
            _ => missing(vm),
        },
        (SlNode::Python { code, .. }, "code") => {
            let c = py::import_attr(vm, "renpy.ast", "PyCode")?;
            let src = expr(vm, code)?;
            let loc = vm.types.new_tuple(vec![py::s(vm, &code.filename), Value::Int(code.line as i64)]);
            vm.call(&c, vec![src, loc, py::s(vm, "exec")], Vec::new())
        }
        (SlNode::Default { variable, expression, .. }, _) => match name.as_str() {
            "variable" => Ok(py::s(vm, variable)),
            "expression" => expr(vm, expression),
            _ => missing(vm),
        },
        (
            SlNode::Use {
                target,
                expression,
                args,
                id,
                block,
                variable,
                loc,
            },
            _,
        ) => match name.as_str() {
            "target" => {
                if *expression {
                    let e = crate::lexer::PyExpr::new(target.as_str(), loc.0.clone(), loc.1);
                    expr(vm, &e)
                } else {
                    Ok(py::s(vm, target))
                }
            }
            "args" => match args {
                Some(a) => super::parameter::argument_info_value(vm, a, loc),
                None => Ok(Value::None),
            },
            "id" => opt_expr(vm, id),
            "block" => match block {
                Some(_) => {
                    let mut p = view.path.clone();
                    p.push(Step::Body);
                    make(vm, &ast, p)
                }
                None => Ok(Value::None),
            },
            "variable" => Ok(variable.as_ref().map(|x| py::s(vm, x)).unwrap_or(Value::None)),
            "ast" => {
                if *expression {
                    return Ok(Value::None);
                }
                let n = py::s(vm, target);
                let f = py::import_attr(vm, "renpy.display.screen", "get_screen_variant")?;
                let sc = vm.call(&f, vec![n], Vec::new())?;
                if py::is_none(&sc) {
                    Ok(Value::None)
                } else {
                    py::get_or_none(vm, &sc, "ast")
                }
            }
            _ => missing(vm),
        },
        (
            SlNode::CustomUse {
                target,
                positional,
                block: _,
                variable,
                ..
            },
            _,
        ) => match name.as_str() {
            "target" => Ok(py::s(vm, target)),
            "positional" => {
                let mut v = Vec::new();
                for e in positional {
                    v.push(expr(vm, e)?);
                }
                Ok(vm.types.new_list(v))
            }
            "block" => {
                let mut p = view.path.clone();
                p.push(Step::Body);
                make(vm, &ast, p)
            }
            "variable" => Ok(variable.as_ref().map(|x| py::s(vm, x)).unwrap_or(Value::None)),
            "ast" => Ok(Value::None),
            _ => missing(vm),
        },
        _ => missing(vm),
    }
}

// Structural queries -----------------------------------------------------------------------------------

fn n_used_screens(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let cb = args.get(1).cloned().unwrap_or(Value::None);
    match resolve(&view.ast.screen.block, &view.path) {
        Some(Target::Block(b)) => super::slast::walk_used(vm, b, &cb)?,
        Some(Target::Node(n)) => super::slast::walk_used_node(vm, n, &cb)?,
        None => {}
    }
    Ok(Value::None)
}

fn node_has_transclude(n: &SlNode) -> bool {
    match n {
        SlNode::Transclude { .. } => true,
        SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => entries
            .iter()
            .any(|(_, b)| super::slast::block_has_transclude(b)),
        // Upstream: a use with a block is transcluded by the used screen.
        SlNode::Use { .. } | SlNode::CustomUse { .. } => false,
        n => node_block(n).is_some_and(super::slast::block_has_transclude),
    }
}

fn n_has_transclude(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    Ok(Value::Bool(match resolve(&view.ast.screen.block, &view.path) {
        Some(Target::Block(b)) => super::slast::block_has_transclude(b),
        Some(Target::Node(n)) => node_has_transclude(n),
        None => false,
    }))
}

fn node_has_python(n: &SlNode) -> bool {
    match n {
        SlNode::Python { .. } | SlNode::Default { .. } => true,
        SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => {
            entries.iter().any(|(_, b)| b.children.iter().any(node_has_python))
        }
        SlNode::Use { .. } | SlNode::CustomUse { .. } => false,
        n => node_block(n).is_some_and(|b| b.children.iter().any(node_has_python)),
    }
}

fn n_has_python(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    Ok(Value::Bool(match resolve(&view.ast.screen.block, &view.path) {
        Some(Target::Block(b)) => b.children.iter().any(node_has_python),
        Some(Target::Node(n)) => node_has_python(n),
        None => false,
    }))
}

fn block_keyword_exist(b: &Block, name: &str) -> bool {
    b.keyword.iter().any(|(k, _)| k == name)
        || b.children.iter().any(|c| match c {
            SlNode::If { entries, .. } => entries.iter().any(|(_, b)| block_keyword_exist(b, name)),
            _ => false,
        })
}

fn n_keyword_exist(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let name = py::string(vm, &args[1])?;
    Ok(Value::Bool(match resolve(&view.ast.screen.block, &view.path) {
        Some(Target::Block(b)) => block_keyword_exist(b, &name),
        Some(Target::Node(SlNode::If { entries, .. })) => {
            entries.iter().any(|(_, b)| block_keyword_exist(b, &name))
        }
        Some(Target::Node(n)) => node_block(n).is_some_and(|b| block_keyword_exist(b, &name)),
        None => false,
    }))
}

fn n_has_noncondition_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    fn any(b: &Block) -> bool {
        b.children.iter().any(|c| match c {
            SlNode::If { entries, .. } => entries.iter().any(|(_, b)| any(b)),
            _ => true,
        })
    }
    Ok(Value::Bool(match resolve(&view.ast.screen.block, &view.path) {
        Some(Target::Block(b)) => any(b),
        Some(Target::Node(n)) => node_block(n).is_some_and(any),
        None => false,
    }))
}

fn n_self(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn n_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn n_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let t = resolve(&view.ast.screen.block, &view.path);
    let (name, l) = match &t {
        Some(t) => (class_name(t), loc_of(t, &view.ast)),
        None => ("SLNode", view.ast.screen.loc.clone()),
    };
    Ok(py::s(vm, &format!("<{name} at {}:{}>", l.0, l.1)))
}

fn n_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = this(vm, &args[0])?;
    let Some(b) = view_of(&args[1]) else {
        return Ok(Value::Bool(false));
    };
    let same_path = a.path.len() == b.path.len()
        && a.path.iter().zip(&b.path).all(|(x, y)| match (x, y) {
            (Step::Child(i), Step::Child(j)) | (Step::Entry(i), Step::Entry(j)) => i == j,
            (Step::Body, Step::Body) => true,
            _ => false,
        });
    Ok(Value::Bool(Rc::ptr_eq(&a.ast, &b.ast) && same_path))
}

fn n_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = this(vm, &args[0])?;
    let mut h = Rc::as_ptr(&a.ast) as usize as i64;
    for s in &a.path {
        h = h.wrapping_mul(31).wrapping_add(match s {
            Step::Child(i) => *i as i64 * 3,
            Step::Entry(i) => *i as i64 * 3 + 1,
            Step::Body => 2,
        });
    }
    Ok(Value::Int(h))
}

fn n_report_traceback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let Some(t) = resolve(&view.ast.screen.block, &view.path) else {
        return Ok(Value::None);
    };
    let l = loc_of(&t, &view.ast);
    let name = args.get(1).cloned().unwrap_or(Value::None);
    let line = vm.types.new_tuple(vec![
        py::s(vm, &l.0),
        Value::Int(l.1 as i64),
        name,
        Value::None,
    ]);
    Ok(vm.types.new_list(vec![line]))
}

fn n_dump_const(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let prefix = match args.get(1) {
        Some(p) => py::string(vm, p)?,
        None => String::new(),
    };
    let t = resolve(&view.ast.screen.block, &view.path);
    let (name, l) = match &t {
        Some(t) => (class_name(t), loc_of(t, &view.ast)),
        None => return Ok(Value::None),
    };
    let text = format!("{prefix}  (not-const) {name} {}:{}", l.0, l.1);
    if let Ok(log) = py::import_attr(vm, MODULE, "profile_log") {
        if !py::is_none(&log) {
            let _ = vm.call_method(&log, "write", vec![py::s(vm, "%s"), py::s(vm, &text)]);
        }
    }
    Ok(Value::None)
}

fn n_debug_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let Some(t) = resolve(&view.ast.screen.block, &view.path) else {
        return Ok(Value::None);
    };
    let l = loc_of(&t, &view.ast);
    if let Ok(f) = py::import_attr(vm, "renpy.display.log", "write") {
        vm.call(
            &f,
            vec![py::s(vm, &format!("  {}:{} {}", l.0, l.1, class_name(&t)))],
            Vec::new(),
        )?;
    }
    Ok(Value::None)
}

// Execution through an SLContext ----------------------------------------------------------------------

fn cattr(vm: &mut Vm, c: &Value, n: &str, default: impl FnOnce(&Vm) -> Value) -> PyResult<Value> {
    match py::get_opt(vm, c, n)? {
        Some(v) if !py::is_none(&v) => Ok(v),
        _ => {
            let d = default(vm);
            py::set(vm, c, n, d.clone())?;
            Ok(d)
        }
    }
}

fn cbool(vm: &mut Vm, c: &Value, n: &str) -> PyResult<bool> {
    match py::get_opt(vm, c, n)? {
        Some(v) => py::truthy(vm, &v),
        None => Ok(false),
    }
}

/// Runs `f` with an executor context built from a Python `SLContext`, then
/// writes the context's changed state back.
fn with_context(
    vm: &mut Vm,
    view: &NodeView,
    context: &Value,
    f: impl FnOnce(&mut Vm, &mut Ctx) -> PyResult<super::slast::Flow>,
) -> PyResult<super::slast::Flow> {
    let scope = cattr(vm, context, "scope", |vm| vm.types.new_dict())?;
    let root_scope = cattr(vm, context, "root_scope", |_| scope.clone())?;
    let children = cattr(vm, context, "children", |vm| vm.types.new_list(Vec::new()))?;
    let keywords = cattr(vm, context, "keywords", |vm| vm.types.new_dict())?;
    let style_prefix = py::get_opt(vm, context, "style_prefix")?.unwrap_or(Value::None);
    let new_cache = cattr(vm, context, "new_cache", |vm| vm.types.new_dict())?;
    let old_cache = cattr(vm, context, "old_cache", |vm| vm.types.new_dict())?;
    let miss_cache = cattr(vm, context, "miss_cache", |vm| vm.types.new_dict())?;
    let use_index = cattr(vm, context, "use_index", |vm| vm.types.new_dict())?;
    let predicted = cattr(vm, context, "predicted", |vm| {
        vm.types.new_set(Default::default())
    })?;
    let old_use_cache = cattr(vm, context, "old_use_cache", |vm| vm.types.new_dict())?;
    let new_use_cache = cattr(vm, context, "new_use_cache", |vm| vm.types.new_dict())?;
    let showif = match py::get_opt(vm, context, "showif")? {
        Some(v) if !py::is_none(&v) => Some(py::truthy(vm, &v)?),
        _ => None,
    };
    let predicting = cbool(vm, context, "predicting")?;
    let updating = cbool(vm, context, "updating")?;
    let unlikely = cbool(vm, context, "unlikely")?;
    let mut ctx = Ctx {
        ast: &view.ast,
        scope,
        root_scope,
        children,
        keywords,
        style_prefix,
        new_cache,
        old_cache,
        miss_cache,
        use_index,
        predicting,
        updating,
        predicted,
        showif,
        fail: Cell::new(cbool(vm, context, "fail")?),
        parent: None,
        transclude: None,
        unlikely,
        old_use_cache,
        new_use_cache,
        sticky: Vec::new(),
    };
    let r = f(vm, &mut ctx);
    py::set(vm, context, "style_prefix", ctx.style_prefix.clone())?;
    py::set(vm, context, "keywords", ctx.keywords.clone())?;
    py::set(vm, context, "fail", Value::Bool(ctx.fail.get()))?;
    r
}

fn flow_error(vm: &mut Vm, f: super::slast::Flow) -> PyResult<Value> {
    let cls = match f {
        super::slast::Flow::Next => return Ok(Value::None),
        super::slast::Flow::Break => "SLBreakException",
        super::slast::Flow::Continue => "SLContinueException",
    };
    let c = py::import_attr(vm, MODULE, cls).or_else(|_| py::import_attr(vm, MODULE, &format!("_{cls}")))?;
    let e = vm.call(&c, Vec::new(), Vec::new())?;
    Err(crysirror_python::PyErr::Exc(
        e.obj().cloned().ok_or_else(|| vm.type_error(cls))?,
    ))
}

/// `execute(context)`.
fn n_execute(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let context = arg_or(args, kw, 1, "context", Value::None);
    let ast = view.ast.clone();
    let flow = with_context(vm, &view, &context, |vm, ctx| {
        match resolve(&ast.screen.block, &view.path) {
            Some(Target::Block(b)) => {
                super::slast::block_keywords(vm, ctx, b)?;
                super::slast::exec_children(vm, ctx, &b.children)
            }
            Some(Target::Node(n)) => super::slast::exec_node(vm, ctx, n),
            None => Err(py::error(vm, "stale screen-language node")),
        }
    })?;
    flow_error(vm, flow)
}

/// `keywords(context)`: the properties a block (and the chosen branches of
/// its `if` children) contribute to `context.keywords`.
fn n_keywords(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let view = this(vm, &args[0])?;
    let context = arg_or(args, kw, 1, "context", Value::None);
    let ast = view.ast.clone();
    with_context(vm, &view, &context, |vm, ctx| {
        match resolve(&ast.screen.block, &view.path) {
            Some(Target::Block(b)) => super::slast::block_keywords(vm, ctx, b)?,
            Some(Target::Node(SlNode::If { entries, .. })) => {
                for (cond, b) in entries {
                    let take = match cond {
                        None => true,
                        Some(c) => {
                            let v = super::atlrt::eval(vm, c, &ctx.scope)?;
                            py::truthy(vm, &v)?
                        }
                    };
                    if take {
                        super::slast::block_keywords(vm, ctx, b)?;
                        break;
                    }
                }
            }
            _ => {}
        }
        Ok(super::slast::Flow::Next)
    })?;
    Ok(Value::None)
}

fn n_copy_on_change(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cache = args.get(1).cloned().unwrap_or(Value::None);
    super::slast::copy_on_change_cache(vm, &cache)?;
    Ok(Value::None)
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error(
        "screen-language nodes are created by the screen parser (renpy.sl2.slparser.parse_screen)",
    ))
}

fn node_type(vm: &mut Vm, name: &str, base: &ObjRef, methods: py::Methods) -> ObjRef {
    let c = vm.types.new_builtin_type(
        name,
        base,
        crysirror_python::object::BaseKind::Native,
        false,
    );
    let n = vm.types.new_builtin("__new__", no_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&c, "__new__", n);
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str(MODULE);
    vm.types.set_attr(&c, "__module__", mn);
    c
}

/// Creates the node classes; returns `SLBlock` (the base of `SLScreen`).
pub fn install_classes(vm: &mut Vm, m: &ObjRef) -> PyResult<ObjRef> {
    let rel = super::slparser::release(vm);
    let child_statements = rel >= (7, 6) && rel.0 * 100 + rel.1 != 800;
    let object = vm.types.object.clone();
    let base_methods: py::Methods = &[
        ("__getattr__", n_getattr),
        ("__repr__", n_repr),
        ("__eq__", n_eq),
        ("__hash__", n_hash),
        ("instantiate", n_self),
        ("copy", n_self),
        ("report_traceback", n_report_traceback),
        ("analyze", n_none),
        ("prepare", n_none),
        ("execute", n_execute),
        ("keywords", n_keywords),
        ("copy_on_change", n_copy_on_change),
        ("debug_line", n_debug_line),
        ("used_screens", n_used_screens),
        ("has_transclude", n_has_transclude),
        ("has_python", n_has_python),
        ("dump_const", n_dump_const),
    ];
    let node = node_type(vm, "SLNode", &object, base_methods);
    let set = |vm: &Vm, n: &str, c: &ObjRef| crysirror_python::modules::set(vm, m, n, Value::Obj(c.clone()));
    set(vm, "SLNode", &node);
    let block = node_type(
        vm,
        "SLBlock",
        &node,
        &[
            ("keyword_exist", n_keyword_exist),
            ("has_noncondition_child", n_has_noncondition_child),
        ],
    );
    set(vm, "SLBlock", &block);
    let disp = node_type(vm, "SLDisplayable", &block, &[]);
    set(vm, "SLDisplayable", &disp);
    let if_ = node_type(vm, "SLIf", &node, &[("keyword_exist", n_keyword_exist)]);
    set(vm, "SLIf", &if_);
    let showif = node_type(vm, "SLShowIf", &node, &[]);
    set(vm, "SLShowIf", &showif);
    let for_ = node_type(vm, "SLFor", &block, &[]);
    set(vm, "SLFor", &for_);
    for n in ["SLPython", "SLPass", "SLDefault", "SLUse"] {
        let c = node_type(vm, n, &node, &[]);
        set(vm, n, &c);
    }
    // Nodes of later releases exist for pickled screens of any release, but
    // are module names only where upstream defines them.
    let mut hidden = Vec::new();
    for (n, public) in [
        ("SLTransclude", rel >= (6, 99)),
        ("SLCustomUse", rel >= (7, 4)),
        ("SLBreak", child_statements),
        ("SLContinue", child_statements),
    ] {
        let c = node_type(vm, n, &node, &[]);
        if public {
            set(vm, n, &c);
        }
        hidden.push((n, c));
    }
    vm.ext_set(Hidden(hidden));
    Ok(block)
}
