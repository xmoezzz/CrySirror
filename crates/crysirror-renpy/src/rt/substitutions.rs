// SPDX-License-Identifier: MPL-2.0
//! `renpy.substitutions` (`[name]` text interpolation), after Ren'Py
//! `substitutions.py` (MIT) in two forms:
//!
//! * up to 8.1, a `string.Formatter` over `[field!conv:spec]` fields
//!   (`get_field` attribute/index chains); conversions `r s t q` before
//!   7.4, `r s t q u l c i` with validation from 7.4;
//! * from 8.2, `parse`/`interpolate`, where `config.interpolate_exprs`
//!   evaluates fields as Python expressions and `[expr=]` echoes them.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.substitutions";

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm)
        .map(|r| r.profile.at_least(major, minor))
        .unwrap_or(true)
}

/// (literal, field, conversion, format) pieces.
type Field = (String, Option<String>, Option<String>, Option<String>);

/// The pre-8.2 `Formatter.parse`.
fn parse_old(s: &str) -> Result<Vec<Field>, String> {
    #[derive(PartialEq)]
    enum St {
        Literal,
        Open,
        Value,
        Format,
        Conversion,
    }
    let mut out = Vec::new();
    let (mut literal, mut value, mut format) = (String::new(), String::new(), String::new());
    let mut conversion: Option<String> = None;
    let mut depth = 0;
    let mut st = St::Literal;
    let emit = |out: &mut Vec<Field>,
                literal: &mut String,
                value: &mut String,
                format: &mut String,
                conversion: &mut Option<String>| {
        out.push((
            std::mem::take(literal),
            Some(std::mem::take(value)),
            conversion.take(),
            Some(std::mem::take(format)),
        ));
    };
    for c in s.chars() {
        match st {
            St::Literal => {
                if c == '[' {
                    st = St::Open;
                } else {
                    literal.push(c);
                }
            }
            St::Open => {
                if c == '[' {
                    literal.push(c);
                    st = St::Literal;
                } else {
                    value = c.to_string();
                    st = St::Value;
                    depth = 0;
                }
            }
            St::Value => match c {
                '[' => {
                    depth += 1;
                    value.push(c);
                }
                ']' if depth > 0 => {
                    depth -= 1;
                    value.push(c);
                }
                ']' => {
                    emit(
                        &mut out,
                        &mut literal,
                        &mut value,
                        &mut format,
                        &mut conversion,
                    );
                    st = St::Literal;
                }
                ':' => st = St::Format,
                '!' => {
                    st = St::Conversion;
                    conversion = Some(String::new());
                }
                _ => value.push(c),
            },
            St::Format => match c {
                ']' => {
                    emit(
                        &mut out,
                        &mut literal,
                        &mut value,
                        &mut format,
                        &mut conversion,
                    );
                    st = St::Literal;
                }
                '!' => {
                    st = St::Conversion;
                    conversion = Some(String::new());
                }
                _ => format.push(c),
            },
            St::Conversion => {
                if c == ']' {
                    emit(
                        &mut out,
                        &mut literal,
                        &mut value,
                        &mut format,
                        &mut conversion,
                    );
                    st = St::Literal;
                } else {
                    conversion.get_or_insert_with(String::new).push(c);
                }
            }
        }
    }
    if st != St::Literal {
        return Err(format!(
            "String {} ends with an open format operation.",
            py_repr(s)
        ));
    }
    if !literal.is_empty() {
        out.push((literal, None, None, None));
    }
    Ok(out)
}

fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\'', "\\'"))
    }
}

const FLAGS: &str = "rstiqulc!";

/// The 8.2+ `parse`.
fn parse_new(s: &str) -> Result<Vec<Field>, String> {
    #[derive(PartialEq, Clone, Copy)]
    enum St {
        Literal,
        Expression,
        Conversion,
        Format,
    }
    let c: Vec<char> = s.chars().collect();
    let sub = |a: usize, b: usize| -> String { c[a.min(c.len())..b.min(c.len())].iter().collect() };
    let mut out = Vec::new();
    let size = c.len() as isize - 1;
    let mut pos: isize = -1;
    let (mut cut, mut mark) = (0usize, 0usize);
    let (mut brackets, mut parens) = (0, 0);
    let mut lit = String::new();
    let mut expr: Option<String> = None;
    let mut conv: Option<String> = None;
    let mut fmt: Option<String> = None;
    let mut st = St::Literal;
    while pos < size {
        pos += 1;
        let p = pos as usize;
        let ch = c[p];
        match st {
            St::Literal => {
                if ch == '[' {
                    lit.push_str(&sub(cut, p));
                    cut = p + 1;
                    if c.get(p + 1) == Some(&'[') {
                        pos += 1;
                    } else {
                        st = St::Expression;
                    }
                }
            }
            St::Expression => {
                if ch == '(' {
                    parens += 1;
                } else if ch == ')' {
                    if parens == 0 {
                        break;
                    }
                    parens -= 1;
                } else if ch == '"' || ch == '\'' {
                    let mut chars = 1;
                    let mut found = 0;
                    if c.get(p + 1) == Some(&ch) && c.get(p + 2) == Some(&ch) {
                        chars += 2;
                        pos += 2;
                    }
                    while pos < size {
                        pos += 1;
                        let n = c[pos as usize];
                        if n == ch {
                            found += 1;
                            if found == chars {
                                break;
                            }
                        } else {
                            if n == '\\' {
                                pos += 1;
                            }
                            found = 0;
                        }
                    }
                } else if parens > 0 {
                } else if ch == '[' {
                    brackets += 1;
                } else if ch == ']' {
                    if brackets > 0 {
                        brackets -= 1;
                    } else {
                        out.push((std::mem::take(&mut lit), Some(sub(cut, p)), None, None));
                        cut = p + 1;
                        st = St::Literal;
                    }
                } else if brackets > 0 {
                } else if ch == '!' {
                    if c.get(p + 1) == Some(&'=') {
                        pos += 1;
                    } else {
                        st = St::Conversion;
                        expr = Some(sub(cut, p));
                        cut = p + 1;
                    }
                } else if ch == ':' {
                    st = St::Format;
                    expr = Some(sub(cut, p));
                    cut = p + 1;
                }
            }
            St::Conversion => {
                if ch == ']' {
                    out.push((
                        std::mem::take(&mut lit),
                        expr.take(),
                        Some(sub(cut, p)),
                        fmt.take(),
                    ));
                    cut = p + 1;
                    st = St::Literal;
                } else if ch == ':' {
                    st = St::Format;
                    conv = Some(sub(cut, p));
                    cut = p + 1;
                } else if !FLAGS.contains(ch) {
                    if fmt.is_none() {
                        return Err(format!("invalid conversion '{ch}'"));
                    }
                    st = St::Format;
                    pos = cut as isize;
                    cut = mark;
                }
            }
            St::Format => {
                if ch == ']' {
                    out.push((
                        std::mem::take(&mut lit),
                        expr.take(),
                        conv.take(),
                        Some(sub(cut, p)),
                    ));
                    cut = p + 1;
                    st = St::Literal;
                } else if conv.is_none() && ch == '!' {
                    st = St::Conversion;
                    fmt = Some(sub(cut, p));
                    mark = cut;
                    cut = p + 1;
                }
            }
        }
    }
    if st != St::Literal {
        return Err(format!(
            "String {} ends with an open format operation.",
            py_repr(s)
        ));
    }
    if cut as isize <= size {
        lit.push_str(&sub(cut, c.len()));
    }
    if !lit.is_empty() {
        out.push((lit, None, None, None));
    }
    Ok(out)
}

/// The lookup scope: dicts searched in order (`MultipleDict`).
struct Scope(Vec<Value>);

impl Scope {
    fn get(&self, vm: &mut Vm, key: &str) -> PyResult<Option<Value>> {
        let k = vm.types.new_str(key);
        for d in &self.0 {
            if ops::contains(vm, d, &k)? {
                return Ok(Some(ops::getitem(vm, d, k)?));
            }
        }
        Ok(None)
    }

    fn missing(&self, vm: &mut Vm, key: &str) -> crysirror_python::PyErr {
        if self.0.len() == 1 {
            return vm.exc_msg("KeyError", format!("'{key}'"));
        }
        let kind = if at_least(vm, 8, 2) {
            "KeyError"
        } else {
            "NameError"
        };
        vm.exc_msg(kind, format!("Name '{key}' is not defined."))
    }
}

/// `string.Formatter.get_field`: `name(.attr|[index])*`.
fn get_field(vm: &mut Vm, field: &str, scope: &Scope) -> PyResult<Value> {
    let end = field.find(['.', '[']).unwrap_or(field.len());
    let first = &field[..end];
    let mut obj = match scope.get(vm, first)? {
        Some(v) => v,
        None => return Err(scope.missing(vm, first)),
    };
    let mut rest = &field[end..];
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let e = r.find(['.', '[']).unwrap_or(r.len());
            obj = py::get(vm, &obj, &r[..e])?;
            rest = &r[e..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let Some(e) = r.find(']') else {
                return Err(vm.exc_msg("ValueError", "Missing ']' in format string"));
            };
            let key = &r[..e];
            let k = match key.parse::<i64>() {
                Ok(i) => Value::Int(i),
                Err(_) => vm.types.new_str(key),
            };
            obj = ops::getitem(vm, &obj, k)?;
            rest = &r[e + 1..];
        } else {
            return Err(vm.exc_msg(
                "ValueError",
                "Only '.' or '[' may follow ']' in format field specifier",
            ));
        }
    }
    Ok(obj)
}

fn to_str(vm: &mut Vm, v: &Value) -> PyResult<String> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_owned());
    }
    Ok(ops::str(vm, v)?.to_string())
}

fn translate_string(vm: &mut Vm, s: &str) -> PyResult<String> {
    let f = py::import_attr(vm, "renpy.translation", "translate_string")?;
    let v = vm.types.new_str(s);
    let r = vm.call(&f, vec![v], Vec::new())?;
    to_str(vm, &r)
}

/// The conversion flags of the release (`convert_field` / `convert`).
fn convert(vm: &mut Vm, value: Value, conv: &str, scope: &Scope) -> PyResult<Value> {
    if !at_least(vm, 7, 4) {
        let mut value = value;
        if conv.contains('r') {
            {
                let t = ops::repr(vm, &value)?;
                value = vm.types.new_str(&t);
            }
        } else if conv.contains('s') {
            {
                let t = to_str(vm, &value)?;
                value = vm.types.new_str(&t);
            }
        }
        if conv.contains('t') {
            let s = to_str(vm, &value)?;
            {
                let t = translate_string(vm, &s)?;
                value = vm.types.new_str(&t);
            }
        }
        if conv.contains('q') {
            let s = to_str(vm, &value)?;
            value = vm.types.new_str(&s.replace('{', "{{"));
        }
        return Ok(value);
    }
    if conv.is_empty() {
        return Err(vm.exc_msg("ValueError", "Conversion specifier can't be empty."));
    }
    let new = at_least(vm, 8, 2);
    if !new && conv.chars().any(|c| !"rstqulci!".contains(c)) {
        return Err(vm.exc_msg(
            "ValueError",
            "Unknown symbols in conversion specifier, this must use only the \"rstqulci\".",
        ));
    }
    let mut conv: String = conv.to_owned();
    let mut value = value;
    if conv.contains('r') {
        {
            let t = ops::repr(vm, &value)?;
            value = vm.types.new_str(&t);
        }
        conv = conv.replace('r', "");
    } else if conv.contains('s') {
        {
            let t = to_str(vm, &value)?;
            value = vm.types.new_str(&t);
        }
        conv = conv.replace('s', "");
    }
    if conv.is_empty() || (new && conv.chars().all(|c| c == '!')) {
        return Ok(value);
    }
    let mut s = to_str(vm, &value)?;
    if conv.contains('t') {
        s = translate_string(vm, &s)?;
    }
    if conv.contains('i') {
        s = if new {
            interpolate_new(vm, &s, scope)?
        } else if at_least(vm, 8, 0) {
            interpolate_old(vm, &s, scope)?
        } else {
            let v = vm.types.new_str(&s);
            let (r, _) = substitute(vm, &v, &Value::None, false, false)?;
            to_str(vm, &r)?
        };
    }
    if conv.contains('q') {
        s = s.replace('{', "{{");
    }
    if conv.contains('u') {
        s = s.to_uppercase();
    }
    if conv.contains('l') {
        s = s.to_lowercase();
    }
    if conv.contains('c') && !s.is_empty() {
        let mut it = s.chars();
        let f = it.next().unwrap();
        s = f.to_uppercase().collect::<String>() + it.as_str();
    }
    Ok(vm.types.new_str(&s))
}

fn format_value(vm: &mut Vm, v: &Value, spec: &str) -> PyResult<String> {
    let f = py::import_attr(vm, "builtins", "format")?;
    let s = vm.types.new_str(spec);
    let r = vm.call(&f, vec![v.clone(), s], Vec::new())?;
    to_str(vm, &r)
}

/// `formatter.vformat(s, (), scope)` of the old formatter.
fn interpolate_old(vm: &mut Vm, s: &str, scope: &Scope) -> PyResult<String> {
    let pieces = parse_old(s).map_err(|e| py::error(vm, e))?;
    let mut rv = String::new();
    for (lit, field, conv, fmt) in pieces {
        rv.push_str(&lit);
        let Some(field) = field else { continue };
        let mut v = get_field(vm, &field, scope)?;
        if let Some(c) = conv {
            if !c.is_empty() || at_least(vm, 7, 4) {
                v = convert(vm, v, &c, scope)?;
            }
        }
        let spec = fmt.unwrap_or_default();
        let spec = if spec.contains('[') {
            interpolate_old(vm, &spec, scope)?
        } else {
            spec
        };
        rv.push_str(&format_value(vm, &v, &spec)?);
    }
    Ok(rv)
}

fn is_simple_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// The 8.2+ `interpolate`.
fn interpolate_new(vm: &mut Vm, s: &str, scope: &Scope) -> PyResult<String> {
    let pieces = parse_new(s).map_err(|e| vm.exc_msg("ValueError", e))?;
    let mut rv = String::new();
    let exprs = py::config(vm, "interpolate_exprs").unwrap_or(Value::Bool(false));
    for (lit, expr, conv, fmt) in pieces {
        rv.push_str(&lit);
        let Some(expr) = expr else { continue };
        let mut conv = match conv {
            None => String::new(),
            Some(c) if c.is_empty() => {
                return Err(vm.exc_msg("ValueError", "conversion specifier cannot be empty"));
            }
            Some(c) => c,
        };
        let mut code = expr.trim().to_owned();
        if code.is_empty() {
            return Err(vm.exc_msg("ValueError", "expected expression"));
        }
        let mut fmt = fmt;
        if code.ends_with('=') {
            rv.push_str(&expr);
            code.pop();
            if conv.is_empty() && fmt.is_none() {
                conv = "r".into();
            }
        }
        let value = if py::truthy(vm, &exprs)? {
            match scope.get(vm, &code)? {
                Some(v) if is_simple_name(&code) => v,
                _ => {
                    let r = eval_in(vm, &code, scope);
                    match r {
                        Ok(v) => v,
                        Err(e) => {
                            if exprs.as_str() == Some("fallback") {
                                get_field(vm, &code, scope).map_err(|_| e)?
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        } else {
            get_field(vm, &code, scope)?
        };
        let value = if conv.is_empty() {
            value
        } else {
            convert(vm, value, &conv, scope)?
        };
        rv.push_str(&format_value(vm, &value, &fmt.take().unwrap_or_default())?);
    }
    Ok(rv)
}

/// `py_eval(code, {}, scope)` with the scope as a mapping.
fn eval_in(vm: &mut Vm, code: &str, scope: &Scope) -> PyResult<Value> {
    let locals = if scope.0.len() == 1 {
        scope.0[0].clone()
    } else {
        let md = py::import_attr(vm, MODULE, "MultipleDict")?;
        vm.call(&md, scope.0.clone(), Vec::new())?
    };
    let f = py::import_attr(vm, "renpy.python", "py_eval")?;
    let c = vm.types.new_str(code);
    vm.call(&f, vec![c, py::dict(vm), locals], Vec::new())
}

/// `substitute(s, scope=None, force=False, translate=True)` ->
/// (text, changed).
pub fn substitute(
    vm: &mut Vm,
    s: &Value,
    scope: &Value,
    force: bool,
    translate: bool,
) -> PyResult<(Value, bool)> {
    let mut text = to_str(vm, s)?;
    if translate {
        text = translate_string(vm, &text)?;
    }
    if !py::config_bool(vm, "new_substitutions").unwrap_or(true) && !force {
        return Ok((vm.types.new_str(&text), false));
    }
    if !text.contains('[') {
        return Ok((vm.types.new_str(&text), false));
    }
    let mut dicts = Vec::new();
    if !py::is_none(scope) {
        dicts.push(scope.clone());
    }
    let new = at_least(vm, 8, 2);
    if new {
        let sd = py::import_attr(vm, "renpy.python", "store_dicts")?;
        let k = py::s(vm, "store.interpolate");
        if ops::contains(vm, &sd, &k)? {
            dicts.push(ops::getitem(vm, &sd, k)?);
        }
    }
    dicts.push(super::python::store_dict(vm, "store")?);
    let scope = Scope(dicts);
    let r = if new {
        interpolate_new(vm, &text, &scope)
    } else {
        interpolate_old(vm, &text, &scope)
    };
    let out = match r {
        Ok(o) => o,
        Err(e) => {
            let predicting = py::import_attr(vm, "renpy.display.predict", "predicting")
                .unwrap_or(Value::Bool(false));
            if at_least(vm, 7, 0) && py::truthy(vm, &predicting)? {
                return Ok((py::s(vm, " "), true));
            }
            return Err(e);
        }
    };
    let changed = out != text;
    Ok((vm.types.new_str(&out), changed))
}

fn f_substitute(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let scope = arg_or(&args[1..], kw, 0, "scope", Value::None);
    let force = arg_or(&args[1..], kw, 1, "force", Value::Bool(false));
    let translate = arg_or(&args[1..], kw, 2, "translate", Value::Bool(true));
    let (f, t) = (py::truthy(vm, &force)?, py::truthy(vm, &translate)?);
    let (s, changed) = substitute(vm, &args[0], &scope, f, t)?;
    Ok(vm.types.new_tuple(vec![s, Value::Bool(changed)]))
}

fn pieces_value(vm: &mut Vm, p: Vec<Field>) -> Value {
    let opt = |vm: &Vm, s: Option<String>| s.map(|s| vm.types.new_str(&s)).unwrap_or(Value::None);
    let items = p
        .into_iter()
        .map(|(l, e, c, f)| {
            let l = vm.types.new_str(&l);
            let (e, c, f) = (opt(vm, e), opt(vm, c), opt(vm, f));
            vm.types.new_tuple(vec![l, e, c, f])
        })
        .collect();
    vm.types.new_list(items)
}

fn f_parse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let r = if at_least(vm, 8, 2) {
        parse_new(&s)
    } else {
        parse_old(&s)
    };
    match r {
        Ok(p) => Ok(pieces_value(vm, p)),
        Err(e) => Err(vm.exc_msg("ValueError", e)),
    }
}

fn scope_of(vm: &mut Vm, v: &Value) -> PyResult<Scope> {
    if let Some(d) = py::get_opt(vm, v, "dicts")? {
        return Ok(Scope(py::items(vm, &d)?));
    }
    Ok(Scope(vec![v.clone()]))
}

fn f_interpolate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let scope = scope_of(vm, &args[1])?;
    let r = if at_least(vm, 8, 2) {
        interpolate_new(vm, &s, &scope)?
    } else {
        interpolate_old(vm, &s, &scope)?
    };
    Ok(vm.types.new_str(&r))
}

fn f_convert(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::string(vm, &args[1])?;
    let scope = match args.get(2) {
        Some(s) => scope_of(vm, s)?,
        None => Scope(vec![super::python::store_dict(vm, "store")?]),
    };
    convert(vm, args[0].clone(), &c, &scope)
}

fn md_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = vm.types.new_tuple(args[1..].to_vec());
    py::set(vm, &args[0], "dicts", t)?;
    Ok(Value::None)
}

fn md_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let scope = scope_of(vm, &args[0])?;
    let key = py::string(vm, &args[1])?;
    match scope.get(vm, &key)? {
        Some(v) => Ok(v),
        None => Err(scope.missing(vm, &key)),
    }
}

fn md_contains(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let scope = scope_of(vm, &args[0])?;
    let key = py::string(vm, &args[1])?;
    Ok(Value::Bool(scope.get(vm, &key)?.is_some()))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("substitute", f_substitute),
            ("parse", f_parse),
            ("interpolate", f_interpolate),
            ("convert", f_convert),
        ],
    );
    let md = py::class(
        vm,
        MODULE,
        "MultipleDict",
        &[],
        &[
            ("__init__", md_init),
            ("__getitem__", md_getitem),
            ("__contains__", md_contains),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "MultipleDict", Value::Obj(md));
    crysirror_python::modules::set(vm, &m, "update_translations", Value::Bool(false));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
