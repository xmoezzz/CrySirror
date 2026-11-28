// SPDX-License-Identifier: MPL-2.0
//! `renpy.sl2.slparser` and `renpy.sl2.slproperties`, after Ren'Py
//! 6.18 – 8.6 (MIT): the Python face of the screen-language statement
//! registry.
//!
//! The parser objects (`DisplayableParser`, `IfParser`, `ScreenParser`, ...)
//! are the upstream declarative records: their `positional`, `keyword` and
//! `children` tables, the module registries (`all_statements`,
//! `statement_names`/`statements`, `all_child_statements`,
//! `childbearing_statements`, `properties`/`all_keyword_names`,
//! `incompatible_props`) and `init()` behave as in the running release.
//! The built-in tables are replayed at import from `sl_tables` (generated
//! from each release's `slproperties.py`/`sldisplayables.py`).
//!
//! Screens themselves are parsed by the Rust parser (`crate::sl2`) against
//! the runtime's statement registry: `parse_screen(l, loc)` runs it on the
//! lexer behind a Python `Lexer`. `DisplayableParser`/`CustomParser` objects
//! built after start-up (`renpy.register_sl_displayable`,
//! `renpy.register_sl_statement`) add statements to that registry, and their
//! positional arguments, which change parsing, follow `add_positional`. A
//! displayable callable is recorded by its dotted name (`module.qualname`,
//! store names resolving in the store), as pickled screen ASTs name it.
//! The per-statement `parse` methods are the Rust parser's internals; called
//! from Python they raise, naming `parse_screen` as the entry point.
use super::{
    py::{self, *},
    sl_tables::{ListDef, Op, Stmt},
};
use crate::sl2::{NChildren, StatementSpec};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const MODULE: &str = "renpy.sl2.slparser";
const PROPS: &str = "renpy.sl2.slproperties";

pub const STYLE_PREFIXES: [&str; 10] = [
    "",
    "insensitive_",
    "hover_",
    "idle_",
    "activate_",
    "selected_",
    "selected_insensitive_",
    "selected_hover_",
    "selected_idle_",
    "selected_activate_",
];

// Releases ---------------------------------------------------------------------------------------------

/// The running release, `(major, minor)` (8.6 without a runtime).
pub fn release(vm: &Vm) -> (u32, u32) {
    super::state::try_rt(vm)
        .map(|r| (r.profile.version[0], r.profile.version[1]))
        .unwrap_or((8, 6))
}

/// The reference release whose tables apply: the latest reference tag of
/// the same major version not after the running release.
pub fn ref_tag(vm: &Vm, tags: &[[u32; 2]]) -> usize {
    let (maj, min) = release(vm);
    let mut best = None;
    for (i, t) in tags.iter().enumerate() {
        if t[0] == maj && t[1] <= min {
            best = Some(i);
        }
    }
    best.or_else(|| tags.iter().position(|t| t[0] == maj))
        .or_else(|| {
            tags.iter()
                .enumerate()
                .filter(|(_, t)| (t[0], t[1]) <= (maj, min))
                .map(|(i, _)| i)
                .last()
        })
        .unwrap_or(0)
}

/// Feature set of the running release's `slparser.py`.
#[derive(Clone, Copy)]
struct Feat((u32, u32));

impl Feat {
    fn of(vm: &Vm) -> Feat {
        let t = super::sl_tables::SL2[ref_tag(vm, &tags())].0;
        Feat((t[0], t[1]))
    }
    /// 7.6 / 8.1+: `all_child_statements`, `child_statement` parsers,
    /// `break`/`continue`, incompatible-property checks.
    fn child_statements(self) -> bool {
        self.0 >= (7, 6) && self.0 != (8, 0)
    }
    /// 7.6 and 8.1 keep `statement_names`; later releases `statements`.
    fn statement_names(self) -> bool {
        self.0 == (7, 6) || self.0 == (8, 1)
    }
    fn statements_dict(self) -> bool {
        self.child_statements() && !self.statement_names()
    }
    fn all_keyword_names(self) -> bool {
        self.0 < (7, 0)
    }
    fn custom(self) -> bool {
        self.0 >= (6, 99)
    }
    fn pass_statement(self) -> bool {
        self.0 >= (7, 4)
    }
    fn proxy(self) -> bool {
        self.0 >= (8, 4)
    }
    fn copy_properties(self) -> bool {
        self.0 == (7, 8) || self.0 >= (8, 2)
    }
    /// `CustomParser(name, positional=0, children, screen)` before 7.4.
    fn custom_positional(self) -> bool {
        self.0 < (7, 4)
    }
    /// `ScreenParser` is not one of the statements (7.3+).
    fn screen_not_statement(self) -> bool {
        self.0 >= (7, 3)
    }
}

fn tags() -> Vec<[u32; 2]> {
    super::sl_tables::SL2.iter().map(|(t, _)| *t).collect()
}

// Module access ----------------------------------------------------------------------------------------

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn mset(vm: &mut Vm, n: &str, v: Value) -> PyResult<()> {
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, n, v)
}

fn is_a(vm: &mut Vm, v: &Value, cls: &str) -> PyResult<bool> {
    let c = mget(vm, cls)?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

fn str_list(vm: &Vm, items: &[&str]) -> Value {
    vm.types
        .new_list(items.iter().map(|s| vm.types.new_str(s)).collect())
}

// Statement registry (Rust parser) ---------------------------------------------------------------------

fn nchildren(vm: &mut Vm, v: &Value) -> PyResult<NChildren> {
    if v.as_str() == Some("many") {
        return Ok(NChildren::Many);
    }
    if py::is_none(v) {
        return Ok(NChildren::Zero);
    }
    if let Ok(m) = mget(vm, "many") {
        if py::same(&m, v) {
            return Ok(NChildren::Many);
        }
    }
    // Upstream compares with 0 and 1 only; any other value bears children
    // (00icon.rpy passes a string).
    if !matches!(v, Value::Int(_) | Value::Bool(_) | Value::Float(_)) {
        return Ok(NChildren::Many);
    }
    Ok(match py::int(vm, v)? {
        0 => NChildren::Zero,
        1 => NChildren::One,
        _ => NChildren::Many,
    })
}

fn dotted_name(vm: &mut Vm, f: &Value) -> PyResult<String> {
    if let Some(s) = f.as_str() {
        return Ok(s.to_owned());
    }
    let module = py::get_opt(vm, f, "__module__")?.filter(|m| !py::is_none(m));
    let name = match py::get_opt(vm, f, "__qualname__")?.filter(|n| !py::is_none(n)) {
        Some(n) => n,
        None => py::get(vm, f, "__name__")?,
    };
    let name = py::string(vm, &name)?;
    Ok(match module {
        Some(m) => format!("{}.{name}", py::string(vm, &m)?),
        None => name,
    })
}

fn register(vm: &mut Vm, spec: StatementSpec) {
    let Some(r) = super::state::try_rt(vm) else {
        return;
    };
    let mut reg = r.registry.borrow_mut();
    reg.statements.retain(|s| s.name != spec.name);
    reg.statements.push(spec);
}

fn registered(vm: &mut Vm, this: &Value) -> PyResult<bool> {
    let r = py::get_opt(vm, this, "_crysirror_registered")?;
    match r {
        Some(v) => py::truthy(vm, &v),
        None => Ok(false),
    }
}

/// Mirrors a positional argument of a registered statement.
fn register_positional(vm: &mut Vm, this: &Value, p: String) -> PyResult<()> {
    if !registered(vm, this)? {
        return Ok(());
    }
    let name = py::get(vm, this, "name")?;
    let name = py::string(vm, &name)?;
    if let Some(r) = super::state::try_rt(vm) {
        let mut reg = r.registry.borrow_mut();
        if let Some(s) = reg.statements.iter_mut().rev().find(|s| s.name == name) {
            s.positional.push(p);
        }
    }
    Ok(())
}

// Clauses ----------------------------------------------------------------------------------------------

fn attach(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let p = mget(vm, "parser")?;
    if py::truthy(vm, &p)? {
        vm.call_method(&p, "add", vec![this.clone()])?;
    }
    Ok(())
}

fn keyword_names(vm: &mut Vm) -> PyResult<Option<Value>> {
    if Feat::of(vm).all_keyword_names() {
        return Ok(Some(mget(vm, "all_keyword_names")?));
    }
    Ok(None)
}

fn note_property(vm: &mut Vm, prefix: &str, style: bool, name: &str) -> PyResult<()> {
    if let Some(all) = keyword_names(vm)? {
        if style {
            for j in STYLE_PREFIXES {
                ops::set_add(vm, &all, py::s(vm, &format!("{prefix}{j}{name}")))?;
            }
        } else {
            ops::set_add(vm, &all, py::s(vm, name))?;
        }
        return Ok(());
    }
    let props = mget(vm, "properties")?;
    let key = vm
        .types
        .new_tuple(vec![py::s(vm, prefix), Value::Bool(style)]);
    let set = ops::getitem(vm, &props, key)?;
    ops::set_add(vm, &set, py::s(vm, name))
}

fn positional_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    py::set(vm, &args[0], "name", name)?;
    attach(vm, &args[0])?;
    Ok(Value::None)
}

fn keyword_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    py::set(vm, &args[0], "name", name.clone())?;
    let n = py::string(vm, &name)?;
    note_property(vm, "", false, &n)?;
    attach(vm, &args[0])?;
    Ok(Value::None)
}

fn style_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    py::set(vm, &args[0], "name", name.clone())?;
    let n = py::string(vm, &name)?;
    note_property(vm, "", true, &n)?;
    attach(vm, &args[0])?;
    Ok(Value::None)
}

fn prefix_style_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let prefix = arg_or(args, kw, 1, "prefix", Value::None);
    let name = arg_or(args, kw, 2, "name", Value::None);
    py::set(vm, &args[0], "prefix", prefix.clone())?;
    py::set(vm, &args[0], "name", name.clone())?;
    let p = py::string(vm, &prefix)?;
    let n = py::string(vm, &name)?;
    note_property(vm, &p, true, &n)?;
    if Feat::of(vm).proxy() {
        update_incompatible(vm, &p, &n)?;
    }
    attach(vm, &args[0])?;
    Ok(Value::None)
}

// Incompatible properties (7.6 / 8.1+) -------------------------------------------------------------------

fn update_incompatible(vm: &mut Vm, prefix: &str, prop: &str) -> PyResult<()> {
    let ip = mget(vm, "incompatible_props")?;
    let Some(res) = ops::dict_get(vm, ip.obj().unwrap(), &py::s(vm, prop))? else {
        return Ok(());
    };
    if py::is_none(&res) {
        return Ok(());
    }
    let items: Vec<Value> = py::items(vm, &res)?
        .iter()
        .map(|p| -> PyResult<Value> {
            let s = py::string(vm, p)?;
            Ok(py::s(vm, &format!("{prefix}{s}")))
        })
        .collect::<PyResult<_>>()?;
    let fs = Value::Obj(vm.types.frozenset.clone());
    let l = vm.types.new_list(items);
    let f = vm.call(&fs, vec![l], Vec::new())?;
    ops::setitem(vm, &ip, py::s(vm, &format!("{prefix}{prop}")), f)
}

fn f_update_incompatible_props(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = py::string(vm, &arg_or(args, kw, 0, "prefix", Value::None))?;
    let n = py::string(vm, &arg_or(args, kw, 1, "prop", Value::None))?;
    update_incompatible(vm, &p, &n)?;
    Ok(Value::None)
}

/// `check_incompatible_props(new, olds)`: the first of `olds` that sets a
/// property `new` also sets, or False.
fn f_check_incompatible_props(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let new = arg_or(args, kw, 0, "new", Value::None);
    let olds = arg_or(args, kw, 1, "olds", Value::None);
    let ip = mget(vm, "incompatible_props")?;
    let ipo = ip.obj().unwrap().clone();
    let mut newly: Vec<Value> = match ops::dict_get(vm, &ipo, &new)? {
        Some(v) => py::items(vm, &v)?,
        None => Vec::new(),
    };
    newly.push(new);
    for old in py::items(vm, &olds)? {
        let sets: Vec<Value> = match ops::dict_get(vm, &ipo, &old)? {
            Some(v) => py::items(vm, &v)?,
            None => vec![old.clone()],
        };
        for a in &newly {
            for b in &sets {
                if ops::eq(vm, a, b)? {
                    return Ok(old);
                }
            }
        }
    }
    Ok(Value::Bool(false))
}

// Parser -----------------------------------------------------------------------------------------------

/// `Parser.__init__(name, statement/child_statement=True)`.
fn parser_setup(vm: &mut Vm, this: &Value, name: Value, statement: bool) -> PyResult<()> {
    let f = Feat::of(vm);
    py::set(vm, this, "name", name.clone())?;
    py::set(vm, this, "positional", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "keyword", vm.types.new_dict())?;
    py::set(vm, this, "children", vm.types.new_dict())?;
    if f.0 >= (7, 3) {
        py::set(vm, this, "variable", Value::Bool(false))?;
    }
    if f.child_statements() {
        if f.statement_names() {
            let s = mget(vm, "statement_names")?;
            ops::set_add(vm, &s, name.clone())?;
        } else {
            let s = mget(vm, "statements")?;
            ops::setitem(vm, &s, name, this.clone())?;
        }
        if statement {
            let l = mget(vm, "all_child_statements")?;
            ops::list_append(vm, &l, this.clone())?;
        }
    } else if statement {
        let l = mget(vm, "all_statements")?;
        ops::list_append(vm, &l, this.clone())?;
    }
    mset(vm, "parser", this.clone())
}

fn statement_flag(vm: &mut Vm, args: &[Value], kw: Kw, i: usize) -> PyResult<bool> {
    let name = if Feat::of(vm).child_statements() {
        "child_statement"
    } else {
        "statement"
    };
    py::truthy(vm, &arg_or(args, kw, i, name, Value::Bool(true)))
}

fn parser_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    let st = statement_flag(vm, args, kw, 2)?;
    parser_setup(vm, &args[0], name, st)?;
    Ok(Value::None)
}

fn parser_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ops::type_name(vm, &args[0]);
    let n = py::get_or_none(vm, &args[0], "name")?;
    let n = py::string(vm, &n)?;
    Ok(py::s(vm, &format!("<{t}: {n}>")))
}

fn parser_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let i = &args[1];
    if matches!(i.payload(), Some(crysirror_python::Payload::List(_))) {
        for j in py::items(vm, i)? {
            vm.call_method(this, "add", vec![j])?;
        }
        return Ok(Value::None);
    }
    if is_a(vm, i, "Positional")? {
        let pos = py::get(vm, this, "positional")?;
        ops::list_append(vm, &pos, i.clone())?;
        let n = py::get(vm, i, "name")?;
        let n = py::string(vm, &n)?;
        register_positional(vm, this, n)?;
    } else if is_a(vm, i, "Keyword")? {
        let k = py::get(vm, this, "keyword")?;
        let n = py::get(vm, i, "name")?;
        ops::setitem(vm, &k, n, i.clone())?;
    } else if is_a(vm, i, "Style")? {
        let k = py::get(vm, this, "keyword")?;
        let n = py::get(vm, i, "name")?;
        let n = py::string(vm, &n)?;
        for j in STYLE_PREFIXES {
            ops::setitem(vm, &k, py::s(vm, &format!("{j}{n}")), i.clone())?;
        }
    } else if is_a(vm, i, "PrefixStyle")? {
        let k = py::get(vm, this, "keyword")?;
        let p = py::get(vm, i, "prefix")?;
        let p = py::string(vm, &p)?;
        let n = py::get(vm, i, "name")?;
        let n = py::string(vm, &n)?;
        for j in STYLE_PREFIXES {
            ops::setitem(vm, &k, py::s(vm, &format!("{p}{j}{n}")), i.clone())?;
        }
    } else if is_a(vm, i, "Parser")? {
        let c = py::get(vm, this, "children")?;
        let n = py::get(vm, i, "name")?;
        ops::setitem(vm, &c, n, i.clone())?;
    }
    Ok(Value::None)
}

/// `Parser.parse` and friends: screens are parsed by `parse_screen`.
fn parser_parse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ops::type_name(vm, &args[0]);
    if t == "Parser" {
        return Err(py::error(vm, "Not Implemented"));
    }
    Err(py::error(
        vm,
        format!(
            "{t}.parse: screen-language statements are parsed by the native screen \
             parser; parse whole screens with renpy.sl2.slparser.parse_screen(l, loc)."
        ),
    ))
}

fn parser_parse_layout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get_or_none(vm, &args[0], "name")?;
    let n = py::string(vm, &n)?;
    let l = args.get(2).cloned().unwrap_or(Value::None);
    let msg = format!("The {n} statement cannot be used as a container for the has statement.");
    if !py::is_none(&l) {
        vm.call_method(&l, "error", vec![py::s(vm, &msg)])?;
    }
    Err(py::error(vm, msg))
}

/// `get_keyword_error(name)` (8.6): the message for an unknown property.
fn parser_get_keyword_error(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::get_or_none(vm, &args[0], "name")?;
    let n = py::string(vm, &n)?;
    let r = ops::repr(vm, &args[1])?;
    let mut candidates = Vec::new();
    for k in ["keyword", "children"] {
        let d = py::get(vm, &args[0], k)?;
        candidates.extend(py::items(vm, &d)?);
    }
    let mut rv = format!("{r} is not a keyword argument or valid child of the {n} statement.");
    if let Ok(f) = py::import_attr(vm, "renpy.error", "compute_closest_value") {
        let l = vm.types.new_list(candidates);
        let s = vm.call(&f, vec![args[1].clone(), l], Vec::new())?;
        if py::truthy(vm, &s)? {
            let sr = ops::repr(vm, &s)?;
            rv.push_str(&format!(" Did you mean: {sr}?"));
        }
    }
    Ok(py::s(vm, &rv))
}

fn add_clause(vm: &mut Vm, cls: &str, args: Vec<Value>) -> PyResult<Value> {
    let c = mget(vm, cls)?;
    vm.call(&c, args, Vec::new())
}

/// The `add_*` methods attach to the global `parser`, which is this
/// statement only while it is the newest one (upstream semantics).
fn p_add_positional(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_clause(vm, "Positional", vec![args[1].clone()])?;
    Ok(args[0].clone())
}

fn p_add_property(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_clause(vm, "Keyword", vec![args[1].clone()])?;
    Ok(args[0].clone())
}

fn p_add_style_property(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_clause(vm, "Style", vec![args[1].clone()])?;
    Ok(args[0].clone())
}

fn p_add_prefix_style_property(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_clause(vm, "PrefixStyle", vec![args[1].clone(), args[2].clone()])?;
    Ok(args[0].clone())
}

fn p_add_property_group(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let group = arg_or(args, kw, 1, "group", Value::None);
    let prefix = arg_or(args, kw, 2, "prefix", py::s(vm, ""));
    let groups = py::import_attr(vm, PROPS, "property_groups")?;
    if !ops::contains(vm, &groups, &group)? {
        let r = ops::repr(vm, &group)?;
        return Err(py::error(vm, format!("{r} is not a known property group.")));
    }
    let p = py::string(vm, &prefix)?;
    let g = ops::getitem(vm, &groups, group)?;
    for prop in py::items(vm, &g)? {
        let n = py::get(vm, &prop, "name")?;
        let n = py::string(vm, &n)?;
        if is_a(vm, &prop, "Keyword")? {
            add_clause(vm, "Keyword", vec![py::s(vm, &format!("{p}{n}"))])?;
        } else {
            add_clause(vm, "PrefixStyle", vec![prefix.clone(), py::s(vm, &n)])?;
        }
    }
    Ok(args[0].clone())
}

fn known_statement(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let found = if Feat::of(vm).statements_dict() {
        let s = mget(vm, "statements")?;
        ops::dict_get(vm, s.obj().unwrap(), name)?
    } else {
        let key = if Feat::of(vm).child_statements() {
            "all_child_statements"
        } else {
            "all_statements"
        };
        let l = mget(vm, key)?;
        let mut f = None;
        for p in py::items(vm, &l)? {
            let n = py::get(vm, &p, "name")?;
            if ops::eq(vm, &n, name)? {
                f = Some(p);
            }
        }
        f
    };
    match found {
        Some(p) => Ok(p),
        None => {
            let r = ops::repr(vm, name)?;
            Err(py::error(vm, format!("{r} is not a known screen statement")))
        }
    }
}

fn p_copy_positionals(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = known_statement(vm, &args[1])?;
    for pos in py::get_items(vm, &p, "positional")? {
        let n = py::get(vm, &pos, "name")?;
        add_clause(vm, "Positional", vec![n])?;
    }
    Ok(Value::None)
}

fn p_copy_keywords(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = known_statement(vm, &args[1])?;
    let kd = py::get(vm, &p, "keyword")?;
    let mut seen: Vec<Value> = Vec::new();
    for (_, v) in ops::mapping_items(vm, &kd)? {
        if seen.iter().any(|s| py::same(s, &v)) {
            continue;
        }
        seen.push(v.clone());
        let n = py::get(vm, &v, "name")?;
        if is_a(vm, &v, "Keyword")? {
            add_clause(vm, "Keyword", vec![n])?;
        } else if is_a(vm, &v, "Style")? {
            add_clause(vm, "Style", vec![n])?;
        } else if is_a(vm, &v, "PrefixStyle")? {
            let pr = py::get(vm, &v, "prefix")?;
            add_clause(vm, "PrefixStyle", vec![pr, n])?;
        }
    }
    Ok(Value::None)
}

fn p_copy_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    p_copy_positionals(vm, args, &[])?;
    p_copy_keywords(vm, args, &[])?;
    Ok(Value::None)
}

// Statement parsers ------------------------------------------------------------------------------------

fn childbearing(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let cb = mget(vm, "childbearing_statements")?;
    ops::set_add(vm, &cb, this.clone())
}

fn is_zero(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    ops::eq(vm, v, &Value::Int(0))
}

/// `DisplayableParser(name, displayable, style, nchildren=0, scope=False,
/// [text_style=None (6.18),] pass_context=False, imagemap=False,
/// replaces=False, default_keywords={}, hotspot=False,
/// default_properties=True, unique=False)`.
fn dp_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = Feat::of(vm);
    let names: &[&str] = if f.0 < (6, 99) {
        &[
            "name",
            "displayable",
            "style",
            "nchildren",
            "scope",
            "text_style",
            "pass_context",
            "imagemap",
            "replaces",
            "default_keywords",
        ]
    } else {
        &[
            "name",
            "displayable",
            "style",
            "nchildren",
            "scope",
            "pass_context",
            "imagemap",
            "replaces",
            "default_keywords",
            "hotspot",
            "default_properties",
            "unique",
        ]
    };
    let get = |vm: &mut Vm, n: &str, d: Value| -> Value {
        let _ = vm;
        match names.iter().position(|x| *x == n) {
            Some(i) => arg_or(&args[1..], kw, i, n, d),
            None => d,
        }
    };
    let name = get(vm, "name", Value::None);
    parser_setup(vm, this, name.clone(), true)?;
    let displayable = get(vm, "displayable", Value::None);
    py::set(vm, this, "displayable", displayable.clone())?;
    let mut nch = get(vm, "nchildren", Value::Int(0));
    if f.0 >= (6, 99) && nch.as_str() == Some("many") {
        nch = mget(vm, "many")?;
    }
    py::set(vm, this, "nchildren", nch.clone())?;
    if !is_zero(vm, &nch)? {
        childbearing(vm, this)?;
    }
    let style = get(vm, "style", Value::None);
    py::set(vm, this, "style", style.clone())?;
    let mut flags = Vec::new();
    for n in [
        "scope",
        "pass_context",
        "imagemap",
        "hotspot",
        "replaces",
        "unique",
    ] {
        if f.0 < (6, 99) && matches!(n, "hotspot" | "unique") {
            continue;
        }
        if n == "unique" && f.0 < (7, 5) {
            continue;
        }
        let v = get(vm, n, Value::Bool(false));
        py::set(vm, this, n, v.clone())?;
        flags.push((n, py::truthy(vm, &v)?));
    }
    if f.0 < (6, 99) {
        let ts = get(vm, "text_style", Value::None);
        py::set(vm, this, "text_style", ts)?;
    }
    let dk = get(vm, "default_keywords", Value::None);
    let dk = if py::is_none(&dk) {
        vm.types.new_dict()
    } else {
        dk
    };
    py::set(vm, this, "default_keywords", dk.clone())?;
    if f.0 >= (7, 4) {
        py::set(vm, this, "variable", Value::Bool(true))?;
    }
    if f.0 >= (6, 99) {
        add_clause(vm, "Keyword", vec![py::s(vm, "arguments")])?;
        add_clause(vm, "Keyword", vec![py::s(vm, "properties")])?;
        if f.child_statements() {
            add_clause(vm, "Keyword", vec![py::s(vm, "prefer_screen_to_id")])?;
        }
        let dp = get(vm, "default_properties", Value::Bool(true));
        if py::truthy(vm, &dp)? {
            for g in ["ui_properties", "position_properties"] {
                let l = py::import_attr(vm, PROPS, g)?;
                f_add(vm, &[l], &[])?;
            }
        }
    }
    // Statements created after start-up also extend the parser registry.
    let builtin = mget(vm, "_crysirror_builtin")?;
    let builtin = py::truthy(vm, &builtin)?;
    if !builtin {
        let flag = |n: &str| flags.iter().any(|(k, v)| *k == n && *v);
        let mut default_keywords = Vec::new();
        for (k, val) in ops::mapping_items(vm, &dk)? {
            let r = ops::repr(vm, &val)?;
            default_keywords.push((py::string(vm, &k)?, r));
        }
        // `keywords.pop("style_suffix", None) or self.style`: a false style is none.
        let style = if py::truthy(vm, &style)? {
            Some(py::string(vm, &style)?)
        } else {
            None
        };
        let spec = StatementSpec {
            name: py::string(vm, &name)?,
            displayable: dotted_name(vm, &displayable)?,
            style,
            nchildren: nchildren(vm, &nch)?,
            positional: Vec::new(),
            scope: flag("scope"),
            pass_context: flag("pass_context"),
            imagemap: flag("imagemap"),
            hotspot: flag("hotspot"),
            replaces: flag("replaces"),
            unique: flag("unique"),
            default_keywords,
            custom_screen: None,
        };
        register(vm, spec);
        py::set(vm, this, "_crysirror_registered", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn if_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let name = arg_or(args, kw, 1, "name", Value::None);
    parser_setup(vm, this, name, true)?;
    let nt = arg_or(args, kw, 2, "node_type", Value::None);
    let pc = arg_or(args, kw, 3, "parent_contents", Value::Bool(false));
    py::set(vm, this, "node_type", nt)?;
    py::set(vm, this, "parent_contents", pc.clone())?;
    if !py::truthy(vm, &pc)? {
        childbearing(vm, this)?;
    }
    Ok(Value::None)
}

fn childbearing_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 1, "name", Value::None);
    parser_setup(vm, &args[0], name, true)?;
    childbearing(vm, &args[0])?;
    Ok(Value::None)
}

fn screen_parser_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let st = !Feat::of(vm).screen_not_statement();
    parser_setup(vm, &args[0], py::s(vm, "screen"), st)?;
    Ok(Value::None)
}

/// `CustomParser(name, [positional=0 (before 7.4),] children="many", screen=None)`.
fn custom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = Feat::of(vm);
    let name = arg_or(args, kw, 1, "name", Value::None);
    let (positional, ci, si) = if f.custom_positional() {
        let p = arg_or(args, kw, 2, "positional", Value::Int(0));
        (Some(p), 3, 4)
    } else {
        (None, 2, 3)
    };
    let mut children = arg_or(args, kw, ci, "children", py::s(vm, "many"));
    let screen = arg_or(args, kw, si, "screen", Value::None);
    parser_setup(vm, this, name.clone(), true)?;
    let nch = nchildren(vm, &children)?;
    if children.as_str() == Some("many") {
        children = mget(vm, "many")?;
    }
    let all = if f.child_statements() {
        "all_child_statements"
    } else {
        "all_statements"
    };
    let cb = mget(vm, "childbearing_statements")?;
    for i in py::items(vm, &cb)? {
        vm.call_method(&i, "add", vec![this.clone()])?;
    }
    let sp = mget(vm, "screen_parser")?;
    vm.call_method(&sp, "add", vec![this.clone()])?;
    py::set(vm, this, "nchildren", children.clone())?;
    if !is_zero(vm, &children)? {
        childbearing(vm, this)?;
        let l = mget(vm, all)?;
        for i in py::items(vm, &l)? {
            vm.call_method(this, "add", vec![i])?;
        }
    }
    let screen_name = if py::is_none(&screen) {
        name.clone()
    } else {
        screen
    };
    let n = py::string(vm, &name)?;
    let mut pos_names = Vec::new();
    if let Some(p) = &positional {
        py::set(vm, this, "positional", p.clone())?;
        for i in 0..py::int(vm, p)?.max(0) {
            pos_names.push(format!("_positional{i}"));
        }
    }
    let spec = StatementSpec {
        name: n.clone(),
        displayable: String::new(),
        style: None,
        nchildren: nch,
        positional: pos_names,
        scope: false,
        pass_context: false,
        imagemap: false,
        hotspot: false,
        replaces: false,
        unique: false,
        default_keywords: Vec::new(),
        custom_screen: Some(py::string(vm, &screen_name)?),
    };
    register(vm, spec);
    py::set(vm, this, "_crysirror_registered", Value::Bool(true))?;
    if f.0 >= (7, 0) {
        add_clause(vm, "Keyword", vec![py::s(vm, "arguments")])?;
        add_clause(vm, "Keyword", vec![py::s(vm, "properties")])?;
    }
    let ifs = mget(vm, "if_statement")?;
    vm.call_method(this, "add", vec![ifs])?;
    if f.pass_statement() {
        let ps = mget(vm, "pass_statement")?;
        vm.call_method(this, "add", vec![ps])?;
    }
    mset(vm, "parser", Value::None)?;
    py::set(vm, this, "screen", screen_name)?;
    if f.0 >= (7, 4) {
        py::set(vm, this, "variable", Value::Bool(true))?;
    }
    Ok(Value::None)
}

// Module functions -------------------------------------------------------------------------------------

fn f_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = mget(vm, "parser")?;
    vm.call_method(&p, "add", vec![args[0].clone()])?;
    Ok(Value::None)
}

/// `register_sl_displayable(*args, **kwargs)`.
fn f_register_sl_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = mget(vm, "DisplayableParser")?;
    let rv = vm.call(&c, args.to_vec(), kw.to_vec())?;
    let cb = mget(vm, "childbearing_statements")?;
    for i in py::items(vm, &cb)? {
        vm.call_method(&i, "add", vec![rv.clone()])?;
    }
    let sp = mget(vm, "screen_parser")?;
    vm.call_method(&sp, "add", vec![rv.clone()])?;
    let n = py::get(vm, &rv, "nchildren")?;
    if !is_zero(vm, &n)? {
        childbearing(vm, &rv)?;
        let all = if Feat::of(vm).child_statements() {
            "all_child_statements"
        } else {
            "all_statements"
        };
        let l = mget(vm, all)?;
        for i in py::items(vm, &l)? {
            vm.call_method(&rv, "add", vec![i])?;
        }
    }
    Ok(rv)
}

/// `register_sl_statement(...)`: `CustomParser`.
fn f_register_sl_statement(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = mget(vm, "CustomParser")?;
    vm.call(&c, args.to_vec(), kw.to_vec())
}

fn f_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let f = Feat::of(vm);
    let all = if f.child_statements() {
        mget(vm, "all_child_statements")?
    } else {
        mget(vm, "all_statements")?
    };
    let sp = mget(vm, "screen_parser")?;
    vm.call_method(&sp, "add", vec![all.clone()])?;
    let cb = mget(vm, "childbearing_statements")?;
    let ifs = mget(vm, "if_statement")?;
    for i in py::items(vm, &all)? {
        if ops::set_contains(vm, &cb, &i)? {
            vm.call_method(&i, "add", vec![all.clone()])?;
        } else {
            vm.call_method(&i, "add", vec![ifs.clone()])?;
            if f.child_statements() {
                let ps = mget(vm, "pass_statement")?;
                vm.call_method(&i, "add", vec![ps])?;
            }
        }
    }
    Ok(Value::None)
}

/// `parse_screen(l, loc)`: the Rust screen parser over the lexer behind a
/// Python `Lexer`; returns the `SLScreen`.
fn f_parse_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let l = arg_or(args, kw, 0, "l", Value::None);
    let loc = arg_or(args, kw, 1, "loc", Value::None);
    let st = super::lexer::state(vm, &l)?;
    let loc: crate::sl2::Loc = if py::is_none(&loc) {
        st.borrow().location()
    } else {
        let f = ops::getitem(vm, &loc, Value::Int(0))?;
        let n = ops::getitem(vm, &loc, Value::Int(1))?;
        (py::string(vm, &f)?.into(), py::int(vm, &n)? as u32)
    };
    let cx = super::statements::ParseCx::new(vm);
    let screen = {
        let mut arena = crate::ast::Arena::default();
        let mut lex = st.borrow_mut();
        let reg = crate::parser::ScriptContext::sl2_registry(&cx).clone();
        let mut pa = |ll: &mut crate::lexer::Lexer| {
            crate::parser::Parser::new(&mut arena, &cx).parse_parameters(ll)
        };
        reg.parse_screen(&mut lex, &loc, &cx, &mut pa)
    };
    let errors = cx.finish(vm);
    super::statements::record_errors(vm, errors)?;
    let screen = screen.map_err(|e| super::lexer::parse_error(vm, &e))?;
    super::slast::screen_value(vm, Rc::new(screen))
}

// Table replay -----------------------------------------------------------------------------------------

/// Creates the clause object of `op` (attached to the current parser).
fn replay_clause(vm: &mut Vm, op: &Op) -> PyResult<Vec<Value>> {
    Ok(match op {
        Op::K(n) => vec![add_clause(vm, "Keyword", vec![py::s(vm, n)])?],
        Op::S(n) => vec![add_clause(vm, "Style", vec![py::s(vm, n)])?],
        Op::PS(p, n) => vec![add_clause(
            vm,
            "PrefixStyle",
            vec![py::s(vm, p), py::s(vm, n)],
        )?],
        Op::Pos(n) => vec![add_clause(vm, "Positional", vec![py::s(vm, n)])?],
        Op::Atl => {
            let props = py::import_attr(vm, "renpy.atl", "PROPERTIES")?;
            let mut v = Vec::new();
            for k in py::items(vm, &props)? {
                v.push(add_clause(vm, "Style", vec![k])?);
            }
            v
        }
        Op::Add(_) | Op::P(_) => Vec::new(),
    })
}

/// Builds a module-level list of the property tables.
pub fn list_value(vm: &mut Vm, def: &ListDef, lists: &dyn Fn(&str) -> Option<Value>) -> PyResult<Value> {
    Ok(match def {
        ListDef::Names(n) => str_list(vm, n),
        ListDef::Objs(ops) => {
            let mut v = Vec::new();
            for o in ops.iter() {
                v.extend(replay_clause(vm, o)?);
            }
            vm.types.new_list(v)
        }
        ListDef::Groups(g) => {
            let d = vm.types.new_dict();
            for (k, l) in g.iter() {
                let lv = lists(l).unwrap_or_else(|| vm.types.new_list(Vec::new()));
                py::dict_set(vm, &d, k, lv)?;
            }
            d
        }
    })
}

fn resolve_dotted(vm: &mut Vm, dotted: &str) -> Value {
    if let Some((m, n)) = dotted.rsplit_once('.') {
        if let Ok(v) = py::import_attr(vm, m, n) {
            return v;
        }
    }
    // Callables of modules this runtime does not provide (for example the
    // 8.6 IME candidate placer) keep their dotted name.
    vm.types.new_str(dotted)
}

/// Replays `DisplayableParser(...)`/clause/`add` calls of a statement table
/// into the module registries without changing the parse registry.
pub fn replay_statements(vm: &mut Vm, ops_: &[Op], lists: &dyn Fn(&mut Vm, &str) -> PyResult<Value>) -> PyResult<()> {
    let old = mget(vm, "_crysirror_builtin")?;
    mset(vm, "_crysirror_builtin", Value::Bool(true))?;
    let r = (|| -> PyResult<()> {
        for o in ops_ {
            match o {
                Op::P(s) => replay_parser(vm, s)?,
                Op::Add(l) => {
                    let v = lists(vm, l)?;
                    f_add(vm, &[v], &[])?;
                }
                other => {
                    replay_clause(vm, other)?;
                }
            }
        }
        Ok(())
    })();
    mset(vm, "_crysirror_builtin", old)?;
    r?;
    mset(vm, "parser", Value::None)
}

fn replay_parser(vm: &mut Vm, s: &Stmt) -> PyResult<()> {
    let disp = resolve_dotted(vm, s.displayable);
    let style = match s.style {
        Some(x) => py::s(vm, x),
        None => Value::None,
    };
    let nch = match s.nchildren {
        -1 => mget(vm, "many")?,
        n => Value::Int(n as i64),
    };
    let mut kw: Vec<(Rc<str>, Value)> = Vec::new();
    let b = Value::Bool;
    if s.scope {
        kw.push(("scope".into(), b(true)));
    }
    if s.pass_context {
        kw.push(("pass_context".into(), b(true)));
    }
    if s.imagemap {
        kw.push(("imagemap".into(), b(true)));
    }
    if s.replaces {
        kw.push(("replaces".into(), b(true)));
    }
    if s.hotspot {
        kw.push(("hotspot".into(), b(true)));
    }
    if !s.default_properties {
        kw.push(("default_properties".into(), b(false)));
    }
    if s.unique {
        kw.push(("unique".into(), b(true)));
    }
    if !s.default_keywords.is_empty() {
        let d = vm.types.new_dict();
        for (k, v) in s.default_keywords {
            py::dict_set(vm, &d, k, py::s(vm, v))?;
        }
        kw.push(("default_keywords".into(), d));
    }
    let c = mget(vm, "DisplayableParser")?;
    vm.call(&c, vec![py::s(vm, s.name), disp, style, nch], kw)?;
    Ok(())
}

// Modules ----------------------------------------------------------------------------------------------

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let f = Feat::of(vm);
    let mut funcs: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("add", f_add),
        ("init", f_init),
        ("parse_screen", f_parse_screen),
    ];
    if f.custom() {
        funcs.push(("register_sl_displayable", f_register_sl_displayable));
        funcs.push(("register_sl_statement", f_register_sl_statement));
    }
    if f.child_statements() {
        funcs.push(("check_incompatible_props", f_check_incompatible_props));
    }
    if f.proxy() {
        funcs.push(("update_incompatible_props", f_update_incompatible_props));
    }
    let m = crysirror_python::modules::module_with(vm, MODULE, &funcs);
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "STYLE_PREFIXES", str_list(vm, &STYLE_PREFIXES));
    set(vm, "parser", Value::None);
    set(vm, "_crysirror_builtin", Value::Bool(false));
    if f.child_statements() {
        if f.statement_names() {
            set(vm, "statement_names", vm.types.new_set(Default::default()));
        } else {
            set(vm, "statements", vm.types.new_dict());
        }
        set(vm, "all_child_statements", vm.types.new_list(Vec::new()));
    } else {
        set(vm, "all_statements", vm.types.new_list(Vec::new()));
    }
    set(
        vm,
        "childbearing_statements",
        vm.types.new_set(Default::default()),
    );
    if f.all_keyword_names() {
        set(vm, "all_keyword_names", vm.types.new_set(Default::default()));
    } else {
        let dd = py::import_attr(vm, "collections", "defaultdict")?;
        let st = Value::Obj(vm.types.set.clone());
        let props = vm.call(&dd, vec![st], Vec::new())?;
        set(vm, "properties", props);
    }
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let many = vm.call(&sentinel, vec![py::s(vm, "many")], Vec::new())?;
    set(vm, "many", many);
    for (n, init) in [
        ("Positional", positional_init as crysirror_python::object::NativeFn),
        ("Keyword", keyword_init),
        ("Style", style_init),
        ("PrefixStyle", prefix_style_init),
    ] {
        let c = py::class(vm, MODULE, n, &[], &[("__init__", init)], &[])?;
        set(vm, n, Value::Obj(c));
    }
    if f.child_statements() {
        let pp = py::import_attr(vm, "renpy.styledata.stylesets", "proxy_properties")?;
        if f.proxy() {
            set(vm, "proxy_properties", pp.clone());
            let ip = vm.call_method(&pp, "copy", Vec::new())?;
            set(vm, "incompatible_props", ip);
        } else {
            // 7.6 – 8.3: `proxy_properties as incompatible_props`.
            set(vm, "incompatible_props", pp);
        }
    }
    let mut methods: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("__init__", parser_init),
        ("__repr__", parser_repr),
        ("add", parser_add),
        ("parse_statement", parser_parse),
        ("parse_layout", parser_parse_layout),
        ("parse", parser_parse),
        ("parse_contents", parser_parse),
    ];
    if f.0 >= (6, 99) {
        methods.extend([
            ("add_positional", p_add_positional as crysirror_python::object::NativeFn),
            ("add_property", p_add_property),
            ("add_style_property", p_add_style_property),
            ("add_prefix_style_property", p_add_prefix_style_property),
            ("add_property_group", p_add_property_group),
        ]);
    }
    if f.copy_properties() {
        methods.extend([
            ("copy_positionals", p_copy_positionals as crysirror_python::object::NativeFn),
            ("copy_keywords", p_copy_keywords),
            ("copy_properties", p_copy_properties),
        ]);
    }
    if f.0 >= (8, 6) {
        methods.push(("get_keyword_error", parser_get_keyword_error));
    }
    let nch_attr: Vec<(&str, Value)> = if f.0 >= (6, 99) {
        vec![("nchildren", py::s(vm, "many"))]
    } else {
        Vec::new()
    };
    let base = Value::Obj(py::class(vm, MODULE, "Parser", &[], &methods, &nch_attr)?);
    set(vm, "Parser", base.clone());
    let sub = |vm: &mut Vm, n: &str, init: Option<crysirror_python::object::NativeFn>| -> PyResult<Value> {
        let ms: Vec<(&'static str, crysirror_python::object::NativeFn)> = match init {
            Some(i) => vec![("__init__", i)],
            None => Vec::new(),
        };
        let c = Value::Obj(py::class(vm, MODULE, n, &[base.clone()], &ms, &[])?);
        crysirror_python::modules::set(vm, &m, n, c.clone());
        Ok(c)
    };
    sub(vm, "DisplayableParser", Some(dp_init))?;
    let ifp = sub(vm, "IfParser", Some(if_init))?;
    let forp = sub(vm, "ForParser", Some(childbearing_init))?;
    let olp = sub(vm, "OneLinePythonParser", None)?;
    let mlp = sub(vm, "MultiLinePythonParser", None)?;
    let passp = sub(vm, "PassParser", None)?;
    let defp = sub(vm, "DefaultParser", None)?;
    let usep = sub(vm, "UseParser", Some(childbearing_init))?;
    let breakp = if f.child_statements() {
        Some((sub(vm, "BreakParser", None)?, sub(vm, "ContinueParser", None)?))
    } else {
        None
    };
    let transp = if f.custom() {
        sub(vm, "CustomParser", Some(custom_init))?;
        Some(sub(vm, "TranscludeParser", None)?)
    } else {
        None
    };
    let screenp = sub(vm, "ScreenParser", Some(screen_parser_init))?;
    // Module statements, as slparser.py creates them.
    let slast = py::import(vm, "renpy.sl2.slast")?;
    let sl_if = py::get(vm, &slast, "SLIf")?;
    let sl_showif = py::get(vm, &slast, "SLShowIf")?;
    let s = |vm: &Vm, x: &str| vm.types.new_str(x);
    let ifs = vm.call(&ifp, vec![s(vm, "if"), sl_if, Value::Bool(true)], Vec::new())?;
    set(vm, "if_statement", ifs);
    vm.call(&ifp, vec![s(vm, "showif"), sl_showif, Value::Bool(false)], Vec::new())?;
    let fp = vm.call(&forp, vec![s(vm, "for")], Vec::new())?;
    if let Some((bp, cp)) = breakp {
        set(vm, "for_parser", fp.clone());
        let b = vm.call(&bp, vec![s(vm, "break"), Value::Bool(false)], Vec::new())?;
        vm.call_method(&fp, "add", vec![b])?;
        let c = vm.call(&cp, vec![s(vm, "continue"), Value::Bool(false)], Vec::new())?;
        vm.call_method(&fp, "add", vec![c])?;
    }
    vm.call(&olp, vec![s(vm, "$")], Vec::new())?;
    vm.call(&mlp, vec![s(vm, "python")], Vec::new())?;
    let ps = vm.call(&passp, vec![s(vm, "pass")], Vec::new())?;
    if f.pass_statement() {
        set(vm, "pass_statement", ps);
    }
    vm.call(&defp, vec![s(vm, "default")], Vec::new())?;
    vm.call(&usep, vec![s(vm, "use")], Vec::new())?;
    if f.custom() {
        for k in ["style_prefix", "style_group"] {
            add_clause(vm, "Keyword", vec![s(vm, k)])?;
        }
    }
    if let Some(tp) = transp {
        vm.call(&tp, vec![s(vm, "transclude")], Vec::new())?;
    }
    let sp = vm.call(&screenp, Vec::new(), Vec::new())?;
    set(vm, "screen_parser", sp);
    let mut keys = vec!["modal", "zorder", "variant", "predict"];
    if f.custom() {
        keys.extend(["style_group", "style_prefix", "layer"]);
    }
    if f.0 >= (7, 4) {
        keys.push("sensitive");
    }
    if f.child_statements() {
        keys.push("roll_forward");
    }
    for k in keys {
        add_clause(vm, "Keyword", vec![s(vm, k)])?;
    }
    set(vm, "parser", Value::None);
    Ok(m)
}

fn slproperties_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, PROPS, &[]);
    let t = super::sl_tables::SL2[ref_tag(vm, &tags())].1;
    py::import(vm, MODULE)?;
    for n in ["Keyword", "Style", "PrefixStyle"] {
        let c = mget(vm, n)?;
        crysirror_python::modules::set(vm, &m, n, c);
    }
    let old = mget(vm, "parser")?;
    mset(vm, "parser", Value::None)?;
    // 6.18 has no slproperties.py; its lists live in sldisplayables.py and
    // are shared with this module.
    let defs = t.properties.unwrap_or(t.displayable_lists);
    let mo = Value::Obj(m.clone());
    let r = (|| -> PyResult<()> {
        for (n, d) in defs.iter() {
            let lookup = |k: &str| crysirror_python::ops::get_raw_attr(&m, k);
            let v = list_value(vm, d, &lookup)?;
            py::set(vm, &mo, n, v)?;
        }
        Ok(())
    })();
    mset(vm, "parser", old)?;
    r?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
    vm.register_native_module(PROPS, slproperties_module);
}
