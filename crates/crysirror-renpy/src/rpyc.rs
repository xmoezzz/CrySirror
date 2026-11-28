// SPDX-License-Identifier: MPL-2.0
//! Compiled script loading (`renpy/script.py` `read_rpyc_data`/`load_file`).
//!
//! `.rpyc` files hold a pickled `(data, stmts)` pair whose objects are
//! instances of `renpy.ast`, `renpy.atl`, `renpy.sl2.slast`, … classes.
//! Unpickling resolves every `renpy.*` global to a data shell class created
//! in the interpreter (no Ren'Py code runs); the shells are then converted
//! into native nodes by attribute name. Missing attributes take the class
//! defaults of the pickling release (`cslots` omits default values).

use crate::ast::*;
use crate::atl::{RawBlock, RawMultipurpose, RawStatement};
use crate::lexer::PyExpr;
use crate::sl2::{self, SlNode};
use crysirror_python::{Payload, PyResult, Value, Vm, ops};
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;

pub const RPYC2_HEADER: &[u8] = b"RENPY RPC2";

#[derive(Debug)]
pub enum RpycError {
    Format(String),
    Python(String),
}

impl std::fmt::Display for RpycError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpycError::Format(m) | RpycError::Python(m) => f.write_str(m),
        }
    }
}

fn inflate(d: &[u8]) -> Result<Vec<u8>, RpycError> {
    let mut out = Vec::new();
    const MAX_SLOT_BYTES: u64 = 64 * 1024 * 1024;
    flate2::read::ZlibDecoder::new(d)
        .take(MAX_SLOT_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| RpycError::Format(format!("zlib: {e}")))?;
    if out.len() as u64 > MAX_SLOT_BYTES {
        return Err(RpycError::Format(
            "Ren'Py compiled script slot exceeds memory limit".into(),
        ));
    }
    Ok(out)
}

/// Slot payload (decompressed), or None when the slot is absent.
pub fn read_slot(data: &[u8], slot: u32) -> Result<Option<Vec<u8>>, RpycError> {
    if !data.starts_with(RPYC2_HEADER) {
        if slot != 1 {
            return Ok(None);
        }
        return inflate(data).map(Some);
    }
    let mut pos = RPYC2_HEADER.len();
    loop {
        let hdr = data
            .get(pos..pos + 12)
            .ok_or_else(|| RpycError::Format("truncated slot table".into()))?;
        let s = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let start = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as usize;
        if s == 0 {
            return Ok(None);
        }
        if s == slot {
            let body = data
                .get(
                    start
                        ..start
                            .checked_add(len)
                            .ok_or_else(|| RpycError::Format("slot overflow".into()))?,
                )
                .ok_or_else(|| RpycError::Format("slot outside file".into()))?;
            return inflate(body).map(Some);
        }
        pos += 12;
    }
}

const SHELL_SOURCE: &str = r#"
class _Shell(object):
    def __init__(self, *args, **kwargs):
        self._args = args
    def __setstate__(self, state):
        if isinstance(state, dict):
            self.__dict__.update(state)
        elif isinstance(state, tuple) and len(state) == 2 and (state[0] is None or isinstance(state[0], dict)) and (state[1] is None or isinstance(state[1], dict)):
            if state[0]:
                self.__dict__.update(state[0])
            if state[1]:
                self.__dict__.update(state[1])
        else:
            self._state = state

class PyExpr(str):
    def __new__(cls, s, filename=None, linenumber=0, *rest):
        self = str.__new__(cls, s)
        self.filename = filename
        self.linenumber = linenumber
        return self
    def __getnewargs__(self):
        return (str(self), self.filename, self.linenumber)
    def __setstate__(self, state):
        if isinstance(state, dict):
            self.__dict__.update(state)
        elif isinstance(state, tuple):
            for d in state:
                if isinstance(d, dict):
                    self.__dict__.update(d)

def _make(module, name):
    if name == 'PyExpr':
        cls = type(name, (PyExpr,), {})
    else:
        cls = type(name, (_Shell,), {})
    cls.__module__ = module
    cls._shell_module = module
    cls._shell_name = name
    return cls
"#;

/// Unpickling context with per-(module, name) shell classes.
pub struct Shells {
    ns: crysirror_python::ObjRef,
    classes: HashMap<(String, String), Value>,
}

impl Shells {
    pub fn new(vm: &mut Vm) -> Result<Shells, RpycError> {
        let ns = crysirror_python::run_source(vm, SHELL_SOURCE, crysirror_python::Dialect::Py3)
            .map_err(RpycError::Python)?;
        Ok(Shells {
            ns,
            classes: HashMap::new(),
        })
    }

    /// The namespace defining `_make`/`PyExpr` (for tools that build pickles).
    pub fn namespace(&self) -> crysirror_python::ObjRef {
        self.ns.clone()
    }

    fn shell(&mut self, vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
        let key = (module.to_string(), name.to_string());
        if let Some(c) = self.classes.get(&key) {
            return Ok(c.clone());
        }
        let make = self.ns.dict_get_str_raw("_make").unwrap();
        let m = vm.types.new_str(module);
        let n = vm.types.new_str(name);
        let c = vm.call(&make, vec![m, n], Vec::new())?;
        self.classes.insert(key, c.clone());
        Ok(c)
    }

    /// `find_class` for script pickles: Ren'Py classes become shells, other
    /// modules (collections, copyreg, builtins) resolve normally.
    pub fn find_class(&mut self, vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
        match (module, name) {
            ("renpy.cslots", "__newobj__") | ("renpy.object", "__newobj__") => {
                return crysirror_python::pickle::default_find_class(vm, "copyreg", "__newobj__");
            }
            _ => {}
        }
        if module == "renpy"
            || module.starts_with("renpy.")
            || module == "store"
            || module.starts_with("store.")
        {
            return self.shell(vm, module, name);
        }
        crysirror_python::pickle::default_find_class(vm, module, name)
    }
}

/// Loads `(data, stmts)` from rpyc bytes, preferring slot 2 (static
/// transforms applied) like Ren'Py.
pub fn unpickle_script(
    vm: &mut Vm,
    shells: &mut Shells,
    bytes: &[u8],
) -> Result<(Value, Vec<Value>), RpycError> {
    let payload = match read_slot(bytes, 2)? {
        Some(p) => p,
        None => read_slot(bytes, 1)?.ok_or_else(|| RpycError::Format("no script slot".into()))?,
    };
    let opts = crysirror_python::pickle::UnpickleOptions::default();
    let mut fc = |vm: &mut Vm, m: &str, n: &str| shells.find_class(vm, m, n);
    let v = crysirror_python::pickle::loads(vm, &payload, &mut fc, &opts)
        .map_err(|e| RpycError::Python(py_err(vm, e)))?;
    let parts = ops::collect(vm, &v).map_err(|e| RpycError::Python(py_err(vm, e)))?;
    if parts.len() != 2 {
        return Err(RpycError::Format(
            "script pickle is not (data, stmts)".into(),
        ));
    }
    let stmts = ops::collect(vm, &parts[1]).map_err(|e| RpycError::Python(py_err(vm, e)))?;
    Ok((parts[0].clone(), stmts))
}

pub fn py_err(vm: &mut Vm, e: crysirror_python::PyErr) -> String {
    match e {
        crysirror_python::PyErr::Exc(o) => crysirror_python::format_exception(vm, &o),
        crysirror_python::PyErr::Suspend(_) => "suspended".into(),
    }
}

/// Converts shell objects into arena nodes.
pub struct Converter<'a> {
    pub vm: &'a mut Vm,
    pub arena: &'a mut Arena,
    pub registry: &'a sl2::Registry,
    /// Node shells already converted (pickles share nodes, e.g. `next`).
    seen: HashMap<i64, NodeId>,
    pub errors: Vec<String>,
}

fn class_name(vm: &Vm, v: &Value) -> (String, String) {
    let t = ops::type_of(vm, v);
    let m = t
        .dict_get_str_raw("_shell_module")
        .and_then(|x| x.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let n = t
        .dict_get_str_raw("_shell_name")
        .and_then(|x| x.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| ops::type_name(vm, v));
    (m, n)
}

impl<'a> Converter<'a> {
    pub fn new(vm: &'a mut Vm, arena: &'a mut Arena, registry: &'a sl2::Registry) -> Self {
        Converter {
            vm,
            arena,
            registry,
            seen: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn attr(&mut self, o: &Value, name: &str) -> Option<Value> {
        let v = match o {
            Value::Obj(ob) => ops::get_raw_attr(ob, name),
            _ => None,
        }?;
        if v.is_none() {
            return None;
        }
        Some(v)
    }

    fn s(&mut self, v: &Value) -> String {
        match v.as_str() {
            Some(s) => s.to_string(),
            None => ops::str(self.vm, v).unwrap_or_default(),
        }
    }

    fn str_attr(&mut self, o: &Value, name: &str) -> Option<String> {
        let v = self.attr(o, name)?;
        Some(self.s(&v))
    }

    fn bool_attr(&mut self, o: &Value, name: &str, default: bool) -> bool {
        match self.attr(o, name) {
            Some(v) => ops::truthy(self.vm, &v).unwrap_or(default),
            None => default,
        }
    }

    fn int_attr(&mut self, o: &Value, name: &str, default: i64) -> i64 {
        match self.attr(o, name) {
            Some(Value::Int(i)) => i,
            Some(v) => ops::index_of(self.vm, &v).unwrap_or(default),
            None => default,
        }
    }

    fn list(&mut self, v: &Value) -> Vec<Value> {
        ops::collect(self.vm, v).unwrap_or_default()
    }

    fn loc(&mut self, o: &Value) -> (Rc<str>, u32) {
        let f = self
            .str_attr(o, "filename")
            .unwrap_or_else(|| "<rpyc>".into());
        let l = self.int_attr(o, "linenumber", 0);
        (f.as_str().into(), l.max(0) as u32)
    }

    /// PyExpr (str subclass with filename/linenumber) or plain str.
    fn pyexpr(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> PyExpr {
        let text = self.s(v);
        let (f, l) = match v {
            Value::Obj(o) => (
                ops::get_raw_attr(o, "filename")
                    .and_then(|x| x.as_str().map(|s| Rc::<str>::from(s)))
                    .unwrap_or_else(|| loc.0.clone()),
                ops::get_raw_attr(o, "linenumber")
                    .and_then(|x| {
                        if let Value::Int(i) = x {
                            Some(i as u32)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(loc.1),
            ),
            _ => (loc.0.clone(), loc.1),
        };
        PyExpr::new(text, f, l)
    }

    fn opt_expr(&mut self, o: &Value, name: &str, loc: &(Rc<str>, u32)) -> Option<PyExpr> {
        let v = self.attr(o, name)?;
        Some(self.pyexpr(&v, loc))
    }

    /// PyCode: state tuple (version, source, location, mode[, py]).
    fn pycode(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> PyCode {
        let (src, location, mode, py3) = match v.obj().and_then(|o| ops::get_raw_attr(o, "_state"))
        {
            Some(st) => {
                let p = self.list(&st);
                (
                    p.get(1).cloned().unwrap_or(Value::None),
                    p.get(2).cloned(),
                    p.get(3).cloned(),
                    p.get(4)
                        .map(|x| matches!(x, Value::Int(3)))
                        .unwrap_or(false),
                )
            }
            None => (
                self.attr(v, "source").unwrap_or(Value::None),
                self.attr(v, "location"),
                self.attr(v, "mode"),
                self.attr(v, "py")
                    .map(|x| matches!(x, Value::Int(3)))
                    .unwrap_or(false),
            ),
        };
        let mut l = loc.clone();
        if let Some(locv) = location {
            let parts = self.list(&locv);
            if let (Some(f), Some(n)) = (parts.first(), parts.get(1)) {
                l = (
                    self.s(f).as_str().into(),
                    match n {
                        Value::Int(i) => *i as u32,
                        _ => loc.1,
                    },
                );
            }
        }
        let source = self.pyexpr(&src, &l);
        let mode = match mode.as_ref().and_then(|m| m.as_str()) {
            Some("eval") => "eval",
            Some("hide") => "hide",
            _ => "exec",
        };
        PyCode { source, mode, py3 }
    }

    fn signature(&mut self, v: Option<Value>) -> Option<Signature> {
        let v = v?;
        let (_, cls) = class_name(self.vm, &v);
        let mut sig = Signature::default();
        match cls.as_str() {
            "ParameterInfo" => {
                // (name, default) list plus positional/extrapos/extrakw.
                let params = self
                    .attr(&v, "parameters")
                    .map(|p| self.list(&p))
                    .unwrap_or_default();
                let positional: Vec<String> = self
                    .attr(&v, "positional")
                    .map(|p| self.list(&p))
                    .unwrap_or_default()
                    .iter()
                    .map(|x| self.s(x))
                    .collect();
                for p in params {
                    let pair = self.list(&p);
                    let name = pair.first().map(|x| self.s(x)).unwrap_or_default();
                    let default = pair.get(1).filter(|d| !d.is_none()).map(|d| self.s(d));
                    let kind = if positional.contains(&name) { 1 } else { 3 };
                    sig.parameters.push(Parameter {
                        name,
                        kind,
                        default,
                    });
                }
                if let Some(x) = self.str_attr(&v, "extrapos") {
                    sig.parameters.push(Parameter {
                        name: x,
                        kind: 2,
                        default: None,
                    });
                }
                if let Some(x) = self.str_attr(&v, "extrakw") {
                    sig.parameters.push(Parameter {
                        name: x,
                        kind: 4,
                        default: None,
                    });
                }
            }
            _ => {
                // renpy.parameter.Signature: parameters is a dict of Parameter.
                let params = self.attr(&v, "parameters");
                let items = match params {
                    Some(p) if matches!(p.payload(), Some(Payload::Dict(_))) => {
                        ops::mapping_items(self.vm, &p)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(_, v)| v)
                            .collect()
                    }
                    Some(p) => self.list(&p),
                    None => Vec::new(),
                };
                for p in items {
                    let name = self.str_attr(&p, "name").unwrap_or_default();
                    let kind = self.int_attr(&p, "kind", 1) as u8;
                    let default = match self.attr(&p, "default") {
                        Some(d) => {
                            let (_, dc) = class_name(self.vm, &d);
                            if dc == "_empty" || dc == "Sentinel" {
                                None
                            } else {
                                Some(self.s(&d))
                            }
                        }
                        None => None,
                    };
                    sig.parameters.push(Parameter {
                        name,
                        kind,
                        default,
                    });
                }
            }
        }
        Some(sig)
    }

    fn arguments(&mut self, v: Option<Value>) -> Option<ArgumentInfo> {
        let v = v?;
        let mut info = ArgumentInfo::default();
        for a in self
            .attr(&v, "arguments")
            .map(|x| self.list(&x))
            .unwrap_or_default()
        {
            let pair = self.list(&a);
            let name = pair.first().filter(|n| !n.is_none()).map(|n| self.s(n));
            let e = pair.get(1).map(|e| self.s(e)).unwrap_or_default();
            info.arguments.push((name, e));
        }
        if let Some(si) = self.attr(&v, "starred_indexes") {
            info.starred = self
                .list(&si)
                .iter()
                .filter_map(|x| {
                    if let Value::Int(i) = x {
                        Some(*i as usize)
                    } else {
                        None
                    }
                })
                .collect();
        }
        if let Some(si) = self.attr(&v, "doublestarred_indexes") {
            info.doublestarred = self
                .list(&si)
                .iter()
                .filter_map(|x| {
                    if let Value::Int(i) = x {
                        Some(*i as usize)
                    } else {
                        None
                    }
                })
                .collect();
        }
        // Older releases: extrapos/extrakw expressions appended as starred.
        if let Some(e) = self.str_attr(&v, "extrapos") {
            info.starred.push(info.arguments.len());
            info.arguments.push((None, e));
        }
        if let Some(e) = self.str_attr(&v, "extrakw") {
            info.doublestarred.push(info.arguments.len());
            info.arguments.push((None, e));
        }
        Some(info)
    }

    fn imspec(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> ImSpec {
        let p = self.list(v);
        let names =
            |c: &mut Self, x: &Value| -> Vec<String> { c.list(x).iter().map(|n| c.s(n)).collect() };
        let mut spec = ImSpec::default();
        let exprs = |c: &mut Self, x: &Value| -> Vec<PyExpr> {
            c.list(x).iter().map(|e| c.pyexpr(e, loc)).collect()
        };
        match p.len() {
            3 => {
                spec.name = names(self, &p[0]);
                spec.at_list = exprs(self, &p[1]);
                spec.layer = p[2].as_str().map(|s| s.to_string());
            }
            n if n >= 6 => {
                spec.name = names(self, &p[0]);
                spec.expression = (!p[1].is_none()).then(|| self.pyexpr(&p[1], loc));
                spec.tag = p[2].as_str().map(|s| s.to_string());
                spec.at_list = exprs(self, &p[3]);
                spec.layer = p[4].as_str().map(|s| s.to_string());
                spec.zorder = (!p[5].is_none()).then(|| self.pyexpr(&p[5], loc));
                if n >= 7 {
                    spec.behind = names(self, &p[6]);
                }
            }
            _ => self.errors.push(format!(
                "{}:{}: unknown imspec of {} items",
                loc.0,
                loc.1,
                p.len()
            )),
        }
        spec
    }

    fn block(&mut self, v: Option<Value>) -> Vec<NodeId> {
        let Some(v) = v else { return Vec::new() };
        let items = self.list(&v);
        let mut out = Vec::new();
        for it in items {
            if let Some(id) = self.node(&it) {
                out.push(id);
            }
        }
        out
    }

    pub fn convert_block(&mut self, stmts: &[Value]) -> Vec<NodeId> {
        stmts.iter().filter_map(|s| self.node(s)).collect()
    }

    fn name(&mut self, o: &Value) -> Option<NodeName> {
        let n = self.attr(o, "name")?;
        if let Some(s) = n.as_str() {
            return Some(NodeName::Label(s.into()));
        }
        let parts = self.list(&n);
        if !parts.is_empty() {
            let mut atoms = Vec::new();
            for p in &parts {
                match p {
                    Value::Int(i) => atoms.push(crate::ast::NameAtom::Int(*i)),
                    Value::Float(x) => atoms.push(crate::ast::NameAtom::Float(x.to_bits())),
                    Value::Bool(b) => atoms.push(crate::ast::NameAtom::Int(*b as i64)),
                    v if v.as_str().is_some() => {
                        atoms.push(crate::ast::NameAtom::Str(v.as_str().unwrap().into()))
                    }
                    _ => {
                        atoms.clear();
                        break;
                    }
                }
            }
            if atoms.len() == parts.len() {
                return Some(NodeName::from_atoms(atoms));
            }
        }
        let r = ops::repr(self.vm, &n).unwrap_or_default();
        Some(NodeName::Other(r.as_str().into()))
    }

    pub fn node(&mut self, o: &Value) -> Option<NodeId> {
        if o.is_none() {
            return None;
        }
        let key = o.id();
        if let Some(id) = self.seen.get(&key) {
            return Some(*id);
        }
        let (module, cls) = class_name(self.vm, o);
        let loc = self.loc(o);
        // Reserve the slot first: blocks may refer back (statement_start).
        let id = self.arena.add(&loc, NodeKind::Pass);
        self.seen.insert(key, id);
        let kind = match self.node_kind(o, &module, &cls, &loc) {
            Some(k) => k,
            None => {
                self.errors.push(format!(
                    "{}:{}: unsupported pickled node {module}.{cls}",
                    loc.0, loc.1
                ));
                NodeKind::Pass
            }
        };
        let name = self.name(o);
        let rollback = match self.str_attr(o, "rollback").as_deref() {
            Some("never") => Rollback::Never,
            Some("force") => Rollback::Force,
            _ => Rollback::Normal,
        };
        let ss = self
            .attr(o, "statement_start")
            .and_then(|s| if s.is(o) { Some(id) } else { self.node(&s) });
        let n = self.arena.get_mut(id);
        n.kind = kind;
        n.name = name;
        n.rollback = rollback;
        n.statement_start = ss;
        Some(id)
    }

    fn atl(&mut self, v: Option<Value>) -> Option<Rc<RawBlock>> {
        let v = v?;
        Some(Rc::new(self.raw_block(&v)))
    }

    fn raw_loc(&mut self, v: &Value) -> (Rc<str>, u32) {
        match self.attr(v, "loc") {
            Some(l) => {
                let p = self.list(&l);
                let f = p.first().map(|x| self.s(x)).unwrap_or_default();
                let n = match p.get(1) {
                    Some(Value::Int(i)) => *i as u32,
                    _ => 0,
                };
                (f.as_str().into(), n)
            }
            None => ("<atl>".into(), 0),
        }
    }

    fn raw_block(&mut self, v: &Value) -> RawBlock {
        let loc = self.raw_loc(v);
        let stmts = self
            .attr(v, "statements")
            .map(|x| self.list(&x))
            .unwrap_or_default();
        let animation = self.bool_attr(v, "animation", false);
        let statements = stmts.iter().filter_map(|s| self.raw_statement(s)).collect();
        RawBlock {
            loc,
            statements,
            animation,
        }
    }

    fn raw_statement(&mut self, v: &Value) -> Option<RawStatement> {
        let (_, cls) = class_name(self.vm, v);
        let loc = self.raw_loc(v);
        Some(match cls.as_str() {
            "RawBlock" => RawStatement::Block(self.raw_block(v)),
            "RawMultipurpose" => {
                let mut rm = RawMultipurpose {
                    loc: Some(loc.clone()),
                    ..Default::default()
                };
                rm.warper = self.str_attr(v, "warper");
                rm.duration = self.opt_expr(v, "duration", &loc);
                rm.warp_function = self.opt_expr(v, "warp_function", &loc);
                for p in self
                    .attr(v, "properties")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                {
                    let pr = self.list(&p);
                    if pr.len() == 2 {
                        let n = self.s(&pr[0]);
                        let e = self.pyexpr(&pr[1], &loc);
                        rm.properties.push((n, e));
                    }
                }
                for p in self
                    .attr(v, "expressions")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                {
                    let pr = self.list(&p);
                    if !pr.is_empty() {
                        let e = self.pyexpr(&pr[0], &loc);
                        let w = pr
                            .get(1)
                            .filter(|w| !w.is_none())
                            .map(|w| self.pyexpr(w, &loc));
                        rm.expressions.push((e, w));
                    }
                }
                for p in self
                    .attr(v, "splines")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                {
                    let pr = self.list(&p);
                    if pr.len() == 2 {
                        let n = self.s(&pr[0]);
                        let knots = self
                            .list(&pr[1])
                            .iter()
                            .map(|k| self.pyexpr(k, &loc))
                            .collect();
                        rm.splines.push((n, knots));
                    }
                }
                rm.revolution = self.str_attr(v, "revolution");
                rm.circles = self.opt_expr(v, "circles", &loc);
                RawStatement::Multipurpose(rm)
            }
            "RawRepeat" => RawStatement::Repeat {
                repeats: self.opt_expr(v, "repeats", &loc),
                loc,
            },
            "RawParallel" => RawStatement::Parallel {
                blocks: self
                    .attr(v, "blocks")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .iter()
                    .map(|b| self.raw_block(b))
                    .collect(),
                loc,
            },
            "RawChoice" => {
                let mut choices = Vec::new();
                for c in self
                    .attr(v, "choices")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                {
                    let p = self.list(&c);
                    if p.len() == 2 {
                        let e = self.pyexpr(&p[0], &loc);
                        let b = self.raw_block(&p[1]);
                        choices.push((e, b));
                    }
                }
                RawStatement::Choice { loc, choices }
            }
            "RawOn" => {
                let mut handlers = Vec::new();
                if let Some(h) = self.attr(v, "handlers") {
                    for (k, b) in ops::mapping_items(self.vm, &h).unwrap_or_default() {
                        let n = self.s(&k);
                        let rb = self.raw_block(&b);
                        handlers.push((n, rb));
                    }
                }
                RawStatement::On { loc, handlers }
            }
            "RawTime" => RawStatement::Time {
                time: self.opt_expr(v, "time", &loc)?,
                loc,
            },
            "RawEvent" => RawStatement::Event {
                name: self.str_attr(v, "name")?,
                loc,
            },
            "RawFunction" => RawStatement::Function {
                expr: self.opt_expr(v, "expr", &loc)?,
                loc,
            },
            "RawContainsExpr" => RawStatement::ContainsExpr {
                expr: self.opt_expr(v, "expression", &loc)?,
                loc,
            },
            "RawChild" => RawStatement::Child {
                children: self
                    .attr(v, "children")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .iter()
                    .map(|b| self.raw_block(b))
                    .collect(),
                loc,
            },
            other => {
                self.errors.push(format!(
                    "{}:{}: unsupported ATL statement {other}",
                    loc.0, loc.1
                ));
                return None;
            }
        })
    }

    fn say(&mut self, o: &Value, loc: &(Rc<str>, u32)) -> NodeKind {
        let attributes = self
            .attr(o, "attributes")
            .map(|a| self.list(&a).iter().map(|x| self.s(x)).collect());
        let temporary = self
            .attr(o, "temporary_attributes")
            .map(|a| self.list(&a).iter().map(|x| self.s(x)).collect());
        let args = self.attr(o, "arguments");
        NodeKind::Say {
            who: self.opt_expr(o, "who", loc),
            what: self.str_attr(o, "what").unwrap_or_default(),
            with_: self.opt_expr(o, "with_", loc),
            interact: self.bool_attr(o, "interact", true),
            attributes,
            temporary_attributes: temporary,
            arguments: self.arguments(args),
            identifier: self.str_attr(o, "identifier"),
            explicit_identifier: self.bool_attr(o, "explicit_identifier", false),
        }
    }

    fn node_kind(
        &mut self,
        o: &Value,
        module: &str,
        cls: &str,
        loc: &(Rc<str>, u32),
    ) -> Option<NodeKind> {
        if module != "renpy.ast" && !module.is_empty() {
            return None;
        }
        Some(match cls {
            "Say" => self.say(o, loc),
            "Init" => NodeKind::Init {
                block: {
                    let b = self.attr(o, "block");
                    self.block(b)
                },
                priority: self.int_attr(o, "priority", 0) as i32,
            },
            "Label" => {
                let p = self.attr(o, "parameters");
                let b = self.attr(o, "block");
                NodeKind::Label {
                    name: self.str_attr(o, "name").unwrap_or_default().as_str().into(),
                    block: self.block(b),
                    parameters: self.signature(p),
                    hide: self.bool_attr(o, "hide", false),
                }
            }
            "Python" | "EarlyPython" => {
                let code = self.attr(o, "code").map(|c| self.pycode(&c, loc))?;
                let hide = self.bool_attr(o, "hide", false);
                let store = self.str_attr(o, "store").unwrap_or_else(|| "store".into());
                if cls == "Python" {
                    NodeKind::Python { code, hide, store }
                } else {
                    NodeKind::EarlyPython { code, hide, store }
                }
            }
            "Image" => {
                let name = self
                    .attr(o, "imgname")
                    .map(|n| self.list(&n).iter().map(|x| self.s(x)).collect())
                    .unwrap_or_default();
                let code = self.attr(o, "code").map(|c| self.pycode(&c, loc));
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                NodeKind::Image {
                    name,
                    code,
                    atl: self.atl(atl),
                }
            }
            "Transform" => {
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                let p = self.attr(o, "parameters");
                NodeKind::Transform {
                    store: self.str_attr(o, "store").unwrap_or_else(|| "store".into()),
                    name: self.str_attr(o, "varname").unwrap_or_default(),
                    atl: self.atl(atl)?,
                    parameters: self.signature(p),
                }
            }
            "Show" => {
                let spec = self
                    .attr(o, "imspec")
                    .map(|s| self.imspec(&s, loc))
                    .unwrap_or_default();
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                NodeKind::Show {
                    imspec: spec,
                    atl: self.atl(atl),
                }
            }
            "ShowLayer" | "Camera" => {
                let at_list = self
                    .attr(o, "at_list")
                    .map(|a| self.list(&a).iter().map(|e| self.pyexpr(e, loc)).collect())
                    .unwrap_or_default();
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                let layer = self.str_attr(o, "layer").unwrap_or_else(|| "master".into());
                let atl = self.atl(atl);
                if cls == "Camera" {
                    NodeKind::Camera {
                        layer,
                        at_list,
                        atl,
                    }
                } else {
                    NodeKind::ShowLayer {
                        layer,
                        at_list,
                        atl,
                    }
                }
            }
            "Scene" => {
                let spec = self.attr(o, "imspec").map(|s| self.imspec(&s, loc));
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                NodeKind::Scene {
                    imspec: spec,
                    layer: self.str_attr(o, "layer"),
                    atl: self.atl(atl),
                }
            }
            "Hide" => NodeKind::Hide {
                imspec: self
                    .attr(o, "imspec")
                    .map(|s| self.imspec(&s, loc))
                    .unwrap_or_default(),
            },
            "With" => NodeKind::With {
                expr: self.opt_expr(o, "expr", loc),
                paired: self.opt_expr(o, "paired", loc),
            },
            "Call" => {
                let a = self.attr(o, "arguments");
                NodeKind::Call {
                    label: self.str_attr(o, "label").unwrap_or_default(),
                    expression: self.bool_attr(o, "expression", false),
                    arguments: self.arguments(a),
                    global_label: self.str_attr(o, "global_label").unwrap_or_default(),
                }
            }
            "Return" => NodeKind::Return {
                expression: self.opt_expr(o, "expression", loc),
            },
            "Menu" => {
                let mut items = Vec::new();
                let item_args = self
                    .attr(o, "item_arguments")
                    .map(|a| self.list(&a))
                    .unwrap_or_default();
                for (i, it) in self
                    .attr(o, "items")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                {
                    let p = self.list(&it);
                    let label = p.first().map(|x| self.s(x)).unwrap_or_default();
                    let condition = p
                        .get(1)
                        .map(|x| self.pyexpr(x, loc))
                        .unwrap_or_else(|| PyExpr::new("True", loc.0.clone(), loc.1));
                    let block = p.get(2).filter(|b| !b.is_none()).map(|b| {
                        let b = b.clone();
                        self.block(Some(b))
                    });
                    let arguments =
                        self.arguments(item_args.get(i).cloned().filter(|v| !v.is_none()));
                    items.push(MenuItem {
                        label,
                        condition,
                        block,
                        arguments,
                    });
                }
                let a = self.attr(o, "arguments");
                NodeKind::Menu {
                    items,
                    set: self.opt_expr(o, "set", loc),
                    with_: self.opt_expr(o, "with_", loc),
                    has_caption: self.bool_attr(o, "has_caption", false),
                    arguments: self.arguments(a),
                }
            }
            "Jump" => NodeKind::Jump {
                target: self.str_attr(o, "target").unwrap_or_default(),
                expression: self.bool_attr(o, "expression", false),
                global_label: self.str_attr(o, "global_label").unwrap_or_default(),
            },
            "Pass" => NodeKind::Pass,
            "While" => {
                let b = self.attr(o, "block");
                NodeKind::While {
                    condition: self
                        .opt_expr(o, "condition", loc)
                        .unwrap_or_else(|| PyExpr::new("False", loc.0.clone(), loc.1)),
                    block: self.block(b),
                }
            }
            "If" => {
                let mut entries = Vec::new();
                for e in self
                    .attr(o, "entries")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                {
                    let p = self.list(&e);
                    let c = p
                        .first()
                        .map(|x| self.pyexpr(x, loc))
                        .unwrap_or_else(|| PyExpr::new("True", loc.0.clone(), loc.1));
                    let b = self.block(p.get(1).cloned());
                    entries.push((c, b));
                }
                NodeKind::If { entries }
            }
            "UserStatement" => {
                let subparses = self
                    .attr(o, "subparses")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .iter()
                    .map(|sp| {
                        let b = self.attr(sp, "block");
                        self.block(b)
                    })
                    .collect();
                let code_block = self.attr(o, "code_block").map(|b| self.block(Some(b)));
                let atl = self.attr(o, "atl").filter(|v| !matches!(v, Value::None));
                let atl = self.atl(atl);
                NodeKind::UserStatement {
                    line: self.str_attr(o, "line").unwrap_or_default(),
                    block: Rc::new(
                        self.attr(o, "block")
                            .map(|b| self.grouped(&b, loc))
                            .unwrap_or_default(),
                    ),
                    parsed: self.attr(o, "parsed"),
                    subparses,
                    code_block,
                    init_priority: self.int_attr(o, "init_priority", 0) as i32,
                    translatable: self.bool_attr(o, "translatable", false),
                    atl,
                }
            }
            "PostUserStatement" => NodeKind::PostUserStatement {
                parent: self.attr(o, "parent").and_then(|p| self.node(&p))?,
            },
            "Define" => NodeKind::Define {
                store: self.str_attr(o, "store").unwrap_or_else(|| "store".into()),
                name: self.str_attr(o, "varname").unwrap_or_default(),
                index: self.attr(o, "index").map(|i| self.pycode(&i, loc).source),
                operator: self.str_attr(o, "operator").unwrap_or_else(|| "=".into()),
                code: self.attr(o, "code").map(|c| self.pycode(&c, loc))?,
            },
            "Default" => NodeKind::Default {
                store: self.str_attr(o, "store").unwrap_or_else(|| "store".into()),
                name: self.str_attr(o, "varname").unwrap_or_default(),
                code: self.attr(o, "code").map(|c| self.pycode(&c, loc))?,
            },
            "Screen" => {
                let s = self.attr(o, "screen")?;
                NodeKind::Screen {
                    screen: Rc::new(self.screen(&s, loc)?),
                }
            }
            "Style" => {
                let mut properties = Vec::new();
                if let Some(p) = self.attr(o, "properties") {
                    for (k, v) in ops::mapping_items(self.vm, &p).unwrap_or_default() {
                        let n = self.s(&k);
                        let e = self.pyexpr(&v, loc);
                        properties.push((n, e));
                    }
                }
                NodeKind::Style(Box::new(Style {
                    name: self.str_attr(o, "style_name").unwrap_or_default(),
                    parent: self.str_attr(o, "parent"),
                    clear: self.bool_attr(o, "clear", false),
                    take: self.str_attr(o, "take"),
                    delattr: self
                        .attr(o, "delattr")
                        .map(|d| self.list(&d).iter().map(|x| self.s(x)).collect())
                        .unwrap_or_default(),
                    variant: self.opt_expr(o, "variant", loc),
                    properties,
                }))
            }
            "Testcase" => NodeKind::Testcase {
                name: self.str_attr(o, "label").unwrap_or_default(),
            },
            "RPY" => {
                let r = self
                    .attr(o, "rest")
                    .map(|r| self.list(&r))
                    .unwrap_or_default();
                NodeKind::Rpy {
                    rest: (
                        r.first().map(|x| self.s(x)).unwrap_or_default(),
                        r.get(1).map(|x| self.s(x)).unwrap_or_default(),
                    ),
                }
            }
            "Translate" => {
                let b = self.attr(o, "block");
                NodeKind::Translate {
                    identifier: self.str_attr(o, "identifier").unwrap_or_default(),
                    language: self.str_attr(o, "language"),
                    block: self.block(b),
                    alternate: self.str_attr(o, "alternate"),
                }
            }
            "TranslateSay" => {
                let say = self.say(o, loc);
                NodeKind::TranslateSay {
                    identifier: self.str_attr(o, "identifier").unwrap_or_default(),
                    language: self.str_attr(o, "language"),
                    say: Box::new(say),
                }
            }
            "EndTranslate" => NodeKind::EndTranslate,
            "TranslateString" => NodeKind::TranslateString {
                language: self.str_attr(o, "language"),
                old: self.str_attr(o, "old").unwrap_or_default(),
                new: self.str_attr(o, "new").unwrap_or_default(),
            },
            "TranslatePython" => NodeKind::TranslatePython {
                language: self.str_attr(o, "language"),
                code: self.attr(o, "code").map(|c| self.pycode(&c, loc))?,
            },
            "TranslateBlock" | "TranslateEarlyBlock" => {
                let b = self.attr(o, "block");
                let block = self.block(b);
                let language = self.str_attr(o, "language");
                if cls == "TranslateBlock" {
                    NodeKind::TranslateBlock { language, block }
                } else {
                    NodeKind::TranslateEarlyBlock { language, block }
                }
            }
            _ => return None,
        })
    }

    /// Re-creates lexer lines from a pickled UserStatement block
    /// (`[(filename, linenumber, text, subblock)]`).
    fn grouped(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> Vec<crate::lexer::GroupedLine> {
        let mut out = Vec::new();
        for it in self.list(v) {
            let p = self.list(&it);
            if p.len() < 3 {
                continue;
            }
            let f: Rc<str> = self.s(&p[0]).as_str().into();
            let n = match &p[1] {
                Value::Int(i) => *i as u32,
                _ => loc.1,
            };
            let (indent, text, sub) = if p.len() >= 5 {
                (
                    match &p[2] {
                        Value::Int(i) => *i as usize,
                        _ => 0,
                    },
                    self.s(&p[3]),
                    p.get(4).cloned(),
                )
            } else {
                (0, self.s(&p[2]), p.get(3).cloned())
            };
            let block = sub.map(|s| self.grouped(&s, loc)).unwrap_or_default();
            out.push(crate::lexer::GroupedLine {
                filename: f,
                number: n,
                indent,
                text: text.chars().collect::<Vec<_>>().into(),
                block: Rc::new(block),
            });
        }
        out
    }

    // ---- screens ----

    fn sl_block(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> sl2::Block {
        let mut b = sl2::Block::default();
        for kv in self
            .attr(v, "keyword")
            .map(|x| self.list(&x))
            .unwrap_or_default()
        {
            let p = self.list(&kv);
            if p.len() == 2 {
                let n = self.s(&p[0]);
                let e = self.pyexpr(&p[1], loc);
                b.keyword.push((n, e));
            }
        }
        for c in self
            .attr(v, "children")
            .map(|x| self.list(&x))
            .unwrap_or_default()
        {
            if let Some(n) = self.sl_node(&c, loc) {
                b.children.push(n);
            }
        }
        let atl = self.attr(v, "atl_transform");
        b.atl_transform = self.atl(atl);
        b.variable = self.str_attr(v, "variable");
        b
    }

    fn sl_loc(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> (Rc<str>, u32) {
        match self.attr(v, "location") {
            Some(l) => {
                let p = self.list(&l);
                let f = p
                    .first()
                    .map(|x| self.s(x))
                    .unwrap_or_else(|| loc.0.to_string());
                let n = match p.get(1) {
                    Some(Value::Int(i)) => *i as u32,
                    _ => loc.1,
                };
                (f.as_str().into(), n)
            }
            None => loc.clone(),
        }
    }

    fn sl_entries(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> Vec<(Option<PyExpr>, sl2::Block)> {
        let mut out = Vec::new();
        for e in self
            .attr(v, "entries")
            .map(|x| self.list(&x))
            .unwrap_or_default()
        {
            let p = self.list(&e);
            if p.len() == 2 {
                let c = (!p[0].is_none()).then(|| self.pyexpr(&p[0], loc));
                let b = self.sl_block(&p[1], loc);
                out.push((c, b));
            }
        }
        out
    }

    /// The statement of an SLDisplayable. Before 7.x it records no name:
    /// statements sharing a displayable (`vbox`/`hbox` → MultiBox) are told
    /// apart by their style.
    fn statement_for(&mut self, disp: Option<Value>, name: Option<String>, style: Option<String>) -> String {
        if let Some(n) = name {
            if self.registry.get(&n).is_some() {
                return n;
            }
        }
        let Some(d) = disp else { return "null".into() };
        let m = ops::getattr_opt(self.vm, &d, "__module__")
            .ok()
            .flatten()
            .and_then(|x| x.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let n = ops::getattr_opt(self.vm, &d, "__name__")
            .ok()
            .flatten()
            .and_then(|x| x.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let dotted = format!("{m}.{n}");
        let candidates: Vec<_> = self.registry.statements.iter().filter(|s| s.displayable == dotted).collect();
        candidates
            .iter()
            .find(|s| style.is_some() && s.style == style)
            .or(candidates.first())
            .map(|s| s.name.clone())
            .unwrap_or(dotted)
    }

    fn sl_node(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> Option<SlNode> {
        let (_, cls) = class_name(self.vm, v);
        let loc = self.sl_loc(v, loc);
        Some(match cls.as_str() {
            "SLDisplayable" => {
                let disp = self.attr(v, "displayable");
                let name = self.str_attr(v, "name");
                let style = self.str_attr(v, "style");
                let statement = self.statement_for(disp, name, style);
                let positional = self
                    .attr(v, "positional")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .iter()
                    .map(|e| self.pyexpr(e, &loc))
                    .collect();
                let block = self.sl_block(v, &loc);
                SlNode::Displayable(Box::new(sl2::Displayable {
                    loc,
                    statement,
                    positional,
                    block,
                }))
            }
            "SLIf" => SlNode::If {
                entries: self.sl_entries(v, &loc),
                loc,
            },
            "SLShowIf" => SlNode::ShowIf {
                entries: self.sl_entries(v, &loc),
                loc,
            },
            "SLFor" => SlNode::For {
                variable: self.str_attr(v, "variable").unwrap_or_default(),
                expression: self.opt_expr(v, "expression", &loc)?,
                index_expression: self.opt_expr(v, "index_expression", &loc),
                block: self.sl_block(v, &loc),
                loc,
            },
            "SLPython" => {
                let code = self.attr(v, "code").map(|c| self.pycode(&c, &loc))?;
                SlNode::Python {
                    loc,
                    code: code.source,
                }
            }
            "SLPass" => SlNode::Pass { loc },
            "SLDefault" => SlNode::Default {
                variable: self.str_attr(v, "variable").unwrap_or_default(),
                expression: self.opt_expr(v, "expression", &loc)?,
                loc,
            },
            "SLUse" => {
                let target = self.attr(v, "target")?;
                let expression = !matches!(target.as_str(), Some(_))
                    || ops::is_user_type(&ops::type_of(self.vm, &target));
                let target = self.s(&target);
                let args = self.attr(v, "args");
                let block = self.attr(v, "block").map(|b| self.sl_block(&b, &loc));
                SlNode::Use {
                    target,
                    expression,
                    args: self.arguments(args),
                    id: self.opt_expr(v, "id", &loc),
                    block,
                    variable: self.str_attr(v, "variable"),
                    loc,
                }
            }
            "SLTransclude" => SlNode::Transclude { loc },
            "SLCustomUse" => {
                let positional = self
                    .attr(v, "positional")
                    .map(|x| self.list(&x))
                    .unwrap_or_default()
                    .iter()
                    .map(|e| self.pyexpr(e, &loc))
                    .collect();
                let block = self
                    .attr(v, "block")
                    .map(|b| self.sl_block(&b, &loc))
                    .unwrap_or_default();
                SlNode::CustomUse {
                    target: self.str_attr(v, "target").unwrap_or_default(),
                    positional,
                    block,
                    variable: self.str_attr(v, "variable"),
                    loc,
                }
            }
            "SLBreak" => SlNode::Break { loc },
            "SLContinue" => SlNode::Continue { loc },
            other => {
                self.errors.push(format!(
                    "{}:{}: unsupported screen node {other}",
                    loc.0, loc.1
                ));
                return None;
            }
        })
    }

    fn screen(&mut self, v: &Value, loc: &(Rc<str>, u32)) -> Option<sl2::Screen> {
        let (_, cls) = class_name(self.vm, v);
        if cls != "SLScreen" {
            // Screen language 1 (`renpy.screenlang.ScreenLangScreen`) is
            // compiled into Python code; it is an explicit unsupported profile here.
            self.errors.push(format!(
                "{}:{}: screen language 1 screen ({cls}) is not supported",
                loc.0, loc.1
            ));
            return None;
        }
        let loc = self.sl_loc(v, loc);
        let p = self.attr(v, "parameters");
        let block = self.sl_block(v, &loc);
        let kw = |c: &mut Self, n: &str, d: &str| c.str_attr(v, n).unwrap_or_else(|| d.to_string());
        Some(sl2::Screen {
            name: self.str_attr(v, "name").unwrap_or_default(),
            parameters: self.signature(p),
            tag: self.str_attr(v, "tag"),
            docstring: self.str_attr(v, "docstring"),
            modal: kw(self, "modal", "False"),
            zorder: kw(self, "zorder", "0"),
            variant: kw(self, "variant", "None"),
            predict: kw(self, "predict", "None"),
            layer: kw(self, "layer", "'screens'"),
            sensitive: kw(self, "sensitive", "True"),
            roll_forward: kw(self, "roll_forward", "None"),
            block,
            loc,
        })
    }
}
