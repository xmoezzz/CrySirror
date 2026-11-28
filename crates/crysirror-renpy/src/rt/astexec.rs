// SPDX-License-Identifier: MPL-2.0
//! `renpy.ast` at run time, after Ren'Py 8.3 `ast.py` (MIT): the
//! `execute` and `predict` behaviour of every statement node over the Rust
//! script arena, and the module helpers (`next_node`, `statement_name`,
//! `eval_who`, `say_menu_with`, `show_imspec`, `predict_imspec`,
//! namespaces and the define/default statement lists).
use super::{
    nodes,
    py::{self, *},
    python,
    state::rt,
};
use crate::{
    ast::{ImSpec, NodeId, NodeKind, PyCode},
    lexer::PyExpr,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.ast";

const EARLY_CONFIG: &[&str] = &[
    "save_directory",
    "allow_duplicate_labels",
    "keyword_after_python",
    "steam_appid",
    "name",
    "version",
    "save_token_keys",
    "check_conflicting_properties",
    "check_translate_none",
    "defer_tl_scripts",
    "munge_in_strings",
    "interface_layer",
];

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn context(vm: &mut Vm) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.game", "context")?;
    vm.call(&f, Vec::new(), Vec::new())
}

fn exports(
    vm: &mut Vm,
    name: &str,
    args: Vec<Value>,
    kw: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", name)?;
    vm.call(&f, args, kw)
}

fn store(vm: &mut Vm) -> PyResult<Value> {
    python::store_dict(vm, "store")
}

fn store_set(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let s = store(vm)?;
    py::dict_set(vm, &s, n, v)
}

fn store_get(vm: &mut Vm, n: &str) -> PyResult<Value> {
    let s = store(vm)?;
    let k = py::s(vm, n);
    Ok(ops::dict_get(vm, s.obj().unwrap(), &k)?.unwrap_or(Value::None))
}

/// Evaluates a script expression in the store.
pub fn ev(vm: &mut Vm, e: &PyExpr) -> PyResult<Value> {
    let c = python::compile(vm, e.text.trim(), "eval", &e.filename, e.line.max(1), None)?;
    python::eval_bytecode(vm, c, &Value::None, &Value::None)
}

fn ev_text(vm: &mut Vm, text: &str, filename: &str, line: u32) -> PyResult<Value> {
    let c = python::compile(vm, text.trim(), "eval", filename, line.max(1), None)?;
    python::eval_bytecode(vm, c, &Value::None, &Value::None)
}

fn py_of(code: &PyCode) -> Option<i64> {
    code.py3.then_some(3)
}

fn exec_code(vm: &mut Vm, code: &PyCode, hide: bool, store: &str) -> PyResult<()> {
    let mode = if code.mode == "hide" || hide {
        "hide"
    } else {
        "exec"
    };
    let c = python::compile(
        vm,
        &code.source.text,
        mode,
        &code.source.filename,
        code.source.line.max(1),
        py_of(code),
    )?;
    python::exec_bytecode(vm, c, hide, &Value::None, &Value::None, store)
}

fn eval_code(vm: &mut Vm, code: &PyCode) -> PyResult<Value> {
    let c = python::compile(
        vm,
        code.source.text.trim(),
        "eval",
        &code.source.filename,
        code.source.line.max(1),
        py_of(code),
    )?;
    python::eval_bytecode(vm, c, &Value::None, &Value::None)
}

// Module helpers ---------------------------------------------------------------------------------------

pub fn statement_name(vm: &mut Vm, name: &str) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    let n = py::s(vm, name);
    py::set(vm, &m, "current_statement_name", n.clone())?;
    let cbs = py::config(vm, "statement_callbacks")?;
    for cb in py::items(vm, &cbs)? {
        vm.call(&cb, vec![n.clone()], Vec::new())?;
    }
    Ok(())
}

fn f_statement_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    statement_name(vm, &n)?;
    Ok(Value::None)
}

pub fn next_node(vm: &mut Vm, n: Value) -> PyResult<()> {
    let ctx = context(vm)?;
    py::set(vm, &ctx, "next_node", n)
}

fn next_id(vm: &mut Vm, id: Option<NodeId>) -> PyResult<()> {
    let h = nodes::opt_handle(vm, id)?;
    next_node(vm, h)
}

fn f_next_node(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    next_node(vm, args[0].clone())?;
    Ok(Value::None)
}

/// `say_menu_with(expression, callback)`.
fn say_menu_with(vm: &mut Vm, expr: &Option<PyExpr>, callback: &Value) -> PyResult<()> {
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    let transitions = py::get_or_none(vm, &prefs, "transitions")?;
    let what = match expr {
        Some(e) => ev(vm, e)?,
        None => {
            let dt = store_get(vm, "default_transition")?;
            if py::truthy(vm, &dt)? && matches!(transitions, Value::Int(2)) {
                dt
            } else {
                return Ok(());
            }
        }
    };
    if !py::truthy(vm, &what)? {
        return Ok(());
    }
    if py::truthy(vm, &transitions)? {
        vm.call(callback, vec![what], Vec::new())?;
    }
    Ok(())
}

fn f_say_menu_with(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let e = if py::is_none(&args[0]) {
        None
    } else {
        Some(PyExpr::new(py::string(vm, &args[0])?, "<none>".into(), 1))
    };
    say_menu_with(vm, &e, &args[1])?;
    Ok(Value::None)
}

/// `eval_who(who, fast)`: character stores first, then evaluation.
pub fn eval_who(vm: &mut Vm, who: &Option<PyExpr>) -> PyResult<Value> {
    let Some(who) = who else {
        return Ok(Value::None);
    };
    let name = py::s(vm, &who.text);
    let sds = py::import_attr(vm, "renpy.python", "store_dicts")?;
    let ck = py::s(vm, "store.character");
    if let Some(d) = ops::dict_get(vm, sds.obj().unwrap(), &ck)? {
        if let Some(v) = ops::dict_get(vm, d.obj().unwrap(), &name)? {
            if !py::is_none(&v) {
                return Ok(v);
            }
        }
    }
    let s = store(vm)?;
    if let Some(v) = ops::dict_get(vm, s.obj().unwrap(), &name)? {
        if !py::is_none(&v) {
            return Ok(v);
        }
    }
    match ev(vm, who) {
        Ok(v) => Ok(v),
        Err(e) if matches!(e, crysirror_python::PyErr::Suspend(..)) => Err(e),
        Err(_) => Err(py::error(
            vm,
            format!("Sayer '{}' is not defined.", who.text),
        )),
    }
}

fn f_eval_who(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[0]) {
        return Ok(Value::None);
    }
    let t = py::string(vm, &args[0])?;
    eval_who(vm, &Some(PyExpr::new(t, "<none>".into(), 1)))
}

fn str_tuple(vm: &Vm, v: &[String]) -> Value {
    let items = v.iter().map(|s| vm.types.new_str(s)).collect();
    vm.types.new_tuple(items)
}

/// `show_imspec(imspec, atl)`.
fn show_imspec(vm: &mut Vm, spec: &ImSpec, atl: &Option<Rc<crate::atl::RawBlock>>) -> PyResult<()> {
    let zorder = match &spec.zorder {
        Some(z) => ev(vm, z)?,
        None => Value::None,
    };
    let expression = match &spec.expression {
        Some(e) => {
            let v = ev(vm, e)?;
            super::easy::displayable(vm, &v, &Value::None, false)?
        }
        None => Value::None,
    };
    let mut at_list = Vec::new();
    for e in &spec.at_list {
        at_list.push(ev(vm, e)?);
    }
    let name = str_tuple(vm, &spec.name);
    let tag = spec
        .tag
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    let layer = spec
        .layer
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    let tag_or_name = if py::is_none(&tag) {
        name.clone()
    } else {
        tag.clone()
    };
    let expr_untagged = !py::is_none(&expression) && py::is_none(&tag);
    let layer = exports(
        vm,
        "default_layer",
        vec![layer, tag_or_name, Value::Bool(expr_untagged)],
        Vec::new(),
    )?;
    let atl = match atl {
        Some(a) => super::atlrt::raw_value(vm, a.clone())?,
        None => Value::None,
    };
    let behind = vm
        .types
        .new_list(spec.behind.iter().map(|b| vm.types.new_str(b)).collect());
    let show = py::config(vm, "show")?;
    vm.call(
        &show,
        vec![name],
        vec![
            ("at_list".into(), vm.types.new_list(at_list)),
            ("layer".into(), layer),
            ("what".into(), expression),
            ("zorder".into(), zorder),
            ("tag".into(), tag),
            ("behind".into(), behind),
            ("atl".into(), atl),
        ],
    )?;
    Ok(())
}

/// `predict_imspec(imspec, scene, atl)`; prediction failures are ignored.
fn predict_imspec(
    vm: &mut Vm,
    spec: &ImSpec,
    scene: bool,
    atl: &Option<Rc<crate::atl::RawBlock>>,
) -> PyResult<()> {
    let img = match &spec.expression {
        Some(e) => {
            match ev(vm, e).and_then(|v| super::easy::displayable(vm, &v, &Value::None, false)) {
                Ok(v) => v,
                Err(_) => return Ok(()),
            }
        }
        None => Value::None,
    };
    let mut at_list = Vec::new();
    for e in &spec.at_list {
        if let Ok(v) = ev(vm, e) {
            at_list.push(v);
        }
    }
    if let Some(a) = atl {
        if let Ok(t) =
            super::atlrt::new_atl_transform(vm, a.clone(), None, Value::None, Value::None)
        {
            at_list.push(t);
        }
    }
    let name = str_tuple(vm, &spec.name);
    let tag = spec
        .tag
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    let layer = spec
        .layer
        .as_ref()
        .map(|t| py::s(vm, t))
        .unwrap_or(Value::None);
    let tag_or_name = if py::is_none(&tag) {
        name.clone()
    } else {
        tag.clone()
    };
    let layer = exports(
        vm,
        "default_layer",
        vec![layer, tag_or_name, Value::Bool(spec.expression.is_some())],
        Vec::new(),
    )?;
    if scene {
        let ctx = context(vm)?;
        let images = py::get(vm, &ctx, "images")?;
        vm.call_method(&images, "predict_scene", vec![layer.clone()])?;
    }
    exports(
        vm,
        "predict_show",
        vec![name, layer],
        vec![
            ("what".into(), img),
            ("tag".into(), tag),
            ("at_list".into(), vm.types.new_list(at_list)),
        ],
    )?;
    Ok(())
}

// Namespaces -------------------------------------------------------------------------------------------

fn special_namespaces(vm: &mut Vm) -> PyResult<Value> {
    py::config(vm, "special_namespaces")
}

/// `renpy.ast.create_store(name)`: special namespaces are not stores.
pub fn create_store(vm: &mut Vm, name: &str) -> PyResult<()> {
    let sn = special_namespaces(vm)?;
    let k = py::s(vm, name);
    if ops::contains(vm, &sn, &k)? {
        return Ok(());
    }
    let maybe: Vec<&str> = name.split('.').take(2).collect();
    let maybe = maybe.join(".");
    let mk = py::s(vm, &maybe);
    if ops::contains(vm, &sn, &mk)? {
        let ns = ops::getitem(vm, &sn, mk)?;
        let allow = py::get_or_none(vm, &ns, "allow_child_namespaces")?;
        if !py::truthy(vm, &allow)? {
            return Err(py::error(
                vm,
                format!(
                    "Creating stores within the {} namespace is not supported.",
                    &maybe[6.min(maybe.len())..]
                ),
            ));
        }
    }
    python::create_store(vm, name)
}

fn f_create_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    create_store(vm, &n)?;
    Ok(Value::None)
}

/// `get_namespace(store)` -> (namespace, special).
fn get_namespace(vm: &mut Vm, store: &str) -> PyResult<(Value, bool)> {
    let sn = special_namespaces(vm)?;
    let k = py::s(vm, store);
    if ops::contains(vm, &sn, &k)? {
        return Ok((ops::getitem(vm, &sn, k)?, true));
    }
    let c = mget(vm, "StoreNamespace")?;
    Ok((vm.call(&c, vec![k], Vec::new())?, false))
}

fn f_get_namespace(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    let (ns, special) = get_namespace(vm, &n)?;
    Ok(vm.types.new_tuple(vec![ns, Value::Bool(special)]))
}

fn sn_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "store", args[1].clone())?;
    Ok(Value::None)
}

fn sn_dict(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let s = py::get(vm, this, "store")?;
    let s = py::string(vm, &s)?;
    python::store_dict(vm, &s)
}

fn sn_set(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = sn_dict(vm, &args[0])?;
    ops::setitem(vm, &d, args[1].clone(), args[2].clone())?;
    Ok(Value::None)
}

fn sn_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = sn_dict(vm, &args[0])?;
    ops::getitem(vm, &d, args[1].clone())
}

// Define / default -------------------------------------------------------------------------------------

fn define_set(
    vm: &mut Vm,
    store: &str,
    name: &str,
    index: &Option<PyExpr>,
    operator: &str,
    code: &PyCode,
) -> PyResult<()> {
    let value = eval_code(vm, code)?;
    let (ns, _) = get_namespace(vm, store)?;
    let n = py::s(vm, name);
    if index.is_none() && operator == "=" {
        vm.call_method(&ns, "set", vec![n, value])?;
        return Ok(());
    }
    let base = vm.call_method(&ns, "get", vec![n.clone()])?;
    let mut old = base.clone();
    let mut key = Value::None;
    if let Some(i) = index {
        key = ev(vm, i)?;
        if operator != "=" {
            old = ops::getitem(vm, &base, key.clone())?;
        }
    }
    let new = match operator {
        "+=" => ops::binary_op(
            vm,
            &old,
            &value,
            crysirror_python::ast::BinOp::Add,
            false,
            false,
        )?,
        "|=" => ops::binary_op(
            vm,
            &old,
            &value,
            crysirror_python::ast::BinOp::BitOr,
            false,
            false,
        )?,
        _ => value,
    };
    if index.is_some() {
        ops::setitem(vm, &base, key, new)?;
    } else {
        vm.call_method(&ns, "set", vec![n, new])?;
    }
    Ok(())
}

fn record_definition(vm: &mut Vm, store: &str, name: &str, id: NodeId) -> PyResult<()> {
    let full = if store == "store" {
        name.to_owned()
    } else {
        format!("{}.{}", &store[6.min(store.len())..], name)
    };
    let r = rt(vm);
    let (f, l) = {
        let s = r.script.borrow();
        let n = s.arena.get(id);
        (n.filename.to_string(), n.linenumber)
    };
    if let Ok(dump) = py::import_attr(vm, "renpy.dump", "definitions") {
        let t = vec![py::s(vm, &full), py::s(vm, &f), Value::Int(l as i64)];
        let t = vm.types.new_tuple(t);
        ops::list_append(vm, &dump, t)?;
    }
    Ok(())
}

/// `Default.execute_default(start)`.
pub fn execute_default(vm: &mut Vm, id: NodeId, start: bool) -> PyResult<()> {
    let r = rt(vm);
    let kind = r.script.borrow().arena.get(id).kind.clone();
    let NodeKind::Default { store, name, code } = kind else {
        if let NodeKind::UserStatement { .. } = kind {
            user_call(vm, id, "execute_default", Vec::new())?;
        }
        return Ok(());
    };
    let (ns, special) = get_namespace(vm, &store)?;
    let n = py::s(vm, &name);
    if special {
        let v = eval_code(vm, &code)?;
        vm.call_method(&ns, "set_default", vec![n, v])?;
        return Ok(());
    }
    let d = python::store_dict(vm, &store)?;
    let dk = py::s(vm, "_defaults_set");
    let ebc = py::get(vm, &d, "ever_been_changed")?;
    let defaults_set = match ops::dict_get(vm, d.obj().unwrap(), &dk)? {
        Some(s) if !py::is_none(&s) => s,
        _ => {
            let c = py::import_attr(vm, "renpy.revertable", "RevertableSet")?;
            let s = vm.call(&c, Vec::new(), Vec::new())?;
            ops::setitem(vm, &d, dk.clone(), s.clone())?;
            ops::set_add(vm, &ebc, dk)?;
            s
        }
    };
    let developer = py::config(vm, "developer")?;
    if ops::contains(vm, &defaults_set, &n)? {
        if start && py::truthy(vm, &developer)? {
            return Err(py::error(
                vm,
                format!("{store}.{name} is being given a default a second time."),
            ));
        }
        return Ok(());
    }
    if start && matches!(developer, Value::Bool(true)) {
        let full = format!("{store}.{name}");
        let sds = py::import_attr(vm, "renpy.python", "store_dicts")?;
        let fk = py::s(vm, &full);
        if ops::contains(vm, &sds, &fk)? {
            return Err(py::error(
                vm,
                format!(
                    "{full} is being given a default, but a store with that name already exists."
                ),
            ));
        }
    }
    if start || !ops::set_contains(vm, &ebc, &n)? {
        let v = eval_code(vm, &code)?;
        ops::setitem(vm, &d, n.clone(), v)?;
    }
    ops::set_add(vm, &ebc, n.clone())?;
    vm.call_method(&defaults_set, "add", vec![n])?;
    Ok(())
}

fn default_statements(vm: &mut Vm) -> PyResult<Value> {
    mget(vm, "default_statements")
}

fn f_redefine(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let stores = args[0].clone();
    let ds = mget(vm, "define_statements")?;
    for h in py::items(vm, &ds)? {
        let Some(id) = nodes::id_of(&h) else { continue };
        let r = rt(vm);
        let kind = r.script.borrow().arena.get(id).kind.clone();
        if let NodeKind::Define {
            store,
            name,
            index,
            operator,
            code,
        } = kind
        {
            let sk = py::s(vm, &store);
            if ops::contains(vm, &stores, &sk)? {
                define_set(vm, &store, &name, &index, &operator, &code)?;
            }
        }
    }
    Ok(Value::None)
}

fn f_apply_arguments(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let params = arg_or(args, kw, 0, "parameters", Value::None);
    let a = arg_or(args, kw, 1, "args", Value::None);
    let k = arg_or(args, kw, 2, "kwargs", Value::None);
    let ignore = py::truthy(
        vm,
        &arg_or(args, kw, 3, "ignore_errors", Value::Bool(false)),
    )?;
    let (a, k) = super::params::split(vm, &a, &k)?;
    let sig = py::native::<Rc<crate::ast::Signature>, _>(&params, |s| s.clone());
    if !py::is_none(&params) && sig.is_none() {
        let kd = py::kwargs_dict(vm, &k)?;
        let at = vm.types.new_tuple(a);
        return vm.call_method(&params, "apply", vec![at, kd, Value::Bool(ignore)]);
    }
    let loc: (Rc<str>, u32) = ("<none>".into(), 1);
    super::params::apply_arguments(vm, sig.as_deref(), &loc, &a, &k, ignore)
}

// User statements --------------------------------------------------------------------------------------

/// `UserStatement.parsed`, parsed on first use when the statement was
/// loaded without it (`renpy.statements.parse`).
fn user_parsed(vm: &mut Vm, id: NodeId) -> PyResult<Value> {
    let r = rt(vm);
    let (parsed, line) = match &r.script.borrow().arena.get(id).kind {
        NodeKind::UserStatement { parsed, line, .. } => (parsed.clone(), line.clone()),
        _ => return Ok(Value::None),
    };
    if let Some(p) = parsed.filter(|p| !py::is_none(p)) {
        return Ok(p);
    }
    let h = nodes::handle(vm, id)?;
    let block = py::get(vm, &h, "block")?;
    let p = super::statements::parse(vm, &h, &line, &block)?;
    if let NodeKind::UserStatement { parsed, .. } =
        &mut r.script.borrow_mut().arena.get_mut(id).kind
    {
        *parsed = Some(p.clone());
    }
    Ok(p)
}

/// `Node.scry()` and its per-statement overrides (renpy/ast.py 8.3): what
/// is known about the statement's future (`Scry`).
pub fn scry(vm: &mut Vm, id: NodeId) -> PyResult<Value> {
    let (kind, next) = {
        let r = rt(vm);
        let s = r.script.borrow();
        let n = s.arena.get(id);
        (n.kind.clone(), n.next)
    };
    let cls = py::import_attr(vm, MODULE, "Scry")?;
    let rv = vm.call(&cls, Vec::new(), Vec::new())?;
    let set_next = |vm: &mut Vm, n: Option<NodeId>| -> PyResult<()> {
        let h = nodes::opt_handle(vm, n)?;
        py::set(vm, &rv, "_next", h)
    };
    set_next(vm, next)?;
    match &kind {
        NodeKind::Say { .. } => scry_say(vm, id, &kind, &rv)?,
        NodeKind::TranslateSay {
            identifier, say, ..
        } => {
            let found = super::translation::lookup_translate(vm, identifier, None)?;
            let me = nodes::handle(vm, id)?;
            if py::is_none(&found) || py::same(&found, &me) {
                scry_say(vm, id, say, &rv)?;
            }
        }
        NodeKind::Python { .. } => py::set(vm, &rv, "interacts", Value::Bool(true))?,
        NodeKind::Call { .. }
        | NodeKind::Return { .. }
        | NodeKind::While { .. }
        | NodeKind::If { .. } => set_next(vm, None)?,
        NodeKind::Menu { has_caption, .. } => {
            set_next(vm, None)?;
            py::set(vm, &rv, "interacts", Value::Bool(true))?;
            if *has_caption {
                py::set(vm, &rv, "menu_with_caption", Value::Bool(true))?;
            }
        }
        NodeKind::Jump {
            target, expression, ..
        } => {
            if *expression {
                set_next(vm, None)?;
            } else {
                let t = py::s(vm, target);
                let n = nodes::lookup(vm, &t)?;
                set_next(vm, Some(n))?;
            }
        }
        NodeKind::UserStatement { code_block, .. } => {
            let n = user_next(vm, id, next, code_block)?;
            py::set(vm, &rv, "_next", n)?;
            user_call(vm, id, "scry", vec![rv.clone()])?;
        }
        NodeKind::Translate {
            identifier,
            alternate,
            ..
        } => {
            let n = super::translation::lookup_translate(vm, identifier, alternate.as_deref())?;
            py::set(vm, &rv, "_next", n)?;
        }
        _ => {}
    }
    Ok(rv)
}

fn scry_say(vm: &mut Vm, id: NodeId, kind: &NodeKind, rv: &Value) -> PyResult<()> {
    let NodeKind::Say {
        who,
        what,
        interact,
        ..
    } = kind
    else {
        return Ok(());
    };
    let who = eval_who(vm, who)?;
    py::set(vm, rv, "who", who.clone())?;
    py::set(vm, rv, "say", Value::Bool(true))?;
    let h = nodes::handle(vm, id)?;
    if let Ok(Some(args)) = py::get_opt(vm, &h, "arguments")
        && !py::is_none(&args)
        && let Ok(ev) = py::method(vm, &args, "evaluate", Vec::new())
        && let Ok(kw) = ops::getitem(vm, &ev, Value::Int(1))
        && let Ok(m) = ops::getitem(vm, &kw, py::s(vm, "multiple"))
    {
        py::set(vm, rv, "multiple", m)?;
    }
    if *interact {
        let f = py::import_attr(vm, "renpy.exports", "scry_say")?;
        let w = py::s(vm, what);
        vm.call(&f, vec![who, w, rv.clone()], Vec::new())?;
    } else {
        py::set(vm, rv, "interacts", Value::Bool(false))?;
        let dne = py::import_attr(vm, MODULE, "DoesNotExtend")?;
        py::set(vm, rv, "extend_text", dne)?;
    }
    Ok(())
}

/// `UserStatement.call(method, *args)` through `renpy.statements.call`.
fn user_call(vm: &mut Vm, id: NodeId, method: &str, args: Vec<Value>) -> PyResult<Value> {
    user_call_kw(vm, id, method, args, &[])
}

fn user_call_kw(
    vm: &mut Vm,
    id: NodeId,
    method: &str,
    args: Vec<Value>,
    kw: Kw,
) -> PyResult<Value> {
    let parsed = user_parsed(vm, id)?;
    super::statements::call_method(vm, method, &parsed, args, kw)
}

fn user_get(vm: &mut Vm, id: NodeId, key: &str) -> PyResult<Value> {
    let parsed = user_parsed(vm, id)?;
    let f = py::import_attr(vm, "renpy.statements", "get")?;
    vm.call(&f, vec![py::s(vm, key), parsed], Vec::new())
}

fn user_name(vm: &mut Vm, id: NodeId) -> PyResult<String> {
    let parsed = user_parsed(vm, id)?;
    let f = py::import_attr(vm, "renpy.statements", "get_name")?;
    let n = vm.call(&f, vec![parsed], Vec::new())?;
    py::string(vm, &n)
}

fn user_next(
    vm: &mut Vm,
    id: NodeId,
    next: Option<NodeId>,
    code_block: &Option<Vec<NodeId>>,
) -> PyResult<Value> {
    let rv = match code_block.as_ref().and_then(|b| b.first()) {
        Some(first) => {
            let h = nodes::handle(vm, *first)?;
            let name = py::get_or_none(vm, &h, "name")?;
            user_call(vm, id, "next", vec![name])?
        }
        None => user_call(vm, id, "next", Vec::new())?,
    };
    if !py::is_none(&rv) {
        let n = nodes::lookup(vm, &rv)?;
        return nodes::handle(vm, n);
    }
    nodes::opt_handle(vm, next)
}

/// `UserStatement.execute_init`: runs the init callback and records
/// default execution.
pub fn user_execute_init(vm: &mut Vm, id: NodeId) -> PyResult<i32> {
    user_call(vm, id, "execute_init", Vec::new())?;
    let mut priority = 0;
    let init = user_get(vm, id, "init")?;
    if py::truthy(vm, &init)? {
        priority = 1;
    }
    let ed = user_get(vm, id, "execute_default")?;
    if py::truthy(vm, &ed)? {
        let ds = default_statements(vm)?;
        let h = nodes::handle(vm, id)?;
        ops::list_append(vm, &ds, h)?;
    }
    Ok(priority)
}

// Execution --------------------------------------------------------------------------------------------

/// `Node.execute()` of node `id`.
pub fn execute(vm: &mut Vm, id: NodeId) -> PyResult<()> {
    let r = rt(vm);
    let node = r.script.borrow().arena.get(id).clone();
    let next = node.next;
    match &node.kind {
        NodeKind::Say { .. } | NodeKind::TranslateSay { .. } => {
            execute_say(vm, id, &node.kind, next)
        }
        NodeKind::Init { .. } => {
            next_id(vm, next)?;
            super::execution::not_infinite_loop(vm, 60.);
            statement_name(vm, "init")
        }
        NodeKind::Label {
            name, parameters, ..
        } => {
            next_id(vm, next)?;
            statement_name(vm, "label")?;
            let ctx = context(vm)?;
            vm.call_method(&ctx, "mark_seen", Vec::new())?;
            let a = store_get(vm, "_args")?;
            let k = store_get(vm, "_kwargs")?;
            let (a, k) = super::params::split(vm, &a, &k)?;
            let loc = (node.filename.clone(), node.linenumber);
            let values =
                super::params::apply_arguments(vm, parameters.as_ref(), &loc, &a, &k, false)?;
            let kw = py::str_items(vm, &values)?;
            exports(vm, "dynamic", Vec::new(), kw)?;
            store_set(vm, "_args", Value::None)?;
            store_set(vm, "_kwargs", Value::None)?;
            let la = py::get_or_none(vm, &ctx, "last_abnormal")?;
            let ln = py::s(vm, name);
            for cfg in ["label_callback", "label_callbacks"] {
                let cb = py::config(vm, cfg)?;
                super::easy::run_callbacks(vm, &cb, vec![ln.clone(), la.clone()], Vec::new())?;
            }
            Ok(())
        }
        NodeKind::Python { code, hide, store } => {
            next_id(vm, next)?;
            statement_name(vm, "python")?;
            let r = exec_code(vm, code, *hide, store);
            let ctx = context(vm)?;
            let ip = py::get_or_none(vm, &ctx, "init_phase")?;
            if !py::truthy(vm, &ip)? {
                let cbs = py::config(vm, "python_callbacks")?;
                for cb in py::items(vm, &cbs)? {
                    vm.call(&cb, Vec::new(), Vec::new())?;
                }
            }
            r
        }
        NodeKind::EarlyPython { .. } => {
            next_id(vm, next)?;
            super::execution::not_infinite_loop(vm, 60.);
            statement_name(vm, "python early")
        }
        NodeKind::Image { name, code, atl } => {
            next_id(vm, next)?;
            statement_name(vm, "image")?;
            let img = match code {
                Some(c) => eval_code(vm, c)?,
                None => {
                    let raw = atl
                        .clone()
                        .ok_or_else(|| py::error(vm, "image statement without a value"))?;
                    super::atlrt::new_atl_transform(vm, raw, None, Value::None, Value::None)?
                }
            };
            let n = str_tuple(vm, name);
            exports(vm, "image", vec![n, img], Vec::new())?;
            Ok(())
        }
        NodeKind::Transform {
            store,
            name,
            atl,
            parameters,
        } => {
            next_id(vm, next)?;
            statement_name(vm, "transform")?;
            let sig = parameters.clone().unwrap_or_default();
            let t = super::atlrt::new_atl_transform(
                vm,
                atl.clone(),
                Some(sig),
                Value::None,
                Value::None,
            )?;
            if let Ok(dump) = py::import_attr(vm, "renpy.dump", "transforms") {
                let tt = vec![
                    py::s(vm, name),
                    py::s(vm, &node.filename),
                    Value::Int(node.linenumber as i64),
                ];
                let tt = vm.types.new_tuple(tt);
                ops::list_append(vm, &dump, tt)?;
            }
            let n = py::s(vm, name);
            exports(vm, "pure", vec![n.clone()], Vec::new())?;
            let (ns, _) = get_namespace(vm, store)?;
            vm.call_method(&ns, "set", vec![n, t])?;
            Ok(())
        }
        NodeKind::Show { imspec, atl } => {
            next_id(vm, next)?;
            statement_name(vm, "show")?;
            show_imspec(vm, imspec, atl)
        }
        NodeKind::ShowLayer {
            layer,
            at_list,
            atl,
        }
        | NodeKind::Camera {
            layer,
            at_list,
            atl,
        } => {
            let camera = matches!(node.kind, NodeKind::Camera { .. });
            next_id(vm, next)?;
            statement_name(vm, "show layer")?;
            let mut at = Vec::new();
            for e in at_list {
                at.push(ev(vm, e)?);
            }
            if let Some(a) = atl {
                at.push(super::atlrt::new_atl_transform(
                    vm,
                    a.clone(),
                    None,
                    Value::None,
                    Value::None,
                )?);
            }
            let mut kw: Vec<(Rc<str>, Value)> = vec![("layer".into(), py::s(vm, layer))];
            if camera {
                kw.push(("camera".into(), Value::Bool(true)));
            }
            exports(vm, "layer_at_list", vec![vm.types.new_list(at)], kw)?;
            Ok(())
        }
        NodeKind::Scene { imspec, layer, atl } => {
            next_id(vm, next)?;
            statement_name(vm, "scene")?;
            let scene = py::config(vm, "scene")?;
            let l = layer.as_ref().map(|l| py::s(vm, l)).unwrap_or(Value::None);
            vm.call(&scene, vec![l], Vec::new())?;
            if let Some(spec) = imspec {
                show_imspec(vm, spec, atl)?;
            }
            Ok(())
        }
        NodeKind::Hide { imspec } => {
            next_id(vm, next)?;
            statement_name(vm, "hide")?;
            let name = str_tuple(vm, &imspec.name);
            let tag = match &imspec.tag {
                Some(t) => py::s(vm, t),
                None => name,
            };
            let layer = imspec
                .layer
                .as_ref()
                .map(|l| py::s(vm, l))
                .unwrap_or(Value::None);
            let layer = exports(vm, "default_layer", vec![layer, tag.clone()], Vec::new())?;
            let hide = py::config(vm, "hide")?;
            vm.call(&hide, vec![tag, layer], Vec::new())?;
            Ok(())
        }
        NodeKind::With { expr, paired } => {
            next_id(vm, next)?;
            statement_name(vm, "with")?;
            let trans = match expr {
                Some(e) => ev(vm, e)?,
                None => Value::None,
            };
            let paired = match paired {
                Some(p) => ev(vm, p)?,
                None => Value::None,
            };
            exports(
                vm,
                "with_statement",
                vec![trans],
                vec![("paired".into(), paired)],
            )?;
            Ok(())
        }
        NodeKind::Call {
            label,
            expression,
            arguments,
            global_label,
        } => {
            statement_name(vm, "call")?;
            let mut target = py::s(vm, label);
            if *expression {
                target = ev_text(vm, label, &node.filename, node.linenumber)?;
                if let Some(t) = target.as_str().filter(|t| t.starts_with('.')) {
                    let full = format!("{global_label}{t}");
                    target = py::s(vm, &full);
                }
            }
            let nh = nodes::opt_handle(vm, next)?;
            let site = py::get_or_none(vm, &nh, "name")?;
            let ctx = context(vm)?;
            let rv = super::execution::call(vm, &ctx, &target, site)?;
            next_node(vm, rv)?;
            py::set(vm, &ctx, "abnormal", Value::Bool(true))?;
            match arguments {
                Some(a) => {
                    let loc = (node.filename.clone(), node.linenumber);
                    let (a, k) = super::params::evaluate(vm, a, &loc, &Value::None)?;
                    store_set(vm, "_args", a)?;
                    store_set(vm, "_kwargs", k)?;
                }
                None => {
                    store_set(vm, "_args", Value::None)?;
                    store_set(vm, "_kwargs", Value::None)?;
                }
            }
            Ok(())
        }
        NodeKind::Return { expression } => {
            statement_name(vm, "return")?;
            let v = match expression {
                Some(e) => ev(vm, e)?,
                None => Value::None,
            };
            store_set(vm, "_return", v)?;
            let ctx = context(vm)?;
            let ip = py::get_or_none(vm, &ctx, "init_phase")?;
            if py::truthy(vm, &ip)? {
                let rs = py::get(vm, &ctx, "return_stack")?;
                if ops::len(vm, &rs)? == 0 {
                    if py::config_bool(vm, "developer")? {
                        return Err(py::error(vm, "Unexpected return during the init phase."));
                    }
                    return Ok(());
                }
            }
            let n = py::method_kw(
                vm,
                &ctx,
                "lookup_return",
                Vec::new(),
                vec![("pop".into(), Value::Bool(true))],
            )?;
            next_node(vm, n)?;
            vm.call_method(&ctx, "pop_dynamic", Vec::new())?;
            Ok(())
        }
        NodeKind::Menu { .. } => {
            execute_menu(vm, &node.kind, next, (&node.filename, node.linenumber))
        }
        NodeKind::Jump {
            target,
            expression,
            global_label,
        } => {
            statement_name(vm, "jump")?;
            let mut t = py::s(vm, target);
            if *expression {
                t = ev_text(vm, target, &node.filename, node.linenumber)?;
                if let Some(s) = t.as_str().filter(|s| s.starts_with('.')) {
                    let full = format!("{global_label}{s}");
                    t = py::s(vm, &full);
                }
            }
            let rv = nodes::lookup(vm, &t)?;
            let ctx = context(vm)?;
            py::set(vm, &ctx, "abnormal", Value::Bool(true))?;
            next_id(vm, Some(rv))
        }
        NodeKind::Pass => {
            next_id(vm, next)?;
            statement_name(vm, "pass")
        }
        NodeKind::While { condition, block } => {
            next_id(vm, next)?;
            statement_name(vm, "while")?;
            let c = ev(vm, condition)?;
            if py::truthy(vm, &c)? {
                next_id(vm, block.first().copied())?;
            }
            Ok(())
        }
        NodeKind::If { entries } => {
            next_id(vm, next)?;
            statement_name(vm, "if")?;
            for (cond, block) in entries {
                let c = ev(vm, cond)?;
                if py::truthy(vm, &c)? {
                    next_id(vm, block.first().copied())?;
                    return Ok(());
                }
            }
            Ok(())
        }
        NodeKind::UserStatement {
            code_block, atl, ..
        } => {
            let n = user_next(vm, id, next, code_block)?;
            next_node(vm, n)?;
            let name = user_name(vm, id)?;
            statement_name(vm, &name)?;
            match atl {
                Some(a) => {
                    let t = super::atlrt::new_atl_transform(
                        vm,
                        a.clone(),
                        None,
                        Value::None,
                        Value::None,
                    )?;
                    user_call_kw(vm, id, "execute", Vec::new(), &[("atl".into(), t)])?;
                }
                None => {
                    user_call(vm, id, "execute", Vec::new())?;
                }
            }
            Ok(())
        }
        NodeKind::PostUserStatement { parent } => {
            next_id(vm, next)?;
            let name = user_name(vm, *parent)?;
            statement_name(vm, &format!("post {name}"))?;
            user_call(vm, *parent, "post_execute", Vec::new())?;
            Ok(())
        }
        NodeKind::Define {
            store,
            name,
            index,
            operator,
            code,
        } => {
            next_id(vm, next)?;
            statement_name(vm, "define")?;
            let ds = mget(vm, "define_statements")?;
            let h = nodes::handle(vm, id)?;
            ops::list_append(vm, &ds, h)?;
            record_definition(vm, store, name, id)?;
            if operator == "=" && index.is_none() {
                let (ns, _) = get_namespace(vm, store)?;
                let pure = py::get_opt(vm, &ns, "pure")?.unwrap_or(Value::Bool(true));
                if py::truthy(vm, &pure)? {
                    let full = py::s(vm, &format!("{store}.{name}"));
                    exports(vm, "pure", vec![full], Vec::new())?;
                }
            }
            define_set(vm, store, name, index, operator, code)
        }
        NodeKind::Default { store, name, code } => {
            next_id(vm, next)?;
            statement_name(vm, "default")?;
            let (ns, special) = get_namespace(vm, store)?;
            if special {
                let v = eval_code(vm, code)?;
                let n = py::s(vm, name);
                vm.call_method(&ns, "set_default", vec![n, v])?;
                let rep =
                    py::get_opt(vm, &ns, "repeat_at_default_time")?.unwrap_or(Value::Bool(false));
                if py::truthy(vm, &rep)? {
                    let ds = default_statements(vm)?;
                    let h = nodes::handle(vm, id)?;
                    ops::list_append(vm, &ds, h)?;
                }
                return Ok(());
            }
            let ds = default_statements(vm)?;
            let h = nodes::handle(vm, id)?;
            ops::list_append(vm, &ds, h)?;
            record_definition(vm, store, name, id)
        }
        NodeKind::Screen { screen } => {
            next_id(vm, next)?;
            statement_name(vm, "screen")?;
            let sv = super::slast::screen_value(vm, screen.clone())?;
            let loc = vm.types.new_tuple(vec![
                py::s(vm, &node.filename),
                Value::Int(node.linenumber as i64),
            ]);
            vm.call_method(&sv, "define", vec![loc])?;
            if let Ok(dump) = py::import_attr(vm, "renpy.dump", "screens") {
                let t = vec![
                    py::s(vm, &screen.name),
                    py::s(vm, &node.filename),
                    Value::Int(node.linenumber as i64),
                ];
                let t = vm.types.new_tuple(t);
                ops::list_append(vm, &dump, t)?;
            }
            Ok(())
        }
        NodeKind::Style(style) => {
            next_id(vm, next)?;
            statement_name(vm, "style")?;
            let ctx = context(vm)?;
            let ip = py::get_or_none(vm, &ctx, "init_phase")?;
            if py::config_bool(vm, "defer_styles")? && py::truthy(vm, &ip)? {
                let ds = py::import_attr(vm, "renpy.translation", "deferred_styles")?;
                let h = nodes::handle(vm, id)?;
                ops::list_append(vm, &ds, h)?;
                return Ok(());
            }
            apply_style_node(vm, style)
        }
        NodeKind::Testcase { name } => {
            next_id(vm, next)?;
            statement_name(vm, "testcase")?;
            let tc = py::import_attr(vm, "renpy.test.testexecution", "testcases")?;
            let n = py::s(vm, name);
            let h = nodes::handle(vm, id)?;
            ops::setitem(vm, &tc, n, h)?;
            Ok(())
        }
        NodeKind::Rpy { .. } => {
            next_id(vm, next)?;
            statement_name(vm, "rpy")
        }
        NodeKind::Translate {
            identifier,
            language,
            alternate,
            ..
        } => {
            if language.is_some() {
                next_id(vm, next)?;
                return Err(py::error(vm, "Translation nodes cannot be run directly."));
            }
            seen_translate(vm, identifier)?;
            let target = lookup_translate(vm, identifier, alternate)?;
            next_node(vm, target)?;
            let ctx = context(vm)?;
            py::set(vm, &ctx, "translate_identifier", py::s(vm, identifier))?;
            let alt = alternate
                .as_ref()
                .map(|a| py::s(vm, a))
                .unwrap_or(Value::None);
            py::set(vm, &ctx, "alternate_translate_identifier", alt)?;
            Ok(())
        }
        NodeKind::EndTranslate => {
            next_id(vm, next)?;
            let ctx = context(vm)?;
            py::set(vm, &ctx, "translate_identifier", Value::None)?;
            py::set(vm, &ctx, "alternate_translate_identifier", Value::None)?;
            Ok(())
        }
        NodeKind::TranslateString { language, old, new } => {
            next_id(vm, next)?;
            let lang = language
                .as_ref()
                .map(|l| py::s(vm, l))
                .unwrap_or(Value::None);
            let newloc = vm.types.new_tuple(vec![
                py::s(vm, &node.filename),
                Value::Int(node.linenumber as i64 + 1),
            ]);
            let f = py::import_attr(vm, "renpy.translation", "add_string_translation")?;
            vm.call(
                &f,
                vec![lang, py::s(vm, old), py::s(vm, new), newloc],
                Vec::new(),
            )?;
            Ok(())
        }
        NodeKind::TranslatePython { .. }
        | NodeKind::TranslateBlock { .. }
        | NodeKind::TranslateEarlyBlock { .. } => next_id(vm, next),
    }
}

fn seen_translate(vm: &mut Vm, identifier: &str) -> PyResult<()> {
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    if py::is_none(&p) {
        return Ok(());
    }
    let st = py::get_or_none(vm, &p, "_seen_translates")?;
    if py::is_none(&st) {
        return Ok(());
    }
    let k = py::s(vm, identifier);
    if !ops::contains(vm, &st, &k)? {
        vm.call_method(&st, "add", vec![k])?;
        let g = py::import(vm, "renpy.game")?;
        for n in ["seen_translates_count", "new_translates_count"] {
            let v = py::get_or_none(vm, &g, n)?;
            let v = py::int(vm, &v).unwrap_or(0) + 1;
            py::set(vm, &g, n, Value::Int(v))?;
        }
    }
    Ok(())
}

fn lookup_translate(vm: &mut Vm, identifier: &str, alternate: &Option<String>) -> PyResult<Value> {
    super::translation::lookup_translate(vm, identifier, alternate.as_deref())
}

fn execute_say(vm: &mut Vm, id: NodeId, kind: &NodeKind, next: Option<NodeId>) -> PyResult<()> {
    let (say, ident) = match kind {
        NodeKind::TranslateSay {
            identifier,
            language,
            say,
        } => (&**say, Some((identifier.clone(), language.clone()))),
        k => (k, None),
    };
    let NodeKind::Say {
        who,
        what,
        with_,
        interact,
        attributes,
        temporary_attributes,
        arguments,
        ..
    } = say
    else {
        return Err(py::error(vm, "malformed say node"));
    };
    next_id(vm, next)?;
    let ctx = context(vm)?;
    if let Some((identifier, language)) = &ident {
        py::set(vm, &ctx, "translate_identifier", py::s(vm, identifier))?;
        py::set(vm, &ctx, "alternate_translate_identifier", Value::None)?;
        if language.is_none() {
            seen_translate(vm, identifier)?;
            let node = lookup_translate(vm, identifier, &None)?;
            if let Some(nid) = nodes::id_of(&node) {
                if nid != id {
                    next_node(vm, node)?;
                    return Ok(());
                }
            }
        }
    }
    let attrs = attributes
        .as_ref()
        .map(|a| str_tuple(vm, a))
        .unwrap_or(Value::None);
    let tattrs = temporary_attributes
        .as_ref()
        .map(|a| str_tuple(vm, a))
        .unwrap_or(Value::None);
    let r = (|| -> PyResult<()> {
        py::set(vm, &ctx, "say_attributes", attrs)?;
        py::set(vm, &ctx, "temporary_attributes", tattrs)?;
        let whov = eval_who(vm, who)?;
        if !py::is_none(&whov) {
            let sn = py::get_opt(vm, &whov, "statement_name")?.unwrap_or_else(|| py::s(vm, "say"));
            let sn = if crysirror_python::builtins::is_callable(vm, &sn) {
                vm.call(&sn, Vec::new(), Vec::new())?
            } else {
                sn
            };
            let sn = py::string(vm, &sn)?;
            statement_name(vm, &sn)?;
        } else {
            statement_name(vm, "say")?;
        }
        if !py::is_none(&whov)
            && !crysirror_python::builtins::is_callable(vm, &whov)
            && whov.as_str().is_none()
        {
            let w = who.as_ref().map(|w| w.text.to_string()).unwrap_or_default();
            return Err(py::error(
                vm,
                format!("Sayer {w} is not a function or string."),
            ));
        }
        let mut whatv = py::s(vm, what);
        let filter = py::config(vm, "say_menu_text_filter")?;
        if !py::is_none(&filter) {
            whatv = vm.call(&filter, vec![whatv], Vec::new())?;
        }
        store_set(vm, "_last_raw_what", whatv.clone())?;
        let (args, kwargs) = match arguments {
            Some(a) => {
                let r = rt(vm);
                let loc = {
                    let s = r.script.borrow();
                    let n = s.arena.get(id);
                    (n.filename.clone(), n.linenumber)
                };
                super::params::evaluate(vm, a, &loc, &Value::None)?
            }
            None => (vm.types.new_tuple(Vec::new()), vm.types.new_dict()),
        };
        vm.call_method(
            &kwargs,
            "setdefault",
            vec![py::s(vm, "interact"), Value::Bool(*interact)],
        )?;
        let record = py::get_opt(vm, &whov, "record_say")?.unwrap_or(Value::Bool(true));
        if py::truthy(vm, &record)? {
            let wt = who
                .as_ref()
                .map(|w| py::s(vm, &w.text))
                .unwrap_or(Value::None);
            store_set(vm, "_last_say_who", wt)?;
            store_set(vm, "_last_say_what", whatv.clone())?;
            store_set(vm, "_last_say_args", args.clone())?;
            store_set(vm, "_last_say_kwargs", kwargs.clone())?;
        }
        let iface = py::import_attr(vm, "renpy.game", "interface")?;
        let st = py::get(vm, &iface, "set_transition")?;
        say_menu_with(vm, with_, &st)?;
        let mut a = vec![whov, whatv];
        a.extend(py::items(vm, &args)?);
        let k = py::str_items(vm, &kwargs)?;
        exports(vm, "say", a, k)?;
        Ok(())
    })();
    let ctx = context(vm)?;
    py::set(vm, &ctx, "say_attributes", Value::None)?;
    py::set(vm, &ctx, "temporary_attributes", Value::None)?;
    store_set(vm, "_last_raw_what", py::s(vm, ""))?;
    r?;
    if ident.is_some() {
        py::set(vm, &ctx, "translate_identifier", Value::None)?;
        py::set(vm, &ctx, "alternate_translate_identifier", Value::None)?;
    }
    Ok(())
}

fn execute_menu(
    vm: &mut Vm,
    kind: &NodeKind,
    next: Option<NodeId>,
    loc: (&Rc<str>, u32),
) -> PyResult<()> {
    let NodeKind::Menu {
        items,
        set,
        with_,
        has_caption,
        arguments,
    } = kind
    else {
        unreachable!()
    };
    next_id(vm, next)?;
    let loc = (loc.0.clone(), loc.1);
    let (args, kwargs) = match arguments {
        Some(a) => {
            let (a, k) = super::params::evaluate(vm, a, &loc, &Value::None)?;
            (a, k)
        }
        None => (Value::None, Value::None),
    };
    let mut name = "menu".to_owned();
    if !py::is_none(&kwargs) {
        let nvl = vm.call_method(&kwargs, "get", vec![py::s(vm, "nvl")])?;
        if matches!(nvl, Value::Bool(true)) {
            name = "menu-nvl".into();
        }
    }
    if *has_caption || py::config_bool(vm, "choice_empty_window").unwrap_or(false) {
        name.push_str("-with-caption");
    }
    statement_name(vm, &name)?;
    let mut choices = Vec::new();
    let mut narration = Vec::new();
    let mut item_arguments = Vec::new();
    let filter = py::config(vm, "say_menu_text_filter")?;
    let narrator_menu = py::config_bool(vm, "narrator_menu")?;
    for (i, it) in items.iter().enumerate() {
        let mut label = py::s(vm, &it.label);
        if !py::is_none(&filter) {
            label = vm.call(&filter, vec![label], Vec::new())?;
        }
        let cond = py::s(vm, &it.condition.text);
        let has_item = match &it.block {
            None => {
                if narrator_menu && py::truthy(vm, &label)? {
                    narration.push(label);
                    false
                } else {
                    choices.push(vm.types.new_tuple(vec![label, cond, Value::None]));
                    true
                }
            }
            Some(b) => {
                choices.push(vm.types.new_tuple(vec![label, cond, Value::Int(i as i64)]));
                next_id(vm, b.first().copied())?;
                true
            }
        };
        if has_item {
            let ia = match &it.arguments {
                Some(a) => {
                    let (a, k) = super::params::evaluate(vm, a, &loc, &Value::None)?;
                    vm.types.new_tuple(vec![a, k])
                }
                None => {
                    let t = vm.types.new_tuple(Vec::new());
                    let d = vm.types.new_dict();
                    vm.types.new_tuple(vec![t, d])
                }
            };
            item_arguments.push(ia);
        }
    }
    if !narration.is_empty() {
        let parts: Vec<String> = narration
            .iter()
            .map(|n| py::string(vm, n))
            .collect::<PyResult<_>>()?;
        exports(
            vm,
            "say",
            vec![Value::None, py::s(vm, &parts.join("\n"))],
            vec![("interact".into(), Value::Bool(false))],
        )?;
    }
    let iface = py::import_attr(vm, "renpy.game", "interface")?;
    let st = py::get(vm, &iface, "set_transition")?;
    say_menu_with(vm, with_, &st)?;
    let setv = match set {
        Some(e) => ev(vm, e)?,
        None => Value::None,
    };
    let choice = exports(
        vm,
        "menu",
        vec![
            vm.types.new_list(choices),
            setv,
            args,
            kwargs,
            vm.types.new_list(item_arguments),
        ],
        Vec::new(),
    )?;
    if py::is_none(&choice) {
        return next_id(vm, next);
    }
    let i = py::int(vm, &choice)? as usize;
    let first = items
        .get(i)
        .and_then(|it| it.block.as_ref())
        .and_then(|b| b.first().copied());
    next_id(vm, first)
}

pub fn apply_style_node(vm: &mut Vm, style: &crate::ast::Style) -> PyResult<()> {
    if let Some(v) = &style.variant {
        let variant = ev(vm, v)?;
        let ok = exports(vm, "variant", vec![variant], Vec::new())?;
        if !py::truthy(vm, &ok)? {
            return Ok(());
        }
    }
    let gocs = py::import_attr(vm, "renpy.style", "get_or_create_style")?;
    let s = vm.call(&gocs, vec![py::s(vm, &style.name)], Vec::new())?;
    if style.clear {
        vm.call_method(&s, "clear", Vec::new())?;
    }
    if let Some(p) = &style.parent {
        vm.call_method(&s, "set_parent", vec![py::s(vm, p)])?;
    }
    if let Some(t) = &style.take {
        vm.call_method(&s, "take", vec![py::s(vm, t)])?;
    }
    for d in &style.delattr {
        vm.call_method(&s, "delattr", vec![py::s(vm, d)])?;
    }
    if !style.properties.is_empty() {
        let props = vm.types.new_dict();
        for (name, e) in &style.properties {
            let v = ev(vm, e)?;
            if name == "properties" {
                vm.call_method(&props, "update", vec![v])?;
            } else {
                py::dict_set(vm, &props, name, v)?;
            }
        }
        vm.call_method(&s, "add_properties", vec![props])?;
    }
    Ok(())
}

/// `Node.early_execute()`, run as the script loads.
pub fn early_execute(vm: &mut Vm, id: NodeId) -> PyResult<()> {
    let r = rt(vm);
    let kind = r.script.borrow().arena.get(id).kind.clone();
    match kind {
        NodeKind::EarlyPython { code, hide, store } => {
            python::create_store(vm, &store)?;
            exec_code(vm, &code, hide, &store)
        }
        NodeKind::Define {
            store,
            name,
            index,
            operator,
            code,
        } => {
            create_store(vm, &store)?;
            if operator != "=" || index.is_some() {
                return Ok(());
            }
            if store == "store.config" && EARLY_CONFIG.contains(&name.as_str()) {
                let v = eval_code(vm, &code)?;
                let cfg = py::import(vm, "renpy.config")?;
                py::set(vm, &cfg, &name, v)?;
            }
            Ok(())
        }
        NodeKind::Default { store, .. }
        | NodeKind::Python { store, .. }
        | NodeKind::Transform { store, .. } => create_store(vm, &store),
        _ => Ok(()),
    }
}

// Prediction -------------------------------------------------------------------------------------------

/// `Node.predict()`: the nodes that can run next (None entries allowed).
pub fn predict(vm: &mut Vm, id: NodeId) -> PyResult<Vec<Option<NodeId>>> {
    let r = rt(vm);
    let node = r.script.borrow().arena.get(id).clone();
    let next = node.next;
    Ok(match &node.kind {
        NodeKind::Say {
            who,
            what,
            with_,
            attributes,
            temporary_attributes,
            ..
        } => {
            predict_say(vm, who, what, with_, attributes, temporary_attributes)?;
            vec![next]
        }
        NodeKind::Show { imspec, atl } => {
            predict_imspec(vm, imspec, false, atl)?;
            vec![next]
        }
        NodeKind::Scene { imspec, atl, .. } => {
            if let Some(s) = imspec {
                predict_imspec(vm, s, true, atl)?;
            }
            vec![next]
        }
        NodeKind::Hide { imspec } => {
            let tag = imspec
                .tag
                .clone()
                .unwrap_or_else(|| imspec.name.first().cloned().unwrap_or_default());
            let t = py::s(vm, &tag);
            let layer = imspec
                .layer
                .as_ref()
                .map(|l| py::s(vm, l))
                .unwrap_or(Value::None);
            let layer = exports(vm, "default_layer", vec![layer, t.clone()], Vec::new())?;
            let ctx = context(vm)?;
            let images = py::get(vm, &ctx, "images")?;
            let tt = vm.types.new_tuple(vec![t]);
            vm.call_method(&images, "predict_hide", vec![layer, tt])?;
            vec![next]
        }
        NodeKind::With { expr: Some(e), .. } => {
            if let Ok(t) = ev(vm, e) {
                if py::truthy(vm, &t).unwrap_or(false) {
                    if let Ok(d) = vm.call(
                        &t,
                        Vec::new(),
                        vec![
                            ("old_widget".into(), Value::None),
                            ("new_widget".into(), Value::None),
                        ],
                    ) {
                        let _ = super::predict::displayable(vm, &d);
                    }
                }
            }
            vec![next]
        }
        NodeKind::Call {
            label,
            expression,
            global_label,
            ..
        }
        | NodeKind::Jump {
            target: label,
            expression,
            global_label,
        } => {
            let is_call = matches!(node.kind, NodeKind::Call { .. });
            let mut target = py::s(vm, label);
            if *expression {
                if label.contains('(') {
                    return Ok(Vec::new());
                }
                target = match ev_text(vm, label, &node.filename, node.linenumber) {
                    Ok(v) => v,
                    Err(_) => return Ok(Vec::new()),
                };
                if let Some(t) = target.as_str().filter(|t| t.starts_with('.')) {
                    let full = format!("{global_label}{t}");
                    target = py::s(vm, &full);
                }
                if nodes::find(vm, &target)?.is_none() {
                    return Ok(Vec::new());
                }
            }
            if is_call {
                let nh = nodes::opt_handle(vm, next)?;
                let site = py::get_or_none(vm, &nh, "name")?;
                let ctx = context(vm)?;
                let n = vm.call_method(&ctx, "predict_call", vec![target, site])?;
                vec![nodes::id_of(&n)]
            } else {
                vec![nodes::find(vm, &target)?]
            }
        }
        NodeKind::Return { .. } => {
            let ctx = context(vm)?;
            let n = vm.call_method(&ctx, "predict_return", Vec::new())?;
            vec![nodes::id_of(&n)]
        }
        NodeKind::Menu { items, with_, .. } => {
            let cb = py::builtin(vm, "predict_with", predict_with);
            say_menu_with(vm, with_, &cb)?;
            let pm = py::import_attr(vm, "store", "predict_menu")?;
            vm.call(&pm, Vec::new(), Vec::new())?;
            items
                .iter()
                .filter_map(|i| i.block.as_ref().and_then(|b| b.first().copied()))
                .map(Some)
                .collect()
        }
        NodeKind::While { block, .. } => vec![block.first().copied(), next],
        NodeKind::If { entries } => {
            let mut v: Vec<Option<NodeId>> =
                entries.iter().map(|(_, b)| b.first().copied()).collect();
            v.push(next);
            v
        }
        NodeKind::Translate {
            identifier,
            alternate,
            ..
        } => {
            let n = lookup_translate(vm, identifier, alternate)?;
            vec![nodes::id_of(&n)]
        }
        NodeKind::UserStatement { atl, subparses, .. } => {
            let preds = user_call(vm, id, "predict", Vec::new())?;
            if !py::is_none(&preds) {
                let ep = py::import_attr(vm, "renpy.easy", "predict")?;
                for i in ops::collect(vm, &preds)? {
                    vm.call(&ep, vec![i], Vec::new())?;
                }
            }
            if let Some(a) = atl {
                let t =
                    super::atlrt::new_atl_transform(vm, a.clone(), None, Value::None, Value::None)?;
                let pd = py::import_attr(vm, "renpy.display.predict", "displayable")?;
                vm.call(&pd, vec![t], Vec::new())?;
            }
            let pa = user_get(vm, id, "predict_all")?;
            if py::truthy(vm, &pa)? {
                let mut v: Vec<Option<NodeId>> =
                    subparses.iter().map(|b| b.first().copied()).collect();
                v.push(next);
                return Ok(v);
            }
            let nh = nodes::opt_handle(vm, next)?;
            let nl = py::get_or_none(vm, &nh, "name")?;
            let nl = user_call(vm, id, "predict_next", vec![nl])?;
            if !py::is_none(&nl) {
                let mut v = Vec::new();
                for i in ops::collect(vm, &nl)? {
                    if py::is_none(&i) {
                        continue;
                    }
                    if let Some(n) = nodes::find(vm, &i)? {
                        v.push(Some(n));
                    }
                }
                return Ok(v);
            }
            vec![next]
        }
        _ => vec![next],
    })
}

fn predict_with(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = vm.call(
        &args[0],
        Vec::new(),
        vec![
            ("old_widget".into(), Value::None),
            ("new_widget".into(), Value::None),
        ],
    )?;
    super::predict::displayable(vm, &d)?;
    Ok(Value::None)
}

fn predict_say(
    vm: &mut Vm,
    who: &Option<PyExpr>,
    what: &str,
    with_: &Option<PyExpr>,
    attributes: &Option<Vec<String>>,
    temporary: &Option<Vec<String>>,
) -> PyResult<()> {
    let ctx = context(vm)?;
    let old_a = py::get_or_none(vm, &ctx, "say_attributes")?;
    let old_t = py::get_or_none(vm, &ctx, "temporary_attributes")?;
    let a = attributes
        .as_ref()
        .map(|a| str_tuple(vm, a))
        .unwrap_or(Value::None);
    let t = temporary
        .as_ref()
        .map(|a| str_tuple(vm, a))
        .unwrap_or(Value::None);
    py::set(vm, &ctx, "say_attributes", a)?;
    py::set(vm, &ctx, "temporary_attributes", t)?;
    let r = (|| -> PyResult<()> {
        let whov = eval_who(vm, who)?;
        let cb = py::builtin(vm, "predict_with", predict_with);
        say_menu_with(vm, with_, &cb)?;
        let mut whatv = py::s(vm, what);
        let filter = py::config(vm, "say_menu_text_filter")?;
        if !py::is_none(&filter) {
            whatv = vm.call(&filter, vec![whatv], Vec::new())?;
        }
        exports(vm, "predict_say", vec![whov, whatv], Vec::new())?;
        Ok(())
    })();
    py::set(vm, &ctx, "say_attributes", old_a)?;
    py::set(vm, &ctx, "temporary_attributes", old_t)?;
    r
}

// PyExpr -----------------------------------------------------------------------------------------------

fn pe_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cls = args[0].clone();
    let s = arg_or(&args[1..], kw, 0, "s", py::s(vm, ""));
    let str_t = Value::Obj(vm.types.str_.clone());
    let new = py::get(vm, &str_t, "__new__")?;
    let this = vm.call(&new, vec![cls, s], Vec::new())?;
    py::set(
        vm,
        &this,
        "filename",
        arg_or(&args[1..], kw, 1, "filename", Value::None),
    )?;
    py::set(
        vm,
        &this,
        "linenumber",
        arg_or(&args[1..], kw, 2, "linenumber", Value::Int(0)),
    )?;
    py::set(
        vm,
        &this,
        "py",
        arg_or(&args[1..], kw, 3, "py", Value::Int(3)),
    )?;
    Ok(this)
}

fn pe_getnewargs(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::string(vm, this)?;
    let f = py::get_or_none(vm, this, "filename")?;
    let l = py::get_or_none(vm, this, "linenumber")?;
    let p = py::get_or_none(vm, this, "py")?;
    Ok(vm.types.new_tuple(vec![py::s(vm, &s), f, l, p]))
}

fn pe_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn f_probably_side_effect_free(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(Value::Bool(!s.contains('(')))
}

fn f_chain_block(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

// Python-callable imspec helpers ---------------------------------------------------------------

fn opt_text(vm: &mut Vm, v: &Value) -> PyResult<Option<String>> {
    if py::is_none(v) { Ok(None) } else { py::string(vm, v).map(Some) }
}

fn pyexpr_of(vm: &mut Vm, v: &Value) -> PyResult<PyExpr> {
    let text = py::string(vm, v)?;
    let filename = py::get_opt(vm, v, "filename")?.filter(|f| !py::is_none(f));
    let filename = match filename {
        Some(f) => py::string(vm, &f)?,
        None => "<imspec>".to_owned(),
    };
    let line = match py::get_opt(vm, v, "linenumber")? {
        Some(l) if !py::is_none(&l) => py::int(vm, &l)? as u32,
        _ => 1,
    };
    Ok(PyExpr::new(text, filename.into(), line))
}

/// An upstream imspec tuple: `(name, at_list, layer)` or the 6/7-tuple
/// `(name, expression, tag, at_list, layer, zorder[, behind])`.
fn imspec_from(vm: &mut Vm, v: &Value) -> PyResult<ImSpec> {
    let t = py::items(vm, v)?;
    let names = |vm: &mut Vm, v: &Value| -> PyResult<Vec<String>> {
        if let Some(s) = v.as_str() {
            return Ok(s.split_whitespace().map(str::to_owned).collect());
        }
        py::items(vm, v)?.iter().map(|x| py::string(vm, x)).collect()
    };
    let exprs = |vm: &mut Vm, v: &Value| -> PyResult<Vec<PyExpr>> {
        if py::is_none(v) {
            return Ok(Vec::new());
        }
        py::items(vm, v)?.iter().map(|x| pyexpr_of(vm, x)).collect()
    };
    let opt_expr = |vm: &mut Vm, v: &Value| -> PyResult<Option<PyExpr>> {
        if py::is_none(v) { Ok(None) } else { pyexpr_of(vm, v).map(Some) }
    };
    match t.len() {
        3 => Ok(ImSpec {
            name: names(vm, &t[0])?,
            expression: None,
            tag: None,
            at_list: exprs(vm, &t[1])?,
            layer: opt_text(vm, &t[2])?,
            zorder: None,
            behind: Vec::new(),
        }),
        6 | 7 => Ok(ImSpec {
            name: names(vm, &t[0])?,
            expression: opt_expr(vm, &t[1])?,
            tag: opt_text(vm, &t[2])?,
            at_list: exprs(vm, &t[3])?,
            layer: opt_text(vm, &t[4])?,
            zorder: opt_expr(vm, &t[5])?,
            behind: if t.len() == 7 && !py::is_none(&t[6]) { names(vm, &t[6])? } else { Vec::new() },
        }),
        n => Err(vm.exc_msg("ValueError", format!("an imspec has 3, 6 or 7 items, not {n}"))),
    }
}

fn atl_arg(vm: &mut Vm, v: Option<&Value>) -> PyResult<Option<Rc<crate::atl::RawBlock>>> {
    match v {
        None => Ok(None),
        Some(v) if py::is_none(v) => Ok(None),
        Some(v) => super::atlrt::raw_of(v).map(Some).ok_or_else(|| vm.type_error("atl must be a renpy.atl.RawBlock")),
    }
}

fn f_show_imspec(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let spec = imspec_from(vm, &args[0])?;
    let atl = atl_arg(vm, arg(args, kw, 1, "atl"))?;
    show_imspec(vm, &spec, &atl)?;
    Ok(Value::None)
}

fn f_predict_imspec(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let spec = imspec_from(vm, &args[0])?;
    let scene = match arg(args, kw, 1, "scene") {
        Some(v) => py::truthy(vm, v)?,
        None => false,
    };
    let atl = atl_arg(vm, arg(args, kw, 2, "atl"))?;
    predict_imspec(vm, &spec, scene, &atl)?;
    Ok(Value::None)
}

/// `get_imspec_tag(imspec)` (8.6): the tag, else the first name component.
fn f_get_imspec_tag(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::items(vm, &args[0])?;
    let rv = match t.len() {
        6 | 7 if py::truthy(vm, &t[2])? => t[2].clone(),
        _ => t[0].clone(),
    };
    if let Some(crysirror_python::Payload::Tuple(items)) = rv.payload() {
        return Ok(items.first().cloned().unwrap_or(Value::None));
    }
    Ok(rv)
}

fn f_reach_any(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

/// `get_reachable_nodes(entry_nodes, node_validator=_reach_any, seen=None)`
/// (8.4+): breadth-first closure over `Node.get_reachable()`.
fn f_get_reachable_nodes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let entry = py::items(vm, &args[0])?;
    let validator = match arg(args, kw, 1, "node_validator") {
        Some(v) => v.clone(),
        None => mget(vm, "_reach_any")?,
    };
    let seen = match arg(args, kw, 2, "seen") {
        Some(v) if !py::is_none(v) => v.clone(),
        _ => vm.types.new_set(Default::default()),
    };
    let mut result = Vec::new();
    let mut work: std::collections::VecDeque<Value> = entry.into();
    while let Some(node) = work.pop_front() {
        if ops::contains(vm, &seen, &node)? {
            continue;
        }
        ops::set_add(vm, &seen, node.clone())?;
        result.push(node.clone());
        let reach = py::method(vm, &node, "get_reachable", Vec::new())?;
        for n in py::items(vm, &reach)? {
            if ops::contains(vm, &seen, &n)? {
                continue;
            }
            let ok = vm.call(&validator, vec![node.clone(), n.clone()], Vec::new())?;
            if !py::truthy(vm, &ok)? {
                continue;
            }
            work.push_back(n);
        }
    }
    Ok(vm.types.new_list(result))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("statement_name", f_statement_name),
            ("next_node", f_next_node),
            ("say_menu_with", f_say_menu_with),
            ("eval_who", f_eval_who),
            ("create_store", f_create_store),
            ("get_namespace", f_get_namespace),
            ("redefine", f_redefine),
            ("apply_arguments", f_apply_arguments),
            ("probably_side_effect_free", f_probably_side_effect_free),
            ("chain_block", f_chain_block),
            ("show_imspec", f_show_imspec),
            ("predict_imspec", f_predict_imspec),
        ],
    );
    nodes::add_ast_classes(vm, &m)?;
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "current_statement_name", py::s(vm, "init"));
    set(vm, "define_statements", vm.types.new_list(Vec::new()));
    set(vm, "default_statements", vm.types.new_list(Vec::new()));
    let ec = vm.types.new_set(Default::default());
    for s in EARLY_CONFIG {
        ops::set_add(vm, &ec, py::s(vm, s))?;
    }
    set(vm, "EARLY_CONFIG", ec);
    let obj = Value::Obj(vm.types.object.clone());
    let sn = py::class(
        vm,
        MODULE,
        "StoreNamespace",
        &[obj.clone()],
        &[
            ("__init__", sn_init),
            ("set", sn_set),
            ("set_default", sn_set),
            ("get", sn_get),
        ],
        &[
            ("pure", Value::Bool(true)),
            ("repeat_at_default_time", Value::Bool(false)),
        ],
    )?;
    set(vm, "StoreNamespace", Value::Obj(sn));
    let str_t = Value::Obj(vm.types.str_.clone());
    let pn = vm.types.new_builtin("__new__", pe_new);
    let pn = py::staticmethod(vm, pn);
    let cp = vm.types.new_builtin("checkpoint", pe_none);
    let cp = py::staticmethod(vm, cp);
    let rv = vm.types.new_builtin("revert", pe_none);
    let rv = py::staticmethod(vm, rv);
    let pe = py::class(
        vm,
        MODULE,
        "PyExpr",
        &[str_t],
        &[("__getnewargs__", pe_getnewargs)],
        &[("__new__", pn), ("checkpoint", cp), ("revert", rv)],
    )?;
    set(vm, "PyExpr", Value::Obj(pe));
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let dne = vm.call(&sentinel, vec![py::s(vm, "DoesNotExtend")], Vec::new())?;
    set(vm, "DoesNotExtend", dne);
    // `Scry` is defined with the node classes (nodes::add_ast_classes).
    let _ = obj;
    let ai = py::import_attr(vm, "renpy.parameter", "ArgumentInfo")?;
    set(vm, "ArgumentInfo", ai);
    for n in ["Signature", "ParameterInfo"] {
        if let Ok(v) = py::import_attr(vm, "renpy.atl", "Signature") {
            set(vm, n, v);
        }
    }
    let profile = rt(vm).profile.clone();
    let at_least = |a: u32, b: u32| profile.at_least(a, b);
    if at_least(7, 5) {
        for n in ["EMPTY_PARAMETERS", "EMPTY_ARGUMENTS", "Parameter"] {
            if let Ok(v) = py::import_attr(vm, "renpy.parameter", n) {
                set(vm, n, v);
            }
        }
    }
    if at_least(7, 0) && !at_least(7, 4) {
        let re_compile = py::import_attr(vm, "re", "compile")?;
        let p = vm.call(&re_compile, vec![py::s(vm, "[a-zA-Z_][a-zA-Z_0-9]*$")], Vec::new())?;
        set(vm, "fast_who_pattern", p);
    }
    if at_least(7, 0) && !at_least(7, 5) {
        set(vm, "pyexpr_list", vm.types.new_list(Vec::new()));
    }
    if at_least(8, 4) {
        for (src, n) in [
            ("renpy.astsupport", "PyExpr"),
            ("renpy.astsupport", "hash32"),
            ("renpy.cslots", "Object"),
            ("renpy.cslots", "Slot"),
            ("renpy.cslots", "IntegerSlot"),
        ] {
            let v = py::import_attr(vm, src, n)?;
            set(vm, n, v);
        }
        let ra = py::builtin(vm, "_reach_any", f_reach_any);
        set(vm, "_reach_any", ra);
        let gr = py::builtin(vm, "get_reachable_nodes", f_get_reachable_nodes);
        set(vm, "get_reachable_nodes", gr);
    }
    if at_least(8, 6) {
        let g = py::builtin(vm, "get_imspec_tag", f_get_imspec_tag);
        set(vm, "get_imspec_tag", g);
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
