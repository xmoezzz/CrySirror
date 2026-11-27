// SPDX-License-Identifier: MPL-2.0
//! `renpy.parser`, after Ren'Py 8.3 `parser.py` (MIT): the parse-error
//! queues and the entry points Python code uses (`parse_block`,
//! `parse_statement`, `parse_arguments`, `parse`). Statements are parsed
//! by the Rust parser (`crate::parser`) over the script arena.
use super::{
    lexer as rlexer, nodes,
    py::{self, *},
    statements::{self, ParseCx},
};
use crate::ast::NodeId;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};
use std::rc::Rc;

const MODULE: &str = "renpy.parser";

fn handles(vm: &mut Vm, ids: Vec<NodeId>) -> PyResult<Value> {
    let mut v = Vec::new();
    for id in ids {
        v.push(nodes::handle(vm, id)?);
    }
    Ok(vm.types.new_list(v))
}

fn f_parse_statement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = rlexer::state(vm, &args[0])?;
    let ids = statements::parse_with(vm, |p| p.parse_statement(&mut st.borrow_mut()))?;
    let mut h = Vec::new();
    for id in &ids {
        h.push(nodes::handle(vm, *id)?);
    }
    Ok(if h.len() == 1 { h.pop().unwrap() } else { vm.types.new_list(h) })
}

fn f_parse_block(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = rlexer::state(vm, &args[0])?;
    let ids = statements::parse_with(vm, |p| p.parse_block(&mut st.borrow_mut()))?;
    handles(vm, ids)
}

fn f_parse_arguments(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = rlexer::state(vm, &args[0])?;
    let (info, loc) = {
        let mut arena = crate::ast::Arena::default();
        let cx = ParseCx::new(vm);
        let mut p = crate::parser::Parser::new(&mut arena, &cx);
        let mut l = st.borrow_mut();
        (p.parse_arguments(&mut l), l.location())
    };
    match info.map_err(|e| rlexer::parse_error(vm, &e))? {
        Some(i) => super::parameter::argument_info_value(vm, &i, &loc),
        None => Ok(Value::None),
    }
}

/// `parse(fn, filedata=None, linenumber=1)`: parses source text into
/// statements (with the trailing `return`), or None after errors.
fn f_parse(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[0])?;
    let data = arg_or(&args[1..], kw, 0, "filedata", Value::None);
    let first = py::int(vm, &arg_or(&args[1..], kw, 1, "linenumber", Value::Int(1)))? as u32;
    let text = if py::is_none(&data) {
        let r = super::state::rt(vm);
        let bytes = r.files.read(&name).or_else(|| r.files.common_read(&name));
        match bytes {
            Some(b) => String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_owned(),
            None => return Err(vm.exc_msg("IOError", format!("Could not find '{name}'."))),
        }
    } else {
        py::string(vm, &data)?
    };
    let info = vm.types.new_str(&format!("While parsing {name}."));
    let game = py::import(vm, "renpy.game")?;
    py::set(vm, &game, "exception_info", info)?;
    let f: Rc<str> = name.as_str().into();
    let grouped = crate::lexer::list_logical_lines(&f, &text, first).and_then(|l| crate::lexer::group_logical_lines(&f, l));
    let block = match grouped {
        Ok(b) => b,
        Err(e) => {
            statements::record_errors(vm, vec![e])?;
            return Ok(Value::None);
        }
    };
    let last = text.lines().count() as u32 + first.saturating_sub(1);
    let ids = statements::parse_with(vm, |p| Ok(p.parse_file(Rc::new(block), &name, last)))?;
    let errors = py::import_attr(vm, MODULE, "parse_errors")?;
    if py::truthy(vm, &errors)? {
        return Ok(Value::None);
    }
    handles(vm, ids)
}

fn release_deferred_errors(vm: &mut Vm) -> PyResult<()> {
    let q = py::import_attr(vm, MODULE, "deferred_parse_errors")?;
    let errors = py::import_attr(vm, MODULE, "parse_errors")?;
    let c = |vm: &mut Vm, n: &str| py::config_bool(vm, n).unwrap_or(false);
    let rules = [
        ("check_conflicting_properties", c(vm, "check_conflicting_properties")),
        ("check_translate_none", c(vm, "early_developer") && c(vm, "check_translate_none")),
        ("duplicate_id", c(vm, "early_developer")),
        ("atl_pos_only", !c(vm, "atl_pos_only")),
    ];
    for (queue, release) in rules {
        let k = vm.types.new_str(queue);
        let popped = vm.call_method(&q, "pop", vec![k, vm.types.new_tuple(Vec::new())])?;
        if release {
            vm.call_method(&errors, "extend", vec![popped])?;
        }
    }
    if py::truthy(vm, &q)? {
        let keys = py::items(vm, &q)?;
        let mut names = Vec::new();
        for k in keys {
            names.push(crysirror_python::ops::repr(vm, &k)?);
        }
        return Err(py::error(vm, format!("Unknown deferred error label(s) : ({},)", names.join(", "))));
    }
    Ok(())
}

fn f_release_deferred_errors(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    release_deferred_errors(vm)?;
    Ok(Value::None)
}

fn f_get_parse_errors(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    release_deferred_errors(vm)?;
    let m = py::import(vm, MODULE)?;
    let rv = py::get(vm, &m, "parse_errors")?;
    py::set(vm, &m, "parse_errors", vm.types.new_list(Vec::new()))?;
    Ok(rv)
}

/// `report_parse_errors()`: True when the script had errors, which go to
/// the error screen (`renpy.display.error`).
fn f_report_parse_errors(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    release_deferred_errors(vm)?;
    let errors = py::import_attr(vm, MODULE, "parse_errors")?;
    if !py::truthy(vm, &errors)? {
        return Ok(Value::Bool(false));
    }
    let mut full = String::new();
    for e in py::items(vm, &errors)? {
        full.push_str(&py::string(vm, &e)?);
        full.push_str("\n\n");
    }
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "debug_sound", Value::Bool(false))?;
    let v = py::import_attr(vm, "renpy", "version")?;
    let v = py::string(vm, &v)?;
    let report = format!("I'm sorry, but errors were detected in your script. Please correct the\nerrors listed below, and try again.\n\n\n{}\nRen'Py Version: {v}\n", full.trim_end());
    super::log::push(vm, "errors", &report);
    super::link::diagnostic(vm, format!("renpy parse errors: {}", full.trim_end()));
    let f = py::import_attr(vm, "renpy.display.error", "report_parse_errors")?;
    {
        let t = vm.types.new_str(&full);
        let n = py::s(vm, "errors.txt");
        vm.call(&f, vec![t, n], Vec::new())?;
    }
    Ok(Value::Bool(true))
}

fn trie_add(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "renpy.parser.statements.add is not available in this runtime; use renpy.register_statement."))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("parse_statement", f_parse_statement),
            ("parse_block", f_parse_block),
            ("parse_arguments", f_parse_arguments),
            ("parse", f_parse),
            ("release_deferred_errors", f_release_deferred_errors),
            ("get_parse_errors", f_get_parse_errors),
            ("report_parse_errors", f_report_parse_errors),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "parse_errors", vm.types.new_list(Vec::new()));
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let list = Value::Obj(vm.types.list.clone());
    let q = vm.call(&dd, vec![list], Vec::new())?;
    set(vm, "deferred_parse_errors", q);
    for n in ["Lexer", "ParseError", "SubParse"] {
        let v = py::import_attr(vm, "renpy.lexer", n)?;
        set(vm, n, v);
    }
    let trie = py::class(vm, MODULE, "ParseTrie", &[], &[("add", trie_add)], &[])?;
    let t = vm.call(&Value::Obj(trie.clone()), Vec::new(), Vec::new())?;
    set(vm, "ParseTrie", Value::Obj(trie));
    set(vm, "statements", t);
    set(vm, "translate_none_files", vm.types.new_set(Default::default()));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
