// SPDX-License-Identifier: MPL-2.0
//! `renpy.gl2.gl2shadercache`, after Ren'Py 7.3 – 8.6
//! `renpy/gl2/gl2shadercache.py` (MIT): the shader-part registry
//! (`register_shader`, `ShaderPart`), shader source generation (`source`),
//! the GLSL dialect tables and translation of 8.6, and `ShaderCache`.
//!
//! Release differences follow upstream: 7.3 – 8.3 keep variables as
//! `(storage, type, name)` tuples and parts as `(priority, text)`; 8.4
//! parses variables into `renpy.gl2.gl2shader.Variable` objects and keeps
//! `(priority, part name, text)` parts; 8.6 adds the `glsl=` dialect
//! argument, GLSL ES 1.00 <-> 3.00 translation, dialect checks and the
//! program-binary cache. `//` comments in variables are stripped from
//! 7.6/8.1, `private_uniforms`, name substitution and `variable_types` exist
//! from 7.8/8.3, operation names (`u_x__y`) from 8.4.
//!
//! Unavailable host: CrySirror draws through its own renderer, not an
//! OpenGL context. Shader source generation, dialect checks and the
//! registry work; compiling a program (`ShaderCache.get` of a combination
//! that is not cached) raises an exception naming the missing GL driver,
//! and program binaries are never supported (`program_binary_supported` is
//! False, as upstream when the driver lacks `GL_ARB_get_program_binary`).
//! When `renpy.gl2.gl2shader` cannot be imported, `Variable` objects come
//! from a native implementation of the same parser kept as `_Variable`.
use super::{
    py::{self, *},
    state::rt,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::collections::HashSet;

const M: &str = "renpy.gl2.gl2shadercache";

fn at_least(vm: &Vm, a: u32, b: u32) -> bool {
    rt(vm).profile.at_least(a, b)
}

fn since_pair(vm: &Vm, a: u32, b: u32) -> bool {
    rt(vm).profile.since_pair(a, b)
}

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, M, n)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `re.finditer(r"\b\w+\b", s)` words.
pub fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !is_word(c))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

// Name substitution ---------------------------------------------------------------------

fn expand_name(part: &str, s: &str) -> String {
    let name = part.replace('.', "_");
    for p in ["u", "a", "v", "l"] {
        if let Some(rest) = s.strip_prefix(&format!("{p}__")) {
            return format!("{p}_{name}_{rest}");
        }
    }
    s.to_owned()
}

/// `re.sub(r'[uavl]__\w+', expand_match, s)` (8.4 adds the leading `\b`).
fn substitute_names(part: &str, s: &str, boundary: bool) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < c.len() {
        let at_boundary = i == 0 || !is_word(c[i - 1]);
        if "uavl".contains(c[i])
            && (!boundary || at_boundary)
            && c.get(i + 1) == Some(&'_')
            && c.get(i + 2) == Some(&'_')
            && c.get(i + 3).is_some_and(|x| is_word(*x))
        {
            let mut j = i + 3;
            while j < c.len() && is_word(c[j]) {
                j += 1;
            }
            let m: String = c[i..j].iter().collect();
            out.push_str(&expand_name(part, &m));
            i = j;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// `re.sub(r"\bu_(\w+)__(\w+)", expand_operation, s)`: the greedy first
/// group ends at the last `__` of the word that leaves a second group.
fn substitute_operations(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < c.len() {
        if !is_word(c[i]) {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < c.len() && is_word(c[j]) {
            j += 1;
        }
        let w: String = c[i..j].iter().collect();
        let mut done = false;
        if let Some(body) = w.strip_prefix("u_") {
            let b: Vec<char> = body.chars().collect();
            let mut k = b.len();
            while k >= 2 {
                k -= 1;
                if k >= 2 && b[k - 1] == '_' && b[k - 2] == '_' && k < b.len() && k - 2 >= 1 {
                    let g1: String = b[..k - 2].iter().collect();
                    let g2: String = b[k..].iter().collect();
                    out.push_str(&format!("u_{g1}_OP_{g2}"));
                    done = true;
                    break;
                }
            }
        }
        if !done {
            out.push_str(&w);
        }
        i = j;
    }
    out
}

fn substitute(vm: &Vm, part: &str, s: &str) -> String {
    let v84 = at_least(vm, 8, 4);
    let rv = substitute_names(part, s, v84);
    if v84 { substitute_operations(&rv) } else { rv }
}

fn part_name(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let n = py::get(vm, this, "name")?;
    py::string(vm, &n)
}

fn sp_expand_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = part_name(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    Ok(py::s(vm, &expand_name(&p, &s)))
}

fn sp_expand_match(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let g = py::method(vm, &args[1], "group", vec![Value::Int(0)])?;
    sp_expand_name(vm, &[args[0].clone(), g], &[])
}

fn sp_expand_operation(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::method(vm, &args[1], "group", vec![Value::Int(1)])?;
    let b = py::method(vm, &args[1], "group", vec![Value::Int(2)])?;
    let (a, b) = (py::string(vm, &a)?, py::string(vm, &b)?);
    Ok(py::s(vm, &format!("u_{a}_OP_{b}")))
}

fn sp_substitute_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = part_name(vm, &args[0])?;
    let s = py::string(vm, &args[1])?;
    Ok(py::s(vm, &substitute(vm, &p, &s)))
}

// Variable (renpy.gl2.gl2shader.Variable) ------------------------------------------------

const ATTRIBUTE_TYPES: &[&str] = &["float", "vec2", "vec3", "vec4"];
const UNIFORM_TYPES_84: &[&str] = &[
    "float", "vec2", "vec3", "vec4", "int", "ivec2", "ivec3", "ivec4", "bool", "bvec2", "bvec3",
    "bvec4", "mat2", "mat3", "mat4", "sampler2D",
];
const UNIFORM_TYPES_86_EXTRA: &[&str] = &["sampler3D", "samplerCube", "sampler2DArray"];
const PRECISIONS: &[&str] = &["highp", "mediump", "lowp"];
const PRECISION_RANK: &[Option<&str>] = &[None, Some("lowp"), Some("mediump"), Some("highp")];
const INTERPOLATIONS: &[&str] = &["flat", "smooth", "noperspective"];
const INTERPOLATION_RANK: &[Option<&str>] =
    &[None, Some("smooth"), Some("noperspective"), Some("flat")];
const FLAT_TYPES: &[&str] = &["int", "ivec2", "ivec3", "ivec4", "bool", "bvec2", "bvec3", "bvec4"];

#[derive(Default, Clone)]
struct VarSpec {
    storage: Option<&'static str>,
    ty: Option<String>,
    name: Option<String>,
    array: Option<i64>,
    line: String,
    precision: Option<String>,
    invariant: bool,
    interpolation: Option<String>,
}

/// `Variable.__init__(shader_name, line, fragment=False)` of 8.4 – 8.6.
fn parse_variable(v86: bool, shader: &str, line: &str, fragment: bool) -> Result<VarSpec, String> {
    let mut l: &str = line.trim().trim_end_matches([';', ' ']);
    let mut rv = VarSpec { line: l.to_owned(), ..Default::default() };
    fn word<'a>(l: &mut &'a str) -> Option<&'a str> {
        let t = l.trim_start();
        let n = t.find(|c: char| !is_word(c)).unwrap_or(t.len());
        if n == 0 {
            return None;
        }
        let w = &t[..n];
        *l = &t[n..];
        Some(w)
    }
    fn array(l: &mut &str) -> Option<i64> {
        let t = l.trim_start().strip_prefix('[')?.trim_start();
        let n = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
        if n == 0 {
            return None;
        }
        let v = t[..n].parse().ok()?;
        let rest = t[n..].trim_start().strip_prefix(']')?;
        *l = rest;
        Some(v)
    }
    let mut token = word(&mut l);
    if v86 && token == Some("layout") {
        let t = l.trim_start();
        if t.starts_with('(')
            && let Some(e) = t.find(')')
        {
            l = &t[e + 1..];
        }
        if word(&mut l) == Some("out") {
            return Err(format!(
                "In {shader}, '{}' declares an additional fragment output. This syntax is reserved for multiple render targets. Location 0 is Ren'Py's fragment_color; outputs declared by a shader part will start at 1.",
                line.trim()
            ));
        }
        return Err(format!(
            "In {shader}, layout qualifiers are not supported, in '{}'. Declare the variable without one.",
            line.trim()
        ));
    }
    if v86 {
        loop {
            match token {
                Some("invariant") => rv.invariant = true,
                Some(t) if INTERPOLATIONS.contains(&t) => rv.interpolation = Some(t.to_owned()),
                _ => break,
            }
            token = word(&mut l);
        }
    } else if token == Some("invariant") {
        token = word(&mut l);
    }
    let uniform: Vec<&str> = if v86 {
        UNIFORM_TYPES_84.iter().chain(UNIFORM_TYPES_86_EXTRA).copied().collect()
    } else {
        UNIFORM_TYPES_84.to_vec()
    };
    let varying: Vec<&str> = uniform.clone();
    let (storage, types): (&'static str, Vec<&str>) = match token {
        Some("uniform") => ("uniform", uniform),
        Some("attribute") => ("attribute", ATTRIBUTE_TYPES.to_vec()),
        Some("varying") => ("varying", varying),
        Some("out") if v86 => ("varying", varying),
        Some("in") if v86 && fragment => ("varying", varying),
        Some("in") if v86 => ("attribute", ATTRIBUTE_TYPES.to_vec()),
        _ => return Ok(rv),
    };
    rv.storage = Some(storage);
    if v86 && rv.invariant && storage != "varying" {
        return Err(format!(
            "In {shader}, invariant can only qualify a varying, not '{storage}', in '{line}'."
        ));
    }
    if let Some(i) = &rv.interpolation
        && storage != "varying"
    {
        return Err(format!(
            "In {shader}, {i} can only qualify a varying, not '{storage}', in '{line}'."
        ));
    }
    let mut token = word(&mut l);
    if let Some(t) = token
        && PRECISIONS.contains(&t)
    {
        rv.precision = Some(t.to_owned());
        token = word(&mut l);
    }
    let Some(t) = token.filter(|t| types.contains(t)) else {
        let t = token.unwrap_or("None");
        return Err(if v86 {
            format!(
                "In {shader}, Unsupported type {t} in '{line}'. Only float, int, bool, vec<2-4>, ivec<2-4>, bvec<2-4>, mat<2-4>, sampler2D, sampler3D, samplerCube, and sampler2DArray are supported."
            )
        } else {
            format!(
                "In {shader}, Unsupported type {t} in '{line}'. Only float, int, bool, vec<2-4>, ivec<2-4>, bvec<2-4>, mat<2-4>, and sampler2D are supported."
            )
        });
    };
    rv.ty = Some(t.to_owned());
    rv.array = array(&mut l);
    rv.name = word(&mut l).map(str::to_owned);
    if rv.name.is_none() {
        return Err(format!("In {shader}, couldn't find name in '{line}'."));
    }
    if rv.array.is_none() {
        rv.array = array(&mut l);
    }
    if !l.trim_end().is_empty() {
        return Err(format!("Spurious tokens after the name in '{line}'."));
    }
    Ok(rv)
}

fn opt_s(vm: &Vm, s: &Option<String>) -> Value {
    s.as_ref().map(|s| vm.types.new_str(s)).unwrap_or(Value::None)
}

fn shader_error(vm: &mut Vm, msg: String) -> crysirror_python::PyErr {
    match py::import_attr(vm, "renpy.gl2.gl2shader", "ShaderError") {
        Ok(c) => match vm.call(&c, vec![py::s(vm, &msg)], Vec::new()) {
            Ok(e) => match e.obj() {
                Some(o) => crysirror_python::PyErr::Exc(o.clone()),
                None => py::error(vm, msg),
            },
            Err(e) => e,
        },
        Err(_) => py::error(vm, msg),
    }
}

fn var_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let shader = py::string(vm, &py::arg_or(args, kw, 1, "shader_name", Value::None))?;
    let line = py::string(vm, &py::arg_or(args, kw, 2, "line", Value::None))?;
    let fragment = py::arg_or(args, kw, 3, "fragment", Value::Bool(false));
    let fragment = py::truthy(vm, &fragment)?;
    let v86 = at_least(vm, 8, 6);
    let spec = match parse_variable(v86, &shader, &line, fragment) {
        Ok(s) => s,
        Err(m) => return Err(shader_error(vm, m)),
    };
    py::set(vm, this, "line", py::s(vm, &spec.line))?;
    let storage = spec.storage.map(|s| py::s(vm, s)).unwrap_or(Value::None);
    py::set(vm, this, "storage", storage)?;
    py::set(vm, this, "type", opt_s(vm, &spec.ty))?;
    py::set(vm, this, "name", opt_s(vm, &spec.name))?;
    py::set(vm, this, "array", spec.array.map(Value::Int).unwrap_or(Value::None))?;
    if v86 {
        py::set(vm, this, "shader_name", py::s(vm, &shader))?;
        py::set(vm, this, "precision", opt_s(vm, &spec.precision))?;
        py::set(vm, this, "invariant", Value::Bool(spec.invariant))?;
        py::set(vm, this, "interpolation", opt_s(vm, &spec.interpolation))?;
        py::set(vm, this, "interpolation_warnings", vm.types.new_set(Default::default()))?;
    }
    Ok(Value::None)
}

fn var_key(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let mut k = Vec::new();
    for n in ["storage", "type", "name", "array"] {
        k.push(py::get_or_none(vm, v, n)?);
    }
    Ok(vm.types.new_tuple(k))
}

fn var_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = var_key(vm, &args[0])?;
    Ok(Value::Int(ops::hash(vm, &k)?))
}

fn var_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = var_key(vm, &args[0])?;
    let b = var_key(vm, &args[1])?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn opt_str_attr(vm: &mut Vm, v: &Value, n: &str) -> PyResult<Option<String>> {
    let x = py::get_or_none(vm, v, n)?;
    if py::is_none(&x) { Ok(None) } else { Ok(Some(py::string(vm, &x)?)) }
}

/// `Variable.declaration(fragment, gles, version)` (8.6).
fn var_declaration(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let fragment = py::truthy(vm, &args[1])?;
    let gles = py::truthy(vm, &args[2])?;
    let version = py::int(vm, &args[3])?;
    let modern = version >= 300;
    let storage = opt_str_attr(vm, this, "storage")?.unwrap_or_default();
    let ty = opt_str_attr(vm, this, "type")?.unwrap_or_default();
    let name = opt_str_attr(vm, this, "name")?.unwrap_or_default();
    let invariant = py::get_or_none(vm, this, "invariant")?;
    let invariant = py::truthy(vm, &invariant)?;
    let precision = opt_str_attr(vm, this, "precision")?;
    let shader = opt_str_attr(vm, this, "shader_name")?.unwrap_or_default();
    let s = if storage == "uniform" {
        "uniform"
    } else if !modern {
        storage.as_str()
    } else if storage == "attribute" {
        "in"
    } else if fragment {
        "in"
    } else {
        "out"
    };
    let mut rv: Vec<String> = Vec::new();
    if invariant && !(gles && version == 300 && fragment) {
        rv.push("invariant".into());
    }
    let mut interpolation = opt_str_attr(vm, this, "interpolation")?;
    if interpolation.is_none() && storage == "varying" && FLAT_TYPES.contains(&ty.as_str()) {
        interpolation = Some("flat".into());
    }
    let unsupported = (interpolation.as_deref() == Some("flat") && !modern)
        || (interpolation.as_deref() == Some("noperspective") && (!modern || gles));
    if unsupported {
        let dialect = if gles {
            format!("GLSL ES {version}")
        } else {
            format!("desktop GLSL {version}")
        };
        let i = interpolation.clone().unwrap_or_default();
        let message = format!(
            "In shader {shader}, {i} interpolation on {name} is not supported by {dialect}; it will be omitted."
        );
        if py::config_bool(vm, "developer").unwrap_or(false) {
            return Err(shader_error(vm, message));
        }
        let warnings = py::get(vm, this, "interpolation_warnings")?;
        let w = vm.types.new_tuple(vec![py::s(vm, &i), Value::Bool(gles), Value::Int(version)]);
        if !ops::contains(vm, &warnings, &w)? {
            ops::set_add(vm, &warnings, w)?;
            super::log::write(vm, &message);
        }
        interpolation = None;
    }
    if modern
        && storage == "varying"
        && let Some(i) = &interpolation
    {
        rv.push(i.clone());
    }
    rv.push(s.to_owned());
    if let Some(p) = precision
        && version != 120
    {
        rv.push(p);
    }
    rv.push(ty);
    let array = py::get_or_none(vm, this, "array")?;
    if py::is_none(&array) {
        rv.push(name);
    } else {
        let a = py::int(vm, &array)?;
        rv.push(format!("{name}[{a}]"));
    }
    Ok(py::s(vm, &rv.join(" ")))
}

fn rank(table: &[Option<&str>], v: &Option<String>) -> usize {
    table.iter().position(|x| x.map(str::to_owned) == *v).unwrap_or(0)
}

/// `Variable.merge(other)` (8.6).
fn var_merge(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = (&args[0], &args[1]);
    let q = |vm: &mut Vm, v: &Value| -> PyResult<(Option<String>, bool, Option<String>)> {
        let inv = py::get_or_none(vm, v, "invariant")?;
        Ok((
            opt_str_attr(vm, v, "precision")?,
            py::truthy(vm, &inv)?,
            opt_str_attr(vm, v, "interpolation")?,
        ))
    };
    let qa = q(vm, a)?;
    let qb = q(vm, b)?;
    if qa == qb {
        return Ok(a.clone());
    }
    let copy = py::import_attr(vm, "copy", "copy")?;
    let rv = vm.call(&copy, vec![a.clone()], Vec::new())?;
    py::set(vm, &rv, "invariant", Value::Bool(qa.1 || qb.1))?;
    let p = if rank(PRECISION_RANK, &qb.0) > rank(PRECISION_RANK, &qa.0) { qb.0 } else { qa.0 };
    py::set(vm, &rv, "precision", opt_s(vm, &p))?;
    let i = if rank(INTERPOLATION_RANK, &qb.2) > rank(INTERPOLATION_RANK, &qa.2) {
        qb.2
    } else {
        qa.2
    };
    py::set(vm, &rv, "interpolation", opt_s(vm, &i))?;
    Ok(rv)
}

fn variable_class(vm: &mut Vm) -> PyResult<Value> {
    if let Ok(c) = py::import_attr(vm, "renpy.gl2.gl2shader", "Variable") {
        return Ok(c);
    }
    mget(vm, "_Variable")
}

// Dialects (8.6) --------------------------------------------------------------------------

const DIALECTS: [i64; 2] = [100, 300];

fn dialect_versions(d: i64) -> (i64, i64) {
    if d == 100 { (100, 120) } else { (300, 330) }
}

fn glsl_dialect(version: i64) -> Option<i64> {
    match version {
        100 | 120 => Some(100),
        300 | 330 => Some(300),
        _ => None,
    }
}

fn dialect_name(d: i64) -> String {
    format!("GLSL ES {}.{:02}", d / 100, d % 100)
}

fn f_dialect_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::int(vm, &args[0])?;
    Ok(py::s(vm, &dialect_name(d)))
}

fn config_dialect(vm: &mut Vm) -> PyResult<i64> {
    let v = py::config(vm, "glsl_version")?;
    let d = match &v {
        Value::Int(i) => glsl_dialect(*i),
        _ => None,
    };
    match d {
        Some(d) => Ok(d),
        None => {
            let r = ops::repr(vm, &v)?;
            Err(py::error(
                vm,
                format!(
                    "config.glsl_version is {r}, which is not a known GLSL dialect. Use 100 for GLSL ES 1.00, or 300 for GLSL ES 3.00."
                ),
            ))
        }
    }
}

fn f_config_dialect(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(config_dialect(vm)?))
}

/// `parse_glsl_version(s)`: "4.60 NVIDIA" -> 460, "OpenGL ES GLSL ES 3.2" -> 320.
fn f_parse_glsl_version(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&args[0]) {
        return Ok(Value::None);
    }
    let s = py::string(vm, &args[0])?;
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_ascii_digit() {
            let mut j = i;
            while j < c.len() && c[j].is_ascii_digit() {
                j += 1;
            }
            if c.get(j) == Some(&'.') && c.get(j + 1).is_some_and(|x| x.is_ascii_digit()) {
                let mut k = j + 1;
                while k < c.len() && c[k].is_ascii_digit() {
                    k += 1;
                }
                let major: i64 = c[i..j].iter().collect::<String>().parse().unwrap_or(0);
                let mut minor: String = c[j + 1..k].iter().collect();
                while minor.len() < 2 {
                    minor.push('0');
                }
                let minor: i64 = minor.parse().unwrap_or(0);
                return Ok(Value::Int(major * 100 + minor));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    Ok(Value::None)
}

fn dialect_version(gles: bool, d: i64) -> i64 {
    let (g, desk) = dialect_versions(d);
    if gles { g } else { desk }
}

fn f_dialect_version(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let gles = py::truthy(vm, &args[0])?;
    let d = py::int(vm, &args[1])?;
    if !DIALECTS.contains(&d) {
        return Err(vm.exc_msg("KeyError", d.to_string()));
    }
    Ok(Value::Int(dialect_version(gles, d)))
}

fn emit_version(gles: bool, glsl: Option<i64>) -> i64 {
    let glsl = glsl.unwrap_or(0);
    let mut rv = dialect_version(gles, DIALECTS[0]);
    for d in DIALECTS {
        let v = dialect_version(gles, d);
        if glsl >= v {
            rv = v;
        }
    }
    rv
}

fn f_emit_version(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let gles = py::truthy(vm, &args[0])?;
    let g = if py::is_none(&args[1]) { None } else { Some(py::int(vm, &args[1])?) };
    Ok(Value::Int(emit_version(gles, g)))
}

fn apply_rules(vm: &mut Vm, rules: &str, text: Value) -> PyResult<Value> {
    let rules = mget(vm, rules)?;
    let mut text = text;
    for r in py::items(vm, &rules)? {
        let p = py::items(vm, &r)?;
        text = py::method(vm, &p[0], "sub", vec![p[1].clone(), text])?;
    }
    Ok(text)
}

/// `translate(text, source_version, target_version)`.
fn f_translate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let src = py::int(vm, &args[1])?;
    let target = py::int(vm, &args[2])?;
    let Some(td) = glsl_dialect(target) else {
        return Err(vm.exc_msg("KeyError", target.to_string()));
    };
    if src == td {
        return Ok(args[0].clone());
    }
    let rules = if src > td { "MODERN_TO_LEGACY" } else { "LEGACY_TO_MODERN" };
    apply_rules(vm, rules, args[0].clone())
}

/// `newer_markers(dialect)`: (dialect, pattern, name) of later dialects.
fn newer_markers(vm: &mut Vm, dialect: i64) -> PyResult<Vec<(i64, Value, Value)>> {
    let mut rv = Vec::new();
    for d in DIALECTS {
        if d <= dialect {
            continue;
        }
        for table in ["TRANSLATED_MARKERS", "UNTRANSLATED_MARKERS"] {
            let t = mget(vm, table)?;
            let l = vm.call_method(&t, "get", vec![Value::Int(d), vm.types.new_list(Vec::new())])?;
            for e in py::items(vm, &l)? {
                let p = py::items(vm, &e)?;
                rv.push((d, p[0].clone(), p[1].clone()));
            }
        }
    }
    Ok(rv)
}

fn f_newer_markers(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::int(vm, &args[0])?;
    let items = newer_markers(vm, d)?
        .into_iter()
        .map(|(d, p, n)| vm.types.new_tuple(vec![Value::Int(d), p, n]))
        .collect();
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

/// `merge_variables(variables)`.
fn merge_variables(vm: &mut Vm, vars: &Value) -> PyResult<Vec<Value>> {
    let merged = vm.types.new_dict();
    let mut order = Vec::new();
    for v in ops::collect(vm, vars)? {
        let k = var_key(vm, &v)?;
        let old = vm.call_method(&merged, "get", vec![k.clone(), Value::None])?;
        let nv = if py::is_none(&old) {
            order.push(k.clone());
            v
        } else {
            py::method(vm, &old, "merge", vec![v])?
        };
        ops::setitem(vm, &merged, k, nv)?;
    }
    let mut vals = Vec::new();
    for k in order {
        vals.push(ops::getitem(vm, &merged, k)?);
    }
    let mut keyed = Vec::new();
    for v in vals {
        let n = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
        let s = opt_str_attr(vm, &v, "storage")?.unwrap_or_default();
        let t = opt_str_attr(vm, &v, "type")?.unwrap_or_default();
        keyed.push(((n, s, t), v));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(keyed.into_iter().map(|x| x.1).collect())
}

fn f_merge_variables(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = merge_variables(vm, &args[0])?;
    Ok(vm.types.new_list(l))
}

/// `link_variables(vertex_variables, fragment_variables)`.
fn f_link_variables(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let vertex = merge_variables(vm, &args[0])?;
    let fragment = merge_variables(vm, &args[1])?;
    let mut linked: Vec<(String, Option<Value>)> = Vec::new();
    for v in vertex.iter().chain(fragment.iter()) {
        if opt_str_attr(vm, v, "storage")?.as_deref() != Some("varying") {
            continue;
        }
        let n = opt_str_attr(vm, v, "name")?.unwrap_or_default();
        match linked.iter().position(|(k, _)| *k == n) {
            None => linked.push((n, Some(v.clone()))),
            Some(i) => {
                let Some(old) = linked[i].1.clone() else { continue };
                let ot = (opt_str_attr(vm, &old, "type")?, py::get_or_none(vm, &old, "array")?);
                let nt = (opt_str_attr(vm, v, "type")?, py::get_or_none(vm, v, "array")?);
                let same_array = ops::eq(vm, &ot.1, &nt.1)?;
                linked[i].1 = if ot.0 == nt.0 && same_array {
                    Some(py::method(vm, &old, "merge", vec![v.clone()])?)
                } else {
                    None
                };
            }
        }
    }
    let mut relink = |vm: &mut Vm, vars: Vec<Value>| -> PyResult<Value> {
        let mut out = Vec::new();
        for v in vars {
            if opt_str_attr(vm, &v, "storage")?.as_deref() == Some("varying") {
                let n = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
                let l = linked.iter().find(|(k, _)| *k == n).and_then(|(_, x)| x.clone());
                out.push(l.unwrap_or(v));
            } else {
                out.push(v);
            }
        }
        Ok(vm.types.new_list(out))
    };
    let a = relink(vm, vertex)?;
    let b = relink(vm, fragment)?;
    Ok(vm.types.new_tuple(vec![a, b]))
}

// ShaderPart ----------------------------------------------------------------------------------

/// `register_shader(name, **kwargs)` -> `ShaderPart(name, **kwargs)`.
fn register_shader(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = mget(vm, "ShaderPart")?;
    let kwargs = kw.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    vm.call(&c, args.to_vec(), kwargs)
}

fn sp_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v74 = at_least(vm, 7, 4);
    let comments = since_pair(vm, 6, 1);
    let v83 = since_pair(vm, 8, 3);
    let v84 = at_least(vm, 8, 4);
    let v86 = at_least(vm, 8, 6);
    let name_v = py::arg_or(args, kw, 1, "name", Value::None);
    let name = py::string(vm, &name_v)?;
    if name.is_empty() || !name.chars().all(|c| is_word(c) || c == '.') {
        let r = ops::repr(vm, &name_v)?;
        return Err(py::error(
            vm,
            format!(
                "The shader name {r} contains an invalid character. Shader names are limited to ASCII alphanumeric characters, _, and ."
            ),
        ));
    }
    let mut named = vec!["name", "variables", "vertex_functions", "fragment_functions"];
    if v83 {
        named.push("private_uniforms");
    }
    if v86 {
        named.push("glsl");
    }
    let glsl = if v86 { py::arg_or(args, kw, 6, "glsl", Value::None) } else { Value::None };
    let declared = if py::is_none(&glsl) {
        None
    } else {
        match &glsl {
            Value::Int(i) if glsl_dialect(*i).is_some() => glsl_dialect(*i),
            _ => {
                let r = ops::repr(vm, &glsl)?;
                return Err(py::error(
                    vm,
                    format!(
                        "In shader {name}: glsl={r} is not a known GLSL dialect. Use glsl=100 (or the equivalent glsl=120) for GLSL ES 1.00, or glsl=300 (or the equivalent glsl=330) for GLSL ES 3.00."
                    ),
                ));
            }
        }
    };
    py::set(vm, this, "name", name_v.clone())?;
    let parts = mget(vm, "shader_part")?;
    ops::setitem(vm, &parts, name_v, this.clone())?;
    if v86 {
        py::set(vm, this, "declared_glsl", declared.map(Value::Int).unwrap_or(Value::None))?;
        py::set(vm, this, "checked_glsl", Value::Bool(false))?;
        for n in ["inferred_glsl", "found_dialect", "found_feature", "legacy_storage"] {
            py::set(vm, this, n, Value::None)?;
        }
    }
    let s_or = |vm: &mut Vm, i: usize, n: &str| -> PyResult<String> {
        match py::arg(args, kw, i, n) {
            Some(v) if !py::is_none(v) => py::string(vm, v),
            _ => Ok(String::new()),
        }
    };
    let variables = s_or(vm, 2, "variables")?;
    let mut vf_s = s_or(vm, 3, "vertex_functions")?;
    let mut ff_s = s_or(vm, 4, "fragment_functions")?;
    let (mut vused, mut fused) = (HashSet::new(), HashSet::new());
    if v86 {
        vf_s = substitute(vm, &name, &vf_s);
        ff_s = substitute(vm, &name, &ff_s);
        vused.extend(words(&vf_s));
        fused.extend(words(&ff_s));
        py::set(vm, this, "vertex_functions", py::s(vm, &vf_s))?;
        py::set(vm, this, "fragment_functions", py::s(vm, &ff_s))?;
    } else {
        let vf = py::arg_or(args, kw, 3, "vertex_functions", py::s(vm, ""));
        let ff = py::arg_or(args, kw, 4, "fragment_functions", py::s(vm, ""));
        py::set(vm, this, "vertex_functions", vf)?;
        py::set(vm, this, "fragment_functions", ff)?;
    }
    let private = if v83 {
        let p = py::arg_or(args, kw, 5, "private_uniforms", Value::Bool(false));
        py::truthy(vm, &p)?
    } else {
        false
    };
    let (mut vparts, mut fparts) = (Vec::new(), Vec::new());
    for (k, v) in kw.iter().filter(|(k, _)| !named.contains(&&**k)) {
        let mut text = py::string(vm, v)?;
        if v83 {
            text = substitute(vm, &name, &text);
        }
        let (shader, priority) = k.split_once('_').unwrap_or((k, ""));
        let (parts, used) = match (shader, priority.parse::<i64>()) {
            ("vertex", Ok(p)) => ((&mut vparts, p), &mut vused),
            ("fragment", Ok(p)) => ((&mut fparts, p), &mut fused),
            _ => {
                return Err(py::error(
                    vm,
                    "Keyword arguments to ShaderPart must be of the form {vertex,fragment}_{priority}.",
                ));
            }
        };
        used.extend(words(&text));
        let t = if v84 {
            vm.types.new_tuple(vec![Value::Int(parts.1), py::s(vm, &name), py::s(vm, &text)])
        } else {
            vm.types.new_tuple(vec![Value::Int(parts.1), py::s(vm, &text)])
        };
        parts.0.push(t);
    }
    py::set(vm, this, "vertex_parts", vm.types.new_list(vparts))?;
    py::set(vm, this, "fragment_parts", vm.types.new_list(fparts))?;
    let variables = if v83 { substitute(vm, &name, &variables) } else { variables };
    let types = py::dict(vm);
    let mut uniforms = Vec::new();
    let vvars = vm.types.new_set(Default::default());
    let fvars = vm.types.new_set(Default::default());
    let mut legacy_storage: Option<String> = None;
    let vc = if v84 { Some(variable_class(vm)?) } else { None };
    for raw in variables.split('\n') {
        let l = if comments { raw.split("//").next().unwrap_or("") } else { raw };
        if let Some(vc) = &vc {
            let l = l.trim();
            if l.is_empty() {
                continue;
            }
            if v86 && legacy_storage.is_none() {
                legacy_storage = words(l)
                    .into_iter()
                    .find(|w| w == "attribute" || w == "varying");
            }
            let v = vm.call(vc, vec![py::s(vm, &name), py::s(vm, l)], Vec::new())?;
            let storage = opt_str_attr(vm, &v, "storage")?;
            let vname = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
            let vtype = opt_str_attr(vm, &v, "type")?.unwrap_or_default();
            if !matches!(storage.as_deref(), Some("uniform" | "attribute" | "varying")) {
                let r = ops::repr(vm, &py::s(vm, l))?;
                return Err(py::error(
                    vm,
                    if v86 {
                        format!(
                            "In shader {name}: Unknown shader variable line {r}. Only the form '{{uniform,in,out}} {{type}} {{name}};' is allowed, or the equivalent '{{uniform,attribute,varying}} {{type}} {{name}};'."
                        )
                    } else {
                        format!(
                            "In shader {name}: Unknown shader variable line {r}. Only the form '{{uniform,attribute,vertex}} {{type}} {{name}} is allowed."
                        )
                    },
                ));
            }
            let array = py::get_or_none(vm, &v, "array")?;
            let t = if py::truthy(vm, &array)? { format!("{vtype}[]") } else { vtype.clone() };
            py::dict_set(vm, &types, &vname, py::s(vm, &t))?;
            if vused.contains(&vname) {
                ops::set_add(vm, &vvars, v.clone())?;
            }
            if fused.contains(&vname) {
                ops::set_add(vm, &fvars, v.clone())?;
            }
            if storage.as_deref() == Some("uniform") {
                if !private {
                    let au = py::import_attr(vm, "renpy.display.transform", "add_uniform")?;
                    vm.call(&au, vec![py::s(vm, &vname), py::s(vm, &vtype)], Vec::new())?;
                }
                uniforms.push(py::s(vm, &vname));
            }
            continue;
        }
        let l = l.trim_matches(|c| c == ' ' || c == ';');
        let a: Vec<&str> = l.split_whitespace().collect();
        if a.is_empty() {
            continue;
        }
        if a.len() != 3 {
            let r = ops::repr(vm, &py::s(vm, l))?;
            if v74 {
                return Err(py::error(
                    vm,
                    format!(
                        "{name}: Unknown shader variable line {r}. Only the form '{{uniform,attribute,vertex}} {{type}} {{name}} is allowed."
                    ),
                ));
            }
            // 7.3 prints the message and then fails unpacking the line.
            let print = py::import_attr(vm, "builtins", "print")?;
            let msg = format!(
                "Unknown shader variable line {r}. Only the form '{{uniform,attribute,vertex}} {{type}} {{name}} is allowed."
            );
            vm.call(&print, vec![py::s(vm, &msg)], Vec::new())?;
            if a.len() < 3 {
                return Err(vm.exc_msg("IndexError", "tuple index out of range"));
            }
        }
        if v83 {
            py::dict_set(vm, &types, a[2], py::s(vm, a[1]))?;
        }
        let t = vm.types.new_tuple(a.iter().map(|x| vm.types.new_str(x)).collect());
        if vused.contains(a[2]) {
            ops::set_add(vm, &vvars, t.clone())?;
        }
        if fused.contains(a[2]) {
            ops::set_add(vm, &fvars, t)?;
        }
        if a[0] == "uniform" && v74 {
            if !private {
                let au = py::import_attr(vm, "renpy.display.transform", "add_uniform")?;
                vm.call(&au, vec![py::s(vm, a[2])], Vec::new())?;
            }
            uniforms.push(py::s(vm, a[2]));
        }
    }
    py::set(vm, this, "vertex_variables", vvars)?;
    py::set(vm, this, "fragment_variables", fvars)?;
    if v83 {
        py::set(vm, this, "variable_types", types)?;
        py::set(vm, this, "uniforms", vm.types.new_list(uniforms))?;
    }
    if v74 {
        py::set(vm, this, "raw_variables", py::s(vm, &variables))?;
    }
    if v86 {
        py::set(vm, this, "legacy_storage", opt_s(vm, &legacy_storage))?;
        // cache_digest: sha256 of the JSON of the part's defining data.
        let mut data = vec![
            py::s(vm, &name),
            declared.map(Value::Int).unwrap_or(Value::None),
            py::s(vm, &variables),
            py::s(vm, &vf_s),
            py::s(vm, &ff_s),
        ];
        data.push(py::get(vm, this, "vertex_parts")?);
        data.push(py::get(vm, this, "fragment_parts")?);
        let data = vm.types.new_tuple(data);
        let dumps = py::import_attr(vm, "json", "dumps")?;
        let sep = vm.types.new_tuple(vec![py::s(vm, ","), py::s(vm, ":")]);
        let j = vm.call(
            &dumps,
            vec![data],
            vec![("ensure_ascii".into(), Value::Bool(false)), ("separators".into(), sep)],
        )?;
        let b = py::method(vm, &j, "encode", vec![py::s(vm, "utf-8")])?;
        let sha = py::import_attr(vm, "hashlib", "sha256")?;
        let h = vm.call(&sha, vec![b], Vec::new())?;
        let d = py::method(vm, &h, "digest", Vec::new())?;
        py::set(vm, this, "cache_digest", d)?;
    }
    Ok(Value::None)
}

/// `ShaderPart.part_source()`: every chunk of GLSL, comments stripped.
fn part_source(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let mut srcs = Vec::new();
    for n in ["vertex_functions", "fragment_functions"] {
        let v = py::get(vm, this, n)?;
        srcs.push(py::string(vm, &v)?);
    }
    for n in ["vertex_parts", "fragment_parts"] {
        for p in py::get_items(vm, this, n)? {
            let t = ops::getitem(vm, &p, Value::Int(2))?;
            srcs.push(py::string(vm, &t)?);
        }
    }
    let joined: Vec<String> = srcs.into_iter().filter(|s| !s.is_empty()).collect();
    let text = py::s(vm, &joined.join("\n"));
    let c = mget(vm, "GLSL_COMMENTS")?;
    py::method(vm, &c, "sub", vec![py::s(vm, " "), text])
}

fn sp_part_source(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    part_source(vm, &args[0])
}

fn search(vm: &mut Vm, pattern: &Value, text: &Value) -> PyResult<bool> {
    let m = py::method(vm, pattern, "search", vec![text.clone()])?;
    Ok(!py::is_none(&m))
}

/// `find_legacy_syntax()` -> (legacy, modern) or None.
fn find_legacy_syntax(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let ls = opt_str_attr(vm, this, "legacy_storage")?;
    if let Some(l) = ls {
        let modern = if l == "attribute" { "in" } else { "out" };
        return Ok(vm.types.new_tuple(vec![py::s(vm, &l), py::s(vm, modern)]));
    }
    let text = part_source(vm, this)?;
    let markers = mget(vm, "LEGACY_MARKERS")?;
    for m in py::items(vm, &markers)? {
        let p = py::items(vm, &m)?;
        if search(vm, &p[0], &text)? {
            return Ok(vm.types.new_tuple(vec![p[1].clone(), p[2].clone()]));
        }
    }
    Ok(Value::None)
}

fn sp_find_legacy_syntax(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    find_legacy_syntax(vm, &args[0])
}

fn sp_check_modern_syntax(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let dialect = py::int(vm, &args[1])?;
    if !py::is_none(&py::get_or_none(vm, this, "legacy_storage")?) {
        return Ok(Value::None);
    }
    let text = part_source(vm, this)?;
    let markers = mget(vm, "LEGACY_MARKERS")?;
    for m in py::items(vm, &markers)? {
        let p = py::items(vm, &m)?;
        if search(vm, &p[0], &text)? {
            return Ok(Value::None);
        }
    }
    let name = part_name(vm, this)?;
    for (newer, pat, n) in newer_markers(vm, dialect)? {
        if search(vm, &pat, &text)? {
            let n = py::string(vm, &n)?;
            return Err(py::error(
                vm,
                format!(
                    "Shader part {name} uses {n}, which needs {}, but doesn't declare a dialect. Pass glsl={newer} to renpy.register_shader or rewrite it for {}.",
                    dialect_name(newer),
                    dialect_name(dialect)
                ),
            ));
        }
    }
    Ok(Value::None)
}

fn sp_glsl(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let declared = py::get_or_none(vm, this, "declared_glsl")?;
    if !py::is_none(&declared) {
        return Ok(declared);
    }
    let rv = config_dialect(vm)?;
    let checked = py::get_or_none(vm, this, "checked_glsl")?;
    if !py::truthy(vm, &checked)? {
        if rv > DIALECTS[0] {
            let found = find_legacy_syntax(vm, this)?;
            if !py::is_none(&found) {
                let p = py::items(vm, &found)?;
                let (legacy, modern) = (py::string(vm, &p[0])?, py::string(vm, &p[1])?);
                let name = part_name(vm, this)?;
                let oldest = DIALECTS[0];
                super::log::write(
                    vm,
                    &format!(
                        "Shader part {name} uses {legacy}, which only exists in {o}, and doesn't declare a dialect. Treating it as {o}. Pass glsl={oldest} to renpy.register_shader, or rewrite it for {n} ({legacy} becomes {modern}) and pass glsl={rv}.",
                        o = dialect_name(oldest),
                        n = dialect_name(rv)
                    ),
                );
                py::set(vm, this, "inferred_glsl", Value::Int(oldest))?;
            }
        } else {
            sp_check_modern_syntax(vm, &[this.clone(), Value::Int(rv)], &[])?;
        }
        py::set(vm, this, "checked_glsl", Value::Bool(true))?;
    }
    let inferred = py::get_or_none(vm, this, "inferred_glsl")?;
    if !py::is_none(&inferred) {
        return Ok(inferred);
    }
    Ok(Value::Int(rv))
}

fn all_variables(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let mut v = py::get_items(vm, this, "vertex_variables")?;
    v.extend(py::get_items(vm, this, "fragment_variables")?);
    Ok(v)
}

fn untranslated_type(vm: &mut Vm, this: &Value, dialect: i64) -> PyResult<Option<String>> {
    if dialect != 300 {
        return Ok(None);
    }
    for v in all_variables(vm, this)? {
        let t = opt_str_attr(vm, &v, "type")?.unwrap_or_default();
        if opt_str_attr(vm, &v, "storage")?.as_deref() != Some("uniform") && FLAT_TYPES.contains(&t.as_str()) {
            let n = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
            return Ok(Some(format!("{t} {n}")));
        }
    }
    Ok(None)
}

fn untranslated_interpolation(vm: &mut Vm, this: &Value, dialect: i64) -> PyResult<Option<String>> {
    if dialect < 300 {
        return Ok(None);
    }
    for v in all_variables(vm, this)? {
        let i = py::get_opt(vm, &v, "interpolation")?.unwrap_or(Value::None);
        let i = if py::is_none(&i) { None } else { Some(py::string(vm, &i)?) };
        if let Some(i) = i.filter(|i| i == "flat" || i == "noperspective") {
            let n = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
            return Ok(Some(format!("{i} interpolation on {n}")));
        }
    }
    Ok(None)
}

fn opt_value(vm: &Vm, s: Option<String>) -> Value {
    opt_s(vm, &s)
}

fn sp_untranslated_type(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::int(vm, &args[1])?;
    let r = untranslated_type(vm, &args[0], d)?;
    Ok(opt_value(vm, r))
}

fn sp_untranslated_interpolation(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::int(vm, &args[1])?;
    let r = untranslated_interpolation(vm, &args[0], d)?;
    Ok(opt_value(vm, r))
}

fn sp_find_minimum_dialect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let text = part_source(vm, this)?;
    let mut found = DIALECTS[0];
    let mut feature: Option<String> = None;
    for d in DIALECTS {
        if d <= found {
            continue;
        }
        let t = mget(vm, "UNTRANSLATED_MARKERS")?;
        let l = vm.call_method(&t, "get", vec![Value::Int(d), vm.types.new_list(Vec::new())])?;
        let mut name = None;
        for e in py::items(vm, &l)? {
            let p = py::items(vm, &e)?;
            if search(vm, &p[0], &text)? {
                name = Some(py::string(vm, &p[1])?);
                break;
            }
        }
        if name.is_none() {
            name = untranslated_interpolation(vm, this, d)?;
        }
        if name.is_none() {
            name = untranslated_type(vm, this, d)?;
        }
        if name.is_some() {
            found = d;
            feature = name;
        }
    }
    py::set(vm, this, "found_dialect", Value::Int(found))?;
    py::set(vm, this, "found_feature", opt_value(vm, feature))?;
    Ok(Value::None)
}

fn sp_minimum_dialect(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&py::get_or_none(vm, &args[0], "found_dialect")?) {
        sp_find_minimum_dialect(vm, args, &[])?;
    }
    py::get(vm, &args[0], "found_dialect")
}

fn sp_minimum_feature(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&py::get_or_none(vm, &args[0], "found_dialect")?) {
        sp_find_minimum_dialect(vm, args, &[])?;
    }
    py::get(vm, &args[0], "found_feature")
}

// source --------------------------------------------------------------------------------------

/// `source(variables, parts, functions, fragment, gles[, version])`.
fn f_source(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (variables, parts, functions) = (&args[0], &args[1], &args[2]);
    let fragment = py::truthy(vm, &args[3])?;
    let gles = py::truthy(vm, &args[4])?;
    let mut rv = String::new();
    if at_least(vm, 8, 6) {
        let version = py::int(vm, &py::arg_or(args, kw, 5, "version", Value::Int(100)))?;
        if gles && version >= 300 {
            rv.push_str(&format!("#version {version} es\n"));
        } else {
            rv.push_str(&format!("#version {version}\n"));
        }
        if fragment && gles {
            if version >= 300 {
                rv.push_str("precision highp float;\nprecision highp int;\n");
            } else {
                rv.push_str(HIGHP_IFDEF);
            }
        }
        if fragment && version >= 300 {
            rv.push_str("layout(location = 0) out vec4 fragment_color;\n");
        }
        for v in merge_variables(vm, variables)? {
            let d = py::method(
                vm,
                &v,
                "declaration",
                vec![Value::Bool(fragment), Value::Bool(gles), Value::Int(version)],
            )?;
            rv.push_str(&py::string(vm, &d)?);
            rv.push_str(";\n");
        }
        let tr = mget(vm, "translate")?;
        let mut body = String::new();
        for f in ops::collect(vm, functions)? {
            let p = py::items(vm, &f)?;
            let t = vm.call(&tr, vec![p[0].clone(), p[1].clone(), Value::Int(version)], Vec::new())?;
            body.push_str(&py::string(vm, &t)?);
        }
        body.push_str("\nvoid main() {\n");
        py::method(vm, parts, "sort", Vec::new())?;
        for p in ops::collect(vm, parts)? {
            let p = py::items(vm, &p)?;
            let t = vm.call(&tr, vec![p[2].clone(), p[3].clone(), Value::Int(version)], Vec::new())?;
            body.push_str(&py::string(vm, &t)?);
        }
        body.push_str("}\n");
        let mut body = py::s(vm, &body);
        if fragment && version < 300 {
            let (ext, suffix) = if gles {
                ("GL_EXT_shader_texture_lod", "EXT")
            } else {
                ("GL_ARB_shader_texture_lod", "")
            };
            let lf = mget(vm, "LOD_FUNCTIONS")?;
            let r = py::method(vm, &lf, "subn", vec![py::s(vm, &format!("\\g<1>{suffix}(")), body])?;
            let r = py::items(vm, &r)?;
            body = r[0].clone();
            if py::int(vm, &r[1])? > 0 {
                let nl = rv.find('\n').map(|i| i + 1).unwrap_or(rv.len());
                rv.insert_str(nl, &format!("#extension {ext} : require\n"));
            }
        }
        rv.push_str(&py::string(vm, &body)?);
        return Ok(py::s(vm, &rv));
    }
    let v84 = at_least(vm, 8, 4);
    let v74 = at_least(vm, 7, 4);
    if gles {
        rv.push_str(if v74 { "#version 100\n" } else { "\n#version 100 es\n" });
        if fragment {
            rv.push_str(if v84 {
                HIGHP_IFDEF
            } else if v74 {
                "precision mediump float;\n"
            } else {
                "\nprecision mediump float;\n"
            });
        }
    } else {
        rv.push_str(if v74 { "#version 120\n" } else { "\n#version 120\n" });
    }
    let vars_first = since_pair(vm, 8, 3);
    let mut funcs = String::new();
    for f in ops::collect(vm, functions)? {
        funcs.push_str(&py::string(vm, &f)?);
    }
    if !vars_first {
        rv.push_str(&funcs);
    }
    if v84 {
        let mut vs = Vec::new();
        for v in ops::collect(vm, variables)? {
            let n = opt_str_attr(vm, &v, "name")?.unwrap_or_default();
            let l = opt_str_attr(vm, &v, "line")?.unwrap_or_default();
            vs.push((n, l));
        }
        vs.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, l) in vs {
            rv.push_str(&l);
            rv.push_str(";\n");
        }
    } else {
        let mut vs = Vec::new();
        for v in ops::collect(vm, variables)? {
            let p = py::items(vm, &v)?;
            let mut t = Vec::new();
            for x in p {
                t.push(py::string(vm, &x)?);
            }
            vs.push(t);
        }
        vs.sort();
        for t in vs {
            rv.push_str(&format!("{} {} {};\n", t[0], t[1], t[2]));
        }
    }
    if vars_first {
        rv.push_str(&funcs);
    }
    rv.push_str("\nvoid main() {\n");
    py::method(vm, parts, "sort", Vec::new())?;
    for p in ops::collect(vm, parts)? {
        let t = ops::getitem(vm, &p, Value::Int(-1))?;
        rv.push_str(&py::string(vm, &t)?);
    }
    rv.push_str("}\n");
    Ok(py::s(vm, &rv))
}

const HIGHP_IFDEF: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\n    precision highp float;\n    precision highp int;\n#else\n    precision mediump float;\n    precision mediump int;\n#endif\n";

// Program cache (8.6) --------------------------------------------------------------------------

fn sha256(vm: &mut Vm, parts: &[Value]) -> PyResult<Value> {
    let sha = py::import_attr(vm, "hashlib", "sha256")?;
    let h = vm.call(&sha, Vec::new(), Vec::new())?;
    for p in parts {
        py::method(vm, &h, "update", vec![p.clone()])?;
    }
    py::method(vm, &h, "digest", Vec::new())
}

fn pack(vm: &mut Vm, fmt: &str, vals: Vec<Value>) -> PyResult<Value> {
    let p = py::import_attr(vm, "struct", "pack")?;
    let mut a = vec![py::s(vm, fmt)];
    a.extend(vals);
    vm.call(&p, a, Vec::new())
}

fn f_shader_program_key(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let version = args[1].clone();
    let v = pack(vm, "<I", vec![version])?;
    let mut names = Vec::new();
    for n in ops::collect(vm, &args[0])? {
        names.push(py::string(vm, &n)?);
    }
    let b = vm.types.new_bytes(names.join("\0").into_bytes());
    sha256(vm, &[v, b])
}

fn f_shader_recipe_digest(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let gles = py::truthy(vm, &args[1])?;
    let d = config_dialect(vm)?;
    let magic = mget(vm, "PROGRAM_CACHE_RECIPE")?;
    let head = pack(vm, "<BII", vec![Value::Int(gles as i64), args[2].clone(), Value::Int(d)])?;
    let mut parts = vec![magic, head];
    let sp = mget(vm, "shader_part")?;
    for n in ops::collect(vm, &args[0])? {
        let part = vm.call_method(&sp, "get", vec![n.clone(), Value::None])?;
        if py::is_none(&part) {
            let r = ops::repr(vm, &n)?;
            return Err(py::error(vm, format!("{r} is not a known shader part.")));
        }
        let s = py::string(vm, &n)?;
        let nb = s.into_bytes();
        parts.push(pack(vm, "<I", vec![Value::Int(nb.len() as i64)])?);
        parts.push(vm.types.new_bytes(nb));
        parts.push(py::get(vm, &part, "cache_digest")?);
    }
    sha256(vm, &parts)
}

fn f_program_cache_entry_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let gs = py::import_attr(vm, "sys", "getsizeof")?;
    let mut total = 0;
    for v in [&args[0], &args[1]] {
        let s = vm.call(&gs, vec![v.clone()], Vec::new())?;
        total += py::int(vm, &s)?;
    }
    for v in ops::collect(vm, &args[1])? {
        let s = vm.call(&gs, vec![v], Vec::new())?;
        total += py::int(vm, &s)?;
    }
    Ok(Value::Int(total))
}

fn f_decode_program_variable_specs(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::method(vm, &args[0], "decode", vec![py::s(vm, "utf-8")])?;
    let loads = py::import_attr(vm, "json", "loads")?;
    let specs = vm.call(&loads, vec![s], Vec::new())?;
    let Some(list) = (match &specs {
        v if ops::type_name(vm, v) == "list" => Some(py::items(vm, v)?),
        _ => None,
    }) else {
        return Err(vm.value_error("Shader variable metadata is not a list."));
    };
    let mut rv = Vec::new();
    for spec in list {
        if ops::type_name(vm, &spec) != "list" || ops::len(vm, &spec)? != 5 {
            return Err(vm.value_error("Shader variable metadata has an invalid entry."));
        }
        let p = py::items(vm, &spec)?;
        if !matches!(p[0].as_str(), Some("uniform" | "attribute")) {
            return Err(vm.value_error("Shader variable metadata has an invalid storage class."));
        }
        if p[1].as_str().is_none() || p[2].as_str().is_none() {
            return Err(vm.value_error("Shader variable metadata has an invalid type or name."));
        }
        if !(py::is_none(&p[3]) || matches!(p[3], Value::Int(i) if i >= 1)) {
            return Err(vm.value_error("Shader variable metadata has an invalid array size."));
        }
        if !matches!(p[4], Value::Bool(_)) {
            return Err(vm.value_error("Shader variable metadata has an invalid stage."));
        }
        rv.push(vm.types.new_tuple(p));
    }
    Ok(vm.types.new_tuple(rv))
}

fn shader_cache(vm: &mut Vm) -> PyResult<Value> {
    let d = py::import(vm, "renpy.display")?;
    let draw = py::get_opt(vm, &d, "draw")?.unwrap_or(Value::None);
    if py::is_none(&draw) {
        return Ok(Value::None);
    }
    Ok(py::get_opt(vm, &draw, "shader_cache")?.unwrap_or(Value::None))
}

fn f_get_shader_cache(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    shader_cache(vm)
}

fn cache_arg(vm: &mut Vm, args: &[Value], kw: Kw, i: usize) -> PyResult<Value> {
    let c = py::arg_or(args, kw, i, "cache", Value::None);
    if py::is_none(&c) { shader_cache(vm) } else { Ok(c) }
}

fn f_predict_shader(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "predict_shaders").unwrap_or(false) {
        return Ok(Value::None);
    }
    let names = if args[0].as_str().is_some() {
        vm.types.new_tuple(vec![args[0].clone()])
    } else {
        let l = ops::collect(vm, &args[0])?;
        vm.types.new_tuple(l)
    };
    let cache = cache_arg(vm, args, kw, 1)?;
    if py::is_none(&cache) {
        return Ok(Value::None);
    }
    py::method(vm, &cache, "predict", vec![names])?;
    Ok(Value::None)
}

fn predicted_guard(vm: &mut Vm, cache: &Value) -> PyResult<bool> {
    if !py::config_bool(vm, "predict_shaders").unwrap_or(false) {
        let p = py::get(vm, cache, "predicted")?;
        py::method(vm, &p, "clear", Vec::new())?;
        return Ok(false);
    }
    Ok(true)
}

fn f_has_predicted_shaders(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cache = cache_arg(vm, args, kw, 0)?;
    if py::is_none(&cache) || !predicted_guard(vm, &cache)? {
        return Ok(Value::Bool(false));
    }
    let p = py::get(vm, &cache, "predicted")?;
    Ok(Value::Bool(py::truthy(vm, &p)?))
}

fn f_preload_predicted_shader(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cache = cache_arg(vm, args, kw, 0)?;
    if py::is_none(&cache) || !predicted_guard(vm, &cache)? {
        return Ok(Value::Bool(false));
    }
    py::method(vm, &cache, "preload_predicted", Vec::new())
}

// ShaderCache ---------------------------------------------------------------------------------

fn sc_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::arg_or(args, kw, 1, "filename", Value::None);
    let gles = py::arg_or(args, kw, 2, "gles", Value::Bool(false));
    py::set(vm, this, "filename", filename)?;
    py::set(vm, this, "gles", gles.clone())?;
    py::set(vm, this, "cache", vm.types.new_dict())?;
    py::set(vm, this, "missing", vm.types.new_set(Default::default()))?;
    if at_least(vm, 7, 4) {
        py::set(vm, this, "dirty", Value::Bool(false))?;
    }
    if !at_least(vm, 8, 6) {
        return Ok(Value::None);
    }
    let gles_b = py::truthy(vm, &gles)?;
    let glsl = py::arg_or(args, kw, 3, "glsl_version", Value::None);
    py::set(vm, this, "glsl_version", glsl.clone())?;
    let forced = std::env::var("RENPY_GLSL_VERSION").ok();
    if let Some(f) = forced {
        let Some(d) = f.trim().parse::<i64>().ok().and_then(glsl_dialect) else {
            return Err(py::error(
                vm,
                format!("RENPY_GLSL_VERSION is '{f}', which is not one of 100, 120, 300, 330."),
            ));
        };
        py::set(vm, this, "version", Value::Int(dialect_version(gles_b, d)))?;
        py::set(vm, this, "pinned", Value::Bool(true))?;
    } else {
        let g = if py::is_none(&glsl) { None } else { Some(py::int(vm, &glsl)?) };
        py::set(vm, this, "version", Value::Int(emit_version(gles_b, g)))?;
        py::set(vm, this, "pinned", Value::Bool(false))?;
    }
    py::set(vm, this, "downgraded", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "program_cache_filename", Value::None)?;
    py::set(vm, this, "program_cache", vm.types.new_dict())?;
    py::set(vm, this, "program_cache_loaded", Value::Bool(false))?;
    py::set(vm, this, "program_cache_dirty", Value::Bool(false))?;
    // No GL driver: program binaries are never retrievable.
    py::set(vm, this, "program_binary_supported", Value::Bool(false))?;
    py::set(vm, this, "program_cache_size", Value::Int(0))?;
    let od = py::import_attr(vm, "collections", "OrderedDict")?;
    let p = vm.call(&od, Vec::new(), Vec::new())?;
    py::set(vm, this, "predicted", p)?;
    Ok(Value::None)
}

fn filter_partnames(vm: &mut Vm, names: Value) -> PyResult<Value> {
    let f = py::config(vm, "shader_part_filter").unwrap_or(Value::None);
    if py::is_none(&f) {
        return Ok(names);
    }
    let cache = mget(vm, "shader_part_filter_cache")?;
    let n = vm.call_method(&cache, "get", vec![names.clone(), Value::None])?;
    if !py::is_none(&n) {
        return Ok(n);
    }
    let n = vm.call(&f, vec![names.clone()], Vec::new())?;
    ops::setitem(vm, &cache, names, n.clone())?;
    Ok(n)
}

fn normalize_partnames(vm: &mut Vm, names: &Value) -> PyResult<Value> {
    let mut yes = std::collections::BTreeSet::new();
    let mut no = HashSet::new();
    for n in ops::collect(vm, names)? {
        let s = py::string(vm, &n)?;
        match s.strip_prefix('-') {
            Some(r) => {
                no.insert(r.to_owned());
            }
            None => {
                yes.insert(s);
            }
        }
    }
    yes.retain(|x| !no.contains(x));
    let ftl = if at_least(vm, 7, 4) { "renpy.ftl" } else { "" };
    if ftl.is_empty() || !yes.contains(ftl) {
        let d = py::config(vm, "default_shader")?;
        yes.insert(py::string(vm, &d)?);
    }
    let items = yes.iter().map(|s| vm.types.new_str(s)).collect();
    Ok(vm.types.new_tuple(items))
}

fn sc_filter_partnames(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    filter_partnames(vm, args[1].clone())
}

fn sc_normalize_partnames(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    normalize_partnames(vm, &args[1])
}

fn cache_get(vm: &mut Vm, this: &Value, key: &Value) -> PyResult<Value> {
    let c = py::get(vm, this, "cache")?;
    vm.call_method(&c, "get", vec![key.clone(), Value::None])
}

fn sc_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut names = args[1].clone();
    if since_pair(vm, 8, 3) {
        names = filter_partnames(vm, names)?;
    }
    let rv = cache_get(vm, this, &names)?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    sc_get_filtered(vm, &[this.clone(), names], &[])
}

/// Builds the sources of an uncached combination, then needs a GL program.
fn sc_get_filtered(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = &args[1];
    let rv = cache_get(vm, this, names)?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let sorted = normalize_partnames(vm, names)?;
    let rv = cache_get(vm, this, &sorted)?;
    if !py::is_none(&rv) {
        let c = py::get(vm, this, "cache")?;
        ops::setitem(vm, &c, names.clone(), rv.clone())?;
        return Ok(rv);
    }
    let v84 = at_least(vm, 8, 4);
    let v86 = at_least(vm, 8, 6);
    let sp = mget(vm, "shader_part")?;
    let (mut vv, mut vp, mut vf, mut fv, mut fp, mut ff) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for n in ops::collect(vm, &sorted)? {
        let p = vm.call_method(&sp, "get", vec![n.clone(), Value::None])?;
        if py::is_none(&p) {
            let r = ops::repr(vm, &n)?;
            return Err(py::error(vm, format!("{r} is not a known shader part.")));
        }
        let glsl = if v86 { py::get(vm, &p, "glsl")? } else { Value::None };
        vv.extend(py::get_items(vm, &p, "vertex_variables")?);
        fv.extend(py::get_items(vm, &p, "fragment_variables")?);
        for (src, dst) in [("vertex_parts", &mut vp), ("fragment_parts", &mut fp)] {
            for t in py::get_items(vm, &p, src)? {
                if v86 {
                    let mut it = py::items(vm, &t)?;
                    it.push(glsl.clone());
                    dst.push(vm.types.new_tuple(it));
                } else {
                    dst.push(t);
                }
            }
        }
        for (src, dst) in [("vertex_functions", &mut vf), ("fragment_functions", &mut ff)] {
            let f = py::get(vm, &p, src)?;
            dst.push(if v86 { vm.types.new_tuple(vec![f, glsl.clone()]) } else { f });
        }
    }
    let gles = py::get(vm, this, "gles")?;
    let (vvars, fvars) = if v86 {
        let a = vm.types.new_list(vv);
        let b = vm.types.new_list(fv);
        let l = f_link_variables(vm, &[a, b], &[])?;
        let l = py::items(vm, &l)?;
        (l[0].clone(), l[1].clone())
    } else if v84 {
        (ops::make_set(vm, vv)?, ops::make_set(vm, fv)?)
    } else {
        (ops::make_set(vm, vv)?, ops::make_set(vm, fv)?)
    };
    let mut extra = Vec::new();
    if v86 {
        extra.push(py::get(vm, this, "version")?);
    }
    let src = |vm: &mut Vm, vars: Value, parts: Vec<Value>, funcs: Vec<Value>, frag: bool| {
        let mut a = vec![vars, vm.types.new_list(parts), vm.types.new_list(funcs), Value::Bool(frag), gles.clone()];
        a.extend(extra.iter().cloned());
        f_source(vm, &a, &[])
    };
    let vertex = src(vm, vvars, vp, vf, false)?;
    let fragment = src(vm, fvars, fp, ff, true)?;
    if at_least(vm, 7, 4) {
        sc_log_shader(vm, &[this.clone(), py::s(vm, "vertex"), sorted.clone(), vertex], &[])?;
        sc_log_shader(vm, &[this.clone(), py::s(vm, "fragment"), sorted.clone(), fragment], &[])?;
    }
    let names_s = ops::repr(vm, &sorted)?;
    Err(py::error(
        vm,
        format!(
            "Shader {names_s} can't be compiled: GL shader programs need an OpenGL driver, which this runtime does not have."
        ),
    ))
}

fn sc_check(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sp = mget(vm, "shader_part")?;
    for n in ops::collect(vm, &args[1])? {
        if !ops::contains(vm, &sp, &n)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn sc_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = filter_partnames(vm, args[1].clone())?;
    let c = py::get(vm, this, "cache")?;
    if ops::contains(vm, &c, &names)? {
        return Ok(Value::None);
    }
    let norm = normalize_partnames(vm, &names)?;
    if ops::contains(vm, &c, &norm)? {
        return Ok(Value::None);
    }
    let p = py::get(vm, this, "predicted")?;
    py::method(vm, &p, "setdefault", vec![norm, names])?;
    Ok(Value::None)
}

fn sc_preload_predicted(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let p = py::get(vm, this, "predicted")?;
    while py::truthy(vm, &p)? {
        let it = py::method_kw(vm, &p, "popitem", Vec::new(), vec![("last".into(), Value::Bool(false))])?;
        let it = py::items(vm, &it)?;
        let c = py::get(vm, this, "cache")?;
        if ops::contains(vm, &c, &it[0])? {
            continue;
        }
        if let Err(e) = sc_get_filtered(vm, &[this.clone(), it[1].clone()], &[])
            && py::config_bool(vm, "debug_prediction").unwrap_or(false)
        {
            return Err(e);
        }
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

fn sc_save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if at_least(vm, 8, 6) {
        sc_save_program_cache(vm, args, &[])?;
    }
    if at_least(vm, 7, 4) {
        let d = py::get(vm, this, "dirty")?;
        if !py::truthy(vm, &d)? || !py::config_bool(vm, "developer").unwrap_or(false) {
            return Ok(Value::None);
        }
    }
    let c = py::get(vm, this, "cache")?;
    let mut lines = std::collections::BTreeSet::new();
    let mut keys = ops::collect(vm, &c)?;
    keys.extend(py::get_items(vm, this, "missing")?);
    for k in keys {
        let mut parts = Vec::new();
        for n in ops::collect(vm, &k)? {
            parts.push(py::string(vm, &n)?);
        }
        lines.insert(parts.join(" "));
    }
    let nl = if at_least(vm, 8, 6) { "\n" } else { "\r\n" };
    let text: String = lines.iter().map(|l| format!("{l}{nl}")).collect();
    let f = py::get(vm, this, "filename")?;
    let f = py::string(vm, &f)?;
    if vm.host.write_file(&format!("/game/{f}"), text.as_bytes()) {
        if at_least(vm, 7, 4) {
            py::set(vm, this, "dirty", Value::Bool(false))?;
        }
    } else {
        super::log::write(vm, &format!("Saving shaders to '{f}':"));
    }
    Ok(Value::None)
}

fn sc_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = py::get(vm, this, "filename")?;
    let fname = py::string(vm, &f)?;
    let Some(data) = super::resloader::read(vm, &fname) else {
        let r = ops::repr(vm, &f)?;
        super::log::write(vm, &format!("Could not open {r}:"));
        return Ok(Value::None);
    };
    let text = String::from_utf8_lossy(&data).into_owned();
    for l in text.lines() {
        let names: Vec<Value> = l.split_whitespace().map(|s| vm.types.new_str(s)).collect();
        if names.is_empty() {
            continue;
        }
        let t = vm.types.new_tuple(names);
        let ok = sc_check(vm, &[this.clone(), t.clone()], &[])?;
        let missing = py::get(vm, this, "missing")?;
        if !py::truthy(vm, &ok)? {
            ops::set_add(vm, &missing, t)?;
            continue;
        }
        if let Err(e) = sc_get(vm, &[this.clone(), t.clone()], &[]) {
            let r = ops::repr(vm, &t)?;
            super::log::write(vm, &format!("Precompiling shader {r}:"));
            let msg = py::error_text(vm, &e);
            super::log::write(vm, &msg);
            ops::set_add(vm, &missing, t)?;
        }
    }
    Ok(Value::None)
}

fn sc_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let names: &[&str] = if at_least(vm, 8, 6) { &["cache", "missing", "predicted"] } else { &["cache", "missing"] };
    for n in names {
        let v = py::get(vm, &args[0], n)?;
        py::method(vm, &v, "clear", Vec::new())?;
    }
    Ok(Value::None)
}

fn sc_log_shader(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "log_gl_shaders").unwrap_or(false) {
        return Ok(Value::None);
    }
    let kind = py::string(vm, &args[1])?;
    let mut names = Vec::new();
    for n in ops::collect(vm, &args[2])? {
        names.push(py::string(vm, &n)?);
    }
    let mut name = format!("{kind} {} ", names.join(", "));
    let pad = 80usize.saturating_sub(name.chars().count());
    name.push_str(&"-".repeat(pad));
    let text = py::string(vm, &args[3])?;
    super::log::write(vm, &name);
    super::log::write(vm, &text);
    super::log::write(vm, &"-".repeat(80));
    Ok(Value::None)
}

/// Program binaries need a GL driver; with `program_binary_supported`
/// False these follow upstream's unsupported paths.
fn sc_load_program_cache(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "program_cache_loaded", Value::Bool(true))?;
    Ok(Value::None)
}

fn sc_discard_program_binary(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let pc = py::get(vm, this, "program_cache")?;
    let e = vm.call_method(&pc, "pop", vec![args[1].clone(), Value::None])?;
    if !py::is_none(&e) {
        let sz = f_program_cache_entry_size(vm, &[args[1].clone(), e], &[])?;
        let cur = py::get(vm, this, "program_cache_size")?;
        let n = py::int(vm, &cur)? - py::int(vm, &sz)?;
        py::set(vm, this, "program_cache_size", Value::Int(n))?;
        py::set(vm, this, "program_cache_dirty", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn sc_get_program_binary(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    sc_load_program_cache(vm, &args[..1], &[])?;
    let key = f_shader_program_key(vm, &[args[1].clone(), args[2].clone()], &[])?;
    let pc = py::get(vm, &args[0], "program_cache")?;
    let e = vm.call_method(&pc, "get", vec![key.clone(), Value::None])?;
    if py::is_none(&e) {
        return Ok(Value::None);
    }
    let it = py::items(vm, &e)?;
    if !ops::eq(vm, &it[0], &args[3])? {
        sc_discard_program_binary(vm, &[args[0].clone(), key], &[])?;
        return Ok(Value::None);
    }
    let dec = |vm: &mut Vm, b: &Value| py::method(vm, b, "decode", vec![py::s(vm, "utf-8")]);
    let r = (|| -> PyResult<Value> {
        let bin = vm.types.new_tuple(vec![it[1].clone(), it[2].clone()]);
        let v = dec(vm, &it[3])?;
        let f = dec(vm, &it[4])?;
        let specs = f_decode_program_variable_specs(vm, &[it[5].clone()], &[])?;
        Ok(vm.types.new_tuple(vec![bin, v, f, specs]))
    })();
    match r {
        Ok(v) => Ok(v),
        Err(_) => {
            sc_discard_program_binary(vm, &[args[0].clone(), key], &[])?;
            Ok(Value::None)
        }
    }
}

fn sc_noop_unsupported(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::get(vm, &args[0], "program_binary_supported")?;
    if py::truthy(vm, &s)? {
        return Err(py::error(vm, "Program binaries need an OpenGL driver, which this runtime does not have."));
    }
    Ok(Value::None)
}

fn sc_save_program_cache(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    sc_noop_unsupported(vm, args, kw)
}

fn sc_store_program_binary(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    sc_noop_unsupported(vm, args, kw)
}

// Module --------------------------------------------------------------------------------------

fn compile(vm: &mut Vm, pat: &str, flags: Option<&str>) -> PyResult<Value> {
    let c = py::import_attr(vm, "re", "compile")?;
    let mut a = vec![py::s(vm, pat)];
    if let Some(f) = flags {
        a.push(py::import_attr(vm, "re", f)?);
    }
    vm.call(&c, a, Vec::new())
}

fn standalone(n: &str) -> String {
    format!(r"(?<![\w.]){n}\b")
}

fn install_86_constants(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    let fo = "fragment_color";
    set(vm, "FRAGMENT_OUTPUT", py::s(vm, fo));
    let dv = vm.types.new_dict();
    for d in DIALECTS {
        let (a, b) = dialect_versions(d);
        let t = vm.types.new_tuple(vec![Value::Int(a), Value::Int(b)]);
        ops::setitem(vm, &dv, Value::Int(d), t)?;
    }
    set(vm, "DIALECT_VERSIONS", dv);
    set(vm, "DIALECTS", vm.types.new_tuple(DIALECTS.iter().map(|d| Value::Int(*d)).collect()));
    let gd = vm.types.new_dict();
    for v in [100, 120, 300, 330] {
        ops::setitem(vm, &gd, Value::Int(v), Value::Int(glsl_dialect(v).unwrap_or(100)))?;
    }
    set(vm, "GLSL_DIALECTS", gd);
    set(vm, "GLSL_VERSIONS", vm.types.new_tuple([100, 120, 300, 330].iter().map(|d| Value::Int(*d)).collect()));
    set(vm, "STANDALONE", py::s(vm, r"(?<![\w.]){}\b"));
    let pair_list = |vm: &mut Vm, items: &[(String, &str)]| -> PyResult<Value> {
        let mut l = Vec::new();
        for (p, r) in items {
            let c = compile(vm, p, None)?;
            l.push(vm.types.new_tuple(vec![c, py::s(vm, r)]));
        }
        Ok(vm.types.new_list(l))
    };
    let l2m: Vec<(String, &str)> = [
        ("texture2DProjLodEXT", "textureProjLod"),
        ("texture2DLodEXT", "textureLod"),
        ("texture2DProjLod", "textureProjLod"),
        ("texture2DLod", "textureLod"),
        ("texture2DProj", "textureProj"),
        ("texture2D", "texture"),
        ("textureCubeLod", "textureLod"),
        ("textureCube", "texture"),
        ("gl_FragColor", fo),
    ]
    .iter()
    .map(|(a, b)| (standalone(a), *b))
    .collect();
    let v = pair_list(vm, &l2m)?;
    set(vm, "LEGACY_TO_MODERN", v);
    let m2l: Vec<(String, &str)> = vec![
        (standalone(fo), "gl_FragColor"),
        (r"(?<![\w.])textureProjLod\s*\(".into(), "texture2DProjLod("),
        (r"(?<![\w.])textureLod\s*\(".into(), "texture2DLod("),
        (r"(?<![\w.])textureProj\s*\(".into(), "texture2DProj("),
        (r"(?<![\w.])texture\s*\(".into(), "texture2D("),
    ];
    let v = pair_list(vm, &m2l)?;
    set(vm, "MODERN_TO_LEGACY", v);
    let le = vm.types.new_dict();
    let a = vm.types.new_tuple(vec![py::s(vm, "GL_EXT_shader_texture_lod"), py::s(vm, "EXT")]);
    ops::setitem(vm, &le, Value::Bool(true), a)?;
    let b = vm.types.new_tuple(vec![py::s(vm, "GL_ARB_shader_texture_lod"), py::s(vm, "")]);
    ops::setitem(vm, &le, Value::Bool(false), b)?;
    set(vm, "LOD_EXTENSION", le);
    let lf = compile(vm, r"(?<![\w.])(texture2DProjLod|texture2DLod)(?:EXT)?\s*\(", None)?;
    set(vm, "LOD_FUNCTIONS", lf);
    let gc = compile(vm, r"//[^\n]*|/\*.*?\*/", Some("S"))?;
    set(vm, "GLSL_COMMENTS", gc);
    let mut lm = Vec::new();
    for (a, b) in [
        ("gl_FragColor", fo),
        ("gl_FragData", fo),
        ("texture2DProjLodEXT", "textureProjLod"),
        ("texture2DLodEXT", "textureLod"),
        ("texture2DProjLod", "textureProjLod"),
        ("texture2DLod", "textureLod"),
        ("texture2DProj", "textureProj"),
        ("texture2D", "texture"),
        ("textureCubeLod", "textureLod"),
        ("textureCube", "texture"),
    ] {
        let c = compile(vm, &standalone(a), None)?;
        lm.push(vm.types.new_tuple(vec![c, py::s(vm, a), py::s(vm, b)]));
    }
    set(vm, "LEGACY_MARKERS", vm.types.new_list(lm));
    let ls = vm.types.new_dict();
    py::dict_set(vm, &ls, "attribute", py::s(vm, "in"))?;
    py::dict_set(vm, &ls, "varying", py::s(vm, "out"))?;
    set(vm, "LEGACY_STORAGE", ls);
    let tm: Vec<(String, &str)> = vec![
        (standalone(fo), fo),
        (standalone("textureLod"), "textureLod"),
        (standalone("textureProjLod"), "textureProjLod"),
        (r"(?<![\w.])texture\s*\(".into(), "texture"),
    ];
    let tml = pair_list(vm, &tm)?;
    let tmd = vm.types.new_dict();
    ops::setitem(vm, &tmd, Value::Int(300), tml)?;
    set(vm, "TRANSLATED_MARKERS", tmd);
    let um: Vec<(String, &str)> = vec![
        (standalone("textureSize"), "textureSize"),
        (standalone("texelFetch"), "texelFetch"),
        (standalone("textureGrad"), "textureGrad"),
        (standalone("switch"), "switch"),
        ("%".into(), "the % operator"),
        ("<<|>>".into(), "a bitwise shift"),
        (r"(?<!&)&(?!&)|(?<!\|)\|(?!\|)|(?<!\^)\^(?!\^)".into(), "a bitwise operator"),
    ];
    let uml = pair_list(vm, &um)?;
    let umd = vm.types.new_dict();
    ops::setitem(vm, &umd, Value::Int(300), uml)?;
    set(vm, "UNTRANSLATED_MARKERS", umd);
    let flat = vm.types.new_set(Default::default());
    for t in FLAT_TYPES {
        ops::set_add(vm, &flat, py::s(vm, t))?;
    }
    let ut = vm.types.new_dict();
    ops::setitem(vm, &ut, Value::Int(300), flat.clone())?;
    set(vm, "UNTRANSLATED_TYPES", ut);
    set(vm, "FLAT_TYPES", flat);
    set(vm, "PROGRAM_CACHE_MAGIC", vm.types.new_bytes(b"renpy shader programs\x00\x00".to_vec()));
    set(vm, "PROGRAM_CACHE_RECIPE", vm.types.new_bytes(b"renpy shader recipe\x00\x00".to_vec()));
    set(vm, "PROGRAM_CACHE_RECORD", py::s(vm, "<32s32sIIIII"));
    set(vm, "PROGRAM_CACHE_RECORD_SIZE", Value::Int(32 + 32 + 4 * 5));
    set(vm, "MAX_PROGRAM_BINARY_SIZE", Value::Int(16 * 1024 * 1024));
    set(vm, "MAX_PROGRAM_SOURCE_SIZE", Value::Int(4 * 1024 * 1024));
    set(vm, "MAX_PROGRAM_VARIABLES_SIZE", Value::Int(1024 * 1024));
    set(vm, "MAX_PROGRAM_CACHE_SIZE", Value::Int(256 * 1024 * 1024));
    Ok(())
}

fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let v83 = since_pair(vm, 8, 3);
    let v84 = at_least(vm, 8, 4);
    let v86 = at_least(vm, 8, 6);
    let mut funcs: Vec<(&'static str, crysirror_python::object::NativeFn)> =
        vec![("register_shader", register_shader), ("source", f_source)];
    if v86 {
        funcs.extend([
            ("_get_shader_cache", f_get_shader_cache as crysirror_python::object::NativeFn),
            ("predict_shader", f_predict_shader),
            ("has_predicted_shaders", f_has_predicted_shaders),
            ("preload_predicted_shader", f_preload_predicted_shader),
            ("dialect_name", f_dialect_name),
            ("newer_markers", f_newer_markers),
            ("config_dialect", f_config_dialect),
            ("parse_glsl_version", f_parse_glsl_version),
            ("dialect_version", f_dialect_version),
            ("emit_version", f_emit_version),
            ("translate", f_translate),
            ("merge_variables", f_merge_variables),
            ("link_variables", f_link_variables),
            ("shader_program_key", f_shader_program_key),
            ("shader_recipe_digest", f_shader_recipe_digest),
            ("program_cache_entry_size", f_program_cache_entry_size),
            ("decode_program_variable_specs", f_decode_program_variable_specs),
        ]);
    }
    let m = crysirror_python::modules::module_with(vm, M, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "shader_part", py::dict(vm));
    set(vm, "cache", py::dict(vm));
    if v83 {
        set(vm, "shader_part_filter_cache", py::dict(vm));
    }
    if v86 {
        install_86_constants(vm, &m)?;
    }
    let mut methods: Vec<(&'static str, crysirror_python::object::NativeFn)> =
        vec![("__init__", sp_init)];
    if v83 {
        methods.extend([
            ("substitute_name", sp_substitute_name as crysirror_python::object::NativeFn),
            ("expand_name", sp_expand_name),
            ("expand_match", sp_expand_match),
        ]);
    }
    if v84 {
        methods.push(("expand_operation", sp_expand_operation));
    }
    let mut attrs: Vec<(&str, Value)> = Vec::new();
    if v86 {
        methods.extend([
            ("find_minimum_dialect", sp_find_minimum_dialect as crysirror_python::object::NativeFn),
            ("untranslated_type", sp_untranslated_type),
            ("untranslated_interpolation", sp_untranslated_interpolation),
            ("part_source", sp_part_source),
            ("check_modern_syntax", sp_check_modern_syntax),
            ("find_legacy_syntax", sp_find_legacy_syntax),
        ]);
        for (n, f) in [
            ("glsl", sp_glsl as crysirror_python::object::NativeFn),
            ("minimum_dialect", sp_minimum_dialect),
            ("minimum_feature", sp_minimum_feature),
        ] {
            let g = py::builtin(vm, n, f);
            let p = py::property(vm, g, None)?;
            attrs.push((n, p));
        }
    }
    let sp = py::class(vm, M, "ShaderPart", &[], &methods, &attrs)?;
    set(vm, "ShaderPart", Value::Obj(sp));
    let mut sc_methods: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("__init__", sc_init),
        ("get", sc_get),
        ("check", sc_check),
        ("save", sc_save),
        ("load", sc_load),
        ("clear", sc_clear),
        ("log_shader", sc_log_shader),
    ];
    if v86 {
        sc_methods.extend([
            ("load_program_cache", sc_load_program_cache as crysirror_python::object::NativeFn),
            ("discard_program_binary", sc_discard_program_binary),
            ("get_program_binary", sc_get_program_binary),
            ("store_program_binary", sc_store_program_binary),
            ("save_program_cache", sc_save_program_cache),
            ("_filter_partnames", sc_filter_partnames),
            ("_normalize_partnames", sc_normalize_partnames),
            ("predict", sc_predict),
            ("preload_predicted", sc_preload_predicted),
            ("_get_filtered", sc_get_filtered),
        ]);
    }
    let sc = py::class(vm, M, "ShaderCache", &[], &sc_methods, &[])?;
    set(vm, "ShaderCache", Value::Obj(sc));
    if v84 {
        let vmethods: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
            ("__init__", var_init),
            ("__hash__", var_hash),
            ("__eq__", var_eq),
            ("declaration", var_declaration),
            ("merge", var_merge),
        ];
        let vc = py::class(vm, M, "Variable", &[], &vmethods, &[])?;
        set(vm, "_Variable", Value::Obj(vc));
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(M, init_module);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_operations_expand() {
        assert_eq!(substitute_names("my.part", "u__a + xu__b", false), "u_my_part_a + xu_my_part_b");
        assert_eq!(substitute_names("my.part", "u__a + xu__b", true), "u_my_part_a + xu__b");
        assert_eq!(substitute_operations("u_shader__blend + u_x"), "u_shader_OP_blend + u_x");
        assert_eq!(substitute_operations("u_a_b__c__d"), "u_a_b__c_OP_d");
    }

    #[test]
    fn variables_parse_by_release() {
        let v = parse_variable(true, "p", "in vec2 a_pos;", false).unwrap();
        assert_eq!((v.storage, v.ty.as_deref(), v.name.as_deref()), (Some("attribute"), Some("vec2"), Some("a_pos")));
        let v = parse_variable(true, "p", "in vec2 v_pos;", true).unwrap();
        assert_eq!(v.storage, Some("varying"));
        assert!(parse_variable(false, "p", "in vec2 a;", false).unwrap().storage.is_none());
        let v = parse_variable(true, "p", "uniform highp float u_x[4]", false).unwrap();
        assert_eq!((v.array, v.precision.as_deref()), (Some(4), Some("highp")));
        assert!(parse_variable(true, "p", "layout(location=1) out vec4 x;", false).is_err());
        assert!(parse_variable(false, "p", "uniform sampler3D t;", false).is_err());
    }
}
