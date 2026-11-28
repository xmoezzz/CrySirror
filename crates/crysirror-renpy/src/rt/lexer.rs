// SPDX-License-Identifier: MPL-2.0
//! `renpy.lexer`, after Ren'Py 8.3 `lexer.py` (MIT): the script lexer as
//! creator-defined statements see it. The `Lexer` class is a Python class
//! over the Rust lexer (`crate::lexer`), so a statement's parse function and
//! the built-in parser read the same token rules. Its method set is the
//! union of 6.18–8.6 (`renpy.parser.Lexer` before 8.0); `triple_string`
//! returns the 8.x list of monologue strings.
use super::py::{self, *};
use crate::lexer::{GroupedLine, Lexer, ParseError, TripleString};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, rc::Rc};

pub const MODULE: &str = "renpy.lexer";
const STATE: &str = "_crysirror_lexer";

pub type LexRef = Rc<RefCell<Lexer>>;

struct Classes {
    lexer: ObjRef,
    holder: ObjRef,
    parse_error: ObjRef,
    subparse: ObjRef,
}

fn classes(vm: &mut Vm) -> PyResult<Rc<Classes>> {
    if let Some(c) = vm.ext_get::<Rc<Classes>>() {
        return Ok(c.clone());
    }
    py::import(vm, MODULE)?;
    Ok(vm.ext_get::<Rc<Classes>>().expect("renpy.lexer").clone())
}

/// The Rust lexer behind a Python `Lexer`.
pub fn state(vm: &Vm, v: &Value) -> PyResult<LexRef> {
    let st = v.obj().and_then(|o| ops::get_raw_attr(o, STATE));
    st.and_then(|s| py::native::<LexRef, _>(&s, |r| r.clone()))
        .ok_or_else(|| vm.type_error("not a Ren'Py lexer"))
}

/// Wraps a Rust lexer as a Python `Lexer` sharing `subparses`.
pub fn new_lexer(vm: &mut Vm, l: Lexer, subparses: Value) -> PyResult<Value> {
    let c = classes(vm)?;
    let this = vm.call(&Value::Obj(c.lexer.clone()), vec![Value::None], Vec::new())?;
    let o = this.obj().unwrap().clone();
    let h = vm
        .types
        .new_native(&c.holder, Box::new(Rc::new(RefCell::new(l)) as LexRef));
    ops::set_raw_attr(vm, &o, STATE, h);
    ops::set_raw_attr(vm, &o, "subparses", subparses);
    Ok(this)
}

/// `ParseError(filename, number, msg, line, pos)` as a Python exception.
pub fn parse_error(vm: &mut Vm, e: &ParseError) -> crysirror_python::PyErr {
    let c = match classes(vm) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let args = vec![
        vm.types.new_str(&e.filename),
        Value::Int(e.line as i64),
        vm.types.new_str(&e.msg),
        e.text
            .as_deref()
            .map(|t| vm.types.new_str(t))
            .unwrap_or(Value::None),
        e.offset
            .map(|o| Value::Int(o as i64 - 1))
            .unwrap_or(Value::None),
    ];
    match vm.call(&Value::Obj(c.parse_error.clone()), args, Vec::new()) {
        Ok(v) => crysirror_python::PyErr::Exc(v.obj().unwrap().clone()),
        Err(e) => e,
    }
}

pub fn with<R>(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut Lexer) -> R) -> PyResult<R> {
    let l = state(vm, this)?;
    let r = f(&mut l.borrow_mut());
    Ok(r)
}

fn try_with<R>(
    vm: &mut Vm,
    this: &Value,
    f: impl FnOnce(&mut Lexer) -> crate::lexer::PResult<R>,
) -> PyResult<R> {
    let r = with(vm, this, f)?;
    r.map_err(|e| parse_error(vm, &e))
}

fn opt_str(vm: &Vm, s: Option<String>) -> Value {
    s.map(|s| vm.types.new_str(&s)).unwrap_or(Value::None)
}

/// `[(filename, number, text, subblock)]`.
pub fn block_value(vm: &Vm, b: &[GroupedLine]) -> Value {
    let items = b
        .iter()
        .map(|l| {
            let t: String = l.text.iter().collect();
            let f = vm.types.new_str(&l.filename);
            let tv = vm.types.new_str(&t);
            let sub = block_value(vm, &l.block);
            vm.types
                .new_tuple(vec![f, Value::Int(l.number as i64), tv, sub])
        })
        .collect();
    vm.types.new_list(items)
}

/// The inverse of `block_value`; accepts 4- and 5-element entries.
pub fn block_from(vm: &mut Vm, v: &Value) -> PyResult<Vec<GroupedLine>> {
    let mut out = Vec::new();
    for it in py::items(vm, v)? {
        let p = py::items(vm, &it)?;
        let (f, n, indent, text, sub) = match p.len() {
            4 => (&p[0], &p[1], None, &p[2], &p[3]),
            5 => (&p[0], &p[1], Some(&p[2]), &p[3], &p[4]),
            _ => {
                return Err(
                    vm.type_error("lexer block entries are (filename, number, text, subblock)")
                );
            }
        };
        let filename: Rc<str> = py::string(vm, f)?.into();
        let number = py::int(vm, n)? as u32;
        let indent = match indent {
            Some(i) => py::int(vm, i)? as usize,
            None => 0,
        };
        let text: Rc<[char]> = py::string(vm, text)?.chars().collect::<Vec<_>>().into();
        let block = Rc::new(block_from(vm, sub)?);
        out.push(GroupedLine {
            filename,
            number,
            indent,
            text,
            block,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Attributes.

fn l_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let block = arg_or(&args[1..], kw, 0, "block", Value::None);
    if py::is_none(&block) {
        // Constructed by `new_lexer`, which installs the state.
        return Ok(Value::None);
    }
    let init = arg_or(&args[1..], kw, 1, "init", Value::Bool(false));
    let init_offset = arg_or(&args[1..], kw, 2, "init_offset", Value::Int(0));
    let global_label = arg_or(&args[1..], kw, 3, "global_label", Value::None);
    let mondel = arg_or(&args[1..], kw, 4, "monologue_delimiter", py::s(vm, "\n\n"));
    let subparses = arg_or(&args[1..], kw, 5, "subparses", Value::None);
    let b = block_from(vm, &block)?;
    let mut l = Lexer::new(Rc::new(b), py::truthy(vm, &init)?);
    l.init_offset = py::int(vm, &init_offset)? as i32;
    if !py::is_none(&global_label) {
        l.global_label = Some(py::string(vm, &global_label)?.into());
    }
    l.monologue_delimiter = py::string(vm, &mondel)?;
    let c = classes(vm)?;
    let h = vm
        .types
        .new_native(&c.holder, Box::new(Rc::new(RefCell::new(l)) as LexRef));
    let o = this.obj().unwrap().clone();
    ops::set_raw_attr(vm, &o, STATE, h);
    ops::set_raw_attr(vm, &o, "subparses", subparses);
    Ok(Value::None)
}

const FIELDS: &[&str] = &[
    "line",
    "eob",
    "filename",
    "text",
    "number",
    "subblock",
    "pos",
    "init",
    "init_offset",
    "global_label",
    "block",
    "monologue_delimiter",
];

fn l_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[1])?;
    if !FIELDS.contains(&n.as_str()) {
        if n.starts_with("word_cache") {
            return Ok(Value::Int(-1));
        }
        return Err(vm.attr_error(format!("'Lexer' object has no attribute '{n}'")));
    }
    let l = state(vm, &args[0])?;
    let l = l.borrow();
    Ok(match n.as_str() {
        "line" => Value::Int(l.line as i64),
        "eob" => Value::Bool(l.eob),
        "filename" => vm.types.new_str(&l.filename),
        "text" => vm.types.new_str(&l.text.iter().collect::<String>()),
        "number" => Value::Int(l.number as i64),
        "subblock" => block_value(vm, &l.subblock),
        "pos" => Value::Int(l.pos as i64),
        "init" => Value::Bool(l.init),
        "init_offset" => Value::Int(l.init_offset as i64),
        "global_label" => l
            .global_label
            .as_deref()
            .map(|g| vm.types.new_str(g))
            .unwrap_or(Value::None),
        "block" => block_value(vm, &l.block),
        _ => vm.types.new_str(&l.monologue_delimiter),
    })
}

fn l_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, name, v) = (&args[0], &args[1], &args[2]);
    let n = py::string(vm, name)?;
    if !FIELDS.contains(&n.as_str()) {
        let o = this.obj().unwrap().clone();
        ops::set_raw_attr(vm, &o, &n, v.clone());
        return Ok(Value::None);
    }
    let st = state(vm, this)?;
    match n.as_str() {
        "line" => st.borrow_mut().line = py::int(vm, v)? as isize,
        "eob" => st.borrow_mut().eob = py::truthy(vm, v)?,
        "filename" => st.borrow_mut().filename = py::string(vm, v)?.into(),
        "text" => st.borrow_mut().text = py::string(vm, v)?.chars().collect::<Vec<_>>().into(),
        "number" => st.borrow_mut().number = py::int(vm, v)? as u32,
        "subblock" => {
            let b = block_from(vm, v)?;
            st.borrow_mut().subblock = Rc::new(b);
        }
        "pos" => st.borrow_mut().pos = py::int(vm, v)?.max(0) as usize,
        "init" => st.borrow_mut().init = py::truthy(vm, v)?,
        "init_offset" => st.borrow_mut().init_offset = py::int(vm, v)? as i32,
        "global_label" => {
            st.borrow_mut().global_label = if py::is_none(v) {
                None
            } else {
                Some(py::string(vm, v)?.into())
            }
        }
        "block" => {
            let b = block_from(vm, v)?;
            st.borrow_mut().block = Rc::new(b);
        }
        _ => st.borrow_mut().monologue_delimiter = py::string(vm, v)?,
    }
    Ok(Value::None)
}

// ---------------------------------------------------------------------------
// Methods.

fn l_advance(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(with(vm, &args[0], |l| l.advance())?))
}

fn l_unadvance(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    with(vm, &args[0], |l| l.unadvance())?;
    Ok(Value::None)
}

fn l_match_regexp(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let pat = py::string(vm, &args[1])?;
    let r = with(vm, &args[0], |l| l.match_regexp(&pat))?;
    Ok(opt_str(vm, r))
}

fn l_skip_whitespace(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    with(vm, &args[0], |l| l.skip_whitespace())?;
    Ok(Value::None)
}

fn l_match(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let pat = py::string(vm, &args[1])?;
    let r = with(vm, &args[0], |l| l.match_re(&pat))?;
    Ok(opt_str(vm, r))
}

fn l_match_multiple(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = state(vm, &args[0])?;
    let start = st.borrow().pos;
    let mut out = Vec::new();
    for p in &args[1..] {
        let pat = py::string(vm, p)?;
        let r = st.borrow_mut().match_re(&pat);
        match r {
            Some(s) => out.push(vm.types.new_str(&s)),
            None => {
                st.borrow_mut().pos = start;
                return Ok(Value::None);
            }
        }
    }
    Ok(vm.types.new_tuple(out))
}

fn l_keyword(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let w = py::string(vm, &args[1])?;
    let r = with(vm, &args[0], |l| l.keyword(&w))?;
    Ok(if r {
        vm.types.new_str(&w)
    } else {
        py::s(vm, "")
    })
}

fn l_error(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let msg = py::string(vm, &args[1])?;
    let e = with(vm, &args[0], |l| l.error::<()>(msg).unwrap_err())?;
    Err(parse_error(vm, &e))
}

fn l_deferred_error(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let queue = args[1].clone();
    let msg = py::string(vm, &args[2])?;
    let e = with(vm, &args[0], |l| l.error::<()>(msg).unwrap_err())?;
    let err = parse_error(vm, &e);
    if let crysirror_python::PyErr::Exc(o) = err {
        vm.call_method(&Value::Obj(o), "defer", vec![queue])?;
        return Ok(Value::None);
    }
    Err(err)
}

/// `with l.catch_error():` records a ParseError in the parse errors.
fn l_catch_error(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "_CatchError")?;
    vm.call(&c, Vec::new(), Vec::new())
}

fn ce_enter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn ce_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = args.get(2).cloned().unwrap_or(Value::None);
    let pe = Value::Obj(classes(vm)?.parse_error.clone());
    if py::is_none(&v) || !pe.obj().is_some_and(|c| ops::isinstance(vm, &v, c)) {
        return Ok(Value::Bool(false));
    }
    let msg = py::get(vm, &v, "message")?;
    let errors = py::import_attr(vm, "renpy.parser", "parse_errors")?;
    vm.call_method(&errors, "append", vec![msg])?;
    Ok(Value::Bool(true))
}

fn l_eol(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(with(vm, &args[0], |l| l.eol())?))
}

fn l_expect_eol(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    try_with(vm, &args[0], |l| l.expect_eol())?;
    Ok(Value::None)
}

fn l_expect_noblock(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[1])?;
    try_with(vm, &args[0], |l| l.expect_noblock(&s))?;
    Ok(Value::None)
}

fn l_expect_block(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[1])?;
    try_with(vm, &args[0], |l| l.expect_block(&s))?;
    Ok(Value::None)
}

fn l_has_block(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(with(vm, &args[0], |l| l.has_block())?))
}

fn l_subblock_lexer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let init = arg_or(&args[1..], kw, 0, "init", Value::Bool(false));
    let init = py::truthy(vm, &init)?;
    let sub = with(vm, &args[0], |l| l.subblock_lexer(init))?;
    let sp = py::get_or_none(vm, &args[0], "subparses")?;
    new_lexer(vm, sub, sp)
}

fn l_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = with(vm, &args[0], |l| l.string())?;
    Ok(opt_str(vm, r))
}

fn l_triple_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = with(vm, &args[0], |l| l.triple_string())?;
    Ok(match r {
        None => Value::None,
        Some(TripleString::Raw(s)) => vm.types.new_str(&s),
        Some(TripleString::Parts(p)) => {
            let v = p.iter().map(|(_, s)| vm.types.new_str(s)).collect();
            vm.types.new_list(v)
        }
    })
}

macro_rules! opt_method {
    ($name:ident, $m:ident) => {
        fn $name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
            let r = with(vm, &args[0], |l| l.$m())?;
            Ok(opt_str(vm, r))
        }
    };
}

opt_method!(l_integer, integer);
opt_method!(l_float, float);
opt_method!(l_hash, hash);
opt_method!(l_word, word);
opt_method!(l_name, name);
opt_method!(l_image_name_component, image_name_component);

fn l_set_global_label(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[1])?;
    with(vm, &args[0], |l| l.set_global_label(&s))?;
    Ok(Value::None)
}

fn label_name(vm: &mut Vm, this: &Value, declare: bool) -> PyResult<Value> {
    let r = with(vm, this, |l| {
        let old = l.pos;
        let rv = l.label_name()?;
        if declare {
            if let (Some((g, _)), Some(gl)) = (rv.split_once('.'), l.global_label.as_deref()) {
                if g != gl {
                    l.pos = old;
                    return None;
                }
            }
        }
        Some(rv)
    })?;
    Ok(opt_str(vm, r))
}

fn l_label_name(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(&args[1..], kw, 0, "declare", Value::Bool(false));
    let d = py::truthy(vm, &d)?;
    label_name(vm, &args[0], d)
}

fn l_label_name_declare(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    label_name(vm, &args[0], true)
}

fn l_python_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(try_with(vm, &args[0], |l| l.python_string())?))
}

fn l_dotted_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = try_with(vm, &args[0], |l| l.dotted_name())?;
    Ok(opt_str(vm, r))
}

/// `PyExpr(s, filename, number)` as a Python value.
pub fn pyexpr(vm: &mut Vm, s: &str, filename: &str, number: u32) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.ast", "PyExpr")?;
    let a = vec![
        vm.types.new_str(s),
        vm.types.new_str(filename),
        Value::Int(number as i64),
    ];
    vm.call(&c, a, Vec::new())
}

fn expr_value(vm: &mut Vm, this: &Value, s: Option<String>, expr: bool) -> PyResult<Value> {
    let Some(s) = s else { return Ok(Value::None) };
    if !expr {
        return Ok(vm.types.new_str(&s));
    }
    let (f, n) = with(vm, this, |l| (l.filename.clone(), l.number))?;
    pyexpr(vm, &s, &f, n)
}

fn flag(
    vm: &mut Vm,
    args: &[Value],
    kw: Kw,
    i: usize,
    name: &str,
    default: bool,
) -> PyResult<bool> {
    let v = arg_or(&args[1..], kw, i, name, Value::Bool(default));
    py::truthy(vm, &v)
}

fn l_expr(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[1])?;
    let e = flag(vm, args, kw, 1, "expr", true)?;
    expr_value(vm, &args[0], Some(s), e)
}

fn l_delimited_python(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = py::string(vm, &args[1])?;
    let e = flag(vm, args, kw, 1, "expr", true)?;
    let s = try_with(vm, &args[0], |l| l.delimited_python(&d))?;
    expr_value(vm, &args[0], Some(s), e)
}

fn l_python_expression(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = flag(vm, args, kw, 0, "expr", true)?;
    let s = try_with(vm, &args[0], |l| l.python_expression())?;
    expr_value(vm, &args[0], Some(s.text.to_string()), e)
}

fn l_parenthesised_python(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = state(vm, &args[0])?;
    let start = st.borrow().pos;
    let ok = st
        .borrow_mut()
        .parenthesised_python()
        .map_err(|e| parse_error(vm, &e))?;
    if !ok {
        return Ok(Value::None);
    }
    let (s, f, n) = {
        let l = st.borrow();
        (
            l.text[start..l.pos].iter().collect::<String>(),
            l.filename.clone(),
            l.number,
        )
    };
    pyexpr(vm, s.trim(), &f, n)
}

fn l_simple_expression(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let comma = flag(vm, args, kw, 0, "comma", false)?;
    let operator = flag(vm, args, kw, 1, "operator", true)?;
    let image = flag(vm, args, kw, 2, "image", false)?;
    let s = try_with(vm, &args[0], |l| {
        l.simple_expression_text(comma, operator, image)
    })?;
    expr_value(vm, &args[0], s, true)
}

fn l_comma_expression(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = try_with(vm, &args[0], |l| {
        l.simple_expression_text(true, true, false)
    })?;
    expr_value(vm, &args[0], s, true)
}

fn l_say_expression(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = try_with(vm, &args[0], |l| {
        l.simple_expression_text(false, false, false)
    })?;
    expr_value(vm, &args[0], s, true)
}

fn l_checkpoint(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = with(vm, &args[0], |l| l.clone())?;
    let h = classes(vm)?.holder.clone();
    Ok(vm
        .types
        .new_native(&h, Box::new(Rc::new(RefCell::new(c)) as LexRef)))
}

fn l_revert(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let saved = py::native::<LexRef, _>(&args[1], |r| r.borrow().clone())
        .ok_or_else(|| vm.type_error("not a lexer checkpoint"))?;
    let st = state(vm, &args[0])?;
    let mut l = st.borrow_mut();
    let block = l.block.clone();
    *l = saved;
    l.block = block;
    l.eob = l.line >= 0 && l.line as usize >= l.block.len();
    Ok(Value::None)
}

fn l_get_location(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (f, n) = with(vm, &args[0], |l| l.location())?;
    let f = vm.types.new_str(&f);
    Ok(vm.types.new_tuple(vec![f, Value::Int(n as i64)]))
}

fn l_require(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let thing = &args[1];
    let mut name = args
        .get(2)
        .cloned()
        .or_else(|| py::kw(kw, "name").cloned())
        .unwrap_or(Value::None);
    let rest: Vec<_> = kw.iter().filter(|(k, _)| &**k != "name").cloned().collect();
    let rv = if let Some(pat) = thing.as_str() {
        if py::is_none(&name) {
            name = thing.clone();
        }
        let pat = pat.to_owned();
        let r = with(vm, this, |l| l.match_re(&pat))?;
        opt_str(vm, r)
    } else {
        vm.call(thing, Vec::new(), rest)?
    };
    if py::is_none(&rv) {
        if py::is_none(&name) {
            let f = py::get_opt(vm, thing, "__func__")?.unwrap_or_else(|| thing.clone());
            name = py::get_opt(vm, &f, "__name__")?.unwrap_or_else(|| py::s(vm, "thing"));
        }
        let n = py::string(vm, &name)?;
        let e = with(vm, this, |l| {
            l.error::<()>(format!("expected '{n}' not found."))
                .unwrap_err()
        })?;
        return Err(parse_error(vm, &e));
    }
    Ok(rv)
}

fn l_rest(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let e = with(vm, &args[0], |l| l.rest())?;
    pyexpr(vm, &e.text, &e.filename, e.line)
}

fn l_rest_statement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = with(vm, &args[0], |l| {
        let pos = l.pos;
        l.pos = l.text.len();
        l.text[pos..].iter().collect::<String>().trim().to_owned()
    })?;
    Ok(vm.types.new_str(&s))
}

fn l_raw_rest(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = with(vm, &args[0], |l| l.raw_rest())?;
    Ok(vm.types.new_str(&s))
}

fn l_python_block(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = with(vm, &args[0], |l| l.python_block())?;
    Ok(vm.types.new_str(&s))
}

fn l_arguments(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = state(vm, &args[0])?;
    let (info, loc) = {
        let mut arena = crate::ast::Arena::default();
        let cx = super::statements::ParseCx::new(vm);
        let mut p = crate::parser::Parser::new(&mut arena, &cx);
        let mut l = st.borrow_mut();
        let r = p.parse_arguments(&mut l);
        (r, l.location())
    };
    match info.map_err(|e| parse_error(vm, &e))? {
        Some(i) => super::parameter::argument_info_value(vm, &i, &loc),
        None => Ok(Value::None),
    }
}

fn subparse(vm: &mut Vm, this: &Value, ids: Vec<crate::ast::NodeId>) -> PyResult<Value> {
    let sp = py::get_or_none(vm, this, "subparses")?;
    let c = classes(vm)?;
    let mut handles = Vec::new();
    for id in ids {
        handles.push(super::nodes::handle(vm, id)?);
    }
    let block = vm.types.new_list(handles);
    let rv = vm.call(&Value::Obj(c.subparse.clone()), vec![block], Vec::new())?;
    vm.call_method(&sp, "append", vec![rv.clone()])?;
    Ok(rv)
}

fn require_subparses(vm: &mut Vm, this: &Value, what: &str) -> PyResult<()> {
    if py::is_none(&py::get_or_none(vm, this, "subparses")?) {
        return Err(py::error(
            vm,
            format!("A {what} can only be parsed inside a creator-defined statement."),
        ));
    }
    Ok(())
}

fn l_renpy_statement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    require_subparses(vm, &args[0], "renpy_statement")?;
    let st = state(vm, &args[0])?;
    let ids = super::statements::parse_with(vm, |p| {
        let mut l = st.borrow_mut();
        let r = p.parse_statement(&mut l);
        l.unadvance();
        r
    })?;
    subparse(vm, &args[0], ids)
}

fn l_renpy_block(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    require_subparses(vm, &args[0], "renpy_block")?;
    let empty = flag(vm, args, kw, 0, "empty", false)?;
    let st = state(vm, &args[0])?;
    let ids = super::statements::parse_with(vm, |p| {
        let mut l = st.borrow_mut();
        if l.line < 0 {
            l.advance();
        }
        let mut block = Vec::new();
        while !l.eob {
            match p.parse_statement(&mut l) {
                Ok(v) => block.extend(v),
                Err(e) => {
                    p.errors.push(e);
                    l.advance();
                }
            }
        }
        if block.is_empty() {
            if empty {
                let loc = l.location();
                block.push(p.arena.add(&loc, crate::ast::NodeKind::Pass));
            } else {
                return l.error("At least one Ren'Py statement is expected.");
            }
        }
        Ok(block)
    })?;
    subparse(vm, &args[0], ids)
}

const LEXER_METHODS: py::Methods = &[
    ("__init__", l_init),
    ("__getattr__", l_getattr),
    ("__setattr__", l_setattr),
    ("advance", l_advance),
    ("unadvance", l_unadvance),
    ("match_regexp", l_match_regexp),
    ("skip_whitespace", l_skip_whitespace),
    ("match", l_match),
    ("match_multiple", l_match_multiple),
    ("keyword", l_keyword),
    ("catch_error", l_catch_error),
    ("error", l_error),
    ("deferred_error", l_deferred_error),
    ("eol", l_eol),
    ("expect_eol", l_expect_eol),
    ("expect_noblock", l_expect_noblock),
    ("expect_block", l_expect_block),
    ("has_block", l_has_block),
    ("subblock_lexer", l_subblock_lexer),
    ("string", l_string),
    ("triple_string", l_triple_string),
    ("integer", l_integer),
    ("float", l_float),
    ("hash", l_hash),
    ("word", l_word),
    ("name", l_name),
    ("set_global_label", l_set_global_label),
    ("label_name", l_label_name),
    ("label_name_declare", l_label_name_declare),
    ("image_name_component", l_image_name_component),
    ("python_string", l_python_string),
    ("dotted_name", l_dotted_name),
    ("expr", l_expr),
    ("delimited_python", l_delimited_python),
    ("python_expression", l_python_expression),
    ("parenthesised_python", l_parenthesised_python),
    ("simple_expression", l_simple_expression),
    ("comma_expression", l_comma_expression),
    ("say_expression", l_say_expression),
    ("checkpoint", l_checkpoint),
    ("revert", l_revert),
    ("get_location", l_get_location),
    ("require", l_require),
    ("rest", l_rest),
    ("rest_statement", l_rest_statement),
    ("raw_rest", l_raw_rest),
    ("python_block", l_python_block),
    ("arguments", l_arguments),
    ("renpy_statement", l_renpy_statement),
    ("renpy_block", l_renpy_block),
];

// ---------------------------------------------------------------------------
// ParseError.

/// Upstream's message: location, the line, a caret, and a hint for an
/// unterminated string.
fn error_message(
    filename: &str,
    number: i64,
    msg: &str,
    line: Option<String>,
    mut pos: Option<i64>,
    first: bool,
) -> String {
    let mut message = format!("File \"{filename}\", line {number}: {msg}");
    let Some(line) = line.filter(|l| !l.is_empty()) else {
        return message;
    };
    let lines: Vec<&str> = line.split('\n').collect();
    if !lines[0].contains("\"\"\"") && !lines[0].contains("'''") && lines.len() > 1 {
        let chars: Vec<char> = lines[0].chars().collect();
        let mut open: Option<char> = None;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                i += 1;
            } else if Some(c) == open {
                open = None;
            } else if open.is_some() {
            } else if c == '`' || c == '\'' || c == '"' {
                open = Some(c);
            }
            i += 1;
        }
        if let Some(o) = open {
            message.push_str(&format!(
                "\n(Perhaps you left out a {o} at the end of the first line.)"
            ));
        }
    }
    for l in lines {
        message.push_str("\n    ");
        message.push_str(l);
        if let Some(p) = pos {
            let n = l.chars().count() as i64;
            if p <= n {
                message.push_str(&format!("\n    {}^", " ".repeat(p.max(0) as usize)));
                pos = None;
            } else {
                pos = Some(p - n);
            }
        }
        if first {
            break;
        }
    }
    message
}

fn pe_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::string(vm, &arg_or(&args[1..], kw, 0, "filename", Value::None))?;
    let number = py::int(vm, &arg_or(&args[1..], kw, 1, "number", Value::Int(0)))?;
    let msg = py::string(vm, &arg_or(&args[1..], kw, 2, "msg", Value::None))?;
    let line = arg_or(&args[1..], kw, 3, "line", Value::None);
    let line = if py::is_none(&line) {
        None
    } else if let Some(items) = py::seq(&line).filter(|_| line.as_str().is_none()) {
        let mut s = String::new();
        for i in items {
            s.push_str(&py::string(vm, &i)?);
        }
        Some(s)
    } else {
        Some(py::string(vm, &line)?)
    };
    let pos = arg_or(&args[1..], kw, 4, "pos", Value::None);
    let pos = if py::is_none(&pos) {
        None
    } else {
        Some(py::int(vm, &pos)?)
    };
    let first = flag(vm, args, kw, 5, "first", false)?;
    let filename = filename.clone();
    let m = error_message(&filename, number, &msg, line, pos, first);
    let mv = vm.types.new_str(&m);
    py::set(vm, this, "message", mv.clone())?;
    py::set(vm, this, "args", vm.types.new_tuple(vec![mv]))?;
    Ok(Value::None)
}

fn pe_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "message")
}

fn pe_defer(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let q = py::import_attr(vm, "renpy.parser", "deferred_parse_errors")?;
    let l = ops::getitem(vm, &q, args[1].clone())?;
    let m = py::get(vm, &args[0], "message")?;
    vm.call_method(&l, "append", vec![m])?;
    Ok(Value::None)
}

fn sp_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "block", args[1].clone())?;
    Ok(Value::None)
}

fn sp_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = py::get(vm, &args[0], "block")?;
    let items = py::items(vm, &b)?;
    let s = match items.first() {
        None => "<SubParse empty>".to_owned(),
        Some(f) => {
            let n = py::get(vm, f, "filename")?;
            let l = py::get(vm, f, "linenumber")?;
            format!("<SubParse {}:{}>", py::string(vm, &n)?, py::int(vm, &l)?)
        }
    };
    Ok(vm.types.new_str(&s))
}

// ---------------------------------------------------------------------------
// Filenames.

fn f_elide_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    // Script names are already relative to the game's base directory;
    // strip a base-directory prefix the way upstream does.
    let base = py::config(vm, "basedir").ok().filter(|b| !py::is_none(b));
    let base = match base {
        Some(b) => format!(
            "{}/",
            py::string(vm, &b)?.replace('\\', "/").trim_end_matches('/')
        ),
        None => String::new(),
    };
    let f = f.replace('\\', "/");
    let rv = if !base.is_empty() && f.starts_with(&base) {
        f[base.len()..].to_owned()
    } else {
        f
    };
    Ok(vm.types.new_str(&rv))
}

fn f_unelide_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    // There is no host path to resolve: game files are read through the
    // runtime's file set by their game-relative names.
    Ok(vm.types.new_str(&f))
}

fn f_unicode_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    Ok(vm.types.new_str(&f))
}

fn f_munge_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    Ok(vm.types.new_str(&crate::lexer::munge_prefix(&f)))
}

/// `get_line_text(filename, lineno)`: the source line from the game's files.
fn f_get_line_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let n = py::int(vm, &args[1])?;
    let Some(r) = super::state::try_rt(vm) else {
        return Ok(Value::None);
    };
    let data = r.files.read(&f).or_else(|| r.files.common_read(&f));
    let Some(data) = data else {
        return Ok(Value::None);
    };
    let text = String::from_utf8_lossy(&data).into_owned();
    Ok(match text.lines().nth((n - 1).max(0) as usize) {
        Some(l) => vm.types.new_str(&format!("{l}\n")),
        None => Value::None,
    })
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("cannot create lexer state objects"))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("elide_filename", f_elide_filename),
            ("unelide_filename", f_unelide_filename),
            ("unicode_filename", f_unicode_filename),
            ("munge_filename", f_munge_filename),
            ("get_line_text", f_get_line_text),
        ],
    );
    let holder = vm.types.new_builtin_type(
        "LexerState",
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Native,
        true,
    );
    let n = vm.types.new_builtin("__new__", no_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&holder, "__new__", n);
    let exc = Value::Obj(vm.types.exc("Exception"));
    let pe = py::class(
        vm,
        MODULE,
        "ParseError",
        &[exc],
        &[
            ("__init__", pe_init),
            ("__str__", pe_str),
            ("__unicode__", pe_str),
            ("defer", pe_defer),
        ],
        &[],
    )?;
    let lexer = py::class(vm, MODULE, "Lexer", &[], LEXER_METHODS, &[])?;
    let sp = py::class(
        vm,
        MODULE,
        "SubParse",
        &[],
        &[("__init__", sp_init), ("__repr__", sp_repr)],
        &[],
    )?;
    let ce = py::class(
        vm,
        MODULE,
        "_CatchError",
        &[],
        &[("__enter__", ce_enter), ("__exit__", ce_exit)],
        &[],
    )?;
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "ParseError", Value::Obj(pe.clone()));
    set(vm, "Lexer", Value::Obj(lexer.clone()));
    set(vm, "SubParse", Value::Obj(sp.clone()));
    set(vm, "_CatchError", Value::Obj(ce));
    vm.ext_set(Rc::new(Classes {
        lexer,
        holder,
        parse_error: pe,
        subparse: sp,
    }));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
