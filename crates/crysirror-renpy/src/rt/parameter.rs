// SPDX-License-Identifier: MPL-2.0
//! `renpy.parameter` (8.2+; `renpy.ast` before), after Ren'Py 8.3
//! `parameter.py` (MIT): `ArgumentInfo` as script and creator-defined
//! statements see it, plus the shared signature helpers.
//!
//! The layout follows the release: before 7.5 an `ArgumentInfo` holds
//! `arguments`, `extrapos` and `extrakw`; from 7.5 it is a versioned
//! `renpy.object.Object` with `starred_indexes` / `doublestarred_indexes`
//! and upgrades the old layout when unpickled.
use super::py::{self, *};
use crate::ast::ArgumentInfo;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.parameter";

fn starred_layout(vm: &Vm) -> bool {
    super::state::try_rt(vm)
        .map(|r| r.profile.at_least(7, 5))
        .unwrap_or(true)
}

fn class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "ArgumentInfo")
}

/// The Python `ArgumentInfo` of a parsed argument list.
pub fn argument_info_value(
    vm: &mut Vm,
    info: &ArgumentInfo,
    loc: &(Rc<str>, u32),
) -> PyResult<Value> {
    let c = class(vm)?;
    let mut args = Vec::new();
    let (mut extrapos, mut extrakw) = (Value::None, Value::None);
    let starred = starred_layout(vm);
    for (i, (k, v)) in info.arguments.iter().enumerate() {
        let e = super::lexer::pyexpr(vm, v, &loc.0, loc.1)?;
        if !starred && info.starred.contains(&i) {
            extrapos = e;
            continue;
        }
        if !starred && info.doublestarred.contains(&i) {
            extrakw = e;
            continue;
        }
        let k = k
            .as_deref()
            .map(|k| vm.types.new_str(k))
            .unwrap_or(Value::None);
        args.push(vm.types.new_tuple(vec![k, e]));
    }
    let a = vm.types.new_list(args);
    if !starred {
        return vm.call(&c, vec![a, extrapos, extrakw], Vec::new());
    }
    let set = |vm: &mut Vm, v: &[usize]| -> PyResult<Value> {
        let s = vm.types.new_set(Default::default());
        for i in v {
            ops::set_add(vm, &s, Value::Int(*i as i64))?;
        }
        Ok(s)
    };
    let s = set(vm, &info.starred)?;
    let d = set(vm, &info.doublestarred)?;
    vm.call(&c, vec![a, s, d], Vec::new())
}

fn index_set(vm: &mut Vm, this: &Value, name: &str) -> PyResult<Vec<usize>> {
    let Some(v) = py::get_opt(vm, this, name)? else {
        return Ok(Vec::new());
    };
    if py::is_none(&v) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for i in py::items(vm, &v)? {
        out.push(py::int(vm, &i)? as usize);
    }
    Ok(out)
}

/// Reads a Python `ArgumentInfo` (either layout) into the Rust form, with
/// the location of its first expression.
pub fn argument_info_of(vm: &mut Vm, this: &Value) -> PyResult<(ArgumentInfo, (Rc<str>, u32))> {
    let mut info = ArgumentInfo::default();
    let mut loc: Option<(Rc<str>, u32)> = None;
    let mut note = |vm: &mut Vm, e: &Value| -> PyResult<()> {
        if loc.is_none() {
            if let Some(f) = py::get_opt(vm, e, "filename")?.filter(|f| !py::is_none(f)) {
                let n = py::get_opt(vm, e, "linenumber")?.unwrap_or(Value::Int(0));
                loc = Some((py::string(vm, &f)?.into(), py::int(vm, &n)? as u32));
            }
        }
        Ok(())
    };
    let args = py::get(vm, this, "arguments")?;
    for a in py::items(vm, &args)? {
        let p = py::items(vm, &a)?;
        let k = if py::is_none(&p[0]) {
            None
        } else {
            Some(py::string(vm, &p[0])?)
        };
        note(vm, &p[1])?;
        info.arguments.push((k, py::string(vm, &p[1])?));
    }
    info.starred = index_set(vm, this, "starred_indexes")?;
    info.doublestarred = index_set(vm, this, "doublestarred_indexes")?;
    for (field, star) in [("extrapos", true), ("extrakw", false)] {
        let Some(v) = py::get_opt(vm, this, field)?.filter(|v| !py::is_none(v)) else {
            continue;
        };
        note(vm, &v)?;
        let i = info.arguments.len();
        info.arguments.push((None, py::string(vm, &v)?));
        if star {
            info.starred.push(i)
        } else {
            info.doublestarred.push(i)
        }
    }
    Ok((info, loc.unwrap_or_else(|| ("<none>".into(), 1))))
}

fn a_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let arguments = arg_or(&args[1..], kw, 0, "arguments", Value::None);
    py::set(vm, this, "arguments", arguments)?;
    if starred_layout(vm) {
        for (i, n) in [(1, "starred_indexes"), (2, "doublestarred_indexes")] {
            let v = arg_or(&args[1..], kw, i, n, Value::None);
            let v = if py::truthy(vm, &v)? {
                v
            } else {
                vm.types.new_set(Default::default())
            };
            py::set(vm, this, n, v)?;
        }
    } else {
        for (i, n) in [(1, "extrapos"), (2, "extrakw")] {
            let v = arg_or(&args[1..], kw, i, n, Value::None);
            py::set(vm, this, n, v)?;
        }
    }
    Ok(Value::None)
}

/// `after_upgrade(version)`: moves `extrapos`/`extrakw` into `arguments`.
fn a_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let version = py::int(vm, &args[1])?;
    if version >= 1 {
        return Ok(Value::None);
    }
    let arguments = py::get(vm, this, "arguments")?;
    let extrapos = py::get_opt(vm, this, "extrapos")?.unwrap_or(Value::None);
    let extrakw = py::get_opt(vm, this, "extrakw")?.unwrap_or(Value::None);
    let (p, k) = (py::truthy(vm, &extrapos)?, py::truthy(vm, &extrakw)?);
    let length = ops::len(vm, &arguments)? as i64 + p as i64 + k as i64;
    let single = |vm: &mut Vm, i: i64| -> PyResult<Value> {
        let s = vm.types.new_set(Default::default());
        ops::set_add(vm, &s, Value::Int(i))?;
        Ok(s)
    };
    if p {
        let s = single(vm, length - 1)?;
        py::set(vm, this, "starred_indexes", s)?;
        let t = vm.types.new_tuple(vec![Value::None, extrapos.clone()]);
        vm.call_method(&arguments, "append", vec![t])?;
    }
    if k {
        let s = single(vm, length - 1)?;
        py::set(vm, this, "doublestarred_indexes", s)?;
        let t = vm.types.new_tuple(vec![Value::None, extrakw]);
        vm.call_method(&arguments, "append", vec![t])?;
    }
    if p && k {
        let s = single(vm, length - 2)?;
        py::set(vm, this, "starred_indexes", s)?;
    }
    Ok(Value::None)
}

fn a_evaluate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let scope = arg_or(&args[1..], kw, 0, "scope", Value::None);
    let (info, loc) = argument_info_of(vm, &args[0])?;
    let (a, k) = super::params::evaluate(vm, &info, &loc, &scope)?;
    Ok(vm.types.new_tuple(vec![a, k]))
}

pub fn get_code(info: &ArgumentInfo) -> String {
    let mut l = Vec::new();
    for (i, (k, e)) in info.arguments.iter().enumerate() {
        if info.starred.contains(&i) {
            l.push(format!("*{e}"));
        } else if info.doublestarred.contains(&i) {
            l.push(format!("**{e}"));
        } else if let Some(k) = k {
            l.push(format!("{k}={e}"));
        } else {
            l.push(e.clone());
        }
    }
    format!("({})", l.join(", "))
}

fn a_get_code(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (info, _) = argument_info_of(vm, &args[0])?;
    Ok(vm.types.new_str(&get_code(&info)))
}

fn a_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (info, _) = argument_info_of(vm, &args[0])?;
    Ok(vm
        .types
        .new_str(&format!("<ArgumentInfo {}>", get_code(&info))))
}

/// Forwards to `renpy.ast.apply_arguments` (imported at call time: the
/// AST module imports this one).
fn f_apply_arguments(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.ast", "apply_arguments")?;
    vm.call(&f, args.to_vec(), kw.to_vec())
}

// Parameter / ValuedParameter (7.8 / 8.2+ `renpy.parameter`) ---------------------------------

fn p_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cls = Value::Obj(ops::type_of(vm, this));
    let empty = py::get(vm, &cls, "empty")?;
    let name = py::arg_or(&args[1..], kw, 0, "name", Value::None);
    let kind = py::arg_or(&args[1..], kw, 1, "kind", Value::Int(1));
    let default = py::arg_or(&args[1..], kw, 2, "default", empty);
    py::set(vm, this, "name", name)?;
    py::set(vm, this, "kind", kind)?;
    py::set(vm, this, "default", default)?;
    Ok(Value::None)
}

fn p_has_default(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &args[0]));
    let empty = py::get(vm, &cls, "empty")?;
    let d = py::get(vm, &args[0], "default")?;
    Ok(Value::Bool(!py::same(&d, &empty)))
}

fn p_default_value(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "default")?;
    let locals = py::arg_or(&args[1..], kw, 0, "locals", Value::None);
    let globals = py::arg_or(&args[1..], kw, 1, "globals", Value::None);
    let f = py::import_attr(vm, "renpy.python", "py_eval")?;
    vm.call(&f, vec![d], vec![("locals".into(), locals), ("globals".into(), globals)])
}

fn vp_default_value(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "default")
}

fn p_replace(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut fields: Vec<(std::rc::Rc<str>, Value)> = Vec::new();
    for n in ["name", "kind", "default"] {
        let v = match kw.iter().find(|(k, _)| &**k == n) {
            Some((_, v)) => v.clone(),
            None => py::get(vm, this, n)?,
        };
        fields.push((n.into(), v));
    }
    let cls = Value::Obj(ops::type_of(vm, this));
    vm.call(&cls, Vec::new(), fields)
}

fn p_format(vm: &mut Vm, this: &Value, valued: bool) -> PyResult<String> {
    let name = py::get(vm, this, "name")?;
    let name = py::string(vm, &name)?;
    let kind = py::get(vm, this, "kind")?;
    let kind = py::int(vm, &kind)?;
    Ok(match kind {
        2 => format!("*{name}"),
        4 => format!("**{name}"),
        _ => {
            let has = p_has_default(vm, std::slice::from_ref(this), &[])?;
            if matches!(has, Value::Bool(true)) {
                let d = py::get(vm, this, "default")?;
                if valued {
                    format!("{name}={}", ops::repr(vm, &d)?)
                } else {
                    format!("{name}={}", py::string(vm, &d)?)
                }
            } else {
                name
            }
        }
    })
}

fn p_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = p_format(vm, &args[0], false)?;
    Ok(py::s(vm, &t))
}

fn vp_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = p_format(vm, &args[0], true)?;
    Ok(py::s(vm, &t))
}

fn p_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = ops::str(vm, &args[0])?;
    Ok(py::s(vm, &format!("<Parameter {s}>")))
}

fn p_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    if py::same(a, b) {
        return Ok(Value::Bool(true));
    }
    let cls = py::import_attr(vm, MODULE, "Parameter")?;
    if !cls.obj().is_some_and(|c| ops::isinstance(vm, b, c)) {
        return Ok(Value::Bool(false));
    }
    for n in ["name", "kind", "default"] {
        let x = py::get(vm, a, n)?;
        let y = py::get(vm, b, n)?;
        if !ops::eq(vm, &x, &y)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn parameter_classes(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let hd = py::builtin(vm, "has_default", p_has_default);
    let hd = py::property(vm, hd, None)?;
    let mut attrs: Vec<(&str, Value)> = vec![("has_default", hd), ("empty", Value::None), ("__hash__", Value::None)];
    for (i, n) in ["POSITIONAL_ONLY", "POSITIONAL_OR_KEYWORD", "VAR_POSITIONAL", "KEYWORD_ONLY", "VAR_KEYWORD"].iter().enumerate() {
        attrs.push((n, Value::Int(i as i64)));
    }
    let p = py::class(
        vm,
        MODULE,
        "Parameter",
        &[],
        &[
            ("__init__", p_init),
            ("default_value", p_default_value),
            ("replace", p_replace),
            ("__str__", p_str),
            ("__repr__", p_repr),
            ("__eq__", p_eq),
        ],
        &attrs,
    )?;
    set(vm, "Parameter", Value::Obj(p.clone()));
    // `ValuedParameter.empty` is a picklable class used as the sentinel.
    let empty = py::class(vm, &format!("{MODULE}"), "empty", &[], &[], &[])?;
    py::set(vm, &Value::Obj(empty.clone()), "__qualname__", py::s(vm, "ValuedParameter.empty"))?;
    let vp = py::class(
        vm,
        MODULE,
        "ValuedParameter",
        &[Value::Obj(p)],
        &[("default_value", vp_default_value), ("__str__", vp_str)],
        &[("empty", Value::Obj(empty))],
    )?;
    set(vm, "ValuedParameter", Value::Obj(vp.clone()));
    // `renpy.atl` re-exports it from 7.8 on.
    if super::state::try_rt(vm).is_none_or(|r| r.profile.at_least(7, 8)) {
        if let Ok(atl) = py::import(vm, "renpy.atl") {
            py::set(vm, &atl, "ValuedParameter", Value::Obj(vp))?;
        }
    }
    Ok(())
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[]);
    let module = if super::state::try_rt(vm)
        .map(|r| r.profile.at_least(8, 2))
        .unwrap_or(true)
    {
        MODULE
    } else {
        "renpy.ast"
    };
    let methods: py::Methods = &[
        ("__init__", a_init),
        ("evaluate", a_evaluate),
        ("get_code", a_get_code),
        ("__str__", a_get_code),
        ("__repr__", a_repr),
    ];
    let ai = if starred_layout(vm) {
        let base = py::import_attr(vm, "renpy.object", "Object")?;
        let empty = vm.types.new_set(Default::default());
        let empty2 = vm.types.new_set(Default::default());
        let c = py::class(
            vm,
            module,
            "ArgumentInfo",
            &[base],
            methods,
            &[
                ("__version__", Value::Int(1)),
                ("starred_indexes", empty),
                ("doublestarred_indexes", empty2),
            ],
        )?;
        let f = vm.types.new_builtin("after_upgrade", a_after_upgrade);
        vm.types.set_attr(&c, "after_upgrade", f);
        c
    } else {
        py::class(
            vm,
            module,
            "ArgumentInfo",
            &[],
            methods,
            &[("extrapos", Value::None), ("extrakw", Value::None)],
        )?
    };
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "ArgumentInfo", Value::Obj(ai.clone()));
    let empty = vm.types.new_list(Vec::new());
    let ea = vm.call(
        &Value::Obj(ai),
        vec![empty, Value::None, Value::None],
        Vec::new(),
    )?;
    set(vm, "EMPTY_ARGUMENTS", ea);
    parameter_classes(vm, &m)?;
    if let Ok(s) = py::import_attr(vm, "renpy.atl", "Signature") {
        set(vm, "Signature", s.clone());
        set(vm, "ParameterInfo", s.clone());
        let e = vm.call(&s, Vec::new(), Vec::new())?;
        set(vm, "EMPTY_PARAMETERS", e);
    }
    set(
        vm,
        "apply_arguments",
        vm.types.new_builtin("apply_arguments", f_apply_arguments),
    );
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
