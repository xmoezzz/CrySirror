// SPDX-License-Identifier: MPL-2.0
//! Python faces of the Rust script arena: node handles (`renpy.ast.Say`,
//! `renpy.ast.Label`, ... instances whose attributes are read from the
//! arena) and `renpy.game.script` (`renpy.script.Script`: `lookup`,
//! `has_label`, `namemap`).
//!
//! Node names are the values Ren'Py stores in contexts and saves: a label
//! name string, or `(filename, version stamp, serial)` for generated names.
//! One handle exists per node, so `is` comparisons behave as upstream.
use super::{py, state::rt};
use crate::ast::{NameAtom, NodeId, NodeKind, NodeName, Rollback};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const AST: &str = "renpy.ast";
const SCRIPT: &str = "renpy.script";

struct Handle(NodeId);

pub fn id_of(v: &Value) -> Option<NodeId> {
    py::native::<Handle, _>(v, |h| h.0)
}

pub fn name_value(vm: &Vm, n: &NodeName) -> Value {
    match n {
        NodeName::Label(l) => vm.types.new_str(l),
        NodeName::Other(o) => vm.types.new_str(o),
        NodeName::Generated { .. } | NodeName::Tuple(_) => {
            let items = n.atoms().unwrap_or_default().iter().map(|a| match a {
                NameAtom::Str(s) => vm.types.new_str(s),
                NameAtom::Int(i) => Value::Int(*i),
                NameAtom::Float(b) => Value::Float(f64::from_bits(*b)),
            }).collect();
            vm.types.new_tuple(items)
        }
    }
}

/// The node a name value refers to.
pub fn find(vm: &mut Vm, name: &Value) -> PyResult<Option<NodeId>> {
    if let Some(id) = id_of(name) {
        return Ok(Some(id));
    }
    let r = rt(vm);
    let script = r.script.borrow();
    if let Some(s) = name.as_str() {
        let s: Rc<str> = s.into();
        if let Some(id) = script.namemap.get(&NodeName::Label(s.clone())) {
            return Ok(Some(*id));
        }
        return Ok(script.namemap.get(&NodeName::Other(s)).copied());
    }
    let Some(parts) = py::seq(name) else { return Ok(None) };
    let mut atoms = Vec::with_capacity(parts.len());
    for p in &parts {
        atoms.push(match p {
            Value::Int(i) => NameAtom::Int(*i),
            Value::Bool(b) => NameAtom::Int(*b as i64),
            Value::Float(x) => NameAtom::Float(x.to_bits()),
            v => match v.as_str() {
                Some(s) => NameAtom::Str(s.into()),
                None => return Ok(None),
            },
        });
    }
    Ok(script.namemap.get(&NodeName::from_atoms(atoms)).copied())
}

fn class_for(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Say { .. } => "Say",
        NodeKind::Init { .. } => "Init",
        NodeKind::Label { .. } => "Label",
        NodeKind::Python { .. } => "Python",
        NodeKind::EarlyPython { .. } => "EarlyPython",
        NodeKind::Image { .. } => "Image",
        NodeKind::Transform { .. } => "Transform",
        NodeKind::Show { .. } => "Show",
        NodeKind::ShowLayer { .. } => "ShowLayer",
        NodeKind::Camera { .. } => "Camera",
        NodeKind::Scene { .. } => "Scene",
        NodeKind::Hide { .. } => "Hide",
        NodeKind::With { .. } => "With",
        NodeKind::Call { .. } => "Call",
        NodeKind::Return { .. } => "Return",
        NodeKind::Menu { .. } => "Menu",
        NodeKind::Jump { .. } => "Jump",
        NodeKind::Pass => "Pass",
        NodeKind::While { .. } => "While",
        NodeKind::If { .. } => "If",
        NodeKind::UserStatement { .. } => "UserStatement",
        NodeKind::PostUserStatement { .. } => "PostUserStatement",
        NodeKind::Define { .. } => "Define",
        NodeKind::Default { .. } => "Default",
        NodeKind::Screen { .. } => "Screen",
        NodeKind::Style(_) => "Style",
        NodeKind::Testcase { .. } => "Testcase",
        NodeKind::Rpy { .. } => "RPY",
        NodeKind::Translate { .. } => "Translate",
        NodeKind::TranslateSay { .. } => "TranslateSay",
        NodeKind::EndTranslate => "EndTranslate",
        NodeKind::TranslateString { .. } => "TranslateString",
        NodeKind::TranslatePython { .. } => "TranslatePython",
        NodeKind::TranslateBlock { .. } => "TranslateBlock",
        NodeKind::TranslateEarlyBlock { .. } => "TranslateEarlyBlock",
    }
}

/// The handle of node `id`.
pub fn handle(vm: &mut Vm, id: NodeId) -> PyResult<Value> {
    let r = rt(vm);
    if let Some(h) = r.handles.borrow().get(&id) {
        return Ok(Value::Obj(h.clone()));
    }
    let cls = class_for(&r.script.borrow().arena.get(id).kind);
    let c = py::import_attr(vm, AST, cls)?;
    let c = c.obj().cloned().ok_or_else(|| vm.type_error(cls))?;
    let v = vm.types.new_native(&c, Box::new(Handle(id)));
    r.handles.borrow_mut().insert(id, v.obj().cloned().unwrap());
    Ok(v)
}

pub fn opt_handle(vm: &mut Vm, id: Option<NodeId>) -> PyResult<Value> {
    match id {
        Some(id) => handle(vm, id),
        None => Ok(Value::None),
    }
}

fn block_list(vm: &mut Vm, b: &[NodeId]) -> PyResult<Value> {
    let mut v = Vec::with_capacity(b.len());
    for id in b {
        v.push(handle(vm, *id)?);
    }
    Ok(vm.types.new_list(v))
}

fn opt_str(vm: &Vm, s: &Option<String>) -> Value {
    s.as_ref().map(|s| vm.types.new_str(s)).unwrap_or(Value::None)
}

fn strs(vm: &Vm, v: &[String]) -> Value {
    let items = v.iter().map(|s| vm.types.new_str(s)).collect();
    vm.types.new_tuple(items)
}

fn expr(vm: &Vm, e: &crate::lexer::PyExpr) -> Value {
    vm.types.new_str(&e.text)
}

fn opt_expr(vm: &Vm, e: &Option<crate::lexer::PyExpr>) -> Value {
    e.as_ref().map(|e| expr(vm, e)).unwrap_or(Value::None)
}

/// Attribute reads of a node handle.
fn n_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node handle expected"))?;
    let name = args.get(1).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let r = rt(vm);
    let node = r.script.borrow().arena.get(id).clone();
    let v = match name.as_str() {
        "filename" => vm.types.new_str(&node.filename),
        "linenumber" => Value::Int(node.linenumber as i64),
        "name" => node.name.as_ref().map(|n| name_value(vm, n)).unwrap_or(Value::None),
        "next" => opt_handle(vm, node.next)?,
        "statement_start" => match node.statement_start {
            Some(s) => handle(vm, s)?,
            None => args[0].clone(),
        },
        "rollback" => vm.types.new_str(match node.rollback {
            Rollback::Normal => "normal",
            Rollback::Never => "never",
            Rollback::Force => "force",
        }),
        _ => return kind_attr(vm, &node.kind, &name),
    };
    Ok(v)
}

fn kind_attr(vm: &mut Vm, kind: &NodeKind, name: &str) -> PyResult<Value> {
    let missing = |vm: &Vm| Err(vm.exc_msg("AttributeError", name));
    Ok(match (kind, name) {
        (NodeKind::Say { who, .. }, "who") => opt_expr(vm, who),
        (NodeKind::Say { who, .. }, "who_fast") => Value::Bool(who.as_ref().is_some_and(|w| crate::lexer::is_name(&w.text))),
        (NodeKind::Say { what, .. }, "what") => vm.types.new_str(what),
        (NodeKind::Say { with_, .. }, "with_") => opt_expr(vm, with_),
        (NodeKind::Say { interact, .. }, "interact") => Value::Bool(*interact),
        (NodeKind::Say { attributes, .. }, "attributes") => attributes.as_ref().map(|a| strs(vm, a)).unwrap_or(Value::None),
        (NodeKind::Say { temporary_attributes, .. }, "temporary_attributes") => temporary_attributes.as_ref().map(|a| strs(vm, a)).unwrap_or(Value::None),
        (NodeKind::Say { identifier, .. }, "identifier") => opt_str(vm, identifier),
        (NodeKind::Say { explicit_identifier, .. }, "explicit_identifier") => Value::Bool(*explicit_identifier),
        (NodeKind::Say { arguments, .. }, "arguments") => Value::Bool(arguments.is_some()),
        (NodeKind::Label { name, .. }, "label") => vm.types.new_str(name),
        (NodeKind::Label { hide, .. }, "hide") => Value::Bool(*hide),
        (NodeKind::Label { parameters, .. }, "parameters") => match parameters {
            Some(p) => super::atlrt::signature_value(vm, p.clone())?,
            None => Value::None,
        },
        (NodeKind::Label { block, .. } | NodeKind::Init { block, .. } | NodeKind::While { block, .. } | NodeKind::Translate { block, .. } | NodeKind::TranslateBlock { block, .. } | NodeKind::TranslateEarlyBlock { block, .. }, "block") => block_list(vm, block)?,
        (NodeKind::Init { priority, .. }, "priority") => Value::Int(*priority as i64),
        (NodeKind::Python { hide, .. } | NodeKind::EarlyPython { hide, .. }, "hide") => Value::Bool(*hide),
        (NodeKind::Python { store, .. } | NodeKind::EarlyPython { store, .. } | NodeKind::Define { store, .. } | NodeKind::Default { store, .. } | NodeKind::Transform { store, .. }, "store") => vm.types.new_str(store),
        (NodeKind::Python { code, .. } | NodeKind::EarlyPython { code, .. } | NodeKind::Define { code, .. } | NodeKind::Default { code, .. }, "code") => pycode(vm, code)?,
        (NodeKind::Define { name, .. } | NodeKind::Default { name, .. }, "varname") => vm.types.new_str(name),
        (NodeKind::Define { operator, .. }, "operator") => vm.types.new_str(operator),
        (NodeKind::Define { index, .. }, "index") => opt_expr(vm, index),
        (NodeKind::Image { name, .. }, "imgname") => strs(vm, name),
        (NodeKind::Transform { name, .. }, "varname") => vm.types.new_str(name),
        (NodeKind::With { expr: e, .. }, "expr") => opt_expr(vm, e),
        (NodeKind::With { paired, .. }, "paired") => opt_expr(vm, paired),
        (NodeKind::Call { label, .. }, "label") => vm.types.new_str(label),
        (NodeKind::Call { expression, .. } | NodeKind::Jump { expression, .. }, "expression") => Value::Bool(*expression),
        (NodeKind::Call { global_label, .. } | NodeKind::Jump { global_label, .. }, "global_label") => vm.types.new_str(global_label),
        (NodeKind::Jump { target, .. }, "target") => vm.types.new_str(target),
        (NodeKind::Return { expression }, "expression") => opt_expr(vm, expression),
        (NodeKind::Menu { items, .. }, "items") => {
            let mut v = Vec::new();
            for it in items {
                let l = vm.types.new_str(&it.label);
                let c = expr(vm, &it.condition);
                let b = match &it.block {
                    Some(b) => block_list(vm, b)?,
                    None => Value::None,
                };
                v.push(vm.types.new_tuple(vec![l, c, b]));
            }
            vm.types.new_list(v)
        }
        (NodeKind::Menu { set, .. }, "set") => opt_expr(vm, set),
        (NodeKind::Menu { with_, .. }, "with_") => opt_expr(vm, with_),
        (NodeKind::Menu { has_caption, .. }, "has_caption") => Value::Bool(*has_caption),
        (NodeKind::If { entries }, "entries") => {
            let mut v = Vec::new();
            for (c, b) in entries {
                let c = expr(vm, c);
                let b = block_list(vm, b)?;
                v.push(vm.types.new_tuple(vec![c, b]));
            }
            vm.types.new_list(v)
        }
        (NodeKind::While { condition, .. }, "condition") => expr(vm, condition),
        (NodeKind::UserStatement { line, .. }, "line") => vm.types.new_str(line),
        (NodeKind::UserStatement { parsed, .. }, "parsed") => parsed.clone().unwrap_or(Value::None),
        (NodeKind::UserStatement { block, .. }, "block") => super::lexer::block_value(vm, block),
        (NodeKind::UserStatement { code_block, .. }, "code_block") => match code_block {
            Some(b) => block_list(vm, b)?,
            None => Value::None,
        },
        (NodeKind::UserStatement { subparses, .. }, "subparses") => {
            let c = py::import_attr(vm, "renpy.lexer", "SubParse")?;
            let mut v = Vec::new();
            for b in subparses {
                let b = block_list(vm, b)?;
                v.push(vm.call(&c, vec![b], Vec::new())?);
            }
            vm.types.new_list(v)
        }
        (NodeKind::UserStatement { init_priority, .. }, "init_priority") => Value::Int(*init_priority as i64),
        (NodeKind::UserStatement { translatable, .. }, "translatable") => Value::Bool(*translatable),
        (NodeKind::UserStatement { atl, .. }, "atl") => match atl {
            Some(a) => super::atlrt::raw_value(vm, a.clone())?,
            None => Value::None,
        },
        (NodeKind::Screen { screen }, "screen") => super::slast::screen_value(vm, screen.clone())?,
        (NodeKind::Translate { identifier, .. } | NodeKind::TranslateSay { identifier, .. }, "identifier") => vm.types.new_str(identifier),
        (NodeKind::Translate { language, .. } | NodeKind::TranslateSay { language, .. } | NodeKind::TranslateString { language, .. } | NodeKind::TranslatePython { language, .. } | NodeKind::TranslateBlock { language, .. } | NodeKind::TranslateEarlyBlock { language, .. }, "language") => opt_str(vm, language),
        (NodeKind::Translate { alternate, .. }, "alternate") => opt_str(vm, alternate),
        (NodeKind::TranslateString { old, .. }, "old") => vm.types.new_str(old),
        (NodeKind::TranslateString { new, .. }, "new") => vm.types.new_str(new),
        (NodeKind::Testcase { name }, "label") => vm.types.new_str(name),
        _ => return missing(vm),
    })
}

fn pycode(vm: &mut Vm, code: &crate::ast::PyCode) -> PyResult<Value> {
    let c = py::import_attr(vm, AST, "PyCode")?;
    let src = vm.types.new_str(&code.source.text);
    let f = vm.types.new_str(&code.source.filename);
    let loc = vm.types.new_tuple(vec![f, Value::Int(code.source.line as i64)]);
    let mode = vm.types.new_str(code.mode);
    vm.call(&c, vec![src, loc, mode], Vec::new())
}

fn n_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    let r = rt(vm);
    let s = {
        let script = r.script.borrow();
        let n = script.arena.get(id);
        format!("<{} {}:{}>", class_for(&n.kind), n.filename, n.linenumber)
    };
    Ok(vm.types.new_str(&s))
}

/// `diff_info()`: the (class, identity) tuple Ren'Py uses for reload diffs.
fn n_diff_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    let r = rt(vm);
    let (cls, extra) = {
        let script = r.script.borrow();
        let n = script.arena.get(id);
        let extra = match &n.kind {
            NodeKind::Say { who, what, .. } => vec![who.as_ref().map(|w| w.text.to_string()).unwrap_or_default(), what.clone()],
            NodeKind::Label { name, .. } => vec![name.to_string()],
            NodeKind::Jump { target, .. } => vec![target.clone()],
            NodeKind::Call { label, .. } => vec![label.clone()],
            _ => Vec::new(),
        };
        (class_for(&n.kind), extra)
    };
    let mut items = vec![vm.types.new_str(cls)];
    items.extend(extra.iter().map(|e| vm.types.new_str(e)));
    if items.len() == 1 {
        items.push(Value::Int(id as i64));
    }
    Ok(vm.types.new_tuple(items))
}

fn n_get_code(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    let r = rt(vm);
    let code = {
        let script = r.script.borrow();
        match &script.arena.get(id).kind {
            NodeKind::Say { who, what, attributes, temporary_attributes, with_, interact, identifier, explicit_identifier, arguments } => {
                let mut rv = Vec::new();
                if let Some(w) = who {
                    rv.push(w.text.to_string());
                }
                if let Some(a) = attributes {
                    rv.extend(a.iter().cloned());
                }
                if let Some(t) = temporary_attributes.as_ref().filter(|t| !t.is_empty()) {
                    rv.push("@".into());
                    rv.extend(t.iter().cloned());
                }
                rv.push(crate::lexer::encode_say_string(what));
                if !interact {
                    rv.push("nointeract".into());
                }
                if let (Some(id), true) = (identifier, *explicit_identifier) {
                    rv.push("id".into());
                    rv.push(id.clone());
                }
                if let Some(a) = arguments {
                    rv.push(a.get_code());
                }
                if let Some(w) = with_ {
                    rv.push("with".into());
                    rv.push(w.text.to_string());
                }
                rv.join(" ")
            }
            _ => String::new(),
        }
    };
    Ok(vm.types.new_str(&code))
}

fn n_execute(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    super::astexec::execute(vm, id)?;
    Ok(Value::None)
}

fn n_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    let next = super::astexec::predict(vm, id)?;
    let mut v = Vec::new();
    for n in next {
        v.push(opt_handle(vm, n)?);
    }
    Ok(vm.types.new_list(v))
}

fn n_scry(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    super::astexec::scry(vm, id)
}

/// `Scry.__getattr__`: every unset field is None.
fn scry_getattr(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `Scry.next()`: the scry of the next statement, if known.
fn scry_next(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get_or_none(vm, &args[0], "_next")?;
    if py::is_none(&n) {
        return Ok(Value::None);
    }
    Ok(py::method(vm, &n, "scry", Vec::new()).unwrap_or(Value::None))
}

fn n_get_children(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = id_of(&args[0]).ok_or_else(|| vm.type_error("node"))?;
    let f = args.get(1).cloned().unwrap_or(Value::None);
    let r = rt(vm);
    let mut all = Vec::new();
    r.script.borrow().collect(id, &mut all);
    for n in all {
        let h = handle(vm, n)?;
        vm.call(&f, vec![h], Vec::new())?;
    }
    Ok(Value::None)
}

fn n_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let _ = vm;
    Ok(Value::Bool(id_of(&args[0]).is_some() && id_of(&args[0]) == id_of(&args[1])))
}

fn n_hash(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(id_of(&args[0]).unwrap_or(0) as i64))
}

fn n_reduce(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Script nodes are saved by name, not pickled."))
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("script nodes are created by the script loader"))
}

// PyCode -----------------------------------------------------------------------------------------------

fn pc_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "source", py::arg_or(args, kw, 1, "source", Value::None))?;
    py::set(vm, this, "location", py::arg_or(args, kw, 2, "loc", Value::None))?;
    let mode = py::arg_or(args, kw, 3, "mode", vm.types.new_str("exec"));
    py::set(vm, this, "mode", mode)?;
    py::set(vm, this, "bytecode", Value::None)?;
    Ok(Value::None)
}

// Script facade ----------------------------------------------------------------------------------------

fn missing(vm: &mut Vm, label: &Value) -> PyResult<Option<NodeId>> {
    let cb = py::config(vm, "missing_label_callback")?;
    if py::is_none(&cb) {
        return Ok(None);
    }
    let l = vm.call(&cb, vec![label.clone()], Vec::new())?;
    find(vm, &l)
}

pub fn lookup(vm: &mut Vm, label: &Value) -> PyResult<NodeId> {
    if let Some(id) = find(vm, label)? {
        return Ok(id);
    }
    if let Some(id) = missing(vm, label)? {
        return Ok(id);
    }
    let s = ops::str(vm, label)?;
    let e = py::import_attr(vm, "renpy.script", "ScriptError")?;
    let msg = vm.types.new_str(&format!("could not find label '{s}'."));
    let ex = vm.call(&e, vec![msg], Vec::new())?;
    Err(crysirror_python::PyErr::Exc(ex.obj().cloned().unwrap()))
}

fn s_lookup(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = lookup(vm, &args[1])?;
    handle(vm, id)
}

fn s_lookup_or_none(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[1]) {
        return Ok(Value::None);
    }
    let id = match find(vm, &args[1])? {
        Some(id) => Some(id),
        None => missing(vm, &args[1])?,
    };
    opt_handle(vm, id)
}

fn s_has_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(find(vm, &args[1])?.is_some()))
}

fn s_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `script.namemap`: a mapping over node names (handles as values).
fn s_namemap(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, SCRIPT, "NameMap")?;
    let c = c.obj().cloned().unwrap();
    Ok(vm.types.new_native(&c, Box::new(())))
}

fn nm_names(vm: &mut Vm) -> Vec<Value> {
    let r = rt(vm);
    let names: Vec<NodeName> = r.script.borrow().namemap.keys().cloned().collect();
    names.iter().map(|n| name_value(vm, n)).collect()
}

fn nm_contains(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(find(vm, &args[1])?.is_some()))
}

fn nm_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    match find(vm, &args[1])? {
        Some(id) => handle(vm, id),
        None => {
            let r = ops::repr(vm, &args[1])?;
            Err(vm.exc_msg("KeyError", &r))
        }
    }
}

fn nm_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    match find(vm, &args[1])? {
        Some(id) => handle(vm, id),
        None => Ok(args.get(2).cloned().unwrap_or(Value::None)),
    }
}

fn nm_keys(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let k = nm_names(vm);
    Ok(vm.types.new_list(k))
}

fn nm_iter(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let k = nm_keys(vm, args, kw)?;
    ops::iter(vm, &k)
}

fn nm_len(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(rt(vm).script.borrow().namemap.len() as i64))
}

fn native_type(vm: &mut Vm, module: &str, name: &str, base: &ObjRef, methods: py::Methods) -> ObjRef {
    let c = vm.types.new_builtin_type(name, base, crysirror_python::object::BaseKind::Native, false);
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str(module);
    vm.types.set_attr(&c, "__module__", mn);
    c
}

const NODE_CLASSES: &[&str] = &[
    "Say", "Init", "Label", "Python", "EarlyPython", "Image", "Transform", "Show", "ShowLayer", "Camera", "Scene", "Hide", "With", "Call", "Return", "Menu", "Jump", "Pass", "While", "If", "UserStatement", "PostUserStatement", "Define", "Default", "Screen", "Style", "Testcase", "RPY", "Translate", "TranslateSay", "EndTranslate", "TranslateString", "TranslatePython", "TranslateBlock", "TranslateEarlyBlock",
];

/// Adds the node classes and `PyCode` to `renpy.ast` (whose runtime
/// functions live in `astexec`).
pub fn add_ast_classes(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let object = vm.types.object.clone();
    let node = native_type(vm, AST, "Node", &object, &[
        ("__getattr__", n_getattr),
        ("__repr__", n_repr),
        ("__eq__", n_eq),
        ("__hash__", n_hash),
        ("__reduce__", n_reduce),
        ("__reduce_ex__", n_reduce),
        ("diff_info", n_diff_info),
        ("get_code", n_get_code),
        ("execute", n_execute),
        ("predict", n_predict),
        ("get_children", n_get_children),
        ("scry", n_scry),
    ]);
    let nn = vm.types.new_builtin("__new__", no_new);
    let nn = py::staticmethod(vm, nn);
    vm.types.set_attr(&node, "__new__", nn);
    crysirror_python::modules::set(vm, m, "Node", Value::Obj(node.clone()));
    for n in NODE_CLASSES {
        let c = native_type(vm, AST, n, &node, &[]);
        crysirror_python::modules::set(vm, m, n, Value::Obj(c));
    }
    let obj = Value::Obj(vm.types.object.clone());
    let pc = py::class(vm, AST, "PyCode", &[obj.clone()], &[("__init__", pc_init)], &[])?;
    crysirror_python::modules::set(vm, m, "PyCode", Value::Obj(pc));
    let f = Value::Bool(false);
    let scry = py::class(vm, AST, "Scry", &[obj], &[("__getattr__", scry_getattr), ("next", scry_next)], &[
        ("_next", Value::None), ("interacts", Value::None), ("say", f.clone()), ("menu_with_caption", f), ("who", Value::None), ("extend_text", Value::None), ("multiple", Value::None),
    ])?;
    crysirror_python::modules::set(vm, m, "Scry", Value::Obj(scry));
    Ok(())
}

fn init_script(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, SCRIPT, &[]);
    let exc = Value::Obj(vm.types.exc("Exception"));
    let se = py::class(vm, SCRIPT, "ScriptError", &[exc], &[], &[])?;
    crysirror_python::modules::set(vm, &m, "ScriptError", Value::Obj(se));
    let object = vm.types.object.clone();
    let nm = native_type(vm, SCRIPT, "NameMap", &object, &[
        ("__contains__", nm_contains),
        ("__getitem__", nm_getitem),
        ("get", nm_get),
        ("keys", nm_keys),
        ("__iter__", nm_iter),
        ("__len__", nm_len),
    ]);
    crysirror_python::modules::set(vm, &m, "NameMap", Value::Obj(nm));
    let obj = Value::Obj(vm.types.object.clone());
    let namemap = vm.types.new_builtin("namemap", s_namemap);
    let namemap = py::property(vm, namemap, None)?;
    let mut methods: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("lookup", s_lookup),
        ("lookup_or_none", s_lookup_or_none),
        ("has_label", s_has_label),
        ("update_bytecode", s_noop),
        ("save_bytecode", s_noop),
        ("analyze", s_noop),
        ("make_backups", s_noop),
        ("scan_script_files", s_noop),
    ];
    methods.extend_from_slice(super::loader::SCRIPT_METHODS);
    let sc = py::class(vm, SCRIPT, "Script", &[obj], &methods, &[("namemap", namemap)])?;
    crysirror_python::modules::set(vm, &m, "Script", Value::Obj(sc));
    // Module constants of renpy/script.py. The bytecode cache is named by
    // the Python of the game's release (3.9 for 8.0-8.3, 3.12 from 8.4).
    let r = rt(vm);
    let v = r.profile.version;
    let sv = (v[0] * 1_000_000 + v[1] * 1000 + v[2]) as i64;
    crysirror_python::modules::set(vm, &m, "script_version", Value::Int(sv));
    crysirror_python::modules::set(vm, &m, "BYTECODE_VERSION", Value::Int(1));
    let file = if v[0] < 8 {
        let magic = vm.types.new_bytes(vec![0x03, 0xf3, 0x0d, 0x0a]);
        crysirror_python::modules::set(vm, &m, "MAGIC", magic);
        "cache/bytecode.rpyb".to_owned()
    } else {
        crysirror_python::modules::set(vm, &m, "OLD_BYTECODE_FILE", vm.types.new_str("cache/bytecode.rpyb"));
        let py = if r.profile.at_least(8, 4) { "312" } else { "39" };
        format!("cache/bytecode-{py}.rpyb")
    };
    crysirror_python::modules::set(vm, &m, "BYTECODE_FILE", vm.types.new_str(&file));
    let cs = vm.types.new_builtin("collapse_stmts", collapse_stmts);
    crysirror_python::modules::set(vm, &m, "collapse_stmts", cs);
    Ok(m)
}

/// `collapse_stmts(stmts)`: the statements and every statement nested
/// in their blocks, in order.
fn collapse_stmts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for s in py::items(vm, &args[0])? {
        match id_of(&s) {
            Some(id) => ids.push(id),
            None => out.push(s),
        }
    }
    let r = rt(vm);
    let mut all = Vec::new();
    {
        let script = r.script.borrow();
        fn visit(a: &crate::ast::Arena, id: NodeId, all: &mut Vec<NodeId>) {
            all.push(id);
            for b in a.child_blocks(id) {
                for c in b {
                    visit(a, c, all);
                }
            }
        }
        for id in ids {
            visit(&script.arena, id, &mut all);
        }
    }
    for id in all {
        out.push(handle(vm, id)?);
    }
    Ok(vm.types.new_list(out))
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(SCRIPT, init_script);
}
