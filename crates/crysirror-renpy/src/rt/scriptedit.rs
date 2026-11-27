// SPDX-License-Identifier: MPL-2.0
//! `renpy.scriptedit`, after Ren'Py 8.3 (MIT): the logical-line table of
//! the game's `.rpy` files and the AST queries the interactive director
//! uses (`get_line_text`, `nodes_on_line`, `can_add_before`, ...).
//!
//! Game files are mounted read-only, so the editing operations
//! (`insert_line_before`, `remove_line`, `add_to_ast_before`,
//! `remove_from_ast`) raise instead of changing the script; the director
//! reports the error where upstream would rewrite the file.
use super::{nodes, py, state};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.scriptedit";

fn norm(vm: &mut Vm, v: &Value) -> PyResult<String> {
    Ok(py::string(vm, v)?.replace('\\', "/"))
}

fn line_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = py::arg_or(args, kw, 1, "filename", Value::None);
    let f = norm(vm, &f)?;
    let f = py::s(vm, &f);
    py::set(vm, this, "filename", f)?;
    py::set(vm, this, "number", py::arg_or(args, kw, 2, "number", Value::Int(0)))?;
    let start = py::arg_or(args, kw, 3, "start", Value::Int(0));
    for n in ["start", "end", "end_delim"] {
        py::set(vm, this, n, start.clone())?;
    }
    for n in ["text", "full_text"] {
        py::set(vm, this, n, py::s(vm, ""))?;
    }
    Ok(Value::None)
}

fn line_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "filename")?;
    let n = py::get(vm, &args[0], "number")?;
    let t = py::get(vm, &args[0], "text")?;
    let s = format!("<Line {}:{} {}>", py::string(vm, &f)?, ops::str(vm, &n)?, ops::repr(vm, &t)?);
    Ok(py::s(vm, &s))
}

/// Character offsets of the physical line starts of `data`.
fn line_starts(chars: &[char]) -> Vec<usize> {
    let mut v = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            v.push(i + 1);
        }
    }
    v
}

fn blank_or_comment(chars: &[char], from: usize, to: usize) -> bool {
    let t: String = chars[from..to].iter().collect();
    let t = t.trim();
    t.is_empty() || t.starts_with('#')
}

/// `ensure_loaded(filename)`: fills `lines` with the logical lines of a
/// `.rpy` file (`list_logical_lines(fn, add_lines=True)`).
fn ensure_loaded(vm: &mut Vm, filename: &str) -> PyResult<()> {
    if !(filename.ends_with(".rpy") || filename.ends_with(".rpyc")) {
        return Ok(());
    }
    let files = py::import_attr(vm, MODULE, "files")?;
    let key = py::s(vm, filename);
    if ops::contains(vm, &files, &key)? {
        return Ok(());
    }
    ops::set_add(vm, &files, key)?;
    let src = filename.strip_suffix('c').unwrap_or(filename);
    let rel = src.strip_prefix("game/").unwrap_or(src);
    let Some(data) = state::rt(vm).files.read(rel) else { return Ok(()) };
    let text = String::from_utf8_lossy(&data).into_owned();
    let fname: Rc<str> = filename.into();
    let Ok(logical) = crate::lexer::list_logical_lines(&fname, &text, 1) else { return Ok(()) };
    let chars: Vec<char> = text.chars().collect();
    let starts = line_starts(&chars);
    let phys_end = |n: usize| -> (usize, usize) {
        // (end without the line ending, end including it) of physical line n (1-based).
        let s = starts.get(n - 1).copied().unwrap_or(chars.len());
        let delim = starts.get(n).copied().unwrap_or(chars.len());
        let mut e = delim;
        while e > s && matches!(chars[e - 1], '\n' | '\r') {
            e -= 1;
        }
        (e, delim)
    };
    let lines = py::import_attr(vm, MODULE, "lines")?;
    let cls = py::import_attr(vm, MODULE, "Line")?;
    for (i, (number, _)) in logical.iter().enumerate() {
        let n = *number as usize;
        let start = starts.get(n - 1).copied().unwrap_or(chars.len());
        let next = logical.get(i + 1).map_or(starts.len(), |(m, _)| *m as usize - 1);
        let mut last = next.max(n);
        while last > n {
            let s = starts.get(last - 1).copied().unwrap_or(chars.len());
            if blank_or_comment(&chars, s, phys_end(last).0) {
                last -= 1;
            } else {
                break;
            }
        }
        let (end, end_delim) = phys_end(last);
        let fv = py::s(vm, filename);
        let l = vm.call(&cls, vec![fv.clone(), Value::Int(n as i64), Value::Int(start as i64)], Vec::new())?;
        py::set(vm, &l, "end", Value::Int(end as i64))?;
        py::set(vm, &l, "end_delim", Value::Int(end_delim as i64))?;
        let t: String = chars[start..end].iter().collect();
        let ft: String = chars[start..end_delim].iter().collect();
        py::set(vm, &l, "text", py::s(vm, &t))?;
        py::set(vm, &l, "full_text", py::s(vm, &ft))?;
        let k = vm.types.new_tuple(vec![fv, Value::Int(n as i64)]);
        ops::setitem(vm, &lines, k, l)?;
    }
    Ok(())
}

fn f_ensure_loaded(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    ensure_loaded(vm, &f)?;
    Ok(Value::None)
}

fn line_attr(vm: &mut Vm, args: &[Value], attr: &str) -> PyResult<Value> {
    let f = norm(vm, &args[0])?;
    ensure_loaded(vm, &f)?;
    let lines = py::import_attr(vm, MODULE, "lines")?;
    let fv = py::s(vm, &f);
    let k = vm.types.new_tuple(vec![fv, args[1].clone()]);
    if !ops::contains(vm, &lines, &k)? {
        return Ok(Value::None);
    }
    let l = ops::getitem(vm, &lines, k)?;
    py::get(vm, &l, attr)
}

fn f_get_line_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    line_attr(vm, args, "text")
}

fn f_get_full_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    line_attr(vm, args, "full_text")
}

/// `adjust_line_locations(filename, linenumber, char_offset, line_offset)`.
fn f_adjust_line_locations(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = norm(vm, &args[0])?;
    ensure_loaded(vm, &f)?;
    let from = py::int(vm, &args[1])?;
    let co = py::int(vm, &args[2])?;
    let lo = py::int(vm, &args[3])?;
    let m = py::import(vm, MODULE)?;
    let lines = py::get(vm, &m, "lines")?;
    let new = py::dict(vm);
    for (k, line) in ops::mapping_items(vm, &lines)? {
        let p = py::items(vm, &k)?;
        let fname = py::string(vm, &p[0])?;
        let mut ln = py::int(vm, &p[1])?;
        if fname == f && from <= ln {
            ln += lo;
            for (a, d) in [("number", lo), ("start", co), ("end", co), ("end_delim", co)] {
                let v = py::get(vm, &line, a)?;
                let v = py::int(vm, &v)?;
                py::set(vm, &line, a, Value::Int(v + d))?;
            }
        }
        let k = vm.types.new_tuple(vec![p[0].clone(), Value::Int(ln)]);
        ops::setitem(vm, &new, k, line)?;
    }
    py::set(vm, &m, "lines", new)?;
    Ok(Value::None)
}

/// Nodes of the script on `(filename, linenumber)` that can roll back.
fn nodes_on(vm: &mut Vm, filename: &str, line: Option<u32>) -> PyResult<Vec<crate::ast::NodeId>> {
    let r = state::rt(vm);
    let s = r.script.borrow();
    let mut v = Vec::new();
    for i in 0..s.arena.len() {
        let id = i;
        let n = s.arena.get(id);
        if &*n.filename == filename && line.is_none_or(|l| n.linenumber == l) && n.rollback != crate::ast::Rollback::Never {
            v.push(id);
        }
    }
    Ok(v)
}

fn handles(vm: &mut Vm, ids: Vec<crate::ast::NodeId>) -> PyResult<Value> {
    let mut v = Vec::with_capacity(ids.len());
    for id in ids {
        v.push(nodes::handle(vm, id)?);
    }
    Ok(vm.types.new_list(v))
}

fn f_nodes_on_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    ensure_loaded(vm, &f)?;
    let l = py::int(vm, &args[1])? as u32;
    let ids = nodes_on(vm, &f, Some(l))?;
    handles(vm, ids)
}

fn f_nodes_on_line_at_or_after(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    ensure_loaded(vm, &f)?;
    let l = py::int(vm, &args[1])? as u32;
    let all = nodes_on(vm, &f, None)?;
    let r = state::rt(vm);
    let first = all.iter().map(|id| r.script.borrow().arena.get(*id).linenumber).filter(|n| *n >= l).min();
    match first {
        Some(m) => {
            let ids = nodes_on(vm, &f, Some(m))?;
            handles(vm, ids)
        }
        None => Ok(vm.types.new_list(Vec::new())),
    }
}

fn first_and_last(vm: &mut Vm, list: &Value) -> PyResult<(Value, Value)> {
    let items = py::items(vm, list)?;
    let mut firsts = Vec::new();
    let mut lasts = Vec::new();
    for i in &items {
        let mut is_first = true;
        let mut is_last = true;
        let inext = py::get_or_none(vm, i, "next")?;
        for j in &items {
            let jnext = py::get_or_none(vm, j, "next")?;
            if py::same(&jnext, i) {
                is_first = false;
            }
            if py::same(&inext, j) {
                is_last = false;
            }
        }
        if is_first {
            firsts.push(i.clone());
        }
        if is_last {
            lasts.push(i.clone());
        }
    }
    if firsts.len() != 1 {
        return Err(py::error(vm, "Could not find unique first AST node."));
    }
    if lasts.len() != 1 {
        return Err(py::error(vm, "Could not find unique last AST node."));
    }
    Ok((firsts.remove(0), lasts.remove(0)))
}

fn f_first_and_last_nodes(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, b) = first_and_last(vm, &args[0])?;
    Ok(vm.types.new_tuple(vec![a, b]))
}

fn f_can_add_before(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ok = (|| -> PyResult<()> {
        let l = f_nodes_on_line(vm, args, &[])?;
        first_and_last(vm, &l)?;
        Ok(())
    })();
    Ok(Value::Bool(ok.is_ok()))
}

fn read_only(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = args.iter().find_map(|a| a.as_str().filter(|s| s.contains(".rpy")).map(str::to_owned)).unwrap_or_else(|| "the script".into());
    Err(py::error(vm, format!("Script files are read-only in this runtime; {f} cannot be edited.")))
}

/// `adjust_ast_linenumbers(filename, linenumber, offset)`.
fn f_adjust_ast_linenumbers(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let from = py::int(vm, &args[1])?;
    let off = py::int(vm, &args[2])?;
    let r = state::rt(vm);
    let mut s = r.script.borrow_mut();
    for i in 0..s.arena.len() {
        let n = s.arena.get_mut(i);
        if *n.filename == *f && n.linenumber as i64 >= from {
            n.linenumber = (n.linenumber as i64 + off).max(0) as u32;
        }
    }
    Ok(Value::None)
}

fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("ensure_loaded", f_ensure_loaded),
        ("get_line_text", f_get_line_text),
        ("get_full_text", f_get_full_text),
        ("adjust_line_locations", f_adjust_line_locations),
        ("insert_line_before", read_only),
        ("remove_line", read_only),
        ("nodes_on_line", f_nodes_on_line),
        ("nodes_on_line_at_or_after", f_nodes_on_line_at_or_after),
        ("first_and_last_nodes", f_first_and_last_nodes),
        ("adjust_ast_linenumbers", f_adjust_ast_linenumbers),
        ("add_to_ast_before", read_only),
        ("can_add_before", f_can_add_before),
        ("remove_from_ast", read_only),
    ]);
    let line = py::class(vm, MODULE, "Line", &[], &[("__init__", line_init), ("__repr__", line_repr)], &[])?;
    crysirror_python::modules::set(vm, &m, "Line", Value::Obj(line));
    crysirror_python::modules::set(vm, &m, "lines", vm.types.new_dict());
    crysirror_python::modules::set(vm, &m, "files", vm.types.new_set(Default::default()));
    crysirror_python::modules::set(vm, &m, "serial", Value::Int(1));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
