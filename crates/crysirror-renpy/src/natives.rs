// SPDX-License-Identifier: MPL-2.0
//! Native boundaries of the workspace-owned Ren'Py module.
use crate::{
    ast::*,
    engine::{Interaction, Request, State, state},
    lexer::{Lexer, PResult},
    scene::Image,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, modules, ops};
use std::rc::Rc;

pub fn install(vm: &mut Vm) {
    let transform_class = vm.types.new_builtin_type(
        "ATLTransform",
        &vm.types.object,
        crysirror_python::object::BaseKind::Native,
        true,
    );
    vm.types.add_method(&transform_class, "__repr__", atl_repr);
    vm.types.add_method(&transform_class, "__call__", atl_call);
    vm.ext_set(AtlClass(transform_class));
    crate::statements::install(vm);
    vm.register_native_module("_renpy", init_module);
    vm.register_native_module("_renpy_style", crate::style::init_module);
}

fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(modules::module_with(
        vm,
        "_renpy",
        &[
            ("loadable", loadable),
            ("write_log", write_log),
            ("save_persistent", crate::persistent::save),
            ("list_files", list_files),
            ("has_image", has_image),
            ("showing", showing),
            ("label_entry", label_entry),
            ("context", context),
            ("has_label", has_label),
            ("dynamic", dynamic),
            ("context_dynamic", context_dynamic),
            ("call_in_new_context", call_in_new_context),
            ("in_rollback", |vm, _, _| {
                Ok(Value::Bool(state(vm).in_rollback.get()))
            }),
            ("rollback_available", |vm, _, _| {
                Ok(Value::Int(state(vm).rollback_available.get() as i64))
            }),
            ("block_rollback", |vm, _, _| {
                state(vm).rollback_blocked.set(true);
                Ok(Value::None)
            }),
            ("suspend_rollback", suspend_rollback),
            ("roll_forward_info", |vm, _, _| {
                Ok(state(vm).roll_forward.borrow().clone())
            }),
            ("has_screen", has_screen),
            ("show_screen", crate::screen_runtime::show),
            ("hide_screen", crate::screen_runtime::hide),
            ("call_screen", crate::screen_runtime::call),
            ("interact", crate::screen_runtime::interact),
            ("restart_interaction", |vm, _, _| {
                state(vm).screen_refresh.set(true);
                Ok(Value::None)
            }),
            ("register_sl", crate::screens::register),
            ("add_sl_positional", crate::screens::add_positional),
            ("say", say),
            ("in_init", crate::audio::in_init),
            ("audio_play", crate::audio::play),
            ("audio_current", crate::audio::current),
            ("audio_stop", crate::audio::stop),
            ("menu", menu),
            ("pause", pause),
            ("dissolve", dissolve),
            ("register_statement", crate::statements::register),
            ("atl_warper", atl_warper),
            ("image", define_image),
            ("show", show),
            ("hide", hide),
            ("scene", clear),
        ],
    ))
}

fn has_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [label] = args else {
        return Err(vm.type_error("has_label expects one label"));
    };
    let label = ops::str(vm, label)?;
    Ok(Value::Bool(
        state(vm).script.borrow().lookup_label(&label).is_some(),
    ))
}

fn showing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name, layer] = args else {
        return Err(vm.type_error("invalid showing arguments"));
    };
    let name = names(vm, name)?;
    let Some(tag) = name.first() else {
        return Ok(Value::Bool(false));
    };
    let st = state(vm);
    let contexts = st.contexts.borrow();
    let images = contexts
        .last()
        .and_then(|context| context.scene.layers.get(layer.as_str().unwrap_or("master")));
    Ok(Value::Bool(images.is_some_and(|images| {
        images
            .iter()
            .any(|image| image.tag == *tag && (name.len() == 1 || image.name.starts_with(&name)))
    })))
}

fn dynamic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    retain_bindings(vm, args, false)
}

fn context_dynamic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    retain_bindings(vm, args, true)
}

fn retain_bindings(vm: &mut Vm, args: &[Value], context: bool) -> PyResult<Value> {
    let st = state(vm);
    let store = crate::engine::module_namespace(vm, "store")
        .ok_or_else(|| vm.type_error("dynamic without a store"))?;
    let mut contexts = st.contexts.borrow_mut();
    let scope = contexts
        .last_mut()
        .and_then(|active| {
            if context {
                active.dynamic_stack.first_mut()
            } else {
                active.dynamic_stack.last_mut()
            }
        })
        .ok_or_else(|| vm.type_error("dynamic without a context"))?;
    for arg in args {
        let name = arg
            .as_str()
            .ok_or_else(|| vm.type_error("dynamic requires variable names"))?;
        if !scope.iter().any(|(variable, _)| variable == name) {
            scope.push((name.to_owned(), store.dict_get_str_raw(name)));
        }
    }
    Ok(Value::None)
}

fn call_in_new_context(vm: &mut Vm, args: &[Value], kwargs: Kw) -> PyResult<Value> {
    let Some((label, args)) = args.split_first() else {
        return Err(vm.type_error("call_in_new_context expects a label"));
    };
    let label = ops::str(vm, label)?;
    Err(crysirror_python::PyErr::Suspend(Box::new(
        Request::CallInNewContext {
            label,
            args: args.to_vec(),
            kwargs: kwargs.to_vec(),
        },
    )))
}

fn suspend_rollback(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [value] = args else {
        return Err(vm.type_error("suspend_rollback expects a flag"));
    };
    let suspend = ops::truthy(vm, value)?;
    state(vm).rollback_suspended.set(suspend);
    Ok(Value::None)
}

fn context(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let st = state(vm);
    let contexts = st.contexts.borrow();
    contexts
        .last()
        .map(|context| context.py.clone())
        .ok_or_else(|| vm.type_error("no Ren'Py context"))
}

fn has_screen(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name] = args else {
        return Err(vm.type_error("has_screen expects a screen name"));
    };
    let parts = names(vm, name)?;
    let st = state(vm);
    let present = parts
        .first()
        .is_some_and(|name| st.screens.borrow().contains_key(name));
    Ok(Value::Bool(present))
}

fn label_entry(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let state = state(vm);
    let contexts = state.contexts.borrow();
    let context = contexts
        .last()
        .ok_or_else(|| vm.type_error("label without a context"))?;
    Ok(Value::Bool(context.last_abnormal))
}

fn loadable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [value] = args else {
        return Err(vm.type_error("loadable expects a filename"));
    };
    let name = value
        .as_str()
        .ok_or_else(|| vm.type_error("loadable expects a filename"))?;
    Ok(Value::Bool(state(vm).files.exists(name)))
}

fn write_log(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [message] = args else {
        return Err(vm.type_error("write_log expects a message"));
    };
    let message = ops::str(vm, message)?;
    let st = state(vm);
    let mut log = st.log.borrow_mut();
    log.push(message);
    if log.len() > 4096 {
        log.drain(..2048);
    }
    Ok(Value::None)
}

fn list_files(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [common] = args else {
        return Err(vm.type_error("invalid list_files arguments"));
    };
    let include_common = ops::truthy(vm, common)?;
    let st = state(vm);
    let mut names = st.files.list();
    if include_common {
        names.extend(st.files.common_list());
    }
    names.sort_unstable();
    names.dedup();
    Ok(vm
        .types
        .new_list(names.iter().map(|name| vm.types.new_str(name)).collect()))
}

fn has_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name, exact] = args else {
        return Err(vm.type_error("invalid has_image arguments"));
    };
    let mut name = names(vm, name)?;
    let exact = ops::truthy(vm, exact)?;
    let st = state(vm);
    let images = st.images.borrow();
    while !name.is_empty() {
        if images.contains_key(&name) {
            return Ok(Value::Bool(true));
        }
        if exact {
            break;
        }
        name.pop();
    }
    Ok(Value::Bool(false))
}

fn atl_warper(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [function] = args else {
        return Err(vm.type_error("atl_warper expects a function"));
    };
    let name = ops::getattr(vm, function, "__name__")?;
    let name = ops::str(vm, &name)?;
    let st = state(vm);
    st.warpers.borrow_mut().insert(name.clone());
    st.warper_functions
        .borrow_mut()
        .insert(name, function.clone());
    Ok(function.clone())
}

fn dissolve(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [value] = args else {
        return Err(vm.type_error("dissolve expects a duration"));
    };
    let seconds = crysirror_python::methods::to_float(vm, value)?;
    if !seconds.is_finite() || !(0.0..=600.0).contains(&seconds) {
        return Err(vm.value_error("invalid dissolve duration"));
    }
    state(vm).interaction.replace(Some(Interaction::Transition {
        duration_ms: (seconds * 1000.).ceil() as u64,
    }));
    Err(crysirror_python::PyErr::Suspend(Box::new(
        Request::Interact(Default::default()),
    )))
}

fn say(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ([speaker, text], screen) = match args {
        [speaker, text] => ([speaker, text], None),
        [speaker, text, screen] => ([speaker, text], Some(ops::str(vm, screen)?)),
        _ => {
            return Err(vm.type_error("say expects a speaker and text"));
        }
    };
    let speaker = if speaker.is_none() {
        None
    } else {
        Some(ops::str(vm, speaker)?)
    };
    let text = ops::str(vm, text)?;
    let message = Interaction::Say {
        speaker,
        text,
        screen,
    };
    state(vm).interaction.replace(Some(message));
    Err(crysirror_python::PyErr::Suspend(Box::new(
        Request::Interact(Default::default()),
    )))
}

fn menu(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [entries] = args else {
        return Err(vm.type_error("menu expects a list of entries"));
    };
    let mut options = Vec::new();
    let mut caption = None;
    for entry in ops::collect(vm, entries)? {
        let fields = ops::collect(vm, &entry)?;
        let [text, enabled, target] = fields.as_slice() else {
            return Err(vm.type_error("invalid menu entry"));
        };
        if !ops::truthy(vm, enabled)? {
            continue;
        }
        let text = ops::str(vm, text)?;
        if target.is_none() {
            caption = Some(text);
        } else {
            let target = match target {
                Value::Int(value) => *value,
                _ => return Err(vm.type_error("invalid menu target")),
            };
            options.push((text, target));
        }
    }
    if options.is_empty() {
        return Ok(Value::None);
    }
    state(vm)
        .interaction
        .replace(Some(Interaction::Menu { caption, options }));
    Err(crysirror_python::PyErr::Suspend(Box::new(
        Request::Interact(Default::default()),
    )))
}

fn pause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [delay, hard] = args else {
        return Err(vm.type_error("invalid pause arguments"));
    };
    let seconds = if delay.is_none() {
        None
    } else {
        Some(crysirror_python::methods::to_float(vm, delay)?)
    };
    let hard = ops::truthy(vm, hard)?;
    if seconds.is_some_and(|s| !s.is_finite() || s < 0.) {
        return Err(vm.type_error("invalid pause duration"));
    }
    state(vm)
        .interaction
        .replace(Some(Interaction::Pause { seconds, hard }));
    Err(crysirror_python::PyErr::Suspend(Box::new(
        Request::Interact(Default::default()),
    )))
}

fn names(vm: &mut Vm, value: &Value) -> PyResult<Vec<String>> {
    if let Some(name) = value.as_str() {
        return Ok(name.split_whitespace().map(str::to_owned).collect());
    }
    ops::collect(vm, value)?
        .iter()
        .map(|part| ops::str(vm, part))
        .collect()
}

fn define_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name, displayable] = args else {
        return Err(vm.type_error("image expects a name and displayable"));
    };
    let name = names(vm, name)?;
    if name.is_empty() || displayable.is_none() {
        return Err(vm.type_error("an image needs a name and displayable"));
    }
    let st = state(vm);
    if !st.contexts.borrow().last().is_some_and(|c| c.init_phase) {
        return Err(vm.exc_msg("Exception", "images can only be defined during init"));
    }
    st.images.borrow_mut().insert(name, displayable.clone());
    Ok(Value::None)
}

fn show(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [
        name,
        displayable,
        tag,
        transforms,
        layer,
        zorder,
        behind,
        inline,
    ] = args
    else {
        return Err(vm.type_error("invalid show arguments"));
    };
    let name = names(vm, name)?;
    let tag = tag
        .as_str()
        .map(str::to_owned)
        .or_else(|| name.first().cloned())
        .ok_or_else(|| vm.type_error("show expression needs a tag"))?;
    let layer = layer.as_str().unwrap_or("master").to_owned();
    let behind = names(vm, behind)?;
    let st = state(vm);
    let mut value = if displayable.is_none() {
        let images = st.images.borrow();
        let Some((length, image)) = (1..=name.len()).rev().find_map(|length| {
            let image = images.get(&name[..length])?;
            let parameterized = image
                .obj()
                .is_some_and(|o| o.class_name() == "ParameterizedText");
            (length == name.len() || parameterized).then(|| (length, image.clone()))
        }) else {
            return Err(vm.exc_msg(
                "LookupError",
                format!("image {} is not defined", name.join(" ")),
            ));
        };
        drop(images);
        if image
            .obj()
            .is_some_and(|o| o.class_name() == "ParameterizedText")
        {
            let arguments = vm
                .types
                .new_tuple(name[length..].iter().map(|s| vm.types.new_str(s)).collect());
            vm.call_method(&image, "parameterize", vec![arguments])?
        } else {
            image
        }
    } else {
        displayable.clone()
    };
    let mut at = Vec::new();
    value = crate::display::resolve_reference(value, &st)
        .map_err(|error| vm.exc_msg("LookupError", error.to_string()))?;
    if let Some(definition) = atl_definition(&value) {
        at.push(definition);
        value = Value::None;
    }
    for transform in ops::collect(vm, transforms)? {
        at.push(atl_definition(&transform).ok_or_else(|| {
            vm.exc_msg("NotImplementedError", "non-ATL function in image at list")
        })?);
    }
    if !inline.is_none() {
        at.push(atl_definition(inline).ok_or_else(|| vm.type_error("invalid inline ATL"))?);
    }
    let start_ms = st.clock_ms.get();
    let transforms = at
        .into_iter()
        .map(|mut definition| {
            let locals = resolve_atl_locals(vm, &definition, &[], &[])?;
            definition.locals = locals;
            let program = crate::atl_runtime::Program::compile(vm, &definition)?;
            Ok(crate::atl_runtime::Instance { program, start_ms })
        })
        .collect::<PyResult<Vec<_>>>()?;
    st.contexts
        .borrow_mut()
        .last_mut()
        .ok_or_else(|| vm.type_error("show without a context"))?
        .scene
        .show(
            layer,
            Image {
                tag,
                name,
                value,
                transforms,
                zorder: match zorder {
                    Value::Int(value) => *value,
                    _ => 0,
                },
            },
            &behind,
        );
    Ok(Value::None)
}

fn hide(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [tag, layer] = args else {
        return Err(vm.type_error("invalid hide arguments"));
    };
    let Some(tag) = tag.as_str() else {
        return Err(vm.type_error("hide requires an image tag"));
    };
    let st = state(vm);
    let mut contexts = st.contexts.borrow_mut();
    if let Some(images) = contexts
        .last_mut()
        .and_then(|c| c.scene.layers.get_mut(layer.as_str().unwrap_or("master")))
    {
        images.retain(|image| image.tag != tag);
    }
    Ok(Value::None)
}

fn clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [layer] = args else {
        return Err(vm.type_error("scene expects a layer"));
    };
    let st = state(vm);
    if let Some(context) = st.contexts.borrow_mut().last_mut() {
        context
            .scene
            .layers
            .remove(layer.as_str().unwrap_or("master"));
    }
    Ok(Value::None)
}

pub struct ParseCx {
    pub st: Rc<State>,
    pub registry: crate::sl2::Registry,
}

impl crate::atl::ParseContext for ParseCx {
    fn is_warper(&self, name: &str) -> bool {
        self.st.warpers.borrow().contains(name)
    }
}

impl crate::parser::ScriptContext for ParseCx {
    fn user_statement_names(&self) -> Vec<Vec<String>> {
        self.st.user_statements.borrow().keys().cloned().collect()
    }
    fn parse_user_statement(
        &self,
        arena: &mut Arena,
        name: &[String],
        lexer: &mut Lexer,
        loc: &(Rc<str>, u32),
    ) -> PResult<Vec<NodeId>> {
        let definitions = self.st.user_statements.borrow();
        let Some(definition) = definitions.get(name) else {
            return lexer.error(format!("unregistered statement {}", name.join(" ")));
        };
        crate::statements::parse_source(arena, definition, lexer, loc)
    }
    fn sl2_registry(&self) -> &crate::sl2::Registry {
        &self.registry
    }
}

struct AtlClass(ObjRef);

#[derive(Clone)]
pub(crate) struct AtlDefinition {
    pub script: crate::atl::RawBlock,
    pub parameters: Option<Signature>,
    pub globals: ObjRef,
    pub locals: Option<ObjRef>,
}

pub(crate) fn atl_definition(definition: &Value) -> Option<AtlDefinition> {
    let Some(crysirror_python::Payload::Native(payload)) = definition.payload() else {
        return None;
    };
    payload.borrow().downcast_ref::<AtlDefinition>().cloned()
}

fn resolve_atl_locals(
    vm: &mut Vm,
    definition: &AtlDefinition,
    args: &[Value],
    kwargs: Kw,
) -> PyResult<Option<ObjRef>> {
    if args.is_empty() && kwargs.is_empty() && definition.locals.is_some() {
        return Ok(definition.locals.clone());
    }
    let Some(signature) = definition.parameters.as_ref() else {
        if args.is_empty() && kwargs.is_empty() {
            return Ok(None);
        } else {
            return Err(vm.type_error("ATL parameters were already bound"));
        }
    };
    crate::pycompile::bind_parameters(
        vm,
        signature,
        &definition.globals,
        &definition.script.loc,
        args,
        kwargs,
    )
    .map(|scope| scope.obj().cloned())
}

fn atl_call(vm: &mut Vm, args: &[Value], kwargs: Kw) -> PyResult<Value> {
    let Some((value, args)) = args.split_first() else {
        return Err(vm.type_error("invalid ATL call"));
    };
    let mut definition =
        atl_definition(value).ok_or_else(|| vm.type_error("invalid ATL transform"))?;
    definition.locals = resolve_atl_locals(vm, &definition, args, kwargs)?;
    definition.parameters = None;
    Ok(new_atl(vm, definition))
}

fn atl_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [definition] = args else {
        return Err(vm.type_error("expected an ATL transform"));
    };
    let definition =
        atl_definition(definition).ok_or_else(|| vm.type_error("invalid ATL transform"))?;
    let location = &definition.script.loc;
    let parameterized = if definition.parameters.is_none() {
        ""
    } else {
        " with parameters"
    };
    Ok(vm.types.new_str(&format!(
        "<ATLTransform{} at {}:{}>",
        parameterized, location.0, location.1
    )))
}

pub fn atl_transform(
    vm: &mut Vm,
    atl: crate::atl::RawBlock,
    parameters: Option<Signature>,
    globals: ObjRef,
) -> Value {
    new_atl(
        vm,
        AtlDefinition {
            script: atl,
            parameters,
            globals,
            locals: None,
        },
    )
}

fn new_atl(vm: &mut Vm, definition: AtlDefinition) -> Value {
    let class = vm
        .ext_get::<AtlClass>()
        .expect("ATL class missing")
        .0
        .clone();
    vm.types.new_native(&class, Box::new(definition))
}
