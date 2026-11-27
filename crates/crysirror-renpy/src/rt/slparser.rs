// SPDX-License-Identifier: MPL-2.0
//! `renpy.sl2.slparser` and `renpy.sl2.slproperties`, after Ren'Py 8.3
//! (MIT): the Python face of the screen-language statement registry.
//!
//! Screens are parsed by the Rust parser (`crate::sl2`) against the
//! runtime's statement registry. `register_sl_displayable` and
//! `register_sl_statement` (`CustomParser`) add statements to that
//! registry; the returned parser objects record positional arguments,
//! which change parsing, and accept the property declarations that only
//! upstream's parse-time validation uses. A displayable callable is
//! recorded by its dotted name (`module.qualname`, store names resolving
//! in the store), as pickled screen ASTs name it.
use super::{
    py::{self, *},
    slproperty_names::PROPERTY_NAMES,
    state::rt,
};
use crate::sl2::{NChildren, StatementSpec};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.sl2.slparser";

fn nchildren(vm: &mut Vm, v: &Value) -> PyResult<NChildren> {
    if v.as_str() == Some("many") {
        return Ok(NChildren::Many);
    }
    if py::is_none(v) {
        return Ok(NChildren::Zero);
    }
    if let Ok(m) = py::import_attr(vm, MODULE, "many") {
        if py::same(&m, v) {
            return Ok(NChildren::Many);
        }
    }
    // Upstream compares with 0 and 1 only; any other value bears children
    // (00icon.rpy passes a string).
    if !matches!(v, Value::Int(_) | Value::Bool(_) | Value::Float(_)) {
        return Ok(NChildren::Many);
    }
    Ok(match py::int(vm, v)? {
        0 => NChildren::Zero,
        1 => NChildren::One,
        _ => NChildren::Many,
    })
}

fn dotted_name(vm: &mut Vm, f: &Value) -> PyResult<String> {
    let module = py::get_opt(vm, f, "__module__")?.filter(|m| !py::is_none(m));
    let name = match py::get_opt(vm, f, "__qualname__")?.filter(|n| !py::is_none(n)) {
        Some(n) => n,
        None => py::get(vm, f, "__name__")?,
    };
    let name = py::string(vm, &name)?;
    Ok(match module {
        Some(m) => format!("{}.{name}", py::string(vm, &m)?),
        None => name,
    })
}

fn register(vm: &mut Vm, spec: StatementSpec) {
    let r = rt(vm);
    let mut reg = r.registry.borrow_mut();
    reg.statements.retain(|s| s.name != spec.name);
    reg.statements.push(spec);
}

fn parser_object(vm: &mut Vm, class: &str, name: &str) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, class)?;
    let o = vm.call(&c, Vec::new(), vec![("_crysirror_registered".into(), vm.types.new_str(name))])?;
    Ok(o)
}

const DP_ARGS: &[&str] = &["name", "displayable", "style", "nchildren", "scope", "pass_context", "imagemap", "replaces", "default_keywords", "hotspot", "default_properties", "unique"];

/// `register_sl_displayable(name, displayable, style, nchildren=0, ...)`.
fn f_register_sl_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v: Vec<Value> = DP_ARGS.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    let name = py::string(vm, &v[0])?;
    let displayable = dotted_name(vm, &v[1])?;
    // `keywords.pop("style_suffix", None) or self.style`: a false style is none.
    let style = if py::truthy(vm, &v[2])? { Some(py::string(vm, &v[2])?) } else { None };
    let b = |vm: &mut Vm, i: usize| -> PyResult<bool> { py::truthy(vm, &v[i]) };
    let mut default_keywords = Vec::new();
    if !py::is_none(&v[8]) {
        for (k, val) in ops::mapping_items(vm, &v[8])? {
            let r = ops::repr(vm, &val)?;
            default_keywords.push((py::string(vm, &k)?, r));
        }
    }
    let spec = StatementSpec {
        name: name.clone(),
        displayable,
        style,
        nchildren: nchildren(vm, &v[3])?,
        positional: Vec::new(),
        scope: b(vm, 4)?,
        pass_context: b(vm, 5)?,
        imagemap: b(vm, 6)?,
        hotspot: b(vm, 9)?,
        replaces: b(vm, 7)?,
        unique: b(vm, 11)?,
        default_keywords,
        custom_screen: None,
    };
    register(vm, spec);
    let rv = parser_object(vm, "DisplayableParser", &name)?;
    py::set(vm, &rv, "displayable", v[1].clone())?;
    py::set(vm, &rv, "style", v[2].clone())?;
    py::set(vm, &rv, "nchildren", v[3].clone())?;
    Ok(rv)
}

/// `register_sl_statement(name, children="many", screen=None)`.
fn f_register_sl_statement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &arg_or(args, kw, 0, "name", Value::None))?;
    let children = arg_or(args, kw, 1, "children", py::s(vm, "many"));
    let screen = arg_or(args, kw, 2, "screen", Value::None);
    let screen = if py::is_none(&screen) { name.clone() } else { py::string(vm, &screen)? };
    let spec = StatementSpec {
        name: name.clone(),
        displayable: String::new(),
        style: None,
        nchildren: nchildren(vm, &children)?,
        positional: Vec::new(),
        scope: false,
        pass_context: false,
        imagemap: false,
        hotspot: false,
        replaces: false,
        unique: false,
        default_keywords: Vec::new(),
        custom_screen: Some(screen.clone()),
    };
    register(vm, spec);
    let rv = parser_object(vm, "CustomParser", &name)?;
    py::set(vm, &rv, "screen", vm.types.new_str(&screen))?;
    py::set(vm, &rv, "nchildren", children)?;
    Ok(rv)
}

fn p_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let reg = py::kw(kw, "_crysirror_registered").cloned();
    match reg {
        Some(n) => {
            py::set(vm, this, "name", n)?;
            py::set(vm, this, "registered", Value::Bool(true))?;
        }
        None => {
            // A parser built directly (not through the register functions)
            // only declares; the registry is not changed.
            let n = args.get(1).cloned().unwrap_or(Value::None);
            py::set(vm, this, "name", n)?;
            py::set(vm, this, "registered", Value::Bool(false))?;
        }
    }
    py::set(vm, this, "positional", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "keyword", py::dict(vm))?;
    Ok(Value::None)
}

fn p_add_positional(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pos = py::get(vm, this, "positional")?;
    vm.call_method(&pos, "append", vec![args[1].clone()])?;
    let registered = py::get(vm, this, "registered")?;
    if py::truthy(vm, &registered)? {
        let name = py::get(vm, this, "name")?;
        let name = py::string(vm, &name)?;
        let p = py::string(vm, &args[1])?;
        let r = rt(vm);
        let mut reg = r.registry.borrow_mut();
        if let Some(s) = reg.statements.iter_mut().rev().find(|s| s.name == name) {
            s.positional.push(p);
        }
    }
    Ok(args[0].clone())
}

fn p_add_keyword(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let kwd = py::get(vm, &args[0], "keyword")?;
    let last = args.last().cloned().unwrap_or(Value::None);
    if last.as_str().is_some() {
        ops::setitem(vm, &kwd, last, Value::Bool(true))?;
    }
    Ok(args[0].clone())
}

fn p_noop(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn f_init(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn decl_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = vm.types.new_tuple(args[1..].to_vec());
    py::set(vm, &args[0], "args", t)?;
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("register_sl_displayable", f_register_sl_displayable), ("register_sl_statement", f_register_sl_statement), ("init", f_init)],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let methods: py::Methods = &[
        ("__init__", p_init),
        ("add_positional", p_add_positional),
        ("add_property", p_add_keyword),
        ("add_style_property", p_add_keyword),
        ("add_prefix_style_property", p_add_keyword),
        ("add_property_group", p_noop),
        ("copy_properties", p_noop),
        ("add", p_noop),
    ];
    let base = py::class(vm, MODULE, "Parser", &[], methods, &[])?;
    set(vm, "Parser", Value::Obj(base.clone()));
    for n in ["DisplayableParser", "CustomParser"] {
        let c = py::class(vm, MODULE, n, &[Value::Obj(base.clone())], &[], &[])?;
        set(vm, n, Value::Obj(c));
    }
    for n in ["Keyword", "Style", "PrefixStyle"] {
        let c = py::class(vm, MODULE, n, &[], &[("__init__", decl_init)], &[])?;
        set(vm, n, Value::Obj(c));
    }
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let many = vm.call(&sentinel, vec![py::s(vm, "many")], Vec::new())?;
    set(vm, "many", many);
    set(vm, "parser", Value::None);
    // Module data of renpy/sl2/slparser.py (6.99 through 8.x).
    let prefixes = ["", "insensitive_", "hover_", "idle_", "activate_", "selected_", "selected_insensitive_", "selected_hover_", "selected_idle_", "selected_activate_"];
    let sp = vm.types.new_list(prefixes.iter().map(|p| vm.types.new_str(p)).collect());
    set(vm, "STYLE_PREFIXES", sp);
    set(vm, "statements", vm.types.new_dict());
    set(vm, "all_child_statements", vm.types.new_list(Vec::new()));
    set(vm, "childbearing_statements", vm.types.new_set(Default::default()));
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let st = Value::Obj(vm.types.set.clone());
    let props = vm.call(&dd, vec![st], Vec::new())?;
    set(vm, "properties", props);
    let pos = py::class(vm, MODULE, "Positional", &[], &[("__init__", decl_init)], &[])?;
    set(vm, "Positional", Value::Obj(pos));
    Ok(m)
}

fn slproperties_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.sl2.slproperties", &[]);
    for (n, list) in PROPERTY_NAMES {
        let l = vm.types.new_list(list.iter().map(|s| vm.types.new_str(s)).collect());
        crysirror_python::modules::set(vm, &m, n, l);
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
    vm.register_native_module("renpy.sl2.slproperties", slproperties_module);
}
