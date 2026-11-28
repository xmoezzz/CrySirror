// SPDX-License-Identifier: MPL-2.0
//! `renpy.statements` (creator-defined statements), after Ren'Py 8.3
//! `statements.py` (MIT), and the parse context the Rust script parser
//! uses inside the runtime.
//!
//! Upstream calls a statement's Python `parse` function from inside the
//! script parser. Here the Rust parser cannot call into Python while it
//! holds the script arena, so parsing a creator-defined statement is
//! split: the parser handles what upstream decides before `parse` runs
//! (block expectations, `script`/`atl` blocks, `Init` wrapping, the
//! `PostUserStatement`) and queues the statement with its lexer state;
//! `run_pending` then calls `parse` with a `renpy.lexer.Lexer` positioned
//! after the statement name, collects its subparses and label, before
//! the file's nodes are named. A statement whose `parse` consumes lines
//! past its own logical line is not supported.
use super::{
    lexer as rlexer, nodes,
    py::{self, *},
    state::rt,
};
use crate::{
    ast::{Arena, NodeId, NodeKind, NodeName},
    lexer::{GroupedLine, Lexer, PResult, ParseError},
    parser::{Parser, ScriptContext},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

const MODULE: &str = "renpy.statements";

#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    No,
    Yes,
    Possible,
    Script,
    ScriptPossible,
    Atl,
    AtlPossible,
}

/// What the parser needs to know about a registered statement.
#[derive(Clone)]
pub struct UserDef {
    block: BlockKind,
    init: bool,
    init_priority: i32,
    post: bool,
    translatable: bool,
}

type Defs = Rc<RefCell<HashMap<Vec<String>, UserDef>>>;

fn defs(vm: &mut Vm) -> Defs {
    if let Some(d) = vm.ext_get::<Defs>() {
        return d.clone();
    }
    let d: Defs = Rc::default();
    vm.ext_set(d.clone());
    d
}

/// A creator-defined statement waiting for its Python `parse`.
struct Pending {
    id: NodeId,
    name: Vec<String>,
    lexer: Lexer,
}

type Queue = Rc<RefCell<VecDeque<Pending>>>;

fn queue(vm: &mut Vm) -> Queue {
    if let Some(q) = vm.ext_get::<Queue>() {
        return q.clone();
    }
    let q: Queue = Rc::default();
    vm.ext_set(q.clone());
    q
}

// ---------------------------------------------------------------------------
// Parse context.

/// The runtime's script-parse context: creator-defined statements,
/// screen-language statements and ATL warpers as currently registered.
pub struct ParseCx {
    defs: HashMap<Vec<String>, UserDef>,
    names: Vec<Vec<String>>,
    registry: crate::sl2::Registry,
    warpers: HashSet<String>,
    pending: RefCell<Vec<Pending>>,
    errors: RefCell<Vec<ParseError>>,
    release: (u32, u32),
}

impl ParseCx {
    pub fn new(vm: &mut Vm) -> ParseCx {
        let defs = defs(vm).borrow().clone();
        let names = defs.keys().cloned().collect();
        let registry = super::state::try_rt(vm)
            .map(|r| r.registry.borrow().clone())
            .unwrap_or_default();
        let mut warpers: HashSet<String> = crate::atl::BUILTIN_WARPERS
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Ok(w) = py::import_attr(vm, "renpy.atl", "warpers") {
            if let Ok(keys) = py::items(vm, &w) {
                for k in keys {
                    if let Some(s) = k.as_str() {
                        warpers.insert(s.to_owned());
                    }
                }
            }
        }
        let release = super::state::try_rt(vm)
            .map_or((8, 6), |r| (r.profile.version[0], r.profile.version[1]));
        ParseCx {
            defs,
            names,
            registry,
            warpers,
            pending: RefCell::default(),
            errors: RefCell::default(),
            release,
        }
    }

    fn sub_block(&self, arena: &mut Arena, l: &Lexer) -> PResult<Vec<NodeId>> {
        let mut sl = l.subblock_lexer(false);
        let mut p = Parser::new(arena, self);
        let v = p.parse_block(&mut sl)?;
        self.errors.borrow_mut().extend(p.errors);
        Ok(v)
    }

    fn sub_atl(&self, l: &Lexer) -> PResult<Rc<crate::atl::RawBlock>> {
        let mut sl = l.subblock_lexer(false);
        Ok(Rc::new(crate::atl::parse_atl(&mut sl, self)?))
    }

    /// Moves queued statements and nested-parse errors to the runtime.
    pub fn finish(self, vm: &mut Vm) -> Vec<ParseError> {
        let q = queue(vm);
        q.borrow_mut().extend(self.pending.into_inner());
        self.errors.into_inner()
    }
}

impl crate::atl::ParseContext for ParseCx {
    fn is_warper(&self, name: &str) -> bool {
        self.warpers.contains(name)
    }
}

impl ScriptContext for ParseCx {
    fn release(&self) -> (u32, u32) {
        self.release
    }

    fn user_statement_names(&self) -> Vec<Vec<String>> {
        self.names.clone()
    }

    fn parse_user_statement(
        &self,
        arena: &mut Arena,
        name: &[String],
        l: &mut Lexer,
        loc: &(Rc<str>, u32),
    ) -> PResult<Vec<NodeId>> {
        let Some(def) = self.defs.get(name) else {
            return l.error(format!("unregistered statement {}", name.join(" ")));
        };
        let stmt = format!("{} statement", name.join(" "));
        let mut code_block = None;
        let mut atl = None;
        match def.block {
            BlockKind::No => l.expect_noblock(&stmt)?,
            BlockKind::Yes => l.expect_block(&stmt)?,
            BlockKind::Possible => {}
            BlockKind::Script => {
                l.expect_block(&stmt)?;
                code_block = Some(self.sub_block(arena, l)?);
            }
            BlockKind::ScriptPossible if l.has_block() => {
                code_block = Some(self.sub_block(arena, l)?)
            }
            BlockKind::Atl => {
                l.expect_block(&stmt)?;
                atl = Some(self.sub_atl(l)?);
            }
            BlockKind::AtlPossible if l.has_block() => atl = Some(self.sub_atl(l)?),
            _ => {}
        }
        let init_priority = def.init_priority + l.init_offset;
        let id = arena.add(
            loc,
            NodeKind::UserStatement {
                line: l.current_text(),
                block: l.subblock.clone(),
                parsed: None,
                subparses: Vec::new(),
                code_block,
                init_priority,
                translatable: def.translatable,
                atl,
            },
        );
        self.pending.borrow_mut().push(Pending {
            id,
            name: name.to_vec(),
            lexer: l.clone(),
        });
        l.advance();
        let mut rv = vec![id];
        if def.post {
            rv.push(arena.add(loc, NodeKind::PostUserStatement { parent: id }));
        }
        if def.init && !l.init {
            rv = vec![arena.add(
                loc,
                NodeKind::Init {
                    block: rv,
                    priority: init_priority,
                },
            )];
        }
        Ok(rv)
    }

    fn sl2_registry(&self) -> &crate::sl2::Registry {
        &self.registry
    }
}

/// Parse errors as upstream records them in `renpy.parser.parse_errors`.
pub fn record_errors(vm: &mut Vm, errors: Vec<ParseError>) -> PyResult<()> {
    if errors.is_empty() {
        return Ok(());
    }
    let list = py::import_attr(vm, "renpy.parser", "parse_errors")?;
    for e in errors {
        let err = rlexer::parse_error(vm, &e);
        let msg = match &err {
            crysirror_python::PyErr::Exc(o) => py::get(vm, &Value::Obj(o.clone()), "message")?,
            _ => vm.types.new_str(&e.to_string()),
        };
        vm.call_method(&list, "append", vec![msg])?;
    }
    Ok(())
}

/// Runs the Rust parser over the script arena from Python (subparses of
/// creator-defined statements, `renpy.parser.parse_block`). Statements
/// queued meanwhile are parsed before returning.
pub fn parse_with(
    vm: &mut Vm,
    f: impl FnOnce(&mut Parser) -> PResult<Vec<NodeId>>,
) -> PyResult<Vec<NodeId>> {
    let r = rt(vm);
    let cx = ParseCx::new(vm);
    let (result, errors) = {
        let mut script = r.script.borrow_mut();
        let mut p = Parser::new(&mut script.arena, &cx);
        let v = f(&mut p);
        (v, p.errors)
    };
    let mut errors = errors;
    errors.extend(cx.finish(vm));
    record_errors(vm, errors)?;
    let ids = result.map_err(|e| rlexer::parse_error(vm, &e))?;
    run_pending(vm)?;
    Ok(ids)
}

fn name_tuple(vm: &Vm, name: &[String]) -> Value {
    let v = name.iter().map(|s| vm.types.new_str(s)).collect();
    vm.types.new_tuple(v)
}

fn registry(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "registry")
}

fn entry(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let reg = registry(vm)?;
    ops::getitem(vm, &reg, name.clone())
}

fn entry_get(vm: &mut Vm, e: &Value, key: &str) -> PyResult<Value> {
    let k = vm.types.new_str(key);
    vm.call_method(e, "get", vec![k, Value::None])
}

fn is_parse_error(vm: &mut Vm, e: &crysirror_python::PyErr) -> bool {
    let crysirror_python::PyErr::Exc(o) = e else {
        return false;
    };
    match py::import_attr(vm, "renpy.lexer", "ParseError") {
        Ok(Value::Obj(c)) => ops::isinstance(vm, &Value::Obj(o.clone()), &c),
        _ => false,
    }
}

/// Calls the queued statements' `parse` functions (see the module docs).
pub fn run_pending(vm: &mut Vm) -> PyResult<()> {
    let q = queue(vm);
    loop {
        let Some(p) = q.borrow_mut().pop_front() else {
            return Ok(());
        };
        let name = name_tuple(vm, &p.name);
        let e = entry(vm, &name)?;
        let parse = entry_get(vm, &e, "parse")?;
        let subparses = vm.types.new_list(Vec::new());
        let inner = if py::is_none(&parse) {
            Value::None
        } else {
            let l = rlexer::new_lexer(vm, p.lexer, subparses.clone())?;
            match vm.call(&parse, vec![l], Vec::new()) {
                Ok(v) => v,
                Err(err) if is_parse_error(vm, &err) => {
                    let crysirror_python::PyErr::Exc(o) = &err else {
                        unreachable!()
                    };
                    let msg = py::get(vm, &Value::Obj(o.clone()), "message")?;
                    let list = py::import_attr(vm, "renpy.parser", "parse_errors")?;
                    vm.call_method(&list, "append", vec![msg])?;
                    Value::None
                }
                Err(err) => return Err(err),
            }
        };
        let parsed = vm.types.new_tuple(vec![name, inner]);
        let mut blocks = Vec::new();
        for sp in py::items(vm, &subparses)? {
            let b = py::get(vm, &sp, "block")?;
            let mut ids = Vec::new();
            for h in py::items(vm, &b)? {
                if let Some(id) = nodes::id_of(&h) {
                    ids.push(id);
                }
            }
            blocks.push(ids);
        }
        let label = call_method(vm, "label", &parsed, Vec::new(), &[])?;
        let label = if py::is_none(&label) {
            None
        } else {
            Some(py::string(vm, &label)?)
        };
        let r = rt(vm);
        let mut script = r.script.borrow_mut();
        let node = script.arena.get_mut(p.id);
        if let NodeKind::UserStatement {
            parsed: slot,
            subparses: sp,
            ..
        } = &mut node.kind
        {
            *slot = Some(parsed);
            *sp = blocks;
        }
        if let Some(l) = label {
            node.name = Some(NodeName::Label(l.into()));
        }
    }
}

// ---------------------------------------------------------------------------
// Module functions.

const REGISTER_ARGS: &[&str] = &[
    "name",
    "parse",
    "lint",
    "execute",
    "predict",
    "next",
    "scry",
    "block",
    "init",
    "translatable",
    "execute_init",
    "init_priority",
    "label",
    "warp",
    "translation_strings",
    "force_begin_rollback",
    "post_execute",
    "post_label",
    "predict_all",
    "predict_next",
    "execute_default",
    "reachable",
];

fn f_register(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut vals: HashMap<&str, Value> = HashMap::new();
    for (i, a) in args.iter().enumerate() {
        let Some(n) = REGISTER_ARGS.get(i) else {
            return Err(vm.type_error("register() takes at most 22 positional arguments"));
        };
        vals.insert(n, a.clone());
    }
    for (k, v) in kw {
        let Some(n) = REGISTER_ARGS.iter().find(|n| **n == &**k) else {
            return Err(vm.type_error(format!(
                "register() got an unexpected keyword argument '{k}'"
            )));
        };
        vals.insert(n, v.clone());
    }
    let get = |n: &str| vals.get(n).cloned().unwrap_or(Value::None);
    let flag = |vm: &mut Vm, n: &str, d: bool| -> PyResult<bool> {
        match vals.get(n) {
            Some(v) => py::truthy(vm, v),
            None => Ok(d),
        }
    };
    let name_s = py::string(vm, &get("name"))?;
    let parts: Vec<String> = name_s.split_whitespace().map(str::to_owned).collect();
    let name = name_tuple(vm, &parts);
    let mut force = flag(vm, "force_begin_rollback", false)?;
    if py::truthy(vm, &get("label"))? {
        force = true;
    }
    let d = py::dict(vm);
    for n in [
        "parse",
        "lint",
        "execute",
        "execute_init",
        "predict",
        "next",
        "scry",
        "label",
        "warp",
        "translation_strings",
        "post_execute",
        "post_label",
        "predict_next",
        "execute_default",
        "reachable",
    ] {
        py::dict_set(vm, &d, n, get(n))?;
    }
    let rb = py::s(vm, if force { "force" } else { "normal" });
    py::dict_set(vm, &d, "rollback", rb)?;
    let pa = flag(vm, "predict_all", true)?;
    py::dict_set(vm, &d, "predict_all", Value::Bool(pa))?;
    let reg = registry(vm)?;
    ops::setitem(vm, &reg, name, d)?;
    let block = match vals.get("block") {
        None => BlockKind::No,
        Some(Value::Bool(false)) => BlockKind::No,
        Some(Value::Bool(true)) => BlockKind::Yes,
        Some(v) => match v.as_str() {
            Some("script") => BlockKind::Script,
            Some("script-possible") => BlockKind::ScriptPossible,
            Some("atl") => BlockKind::Atl,
            Some("atl-possible") => BlockKind::AtlPossible,
            Some("possible") => BlockKind::Possible,
            _ => {
                let r = ops::repr(vm, v)?;
                return Err(py::error(
                    vm,
                    format!("Unknown \"block\" argument value: {r}"),
                ));
            }
        },
    };
    let init_priority = match vals.get("init_priority") {
        Some(v) => py::int(vm, v)? as i32,
        None => 0,
    };
    let post = !py::is_none(&get("post_execute")) || !py::is_none(&get("post_label"));
    let def = UserDef {
        block,
        init: flag(vm, "init", false)?,
        init_priority,
        post,
        translatable: flag(vm, "translatable", false)?,
    };
    defs(vm).borrow_mut().insert(parts, def);
    Ok(Value::None)
}

/// Longest registered statement name at the lexer position.
fn match_name(names: &[Vec<String>], l: &mut Lexer) -> Option<Vec<String>> {
    let start = l.checkpoint();
    let mut best: Option<(Vec<String>, crate::lexer::Checkpoint)> = None;
    for n in names {
        l.revert(&start);
        let ok = n.iter().all(|w| {
            let old = l.pos;
            let got = l.word().or_else(|| l.match_re(r"\$"));
            if got.as_deref() == Some(w.as_str()) {
                true
            } else {
                l.pos = old;
                false
            }
        });
        if ok
            && best
                .as_ref()
                .map(|(b, _)| n.len() > b.len())
                .unwrap_or(true)
        {
            best = Some((n.clone(), l.checkpoint()));
        }
    }
    match best {
        Some((n, cp)) => {
            l.revert(&cp);
            Some(n)
        }
        None => {
            l.revert(&start);
            None
        }
    }
}

/// `parse(node, line, subblock)`: the runtime parse of a statement loaded
/// without its parsed data.
pub fn parse(vm: &mut Vm, node: &Value, line: &str, subblock: &Value) -> PyResult<Value> {
    let f = py::get(vm, node, "filename")?;
    let filename: Rc<str> = py::string(vm, &f)?.into();
    let n = py::get(vm, node, "linenumber")?;
    let number = py::int(vm, &n)? as u32;
    let block = if py::is_none(subblock) {
        Vec::new()
    } else {
        rlexer::block_from(vm, subblock)?
    };
    let g = GroupedLine {
        filename: filename.clone(),
        number,
        indent: 0,
        text: line.chars().collect::<Vec<_>>().into(),
        block: Rc::new(block),
    };
    let mut l = Lexer::new(Rc::new(vec![g]), false);
    l.advance();
    let names: Vec<Vec<String>> = defs(vm).borrow().keys().cloned().collect();
    let Some(name) = match_name(&names, &mut l) else {
        let e = l
            .error::<()>("Could not find user-defined statement at runtime.")
            .unwrap_err();
        return Err(rlexer::parse_error(vm, &e));
    };
    let nt = name_tuple(vm, &name);
    let e = entry(vm, &nt)?;
    let parse = entry_get(vm, &e, "parse")?;
    let inner = if py::is_none(&parse) {
        Value::None
    } else {
        let l = rlexer::new_lexer(vm, l, Value::None)?;
        vm.call(&parse, vec![l], Vec::new())?
    };
    Ok(vm.types.new_tuple(vec![nt, inner]))
}

fn f_parse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let line = py::string(vm, &args[1])?;
    parse(vm, &args[0], &line, &args[2])
}

/// `call(method, parsed, *args, **kwargs)`.
pub fn call_method(
    vm: &mut Vm,
    method: &str,
    parsed: &Value,
    args: Vec<Value>,
    kw: Kw,
) -> PyResult<Value> {
    let p = py::items(vm, parsed)?;
    let e = entry(vm, &p[0])?;
    let m = entry_get(vm, &e, method)?;
    if py::is_none(&m) {
        return Ok(Value::None);
    }
    let mut a = vec![p[1].clone()];
    a.extend(args);
    vm.call(&m, a, kw.to_vec())
}

fn f_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let method = py::string(vm, &args[0])?;
    call_method(vm, &method, &args[1], args[2..].to_vec(), kw)
}

fn f_get(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let key = py::string(vm, &args[0])?;
    let p = py::items(vm, &args[1])?;
    let e = entry(vm, &p[0])?;
    entry_get(vm, &e, &key)
}

fn f_get_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::items(vm, &args[0])?;
    let mut parts = Vec::new();
    for w in py::items(vm, &p[0])? {
        parts.push(py::string(vm, &w)?);
    }
    Ok(vm.types.new_str(&parts.join(" ")))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("register", f_register),
            ("parse", f_parse),
            ("call", f_call),
            ("get", f_get),
            ("get_name", f_get_name),
        ],
    );
    crysirror_python::modules::set(vm, &m, "registry", py::dict(vm));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
