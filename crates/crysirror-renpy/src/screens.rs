// SPDX-License-Identifier: MPL-2.0
use crate::sl2::Screen;
use crysirror_python::{PyResult, Vm};

pub fn define(vm: &mut Vm, screen: std::rc::Rc<Screen>) -> PyResult<()> {
    crate::engine::state(vm)
        .screens
        .borrow_mut()
        .insert(screen.name.clone(), screen);
    Ok(())
}

use crate::sl2::{NChildren, StatementSpec};
use crysirror_python::{Value, builtins::Kw, ops};

pub fn register(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [
        name,
        displayable,
        children,
        style,
        scope,
        replaces,
        defaults,
        custom,
    ] = args
    else {
        return Err(vm.type_error("invalid screen statement registration"));
    };
    let name = ops::str(vm, name)?;
    let nchildren = match children {
        Value::Int(0) => NChildren::Zero,
        Value::Int(1) => NChildren::One,
        value if value.as_str() == Some("many") => NChildren::Many,
        _ => return Err(vm.type_error("invalid screen statement children count")),
    };
    let style = if style.is_none() {
        None
    } else {
        Some(ops::str(vm, style)?)
    };
    let custom_screen = ops::truthy(vm, custom)?.then(|| name.clone());
    let default_keywords = if defaults.is_none() {
        Vec::new()
    } else {
        ops::mapping_items(vm, defaults)?
            .into_iter()
            .map(|(key, value)| Ok((ops::str(vm, &key)?, ops::repr(vm, &value)?)))
            .collect::<PyResult<Vec<_>>>()?
    };
    let st = crate::engine::state(vm);
    let spec = StatementSpec {
        name: name.clone(),
        displayable: format!("renpy.registered.{name}"),
        style,
        nchildren,
        positional: Vec::new(),
        scope: ops::truthy(vm, scope)?,
        pass_context: false,
        imagemap: false,
        hotspot: false,
        replaces: ops::truthy(vm, replaces)?,
        unique: false,
        default_keywords,
        custom_screen,
    };
    if !displayable.is_none() {
        st.screen_displayables
            .borrow_mut()
            .insert(name.clone(), displayable.clone());
    }
    st.sl2.borrow_mut().statements.push(spec);
    Ok(vm.types.new_str(&name))
}

pub fn add_positional(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name, positional] = args else {
        return Err(vm.type_error("invalid screen positional registration"));
    };
    let name = ops::str(vm, name)?;
    let positional = ops::str(vm, positional)?;
    let st = crate::engine::state(vm);
    let mut registry = st.sl2.borrow_mut();
    let spec = registry
        .statements
        .iter_mut()
        .rev()
        .find(|spec| spec.name == name)
        .ok_or_else(|| vm.type_error("screen statement does not exist"))?;
    spec.positional.push(positional);
    Ok(Value::None)
}
