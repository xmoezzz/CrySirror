// SPDX-License-Identifier: MPL-2.0
//! `renpy.easy`: displayable coercion and small helpers, after Ren'Py 8.3
//! `easy.py` (MIT).
use super::{core::is_displayable, py::{self, *}};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.easy";

fn store_attr(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "store", name)
}

/// `lookup_displayable_prefix(d)`.
pub fn lookup_displayable_prefix(vm: &mut Vm, d: &str) -> PyResult<Option<Value>> {
    let Some((prefix, arg)) = d.split_once(':') else { return Ok(None) };
    let table = py::config(vm, "displayable_prefix")?;
    if py::is_none(&table) {
        return Ok(None);
    }
    let key = vm.types.new_str(prefix);
    let f = vm.call_method(&table, "get", vec![key, Value::None])?;
    if py::is_none(&f) {
        return Ok(None);
    }
    let a = vm.types.new_str(arg);
    let r = vm.call(&f, vec![a], Vec::new())?;
    Ok(Some(displayable(vm, &r, &Value::None, true)?))
}

fn is_color(vm: &mut Vm, d: &Value) -> PyResult<bool> {
    let c = py::import_attr(vm, "renpy.color", "Color")?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, d, c)))
}

fn dynamic_image(vm: &mut Vm, d: &Value, scope: &Value) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.image", "DynamicImage")?;
    vm.call(&c, vec![d.clone()], vec![("scope".into(), scope.clone())])
}

/// `displayable(d, scope=None)` / `displayable_or_none`.
pub fn displayable(vm: &mut Vm, d: &Value, scope: &Value, dynamic: bool) -> PyResult<Value> {
    if is_displayable(vm, d) {
        return Ok(d.clone());
    }
    if let Some(s) = d.as_str() {
        if s.is_empty() {
            return Err(py::error(vm, "An empty string cannot be used as a displayable."));
        }
        if s.contains('[') && dynamic && py::config_bool(vm, "dynamic_images").unwrap_or(true) {
            return dynamic_image(vm, d, scope);
        }
        if let Some(rv) = lookup_displayable_prefix(vm, s)? {
            return Ok(rv);
        }
        if s.starts_with('#') {
            let solid = store_attr(vm, "Solid")?;
            return vm.call(&solid, vec![d.clone()], Vec::new());
        }
        if s.contains('.') {
            let image = store_attr(vm, "Image")?;
            return vm.call(&image, vec![d.clone()], Vec::new());
        }
        let r = store_attr(vm, "ImageReference")?;
        let parts = s.split_whitespace().map(|p| vm.types.new_str(p)).collect();
        let t = vm.types.new_tuple(parts);
        return vm.call(&r, vec![t], Vec::new());
    }
    if is_color(vm, d)? {
        let solid = store_attr(vm, "Solid")?;
        return vm.call(&solid, vec![d.clone()], Vec::new());
    }
    if matches!(d.payload(), Some(crysirror_python::Payload::List(_))) {
        return dynamic_image(vm, d, scope);
    }
    if py::get_opt(vm, d, "_duplicate")?.is_some() {
        return Ok(d.clone());
    }
    if matches!(d, Value::Bool(_)) {
        return Ok(d.clone());
    }
    let r = ops::repr(vm, d)?;
    Err(py::error(vm, format!("Not a displayable: {r}")))
}

fn f_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    displayable(vm, &d, &scope, true)
}

fn f_displayable_or_none(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    let dynamic = match arg(args, kw, 2, "dynamic") { Some(v) => ops::truthy(vm, v)?, None => true };
    displayable(vm, &d, &scope, dynamic)
}

fn f_lookup_displayable_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    let s = py::string(vm, &d)?;
    Ok(lookup_displayable_prefix(vm, &s)?.unwrap_or(Value::None))
}

fn find_image(vm: &mut Vm, name: &Value, single: bool) -> PyResult<bool> {
    let exists = py::import_attr(vm, "renpy.exports", "image_exists")?;
    let r = vm.call(&exists, vec![name.clone()], Vec::new())?;
    if ops::truthy(vm, &r)? {
        return Ok(true);
    }
    let loadable = py::import_attr(vm, "renpy.loader", "loadable")?;
    let r = vm.call(&loadable, vec![name.clone()], vec![("directory".into(), vm.types.new_str("images"))])?;
    if ops::truthy(vm, &r)? {
        return Ok(true);
    }
    let s = py::string(vm, name)?;
    if lookup_displayable_prefix(vm, &s)?.is_some() {
        return Ok(true);
    }
    if single {
        let cb = py::config(vm, "missing_image_callback")?;
        if !py::is_none(&cb) {
            let r = vm.call(&cb, vec![name.clone()], Vec::new())?;
            if ops::truthy(vm, &r)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn substitute(vm: &mut Vm, s: &Value, scope: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    let r = vm.call(&f, vec![s.clone()], vec![("scope".into(), scope.clone()), ("force".into(), Value::Bool(true)), ("translate".into(), Value::Bool(false))])?;
    ops::getitem(vm, &r, Value::Int(0))
}

/// `dynamic_image(d, scope=None, prefix=None, search=None)`.
fn f_dynamic_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let mut scope = arg_or(args, kw, 1, "scope", Value::None);
    let prefix = arg_or(args, kw, 2, "prefix", Value::None);
    let search = arg_or(args, kw, 3, "search", Value::None);
    let list = match d.payload() {
        Some(crysirror_python::Payload::List(l)) => l.borrow().clone(),
        _ => vec![d.clone()],
    };
    let single = list.len() == 1;
    for i in &list {
        let Some(text) = i.as_str() else { continue };
        if !py::is_none(&prefix) && text.contains("[prefix_") {
            let base = if ops::truthy(vm, &scope)? { vm.call(&Value::Obj(vm.types.dict.clone()), vec![scope.clone()], Vec::new())? } else { vm.types.new_dict() };
            scope = base;
            let search_table = py::import_attr(vm, "renpy.styledata.stylesets", "prefix_search")?;
            let prefixes = ops::getitem(vm, &search_table, prefix.clone())?;
            for p in py::items(vm, &prefixes)? {
                py::dict_set(vm, &scope, "prefix_", p)?;
                let rv = substitute(vm, i, &scope)?;
                if find_image(vm, &rv, single)? {
                    return displayable(vm, &rv, &Value::None, true);
                }
                if !py::is_none(&search) {
                    ops::list_append(vm, &search, rv)?;
                }
            }
        } else {
            let rv = substitute(vm, i, &scope)?;
            if find_image(vm, &rv, single)? {
                return displayable(vm, &rv, &Value::None, true);
            }
            if !py::is_none(&search) {
                ops::list_append(vm, &search, rv)?;
            }
        }
    }
    let last = list.last().cloned().unwrap_or(Value::None);
    if !py::is_none(&last) && find_image(vm, &last, single)? {
        return displayable(vm, &last, &Value::None, false);
    }
    Ok(Value::None)
}

fn f_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let d = displayable(vm, &d, &Value::None, true)?;
    if let Ok(p) = py::import_attr(vm, "renpy.display.predict", "displayable") {
        vm.call(&p, vec![d], Vec::new())?;
    }
    Ok(Value::None)
}

fn f_split_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let props = args.first().cloned().unwrap_or(Value::None);
    let prefixes: Vec<String> = args[1..].iter().map(|p| p.as_str().unwrap_or("").to_owned()).collect();
    let out: Vec<Value> = prefixes.iter().map(|_| vm.types.new_dict()).collect();
    if !py::is_none(&props) && ops::truthy(vm, &props)? {
        for (k, v) in py::str_items(vm, &props)? {
            match prefixes.iter().position(|p| k.starts_with(p.as_str())) {
                Some(i) => py::dict_set(vm, &out[i], &k[prefixes[i].len()..], v)?,
                None => return Err(py::error(vm, format!("Property {k} begins with an unknown prefix."))),
            }
        }
    }
    Ok(vm.types.new_list(out))
}

fn iterable(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    if v.as_str().is_some() {
        return Ok(false);
    }
    Ok(ops::iter(vm, v).is_ok())
}

fn f_to_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "value", Value::None);
    let copy = match arg(args, kw, 1, "copy") { Some(c) => ops::truthy(vm, c)?, None => false };
    if let Some(crysirror_python::Payload::List(l)) = v.payload() {
        if !copy {
            return Ok(v.clone());
        }
        let items = l.borrow().clone();
        return Ok(vm.types.new_list(items));
    }
    if iterable(vm, &v)? {
        let items = ops::collect(vm, &v)?;
        return Ok(vm.types.new_list(items));
    }
    Ok(vm.types.new_list(vec![v]))
}

fn f_to_tuple(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "value", Value::None);
    if matches!(v.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return Ok(v);
    }
    if iterable(vm, &v)? {
        let items = ops::collect(vm, &v)?;
        return Ok(vm.types.new_tuple(items));
    }
    Ok(vm.types.new_tuple(vec![v]))
}

pub fn run_callbacks(vm: &mut Vm, cb: &Value, args: Vec<Value>, kw: Vec<(std::rc::Rc<str>, Value)>) -> PyResult<Value> {
    if py::is_none(cb) {
        return Ok(Value::None);
    }
    if let Some(items) = py::seq(cb) {
        let mut rv = Value::None;
        for i in items {
            let r = run_callbacks(vm, &i, args.clone(), kw.clone())?;
            if !py::is_none(&r) {
                rv = r;
            }
        }
        return Ok(rv);
    }
    vm.call(cb, args, kw)
}

fn f_run_callbacks(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cb = args.first().cloned().unwrap_or(Value::None);
    run_callbacks(vm, &cb, args.get(1..).map(|a| a.to_vec()).unwrap_or_default(), kw.to_vec())
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("lookup_displayable_prefix", f_lookup_displayable_prefix),
        ("displayable_or_none", f_displayable_or_none),
        ("displayable", f_displayable),
        ("dynamic_image", f_dynamic_image),
        ("predict", f_predict),
        ("split_properties", f_split_properties),
        ("to_list", f_to_list),
        ("to_tuple", f_to_tuple),
        ("run_callbacks", f_run_callbacks),
    ]);
    let color = py::import_attr(vm, "renpy.color", "Color")?;
    crysirror_python::modules::set(vm, &m, "Color", color.clone());
    crysirror_python::modules::set(vm, &m, "color", color);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
