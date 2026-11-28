// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.shader` (8.2+), after Ren'Py 8.3 `text/shader.py` (MIT):
//! text shader descriptions, `register_textshader`, and the lookup of
//! `textshader` style values. The descriptions are kept as upstream builds
//! them; GLSL text shaders are not compiled by the renderer, which reports
//! text drawn with one.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.text.shader";

const TS_ARGS: &[&str] = &[
    "shader",
    "uniforms",
    "variable_types",
    "extra_slow_time",
    "extra_slow_duration",
    "redraw",
    "redraw_when_slow",
    "include_default",
    "adjust_function",
    "adjust_name_map",
    "doc",
];

fn ts_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v: Vec<Value> = TS_ARGS
        .iter()
        .enumerate()
        .map(|(i, n)| arg_or(&args[1..], kw, i, n, Value::None))
        .collect();
    let parts = if v[0].as_str().is_some() {
        vec![v[0].clone()]
    } else {
        py::items(vm, &v[0])?
    };
    let shader = vm.types.new_tuple(parts);
    let d = |x: &Value, dv: Value| if py::is_none(x) { dv } else { x.clone() };
    py::set(vm, this, "shader", shader.clone())?;
    let est = d(&v[3], Value::Float(0.0));
    let esd = d(&v[4], Value::Float(0.0));
    let rws = d(&v[6], Value::Float(0.0));
    let inc = d(&v[7], Value::Bool(true));
    let uniforms = d(&v[1], vm.types.new_tuple(Vec::new()));
    py::set(vm, this, "extra_slow_time", est.clone())?;
    py::set(vm, this, "extra_slow_duration", esd.clone())?;
    py::set(vm, this, "redraw", v[5].clone())?;
    py::set(vm, this, "redraw_when_slow", rws.clone())?;
    py::set(vm, this, "include_default", inc.clone())?;
    py::set(vm, this, "doc", v[10].clone())?;
    py::set(vm, this, "uniforms", uniforms.clone())?;
    py::set(vm, this, "adjust_function", v[8].clone())?;
    py::set(vm, this, "adjust_name_map", v[9].clone())?;
    py::set(vm, this, "variable_types", v[2].clone())?;
    if !py::is_none(&v[8]) {
        let map = if py::is_none(&v[9]) {
            py::dict(vm)
        } else {
            v[9].clone()
        };
        let mut k = Vec::new();
        for u in py::items(vm, &uniforms)? {
            let p = py::items(vm, &u)?;
            let n = vm.call_method(&map, "get", vec![p[0].clone(), p[0].clone()])?;
            k.push((py::string(vm, &n)?.into(), p[1].clone()));
        }
        vm.call(&v[8], vec![this.clone()], k)?;
    }
    let key = vm
        .types
        .new_tuple(vec![shader, est, esd, v[5].clone(), rws, inc, uniforms]);
    py::set(vm, this, "key", key)?;
    Ok(Value::None)
}

fn ts_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = py::get(vm, &args[0], "key")?;
    Ok(Value::Int(ops::hash(vm, &k)?))
}

fn ts_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "key")?;
    let Some(b) = py::get_opt(vm, &args[1], "key")? else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn combine_redraw(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<Value> {
    if py::is_none(a) {
        return Ok(b.clone());
    }
    if py::is_none(b) {
        return Ok(a.clone());
    }
    Ok(if py::lt(vm, a, b)? {
        a.clone()
    } else {
        b.clone()
    })
}

fn max(vm: &mut Vm, a: &Value, b: &Value) -> PyResult<Value> {
    Ok(if py::gt(vm, b, a)? {
        b.clone()
    } else {
        a.clone()
    })
}

fn since_84(vm: &Vm) -> bool {
    super::textsupport::Rel::of(vm).at_least(8, 4)
}

fn py_combine_redraw(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = arg_or(args, kw, 0, "a", Value::None);
    let b = arg_or(args, kw, 1, "b", Value::None);
    combine_redraw(vm, &a, &b)
}

/// `compute_times(shaders)` -> (extra_slow_time, extra_slow_duration,
/// redraw, redraw_when_slow).
fn py_compute_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let shaders = arg_or(args, kw, 0, "shaders", Value::None);
    let (mut est, mut esd) = (Value::Int(0), Value::Int(0));
    let (mut redraw, mut rws) = (Value::None, Value::None);
    for s in py::items(vm, &shaders)? {
        let v = py::get(vm, &s, "extra_slow_time")?;
        est = max(vm, &est, &v)?;
        let v = py::get(vm, &s, "extra_slow_duration")?;
        esd = max(vm, &esd, &v)?;
        let v = py::get(vm, &s, "redraw")?;
        redraw = combine_redraw(vm, &redraw, &v)?;
        let v = py::get(vm, &s, "redraw_when_slow")?;
        rws = combine_redraw(vm, &rws, &v)?;
    }
    rws = combine_redraw(vm, &rws, &redraw)?;
    Ok(py::tuple(vm, vec![est, esd, redraw, rws]))
}

fn py_to_uniform_value(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let sn = arg_or(args, kw, 0, "shader_name", Value::None);
    let sn = py::string(vm, &sn)?;
    let un = arg_or(args, kw, 1, "uniform_name", Value::None);
    let un = py::string(vm, &un)?;
    let types = arg_or(args, kw, 2, "variable_types", Value::None);
    let value = arg_or(args, kw, 3, "value", Value::None);
    to_uniform_value(vm, &sn, &un, &types, value)
}

fn py_create_args_dict(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", Value::None);
    let name = py::string(vm, &name)?;
    let shader = arg_or(args, kw, 1, "shader", Value::None);
    let s = arg_or(args, kw, 2, "s", Value::None);
    let s = py::string(vm, &s)?;
    create_textshader_args_dict(vm, &name, &shader, &s)
}

fn class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "TextShader")
}

fn ts_combine(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    if py::same(a, b) {
        return Ok(a.clone());
    }
    let g = |vm: &mut Vm, o: &Value, n: &str| py::get(vm, o, n);
    let d = Value::Obj(vm.types.dict.clone());
    let ua = g(vm, a, "uniforms")?;
    let u = vm.call(&d, vec![ua], Vec::new())?;
    let ub = g(vm, b, "uniforms")?;
    let ub = vm.call(&d, vec![ub], Vec::new())?;
    vm.call_method(&u, "update", vec![ub])?;
    let items = vm.call_method(&u, "items", Vec::new())?;
    let items = py::items(vm, &items)?;
    let items = vm.types.new_tuple(items);
    let (sa, sb) = (g(vm, a, "shader")?, g(vm, b, "shader")?);
    let mut sh = py::items(vm, &sa)?;
    sh.extend(py::items(vm, &sb)?);
    let shader = vm.types.new_tuple(sh);
    let (ea, eb) = (g(vm, a, "extra_slow_time")?, g(vm, b, "extra_slow_time")?);
    let est = max(vm, &ea, &eb)?;
    let (da, db) = (
        g(vm, a, "extra_slow_duration")?,
        g(vm, b, "extra_slow_duration")?,
    );
    let esd = max(vm, &da, &db)?;
    let (ra, rb) = (g(vm, a, "redraw")?, g(vm, b, "redraw")?);
    let r = combine_redraw(vm, &ra, &rb)?;
    let (wa, wb) = (g(vm, a, "redraw_when_slow")?, g(vm, b, "redraw_when_slow")?);
    let rws = combine_redraw(vm, &wa, &wb)?;
    let (ia, ib) = (g(vm, a, "include_default")?, g(vm, b, "include_default")?);
    let inc = Value::Bool(py::truthy(vm, &ia)? || py::truthy(vm, &ib)?);
    let c = class(vm)?;
    vm.call(
        &c,
        vec![shader, items, py::dict(vm), est, esd, r, rws, inc],
        Vec::new(),
    )
}

fn ts_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = Value::Obj(vm.types.dict.clone());
    let u = py::get(vm, this, "uniforms")?;
    let u = vm.call(&d, vec![u], Vec::new())?;
    vm.call_method(&u, "update", vec![args[1].clone()])?;
    let items = vm.call_method(&u, "items", Vec::new())?;
    let items = py::items(vm, &items)?;
    let items = vm.types.new_tuple(items);
    let mut a = vec![py::get(vm, this, "shader")?, items];
    for n in [
        "variable_types",
        "extra_slow_time",
        "extra_slow_duration",
        "redraw",
        "redraw_when_slow",
        "include_default",
        "adjust_function",
        "adjust_name_map",
    ] {
        a.push(py::get(vm, this, n)?);
    }
    let c = class(vm)?;
    vm.call(&c, a, Vec::new())
}

fn textshaders(vm: &mut Vm) -> PyResult<Value> {
    py::config(vm, "textshaders")
}

/// `to_uniform_value(shader_name, uniform_name, variable_types, value)`.
fn to_uniform_value(
    vm: &mut Vm,
    shader: &str,
    uniform: &str,
    types: &Value,
    value: Value,
) -> PyResult<Value> {
    let key = py::s(vm, uniform);
    let ty = vm.call_method(types, "get", vec![key, Value::None])?;
    let ty = if py::is_none(&ty) {
        None
    } else {
        Some(py::string(vm, &ty)?)
    };
    let premultiplied = |vm: &mut Vm, v: Value| -> PyResult<Value> {
        let c = py::import_attr(vm, "renpy.easy", "color")?;
        let c = vm.call(&c, vec![v], Vec::new())?;
        py::get(vm, &c, "premultiplied")
    };
    if ty.as_deref() == Some("color") {
        return premultiplied(vm, value);
    }
    let Some(ty) = ty else {
        return Err(vm.value_error(format!("Unknown uniform '{uniform}' in shader '{shader}'.")));
    };
    let mut value = value;
    if let Some(sv) = value.as_str().map(str::to_owned) {
        if ty == "sampler2D" {
            let d = py::import_attr(vm, "renpy.easy", "displayable")?;
            value = vm.call(&d, vec![value], Vec::new())?;
        } else if sv.starts_with('#') {
            value = premultiplied(vm, value)?;
        } else {
            let inner = sv.trim_start_matches('(').trim_end_matches(')');
            let mut nums = Vec::new();
            for p in inner.split(',') {
                match p.trim().parse::<f64>() {
                    Ok(f) => nums.push(Value::Float(f)),
                    Err(e) => {
                        return Err(
                            vm.value_error(format!("Error parsing '{sv}' as a uniform value: {e}"))
                        );
                    }
                }
            }
            value = if nums.len() == 1 {
                nums.remove(0)
            } else {
                vm.types.new_tuple(nums)
            };
        }
    }
    let tuple_len = |v: &Value| match v.payload() {
        Some(crysirror_python::Payload::Tuple(t)) => Some(t.len()),
        _ => None,
    };
    match ty.as_str() {
        "float" => {
            if let Ok(f) = py::f64_of(vm, &value)
                && !matches!(value, Value::Float(_))
            {
                value = Value::Float(f);
            }
            if !matches!(value, Value::Float(_)) {
                return Err(vm.value_error(format!(
                    "Expected a float for '{uniform}' in shader '{shader}'."
                )));
            }
        }
        // int/bool uniforms and integer/boolean vectors are 8.4+.
        "int" if since_84(vm) => {
            let ic = Value::Obj(vm.types.int.clone());
            if let Ok(i) = vm.call(&ic, vec![value.clone()], Vec::new()) {
                value = i;
            }
            if !matches!(value, Value::Int(_) | Value::Bool(_)) {
                return Err(vm.value_error(format!(
                    "Expected an int for '{uniform}' in shader '{shader}'."
                )));
            }
        }
        "bool" if since_84(vm) => {
            value = Value::Bool(ops::truthy(vm, &value)?);
        }
        "vec2" | "vec3" | "vec4" | "ivec2" | "ivec3" | "ivec4" | "bvec2" | "bvec3" | "bvec4"
            if ty.starts_with('v') || since_84(vm) =>
        {
            let n = ty[ty.len() - 1..].parse::<usize>().unwrap_or(0);
            if tuple_len(&value) != Some(n) {
                return Err(vm.value_error(format!(
                    "Expected a {n} component tuple for '{uniform}' in shader '{shader}'."
                )));
            }
        }
        "sampler2D" => {
            let d = py::import_attr(vm, "renpy.display.displayable", "Displayable")
                .or_else(|_| py::import_attr(vm, "renpy.display.core", "Displayable"))?;
            if !d.obj().is_some_and(|c| ops::isinstance(vm, &value, c)) {
                return Err(vm.value_error(format!(
                    "Expected a displayable for '{uniform}' in shader '{shader}'."
                )));
            }
        }
        _ => {}
    }
    Ok(value)
}

/// `register_textshader(name, shaders=(), extra_slow_time=0.0, ...,
/// doc=None, **kwargs)`.
fn f_register_textshader(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let names = [
        "name",
        "shaders",
        "extra_slow_time",
        "extra_slow_duration",
        "redraw",
        "redraw_when_slow",
        "include_default",
        "adjust_function",
        "doc",
    ];
    let v: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| arg_or(args, kw, i, n, Value::None))
        .collect();
    let name = py::string(vm, &v[0])?;
    let mut ts_kwargs: Vec<(String, Value)> = Vec::new();
    let mut part: Vec<(std::rc::Rc<str>, Value)> = Vec::new();
    for (k, val) in kw.iter().filter(|(k, _)| !names.contains(&&**k)) {
        // `glsl=` (the 8.6 shader dialect) goes to register_shader too.
        let glsl = &**k == "glsl" && super::state::rt(vm).profile.at_least(8, 6);
        if &**k == "variables" || glsl || k.starts_with("fragment_") || k.starts_with("vertex_") {
            part.push((k.clone(), val.clone()));
        } else if k.starts_with("u_") {
            ts_kwargs.push((k.to_string(), val.clone()));
        } else {
            return Err(vm.type_error(format!(
                "renpy.register_textshader got an unknown keyword argument '{k}'."
            )));
        }
    }
    let mut shaders = if py::is_none(&v[1]) {
        Vec::new()
    } else if v[1].as_str().is_some() {
        vec![v[1].clone()]
    } else {
        py::items(vm, &v[1])?
    };
    let part_name = format!("textshader.{name}");
    shaders.push(vm.types.new_str(&part_name));
    part.push(("private_uniforms".into(), Value::Bool(true)));
    let rs = py::import_attr(vm, "renpy.exports", "register_shader")?;
    let pn = vm.types.new_str(&part_name);
    let p = vm.call(&rs, vec![pn], part)?;
    let doc = if py::is_none(&v[8]) {
        Value::None
    } else {
        py::method(vm, &p, "substitute_name", vec![v[8].clone()])?
    };
    let mapped = py::dict(vm);
    let expanded = py::dict(vm);
    for (k, val) in &ts_kwargs {
        let kv = py::s(vm, k);
        let e = py::method(vm, &p, "expand_name", vec![kv.clone()])?;
        ops::setitem(vm, &mapped, e.clone(), kv)?;
        ops::setitem(vm, &expanded, e, val.clone())?;
    }
    let types = py::get(vm, &p, "variable_types")?;
    let mut uniforms = Vec::new();
    for k in py::get_items(vm, &p, "uniforms")? {
        if ops::contains(vm, &expanded, &k)? {
            let raw = ops::getitem(vm, &expanded, k.clone())?;
            let ks = py::string(vm, &k)?;
            let val = to_uniform_value(vm, &name, &ks, &types, raw)?;
            uniforms.push(vm.types.new_tuple(vec![k, val]));
        }
    }
    let d = |x: &Value, dv: Value| if py::is_none(x) { dv } else { x.clone() };
    let c = class(vm)?;
    let shader = vm.types.new_tuple(shaders);
    let u = vm.types.new_tuple(uniforms);
    let ts = vm.call(
        &c,
        vec![
            shader,
            u,
            types,
            d(&v[2], Value::Float(0.0)),
            d(&v[3], Value::Float(0.0)),
            v[4].clone(),
            d(&v[5], Value::Float(0.0)),
            d(&v[6], Value::Bool(true)),
            v[7].clone(),
            mapped,
            doc,
        ],
        Vec::new(),
    )?;
    let reg = textshaders(vm)?;
    ops::setitem(vm, &reg, v[0].clone(), ts)?;
    Ok(Value::None)
}

/// `create_textshader_args_dict(name, shader, s)`: the uniforms given by
/// the `name:value:u_x=value` mini-language.
fn create_textshader_args_dict(
    vm: &mut Vm,
    name: &str,
    shader: &Value,
    s: &str,
) -> PyResult<Value> {
    let rv = py::dict(vm);
    let uniforms = py::get_items(vm, shader, "uniforms")?;
    let types = py::get(vm, shader, "variable_types")?;
    for (i, a) in s.split(':').enumerate() {
        let (uniform, value) = match a.split_once('=') {
            Some((u, v)) => (u.trim().to_owned(), v.trim().to_owned()),
            None => {
                let Some(u) = uniforms.get(i) else {
                    return Err(
                        vm.value_error(format!("Too many uniforms given to to shader part: '{s}'"))
                    );
                };
                let k = ops::getitem(vm, u, Value::Int(0))?;
                (py::string(vm, &k)?.trim().to_owned(), a.trim().to_owned())
            }
        };
        let mut uniform = if uniform.starts_with("u_") {
            uniform
        } else {
            format!("u_{uniform}")
        };
        if let Some(rest) = uniform.strip_prefix("u__") {
            uniform = format!("u_textshader_{}_{rest}", name.replace('.', "_"));
        }
        let mut known = false;
        for u in &uniforms {
            let k = ops::getitem(vm, u, Value::Int(0))?;
            known |= k.as_str() == Some(uniform.as_str());
        }
        if !known {
            return Err(vm.value_error(format!("Unknown uniform '{uniform}' in '{name}'.")));
        }
        let vv = py::s(vm, &value);
        let val = to_uniform_value(vm, name, &uniform, &types, vv)?;
        py::dict_set(vm, &rv, &uniform, val)?;
    }
    Ok(rv)
}

fn parse_textshader(vm: &mut Vm, o: &Value) -> PyResult<Value> {
    if py::is_none(o) {
        return Ok(Value::None);
    }
    let c = class(vm)?;
    if c.obj().is_some_and(|c| ops::isinstance(vm, o, c)) {
        return Ok(o.clone());
    }
    let Some(s) = o.as_str().map(str::to_owned) else {
        let r = ops::repr(vm, o)?;
        return Err(py::error(
            vm,
            format!("Expected a TextShader, but got {r}."),
        ));
    };
    if s.contains('|') {
        let mut rv = Value::None;
        for part in s.split('|') {
            let p = vm.types.new_str(part);
            let t = parse_textshader(vm, &p)?;
            rv = if py::is_none(&rv) {
                t
            } else {
                vm.call_method(&rv, "combine", vec![t])?
            };
        }
        return Ok(rv);
    }
    let (n, rest) = s.split_once(':').unwrap_or((&s, ""));
    let n = n.trim();
    let reg = textshaders(vm)?;
    let key = vm.types.new_str(n);
    let rv = vm.call_method(&reg, "get", vec![key, Value::None])?;
    if py::is_none(&rv) {
        return Err(py::error(vm, format!("Unknown text shader '{s}'.")));
    }
    if rest.is_empty() {
        return Ok(rv);
    }
    let d = create_textshader_args_dict(vm, n, &rv, rest)?;
    vm.call_method(&rv, "copy", vec![d])
}

fn f_parse_textshader(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    parse_textshader(vm, &args[0])
}

fn lookup(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let mut name = name.clone();
    let cbs = py::config(vm, "textshader_callbacks").unwrap_or(Value::None);
    if !py::is_none(&cbs) && ops::contains(vm, &cbs, &name)? {
        let f = ops::getitem(vm, &cbs, name.clone())?;
        name = vm.call(&f, Vec::new(), Vec::new())?;
    }
    let cache = py::import_attr(vm, MODULE, "parsed_shader_cache")?;
    if !py::is_none(&name) && ops::contains(vm, &cache, &name)? {
        return ops::getitem(vm, &cache, name);
    }
    let rv = parse_textshader(vm, &name)?;
    if !py::is_none(&name) {
        ops::setitem(vm, &cache, name, rv.clone())?;
    }
    Ok(rv)
}

fn f_get_textshader(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[0]) {
        return Ok(Value::None);
    }
    let rv = lookup(vm, &args[0])?;
    let inc = py::get(vm, &rv, "include_default")?;
    if !py::truthy(vm, &inc)? {
        return Ok(rv);
    }
    let dts = py::config(vm, "default_textshader").unwrap_or(Value::None);
    let default = lookup(vm, &dts)?;
    if py::is_none(&default) {
        return Ok(rv);
    }
    vm.call_method(&default, "combine", vec![rv])
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("register_textshader", f_register_textshader),
            ("parse_textshader", f_parse_textshader),
            ("get_textshader", f_get_textshader),
            ("combine_redraw", py_combine_redraw),
            ("compute_times", py_compute_times),
            ("to_uniform_value", py_to_uniform_value),
            ("create_textshader_args_dict", py_create_args_dict),
        ],
    );
    let c = py::class(
        vm,
        MODULE,
        "TextShader",
        &[],
        &[
            ("__init__", ts_init),
            ("__hash__", ts_hash),
            ("__eq__", ts_eq),
            ("combine", ts_combine),
            ("copy", ts_copy),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "TextShader", Value::Obj(c));
    crysirror_python::modules::set(vm, &m, "parsed_shader_cache", py::dict(vm));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
