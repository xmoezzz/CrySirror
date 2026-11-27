// SPDX-License-Identifier: MPL-2.0
//! Script loading of `renpy.script.Script`, after Ren'Py 8.3 `script.py`
//! (MIT): file order, `.rpyc`/`.rpy` choice (`crate::script::discover`),
//! parsing with creator-defined statements, naming, `finish_load`
//! (name map, init code, early execution) and modules / strings loaded
//! from init code.
//!
//! Init code entries are `(priority, node)` for `init` blocks (the first
//! node of the block) and `(priority, callable)` for creator-defined
//! statements' `execute_init`, as upstream's `get_init` returns them.
use super::{
    astexec, nodes,
    py::{self, *},
    state::{Files, rt},
    statements::{self, ParseCx},
};
use crate::{
    ast::{NodeId, NodeKind},
    files::GameFiles,
    script::{ScriptFile, Source},
};
use crysirror_python::{PyResult, Value, Vm, builtins::Kw, ops};
use std::sync::Arc;

struct FileSet(Arc<dyn Files>);

impl GameFiles for FileSet {
    fn list(&self) -> Vec<String> {
        self.0.list()
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.read(name)
    }
    fn common_list(&self) -> Vec<String> {
        self.0.common_list()
    }
    fn common_read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.common_read(name)
    }
}

struct Shells(crate::rpyc::Shells);

fn with_shells<R>(vm: &mut Vm, f: impl FnOnce(&mut Vm, &mut crate::rpyc::Shells) -> R) -> PyResult<R> {
    let existing = vm.ext_get::<std::rc::Rc<std::cell::RefCell<Shells>>>().map(|s| s.clone());
    let sh = match existing {
        Some(s) => s,
        None => {
            let s = crate::rpyc::Shells::new(vm).map_err(|e| py::error(vm, format!("rpyc unpickler: {e}")))?;
            let s = std::rc::Rc::new(std::cell::RefCell::new(Shells(s)));
            vm.ext_set(s.clone());
            s
        }
    };
    let mut g = sh.borrow_mut();
    Ok(f(vm, &mut g.0))
}

/// Moves the loader's accumulated errors to `renpy.parser.parse_errors`.
fn flush_errors(vm: &mut Vm) -> PyResult<()> {
    let r = rt(vm);
    let errors: Vec<String> = std::mem::take(&mut r.script.borrow_mut().errors);
    if errors.is_empty() {
        return Ok(());
    }
    let list = py::import_attr(vm, "renpy.parser", "parse_errors")?;
    for e in errors {
        let s = vm.types.new_str(&e);
        vm.call_method(&list, "append", vec![s])?;
    }
    Ok(())
}

/// Loads one file: parse, run creator-defined statement parsers, name,
/// chain, register, early-execute. Returns the init code it adds.
fn load_file(vm: &mut Vm, f: &ScriptFile) -> PyResult<Vec<(i32, NodeId)>> {
    let r = rt(vm);
    let stmts = match &f.source {
        Source::Rpyc(bytes) => {
            let registry = r.registry.borrow().clone();
            with_shells(vm, |vm, sh| r.script.borrow_mut().load_rpyc(vm, sh, &registry, f, bytes))?
        }
        Source::Rpy(text) => {
            let cx = ParseCx::new(vm);
            let parsed = r.script.borrow_mut().parse_rpy(&cx, &f.name, text);
            let errors = cx.finish(vm);
            statements::record_errors(vm, errors)?;
            statements::run_pending(vm)?;
            match parsed {
                Some(v) => r.script.borrow_mut().finish_rpy(&v, &f.name),
                None => Vec::new(),
            }
        }
    };
    flush_errors(vm)?;
    if stmts.is_empty() {
        return Ok(Vec::new());
    }
    let (early, init) = {
        let mut s = r.script.borrow_mut();
        s.chain_file(&stmts, &f.name);
        let before = s.initcode.len();
        let early = s.register(&stmts);
        let init: Vec<_> = s.initcode.drain(before..).collect();
        (early, init)
    };
    for id in early {
        astexec::early_execute(vm, id)?;
    }
    Ok(init)
}

/// `(priority, node_or_callable)` entries as Python values.
fn initcode_value(vm: &mut Vm, init: &[(i32, NodeId)]) -> PyResult<Value> {
    let r = rt(vm);
    let mut out = Vec::new();
    for (prio, id) in init {
        let kind = r.script.borrow().arena.get(*id).kind.clone();
        let v = match kind {
            NodeKind::Init { block, .. } => match block.first() {
                Some(first) => nodes::handle(vm, *first)?,
                None => continue,
            },
            NodeKind::UserStatement { .. } => py::closure(vm, "execute_init", f_execute_init, Value::Int(*id as i64)),
            _ => nodes::handle(vm, *id)?,
        };
        out.push(vm.types.new_tuple(vec![Value::Int(*prio as i64), v]));
    }
    Ok(vm.types.new_list(out))
}

fn f_execute_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = py::int(vm, &args[0])? as NodeId;
    let p = astexec::user_execute_init(vm, id)?;
    if let NodeKind::UserStatement { init_priority, .. } = &mut rt(vm).script.borrow_mut().arena.get_mut(id).kind {
        *init_priority = p.max(*init_priority);
    }
    Ok(Value::None)
}

fn sort_init(init: &mut [(i32, NodeId)]) {
    init.sort_by_key(|(p, _)| *p);
}

/// `Script.load_script()`: every script file in order; returns (and
/// appends to `self.initcode`) the sorted init code.
fn s_load_script(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let r = rt(vm);
    let ren_py = r.profile.at_least(8, 1) || (r.profile.at_least(7, 6) && !r.profile.at_least(8, 0));
    let files = crate::script::discover_release(&FileSet(r.files.clone()), ren_py).map_err(|e| py::error(vm, e))?;
    let loaded = py::get(vm, this, "loaded_scripts")?;
    let mut init = Vec::new();
    for f in &files {
        let key = vm.types.new_str(&f.name);
        if ops::contains(vm, &loaded, &key)? {
            continue;
        }
        ops::set_add(vm, &loaded, key)?;
        init.extend(load_file(vm, f)?);
    }
    sort_init(&mut init);
    let v = initcode_value(vm, &init)?;
    let ic = py::get(vm, this, "initcode")?;
    vm.call_method(&ic, "extend", vec![v.clone()])?;
    let tl = py::get(vm, this, "translator")?;
    vm.call_method(&tl, "load", Vec::new())?;
    Ok(v)
}

/// Finds `name.rpym` / `name.rpymc` in the game, then the common files.
fn module_file(vm: &mut Vm, name: &str) -> PyResult<ScriptFile> {
    let r = rt(vm);
    let modules = crate::script::find_module(&FileSet(r.files.clone()), name).map_err(|e| py::error(vm, e))?;
    modules.ok_or_else(|| py::error(vm, format!("Module {name} could not be loaded.")))
}

fn s_load_module(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = py::string(vm, &args[1])?;
    let f = module_file(vm, &name)?;
    let mut init = load_file(vm, &f)?;
    let report = py::import_attr(vm, "renpy.parser", "report_parse_errors")?;
    let errors = vm.call(&report, Vec::new(), Vec::new())?;
    if py::truthy(vm, &errors)? {
        return Err(vm.exc_msg("SystemExit", "-1"));
    }
    sort_init(&mut init);
    rt(vm).script.borrow_mut().chain_translates();
    initcode_value(vm, &init)
}

/// `Script.include_module(name)`: merges a module's init code after the
/// running init node.
fn s_include_module(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mi = s_load_module(vm, args, &[])?;
    let items = py::items(vm, &mi)?;
    let Some(first) = items.first() else { return Ok(Value::None) };
    let ic = py::get(vm, this, "initcode")?;
    let current = py::import_attr(vm, "renpy.game", "initcode_ast_id")?;
    let current = py::int(vm, &current)?;
    let cur = ops::getitem(vm, &ic, Value::Int(current))?;
    let (a, b) = (ops::getitem(vm, first, Value::Int(0))?, ops::getitem(vm, &cur, Value::Int(0))?);
    if py::lt(vm, &a, &b)? {
        let n = py::string(vm, &args[1])?;
        return Err(py::error(vm, format!("Module {n} contains nodes with priority lower than the node that loads it")));
    }
    let all = py::items(vm, &ic)?;
    let merge = (current + 1) as usize;
    let mut tail: Vec<Value> = all[merge.min(all.len())..].to_vec();
    tail.extend(items);
    let mut keyed = Vec::new();
    for (i, v) in tail.into_iter().enumerate() {
        let p = ops::getitem(vm, &v, Value::Int(0))?;
        keyed.push((py::int(vm, &p)?, i, v));
    }
    keyed.sort_by_key(|(p, i, _)| (*p, *i));
    let mut new = all[..merge.min(all.len())].to_vec();
    new.extend(keyed.into_iter().map(|(_, _, v)| v));
    vm.call_method(&ic, "clear", Vec::new())?;
    let l = vm.types.new_list(new);
    vm.call_method(&ic, "extend", vec![l])?;
    Ok(Value::None)
}

/// `Script.load_string(filename, filedata, linenumber=1)` ->
/// `(stmts, initcode)` or `(None, None)`.
fn s_load_string(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = py::string(vm, &args[1])?;
    let data = py::string(vm, &args[2])?;
    let first = py::int(vm, &arg_or(&args[1..], kw, 2, "linenumber", Value::Int(1)))? as u32;
    let none = vm.types.new_tuple(vec![Value::None, Value::None]);
    let r = rt(vm);
    let f: std::rc::Rc<str> = filename.as_str().into();
    let grouped = crate::lexer::list_logical_lines(&f, &data, first).and_then(|l| crate::lexer::group_logical_lines(&f, l));
    let block = match grouped {
        Ok(b) => b,
        Err(e) => {
            statements::record_errors(vm, vec![e])?;
            return Ok(none);
        }
    };
    let cx = ParseCx::new(vm);
    let last = data.lines().count() as u32 + first.saturating_sub(1);
    let (v, errors) = {
        let mut s = r.script.borrow_mut();
        let mut p = crate::parser::Parser::new(&mut s.arena, &cx);
        let v = p.parse_file(std::rc::Rc::new(block), &filename, last);
        (v, p.errors)
    };
    let mut errors = errors;
    errors.extend(cx.finish(vm));
    statements::record_errors(vm, errors)?;
    statements::run_pending(vm)?;
    let release = py::import_attr(vm, "renpy.parser", "release_deferred_errors")?;
    vm.call(&release, Vec::new(), Vec::new())?;
    let pe = py::import_attr(vm, "renpy.parser", "parse_errors")?;
    if v.is_empty() || py::truthy(vm, &pe)? {
        return Ok(none);
    }
    let stmts = {
        let mut s = r.script.borrow_mut();
        let stmts = s.finish_rpy(&v, &filename);
        s.arena.chain_block(&stmts, None);
        stmts
    };
    let (early, mut init) = {
        let mut s = r.script.borrow_mut();
        let before = s.initcode.len();
        let early = s.register(&stmts);
        let init: Vec<_> = s.initcode.drain(before..).collect();
        (early, init)
    };
    for id in early {
        astexec::early_execute(vm, id)?;
    }
    sort_init(&mut init);
    let mut hs = Vec::new();
    for id in &stmts {
        hs.push(nodes::handle(vm, *id)?);
    }
    let hl = vm.types.new_list(hs);
    let ic = initcode_value(vm, &init)?;
    Ok(vm.types.new_tuple(vec![hl, ic]))
}

fn s_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "initcode", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "loaded_scripts", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "load_languages", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "duplicate_labels", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "serial", Value::Int(0))?;
    let st = py::import_attr(vm, "renpy.translation", "ScriptTranslator")?;
    let tl = vm.call(&st, Vec::new(), Vec::new())?;
    py::set(vm, this, "translator", tl)?;
    Ok(Value::None)
}

/// `report_duplicate_labels()`: labels defined twice are parse errors
/// unless `config.allow_duplicate_labels`.
fn s_report_duplicate_labels(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "developer").unwrap_or(false) || py::config_bool(vm, "allow_duplicate_labels").unwrap_or(false) {
        return Ok(Value::Bool(false));
    }
    let dups = rt(vm).script.borrow().duplicate_labels.clone();
    if dups.is_empty() {
        return Ok(Value::Bool(false));
    }
    let pe = py::import_attr(vm, "renpy.parser", "parse_errors")?;
    for d in dups {
        let s = vm.types.new_str(&d);
        vm.call_method(&pe, "append", vec![s])?;
    }
    let report = py::import_attr(vm, "renpy.parser", "report_parse_errors")?;
    let r = vm.call(&report, Vec::new(), Vec::new())?;
    if py::truthy(vm, &r)? {
        let pe = py::import_attr(vm, "renpy.game", "ParseErrorException")?;
        let e = vm.call(&pe, Vec::new(), Vec::new())?;
        return Err(crysirror_python::PyErr::Exc(e.obj().unwrap().clone()));
    }
    Ok(Value::Bool(false))
}

/// Methods added to `renpy.script.Script`.
pub const SCRIPT_METHODS: py::Methods = &[
    ("__init__", s_init),
    ("load_script", s_load_script),
    ("load_module", s_load_module),
    ("include_module", s_include_module),
    ("load_string", s_load_string),
    ("report_duplicate_labels", s_report_duplicate_labels),
];
