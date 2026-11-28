// SPDX-License-Identifier: MPL-2.0
//! `renpy.sl2.slast`, after Ren'Py 8.3 `slast.py` (MIT): execution of
//! screen-language-2 screens over the Rust screen AST (`crate::sl2`).
//!
//! Every screen is evaluated as upstream's non-constant AST. Constant
//! analysis (`pyanalysis`) only lets upstream skip re-evaluating
//! expressions whose values cannot change; the per-node caches below keep
//! the observable reuse semantics: a displayable whose positional
//! arguments, keywords and style prefix compare equal to the previous
//! update is reused with its state, `replaces` hands the old widget to the
//! new one, and transforms take the state of their predecessors.
//!
//! Node serials are the addresses of the nodes inside the screen's shared
//! AST, which lives as long as the `SLScreen` value. Each screen value has
//! its own cache version, so a redefined screen never reads stale caches.
//! The developer-only `profile_screens` log is not produced.
use super::{
    params,
    py::{self, *},
};
use crate::sl2::{Block, Displayable, NChildren, Screen, SlNode, StatementSpec};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

const MODULE: &str = "renpy.sl2.slast";
const STICKY_PROPERTIES: [&str; 2] = ["group_alt", "extra_alt"];

thread_local! {
    static VERSION: Cell<i64> = const { Cell::new(0) };
}

// Screen AST values ------------------------------------------------------------------------------------

/// Resolved statement of one displayable node.
pub(crate) struct NodeRt {
    pub(crate) spec: StatementSpec,
    pub(crate) factory: Value,
}

pub struct ScreenAst {
    pub screen: Rc<Screen>,
    version: i64,
    nodes: RefCell<HashMap<usize, Rc<NodeRt>>>,
    /// Prepared `ATLTransform` of each block with an ATL transform
    /// (`SLBlock.transform`, the `parent_transform` of per-update copies).
    atl: RefCell<HashMap<usize, Value>>,
}

#[derive(Clone)]
struct AstRef(Rc<ScreenAst>);

fn serial<T>(n: &T) -> usize {
    n as *const T as usize
}

pub fn ast_of(v: &Value) -> Option<Rc<ScreenAst>> {
    py::native::<AstRef, _>(v, |a| a.0.clone())
}

fn native_class(vm: &mut Vm, name: &str) -> PyResult<ObjRef> {
    let c = py::import_attr(vm, MODULE, name)?;
    c.obj()
        .cloned()
        .ok_or_else(|| vm.type_error(name.to_owned()))
}

/// The `SLScreen` value of a parsed or unpickled screen.
pub fn screen_value(vm: &mut Vm, screen: Rc<Screen>) -> PyResult<Value> {
    let version = VERSION.with(|v| {
        v.set(v.get() + 1);
        v.get()
    });
    let ast = Rc::new(ScreenAst {
        screen: screen.clone(),
        version,
        nodes: Default::default(),
        atl: Default::default(),
    });
    let c = native_class(vm, "SLScreen")?;
    let rv = vm.types.new_native(&c, Box::new(AstRef(ast)));
    let name = py::s(vm, &screen.name);
    py::set(vm, &rv, "name", name)?;
    for (n, v) in [
        ("modal", &screen.modal),
        ("zorder", &screen.zorder),
        ("variant", &screen.variant),
        ("predict", &screen.predict),
        ("layer", &screen.layer),
        ("sensitive", &screen.sensitive),
        ("roll_forward", &screen.roll_forward),
    ] {
        let v = py::s(vm, v);
        py::set(vm, &rv, n, v)?;
    }
    let tag = screen
        .tag
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    py::set(vm, &rv, "tag", tag)?;
    let doc = screen
        .docstring
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    py::set(vm, &rv, "docstring", doc)?;
    let params = match &screen.parameters {
        Some(p) => super::atlrt::signature_value(vm, p.clone())?,
        None => Value::None,
    };
    py::set(vm, &rv, "parameters", params)?;
    let loc = location(vm, &screen.loc);
    py::set(vm, &rv, "location", loc)?;
    py::set(vm, &rv, "const_ast", rv.clone())?;
    py::set(vm, &rv, "not_const_ast", rv.clone())?;
    py::set(vm, &rv, "version", Value::Int(version))?;
    py::set(vm, &rv, "prepared", Value::Bool(true))?;
    Ok(rv)
}

fn location(vm: &Vm, loc: &(Rc<str>, u32)) -> Value {
    let f = vm.types.new_str(&loc.0);
    vm.types.new_tuple(vec![f, Value::Int(loc.1 as i64)])
}

impl ScreenAst {
    fn node(&self, vm: &mut Vm, d: &Displayable) -> PyResult<Rc<NodeRt>> {
        let key = serial(d);
        if let Some(n) = self.nodes.borrow().get(&key) {
            return Ok(n.clone());
        }
        let spec = super::state::statement(vm, &d.statement);
        let spec = match spec {
            Some(s) => s,
            // A pickled creator-defined displayable whose statement is not
            // registered: the dotted name of its callable.
            None if d.statement.contains('.') => crate::sl2::builtin_statements()
                .into_iter()
                .find(|s| s.displayable == d.statement)
                .unwrap_or_else(|| custom_spec(&d.statement)),
            None => {
                return Err(py::error(
                    vm,
                    format!(
                        "{}:{}: unknown screen language statement {}",
                        d.loc.0, d.loc.1, d.statement
                    ),
                ));
            }
        };
        let factory = resolve(vm, &spec.displayable)?;
        let rt = Rc::new(NodeRt { spec, factory });
        self.nodes.borrow_mut().insert(key, rt.clone());
        Ok(rt)
    }

    pub(crate) fn node_rt(&self, vm: &mut Vm, d: &Displayable) -> PyResult<Rc<NodeRt>> {
        self.node(vm, d)
    }

    pub(crate) fn atl_parent_value(&self, vm: &mut Vm, b: &Block) -> PyResult<Value> {
        self.atl_parent(vm, b)
    }

    fn atl_parent(&self, vm: &mut Vm, b: &Block) -> PyResult<Value> {
        let key = serial(b);
        if let Some(t) = self.atl.borrow().get(&key) {
            return Ok(t.clone());
        }
        let raw = b.atl_transform.clone().expect("block with ATL");
        let t = super::atlrt::new_atl_transform(vm, raw, None, Value::None, Value::None)?;
        self.atl.borrow_mut().insert(key, t.clone());
        Ok(t)
    }
}

fn custom_spec(dotted: &str) -> StatementSpec {
    StatementSpec {
        name: dotted.into(),
        displayable: dotted.into(),
        style: None,
        nchildren: NChildren::Zero,
        positional: Vec::new(),
        scope: false,
        pass_context: false,
        imagemap: false,
        hotspot: false,
        replaces: false,
        unique: false,
        default_keywords: Vec::new(),
        custom_screen: None,
    }
}

/// Resolves the dotted name of a displayable callable; store names (from
/// `renpy.register_sl_displayable`) are looked up in the store.
fn resolve(vm: &mut Vm, dotted: &str) -> PyResult<Value> {
    if let Some((m, n)) = dotted.rsplit_once('.') {
        if let Ok(v) = py::import_attr(vm, m, n) {
            return Ok(v);
        }
        if let Some(rest) = dotted.strip_prefix("store.") {
            return store_lookup(vm, rest);
        }
    }
    store_lookup(vm, dotted)
}

fn store_lookup(vm: &mut Vm, path: &str) -> PyResult<Value> {
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or("");
    let store = py::import(vm, "store")?;
    let mut v = py::get(vm, &store, first)?;
    for p in parts {
        v = py::get(vm, &v, p)?;
    }
    Ok(v)
}

// Python helpers ---------------------------------------------------------------------------------------

fn dget(vm: &mut Vm, d: &Value, k: &Value) -> PyResult<Option<Value>> {
    match d.obj() {
        Some(o) if matches!(o.payload, crysirror_python::Payload::Dict(_)) => {
            ops::dict_get(vm, o, k)
        }
        _ => Ok(None),
    }
}

/// `d.get(k, None) or {}` restricted to dicts.
fn sub_dict(vm: &mut Vm, d: &Value, k: &Value) -> PyResult<Value> {
    match dget(vm, d, k)? {
        Some(v) if is_dict(&v) && ops::len(vm, &v)? > 0 => Ok(v),
        _ => Ok(vm.types.new_dict()),
    }
}

fn is_dict(v: &Value) -> bool {
    matches!(v.payload(), Some(crysirror_python::Payload::Dict(_)))
}

fn dpop(vm: &mut Vm, d: &Value, k: &str) -> PyResult<Option<Value>> {
    let key = py::s(vm, k);
    match d.obj() {
        Some(o) => ops::dict_pop(vm, o, &key),
        None => Ok(None),
    }
}

fn dcontains(vm: &mut Vm, d: &Value, k: &str) -> PyResult<bool> {
    let key = py::s(vm, k);
    ops::contains(vm, d, &key)
}

fn isinstance_of(vm: &mut Vm, v: &Value, module: &str, cls: &str) -> PyResult<bool> {
    let c = py::import_attr(vm, module, cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

fn is_transform(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    isinstance_of(vm, v, "renpy.display.transform", "Transform")
}

fn predicting(vm: &mut Vm) -> bool {
    py::import_attr(vm, "renpy.display.predict", "predicting")
        .ok()
        .is_some_and(|v| matches!(v, Value::Bool(true)))
}

fn predict_displayable(vm: &mut Vm, d: &Value) -> PyResult<()> {
    if py::is_none(d) {
        return Ok(());
    }
    let f = py::import_attr(vm, "renpy.display.predict", "displayable")?;
    vm.call(&f, vec![d.clone()], Vec::new())?;
    Ok(())
}

fn main_of(vm: &mut Vm, d: &Value) -> PyResult<Value> {
    let m = py::get_or_none(vm, d, "_main")?;
    Ok(if py::truthy(vm, &m)? { m } else { d.clone() })
}

// Caches -----------------------------------------------------------------------------------------------

/// `SLCache`: what one displayable node produced at the last update.
#[derive(Default)]
struct SlCache {
    displayable: Option<Value>,
    positional: Option<Value>,
    keywords: Option<Value>,
    children: Option<Value>,
    outer_transform: Option<Value>,
    inner_transform: Option<Value>,
    raw_transform: Option<Value>,
    imagemap: Option<Value>,
    copy_on_change: bool,
    old_showif: Option<Value>,
    style_prefix: Option<Value>,
}

struct CacheRef(Rc<RefCell<SlCache>>);

fn cache_of(v: &Value) -> Option<Rc<RefCell<SlCache>>> {
    py::native::<CacheRef, _>(v, |c| c.0.clone())
}

fn new_cache(vm: &mut Vm) -> PyResult<(Value, Rc<RefCell<SlCache>>)> {
    let c = native_class(vm, "SLCache")?;
    let rc = Rc::new(RefCell::new(SlCache::default()));
    Ok((vm.types.new_native(&c, Box::new(CacheRef(rc.clone()))), rc))
}

fn opt(v: &Option<Value>) -> Value {
    v.clone().unwrap_or(Value::None)
}

/// `copy_on_change`: marks every displayable cache of an update tree, so
/// a reused widget that was copied for a hide or transition is rebuilt.
fn mark_copy_on_change(vm: &mut Vm, cache: &Value, seen: &mut HashSet<i64>) -> PyResult<()> {
    if !seen.insert(cache.id()) {
        return Ok(());
    }
    if let Some(c) = cache_of(cache) {
        c.borrow_mut().copy_on_change = true;
        return Ok(());
    }
    if !is_dict(cache) {
        return Ok(());
    }
    for (k, v) in ops::mapping_items(vm, cache)? {
        if matches!(k.as_str(), Some("scope" | "ast" | "version" | "transclude")) {
            continue;
        }
        if is_dict(&v) || cache_of(&v).is_some() {
            mark_copy_on_change(vm, &v, seen)?;
        }
    }
    Ok(())
}

pub(crate) fn copy_on_change_cache(vm: &mut Vm, cache: &Value) -> PyResult<()> {
    mark_copy_on_change(vm, cache, &mut HashSet::new())
}

/// A constant-analysis level (`NOT_CONST`, `LOCAL_CONST`, `GLOBAL_CONST`).
pub(crate) fn const_value(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, name)
}

// Contexts ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Ctx<'a> {
    pub(crate) ast: &'a ScreenAst,
    pub(crate) scope: Value,
    pub(crate) root_scope: Value,
    pub(crate) children: Value,
    pub(crate) keywords: Value,
    pub(crate) style_prefix: Value,
    pub(crate) new_cache: Value,
    pub(crate) old_cache: Value,
    pub(crate) miss_cache: Value,
    pub(crate) use_index: Value,
    pub(crate) predicting: bool,
    pub(crate) updating: bool,
    pub(crate) predicted: Value,
    pub(crate) showif: Option<bool>,
    pub(crate) fail: Cell<bool>,
    pub(crate) parent: Option<&'a Ctx<'a>>,
    pub(crate) transclude: Option<&'a Block>,
    pub(crate) unlikely: bool,
    pub(crate) old_use_cache: Value,
    pub(crate) new_use_cache: Value,
    pub(crate) sticky: Vec<(Rc<str>, Value)>,
}

impl<'a> Ctx<'a> {
    /// `SLContext(parent)`: shares every container of the parent.
    fn derive(&self) -> Ctx<'a> {
        let c = self.clone();
        c.fail.set(false);
        c
    }

    /// The Python `SLContext` seen by `renpy.ui` and `pass_context`
    /// factories while this context is on the widget stack.
    fn facade(&self, vm: &mut Vm) -> PyResult<Value> {
        let c = py::import_attr(vm, MODULE, "SLContext")?;
        let o = vm.call(&c, Vec::new(), Vec::new())?;
        py::set(vm, &o, "children", self.children.clone())?;
        py::set(vm, &o, "scope", self.scope.clone())?;
        py::set(vm, &o, "root_scope", self.root_scope.clone())?;
        py::set(vm, &o, "style_prefix", self.style_prefix.clone())?;
        py::set(vm, &o, "keywords", self.keywords.clone())?;
        py::set(vm, &o, "predicting", Value::Bool(self.predicting))?;
        py::set(vm, &o, "updating", Value::Bool(self.updating))?;
        py::set(
            vm,
            &o,
            "showif",
            self.showif.map(Value::Bool).unwrap_or(Value::None),
        )?;
        Ok(o)
    }

    fn push(&self, vm: &mut Vm) -> PyResult<()> {
        let f = self.facade(vm)?;
        let stack = super::ui::stack(vm)?;
        vm.call_method(&stack, "append", vec![f])?;
        Ok(())
    }
}

fn pop_stack(vm: &mut Vm) -> PyResult<()> {
    let stack = super::ui::stack(vm)?;
    vm.call_method(&stack, "pop", Vec::new())?;
    Ok(())
}

pub(crate) enum Flow {
    Next,
    Break,
    Continue,
}

pub(crate) type SR = PyResult<Flow>;

fn eval(vm: &mut Vm, e: &crate::lexer::PyExpr, scope: &Value) -> PyResult<Value> {
    super::atlrt::eval(vm, e, scope)
}

fn is_control(e: &PyErr) -> bool {
    matches!(e, PyErr::Suspend(..))
}

// Blocks -----------------------------------------------------------------------------------------------

pub(crate) fn block_has_keyword(b: &Block) -> bool {
    !b.keyword.is_empty() || b.atl_transform.is_some() || b.children.iter().any(node_has_keyword)
}

fn node_has_keyword(n: &SlNode) -> bool {
    match n {
        SlNode::If { entries, .. } => entries.iter().any(|(_, b)| block_has_keyword(b)),
        _ => false,
    }
}

/// `SLBlock.keywords`: evaluates the properties of a block (and of the
/// chosen branches of nested `if` statements) into `ctx.keywords`.
pub(crate) fn block_keywords(vm: &mut Vm, ctx: &mut Ctx, b: &Block) -> PyResult<()> {
    for (k, e) in &b.keyword {
        let v = eval(vm, e, &ctx.scope)?;
        py::dict_set(vm, &ctx.keywords, k, v)?;
    }
    for c in &b.children {
        if let SlNode::If { entries, .. } = c {
            if !entries.iter().any(|(_, b)| block_has_keyword(b)) {
                continue;
            }
            for (cond, block) in entries {
                let take = match cond {
                    None => true,
                    Some(c) => {
                        let v = eval(vm, c, &ctx.scope)?;
                        py::truthy(vm, &v)?
                    }
                };
                if take {
                    block_keywords(vm, ctx, block)?;
                    break;
                }
            }
        }
    }
    if let Some(raw) = &b.atl_transform {
        let t =
            super::atlrt::new_atl_transform(vm, raw.clone(), None, ctx.scope.clone(), Value::None)?;
        let parent = ctx.ast.atl_parent(vm, b)?;
        py::set(vm, &t, "parent_transform", parent)?;
        match dget(vm, &ctx.keywords, &py::s(vm, "at"))? {
            Some(at) => {
                let mut list = match py::seq(&at) {
                    Some(l) => l,
                    None => match ops::collect(vm, &at) {
                        Ok(l) => l,
                        Err(_) => vec![at],
                    },
                };
                list.push(t);
                let l = vm.types.new_list(list);
                py::dict_set(vm, &ctx.keywords, "at", l)?;
            }
            None => py::dict_set(vm, &ctx.keywords, "at", t)?,
        }
    }
    let sp = match dpop(vm, &ctx.keywords, "style_prefix")? {
        Some(v) => Some(v),
        None => dpop(vm, &ctx.keywords, "style_group")?,
    };
    if let Some(sp) = sp {
        ctx.style_prefix = sp;
    }
    Ok(())
}

/// `SLBlock.execute`: while predicting, a failing child is skipped.
pub(crate) fn exec_children(vm: &mut Vm, ctx: &mut Ctx, children: &[SlNode]) -> SR {
    for c in children {
        match exec_node(vm, ctx, c) {
            Ok(Flow::Next) => {}
            Ok(f) => return Ok(f),
            Err(e) if ctx.predicting && !is_control(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Flow::Next)
}

pub(crate) fn exec_node(vm: &mut Vm, ctx: &mut Ctx, n: &SlNode) -> SR {
    match n {
        SlNode::Displayable(d) => exec_displayable(vm, ctx, d),
        SlNode::If { entries, .. } => exec_if(vm, ctx, n, entries),
        SlNode::ShowIf { entries, .. } => exec_showif(vm, ctx, entries),
        SlNode::For {
            variable,
            expression,
            index_expression,
            block,
            ..
        } => exec_for(
            vm,
            ctx,
            n,
            variable,
            expression,
            index_expression.as_ref(),
            block,
        ),
        SlNode::Python { code, .. } => {
            super::atlrt::exec(vm, code, &ctx.scope)?;
            Ok(Flow::Next)
        }
        SlNode::Pass { .. } => Ok(Flow::Next),
        SlNode::Default {
            variable,
            expression,
            ..
        } => {
            if !dcontains(vm, &ctx.scope, variable)? {
                let v = eval(vm, expression, &ctx.scope)?;
                py::dict_set(vm, &ctx.scope, variable, v)?;
            }
            Ok(Flow::Next)
        }
        SlNode::Use {
            loc,
            target,
            expression,
            args,
            id,
            block,
            ..
        } => {
            exec_use(
                vm,
                ctx,
                n,
                loc,
                target,
                *expression,
                args.as_ref(),
                id.as_ref(),
                block.as_ref(),
            )?;
            Ok(Flow::Next)
        }
        SlNode::Transclude { .. } => {
            exec_transclude(vm, ctx, n)?;
            Ok(Flow::Next)
        }
        SlNode::CustomUse {
            target,
            positional,
            block,
            ..
        } => {
            exec_custom_use(vm, ctx, n, target, positional, block)?;
            Ok(Flow::Next)
        }
        SlNode::Break { .. } => Ok(Flow::Break),
        SlNode::Continue { .. } => Ok(Flow::Continue),
    }
}

// if / showif / for --------------------------------------------------------------------------------

fn cond_true(vm: &mut Vm, ctx: &Ctx, cond: &Option<crate::lexer::PyExpr>) -> PyResult<bool> {
    match cond {
        None => Ok(true),
        Some(c) => {
            let v = eval(vm, c, &ctx.scope)?;
            py::truthy(vm, &v)
        }
    }
}

fn exec_if(
    vm: &mut Vm,
    ctx: &mut Ctx,
    n: &SlNode,
    entries: &[(Option<crate::lexer::PyExpr>, Block)],
) -> SR {
    if !ctx.predicting {
        for (cond, block) in entries {
            if cond_true(vm, ctx, cond)? {
                // `i.execute(context)` for each child: errors propagate.
                for c in &block.children {
                    match exec_node(vm, ctx, c)? {
                        Flow::Next => {}
                        f => return Ok(f),
                    }
                }
                return Ok(Flow::Next);
            }
        }
        return Ok(Flow::Next);
    }
    // `execute_predicting`: the likely branch runs normally, the others are
    // predicted once per screen with an unlikely context.
    let key = Value::Int(serial(n) as i64);
    let predict_false = !ops::set_contains(vm, &ctx.predicted, &key)?;
    ops::set_add(vm, &ctx.predicted, key)?;
    let mut first = true;
    for (cond, block) in entries {
        let value = cond_true(vm, ctx, cond).unwrap_or(false);
        if first && value {
            first = false;
            for c in &block.children {
                if let Ok(Flow::Break | Flow::Continue) = exec_node(vm, ctx, c) {
                    break;
                }
            }
        } else if predict_false {
            let mut cx = ctx.derive();
            cx.children = vm.types.new_list(Vec::new());
            cx.unlikely = true;
            for c in &block.children {
                let _ = exec_node(vm, &mut cx, c);
            }
            for d in py::items(vm, &cx.children)? {
                let _ = predict_displayable(vm, &d);
            }
        }
    }
    Ok(Flow::Next)
}

fn exec_showif(
    vm: &mut Vm,
    ctx: &mut Ctx,
    entries: &[(Option<crate::lexer::PyExpr>, Block)],
) -> SR {
    let mut first_true = ctx.showif != Some(false);
    for (cond, block) in entries {
        let mut cx = ctx.derive();
        if !first_true {
            cx.showif = Some(false);
        } else if cond_true(vm, ctx, cond)? {
            cx.showif = Some(true);
            first_true = false;
        } else {
            cx.showif = Some(false);
        }
        for c in &block.children {
            match exec_node(vm, &mut cx, c)? {
                Flow::Next => {}
                f => return Ok(f),
            }
        }
        if cx.fail.get() {
            ctx.fail.set(true);
        }
    }
    Ok(Flow::Next)
}

fn exec_for(
    vm: &mut Vm,
    ctx: &mut Ctx,
    n: &SlNode,
    variable: &str,
    expression: &crate::lexer::PyExpr,
    index: Option<&crate::lexer::PyExpr>,
    block: &Block,
) -> SR {
    let value = match eval(vm, expression, &ctx.scope) {
        Ok(v) => ops::collect(vm, &v),
        Err(e) => Err(e),
    };
    let values = match value {
        Ok(v) => v,
        Err(e) if !ctx.predicting => return Err(e),
        Err(_) => vec![Value::Int(0)],
    };
    let key = Value::Int(serial(n) as i64);
    let newcaches = vm.types.new_dict();
    let oldcaches = sub_dict(vm, &ctx.old_cache, &key)?;
    let misscaches = sub_dict(vm, &ctx.miss_cache, &key)?;
    let mut cx = ctx.derive();
    for (i, v) in values.into_iter().enumerate() {
        py::dict_set(vm, &cx.scope, variable, v)?;
        let mut children = block.children.iter();
        if variable == "_sl2_i" {
            if let Some(first) = children.next() {
                match exec_node(vm, &mut cx, first) {
                    Err(e) if !ctx.predicting => return Err(e),
                    _ => {}
                }
            }
        }
        let index = match index {
            Some(e) => eval(vm, e, &cx.scope)?,
            None => Value::Int(i as i64),
        };
        cx.old_cache = sub_dict(vm, &oldcaches, &index)?;
        cx.miss_cache = sub_dict(vm, &misscaches, &index)?;
        cx.new_cache = vm.types.new_dict();
        ops::setitem(vm, &newcaches, index, cx.new_cache.clone())?;
        let mut flow = Flow::Next;
        for c in children {
            match exec_node(vm, &mut cx, c) {
                Ok(Flow::Next) => {}
                Ok(f) => {
                    flow = f;
                    break;
                }
                Err(e) if ctx.predicting && !is_control(&e) => {}
                Err(e) => return Err(e),
            }
        }
        if matches!(flow, Flow::Break) {
            break;
        }
        if ctx.unlikely {
            break;
        }
    }
    ops::setitem(vm, &ctx.new_cache, key, newcaches)?;
    if cx.fail.get() {
        ctx.fail.set(true);
    }
    Ok(Flow::Next)
}

// Displayables -----------------------------------------------------------------------------------------

struct Made {
    d: Value,
    main: Value,
    old_d: Option<Value>,
    old_main: Value,
    reused: bool,
    imagemap: bool,
    widget_id: Value,
    transform: Value,
    positional: Vec<Value>,
    keywords: Value,
}

fn exec_displayable(vm: &mut Vm, context: &mut Ctx, node: &Displayable) -> SR {
    let rt = context.ast.node(vm, node)?;
    let spec = &rt.spec;
    let key = Value::Int(serial(node) as i64);
    let cache_v = match dget(vm, &context.old_cache, &key)? {
        Some(c) if cache_of(&c).is_some() => c,
        _ => match dget(vm, &context.miss_cache, &key)? {
            Some(c) if cache_of(&c).is_some() => c,
            _ => new_cache(vm)?.0,
        },
    };
    let cache = cache_of(&cache_v).expect("SLCache");
    ops::setitem(vm, &context.new_cache, key, cache_v.clone())?;
    let copy_on_change = cache.borrow().copy_on_change;
    let screen = super::ui::mget(vm, "screen")?;

    let mut ctx = context.derive();
    let made = make_displayable(
        vm,
        context,
        &mut ctx,
        node,
        &rt,
        &cache,
        copy_on_change,
        &screen,
    );
    let (mut made, mut fail) = match made {
        Ok(m) => (Some(m), false),
        Err(e) if context.predicting && !is_control(&e) => (None, true),
        Err(e) => return Err(e),
    };
    let main = made.as_ref().map(|m| m.main.clone()).unwrap_or(Value::None);
    if let Some(var) = &node.block.variable {
        py::dict_set(vm, &context.scope, var, main.clone())?;
    }

    ctx.children = vm.types.new_list(Vec::new());
    ctx.showif = None;
    ctx.push(vm)?;
    let mut flow = Flow::Next;
    let mut err = None;
    for c in &node.block.children {
        match exec_node(vm, &mut ctx, c) {
            Ok(Flow::Next) => {}
            Ok(f) => {
                flow = f;
                break;
            }
            Err(e) if context.predicting && !is_control(&e) => fail = true,
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    ctx.keywords = Value::None;
    pop_stack(vm)?;
    if made.as_ref().is_some_and(|m| m.imagemap) {
        let ims = super::ui::mget(vm, "imagemap_stack")?;
        let im = vm.call_method(&ims, "pop", Vec::new())?;
        cache.borrow_mut().imagemap = Some(im.clone());
        let c = py::get(vm, &im, "cache")?;
        vm.call_method(&c, "finish", Vec::new())?;
    }
    if let Some(e) = err {
        return Err(e);
    }
    if !matches!(flow, Flow::Next) {
        return Ok(flow);
    }
    let Some(m) = made.as_mut().filter(|_| !fail) else {
        predict_displayable(vm, &main)?;
        for d in py::items(vm, &ctx.children)? {
            predict_displayable(vm, &d)?;
        }
        context.fail.set(true);
        return Ok(Flow::Next);
    };

    let old_children = opt(&cache.borrow().children);
    if !ops::eq(vm, &ctx.children, &old_children)? {
        if m.reused && copy_on_change {
            create(vm, context, &ctx, node, &rt, m, &screen, false)?;
            cache.borrow_mut().copy_on_change = false;
            m.reused = false;
        }
        if m.reused {
            vm.call_method(&m.main, "_clear", Vec::new())?;
        }
        let children = py::items(vm, &ctx.children)?;
        if spec.nchildren == NChildren::One && children.len() != 1 {
            let fixed = py::import_attr(vm, "renpy.display.layout", "Fixed")?;
            let f = vm.call(&fixed, Vec::new(), Vec::new())?;
            for c in children {
                vm.call_method(&f, "add", vec![c])?;
            }
            vm.call_method(&m.main, "add", vec![f])?;
        } else {
            for c in children {
                vm.call_method(&m.main, "add", vec![c])?;
            }
        }
    }
    py::set(vm, &m.main, "id", m.widget_id.clone())?;

    let mut d = m.d.clone();
    if !context.predicting {
        if let Some(old_d) = &m.old_d {
            let rb = py::import_attr(vm, "renpy.display.focus", "replaced_by")?;
            ops::setitem(vm, &rb, Value::Int(old_d.id()), d.clone())?;
            if !py::same(&d, &m.main) {
                let op = py::get_items(vm, old_d, "_composite_parts")?;
                let np = py::get_items(vm, &d, "_composite_parts")?;
                for (o, n) in op.into_iter().zip(np) {
                    ops::setitem(vm, &rb, Value::Int(o.id()), n)?;
                }
            }
        }
    }
    {
        let mut c = cache.borrow_mut();
        c.displayable = Some(d.clone());
        c.children = Some(ctx.children.clone());
        c.style_prefix = Some(context.style_prefix.clone());
    }

    // NO_DISPLAYABLE is a 6.99+ name; 6.18 always applies the transform.
    let m_ = py::import(vm, MODULE)?;
    let no_d = py::get_opt(vm, &m_, "NO_DISPLAYABLE")?;
    let transform = if py::truthy(vm, &m.transform)? {
        Some(m.transform.clone())
    } else {
        None
    };
    match transform {
        Some(t) if !no_d.as_ref().is_some_and(|n| py::same(&d, n)) => {
            d = apply_at(vm, context, &cache, m.reused, &t, d)?
        }
        _ => {
            let mut c = cache.borrow_mut();
            c.inner_transform = None;
            c.outer_transform = None;
            c.raw_transform = None;
        }
    }
    if ctx.fail.get() {
        context.fail.set(true);
    }
    if !no_d.as_ref().is_some_and(|n| py::same(&d, n)) {
        if let Some(s) = context.showif {
            d = wrap_in_showif(vm, context, &cache, s, d)?;
        }
        ops::list_append(vm, &context.children, d)?;
    }
    Ok(Flow::Next)
}

fn make_displayable(
    vm: &mut Vm,
    context: &Ctx,
    ctx: &mut Ctx,
    node: &Displayable,
    rt: &NodeRt,
    cache: &Rc<RefCell<SlCache>>,
    copy_on_change: bool,
    screen: &Value,
) -> PyResult<Made> {
    let spec = &rt.spec;
    let mut positional = Vec::with_capacity(node.positional.len());
    for e in &node.positional {
        positional.push(eval(vm, e, &context.scope)?);
    }
    let keywords = vm.types.new_dict();
    for (k, e) in &spec.default_keywords {
        let v = params::eval_text(vm, e, &node.loc, &Value::None)?;
        py::dict_set(vm, &keywords, k, v)?;
    }
    ctx.keywords = keywords.clone();
    block_keywords(vm, ctx, &node.block)?;
    if let Some(a) = dpop(vm, &keywords, "arguments")? {
        if py::truthy(vm, &a)? {
            positional.extend(py::items(vm, &a)?);
        }
    }
    if let Some(p) = dpop(vm, &keywords, "properties")? {
        if py::truthy(vm, &p)? {
            vm.call_method(&keywords, "update", vec![p])?;
        }
    }
    let widget_id = dpop(vm, &keywords, "id")?.unwrap_or(Value::None);
    let transform = dpop(vm, &keywords, "at")?.unwrap_or(Value::None);
    let prefer_screen = dpop(vm, &keywords, "prefer_screen_to_id")?.unwrap_or(Value::Bool(false));
    let mut keywords = keywords;
    if py::truthy(vm, &widget_id)? && !py::is_none(screen) {
        let wp = py::get(vm, screen, "widget_properties")?;
        if ops::contains(vm, &wp, &widget_id)? {
            let props = ops::getitem(vm, &wp, widget_id.clone())?;
            if py::truthy(vm, &prefer_screen)? {
                let nk = vm.call_method(&props, "copy", Vec::new())?;
                vm.call_method(&nk, "update", vec![keywords.clone()])?;
                keywords = nk;
            } else {
                vm.call_method(&keywords, "update", vec![props])?;
            }
        }
    }
    ctx.keywords = keywords.clone();
    let suffix = match dpop(vm, &keywords, "style_suffix")? {
        Some(v) if py::truthy(vm, &v)? => Some(v),
        _ => spec.style.as_ref().map(|s| py::s(vm, s)),
    };
    if let Some(suffix) = suffix {
        if !dcontains(vm, &keywords, "style")? {
            let style = if py::is_none(&ctx.style_prefix) {
                suffix
            } else {
                let p = py::string(vm, &ctx.style_prefix)?;
                let s = py::string(vm, &suffix)?;
                py::s(vm, &format!("{p}_{s}"))
            };
            py::dict_set(vm, &keywords, "style", style)?;
        }
    }
    for k in STICKY_PROPERTIES {
        if let Some(v) = dget(vm, &keywords, &py::s(vm, k))? {
            ctx.sticky.retain(|(n, _)| &**n != k);
            ctx.sticky.push((k.into(), v));
        }
    }
    for (k, v) in ctx.sticky.clone() {
        py::dict_set(vm, &keywords, &k, v)?;
    }

    let old_d = cache.borrow().displayable.clone();
    let old_main = match &old_d {
        Some(d) => main_of(vm, d)?,
        None => Value::None,
    };
    let pos_list = vm.types.new_list(positional.clone());
    let mut can_reuse = match &old_d {
        Some(_) => {
            let cp = opt(&cache.borrow().positional);
            let ck = opt(&cache.borrow().keywords);
            let cs = opt(&cache.borrow().style_prefix);
            ops::eq(vm, &pos_list, &cp)?
                && ops::eq(vm, &keywords, &ck)?
                && ops::eq(vm, &context.style_prefix, &cs)?
        }
        None => false,
    };
    if node.block.variable.is_some() && copy_on_change {
        can_reuse = false;
    }
    if spec.hotspot {
        let ims = super::ui::mget(vm, "imagemap_stack")?;
        let imc = ops::getitem(vm, &ims, Value::Int(-1))?;
        if !cache
            .borrow()
            .imagemap
            .as_ref()
            .is_some_and(|i| py::same(i, &imc))
        {
            can_reuse = false;
        }
        cache.borrow_mut().imagemap = Some(imc);
    }
    let mut m = Made {
        d: Value::None,
        main: Value::None,
        old_d: old_d.clone(),
        old_main: old_main.clone(),
        reused: false,
        imagemap: false,
        widget_id,
        transform,
        positional,
        keywords,
    };
    if can_reuse {
        m.reused = true;
        m.d = old_d.clone().unwrap();
        m.main = old_main;
        register_widget(vm, ctx, screen, &m)?;
        if spec.scope {
            let uses = py::get_or_none(vm, &m.main, "_uses_scope")?;
            if py::truthy(vm, &uses)? {
                let changed = vm.call_method(
                    &m.main,
                    "_scope",
                    vec![ctx.scope.clone(), Value::Bool(!copy_on_change)],
                )?;
                if copy_on_change && py::truthy(vm, &changed)? {
                    m.reused = false;
                }
            }
        }
    }
    if m.reused && spec.imagemap {
        m.imagemap = true;
        let im = opt(&cache.borrow().imagemap);
        vm.call_method(&im, "reuse", Vec::new())?;
        let ims = super::ui::mget(vm, "imagemap_stack")?;
        vm.call_method(&ims, "append", vec![im])?;
    }
    if !m.reused {
        {
            let k = vm.call_method(&m.keywords, "copy", Vec::new())?;
            let mut c = cache.borrow_mut();
            c.positional = Some(pos_list);
            c.keywords = Some(k);
        }
        create(vm, context, ctx, node, rt, &mut m, screen, true)?;
        m.imagemap = spec.imagemap;
        let mut c = cache.borrow_mut();
        c.copy_on_change = false;
        c.children = None;
    }
    Ok(m)
}

fn register_widget(vm: &mut Vm, ctx: &Ctx, screen: &Value, m: &Made) -> PyResult<()> {
    if !py::truthy(vm, &m.widget_id)? || ctx.unlikely || py::is_none(screen) {
        return Ok(());
    }
    let w = py::get(vm, screen, "widgets")?;
    ops::setitem(vm, &w, m.widget_id.clone(), m.main.clone())?;
    let b = py::get(vm, screen, "base_widgets")?;
    ops::setitem(vm, &b, m.widget_id.clone(), m.d.clone())?;
    Ok(())
}

/// Calls the statement's factory with the evaluated arguments.
fn create(
    vm: &mut Vm,
    context: &Ctx,
    ctx: &Ctx,
    node: &Displayable,
    rt: &NodeRt,
    m: &mut Made,
    screen: &Value,
    first: bool,
) -> PyResult<()> {
    let spec = &rt.spec;
    if spec.scope {
        py::dict_set(vm, &m.keywords, "scope", ctx.scope.clone())?;
    }
    let updating = if first {
        ctx.updating
    } else {
        context.updating
    };
    if spec.replaces && updating {
        py::dict_set(vm, &m.keywords, "replaces", m.old_main.clone())?;
    }
    if spec.pass_context {
        let f = ctx.facade(vm)?;
        py::dict_set(vm, &m.keywords, "context", f)?;
    }
    let kw = py::str_items(vm, &m.keywords)?;
    let d = vm.call(&rt.factory, m.positional.clone(), kw)?;
    vm.call_method(&d, "_unique", Vec::new())?;
    let main = main_of(vm, &d)?;
    let loc = location(vm, &node.loc);
    py::set(vm, &main, "_location", loc)?;
    m.d = d;
    m.main = main;
    if first {
        register_widget(vm, ctx, screen, m)?;
    } else if py::truthy(vm, &m.widget_id)? {
        let w = py::get(vm, screen, "widgets")?;
        ops::setitem(vm, &w, m.widget_id.clone(), m.main.clone())?;
        let b = py::get(vm, screen, "base_widgets")?;
        ops::setitem(vm, &b, m.widget_id.clone(), m.d.clone())?;
    }
    Ok(())
}

/// The `at` property: wraps `d` in the transforms, reusing the previous
/// wrappers when the transform list is unchanged.
fn apply_at(
    vm: &mut Vm,
    context: &Ctx,
    cache: &Rc<RefCell<SlCache>>,
    reused: bool,
    transform: &Value,
    d: Value,
) -> PyResult<Value> {
    let raw = opt(&cache.borrow().raw_transform);
    if reused && ops::eq(vm, transform, &raw)? {
        let inner = opt(&cache.borrow().inner_transform);
        if is_transform(vm, &inner)? {
            let child = py::get_or_none(vm, &inner, "child")?;
            if !py::same(&child, &d) {
                py::method_kw(
                    vm,
                    &inner,
                    "set_child",
                    vec![d.clone()],
                    vec![("duplicate".into(), Value::Bool(false))],
                )?;
            }
        }
        return Ok(opt(&cache.borrow().outer_transform));
    }
    let mut old_outer = opt(&cache.borrow().outer_transform);
    {
        let mut c = cache.borrow_mut();
        c.raw_transform = Some(transform.clone());
        c.inner_transform = None;
        c.outer_transform = None;
    }
    let mut d = d;
    let call_child =
        |vm: &mut Vm, t: &Value, d: Value| vm.call(t, Vec::new(), vec![("child".into(), d)]);
    if is_transform(vm, transform)? {
        d = call_child(vm, transform, d)?;
        vm.call_method(&d, "_unique", Vec::new())?;
        let mut c = cache.borrow_mut();
        c.inner_transform = Some(d.clone());
        c.outer_transform = Some(d.clone());
    } else if let Some(list) = py::seq(transform) {
        for t in list {
            if is_transform(vm, &t)? {
                d = call_child(vm, &t, d)?;
                let mut c = cache.borrow_mut();
                c.outer_transform = Some(d.clone());
                if c.inner_transform.is_none() {
                    c.inner_transform = Some(d.clone());
                }
            } else {
                d = vm.call(&t, vec![d], Vec::new())?;
                let mut c = cache.borrow_mut();
                c.raw_transform = None;
                c.outer_transform = None;
                c.inner_transform = None;
            }
            vm.call_method(&d, "_unique", Vec::new())?;
        }
    } else {
        d = vm.call(transform, vec![d], Vec::new())?;
        vm.call_method(&d, "_unique", Vec::new())?;
        let mut c = cache.borrow_mut();
        c.raw_transform = None;
        c.outer_transform = None;
        c.inner_transform = None;
    }
    if is_transform(vm, &d)? {
        if !context.updating {
            old_outer = Value::None;
        }
        vm.call_method(&d, "take_state", vec![old_outer.clone()])?;
        vm.call_method(&d, "take_execution_state", vec![old_outer])?;
    }
    Ok(d)
}

fn wrap_in_showif(
    vm: &mut Vm,
    context: &Ctx,
    cache: &Rc<RefCell<SlCache>>,
    showif: bool,
    d: Value,
) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.sl2.sldisplayables", "ShowIf")?;
    let old = opt(&cache.borrow().old_showif);
    let rv = vm.call(&c, vec![Value::Bool(showif), old], Vec::new())?;
    vm.call_method(&rv, "add", vec![d])?;
    if !context.predicting {
        cache.borrow_mut().old_showif = Some(rv.clone());
    }
    Ok(rv)
}

// use / transclude / custom use ------------------------------------------------------------------------

fn screen_ast(vm: &mut Vm, name: &Value) -> PyResult<Option<(Value, Rc<ScreenAst>)>> {
    let f = py::import_attr(vm, "renpy.display.screen", "get_screen_variant")?;
    let n = match name.as_str() {
        Some(s) => {
            let first = s.split_whitespace().next().unwrap_or("").to_owned();
            py::s(vm, &first)
        }
        None => ops::getitem(vm, name, Value::Int(0))?,
    };
    let s = vm.call(&f, vec![n], Vec::new())?;
    if py::is_none(&s) {
        return Ok(None);
    }
    let ast = py::get_or_none(vm, &s, "ast")?;
    Ok(ast_of(&ast).map(|a| (ast, a)))
}

fn screen_exists(vm: &mut Vm, name: &Value) -> PyResult<bool> {
    let f = py::import_attr(vm, "renpy.display.screen", "get_screen_variant")?;
    let s = vm.call(&f, vec![name.clone()], Vec::new())?;
    Ok(!py::is_none(&s))
}

#[allow(clippy::too_many_arguments)]
fn exec_use(
    vm: &mut Vm,
    context: &mut Ctx,
    n: &SlNode,
    loc: &(Rc<str>, u32),
    target: &str,
    expression: bool,
    args: Option<&crate::ast::ArgumentInfo>,
    id: Option<&crate::lexer::PyExpr>,
    block: Option<&Block>,
) -> PyResult<()> {
    let key = Value::Int(serial(n) as i64);
    let (target_v, id_prefix) = if expression {
        let e = crate::lexer::PyExpr::new(target, loc.0.clone(), loc.1);
        (eval(vm, &e, &context.scope)?, py::s(vm, "_use_expression"))
    } else {
        (py::s(vm, target), py::s(vm, target))
    };
    let found = screen_ast(vm, &target_v)?;
    let Some((ast_v, ast)) = found else {
        if expression && !screen_exists(vm, &target_v)? {
            let t = py::string(vm, &target_v)?;
            return Err(py::error(vm, format!("A screen named {t} does not exist.")));
        }
        return use_screen_function(vm, context, &key, &target_v, loc, args);
    };
    let mut ctx = context.derive();
    let nc = vm.types.new_dict();
    py::dict_set(vm, &nc, "ast", ast_v.clone())?;
    ops::setitem(vm, &context.new_cache, key.clone(), nc.clone())?;
    ctx.new_cache = nc.clone();
    ctx.miss_cache = sub_dict(vm, &context.miss_cache, &key)?;
    if let Some(id) = id {
        let idv = eval(vm, id, &context.scope)?;
        let use_id = vm.types.new_tuple(vec![id_prefix, idv]);
        ctx.old_cache = match dget(vm, &context.old_use_cache, &use_id)? {
            Some(c) if is_dict(&c) && ops::len(vm, &c)? > 0 => c,
            _ => sub_dict(vm, &context.old_cache, &key)?,
        };
        ops::setitem(vm, &ctx.new_use_cache, use_id, nc.clone())?;
    } else {
        ctx.old_cache = sub_dict(vm, &context.old_cache, &key)?;
    }
    let (args, kwargs) = match args.map(|a| params::evaluate(vm, a, loc, &context.scope)) {
        None => (Vec::new(), Vec::new()),
        Some(Ok((a, k))) => params::split(vm, &a, &k)?,
        Some(Err(e)) if !context.predicting => return Err(e),
        Some(Err(_)) => (Vec::new(), Vec::new()),
    };
    run_used(vm, context, ctx, &ast, target, args, kwargs, block)
}

/// Shared tail of `SLUse.execute` and `SLCustomUse.execute`.
#[allow(clippy::too_many_arguments)]
fn run_used<'a>(
    vm: &mut Vm,
    context: &'a Ctx<'a>,
    mut ctx: Ctx<'a>,
    ast: &ScreenAst,
    target: &str,
    args: Vec<Value>,
    kwargs: Vec<(Rc<str>, Value)>,
    transclude: Option<&'a Block>,
) -> PyResult<()> {
    let sk = py::s(vm, "scope");
    let scope = match dget(vm, &ctx.old_cache, &sk)? {
        Some(s) if py::truthy(vm, &s)? => s,
        _ => match dget(vm, &ctx.miss_cache, &sk)? {
            Some(s) if py::truthy(vm, &s)? => s,
            _ => vm.types.new_dict(),
        },
    };
    if !ctx.updating {
        vm.call_method(&scope, "clear", Vec::new())?;
    }
    match &ast.screen.parameters {
        Some(sig) => {
            let values =
                params::apply(vm, sig, &ast.screen.loc, &args, &kwargs, context.predicting)?;
            vm.call_method(&scope, "update", vec![values])?;
        }
        None => {
            if !args.is_empty() {
                return Err(py::error(
                    vm,
                    format!(
                        "Screen {target} does not take positional arguments. ({} given)",
                        args.len()
                    ),
                ));
            }
            vm.call_method(&scope, "clear", Vec::new())?;
            vm.call_method(&scope, "update", vec![context.scope.clone()])?;
            for (k, v) in kwargs {
                py::dict_set(vm, &scope, &k, v)?;
            }
        }
    }
    py::dict_set(vm, &scope, "_scope", scope.clone())?;
    py::dict_set(vm, &ctx.new_cache, "scope", scope.clone())?;
    ctx.scope = scope.clone();
    ctx.parent = Some(context);
    ctx.transclude = transclude;
    let r = {
        // Lifetimes: the used screen's AST is borrowed for this call only.
        let mut inner = Ctx { ast, ..ctx.clone() };
        execute_screen_block(vm, &mut inner).map(|_| inner.fail.get())
    };
    let _ = dpop(vm, &scope, "_scope");
    if r? {
        context.fail.set(true);
    }
    Ok(())
}

fn use_screen_function(
    vm: &mut Vm,
    context: &Ctx,
    key: &Value,
    target: &Value,
    loc: &(Rc<str>, u32),
    args: Option<&crate::ast::ArgumentInfo>,
) -> PyResult<()> {
    let n = match dget(vm, &context.use_index, key)? {
        Some(Value::Int(i)) => i,
        _ => 0,
    };
    ops::setitem(vm, &context.use_index, key.clone(), Value::Int(n + 1))?;
    let outer = match dget(vm, &context.scope, &py::s(vm, "_name"))? {
        Some(v) => v,
        None => vm.types.new_tuple(Vec::new()),
    };
    let name = vm.types.new_tuple(vec![outer, key.clone(), Value::Int(n)]);
    let (a, mut k) = match args {
        Some(a) => {
            let (a, k) = params::evaluate(vm, a, loc, &context.scope)?;
            params::split(vm, &a, &k)?
        }
        None => (Vec::new(), Vec::new()),
    };
    let mut pos = vec![target.clone()];
    pos.extend(a);
    k.push(("_name".into(), name));
    k.push(("_scope".into(), context.scope.clone()));
    let f = py::import_attr(vm, "renpy.display.screen", "use_screen")?;
    vm.call(&f, pos, k)?;
    Ok(())
}

fn exec_transclude(vm: &mut Vm, context: &mut Ctx, n: &SlNode) -> PyResult<()> {
    let Some(block) = context.transclude else {
        return Ok(());
    };
    let key = Value::Int(serial(n) as i64);
    let base = context.parent.unwrap_or(context);
    let mut ctx = base.derive();
    ctx.new_cache = vm.types.new_dict();
    ops::setitem(vm, &context.new_cache, key.clone(), ctx.new_cache.clone())?;
    ctx.old_cache = sub_dict(vm, &context.old_cache, &key)?;
    ctx.miss_cache = sub_dict(vm, &context.miss_cache, &key)?;
    py::dict_set(vm, &ctx.new_cache, "transclude", Value::Bool(true))?;
    ctx.children = context.children.clone();
    ctx.showif = context.showif;
    ctx.push(vm)?;
    let r = (|| -> PyResult<()> {
        block_keywords(vm, &mut ctx, block)?;
        exec_children(vm, &mut ctx, &block.children)?;
        Ok(())
    })();
    pop_stack(vm)?;
    r?;
    if ctx.fail.get() {
        context.fail.set(true);
    }
    Ok(())
}

fn exec_custom_use(
    vm: &mut Vm,
    context: &mut Ctx,
    n: &SlNode,
    target: &str,
    positional: &[crate::lexer::PyExpr],
    block: &Block,
) -> PyResult<()> {
    let key = Value::Int(serial(n) as i64);
    let mut ctx = context.derive();
    ctx.new_cache = vm.types.new_dict();
    ops::setitem(vm, &context.new_cache, key.clone(), ctx.new_cache.clone())?;
    ctx.miss_cache = sub_dict(vm, &context.miss_cache, &key)?;
    let r = (|| -> PyResult<(Vec<Value>, Value)> {
        let mut args = Vec::new();
        for e in positional {
            args.push(eval(vm, e, &context.scope)?);
        }
        let kwargs = vm.types.new_dict();
        ctx.keywords = kwargs.clone();
        block_keywords(vm, &mut ctx, block)?;
        if let Some(a) = dpop(vm, &kwargs, "arguments")? {
            if py::truthy(vm, &a)? {
                args.extend(py::items(vm, &a)?);
            }
        }
        if let Some(p) = dpop(vm, &kwargs, "properties")? {
            if py::truthy(vm, &p)? {
                vm.call_method(&kwargs, "update", vec![p])?;
            }
        }
        if let Some(suffix) = dpop(vm, &kwargs, "style_suffix")? {
            if py::truthy(vm, &suffix)? && !dcontains(vm, &kwargs, "style")? {
                let style = if py::is_none(&ctx.style_prefix) {
                    suffix
                } else {
                    let p = py::string(vm, &ctx.style_prefix)?;
                    let s = py::string(vm, &suffix)?;
                    py::s(vm, &format!("{p}_{s}"))
                };
                py::dict_set(vm, &kwargs, "style", style)?;
            }
        }
        Ok((args, kwargs))
    })();
    let (args, kwargs) = match r {
        Ok(v) => v,
        Err(e) if !context.predicting => return Err(e),
        Err(_) => (Vec::new(), vm.types.new_dict()),
    };
    let id = dpop(vm, &kwargs, "id")?.unwrap_or(Value::None);
    if !py::is_none(&id) {
        let t = py::s(vm, target);
        let use_id = vm.types.new_tuple(vec![t, id]);
        ctx.old_cache = match dget(vm, &context.old_use_cache, &use_id)? {
            Some(c) if is_dict(&c) && ops::len(vm, &c)? > 0 => c,
            _ => sub_dict(vm, &context.old_cache, &key)?,
        };
        ops::setitem(vm, &ctx.new_use_cache, use_id, ctx.new_cache.clone())?;
    } else {
        ctx.old_cache = sub_dict(vm, &context.old_cache, &key)?;
    }
    let t = py::s(vm, target);
    let Some((_, ast)) = screen_ast(vm, &t)? else {
        return Err(py::error(
            vm,
            format!("A screen named {target} does not exist."),
        ));
    };
    let kwargs = py::str_items(vm, &kwargs)?;
    let transclude = if block.children.is_empty() {
        None
    } else {
        Some(block)
    };
    run_used(vm, context, ctx, &ast, target, args, kwargs, transclude)
}

/// `SLScreen.execute`: the screen block's properties, then its children.
fn execute_screen_block(vm: &mut Vm, ctx: &mut Ctx) -> PyResult<()> {
    let ast = ctx.ast;
    block_keywords(vm, ctx, &ast.screen.block)?;
    exec_children(vm, ctx, &ast.screen.block.children)?;
    Ok(())
}

// SLScreen methods -------------------------------------------------------------------------------------

fn this_ast(vm: &Vm, v: &Value) -> PyResult<Rc<ScreenAst>> {
    ast_of(v).ok_or_else(|| vm.type_error("SLScreen expected"))
}

/// `SLScreen.__call__(**scope)`: one update of the current screen.
fn s_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ast = this_ast(vm, this)?;
    let scope = py::kw(kw, "_scope")
        .cloned()
        .ok_or_else(|| vm.type_error("SLScreen called without _scope"))?;
    let predicting = predicting(vm);
    if let Some(sig) = &ast.screen.parameters {
        let a = dget(vm, &scope, &py::s(vm, "_args"))?.unwrap_or(Value::None);
        let k = dget(vm, &scope, &py::s(vm, "_kwargs"))?.unwrap_or(Value::None);
        let (a, k) = params::split(vm, &a, &k)?;
        let values = params::apply_arguments(vm, Some(sig), &ast.screen.loc, &a, &k, predicting)?;
        vm.call_method(&scope, "update", vec![values])?;
    }
    let cs_f = py::import_attr(vm, "renpy.display.screen", "current_screen")?;
    let current = vm.call(&cs_f, Vec::new(), Vec::new())?;
    let phase = py::get(vm, &current, "phase")?;
    let update = py::import_attr(vm, "renpy.display.screen", "UPDATE")?;
    let updating = ops::eq(vm, &phase, &update)?;
    let name = dget(vm, &scope, &py::s(vm, "_name"))?.unwrap_or(Value::Int(0));
    let version = Value::Int(ast.version);
    let get_cache = |vm: &mut Vm, d: &Value| -> PyResult<Value> {
        if let Some(rv) = dget(vm, d, &name)? {
            if is_dict(&rv) {
                let vk = py::s(vm, "version");
                if let Some(v) = dget(vm, &rv, &vk)? {
                    if ops::eq(vm, &v, &version)? {
                        return Ok(rv);
                    }
                }
            }
        }
        let rv = vm.types.new_dict();
        py::dict_set(vm, &rv, "version", version.clone())?;
        ops::setitem(vm, d, name.clone(), rv.clone())?;
        Ok(rv)
    };
    let cur_cache = py::get(vm, &current, "cache")?;
    let old_cache = get_cache(vm, &cur_cache)?;
    let cur_miss = py::get(vm, &current, "miss_cache")?;
    let miss_cache = get_cache(vm, &cur_miss)?;
    let new_cache = vm.types.new_dict();
    py::dict_set(vm, &new_cache, "version", version.clone())?;
    let old_use_cache = py::get(vm, &current, "use_cache")?;
    let mut ctx = Ctx {
        ast: &ast,
        scope: scope.clone(),
        root_scope: scope.clone(),
        children: vm.types.new_list(Vec::new()),
        keywords: vm.types.new_dict(),
        style_prefix: Value::None,
        new_cache: new_cache.clone(),
        old_cache,
        miss_cache,
        use_index: vm.types.new_dict(),
        predicting,
        updating,
        predicted: vm.types.new_set(Default::default()),
        showif: None,
        fail: Cell::new(false),
        parent: None,
        transclude: None,
        unlikely: false,
        old_use_cache,
        new_use_cache: vm.types.new_dict(),
        sticky: Vec::new(),
    };
    execute_screen_block(vm, &mut ctx)?;
    let ia = py::import_attr(vm, "renpy.ui", "implicit_add")?;
    for d in py::items(vm, &ctx.children)? {
        vm.call(&ia, vec![d], Vec::new())?;
    }
    ops::setitem(vm, &cur_cache, name, new_cache)?;
    py::set(vm, &current, "use_cache", ctx.new_use_cache.clone())?;
    Ok(Value::None)
}

/// `SLScreen.define(location)`.
fn s_define(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ast = this_ast(vm, this)?;
    let s = &ast.screen;
    let ev = |vm: &mut Vm, t: &str| params::eval_text(vm, t, &s.loc, &Value::None);
    let variant = ev(vm, &s.variant)?;
    let predict = ev(vm, &s.predict)?;
    let layer = ev(vm, &s.layer)?;
    let roll_forward = ev(vm, &s.roll_forward)?;
    let loc = match arg(args, kw, 1, "location") {
        Some(l) if !py::is_none(l) => l.clone(),
        _ => location(vm, &s.loc),
    };
    let params = py::get_or_none(vm, this, "parameters")?;
    let f = py::import_attr(vm, "renpy.display.screen", "define_screen")?;
    let mut k: Vec<(Rc<str>, Value)> = Vec::new();
    k.push(("modal".into(), py::s(vm, &s.modal)));
    k.push(("zorder".into(), py::s(vm, &s.zorder)));
    k.push((
        "tag".into(),
        s.tag.as_ref().map(|t| py::s(vm, t)).unwrap_or(Value::None),
    ));
    k.push(("variant".into(), variant));
    k.push(("predict".into(), predict));
    k.push(("parameters".into(), params));
    k.push(("location".into(), loc));
    k.push(("layer".into(), layer));
    k.push(("sensitive".into(), py::s(vm, &s.sensitive)));
    k.push(("roll_forward".into(), roll_forward));
    let name = py::s(vm, &s.name);
    vm.call(&f, vec![name, this.clone()], k)?;
    Ok(Value::None)
}

fn s_copy_on_change(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cache = args.get(1).cloned().unwrap_or(Value::None);
    mark_copy_on_change(vm, &cache, &mut HashSet::new())?;
    Ok(Value::None)
}

pub(crate) fn walk_used(vm: &mut Vm, b: &Block, cb: &Value) -> PyResult<()> {
    for n in &b.children {
        walk_used_node(vm, n, cb)?;
    }
    Ok(())
}

pub(crate) fn walk_used_node(vm: &mut Vm, n: &SlNode, cb: &Value) -> PyResult<()> {
    match n {
        SlNode::Displayable(d) => walk_used(vm, &d.block, cb),
        SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => {
            for (_, b) in entries {
                walk_used(vm, b, cb)?;
            }
            Ok(())
        }
        SlNode::For { block, .. } => walk_used(vm, block, cb),
        SlNode::Use {
            target,
            expression,
            block,
            ..
        } => {
            if !*expression {
                let t = py::s(vm, target);
                vm.call(cb, vec![t], Vec::new())?;
            }
            if let Some(b) = block {
                walk_used(vm, b, cb)?;
            }
            Ok(())
        }
        SlNode::CustomUse { target, block, .. } => {
            let t = py::s(vm, target);
            vm.call(cb, vec![t], Vec::new())?;
            walk_used(vm, block, cb)
        }
        _ => Ok(()),
    }
}

fn s_used_screens(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ast = this_ast(vm, &args[0])?;
    let cb = args.get(1).cloned().unwrap_or(Value::None);
    walk_used(vm, &ast.screen.block, &cb)?;
    Ok(Value::None)
}

pub(crate) fn block_has_transclude(b: &Block) -> bool {
    b.children.iter().any(|n| match n {
        SlNode::Transclude { .. } => true,
        SlNode::Displayable(d) => block_has_transclude(&d.block),
        SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => {
            entries.iter().any(|(_, b)| block_has_transclude(b))
        }
        SlNode::For { block, .. } => block_has_transclude(block),
        SlNode::Use { block: Some(b), .. } => block_has_transclude(b),
        SlNode::CustomUse { block, .. } => block_has_transclude(block),
        _ => false,
    })
}

fn s_has_transclude(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ast = this_ast(vm, &args[0])?;
    Ok(Value::Bool(block_has_transclude(&ast.screen.block)))
}

fn s_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn s_reduce(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Screen ASTs are not saved."))
}

fn s_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ast = this_ast(vm, &args[0])?;
    Ok(py::s(
        vm,
        &format!(
            "<SLScreen {} at {}:{}>",
            ast.screen.name, ast.screen.loc.0, ast.screen.loc.1
        ),
    ))
}

/// `renpy.sl2.slast.SLContext`: the widget-stack face of an SL context.
/// `SLContext(parent=None)`: a copy of `parent`'s state, or a fresh one.
fn ctx_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let parent = arg_or(args, kw, 1, "parent", Value::None);
    if !py::is_none(&parent) {
        let src = py::get(vm, &parent, "__dict__")?;
        let dst = py::get(vm, this, "__dict__")?;
        vm.call_method(&dst, "update", vec![src])?;
        return Ok(Value::None);
    }
    let scope = vm.types.new_dict();
    py::set(vm, this, "scope", scope.clone())?;
    py::set(vm, this, "root_scope", scope)?;
    py::set(vm, this, "globals", vm.types.new_dict())?;
    py::set(vm, this, "children", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "keywords", vm.types.new_dict())?;
    py::set(vm, this, "style_prefix", Value::None)?;
    for n in ["new_cache", "old_cache", "miss_cache", "new_use_cache", "old_use_cache"] {
        py::set(vm, this, n, vm.types.new_dict())?;
    }
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let int = Value::Obj(vm.types.int.clone());
    let ui = vm.call(&dd, vec![int], Vec::new())?;
    py::set(vm, this, "use_index", ui)?;
    for n in ["uses_scope", "widgets", "showif", "parent", "transclude"] {
        py::set(vm, this, n, Value::None)?;
    }
    for n in ["debug", "predicting", "updating", "fail", "unlikely"] {
        py::set(vm, this, n, Value::Bool(false))?;
    }
    py::set(vm, this, "predicted", vm.types.new_set(Default::default()))?;
    if super::slparser::release(vm) >= (7, 8) {
        py::set(vm, this, "sticky", vm.types.new_dict())?;
    }
    Ok(Value::None)
}

fn ctx_get_style_group(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get_or_none(vm, &args[0], "style_prefix")
}

fn ctx_set_style_group(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "style_prefix", args[1].clone())?;
    Ok(Value::None)
}

// Module functions -------------------------------------------------------------------------------------

/// `compile_expr(loc, node)` (`compile_expr(node)` in 6.18): compiles an
/// expression for evaluation. Expressions are given as source (`str` or
/// `PyExpr`); Python `ast` node objects have no host in this runtime.
fn f_compile_expr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (loc, node) = match args.len() {
        1 => (Value::None, args[0].clone()),
        _ => (args[0].clone(), args.get(1).cloned().unwrap_or(Value::None)),
    };
    if node.as_str().is_none() {
        return Err(vm.type_error(
            "compile_expr: Python ast nodes are not available in this runtime (no `ast` \
             compiler); pass the expression source",
        ));
    }
    let f = py::import_attr(vm, "renpy.python", "py_compile")?;
    let mut kw: Vec<(Rc<str>, Value)> = Vec::new();
    if !py::is_none(&loc) {
        kw.push(("filename".into(), ops::getitem(vm, &loc, Value::Int(0))?));
        kw.push(("lineno".into(), ops::getitem(vm, &loc, Value::Int(1))?));
    } else {
        kw.push(("filename".into(), py::s(vm, "<screen language>")));
    }
    vm.call(&f, vec![node, py::s(vm, "eval")], kw)
}

/// `analyze_keywords(node, analysis, conditional=GLOBAL_CONST)`.
fn f_analyze_keywords(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let node = arg_or(args, kw, 0, "node", Value::None);
    let analysis = arg_or(args, kw, 1, "analysis", Value::None);
    let gc = const_value(vm, "GLOBAL_CONST")?;
    let mut conditional = arg_or(args, kw, 2, "conditional", gc.clone());
    let min = |vm: &mut Vm, a: Value, b: Value| -> PyResult<Value> {
        Ok(if py::lt(vm, &b, &a)? { b } else { a })
    };
    let mut rv = gc;
    for k in py::get_items(vm, &node, "keyword")? {
        let e = ops::getitem(vm, &k, Value::Int(1))?;
        let c = vm.call_method(&analysis, "is_constant_expr", vec![e])?;
        rv = min(vm, rv, c)?;
        rv = min(vm, rv, conditional.clone())?;
    }
    let sl_if = py::import_attr(vm, MODULE, "SLIf")?;
    for n in py::get_items(vm, &node, "children")? {
        if !sl_if.obj().is_some_and(|c| ops::isinstance(vm, &n, c)) {
            continue;
        }
        for entry in py::get_items(vm, &n, "entries")? {
            let cond = ops::getitem(vm, &entry, Value::Int(0))?;
            let block = ops::getitem(vm, &entry, Value::Int(1))?;
            if !py::is_none(&cond) {
                let c = vm.call_method(&analysis, "is_constant_expr", vec![cond])?;
                conditional = min(vm, conditional, c)?;
            }
            let sub = f_analyze_keywords(vm, &[block, analysis.clone(), conditional.clone()], &[])?;
            rv = min(vm, rv, sub)?;
        }
    }
    Ok(rv)
}

/// `predict_pause(context)` (8.6): a generator that yields when an
/// incremental prediction has used up its time slice.
fn f_predict_pause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let context = args.first().cloned().unwrap_or(Value::None);
    let yp = py::get_opt(vm, &context, "yield_prediction")?.unwrap_or(Value::Bool(false));
    let mut pause = py::truthy(vm, &yp)? && predicting(vm);
    if pause {
        let next = py::import_attr(vm, "renpy.display.predict", "next_predict_pause_ns");
        let now = py::import_attr(vm, "time", "perf_counter_ns")
            .and_then(|f| vm.call(&f, Vec::new(), Vec::new()));
        if let (Ok(next), Ok(now)) = (next, now) {
            pause = !py::lt(vm, &now, &next)?;
        }
    }
    let items = if pause { vec![Value::None] } else { Vec::new() };
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

fn sc_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "version", Value::Int(1))?;
    py::set(vm, this, "const_analyzed", vm.types.new_dict())?;
    py::set(vm, this, "not_const_analyzed", vm.types.new_dict())?;
    py::set(vm, this, "updated", Value::Bool(false))?;
    Ok(Value::None)
}

fn ctx_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "children")?;
    ops::list_append(vm, &c, args[1].clone())?;
    Ok(Value::None)
}

fn ctx_close(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Spurious ui.close()."))
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("cannot create instances directly"))
}

fn native_type(vm: &mut Vm, name: &str, base: &ObjRef, methods: py::Methods) -> ObjRef {
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

/// `from renpy.pyanalysis import ...`: the analysis levels keep upstream's
/// values when that module does not define them.
fn import_analysis(vm: &mut Vm, m: &ObjRef, rel: (u32, u32)) {
    let mut names = vec![("Analysis", None), ("NOT_CONST", Some(0)), ("GLOBAL_CONST", Some(2))];
    if rel >= (6, 99) {
        names.push(("ccache", None));
    }
    if rel >= (7, 3) {
        names.push(("LOCAL_CONST", Some(1)));
    }
    for (n, fallback) in names {
        let v = match py::import_attr(vm, "renpy.pyanalysis", n) {
            Ok(v) => Some(v),
            Err(_) => fallback.map(Value::Int),
        };
        if let Some(v) = v {
            crysirror_python::modules::set(vm, m, n, v);
        }
    }
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let rel = super::slparser::release(vm);
    let child_statements = rel >= (7, 6) && (rel.0, rel.1) != (8, 0);
    // The screen-analysis cache (game/cache/screens.rpyb) only lets
    // upstream skip constant analysis at start-up. Screens here are never
    // const-analysed, so there is nothing to load or save: `load_cache`
    // and `save_cache` leave `scache` empty, as upstream does without a
    // cache file.
    let mut funcs: Vec<(&'static str, crysirror_python::object::NativeFn)> =
        vec![("compile_expr", f_compile_expr)];
    if rel >= (6, 99) {
        funcs.push(("load_cache", s_noop));
        funcs.push(("save_cache", s_noop));
    }
    if child_statements {
        funcs.push(("analyze_keywords", f_analyze_keywords));
    }
    if rel >= (8, 6) {
        funcs.push(("predict_pause", f_predict_pause));
    }
    let m = crysirror_python::modules::module_with(vm, MODULE, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    import_analysis(vm, &m, rel);
    // Imports of slast.py.
    let tf = if rel >= (7, 0) {
        "renpy.display.transform"
    } else {
        "renpy.display.motion"
    };
    let t = py::import_attr(vm, tf, "Transform")?;
    set(vm, "Transform", t);
    if rel >= (7, 3) {
        let a = py::import_attr(vm, "renpy.display.transform", "ATLTransform")?;
        set(vm, "ATLTransform", a);
    }
    let fixed = py::import_attr(vm, "renpy.display.layout", "Fixed")?;
    set(vm, "Fixed", fixed);
    let pd = py::import_attr(vm, "renpy.display.predict", "displayable")?;
    set(vm, "predict_displayable", pd);
    let peb = py::import_attr(vm, "renpy.python", "py_eval_bytecode")?;
    set(vm, "py_eval_bytecode", peb);
    if rel < (6, 99) {
        let pc = py::import_attr(vm, "renpy.python", "py_compile")?;
        set(vm, "py_compile", pc);
    }
    if rel >= (8, 4) {
        let e = py::import_attr(vm, "renpy.python", "py_eval")?;
        set(vm, "eval", e);
    }
    if rel >= (7, 4) {
        let pm = if py::import(vm, "renpy.compat.pickle").is_ok() {
            "renpy.compat.pickle"
        } else {
            "pickle"
        };
        for n in ["dumps", "loads"] {
            let v = py::import_attr(vm, pm, n)?;
            set(vm, n, v);
        }
    }
    // Module data.
    let serial = if rel >= (6, 99) {
        let t = py::import_attr(vm, "time", "time")?;
        let now = vm.call(&t, Vec::new(), Vec::new())?;
        (py::f64_of(vm, &now)? * 1_000_000.0) as i64
    } else {
        0
    };
    set(vm, "serial", Value::Int(serial));
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let ue = vm.call(&sentinel, vec![py::s(vm, "use_expression")], Vec::new())?;
    set(vm, "use_expression", ue);
    let ng = vm.call(&sentinel, vec![py::s(vm, "NotGiven")], Vec::new())?;
    set(vm, "NotGiven", ng);
    set(vm, "filename", py::s(vm, "<screen language>"));
    let log = match py::import_attr(vm, "renpy.log", "open") {
        Ok(open) => vm.call(
            &open,
            vec![py::s(vm, "profile_screen")],
            vec![
                ("developer".into(), Value::Bool(true)),
                ("append".into(), Value::Bool(false)),
                ("flush".into(), Value::Bool(false)),
            ],
        )?,
        Err(_) => Value::None,
    };
    set(vm, "profile_log", log);
    if rel == (7, 8) || rel >= (8, 2) {
        let l = vm.types.new_list(STICKY_PROPERTIES.iter().map(|s| vm.types.new_str(s)).collect());
        set(vm, "STICKY_PROPERTIES", l);
    }
    if rel < (8, 6) {
        let l = Value::Obj(vm.types.list.clone());
        let t = Value::Obj(vm.types.tuple.clone());
        let lt = vm.types.new_tuple(vec![l, t]);
        set(vm, "list_or_tuple", lt);
    }
    // Node classes; SLScreen is an SLBlock.
    let block = super::slast_nodes::install_classes(vm, &m)?;
    let scr = native_type(
        vm,
        "SLScreen",
        &block,
        &[
            ("__call__", s_call),
            ("define", s_define),
            ("copy_on_change", s_copy_on_change),
            ("used_screens", s_used_screens),
            ("has_transclude", s_has_transclude),
            ("analyze_screen", s_noop),
            ("prepare_screen", s_noop),
            ("unprepare_screen", s_noop),
            ("__reduce__", s_reduce),
            ("__reduce_ex__", s_reduce),
            ("__repr__", s_repr),
        ],
    );
    set(vm, "SLScreen", Value::Obj(scr));
    let object = vm.types.object.clone();
    let cache = native_type(
        vm,
        "SLCache",
        &object,
        &[("__reduce__", s_reduce), ("__reduce_ex__", s_reduce)],
    );
    set(vm, "SLCache", Value::Obj(cache));
    let addable = py::import_attr(vm, "renpy.ui", "Addable")?;
    let ctx = py::class(
        vm,
        MODULE,
        "SLContext",
        &[addable],
        &[
            ("__init__", ctx_init),
            ("add", ctx_add),
            ("close", ctx_close),
        ],
        &[],
    )?;
    let g = vm.types.new_builtin("get_style_group", ctx_get_style_group);
    let s = vm.types.new_builtin("set_style_group", ctx_set_style_group);
    let p = py::property(vm, g, Some(s))?;
    py::set(vm, &Value::Obj(ctx.clone()), "style_group", p)?;
    set(vm, "SLContext", Value::Obj(ctx));
    if rel >= (6, 99) {
        let null = py::import_attr(vm, "renpy.display.layout", "Null")?;
        let nd = vm.call(&null, Vec::new(), Vec::new())?;
        set(vm, "NO_DISPLAYABLE", nd);
        let sc = py::class(vm, MODULE, "ScreenCache", &[], &[("__init__", sc_init)], &[])?;
        let inst = vm.call(&Value::Obj(sc.clone()), Vec::new(), Vec::new())?;
        set(vm, "ScreenCache", Value::Obj(sc));
        set(vm, "scache", inst);
        set(vm, "CACHE_FILENAME", py::s(vm, "cache/screens.rpyb"));
    }
    // The loop-control exceptions exist for pickled screens of any
    // release; they are module names from 7.6 / 8.1.
    let exc = Value::Obj(vm.types.exc("Exception"));
    let fe = Value::Obj(py::class(vm, MODULE, "SLForException", &[exc], &[], &[])?);
    let b = py::class(vm, MODULE, "SLBreakException", &[fe.clone()], &[], &[])?;
    let c = py::class(vm, MODULE, "SLContinueException", &[fe.clone()], &[], &[])?;
    if child_statements {
        set(vm, "SLForException", fe);
        set(vm, "SLBreakException", Value::Obj(b));
        set(vm, "SLContinueException", Value::Obj(c));
    } else {
        set(vm, "_SLBreakException", Value::Obj(b));
        set(vm, "_SLContinueException", Value::Obj(c));
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
