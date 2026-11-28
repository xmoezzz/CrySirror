// SPDX-License-Identifier: MPL-2.0
//! Statement execution (`renpy.ast.*.execute`). Statements whose effect
//! belongs to the display, audio or character subsystems call the runtime
//! entry points of the `renpy` package (`renpy._stmt_*`) as suspendable
//! tasks; control statements are handled here.

use crate::ast::*;
use crate::engine::{Engine, ExecError, Flow, Then, Wait};
use crate::lexer::PyExpr;
use crysirror_python::{PyErr, TaskOutcome, Value, ops};
use std::rc::Rc;

type R = Result<Flow, ExecError>;

fn s(e: &Engine, v: &str) -> Value {
    e.vm.types.new_str(v)
}

fn opt_expr(e: &mut Engine, id: NodeId, slot: u8, x: &Option<PyExpr>) -> Result<Value, ExecError> {
    match x {
        Some(x) => Ok(e.eval_cached(id, slot, x)?),
        None => Ok(Value::None),
    }
}

fn str_list(e: &Engine, v: &[String]) -> Value {
    let items = v.iter().map(|x| s(e, x)).collect();
    e.vm.types.new_tuple(items)
}

fn opt_str(e: &Engine, v: &Option<String>) -> Value {
    v.as_ref().map(|x| s(e, x)).unwrap_or(Value::None)
}

/// Evaluated argument tuple and keyword dict of an ArgumentInfo.
pub fn eval_arguments(
    e: &mut Engine,
    id: NodeId,
    info: &Option<ArgumentInfo>,
) -> Result<(Value, Value), ExecError> {
    let args = e.vm.types.new_list(Vec::new());
    let kwargs = e.vm.types.new_dict();
    if let Some(a) = info {
        for (i, (name, expr)) in a.arguments.iter().enumerate() {
            let (f, l) = {
                let n = e.st.script.borrow().arena.get(id).clone();
                (n.filename.clone(), n.linenumber)
            };
            let v = e.eval_cached(id, 100 + i as u8, &PyExpr::new(expr.as_str(), f, l))?;
            if a.starred.contains(&i) {
                for x in ops::collect(&mut e.vm, &v)? {
                    ops::list_append(&mut e.vm, &args, x)?;
                }
            } else if a.doublestarred.contains(&i) {
                for (k, x) in ops::mapping_items(&mut e.vm, &v)? {
                    ops::setitem(&mut e.vm, &kwargs, k, x)?;
                }
            } else if let Some(n) = name {
                let k = s(e, n);
                ops::setitem(&mut e.vm, &kwargs, k, v)?;
            } else {
                ops::list_append(&mut e.vm, &args, v)?;
            }
        }
    }
    let t = ops::collect(&mut e.vm, &args)?;
    Ok((e.vm.types.new_tuple(t), kwargs))
}

/// Calls `renpy.<name>(*args)` as a task; continues at `next` on completion.
fn task(e: &mut Engine, id: NodeId, name: &str, args: Vec<Value>, next: Option<NodeId>) -> R {
    let m = crysirror_python::import::import_module(&mut e.vm, "renpy")?;
    let f = ops::getattr(&mut e.vm, &m, name)?;
    run_task(e, id, &f, args, next)
}

pub fn run_task(
    e: &mut Engine,
    id: NodeId,
    f: &Value,
    args: Vec<Value>,
    _next: Option<NodeId>,
) -> R {
    match e.vm.start_task(f, args, Vec::new()) {
        TaskOutcome::Done(value) => Ok(e.complete_task(id, value)),
        TaskOutcome::Error(exc) => Err(ExecError::Python(PyErr::Exc(exc))),
        TaskOutcome::Suspended(task, req) => Ok(Flow::Wait(Box::new(suspended(
            e,
            task,
            req,
            Then::Node(id),
        )))),
    }
}

pub fn suspended(
    _e: &mut Engine,
    task: crysirror_python::Task,
    req: Box<dyn std::any::Any>,
    then: Then,
) -> Wait {
    match req.downcast::<crate::engine::Request>() {
        Ok(r) => match *r {
            crate::engine::Request::Interact(request) => Wait::Interaction {
                task,
                then,
                request,
            },
            crate::engine::Request::CallInNewContext {
                label,
                args,
                kwargs,
            } => Wait::NestedCall {
                task,
                then,
                label,
                args,
                kwargs,
            },
            _ => Wait::Task { task, then },
        },
        Err(_) => Wait::Task { task, then },
    }
}

fn imspec_args(e: &mut Engine, id: NodeId, spec: &ImSpec) -> Result<Vec<Value>, ExecError> {
    let name = str_list(e, &spec.name);
    let expression = match &spec.expression {
        Some(x) => e.eval_cached(id, 1, x)?,
        None => Value::None,
    };
    let mut at = Vec::new();
    for (i, x) in spec.at_list.iter().enumerate() {
        at.push(e.eval_cached(id, 10 + i as u8, x)?);
    }
    let at = e.vm.types.new_list(at);
    let zorder = opt_expr(e, id, 2, &spec.zorder)?;
    let behind =
        e.vm.types
            .new_list(spec.behind.iter().map(|b| s(e, b)).collect());
    Ok(vec![
        name,
        expression,
        opt_str(e, &spec.tag),
        at,
        opt_str(e, &spec.layer),
        zorder,
        behind,
    ])
}

fn atl_value(e: &mut Engine, atl: &Option<Rc<crate::atl::RawBlock>>) -> Result<Value, ExecError> {
    match atl {
        Some(a) => {
            let globals = e.store_dict("store");
            Ok(crate::natives::atl_transform(
                &mut e.vm,
                (**a).clone(),
                None,
                globals,
            ))
        }
        None => Ok(Value::None),
    }
}

pub fn execute(e: &mut Engine, id: NodeId, node: &Node) -> R {
    let next = node.next;
    match &node.kind {
        NodeKind::Pass
        | NodeKind::Init { .. }
        | NodeKind::EarlyPython { .. }
        | NodeKind::Rpy { .. }
        | NodeKind::Testcase { .. }
        | NodeKind::TranslateEarlyBlock { .. } => Ok(Flow::Next(next)),
        NodeKind::Label {
            name, parameters, ..
        } => {
            bind_label(e, node, parameters.as_ref())?;
            let name = s(e, name);
            task(e, id, "_stmt_label", vec![name], next)
        }
        NodeKind::Python { code, hide, store } => {
            let c = e.compile_code(id, code)?;
            let g = e.ensure_store(store);
            let locals = if *hide {
                Some(e.vm.types.new_dict())
            } else {
                None
            };
            match e.vm.start_code_task(c, g, locals) {
                TaskOutcome::Done(_) => Ok(Flow::Next(e.next_node(next))),
                TaskOutcome::Error(exc) => Err(ExecError::Python(PyErr::Exc(exc))),
                TaskOutcome::Suspended(task, req) => Ok(Flow::Wait(Box::new(suspended(
                    e,
                    task,
                    req,
                    Then::Node(id),
                )))),
            }
        }
        NodeKind::Say {
            who,
            what,
            with_,
            interact,
            attributes,
            temporary_attributes,
            arguments,
            ..
        } => {
            let w = match who {
                Some(x) => e.eval_cached(id, 0, x)?,
                None => Value::None,
            };
            let with = opt_expr(e, id, 1, with_)?;
            let (args, kwargs) = eval_arguments(e, id, arguments)?;
            let attrs = attributes
                .as_ref()
                .map(|a| str_list(e, a))
                .unwrap_or(Value::None);
            let tattrs = temporary_attributes
                .as_ref()
                .map(|a| str_list(e, a))
                .unwrap_or(Value::None);
            let wv = s(e, what);
            let who_name = who.as_ref().map(|w| s(e, &w.text)).unwrap_or(Value::None);
            task(
                e,
                id,
                "_stmt_say",
                vec![
                    w,
                    wv,
                    Value::Bool(*interact),
                    attrs,
                    tattrs,
                    args,
                    kwargs,
                    with,
                    who_name,
                ],
                next,
            )
        }
        NodeKind::Image { name, code, atl } => {
            let img = match code {
                Some(c) => e.eval_cached(id, 0, &c.source)?,
                None => atl_value(e, atl)?,
            };
            let n = str_list(e, name);
            task(e, id, "_stmt_image", vec![n, img], next)
        }
        NodeKind::Transform {
            store,
            name,
            atl,
            parameters,
        } => {
            let g = e.ensure_store(store);
            let t = crate::natives::atl_transform(
                &mut e.vm,
                (**atl).clone(),
                parameters.clone(),
                g.clone(),
            );
            g.dict_set_raw(&e.vm.types, e.vm.types.new_str(name), t);
            Ok(Flow::Next(next))
        }
        NodeKind::Show { imspec, atl } => {
            let mut a = imspec_args(e, id, imspec)?;
            a.push(atl_value(e, atl)?);
            task(e, id, "_stmt_show", a, next)
        }
        NodeKind::Scene { imspec, layer, atl } => {
            let mut a = match imspec {
                Some(spec) => imspec_args(e, id, spec)?,
                None => vec![Value::None; 7],
            };
            a.push(atl_value(e, atl)?);
            a.push(opt_str(e, layer));
            task(e, id, "_stmt_scene", a, next)
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
            let mut at = Vec::new();
            for (i, x) in at_list.iter().enumerate() {
                at.push(e.eval_cached(id, 10 + i as u8, x)?);
            }
            let at = e.vm.types.new_list(at);
            let a = atl_value(e, atl)?;
            let l = s(e, layer);
            task(e, id, "_stmt_show_layer", vec![l, at, a], next)
        }
        NodeKind::Hide { imspec } => {
            let a = imspec_args(e, id, imspec)?;
            task(e, id, "_stmt_hide", a, next)
        }
        NodeKind::With { expr, paired } => {
            let t = opt_expr(e, id, 0, expr)?;
            let p = opt_expr(e, id, 1, paired)?;
            task(e, id, "_stmt_with", vec![t, p], next)
        }
        NodeKind::Call {
            label,
            expression,
            arguments,
            ..
        } => {
            let target = if *expression {
                let f = node.filename.clone();
                let v = e.eval_cached(id, 0, &PyExpr::new(label.as_str(), f, node.linenumber))?;
                ops::str(&mut e.vm, &v)?
            } else {
                label.clone()
            };
            let (args, kwargs) = eval_arguments(e, id, arguments)?;
            let t = e.call_label(&target, next)?;
            e.set_store(
                "_args",
                if arguments.is_some() {
                    args
                } else {
                    Value::None
                },
            );
            e.set_store(
                "_kwargs",
                if arguments.is_some() {
                    kwargs
                } else {
                    Value::None
                },
            );
            Ok(Flow::Jump(t))
        }
        NodeKind::Return { expression } => {
            let v = opt_expr(e, id, 0, expression)?;
            e.set_store("_return", v);
            let r = {
                let mut cs = e.st.contexts.borrow_mut();
                let c = cs.last_mut().unwrap();
                c.call_location_stack.pop();
                c.return_stack.pop()
            };
            e.pop_dynamic()?;
            match r {
                Some(site) => Ok(Flow::Next(site)),
                None => Ok(Flow::Halt),
            }
        }
        NodeKind::Jump {
            target, expression, ..
        } => {
            let t = if *expression {
                let f = node.filename.clone();
                let v = e.eval_cached(id, 0, &PyExpr::new(target.as_str(), f, node.linenumber))?;
                ops::str(&mut e.vm, &v)?
            } else {
                target.clone()
            };
            Ok(Flow::Jump(e.lookup_label(&t)?))
        }
        NodeKind::If { entries } => {
            for (i, (cond, block)) in entries.iter().enumerate() {
                let v = e.eval_cached(id, i as u8, cond)?;
                if ops::truthy(&mut e.vm, &v)? {
                    return Ok(Flow::Next(block.first().copied().or(next)));
                }
            }
            Ok(Flow::Next(next))
        }
        NodeKind::While { condition, block } => {
            let v = e.eval_cached(id, 0, condition)?;
            if ops::truthy(&mut e.vm, &v)? {
                return Ok(Flow::Next(block.first().copied().or(Some(id))));
            }
            Ok(Flow::Next(next))
        }
        NodeKind::Menu {
            items,
            set,
            with_,
            has_caption,
            arguments,
        } => {
            let mut list = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let label = s(e, &it.label);
                let cond = e.eval_cached(id, 20 + i as u8, &it.condition)?;
                let ok = ops::truthy(&mut e.vm, &cond)?;
                let val = if it.block.is_some() {
                    Value::Int(i as i64)
                } else {
                    Value::None
                };
                list.push(e.vm.types.new_tuple(vec![label, Value::Bool(ok), val]));
            }
            let l = e.vm.types.new_list(list);
            let st = opt_expr(e, id, 0, set)?;
            let w = opt_expr(e, id, 1, with_)?;
            let (args, kwargs) = eval_arguments(e, id, arguments)?;
            task(
                e,
                id,
                "_stmt_menu",
                vec![l, st, w, Value::Bool(*has_caption), args, kwargs],
                next,
            )
        }
        NodeKind::UserStatement {
            line,
            parsed,
            block,
            ..
        } => {
            let p = match parsed {
                Some(p) => p.clone(),
                None => crate::statements::parse(&mut e.vm, node, line, block.clone())?,
            };
            task(e, id, "_stmt_user", vec![p], next)
        }
        NodeKind::PostUserStatement { parent } => {
            let p = match &e.st.script.borrow().arena.get(*parent).kind {
                NodeKind::UserStatement { parsed, .. } => parsed.clone().unwrap_or(Value::None),
                _ => Value::None,
            };
            task(e, id, "_stmt_post_user", vec![p], next)
        }
        NodeKind::Define {
            store,
            name,
            index,
            operator,
            code,
        } => {
            e.ensure_definition_store(store);
            let v = e.eval_cached(id, 0, &code.source)?;
            let idx = opt_expr(e, id, 1, index)?;
            let st = s(e, store);
            let n = s(e, name);
            let op = s(e, operator);
            let has_index = Value::Bool(index.is_some());
            task(
                e,
                id,
                "_stmt_define",
                vec![st, n, idx, op, v, has_index],
                next,
            )
        }
        NodeKind::Default { store, name, code } => {
            e.ensure_definition_store(store);
            let st = s(e, store);
            let n = s(e, name);
            // Defaults run at game start/load (`execute_default`).
            let f = node.filename.clone();
            let src = s(e, &code.source.text);
            let fv = s(e, &f);
            task(
                e,
                id,
                "_stmt_default",
                vec![st, n, src, fv, Value::Int(code.source.line as i64)],
                next,
            )
        }
        NodeKind::Screen { screen } => {
            crate::screens::define(&mut e.vm, screen.clone())?;
            Ok(Flow::Next(next))
        }
        NodeKind::Style(style) => {
            crate::style::apply_statement(e, id, style)?;
            Ok(Flow::Next(next))
        }
        NodeKind::Translate {
            identifier, block, ..
        } => {
            let lang = task_value(e, "_translate_language")?;
            if !lang.is_none() {
                let l = ops::str(&mut e.vm, &lang)?;
                let t =
                    e.st.script
                        .borrow()
                        .language_translates
                        .get(&(identifier.clone(), l))
                        .copied();
                if let Some(t) = t {
                    let b = match &e.st.script.borrow().arena.get(t).kind {
                        NodeKind::Translate { block, .. } => block.first().copied(),
                        _ => None,
                    };
                    if let Some(b) = b {
                        return Ok(Flow::Next(Some(b)));
                    }
                }
            }
            let mut cs = e.st.contexts.borrow_mut();
            if let Some(c) = cs.last_mut() {
                c.translate_identifier = Some(identifier.clone());
            }
            drop(cs);
            Ok(Flow::Next(block.first().copied().or(next)))
        }
        NodeKind::TranslateSay {
            say, identifier, ..
        } => {
            {
                let mut cs = e.st.contexts.borrow_mut();
                if let Some(c) = cs.last_mut() {
                    c.translate_identifier = Some(identifier.clone());
                }
            }
            let n = Node {
                kind: (**say).clone(),
                ..node.clone()
            };
            execute(e, id, &n)
        }
        NodeKind::EndTranslate => {
            let mut cs = e.st.contexts.borrow_mut();
            if let Some(c) = cs.last_mut() {
                c.translate_identifier = None;
            }
            Ok(Flow::Next(next))
        }
        NodeKind::TranslateString { language, old, new } => {
            let l = opt_str(e, language);
            let o = s(e, old);
            let n = s(e, new);
            task(e, id, "_stmt_translate_string", vec![l, o, n], next)
        }
        NodeKind::TranslatePython { code, .. } => {
            let c = e.compile_code(id, code)?;
            let g = e.store_dict("store");
            match e.vm.start_code_task(c, g, None) {
                TaskOutcome::Done(_) => Ok(Flow::Next(next)),
                TaskOutcome::Error(exc) => Err(ExecError::Python(PyErr::Exc(exc))),
                TaskOutcome::Suspended(..) => {
                    Err(ExecError::Msg("interaction in translate python".into()))
                }
            }
        }
        NodeKind::TranslateBlock { block, .. } => Ok(Flow::Next(block.first().copied().or(next))),
    }
}

fn bind_label(e: &mut Engine, node: &Node, signature: Option<&Signature>) -> Result<(), ExecError> {
    let store = e.store_dict("store");
    let args = store
        .dict_get_str_raw("_args")
        .filter(|value| !value.is_none());
    let args = args
        .as_ref()
        .map_or(Ok(Vec::new()), |args| ops::collect(&mut e.vm, args))?;
    let kwargs = store
        .dict_get_str_raw("_kwargs")
        .filter(|value| !value.is_none());
    let kwargs = kwargs
        .as_ref()
        .map_or(Ok(Vec::new()), |args| ops::mapping_items(&mut e.vm, args))?;
    let Some(signature) = signature else {
        return if args.is_empty() && kwargs.is_empty() {
            Ok(())
        } else {
            Err(ExecError::Msg(
                "arguments supplied to a label without parameters".into(),
            ))
        };
    };
    let mut words = std::collections::BTreeMap::new();
    for (key, value) in kwargs {
        let name = key
            .as_str()
            .ok_or_else(|| e.vm.type_error("label keyword must be a string"))?;
        words.insert(name.to_owned(), value);
    }
    let mut rest = args.into_iter();
    let mut bindings = Vec::new();
    for parameter in &signature.parameters {
        let value = match parameter.kind {
            0..=1 => {
                let by_position = rest.next();
                let by_name = if parameter.kind == 1 {
                    words.remove(&parameter.name)
                } else {
                    None
                };
                match (by_position, by_name) {
                    (Some(_), Some(_)) => {
                        return Err(ExecError::Msg(format!(
                            "parameter {} has two values",
                            parameter.name
                        )));
                    }
                    (value, named) => value.or(named),
                }
            }
            2 => Some(e.vm.types.new_tuple(rest.by_ref().collect())),
            3 => words.remove(&parameter.name),
            4 => {
                let value = e.vm.types.new_dict();
                for (key, argument) in std::mem::take(&mut words) {
                    let key = e.vm.types.new_str(&key);
                    ops::setitem(&mut e.vm, &value, key, argument)?;
                }
                Some(value)
            }
            _ => return Err(ExecError::Msg("unknown label parameter kind".into())),
        };
        let value = match (value, &parameter.default) {
            (Some(value), _) => value,
            (None, Some(default)) => e.eval(&PyExpr::new(
                default.as_str(),
                node.filename.clone(),
                node.linenumber,
            ))?,
            (None, None) => {
                return Err(ExecError::Msg(format!(
                    "required parameter {} has no value",
                    parameter.name
                )));
            }
        };
        bindings.push((parameter.name.clone(), value));
    }
    if rest.next().is_some() {
        return Err(ExecError::Msg("too many positional label arguments".into()));
    }
    if !words.is_empty() {
        return Err(ExecError::Msg(format!(
            "unknown label arguments: {}",
            words.keys().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    {
        let mut contexts = e.st.contexts.borrow_mut();
        let scope = contexts
            .last_mut()
            .and_then(|c| c.dynamic_stack.last_mut())
            .ok_or_else(|| ExecError::Msg("no dynamic scope for label".into()))?;
        for (name, value) in bindings {
            if !scope.iter().any(|(key, _)| key == &name) {
                scope.push((name.clone(), store.dict_get_str_raw(&name)));
            }
            store.dict_set_raw(&e.vm.types, e.vm.types.new_str(&name), value);
        }
    }
    e.set_store("_args", Value::None);
    e.set_store("_kwargs", Value::None);
    Ok(())
}

fn task_value(e: &mut Engine, name: &str) -> Result<Value, ExecError> {
    let m = crysirror_python::import::import_module(&mut e.vm, "renpy")?;
    let f = ops::getattr(&mut e.vm, &m, name)?;
    Ok(e.vm.call0(&f)?)
}

impl Engine {
    pub fn compile_code(
        &mut self,
        id: NodeId,
        code: &PyCode,
    ) -> Result<Rc<crysirror_python::code::Code>, ExecError> {
        if let Some(c) = self.st.code_cache.borrow().get(&(id, 200)) {
            return Ok(c.clone());
        }
        let d = if code.py3 {
            crysirror_python::Dialect::Py3
        } else {
            self.st.profile.dialect
        };
        let c = crate::pycompile::compile(&self.vm, &code.source, code.mode, d)
            .map_err(ExecError::Msg)?;
        self.st.code_cache.borrow_mut().insert((id, 200), c.clone());
        Ok(c)
    }

    /// A Python statement may have redirected the next node (renpy.jump is
    /// an exception; `next_node` is set by user statements).
    pub fn next_node(&self, default: Option<NodeId>) -> Option<NodeId> {
        self.st
            .contexts
            .borrow()
            .last()
            .and_then(|c| c.next_node)
            .or(default)
    }

    pub fn pop_dynamic(&mut self) -> Result<(), ExecError> {
        let scope = {
            let mut cs = self.st.contexts.borrow_mut();
            cs.last_mut().and_then(|c| {
                if c.dynamic_stack.len() > 1 {
                    c.dynamic_stack.pop()
                } else {
                    None
                }
            })
        };
        if let Some(scope) = scope {
            restore_scope(self, scope.into_iter().rev());
        }
        Ok(())
    }
}

pub(crate) fn restore_scope(
    engine: &mut Engine,
    scope: impl IntoIterator<Item = (String, Option<Value>)>,
) {
    let store = engine.store_dict("store");
    for (key, value) in scope {
        if let Some(value) = value {
            store.dict_set_raw(&engine.vm.types, engine.vm.types.new_str(&key), value);
        } else if let crysirror_python::Payload::Dict(dict) = &store.payload {
            dict.borrow_mut().del_str(&key);
        }
    }
}
