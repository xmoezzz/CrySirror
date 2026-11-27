// SPDX-License-Identifier: MPL-2.0
//! Evaluates native screen expressions and supplies shared UI widgets and actions.
use crate::{
    ast::ArgumentInfo,
    engine::{EResult, Engine, Interaction, Request, state},
    lexer::PyExpr,
    sl2::{Block, Displayable, Screen, SlNode},
};
use crysirror_python::{PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, rc::Rc};

#[derive(Clone)]
pub struct Instance {
    pub name: String,
    pub tag: String,
    pub args: Vec<Value>,
    pub kwargs: Vec<(Value, Value)>,
    pub transient: bool,
    pub layer: String,
}

pub fn show(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    show_impl(vm, args, false)
}

pub fn call(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    show_impl(vm, args, true)?;
    let name = ops::str(vm, &args[0])?;
    state(vm).interaction.replace(Some(Interaction::Screen {
        name,
        timeout_ms: None,
        suppress_overlay: false,
    }));
    Err(PyErr::Suspend(Box::new(Request::Interact(
        Default::default(),
    ))))
}

pub fn interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [
        overlay,
        underlay,
        mouse,
        interaction_type,
        roll_forward,
        pause,
    ] = args
    else {
        return Err(vm.type_error("invalid UI interaction arguments"));
    };
    let suppress_overlay = ops::truthy(vm, overlay)?;
    let suppress_underlay = ops::truthy(vm, underlay)?;
    let pause = match pause {
        Value::None => None,
        Value::Float(value) if value.is_finite() && *value >= 0. => Some(*value),
        Value::Int(value) if *value >= 0 => Some(*value as f64),
        _ => return Err(vm.type_error("invalid UI interaction delay")),
    };
    let st = state(vm);
    let name = st
        .contexts
        .borrow()
        .last()
        .and_then(|context| context.screens.last())
        .map(|screen| screen.name.clone())
        .unwrap_or_default();
    st.interaction.replace(Some(Interaction::Screen {
        name,
        timeout_ms: pause.map(|value| (1000. * value).ceil() as u64),
        suppress_overlay,
    }));
    Err(PyErr::Suspend(Box::new(Request::Interact(
        crate::engine::InteractRequest {
            suppress_overlay,
            suppress_underlay,
            mouse: mouse.as_str().map(str::to_owned),
            type_: interaction_type.as_str().map(str::to_owned),
            roll_forward: roll_forward.clone(),
            pause,
        },
    ))))
}

fn show_impl(vm: &mut Vm, args: &[Value], transient: bool) -> PyResult<Value> {
    let [name, args, kwargs] = args else {
        return Err(vm.type_error("invalid screen arguments"));
    };
    let name = ops::str(vm, name)?;
    let st = state(vm);
    let default_tag = st
        .screens
        .borrow()
        .get(&name)
        .map(|screen| screen.tag.clone().unwrap_or_else(|| name.clone()))
        .ok_or_else(|| vm.exc_msg("LookupError", format!("screen {name} is not defined")))?;
    let mut arguments = ops::mapping_items(vm, kwargs)?;
    let mut tag = default_tag;
    let mut transient = transient;
    let mut layer = "screens".to_owned();
    for (key, value) in &arguments {
        match key.as_str() {
            Some("_transient") => transient |= ops::truthy(vm, value)?,
            Some("_tag") if !value.is_none() => tag = ops::str(vm, value)?,
            Some("_layer") if !value.is_none() => layer = ops::str(vm, value)?,
            _ => {}
        }
    }
    arguments.retain(|(key, _)| !matches!(key.as_str(), Some("_tag" | "_transient" | "_layer")));
    let instance = Instance {
        name,
        tag,
        args: ops::collect(vm, args)?,
        kwargs: arguments,
        transient,
        layer,
    };
    let mut contexts = st.contexts.borrow_mut();
    let active = &mut contexts
        .last_mut()
        .ok_or_else(|| vm.type_error("screen without a context"))?
        .screens;
    active.retain(|screen| screen.tag != instance.tag);
    active.push(instance);
    Ok(Value::None)
}

pub fn hide(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name] = args else {
        return Err(vm.type_error("hide_screen expects a name"));
    };
    let name = ops::str(vm, name)?;
    if let Some(context) = state(vm).contexts.borrow_mut().last_mut() {
        context
            .screens
            .retain(|screen| screen.name != name && screen.tag != name);
    }
    Ok(Value::None)
}

#[derive(Default)]
pub struct Frame {
    pub widgets: Vec<Json>,
    pub actions: BTreeMap<u32, Value>,
}

pub fn frame(engine: &mut Engine, canvas: [i32; 2], suppress_overlay: bool) -> EResult<Frame> {
    let instances = engine
        .st
        .contexts
        .borrow()
        .last()
        .map(|context| context.screens.clone())
        .unwrap_or_default();
    let mut builder = Builder {
        engine,
        frame: Frame::default(),
        budget: 8192,
    };
    let size = [canvas[0] as f32, canvas[1] as f32];
    for instance in instances {
        if suppress_overlay && instance.layer == "overlay" {
            continue;
        }
        let screen = builder.screen(&instance.name)?;
        let scope = builder.scope(&screen, instance.args, instance.kwargs)?;
        let eval = |text: &str| PyExpr::new(text, screen.loc.0.clone(), screen.loc.1);
        let root = builder.id()?;
        let modal_value = builder.eval(&eval(&screen.modal), &scope)?;
        let modal = builder.truth(&modal_value)?;
        let zorder = match builder.eval(&eval(&screen.zorder), &scope)? {
            Value::Int(zorder) => i32::try_from(zorder).map_err(|_| "Ren'Py screen zorder")?,
            _ => return Err("Ren'Py screen zorder must be an integer".into()),
        };
        builder.frame.widgets.push(
            json!({"id":root,"rect":[0.0,0.0,size[0],size[1]],"kind":"panel",
            "interaction_only":true,"modal":modal,"z":zorder}),
        );
        builder.block(&screen.block, &scope, root, size, 0, None)?;
    }
    Ok(builder.frame)
}

struct Builder<'a> {
    engine: &'a mut Engine,
    frame: Frame,
    budget: usize,
}

struct Transclusion<'a> {
    block: &'a Block,
    scope: &'a Value,
    inherited: Option<&'a Transclusion<'a>>,
}

impl Builder<'_> {
    fn id(&self) -> EResult<u32> {
        let count = self.frame.widgets.len();
        if count >= 1024 {
            return Err("Ren'Py screen widget limit".into());
        }
        Ok(16384 + count as u32)
    }

    fn screen(&self, name: &str) -> EResult<Rc<Screen>> {
        self.engine
            .st
            .screens
            .borrow()
            .get(name)
            .cloned()
            .ok_or_else(|| format!("screen {name} is not defined"))
    }

    fn eval(&mut self, expression: &PyExpr, scope: &Value) -> EResult<Value> {
        self.run(expression, "eval", scope)
    }

    fn run(&mut self, expression: &PyExpr, mode: &str, scope: &Value) -> EResult<Value> {
        let dialect = self.engine.st.profile.dialect;
        let code = crate::pycompile::compile(&self.engine.vm, expression, mode, dialect)?;
        let globals = self.engine.store_dict("store");
        self.engine
            .vm
            .run_code(code, globals, Some(scope.clone()))
            .map_err(|e| self.engine.py_err(e))
    }

    fn truth(&mut self, value: &Value) -> EResult<bool> {
        ops::truthy(&mut self.engine.vm, value).map_err(|e| self.engine.py_err(e))
    }

    fn items(&mut self, value: &Value) -> EResult<Vec<Value>> {
        ops::collect(&mut self.engine.vm, value).map_err(|e| self.engine.py_err(e))
    }

    fn write(&mut self, scope: &Value, name: &str, value: Value) {
        let vm = &self.engine.vm;
        scope.obj().expect("screen scope dict").dict_set_raw(
            &vm.types,
            vm.types.new_str(name),
            value,
        );
    }

    fn scope(
        &mut self,
        screen: &Screen,
        args: Vec<Value>,
        kwargs: Vec<(Value, Value)>,
    ) -> EResult<Value> {
        let Some(signature) = &screen.parameters else {
            return if args.is_empty() && kwargs.is_empty() {
                Ok(self.engine.vm.types.new_dict())
            } else {
                Err(format!("screen {} has no parameters", screen.name))
            };
        };
        let kwargs = kwargs
            .into_iter()
            .map(|(key, value)| {
                key.as_str()
                    .map(|key| (Rc::from(key), value))
                    .ok_or("screen keyword is not a string")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let globals = self.engine.store_dict("store");
        crate::pycompile::bind_parameters(
            &mut self.engine.vm,
            signature,
            &globals,
            &screen.loc,
            &args,
            &kwargs,
        )
        .map_err(|err| self.engine.py_err(err))
    }

    fn arguments(
        &mut self,
        arguments: Option<&ArgumentInfo>,
        scope: &Value,
        loc: &(Rc<str>, u32),
    ) -> EResult<(Vec<Value>, Vec<(Value, Value)>)> {
        let mut args = Vec::new();
        let mut kwargs = Vec::new();
        if let Some(arguments) = arguments {
            for (index, (name, code)) in arguments.arguments.iter().enumerate() {
                let value = self.eval(&PyExpr::new(code.as_str(), loc.0.clone(), loc.1), scope)?;
                if arguments.starred.contains(&index) {
                    args.extend(self.items(&value)?);
                } else if arguments.doublestarred.contains(&index) {
                    kwargs.extend(
                        ops::mapping_items(&mut self.engine.vm, &value)
                            .map_err(|e| self.engine.py_err(e))?,
                    );
                } else if let Some(name) = name {
                    kwargs.push((self.engine.vm.types.new_str(name), value));
                } else {
                    args.push(value);
                }
            }
        }
        Ok((args, kwargs))
    }

    fn block(
        &mut self,
        block: &Block,
        scope: &Value,
        parent: u32,
        size: [f32; 2],
        depth: usize,
        transclusion: Option<&Transclusion<'_>>,
    ) -> EResult<()> {
        if depth >= 32 {
            return Err("Ren'Py screen nesting limit".into());
        }
        for node in &block.children {
            self.budget = self
                .budget
                .checked_sub(1)
                .ok_or("Ren'Py screen execution limit")?;
            match node {
                SlNode::Displayable(displayable) => {
                    self.displayable(displayable, scope, parent, size, depth + 1, transclusion)?
                }
                SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => {
                    for (condition, body) in entries {
                        if let Some(condition) = condition {
                            let value = self.eval(condition, scope)?;
                            if !self.truth(&value)? {
                                continue;
                            }
                        }
                        self.block(body, scope, parent, size, depth + 1, transclusion)?;
                        break;
                    }
                }
                SlNode::For {
                    variable,
                    expression,
                    block,
                    ..
                } => {
                    let value = self.eval(expression, scope)?;
                    for entry in self.items(&value)? {
                        self.budget = self
                            .budget
                            .checked_sub(1)
                            .ok_or("Ren'Py screen iteration limit")?;
                        self.write(scope, variable, entry);
                        self.block(block, scope, parent, size, depth + 1, transclusion)?;
                    }
                }
                SlNode::Python { code, .. } => {
                    self.run(code, "exec", scope)?;
                }
                SlNode::Default {
                    variable,
                    expression,
                    ..
                } => {
                    if scope
                        .obj()
                        .and_then(|object| object.dict_get_str_raw(variable))
                        .is_none()
                    {
                        let value = self.eval(expression, scope)?;
                        self.write(scope, variable, value);
                    }
                }
                SlNode::Use {
                    target,
                    expression,
                    args,
                    block,
                    loc,
                    variable: None,
                    id: None,
                    ..
                } => {
                    let target = if *expression {
                        let expr = PyExpr::new(target.as_str(), loc.0.clone(), loc.1);
                        let value = self.eval(&expr, scope)?;
                        value
                            .as_str()
                            .ok_or("Ren'Py screen name must be a string")?
                            .to_owned()
                    } else {
                        target.to_owned()
                    };
                    let screen = self.screen(&target)?;
                    let (args, kwargs) = self.arguments(args.as_ref(), scope, loc)?;
                    let nested = if screen.parameters.is_none() {
                        if !args.is_empty() {
                            return Err(format!("screen {target} has no positional parameters"));
                        }
                        let nested = self.engine.vm.types.new_dict();
                        let values = ops::mapping_items(&mut self.engine.vm, scope)
                            .map_err(|error| self.engine.py_err(error))?;
                        for (key, value) in values.into_iter().chain(kwargs) {
                            nested
                                .obj()
                                .unwrap()
                                .dict_set_raw(&self.engine.vm.types, key, value);
                        }
                        nested
                    } else {
                        self.scope(&screen, args, kwargs)?
                    };
                    let content = block.as_ref().map(|block| Transclusion {
                        block,
                        scope,
                        inherited: transclusion,
                    });
                    self.block(
                        &screen.block,
                        &nested,
                        parent,
                        size,
                        depth + 1,
                        content.as_ref(),
                    )?;
                }
                SlNode::Transclude { .. } => {
                    if let Some(content) = transclusion {
                        self.block(
                            content.block,
                            content.scope,
                            parent,
                            size,
                            depth + 1,
                            content.inherited,
                        )?;
                    }
                }
                SlNode::Pass { .. } => {}
                _ => return Err(format!("unsupported Ren'Py screen node {node:?}")),
            }
        }
        Ok(())
    }

    fn displayable(
        &mut self,
        node: &Displayable,
        scope: &Value,
        parent: u32,
        size: [f32; 2],
        depth: usize,
        transclusion: Option<&Transclusion<'_>>,
    ) -> EResult<()> {
        let kind = node.statement.as_str();
        if node.block.atl_transform.is_some() {
            return Err("ATL inside a Ren'Py screen".into());
        }
        let mut properties = BTreeMap::new();
        for (key, expression) in &node.block.keyword {
            properties.insert(key.as_str(), self.eval(expression, scope)?);
        }
        let label = matches!(kind, "text" | "textbutton");
        let button = matches!(kind, "button" | "textbutton");
        if let Some(style) = properties.get("style").filter(|style| !style.is_none()) {
            let style = match style.as_str() {
                Some(name) => crate::style::get_style(&mut self.engine.vm, name)
                    .map_err(|error| self.engine.py_err(error))?,
                None if crate::style::is_style(style) => style.clone(),
                None => return Err("invalid Ren'Py screen style".into()),
            };
            for key in [
                "size",
                "color",
                "background",
                "xpos",
                "ypos",
                "xanchor",
                "yanchor",
                "xoffset",
                "yoffset",
                "xmaximum",
                "ymaximum",
                "spacing",
            ] {
                if button && matches!(key, "size" | "color") {
                    continue;
                }
                if let std::collections::btree_map::Entry::Vacant(entry) = properties.entry(key) {
                    let value = crate::style::get_prop(
                        &mut self.engine.vm,
                        &style,
                        crate::style::prop_index(key).unwrap(),
                        1,
                    )
                    .map_err(|error| self.engine.py_err(error))?;
                    if !value.is_none() {
                        entry.insert(value);
                    }
                }
            }
        }
        let id = self.id()?;
        let param = |key| properties.get(key);
        let scalar = |key, extent, default| {
            param(key)
                .and_then(|v| offset(v, extent))
                .unwrap_or(default)
        };
        let flow = match kind {
            "vbox" => "column",
            "hbox" => "row",
            _ => "absolute",
        };
        if !matches!(
            kind,
            "text"
                | "textbutton"
                | "button"
                | "frame"
                | "window"
                | "fixed"
                | "hbox"
                | "vbox"
                | "null"
        ) {
            return Err(format!(
                "unsupported Ren'Py screen displayable {kind} at {}:{}",
                node.loc.0, node.loc.1
            ));
        }
        let text = if label {
            let expr = node.positional.first().ok_or("missing screen text")?;
            let value = self.eval(expr, scope)?;
            ops::str(&mut self.engine.vm, &value).map_err(|e| self.engine.py_err(e))?
        } else {
            String::new()
        };
        let font =
            scalar(if button { "text_size" } else { "size" }, 1.0, 24.).clamp(1., 512.) as u16;
        let w = scalar("xsize", size[0], scalar("xmaximum", size[0], size[0]));
        let h = scalar(
            "ysize",
            size[1],
            scalar(
                "ymaximum",
                size[1],
                if label || button {
                    f32::from(font) + 12.
                } else {
                    size[1]
                },
            ),
        );
        let xalign = param("xalign");
        let yalign = param("yalign");
        let x = scalar(
            "xpos",
            size[0],
            xalign.and_then(|v| offset(v, size[0])).unwrap_or(0.),
        ) - scalar(
            "xanchor",
            w,
            xalign.and_then(|v| offset(v, w)).unwrap_or(0.),
        ) + scalar("xoffset", 1., 0.);
        let y = scalar(
            "ypos",
            size[1],
            yalign.and_then(|v| offset(v, size[1])).unwrap_or(0.),
        ) - scalar(
            "yanchor",
            h,
            yalign.and_then(|v| offset(v, h)).unwrap_or(0.),
        ) + scalar("yoffset", 1., 0.);
        let mut enabled = true;
        if let Some(sensitive) = param("sensitive") {
            enabled = self.truth(sensitive)?;
        }
        if let Some(action) = param("action").filter(|action| !action.is_none()) {
            let enabled_action = self
                .engine
                .call_glue("_screen_action_sensitive", vec![action.clone()])?;
            enabled &= self.truth(&enabled_action)?;
            self.frame.actions.insert(id, action.clone());
        } else if button {
            enabled = false;
        }
        let color = match param("background").filter(|value| !value.is_none()) {
            Some(value) => crate::display::solid_background(value).map_err(|e| e.to_string())?,
            None if button => [44, 40, 58, 230],
            None => [0; 4],
        };
        let text_color = match param(if button { "text_color" } else { "color" }) {
            Some(value) => crate::display::color_value(value).map_err(|e| e.to_string())?,
            None => [255; 4],
        };
        let padding = scalar("padding", 1., 0.).clamp(0., 1024.) as u16;
        let gap = scalar("spacing", 1., 0.).clamp(0., 1024.) as u16;
        self.frame.widgets.push(
            json!({"id":id,"parent":parent,"rect":[x,y,w.max(1.),h.max(1.)],
            "kind":if button {"button"} else if label {"label"} else {"panel"},
            "flow":flow,"text":text,"text_size":font,"font":"default","text_color":text_color,
            "color":color,"enabled":enabled,"padding":padding,"gap":gap,
            "interaction_only":!label && !button && color[3] == 0}),
        );
        self.block(&node.block, scope, id, [w, h], depth, transclusion)
    }
}

fn offset(value: &Value, extent: f32) -> Option<f32> {
    let scalar = match value {
        Value::Int(i) => *i as f32,
        Value::Float(f) => *f as f32 * extent,
        _ => return None,
    };
    scalar.is_finite().then_some(scalar)
}
