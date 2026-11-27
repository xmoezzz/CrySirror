// SPDX-License-Identifier: MPL-2.0
//! Argument binding of `renpy.parameter` (8.5) / `renpy.ast` (≤ 8.4):
//! `Signature.apply`, `apply_arguments` and `ArgumentInfo.evaluate`, used
//! by screens, labels and transforms. Defaults are evaluated lazily in the
//! store, as upstream `Parameter.default_value` does.
use super::py;
use crate::{
    ast::{ArgumentInfo, Signature},
    lexer::PyExpr,
};
use crysirror_python::{PyResult, Value, Vm, ops};
use std::rc::Rc;

pub type Loc = (Rc<str>, u32);

pub fn eval_text(vm: &mut Vm, text: &str, loc: &Loc, locals: &Value) -> PyResult<Value> {
    super::atlrt::eval(vm, &PyExpr::new(text, loc.0.clone(), loc.1), locals)
}

fn revertable_dict(vm: &mut Vm) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.revertable", "RevertableDict")?;
    vm.call(&c, Vec::new(), Vec::new())
}

/// `ArgumentInfo.evaluate(scope)` -> (tuple, RevertableDict).
pub fn evaluate(vm: &mut Vm, info: &ArgumentInfo, loc: &Loc, scope: &Value) -> PyResult<(Value, Value)> {
    let mut args = Vec::new();
    let kwargs = revertable_dict(vm)?;
    for (i, (k, v)) in info.arguments.iter().enumerate() {
        let value = eval_text(vm, v, loc, scope)?;
        if info.starred.contains(&i) {
            args.extend(ops::collect(vm, &value)?);
        } else if info.doublestarred.contains(&i) {
            vm.call_method(&kwargs, "update", vec![value])?;
        } else if let Some(k) = k {
            py::dict_set(vm, &kwargs, k, value)?;
        } else {
            args.push(value);
        }
    }
    Ok((vm.types.new_tuple(args), kwargs))
}

const POS_ONLY: u8 = 0;
const VAR_POS: u8 = 2;
const KW_ONLY: u8 = 3;
const VAR_KW: u8 = 4;

/// `Signature.apply(args, kwargs, ignore_errors)`: the callee scope dict.
pub fn apply(vm: &mut Vm, sig: &Signature, loc: &Loc, args: &[Value], kwargs: &[(Rc<str>, Value)], mut ignore_errors: bool) -> PyResult<Value> {
    if !py::config_bool(vm, "developer").unwrap_or(false) {
        ignore_errors = true;
    }
    let mut kwargs: Vec<(Rc<str>, Value)> = kwargs.to_vec();
    let take = |kw: &mut Vec<(Rc<str>, Value)>, n: &str| kw.iter().position(|(k, _)| &**k == n).map(|i| kw.remove(i).1);
    let has = |kw: &Vec<(Rc<str>, Value)>, n: &str| kw.iter().any(|(k, _)| &**k == n);
    let out = vm.types.new_dict();
    let params = &sig.parameters;
    let mut pi = 0;
    let mut ai = 0;
    let fail = |vm: &Vm, msg: String| Err(vm.type_error(msg));
    // Positional arguments.
    loop {
        if ai >= args.len() {
            break;
        }
        let Some(p) = params.get(pi) else {
            if !ignore_errors {
                return fail(vm, "too many positional arguments".into());
            }
            break;
        };
        if p.kind == VAR_KW || p.kind == KW_ONLY {
            if !ignore_errors {
                return fail(vm, "too many positional arguments".into());
            }
            break;
        }
        if p.kind == VAR_POS {
            let t = vm.types.new_tuple(args[ai..].to_vec());
            py::dict_set(vm, &out, &p.name, t)?;
            pi += 1;
            break;
        }
        if has(&kwargs, &p.name) && p.kind != POS_ONLY {
            if !ignore_errors {
                return fail(vm, format!("multiple values for argument {:?}", p.name));
            }
            break;
        }
        py::dict_set(vm, &out, &p.name, args[ai].clone())?;
        ai += 1;
        pi += 1;
    }
    // Remaining parameters from keywords.
    let mut kwargs_param = None;
    for p in &params[pi.min(params.len())..] {
        match p.kind {
            VAR_KW => {
                kwargs_param = Some(p.name.clone());
                continue;
            }
            VAR_POS => continue,
            _ => {}
        }
        match take(&mut kwargs, &p.name) {
            None => {
                if !ignore_errors && p.default.is_none() {
                    return fail(vm, format!("missing a required argument: {:?}", p.name));
                }
            }
            Some(v) => {
                if p.kind == POS_ONLY {
                    if !ignore_errors {
                        return fail(vm, format!("{:?} parameter is positional only, but was passed as a keyword", p.name));
                    }
                    continue;
                }
                py::dict_set(vm, &out, &p.name, v)?;
            }
        }
    }
    if !kwargs.is_empty() {
        if let Some(n) = kwargs_param {
            let d = revertable_dict(vm)?;
            for (k, v) in kwargs {
                py::dict_set(vm, &d, &k, v)?;
            }
            py::dict_set(vm, &out, &n, d)?;
        } else {
            let ignore_extra = take(&mut kwargs, "_ignore_extra_kwargs");
            let ignore_extra = match ignore_extra {
                Some(v) => ops::truthy(vm, &v)?,
                None => false,
            };
            if !(ignore_errors || ignore_extra) {
                return fail(vm, format!("got an unexpected keyword argument {:?}", kwargs[0].0));
            }
        }
    }
    apply_defaults(vm, sig, loc, &out)?;
    Ok(out)
}

/// Fills unbound parameters: evaluated defaults, `()` and `{}`.
pub fn apply_defaults(vm: &mut Vm, sig: &Signature, loc: &Loc, out: &Value) -> PyResult<()> {
    for p in &sig.parameters {
        let k = vm.types.new_str(&p.name);
        if ops::contains(vm, out, &k)? {
            continue;
        }
        let v = if let Some(d) = &p.default {
            eval_text(vm, d, loc, &Value::None)?
        } else if p.kind == VAR_POS {
            vm.types.new_tuple(Vec::new())
        } else if p.kind == VAR_KW {
            revertable_dict(vm)?
        } else {
            continue;
        };
        ops::setitem(vm, out, k, v)?;
    }
    Ok(())
}

/// `renpy.ast.apply_arguments(parameters, args, kwargs, ignore_errors)`.
pub fn apply_arguments(vm: &mut Vm, sig: Option<&Signature>, loc: &Loc, args: &[Value], kwargs: &[(Rc<str>, Value)], mut ignore_errors: bool) -> PyResult<Value> {
    if !py::config_bool(vm, "developer").unwrap_or(false) {
        ignore_errors = true;
    }
    match sig {
        Some(s) => apply(vm, s, loc, args, kwargs, ignore_errors),
        None => {
            if (!args.is_empty() || !kwargs.is_empty()) && !ignore_errors {
                return Err(py::error(vm, "Arguments supplied, but parameter list not present"));
            }
            Ok(vm.types.new_dict())
        }
    }
}

/// Splits a Python (tuple, mapping) pair into Rust argument vectors.
pub fn split(vm: &mut Vm, args: &Value, kwargs: &Value) -> PyResult<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
    let a = if py::is_none(args) { Vec::new() } else { py::items(vm, args)? };
    let k = if py::is_none(kwargs) { Vec::new() } else { py::str_items(vm, kwargs)? };
    Ok((a, k))
}
