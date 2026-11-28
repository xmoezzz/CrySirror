// SPDX-License-Identifier: MPL-2.0
//! Translation tooling, after Ren'Py 6.18 – 8.6 (MIT):
//! `renpy.translation.generation` (the `translate` command: `tl/` files of
//! dialogue and strings, filters), `renpy.translation.scanstrings`
//! (translatable string discovery and launcher priorities),
//! `renpy.translation.dialogue` (the `dialogue` command and the tutorial's
//! `create_dialogue_map`), `renpy.translation.extract` / `merge`
//! (`extract_strings` / `merge_strings`) and, for 6.18, the same tools as
//! they lived in `renpy.translation` itself.
//!
//! Script data comes from the Rust arena (node handles and
//! `renpy.game.script.translator`). File names follow upstream's absolute
//! form: a node in `game/script.rpy` belongs to `/game/script.rpy`
//! (`config.gamedir` is `/game`, `config.commondir` `/renpy/common`).
//! Every file the commands write goes through the interpreter's `open`,
//! which the runtime host maps to the game's scoped Save-mount overlay of
//! the base directory; nothing touches the host file system.
//!
//! The commands read their options with `renpy.arguments.ArgumentParser`
//! as upstream does. The player has no command line, so they are reachable
//! from game code; when the argument parser is unavailable the command
//! raises an exception naming it.
use super::{
    nodes,
    py::{self, *},
    state::rt,
};
use crate::ast::{NodeId, NodeKind};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const GEN: &str = "renpy.translation.generation";
const SCAN: &str = "renpy.translation.scanstrings";
const DIA: &str = "renpy.translation.dialogue";
const EXTRACT: &str = "renpy.translation.extract";
const MERGE: &str = "renpy.translation.merge";
const TL: &str = "renpy.translation";

type Native = crysirror_python::object::NativeFn;

fn at_least(vm: &Vm, a: u32, b: u32) -> bool {
    rt(vm).profile.at_least(a, b)
}

fn since_pair(vm: &Vm, a: u32, b: u32) -> bool {
    rt(vm).profile.since_pair(a, b)
}

fn translator(vm: &mut Vm) -> PyResult<Value> {
    let script = py::import_attr(vm, "renpy.game", "script")?;
    py::get(vm, &script, "translator")
}

fn call(vm: &mut Vm, module: &str, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, Vec::new())
}

fn opt_str(vm: &mut Vm, v: &Value) -> PyResult<Option<String>> {
    if py::is_none(v) { Ok(None) } else { Ok(Some(py::string(vm, v)?)) }
}

fn os_path(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    call(vm, "os.path", name, args)
}

fn join(vm: &mut Vm, parts: &[&str]) -> PyResult<String> {
    let a = parts.iter().map(|p| vm.types.new_str(p)).collect();
    let r = os_path(vm, "join", a)?;
    py::string(vm, &r)
}

/// `os.path.normpath`.
fn normpath(p: &str) -> String {
    let abs = p.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." if parts.last().is_some_and(|l| *l != "..") => {
                parts.pop();
            }
            ".." if abs => {}
            s => parts.push(s),
        }
    }
    let j = parts.join("/");
    match (abs, j.is_empty()) {
        (true, _) => format!("/{j}"),
        (false, true) => ".".into(),
        _ => j,
    }
}

fn config_dir(vm: &mut Vm, name: &str) -> PyResult<String> {
    let v = py::config(vm, name)?;
    Ok(normpath(&py::string(vm, &v)?))
}

/// The absolute name upstream gives a node's file.
pub fn node_path(filename: &str) -> String {
    if filename.starts_with('/') {
        normpath(filename)
    } else {
        normpath(&format!("/{filename}"))
    }
}

/// Contents of a game or common script file by its absolute name.
fn read_path(vm: &mut Vm, path: &str) -> PyResult<Option<String>> {
    let path = normpath(path);
    let common = config_dir(vm, "commondir")?;
    let game = config_dir(vm, "gamedir")?;
    let r = rt(vm);
    let data = if let Some(rel) = path.strip_prefix(&format!("{common}/")) {
        r.files.common_read(rel)
    } else if let Some(rel) = path.strip_prefix(&format!("{game}/")) {
        super::resloader::read(vm, rel).or_else(|| r.files.read(rel))
    } else {
        vm.host.read_file(&path)
    };
    Ok(data.map(|d| {
        String::from_utf8_lossy(&d).trim_start_matches('\u{feff}').replace("\r\n", "\n")
    }))
}

fn exists(vm: &mut Vm, path: &str) -> PyResult<bool> {
    let common = config_dir(vm, "commondir")?;
    let game = config_dir(vm, "gamedir")?;
    let p = normpath(path);
    if p.starts_with(&format!("{common}/")) || p.starts_with(&format!("{game}/")) {
        return Ok(read_path(vm, &p)?.is_some());
    }
    let r = os_path(vm, "exists", vec![py::s(vm, &p)])?;
    py::truthy(vm, &r)
}

/// `open(fn, mode, encoding="utf-8")` through the interpreter's scoped files.
fn open(vm: &mut Vm, path: &str, mode: &str) -> PyResult<Value> {
    let o = py::import_attr(vm, "builtins", "open")?;
    let a = vec![py::s(vm, path), py::s(vm, mode)];
    vm.call(&o, a, vec![("encoding".into(), py::s(vm, "utf-8"))])
}

fn write(vm: &mut Vm, f: &Value, s: &str) -> PyResult<()> {
    py::method(vm, f, "write", vec![py::s(vm, s)])?;
    Ok(())
}

fn close(vm: &mut Vm, f: &Value) -> PyResult<()> {
    py::method(vm, f, "close", Vec::new())?;
    Ok(())
}

fn quote_unicode(s: &str) -> String {
    super::translation::quote_unicode(s)
}

fn elide(vm: &mut Vm, f: &str) -> PyResult<String> {
    let r = call(vm, "renpy.lexer", "elide_filename", vec![py::s(vm, f)])?;
    py::string(vm, &r)
}

// Arena access ------------------------------------------------------------------------------

fn kind(vm: &Vm, id: NodeId) -> NodeKind {
    rt(vm).script.borrow().arena.get(id).kind.clone()
}

/// The say kind of a Say or TranslateSay node.
fn say_kind(k: &NodeKind) -> Option<&NodeKind> {
    match k {
        NodeKind::Say { .. } => Some(k),
        NodeKind::TranslateSay { say, .. } => Some(say),
        _ => None,
    }
}

/// `[t]` for a TranslateSay, else `t.block`.
fn tl_block(vm: &Vm, id: NodeId) -> Vec<NodeId> {
    match kind(vm, id) {
        NodeKind::Translate { block, .. } => block,
        NodeKind::TranslateSay { .. } => vec![id],
        _ => Vec::new(),
    }
}

/// `node.get_code(dialogue_filter)`.
fn get_code(vm: &mut Vm, id: NodeId, filter: Option<&Value>) -> PyResult<String> {
    let k = kind(vm, id);
    let k = match &k {
        NodeKind::TranslateSay { say, .. } => (**say).clone(),
        k => k.clone(),
    };
    let k = match (k, filter) {
        (NodeKind::Say { who, what, attributes, temporary_attributes, with_, interact, identifier, explicit_identifier, arguments }, Some(f)) => {
            let w = vm.call(f, vec![py::s(vm, &what)], Vec::new())?;
            let what = py::string(vm, &w)?;
            NodeKind::Say { who, what, attributes, temporary_attributes, with_, interact, identifier, explicit_identifier, arguments }
        }
        (k, _) => k,
    };
    match crate::translation::get_code(&k) {
        Some(c) => Ok(c),
        None => Err(py::error(vm, "Not Implemented")),
    }
}

fn node_loc(vm: &Vm, id: NodeId) -> (String, u32) {
    let r = rt(vm);
    let s = r.script.borrow();
    let n = s.arena.get(id);
    (n.filename.to_string(), n.linenumber)
}

fn tl_identity(vm: &Vm, id: NodeId) -> Option<(String, Option<String>, Option<String>)> {
    match kind(vm, id) {
        NodeKind::Translate { identifier, language, alternate, .. } => Some((identifier, language, alternate)),
        NodeKind::TranslateSay { identifier, language, .. } => Some((identifier, language, None)),
        _ => None,
    }
}

/// `translator.file_translates[filename]` as (label, node id).
fn file_translates(vm: &mut Vm, filename: &str) -> PyResult<Vec<(Value, NodeId)>> {
    let t = translator(vm)?;
    let ft = py::get(vm, &t, "file_translates")?;
    let l = ops::getitem(vm, &ft, py::s(vm, filename))?;
    let mut rv = Vec::new();
    for e in py::items(vm, &l)? {
        let p = py::items(vm, &e)?;
        if let Some(id) = nodes::id_of(&p[1]) {
            rv.push((p[0].clone(), id));
        }
    }
    Ok(rv)
}

fn has_language_translate(vm: &Vm, identifier: &str, language: Option<&str>) -> bool {
    let Some(l) = language else {
        return rt(vm).script.borrow().translates.contains_key(identifier);
    };
    rt(vm).script.borrow().language_translates.contains_key(&(identifier.to_owned(), l.to_owned()))
}

fn language_translate(vm: &Vm, identifier: &str, language: &str) -> Option<NodeId> {
    rt(vm).script.borrow().language_translates.get(&(identifier.to_owned(), language.to_owned())).copied()
}

// Command-line arguments -------------------------------------------------------------------

enum Arg {
    /// A positional argument.
    Pos(&'static str, &'static str),
    /// A store_true flag: flag, dest, help.
    Flag(&'static str, &'static str, &'static str),
    /// Any other option: flag, keyword arguments.
    Opt(&'static str, Vec<(&'static str, Value)>),
}

/// `ArgumentParser(description=...)`, its arguments and `parse_args()`.
fn parse_args(vm: &mut Vm, command: &str, description: &str, args: Vec<Arg>) -> PyResult<Value> {
    let ap = match py::import_attr(vm, "renpy.arguments", "ArgumentParser") {
        Ok(c) => c,
        Err(_) => {
            return Err(py::error(
                vm,
                format!("The {command} command needs renpy.arguments.ArgumentParser, which this runtime does not provide."),
            ));
        }
    };
    let ap = vm.call(&ap, Vec::new(), vec![("description".into(), py::s(vm, description))])?;
    for a in args {
        let (name, kw): (&str, Vec<(&str, Value)>) = match a {
            Arg::Pos(n, help) => (n, vec![("help", py::s(vm, help))]),
            Arg::Flag(f, dest, help) => (
                f,
                vec![("help", py::s(vm, help)), ("dest", py::s(vm, dest)), ("action", py::s(vm, "store_true"))],
            ),
            Arg::Opt(f, kw) => (f, kw),
        };
        let kw = kw.into_iter().map(|(k, v)| (k.into(), v)).collect();
        py::method_kw(vm, &ap, "add_argument", vec![py::s(vm, name)], kw)?;
    }
    py::method(vm, &ap, "parse_args", Vec::new())
}

fn arg_bool(vm: &mut Vm, ns: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, ns, n)?;
    py::truthy(vm, &v)
}

fn register_command(vm: &mut Vm, name: &str, f: Value) -> PyResult<()> {
    if let Ok(rc) = py::import_attr(vm, "renpy.arguments", "register_command") {
        vm.call(&rc, vec![py::s(vm, name), f], Vec::new())?;
    }
    Ok(())
}

// Filters --------------------------------------------------------------------------------------

fn f_null_filter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn f_empty_filter(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::s(vm, ""))
}

/// The ROT13 table; before 7.4 (and in 6.18) upstream zips "ABCDEFGHIJKLM"
/// with "NMOPQRSTUVWYZ", which is kept.
fn rot13_table(vm: &Vm) -> Vec<(char, char)> {
    let second = if at_least(vm, 7, 4) { "NOPQRSTUVWXYZ" } else { "NMOPQRSTUVWYZ" };
    let mut t = Vec::new();
    for (i, j) in "ABCDEFGHIJKLM".chars().zip(second.chars()) {
        t.push((i, j));
        t.push((j, i));
        let (li, lj) = (i.to_ascii_lowercase(), j.to_ascii_lowercase());
        t.push((li, lj));
        t.push((lj, li));
    }
    t
}

fn rot13(vm: &Vm, s: &str) -> String {
    let mut map = std::collections::HashMap::new();
    for (a, b) in rot13_table(vm) {
        map.insert(a, b);
    }
    s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect()
}

fn f_rot13_transform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(py::s(vm, &rot13(vm, &s)))
}

fn f_rot13_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !at_least(vm, 6, 99) {
        let s = py::string(vm, &args[0])?;
        let r = square_pass(&s, &|t| tag_pass_618_rot13(vm, t));
        return Ok(py::s(vm, &r));
    }
    let f = py::import_attr(vm, GEN, "rot13_transform")?;
    let s = py::string(vm, &args[0])?;
    let r = super::support::generic_filter(vm, &s, &f)?;
    Ok(py::s(vm, &r))
}

/// `piglatin_transform`: `re.sub(r"\w+", replace, s)`.
fn piglatin(s: &str) -> String {
    const LST: &[&str] = &["sh", "gl", "ch", "ph", "tr", "br", "fr", "bl", "gr", "st", "sl", "cl", "pl", "fl"];
    let mut out = String::new();
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut String| {
        if w.is_empty() {
            return;
        }
        let c: Vec<char> = w.chars().collect();
        let first = c[0];
        let two: String = c.iter().take(2).collect();
        let mut rv = if first.is_ascii_digit() {
            w.clone()
        } else if "aeiou".contains(first) {
            format!("{w}ay")
        } else if LST.contains(&two.as_str()) {
            format!("{}{two}ay", c[2..].iter().collect::<String>())
        } else {
            format!("{}{first}ay", c[1..].iter().collect::<String>())
        };
        if first.is_uppercase() {
            // str.capitalize(): first upper, rest lower.
            let mut ch = rv.chars();
            rv = match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &ch.as_str().to_lowercase(),
                None => rv,
            };
        }
        out.push_str(&rv);
        w.clear();
    };
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn f_piglatin_transform(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(py::s(vm, &piglatin(&s)))
}

fn f_piglatin_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    if s == "{#language name and font}" {
        return Ok(py::s(vm, "Igpay Atinlay"));
    }
    let f = py::import_attr(vm, GEN, "piglatin_transform")?;
    let rv = super::support::generic_filter(vm, &s, &f)?;
    // re.sub(r"\{\{(.*)?ay\}", r"{{\1}", rv)
    let sub = py::import_attr(vm, "re", "sub")?;
    let a = vec![py::s(vm, r"\{\{(.*)?ay\}"), py::s(vm, r"{{\1}"), py::s(vm, &rv)];
    vm.call(&sub, a, Vec::new())
}

/// The bracket pass shared by `notags_filter` and 6.18's `rot13_filter`.
fn square_pass(s: &str, tag_pass: &dyn Fn(&str) -> String) -> String {
    let mut squares = 0i64;
    let mut first = false;
    let mut rv = String::new();
    let mut buf = String::new();
    for i in s.chars() {
        if i == '[' {
            if first {
                squares = 0;
            } else {
                rv.push_str(&tag_pass(&buf));
                buf.clear();
                if squares == 0 {
                    first = true;
                }
                squares += 1;
            }
            rv.push('[');
        } else if i == ']' {
            first = false;
            squares -= 1;
            if squares < 0 {
                squares += 1;
            }
            rv.push(']');
        } else if squares != 0 {
            rv.push(i);
        } else {
            buf.push(i);
        }
    }
    if !buf.is_empty() {
        rv.push_str(&tag_pass(&buf));
    }
    rv
}

fn tag_pass_618_rot13(vm: &Vm, s: &str) -> String {
    let (mut brace, mut first) = (false, false);
    let mut rv = String::new();
    for i in s.chars() {
        if i == '{' {
            if first {
                brace = false;
            } else {
                brace = true;
                first = true;
            }
            rv.push('{');
        } else if i == '}' {
            first = false;
            brace = false;
            rv.push('}');
        } else {
            first = false;
            if brace {
                rv.push(i);
            } else {
                rv.push_str(&rot13(vm, &i.to_string()));
            }
        }
    }
    rv
}

/// `notags_filter`; from 7.6/8.1 a doubled `{{` survives and a stray `}`
/// is kept.
pub fn notags(s: &str, doubles: bool) -> String {
    let tag_pass = |s: &str| -> String {
        let (mut brace, mut first) = (false, false);
        let mut rv = String::new();
        for i in s.chars() {
            if i == '{' {
                if first {
                    brace = false;
                    if doubles {
                        first = false;
                        rv.push_str("{{");
                    }
                } else {
                    brace = true;
                    first = true;
                }
            } else if i == '}' {
                first = false;
                if brace {
                    brace = false;
                } else if doubles {
                    rv.push(i);
                }
            } else {
                first = false;
                if !brace {
                    rv.push(i);
                }
            }
        }
        rv
    };
    square_pass(s, &tag_pass)
}

fn f_notags_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let d = since_pair(vm, 6, 1);
    Ok(py::s(vm, &notags(&s, d)))
}

fn combine(vm: &mut Vm, s: &str) -> String {
    let mut doubles = vec!["{{", "%%"];
    if py::config_bool(vm, "lenticular_bracket_ruby").unwrap_or(false) {
        doubles.push("\u{3010}\u{3010}");
    }
    let mut s = s.to_owned();
    for d in doubles {
        while let Some(i) = s.find(d) {
            let c = d.chars().next().unwrap().len_utf8();
            s.replace_range(i..i + c, "");
        }
    }
    s
}

fn f_combine_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let r = combine(vm, &s);
    Ok(py::s(vm, &r))
}

fn f_what_filter(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(py::s(vm, "[what]"))
}

fn f_quote_unicode(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(py::s(vm, &quote_unicode(&s)))
}

fn f_elide_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    call(vm, "renpy.lexer", "elide_filename", args.to_vec())
}

// scanstrings ----------------------------------------------------------------------------------

const STRING_RE_699: &str = r#"(?x)
\b__?\s*\(\s*[uU]?(
\"\"\"(?:\\.|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\'{1,2}|[^\\'])*?'''
|"(?:\\.|[^\\"])*"
|'(?:\\.|[^\\'])*'
)\s*\)
"#;
const STRING_RE_70: &str = r#"(?x)
\b_[_p]?\s*\(\s*[uU]?(
\"\"\"(?:\\.|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\'{1,2}|[^\\'])*?'''
|"(?:\\.|[^\\"])*"
|'(?:\\.|[^\\'])*'
)\s*\)
"#;
const STRING_RE_73: &str = r#"(?x)
\b_[_p]?\s*\(\s*[uU]?(
\"\"\"(?:\\.|\\\n|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\\\n|\'{1,2}|[^\\'])*?'''
|"(?:\\.|[^\\"])*"
|'(?:\\.|[^\\'])*'
)\s*\)
"#;
const STRING_RE_74: &str = r#"(?x)
\b_[_p]?\s*\(\s*[uU]?(
\"\"\"(?:\\.|\\\n|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\\\n|\'{1,2}|[^\\'])*?'''
|"(?:\\.|\\\n|[^\\"])*"
|'(?:\\.|\\\n|[^\\'])*'
)\s*\)
"#;
const STRING_RE_78: &str = r#"(?x)
\b_[_p]?\s*(\((?:[\s\\\n]*[uU]?(?:
\"\"\"(?:\\.|\\\n|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\\\n|\'{1,2}|[^\\'])*?'''
|"(?:\\.|\\\n|[^\\"])*"
|'(?:\\.|\\\n|[^\\'])*'
))+\s*\))
"#;
const STRING_RE_84: &str = r#"(?x)
\b(?:_|__|___|_p)
\s*(\((?:[\s\\\n]*[uU]?(?:
\"\"\"(?:\\.|\\\n|\"{1,2}|[^\\"])*?\"\"\"
|'''(?:\\.|\\\n|\'{1,2}|[^\\'])*?'''
|"(?:\\.|\\\n|[^\\"])*"
|'(?:\\.|\\\n|[^\\'])*'
))+\s*\))
"#;

fn string_re(vm: &Vm) -> &'static str {
    let p = &rt(vm).profile;
    if p.at_least(8, 4) {
        STRING_RE_84
    } else if p.since_pair(8, 2) {
        STRING_RE_78
    } else if p.at_least(7, 4) {
        STRING_RE_74
    } else if p.at_least(7, 3) {
        STRING_RE_73
    } else if p.at_least(7, 0) {
        STRING_RE_70
    } else {
        STRING_RE_699
    }
}

const REGULAR_PRIORITIES: &[(&str, i64, &str)] = &[
    ("script.rpy", 5, "script.rpy"),
    ("options.rpy", 10, "options.rpy"),
    ("gui.rpy", 20, "gui.rpy"),
    ("screens.rpy", 30, "screens.rpy"),
    ("", 100, "launcher.rpy"),
];

const COMMON_PRIORITIES: &[(&str, i64, &str)] = &[
    ("_compat/", 420, "obsolete.rpy"),
    ("_layout/", 410, "obsolete.rpy"),
    ("00layout.rpy", 400, "obsolete.rpy"),
    ("00console.rpy", 320, "developer.rpy"),
    ("_developer/", 310, "developer.rpy"),
    ("_errorhandling.rpym", 220, "error.rpy"),
    ("00gamepad.rpy", 210, "error.rpy"),
    ("00gltest.rpy", 200, "error.rpy"),
    ("00gallery.rpy", 180, "common.rpy"),
    ("00compat.rpy", 180, "common.rpy"),
    ("00updater.rpy", 170, "common.rpy"),
    ("00gamepad.rpy", 160, "common.rpy"),
    ("00iap.rpy", 150, "common.rpy"),
    ("", 50, "common.rpy"),
];

fn priorities(vm: &Vm, table: &[(&str, i64, &str)]) -> Value {
    let items = table
        .iter()
        .map(|(p, n, f)| vm.types.new_tuple(vec![vm.types.new_str(p), Value::Int(*n), vm.types.new_str(f)]))
        .collect();
    vm.types.new_list(items)
}

/// `shorten_filename(filename)` -> (fn, common).
fn shorten(vm: &mut Vm, filename: &str) -> PyResult<(String, bool)> {
    let common = config_dir(vm, "commondir")?;
    let game = config_dir(vm, "gamedir")?;
    let rel = |base: &str| normpath(filename).strip_prefix(&format!("{base}/")).map(str::to_owned);
    if filename.starts_with(&common) {
        Ok((rel(&common).unwrap_or_default(), true))
    } else if filename.starts_with(&game) {
        Ok((rel(&game).unwrap_or_default(), false))
    } else {
        Ok((filename.rsplit('/').next().unwrap_or(filename).to_owned(), false))
    }
}

fn f_shorten_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let (n, c) = shorten(vm, &f)?;
    Ok(vm.types.new_tuple(vec![py::s(vm, &n), Value::Bool(c)]))
}

fn string_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["filename", "line", "text", "comment"].iter().enumerate() {
        py::set(vm, this, n, args[i + 1].clone())?;
    }
    let filename = py::string(vm, &args[1])?;
    let (elided, common) = shorten(vm, &filename)?;
    py::set(vm, this, "elided", py::s(vm, &elided))?;
    py::set(vm, this, "common", Value::Bool(common))?;
    let table = if common { COMMON_PRIORITIES } else { REGULAR_PRIORITIES };
    let normalized = if since_pair(vm, 6, 1) { elided.replace('\\', "/") } else { elided.clone() };
    let (priority, launcher) = table
        .iter()
        .find(|(p, _, _)| normalized.starts_with(p))
        .map(|(_, n, f)| (*n, *f))
        .unwrap_or((500, "unknown.rpy"));
    py::set(vm, this, "priority", Value::Int(priority))?;
    let sk = vm.types.new_tuple(vec![Value::Int(priority), args[1].clone(), args[2].clone()]);
    py::set(vm, this, "sort_key", sk)?;
    py::set(vm, this, "launcher_file", py::s(vm, launcher))?;
    Ok(Value::None)
}

fn string_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "filename")?;
    let f = ops::str(vm, &f)?;
    let l = py::get(vm, &args[0], "line")?;
    let l = ops::str(vm, &l)?;
    let t = py::get(vm, &args[0], "text")?;
    let t = ops::repr(vm, &t)?;
    Ok(py::s(vm, &format!("<String {f}:{l} {t}>")))
}

fn new_string(vm: &mut Vm, filename: &str, line: i64, text: Value, comment: bool) -> PyResult<Value> {
    let c = py::import_attr(vm, SCAN, "String")?;
    vm.call(&c, vec![py::s(vm, filename), Value::Int(line), text, Value::Bool(comment)], Vec::new())
}

/// Translatable strings of a file as (line, text): `additional_strings`
/// first, then `_("...")` and friends in its logical lines.
fn scan_file_strings(vm: &mut Vm, filename: &str) -> PyResult<Vec<(i64, Value)>> {
    let mut rv = Vec::new();
    let t = translator(vm)?;
    let add = py::get(vm, &t, "additional_strings")?;
    let l = ops::getitem(vm, &add, py::s(vm, filename))?;
    for e in py::items(vm, &l)? {
        let p = py::items(vm, &e)?;
        rv.push((py::int(vm, &p[0])?, p[1].clone()));
    }
    let Some(text) = read_path(vm, filename)? else {
        return Ok(rv);
    };
    let fname: std::rc::Rc<str> = filename.into();
    let lines = match crate::lexer::list_logical_lines(&fname, &text, 1) {
        Ok(l) => l,
        Err(e) => return Err(py::error(vm, e.to_string())),
    };
    let re = string_re(vm);
    let finditer = py::import_attr(vm, "re", "finditer")?;
    let eval = py::import_attr(vm, "builtins", "eval")?;
    let p = rt(vm).profile.clone();
    let strip_cont = p.at_least(7, 3);
    let u_prefix = !(p.at_least(8, 4) || p.since_pair(8, 2));
    let has_p = p.at_least(7, 0);
    for (lineno, text) in lines {
        let it = vm.call(&finditer, vec![py::s(vm, re), py::s(vm, &text)], Vec::new())?;
        for m in ops::collect(vm, &it)? {
            let g = py::method(vm, &m, "group", vec![Value::Int(1)])?;
            let Some(mut s) = opt_str(vm, &g)? else { continue };
            if strip_cont {
                s = s.replace("\\\n", "");
            }
            let s = s.trim();
            let src = if u_prefix { format!("u{s}") } else { s.to_owned() };
            let mut v = vm.call(&eval, vec![py::s(vm, &src)], Vec::new())?;
            let g0 = py::method(vm, &m, "group", vec![Value::Int(0)])?;
            if has_p && py::string(vm, &g0)?.starts_with("_p") {
                v = call(vm, "renpy.minstore", "_p", vec![v])?;
            }
            if !p.at_least(7, 0) || py::truthy(vm, &v)? {
                rv.push((lineno as i64, v));
            }
        }
    }
    Ok(rv)
}

fn f_scan_strings(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let mut rv = Vec::new();
    for (line, s) in scan_file_strings(vm, &f)? {
        rv.push(new_string(vm, &f, line, s, false)?);
    }
    Ok(vm.types.new_list(rv))
}

/// 6.99 – 7.3 `generation.scan_strings` and 6.18 `translation.scan_strings`:
/// (line, string) pairs.
fn f_scan_strings_pairs(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let rv = scan_file_strings(vm, &f)?
        .into_iter()
        .map(|(l, s)| vm.types.new_tuple(vec![Value::Int(l), s]))
        .collect();
    Ok(vm.types.new_list(rv))
}

/// `## ` comment runs of a file listed in `config.translate_comments`.
fn scan_comment_runs(vm: &mut Vm, filename: &str) -> PyResult<Vec<(i64, String)>> {
    let tc = py::config(vm, "translate_comments")?;
    if !ops::contains(vm, &tc, &py::s(vm, filename))? {
        return Ok(Vec::new());
    }
    let Some(text) = read_path(vm, filename)? else {
        let r = ops::repr(vm, &py::s(vm, filename))?;
        return Err(vm.exc_msg("FileNotFoundError", format!("No such file or directory: {r}")));
    };
    let mut rv = Vec::new();
    let mut comment: Vec<String> = Vec::new();
    let mut start = 0;
    for (i, l) in text.split('\n').map(str::trim_end).enumerate() {
        if comment.is_empty() {
            start = i as i64 + 1;
        }
        let t = l.trim_start();
        if let Some(c) = t.strip_prefix("## ") {
            comment.push(if comment.is_empty() { c.to_owned() } else { c.trim().to_owned() });
        } else if !comment.is_empty() {
            let mut s = format!("## {}", comment.join(" "));
            if s.ends_with('#') {
                s = s.trim_end_matches(['#', ' ']).to_owned();
            }
            comment.clear();
            rv.push((start, s));
        }
    }
    Ok(rv)
}

fn f_scan_comments_strings(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let mut rv = Vec::new();
    for (l, s) in scan_comment_runs(vm, &f)? {
        let sv = py::s(vm, &s);
        rv.push(new_string(vm, &f, l, sv, true)?);
    }
    Ok(vm.types.new_list(rv))
}

fn f_scan_comments_pairs(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let rv = scan_comment_runs(vm, &f)?
        .into_iter()
        .map(|(l, s)| vm.types.new_tuple(vec![Value::Int(l), vm.types.new_str(&s)]))
        .collect();
    Ok(vm.types.new_list(rv))
}

fn f_scan_additional_strings(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cbs = py::config(vm, "translate_additional_strings_callbacks")?;
    let mut rv = Vec::new();
    for cb in py::items(vm, &cbs)? {
        let r = vm.call(&cb, Vec::new(), Vec::new())?;
        for e in ops::collect(vm, &r)? {
            let p = py::items(vm, &e)?;
            let f = py::string(vm, &p[0])?;
            let l = py::int(vm, &p[1])?;
            rv.push(new_string(vm, &f, l, p[2].clone(), false)?);
        }
    }
    Ok(vm.types.new_list(rv))
}

/// `scan(min_priority=0, max_priority=299, common_only=False)`.
fn scan(vm: &mut Vm, min: i64, max: i64, common_only: bool) -> PyResult<Vec<Value>> {
    let files = translate_list_files(vm)?;
    let mut strings = Vec::new();
    let ss = py::import_attr(vm, SCAN, "scan_strings")?;
    let sc = py::import_attr(vm, SCAN, "scan_comments")?;
    for f in files {
        let f = normpath(&f);
        if !exists(vm, &f)? {
            continue;
        }
        let a = vm.call(&ss, vec![py::s(vm, &f)], Vec::new())?;
        strings.extend(py::items(vm, &a)?);
        let b = vm.call(&sc, vec![py::s(vm, &f)], Vec::new())?;
        strings.extend(py::items(vm, &b)?);
    }
    if at_least(vm, 8, 4) {
        let a = f_scan_additional_strings(vm, &[], &[])?;
        strings.extend(py::items(vm, &a)?);
    }
    let mut keyed = Vec::new();
    for s in strings {
        let k = py::get(vm, &s, "sort_key")?;
        let k = py::items(vm, &k)?;
        let (p, f, l) = (py::int(vm, &k[0])?, py::string(vm, &k[1])?, py::int(vm, &k[2])?);
        keyed.push(((p, f, l), s));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut rv = Vec::new();
    let seen = vm.types.new_set(Default::default());
    for (_, s) in keyed {
        let p = py::get(vm, &s, "priority")?;
        let p = py::int(vm, &p)?;
        if p < min || p > max {
            continue;
        }
        if common_only && !arg_bool(vm, &s, "common")? {
            continue;
        }
        let t = py::get(vm, &s, "text")?;
        if ops::contains(vm, &seen, &t)? {
            continue;
        }
        ops::set_add(vm, &seen, t)?;
        rv.push(s);
    }
    Ok(rv)
}

fn f_scan(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let min = py::int(vm, &arg_or(args, kw, 0, "min_priority", Value::Int(0)))?;
    let max = py::int(vm, &arg_or(args, kw, 1, "max_priority", Value::Int(299)))?;
    let c = arg_or(args, kw, 2, "common_only", Value::Bool(false));
    let c = py::truthy(vm, &c)?;
    let rv = scan(vm, min, max, c)?;
    Ok(vm.types.new_list(rv))
}

// generation -------------------------------------------------------------------------------------

fn gen_attr(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, GEN, n)
}

fn open_tl_file(vm: &mut Vm, path: &str) -> PyResult<Value> {
    let cache = gen_attr(vm, "tl_file_cache")?;
    let k = py::s(vm, path);
    if ops::contains(vm, &cache, &k)? {
        return ops::getitem(vm, &cache, k);
    }
    let f = open_new_tl_file(vm, path, true)?;
    ops::setitem(vm, &cache, k, f.clone())?;
    Ok(f)
}

/// Opens `path` for appending, writing the BOM to a new file and the TODO
/// marker when requested.
fn open_new_tl_file(vm: &mut Vm, path: &str, check_todo: bool) -> PyResult<Value> {
    let ex = os_path(vm, "exists", vec![py::s(vm, path)])?;
    let f = if !py::truthy(vm, &ex)? {
        let dn = os_path(vm, "dirname", vec![py::s(vm, path)])?;
        let _ = call(vm, "os", "makedirs", vec![dn]);
        let f = open(vm, path, "a")?;
        write(vm, &f, "\u{feff}")?;
        f
    } else {
        open(vm, path, "a")?
    };
    let todo = if check_todo {
        let t = gen_attr(vm, "todo")?;
        py::truthy(vm, &t)?
    } else {
        true
    };
    if todo {
        let st = call(vm, "time", "strftime", vec![py::s(vm, "%Y-%m-%d %H:%M")])?;
        let st = py::string(vm, &st)?;
        let marker = if at_least(vm, 6, 99) { "# TO" } else { "# TO" };
        write(vm, &f, &format!("{marker}DO: Translation updated at {st}\n"))?;
    }
    write(vm, &f, "\n")?;
    Ok(f)
}

fn f_open_tl_file(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::string(vm, &args[0])?;
    if !at_least(vm, 6, 99) {
        return open_new_tl_file(vm, &p, false);
    }
    open_tl_file(vm, &p)
}

fn f_close_tl_files(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let cache = gen_attr(vm, "tl_file_cache")?;
    let values = py::method(vm, &cache, "values", Vec::new())?;
    for f in ops::collect(vm, &values)? {
        close(vm, &f)?;
    }
    py::method(vm, &cache, "clear", Vec::new())?;
    Ok(Value::None)
}

fn is_empty_extend(vm: &mut Vm, id: NodeId) -> PyResult<bool> {
    for n in tl_block(vm, id) {
        if get_code(vm, n, None)? != "extend \"\"" {
            return Ok(false);
        }
    }
    Ok(true)
}

fn f_is_empty_extend(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = nodes::id_of(&args[0]).ok_or_else(|| vm.type_error("translate node expected"))?;
    Ok(Value::Bool(is_empty_extend(vm, id)?))
}

fn tl_dir(vm: &mut Vm) -> PyResult<String> {
    let d = py::config(vm, "tl_directory")?;
    py::string(vm, &d)
}

fn gamedir(vm: &mut Vm) -> PyResult<String> {
    let g = py::config(vm, "gamedir")?;
    py::string(vm, &g)
}

fn write_translates(vm: &mut Vm, filename: &str, language: &str, filter: &Value) -> PyResult<()> {
    let (fname, common) = shorten(vm, filename)?;
    if common {
        return Ok(());
    }
    let (gd, tld) = (gamedir(vm)?, tl_dir(vm)?);
    let mut tl_filename = join(vm, &[&gd, &tld, language, &fname])?;
    if tl_filename.ends_with('m') {
        tl_filename.pop();
    }
    let lang = (language != "None").then_some(language);
    let p = rt(vm).profile.clone();
    let alternate = p.at_least(7, 3);
    let newer = p.since_pair(6, 1);
    for (_, t) in file_translates(vm, filename)? {
        let Some((identifier, _, alt)) = tl_identity(vm, t) else { continue };
        if has_language_translate(vm, &identifier, lang) {
            continue;
        }
        if alternate
            && let Some(a) = &alt
            && has_language_translate(vm, a, lang)
        {
            continue;
        }
        if newer {
            if is_empty_extend(vm, t)? {
                continue;
            }
            if let NodeKind::TranslateSay { say, .. } = kind(vm, t)
                && let NodeKind::Say { who: Some(w), .. } = &*say
            {
                let ig = py::config(vm, "translate_ignore_who")?;
                if ops::contains(vm, &ig, &py::s(vm, &w.text))? {
                    continue;
                }
            }
        }
        let f = open_tl_file(vm, &tl_filename)?;
        let (tf, tln) = node_loc(vm, t);
        let lname = lang.unwrap_or("None");
        write(vm, &f, &format!("# {tf}:{tln}\n"))?;
        let ident = if alternate || p.at_least(7, 0) { identifier.replace('.', "_") } else { identifier.clone() };
        write(vm, &f, &format!("translate {lname} {ident}:\n"))?;
        write(vm, &f, "\n")?;
        let block = tl_block(vm, t);
        for &n in &block {
            let c = get_code(vm, n, None)?;
            write(vm, &f, &format!("    # {c}\n"))?;
        }
        for &n in &block {
            let c = get_code(vm, n, Some(filter))?;
            write(vm, &f, &format!("    {c}\n"))?;
        }
        write(vm, &f, "\n")?;
    }
    Ok(())
}

fn f_write_translates(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[0])?;
    let l = py::string(vm, &args[1])?;
    write_translates(vm, &f, &l, &args[2])?;
    Ok(Value::None)
}

fn translation_filename(vm: &mut Vm, s: &Value) -> PyResult<Value> {
    if py::config_bool(vm, "translate_launcher").unwrap_or(false) {
        return py::get(vm, s, "launcher_file");
    }
    if arg_bool(vm, s, "common")? {
        return Ok(py::s(vm, "common.rpy"));
    }
    let e = py::get(vm, s, "elided")?;
    let mut f = py::string(vm, &e)?;
    if since_pair(vm, 6, 1)
        && let Some(stem) = f.strip_suffix("_ren.py")
    {
        f = format!("{stem}.rpy");
    }
    if f.ends_with('m') {
        f.pop();
    }
    Ok(py::s(vm, &f))
}

fn f_translation_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    translation_filename(vm, &args[0])
}

fn strings_for(vm: &mut Vm, language: &str) -> PyResult<Value> {
    let t = translator(vm)?;
    let strings = py::get(vm, &t, "strings")?;
    let k = if language == "None" { Value::None } else { py::s(vm, language) };
    ops::getitem(vm, &strings, k)
}

fn write_strings(
    vm: &mut Vm,
    language: &str,
    filter: &Value,
    min: i64,
    max: i64,
    common_only: bool,
    only: &[Value],
) -> PyResult<()> {
    let stl = strings_for(vm, language)?;
    let translations = py::get(vm, &stl, "translations")?;
    let strings = scan(vm, min, max, common_only)?;
    let mut files: Vec<(String, Vec<Value>)> = Vec::new();
    for s in strings {
        let tlfn = translation_filename(vm, &s)?;
        let Some(mut tlfn) = opt_str(vm, &tlfn)? else { continue };
        let text = py::get(vm, &s, "text")?;
        if ops::contains(vm, &translations, &text)? {
            continue;
        }
        if language == "None" && tlfn == "common.rpy" {
            tlfn = "common.rpym".into();
        }
        if !only.is_empty() {
            let mut found = false;
            for o in only {
                if ops::eq(vm, o, &text)? {
                    found = true;
                }
            }
            if !found {
                continue;
            }
        }
        match files.iter_mut().find(|(f, _)| *f == tlfn) {
            Some((_, l)) => l.push(s),
            None => files.push((tlfn, vec![s])),
        }
    }
    let (gd, tld) = (gamedir(vm)?, tl_dir(vm)?);
    let elide_new = at_least(vm, 7, 3);
    for (tlfn, sl) in files {
        let path = join(vm, &[&gd, &tld, language, &tlfn])?;
        let f = open_tl_file(vm, &path)?;
        write(vm, &f, &format!("translate {language} strings:\n"))?;
        write(vm, &f, "\n")?;
        for s in sl {
            let text = py::get(vm, &s, "text")?;
            let t = vm.call(filter, vec![text.clone()], Vec::new())?;
            let t = py::string(vm, &t)?;
            let text = py::string(vm, &text)?;
            let loc = if elide_new {
                let fname = py::get(vm, &s, "filename")?;
                let fname = py::string(vm, &fname)?;
                elide(vm, &fname)?
            } else {
                let e = py::get(vm, &s, "elided")?;
                py::string(vm, &e)?
            };
            let line = py::get(vm, &s, "line")?;
            let line = ops::str(vm, &line)?;
            write(vm, &f, &format!("    # {loc}:{line}\n"))?;
            write(vm, &f, &format!("    old \"{}\"\n", quote_unicode(&text)))?;
            write(vm, &f, &format!("    new \"{}\"\n", quote_unicode(&t)))?;
            write(vm, &f, "\n")?;
        }
    }
    Ok(())
}

fn f_write_strings(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let lang = py::string(vm, &args[0])?;
    let min = py::int(vm, &args[2])?;
    let max = py::int(vm, &args[3])?;
    let co = py::truthy(vm, &args[4])?;
    let only = arg_or(args, kw, 5, "only_strings", Value::None);
    let only = if py::is_none(&only) { Vec::new() } else { ops::collect(vm, &only)? };
    write_strings(vm, &lang, &args[1], min, max, co, &only)?;
    Ok(Value::None)
}

/// `translate_list_files()`: `config.translate_files` and the game's and
/// common `.rpy`/`.rpym` (and `_ren.py`) files.
fn translate_list_files(vm: &mut Vm) -> PyResult<Vec<String>> {
    let tf = py::config(vm, "translate_files")?;
    let mut rv = Vec::new();
    for f in py::items(vm, &tf)? {
        rv.push(py::string(vm, &f)?);
    }
    let skip_tl = at_least(vm, 7, 3);
    let ren_py = since_pair(vm, 6, 1);
    let l = call(vm, "renpy.loader", "listdirfiles", Vec::new())?;
    for e in py::items(vm, &l)? {
        let p = py::items(vm, &e)?;
        let Some(dir) = opt_str(vm, &p[0])? else { continue };
        let name = py::string(vm, &p[1])?;
        if skip_tl && name.starts_with("tl/") {
            continue;
        }
        let f = join(vm, &[&dir, &name])?;
        if !(f.ends_with(".rpy") || f.ends_with(".rpym") || (ren_py && f.ends_with("_ren.py"))) {
            continue;
        }
        let f = normpath(&f);
        if !exists(vm, &f)? {
            continue;
        }
        rv.push(f);
    }
    Ok(rv)
}

fn f_translate_list_files(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let l = translate_list_files(vm)?;
    let v = l.iter().map(|s| vm.types.new_str(s)).collect();
    Ok(vm.types.new_list(v))
}

fn count_missing(vm: &mut Vm, language: &str, min: i64, max: i64, common_only: bool) -> PyResult<()> {
    let mut missing_translates = 0;
    let newer = since_pair(vm, 6, 1);
    for f in translate_list_files(vm)? {
        for (_, t) in file_translates(vm, &f)? {
            if newer && is_empty_extend(vm, t)? {
                continue;
            }
            let Some((ident, _, _)) = tl_identity(vm, t) else { continue };
            if !has_language_translate(vm, &ident, Some(language)) {
                missing_translates += 1;
            }
        }
    }
    let stl = strings_for(vm, language)?;
    let tr = py::get(vm, &stl, "translations")?;
    let mut missing_strings = 0;
    for s in scan(vm, min, max, common_only)? {
        let tlfn = translation_filename(vm, &s)?;
        if py::is_none(&tlfn) {
            continue;
        }
        let t = py::get(vm, &s, "text")?;
        if ops::contains(vm, &tr, &t)? {
            continue;
        }
        missing_strings += 1;
    }
    let print = py::import_attr(vm, "builtins", "print")?;
    let msg = format!(
        "{language}: {missing_translates} missing dialogue translations, {missing_strings} missing string translations."
    );
    vm.call(&print, vec![py::s(vm, &msg)], Vec::new())?;
    Ok(())
}

fn f_count_missing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::string(vm, &args[0])?;
    let min = py::int(vm, &args[1])?;
    let max = py::int(vm, &args[2])?;
    let c = py::truthy(vm, &args[3])?;
    count_missing(vm, &l, min, max, c)?;
    Ok(Value::None)
}

fn f_translate_command(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !at_least(vm, 6, 99) {
        return translate_command_618(vm);
    }
    let int_t = py::import_attr(vm, "builtins", "int")?;
    let mut spec = vec![
        Arg::Pos("language", "The language to generate translations for."),
        Arg::Flag("--rot13", "rot13", "Apply rot13 while generating translations."),
        Arg::Flag("--piglatin", "piglatin", "Apply pig latin while generating translations."),
        Arg::Flag("--empty", "empty", "Produce empty strings while generating translations."),
        Arg::Flag("--count", "count", "Instead of generating files, print a count of missing translations."),
        Arg::Opt(
            "--min-priority",
            vec![
                ("help", py::s(vm, "Translate strings with more than this priority.")),
                ("dest", py::s(vm, "min_priority")),
                ("default", Value::Int(0)),
                ("type", int_t.clone()),
            ],
        ),
        Arg::Opt(
            "--max-priority",
            vec![
                ("help", py::s(vm, "Translate strings with less than this priority.")),
                ("dest", py::s(vm, "max_priority")),
                ("default", Value::Int(0)),
                ("type", int_t),
            ],
        ),
        Arg::Flag("--strings-only", "strings_only", "Only translate strings (not dialogue)."),
        Arg::Flag("--common-only", "common_only", "Only translate string from the common code."),
        Arg::Opt(
            "--no-todo",
            vec![
                ("help", py::s(vm, "Do not include the TODO flag.")),
                ("dest", py::s(vm, "todo")),
                ("default", Value::Bool(true)),
                ("action", py::s(vm, "store_false")),
            ],
        ),
    ];
    if at_least(vm, 8, 0) || at_least(vm, 7, 4) {
        spec.push(Arg::Opt(
            "--string",
            vec![
                ("help", py::s(vm, "Translate a single string.")),
                ("dest", py::s(vm, "string")),
                ("action", py::s(vm, "append")),
            ],
        ));
    }
    let args = parse_args(vm, "translate", "Generates or updates translations.", spec)?;
    let todo = py::get_or_none(vm, &args, "todo")?;
    let m = py::import(vm, GEN)?;
    py::set(vm, &m, "todo", todo)?;
    let mp = py::get(vm, &args, "max_priority")?;
    let mut max = py::int(vm, &mp)?;
    if max == 0 {
        max = if py::config_bool(vm, "translate_launcher").unwrap_or(false) { 499 } else { 299 };
    }
    let min = py::get(vm, &args, "min_priority")?;
    let min = py::int(vm, &min)?;
    let lang = py::get(vm, &args, "language")?;
    let language = py::string(vm, &lang)?;
    let common_only = arg_bool(vm, &args, "common_only")?;
    if arg_bool(vm, &args, "count")? {
        count_missing(vm, &language, min, max, common_only)?;
        return Ok(Value::Bool(false));
    }
    let filter = if arg_bool(vm, &args, "rot13")? {
        gen_attr(vm, "rot13_filter")?
    } else if arg_bool(vm, &args, "piglatin")? {
        gen_attr(vm, "piglatin_filter")?
    } else if arg_bool(vm, &args, "empty")? {
        gen_attr(vm, "empty_filter")?
    } else {
        gen_attr(vm, "null_filter")?
    };
    let strings_only = arg_bool(vm, &args, "strings_only")?;
    if !strings_only {
        for f in translate_list_files(vm)? {
            write_translates(vm, &f, &language, &filter)?;
        }
    }
    let only = py::get_or_none(vm, &args, "string")?;
    let only = if py::is_none(&only) { Vec::new() } else { ops::collect(vm, &only)? };
    write_strings(vm, &language, &filter, min, max, common_only, &only)?;
    f_close_tl_files(vm, &[], &[])?;
    if py::config_bool(vm, "translate_launcher").unwrap_or(false) && !strings_only {
        let base = py::config(vm, "renpy_base")?;
        let base = py::string(vm, &base)?;
        let src = join(vm, &[&base, "gui", "game", "script.rpy"])?;
        let gd = gamedir(vm)?;
        let dst = join(vm, &[&gd, "tl", &language, "script.rpym"])?;
        if exists(vm, &src)? && !exists(vm, &dst)? {
            let data = vm.host.read_file(&src);
            if let Some(d) = data {
                let f = open(vm, &dst, "w")?;
                write(vm, &f, &String::from_utf8_lossy(&d))?;
                close(vm, &f)?;
            }
        }
    }
    Ok(Value::Bool(false))
}

// dialogue --------------------------------------------------------------------------------------

/// `create_dialogue_map(language)`: dialogue text -> its translation.
fn f_create_dialogue_map(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let language = py::string(vm, &args[0])?;
    let rv = vm.types.new_dict();
    let t = translator(vm)?;
    let ft = py::get(vm, &t, "file_translates")?;
    let values = py::method(vm, &ft, "values", Vec::new())?;
    let get_text = |vm: &Vm, id: NodeId| -> Option<String> {
        let k = kind(vm, id);
        if let Some(NodeKind::Say { what, .. }) = say_kind(&k) {
            return Some(what.clone());
        }
        for b in tl_block(vm, id) {
            if let Some(NodeKind::Say { what, .. }) = say_kind(&kind(vm, b)) {
                return Some(what.clone());
            }
        }
        None
    };
    for v in ops::collect(vm, &values)? {
        for e in py::items(vm, &v)? {
            let p = py::items(vm, &e)?;
            let Some(id) = nodes::id_of(&p[1]) else { continue };
            let Some((ident, _, _)) = tl_identity(vm, id) else { continue };
            let Some(lt) = language_translate(vm, &ident, &language) else { continue };
            if let (Some(a), Some(b)) = (get_text(vm, id), get_text(vm, lt))
                && !a.is_empty()
                && !b.is_empty()
            {
                ops::setitem(vm, &rv, py::s(vm, &a), py::s(vm, &b))?;
            }
        }
    }
    Ok(rv)
}

struct DialogueOptions {
    tdf: bool,
    strings: bool,
    notags: bool,
    escape: bool,
    language: Option<String>,
    menus: Vec<NodeId>,
}

fn escape_tdf(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn dialogue_lines(vm: &mut Vm, filename: &str, o: &DialogueOptions) -> PyResult<Vec<(u32, Vec<String>)>> {
    let p = rt(vm).profile.clone();
    let mut lines = Vec::new();
    let mut prior_who = String::new();
    let wf = if p.at_least(7, 4) { Some(py::import_attr(vm, DIA, "what_filter")?) } else { None };
    for (_, t) in file_translates(vm, filename)? {
        let Some((identifier, _, _)) = tl_identity(vm, t) else { continue };
        let key = identifier.replace('.', "_");
        let tl = match (&o.language, p.at_least(7, 5)) {
            (Some(l), true) => language_translate(vm, &key, l).unwrap_or(t),
            _ => t,
        };
        let (_, tline) = node_loc(vm, t);
        for n in tl_block(vm, tl) {
            let k = kind(vm, n);
            let Some(NodeKind::Say { who, what, .. }) = say_kind(&k) else { continue };
            let mut who = who.as_ref().map(|w| w.text.to_string()).unwrap_or_default();
            if p.at_least(8, 5) {
                let el = py::config(vm, "extend_like_characters")?;
                if ops::contains(vm, &el, &py::s(vm, &who))? {
                    who = format!("{who} {prior_who}");
                } else {
                    prior_who = who.clone();
                }
            }
            let mut what = what.clone();
            if o.notags {
                what = notags(&what, p.since_pair(6, 1));
            }
            if p.since_pair(6, 1) {
                what = combine(vm, &what);
            }
            if o.escape {
                what = quote_unicode(&what);
            } else if o.tdf && p.at_least(7, 0) {
                what = escape_tdf(&what);
            }
            let line = if o.tdf {
                let (nf, nl) = node_loc(vm, n);
                let mut l = vec![identifier.clone(), who, what, nf, nl.to_string()];
                if let Some(wf) = &wf {
                    l.push(get_code(vm, n, Some(wf))?);
                }
                l
            } else {
                vec![what]
            };
            lines.push((tline, line));
        }
    }
    for m in &o.menus {
        let (mf, ml) = node_loc(vm, *m);
        if let NodeKind::Menu { items, .. } = kind(vm, *m) {
            for it in items {
                if it.block.is_some() {
                    continue;
                }
                lines.push((
                    ml,
                    vec!["<menu caption>".into(), String::new(), it.label.clone(), mf.clone(), ml.to_string(), "[what]".into()],
                ));
            }
        }
    }
    if o.strings {
        lines.extend(dialogue_strings(vm, filename, o)?);
        if o.tdf {
            lines.sort_by_key(|x| x.0);
        }
    }
    Ok(lines)
}

fn dialogue_strings(vm: &mut Vm, filename: &str, o: &DialogueOptions) -> PyResult<Vec<(u32, Vec<String>)>> {
    let p = rt(vm).profile.clone();
    let mut lines = Vec::new();
    let elided = elide(vm, filename)?;
    let strings = scan_file_strings(vm, filename)?;
    let stl = strings_for(vm, "None")?;
    let tr = py::get(vm, &stl, "translations")?;
    for (line, sv) in strings {
        if ops::contains(vm, &tr, &sv)? {
            continue;
        }
        ops::setitem(vm, &tr, sv.clone(), sv.clone())?;
        let sv = if p.at_least(7, 5) {
            let lang = o.language.as_ref().map(|l| py::s(vm, l)).unwrap_or(Value::None);
            super::translation::translate_string(vm, &sv, Some(lang))?
        } else {
            sv
        };
        let mut s = py::string(vm, &sv)?;
        if o.notags {
            s = notags(&s, p.since_pair(6, 1));
        }
        if p.since_pair(6, 1) {
            s = combine(vm, &s);
        }
        if o.escape {
            s = quote_unicode(&s);
        } else if o.tdf && p.at_least(7, 0) {
            s = escape_tdf(&s);
        }
        let l = if o.tdf { vec![String::new(), String::new(), s, elided.clone(), line.to_string()] } else { vec![s] };
        lines.push((line as u32, l));
    }
    Ok(lines)
}

fn df_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let p = rt(vm).profile.clone();
    let filename_v = py::arg_or(args, kw, 1, "filename", Value::None);
    let filename = py::string(vm, &filename_v)?;
    py::set(vm, this, "filename", filename_v)?;
    let common = config_dir(vm, "commondir")?;
    if filename.starts_with(&common) {
        return Ok(Value::None);
    }
    let output = py::arg_or(args, kw, 2, "output", Value::None);
    let output = py::string(vm, &output)?;
    let b = |vm: &mut Vm, i: usize, n: &str, d: bool| -> PyResult<bool> {
        let v = py::arg_or(args, kw, i, n, Value::Bool(d));
        py::truthy(vm, &v)
    };
    let tdf = b(vm, 3, "tdf", true)?;
    let (strings, notags_, escape) = if p.at_least(6, 99) && (p.at_least(7, 0) || true) {
        (b(vm, 4, "strings", false)?, b(vm, 5, "notags", true)?, b(vm, 6, "escape", true)?)
    } else {
        (false, true, false)
    };
    let language = if p.at_least(7, 5) {
        let l = py::arg_or(args, kw, 7, "language", Value::None);
        opt_str(vm, &l)?
    } else {
        None
    };
    let mut menus = Vec::new();
    if p.at_least(8, 6) {
        let m = py::arg_or(args, kw, 8, "menu_statements", vm.types.new_tuple(Vec::new()));
        for n in ops::collect(vm, &m)? {
            if let Some(id) = nodes::id_of(&n) {
                menus.push(id);
            }
        }
    }
    for (n, v) in [("tdf", tdf), ("notags", notags_), ("escape", escape), ("strings", strings)] {
        py::set(vm, this, n, Value::Bool(v))?;
    }
    let lv = language.as_ref().map(|l| py::s(vm, l)).unwrap_or(Value::None);
    py::set(vm, this, "language", lv)?;
    let o = DialogueOptions { tdf, strings, notags: notags_, escape, language, menus };
    let f = open(vm, &output, "a")?;
    py::set(vm, this, "f", f.clone())?;
    let r = dialogue_lines(vm, &filename, &o);
    let r = r.and_then(|lines| {
        for (_, l) in lines {
            write(vm, &f, &format!("{}\n", l.join("\t")))?;
        }
        Ok(())
    });
    close(vm, &f)?;
    r?;
    Ok(Value::None)
}

fn df_write_dialogue(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::get(vm, this, "filename")?;
    let filename = py::string(vm, &filename)?;
    let g = |vm: &mut Vm, n: &str| -> PyResult<bool> { arg_bool(vm, this, n) };
    let lang = py::get_or_none(vm, this, "language")?;
    let o = DialogueOptions {
        tdf: g(vm, "tdf")?,
        strings: g(vm, "strings")?,
        notags: g(vm, "notags")?,
        escape: g(vm, "escape")?,
        language: opt_str(vm, &lang)?,
        menus: Vec::new(),
    };
    let f = py::get(vm, this, "f")?;
    for (_, l) in dialogue_lines(vm, &filename, &o)? {
        write(vm, &f, &format!("{}\n", l.join("\t")))?;
    }
    Ok(Value::None)
}

fn df_get_strings(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::get(vm, this, "filename")?;
    let filename = py::string(vm, &filename)?;
    let lang = py::get_or_none(vm, this, "language")?;
    let o = DialogueOptions {
        tdf: arg_bool(vm, this, "tdf")?,
        strings: true,
        notags: arg_bool(vm, this, "notags")?,
        escape: arg_bool(vm, this, "escape")?,
        language: opt_str(vm, &lang)?,
        menus: Vec::new(),
    };
    let mut rv = Vec::new();
    for (line, l) in dialogue_strings(vm, &filename, &o)? {
        let l = l.iter().map(|s| vm.types.new_str(s)).collect();
        rv.push(vm.types.new_tuple(vec![Value::Int(line as i64), vm.types.new_list(l)]));
    }
    Ok(vm.types.new_list(rv))
}

fn f_dialogue_command(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = rt(vm).profile.clone();
    let mut spec = Vec::new();
    if p.at_least(7, 5) {
        spec.push(Arg::Pos("language", "The language to extract dialogue for."));
    }
    if p.at_least(6, 99) {
        spec.extend([
            Arg::Flag("--text", "text", "Output the dialogue as plain text, instead of a tab-delimited file."),
            Arg::Flag("--strings", "strings", "Output all translatable strings, not just dialogue."),
            Arg::Flag("--notags", "notags", "Strip text tags from the dialogue."),
            Arg::Flag("--escape", "escape", "Escape quotes and other special characters."),
        ]);
    } else {
        spec.push(Arg::Flag("--text", "text", "Apply rot13 while generating translations."));
    }
    let args = parse_args(vm, "dialogue", "Generates or updates translations.", spec)?;
    let tdf = !arg_bool(vm, &args, "text")?;
    let base = py::config(vm, "basedir")?;
    let base = py::string(vm, &base)?;
    let output = join(vm, &[&base, if tdf { "dialogue.tab" } else { "dialogue.txt" }])?;
    let f = open(vm, &output, "w")?;
    if tdf {
        let mut h = vec!["Identifier", "Character", "Dialogue", "Filename", "Line Number"];
        if p.at_least(7, 4) {
            h.push("Ren'Py Script");
        }
        write(vm, &f, &format!("{}\n", h.join("\t")))?;
    }
    close(vm, &f)?;
    let mut menus: Vec<(String, NodeId)> = Vec::new();
    if p.at_least(8, 6) {
        let r = rt(vm);
        let s = r.script.borrow();
        for id in s.namemap.values() {
            let n = s.arena.get(*id);
            if matches!(n.kind, NodeKind::Menu { .. }) {
                menus.push((n.filename.to_string(), *id));
            }
        }
    }
    let language = if p.at_least(7, 5) {
        let l = py::get(vm, &args, "language")?;
        let l = py::string(vm, &l)?;
        (!matches!(l.as_str(), "None" | "")).then_some(l)
    } else {
        None
    };
    let lst = call(vm, "renpy.loader", "listdirfiles", Vec::new())?;
    let dfc = py::import_attr(vm, if p.at_least(6, 99) { DIA } else { TL }, "DialogueFile")?;
    for e in py::items(vm, &lst)? {
        let pr = py::items(vm, &e)?;
        let Some(dir) = opt_str(vm, &pr[0])? else { continue };
        let name = py::string(vm, &pr[1])?;
        let fname = join(vm, &[&dir, &name])?;
        if !(fname.ends_with(".rpy") || fname.ends_with(".rpym")) {
            continue;
        }
        let fname = normpath(&fname);
        let mut kw: Vec<(std::rc::Rc<str>, Value)> = vec![("tdf".into(), Value::Bool(tdf))];
        if p.at_least(6, 99) {
            for n in ["strings", "notags", "escape"] {
                let v = arg_bool(vm, &args, n)?;
                kw.push((n.into(), Value::Bool(v)));
            }
        }
        if p.at_least(7, 5) {
            let l = language.as_ref().map(|l| py::s(vm, l)).unwrap_or(Value::None);
            kw.push(("language".into(), l));
        }
        if p.at_least(8, 6) {
            let el = elide(vm, &fname)?;
            let mut ms = Vec::new();
            for (f, id) in &menus {
                if *f == el {
                    ms.push(nodes::handle(vm, *id)?);
                }
            }
            kw.push(("menu_statements".into(), vm.types.new_list(ms)));
        }
        vm.call(&dfc, vec![py::s(vm, &fname), py::s(vm, &output)], kw)?;
    }
    Ok(Value::Bool(false))
}

// extract / merge ---------------------------------------------------------------------------------

fn extract_strings_core(vm: &mut Vm, language: Value, dest: &str, merge: bool, force: bool) -> PyResult<()> {
    let t = translator(vm)?;
    let strings = py::get(vm, &t, "strings")?;
    if !force && !ops::contains(vm, &strings, &language)? {
        let r = ops::repr(vm, &language)?;
        return Err(py::error(vm, format!("Language {r} does not have any translations.")));
    }
    let st = ops::getitem(vm, &strings, language)?;
    let result = vm.types.new_dict();
    if merge {
        let f = open(vm, dest, "r")?;
        let data = py::method(vm, &f, "read", Vec::new())?;
        close(vm, &f)?;
        let d = call(vm, "json", "loads", vec![data])?;
        py::method(vm, &result, "update", vec![d])?;
    }
    let tr = py::get(vm, &st, "translations")?;
    for (k, v) in ops::mapping_items(vm, &tr)? {
        if py::truthy(vm, &v)? && !ops::eq(vm, &v, &k)? {
            ops::setitem(vm, &result, k, v)?;
        }
    }
    let dumps = py::import_attr(vm, "json", "dumps")?;
    let j = vm.call(&dumps, vec![result], vec![("ensure_ascii".into(), Value::Bool(true))])?;
    let j = py::string(vm, &j)?;
    let f = open(vm, dest, "w")?;
    write(vm, &f, &j)?;
    close(vm, &f)?;
    Ok(())
}

fn f_extract_strings_core(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let dest = py::string(vm, &args[1])?;
    let merge = arg_or(args, kw, 2, "merge", Value::Bool(false));
    let merge = py::truthy(vm, &merge)?;
    let force = arg_or(args, kw, 3, "force", Value::Bool(false));
    let force = py::truthy(vm, &force)?;
    extract_strings_core(vm, args[0].clone(), &dest, merge, force)?;
    Ok(Value::None)
}

fn none_language(vm: &mut Vm, ns: &Value) -> PyResult<Value> {
    let l = py::get(vm, ns, "language")?;
    if l.as_str() == Some("None") { Ok(Value::None) } else { Ok(l) }
}

fn f_extract_strings(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let mut spec = vec![
        Arg::Pos("language", "The language to extract translated strings from."),
        Arg::Pos("destination", "The json file to store the translated strings into."),
        Arg::Opt(
            "--merge",
            vec![
                ("help", py::s(vm, "If given, the current contents of the file are preserved, and new contents are merged into the file.")),
                ("action", py::s(vm, "store_true")),
            ],
        ),
    ];
    if at_least(vm, 6, 99) {
        spec.push(Arg::Opt(
            "--force",
            vec![
                ("help", py::s(vm, "If given, no exceptions are thrown if the language does not exist.")),
                ("action", py::s(vm, "store_true")),
            ],
        ));
    }
    let args = parse_args(vm, "extract_strings", "Extracts translated strings.", spec)?;
    let language = none_language(vm, &args)?;
    let dest = py::get(vm, &args, "destination")?;
    let dest = py::string(vm, &dest)?;
    let merge = arg_bool(vm, &args, "merge")?;
    let force = arg_bool(vm, &args, "force")?;
    extract_strings_core(vm, language, &dest, merge, force)?;
    Ok(Value::Bool(false))
}

fn f_merge_strings(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let spec = vec![
        Arg::Pos("language", "The language to merge translated strings to."),
        Arg::Pos("source", "The json file to take translated strings from."),
        Arg::Opt(
            "--reverse",
            vec![("action", py::s(vm, "store_true")), ("help", py::s(vm, "Reverses the languages in the json file."))],
        ),
        Arg::Opt(
            "--replace",
            vec![("action", py::s(vm, "store_true")), ("help", py::s(vm, "Replaces non-trivial translations."))],
        ),
    ];
    let args = parse_args(vm, "merge_strings", "Merges translated strings with the game script.", spec)?;
    let language = none_language(vm, &args)?;
    let t = translator(vm)?;
    let strings = py::get(vm, &t, "strings")?;
    if !ops::contains(vm, &strings, &language)? {
        let r = ops::repr(vm, &language)?;
        return Err(py::error(vm, format!("Language {r} does not have any translations.")));
    }
    let src = py::get(vm, &args, "source")?;
    let src = py::string(vm, &src)?;
    let f = open(vm, &src, "r")?;
    let text = py::method(vm, &f, "read", Vec::new())?;
    close(vm, &f)?;
    let mut data = call(vm, "json", "loads", vec![text])?;
    if arg_bool(vm, &args, "reverse")? {
        let nd = vm.types.new_dict();
        for (k, v) in ops::mapping_items(vm, &data)? {
            ops::setitem(vm, &nd, v, k)?;
        }
        data = nd;
    }
    let st = ops::getitem(vm, &strings, language)?;
    let cfg = py::import(vm, "renpy.config")?;
    py::set(vm, &cfg, "clear_lines", Value::Bool(false))?;
    let replace = arg_bool(vm, &args, "replace")?;
    let tr = py::get(vm, &st, "translations")?;
    let loc = py::get(vm, &st, "translation_loc")?;
    for (k, v) in ops::mapping_items(vm, &tr)? {
        let trivial = !py::truthy(vm, &v)? || ops::eq(vm, &k, &v)?;
        if !trivial && !replace {
            continue;
        }
        if !ops::contains(vm, &data, &k)? || !ops::contains(vm, &loc, &k)? {
            continue;
        }
        let new = ops::getitem(vm, &data, k.clone())?;
        let new = py::string(vm, &new)?;
        let code = format!("new \"{}\"", quote_unicode(&new));
        let l = ops::getitem(vm, &loc, k)?;
        let l = py::items(vm, &l)?;
        let line = py::int(vm, &l[1])?;
        call(vm, "renpy.scriptedit", "insert_line_before", vec![py::s(vm, &code), l[0].clone(), Value::Int(line)])?;
        call(vm, "renpy.scriptedit", "remove_line", vec![l[0].clone(), Value::Int(line + 1)])?;
    }
    Ok(Value::Bool(false))
}

// 6.18: the tools in renpy.translation ------------------------------------------------------------

fn tf_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::string(vm, &args[1])?;
    let language = py::string(vm, &args[2])?;
    py::set(vm, this, "filename", args[1].clone())?;
    py::set(vm, this, "filter", args[3].clone())?;
    let common = config_dir(vm, "commondir")?;
    let game = config_dir(vm, "gamedir")?;
    let (gd, tld) = (gamedir(vm)?, tl_dir(vm)?);
    let mut tl_filename = if let Some(rel) = filename.strip_prefix(&format!("{common}/")) {
        if rel == "_developer.rpym" || rel.starts_with("compat") {
            return Ok(Value::None);
        }
        join(vm, &[&gd, &tld, &language, "common.rpy"])?
    } else if let Some(rel) = filename.strip_prefix(&format!("{game}/")) {
        join(vm, &[&gd, &tld, &language, rel])?
    } else {
        return Err(vm.exc_msg("AttributeError", "'TranslateFile' object has no attribute 'tl_filename'"));
    };
    if tl_filename.ends_with(".rpym") {
        tl_filename.pop();
    }
    py::set(vm, this, "tl_filename", py::s(vm, &tl_filename))?;
    let lang = (language != "None").then_some(language);
    py::set(vm, this, "language", lang.as_ref().map(|l| py::s(vm, l)).unwrap_or(Value::None))?;
    py::set(vm, this, "f", Value::None)?;
    if lang.is_some() {
        py::method(vm, this, "write_translates", Vec::new())?;
    }
    py::method(vm, this, "write_strings", Vec::new())?;
    py::method(vm, this, "close", Vec::new())?;
    Ok(Value::None)
}

fn tf_open(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "f")?;
    if !py::is_none(&f) {
        return Ok(Value::None);
    }
    let p = py::get(vm, &args[0], "tl_filename")?;
    let p = py::string(vm, &p)?;
    let f = open_new_tl_file(vm, &p, false)?;
    py::set(vm, &args[0], "f", f)?;
    Ok(Value::None)
}

fn tf_close(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "f")?;
    if !py::is_none(&f) {
        close(vm, &f)?;
    }
    Ok(Value::None)
}

fn tf_write_translates(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::get(vm, this, "filename")?;
    let filename = py::string(vm, &filename)?;
    let lang = py::get(vm, this, "language")?;
    let lang = opt_str(vm, &lang)?;
    let filter = py::get(vm, this, "filter")?;
    for (_, t) in file_translates(vm, &filename)? {
        let Some((ident, _, _)) = tl_identity(vm, t) else { continue };
        if has_language_translate(vm, &ident, lang.as_deref()) {
            continue;
        }
        tf_open(vm, &args[..1], &[])?;
        let f = py::get(vm, this, "f")?;
        let (tf, tln) = node_loc(vm, t);
        write(vm, &f, &format!("# {tf}:{tln}\n"))?;
        write(vm, &f, &format!("translate {} {ident}:\n", lang.as_deref().unwrap_or("None")))?;
        write(vm, &f, "\n")?;
        let block = tl_block(vm, t);
        for &n in &block {
            let c = get_code(vm, n, None)?;
            write(vm, &f, &format!("    # {c}\n"))?;
        }
        for &n in &block {
            let c = get_code(vm, n, Some(&filter))?;
            write(vm, &f, &format!("    {c}\n"))?;
        }
        write(vm, &f, "\n")?;
    }
    Ok(Value::None)
}

fn tf_write_strings(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = py::get(vm, this, "filename")?;
    let filename = py::string(vm, &filename)?;
    let lang = py::get(vm, this, "language")?;
    let filter = py::get(vm, this, "filter")?;
    let elided = call(vm, "renpy.parser", "elide_filename", vec![py::s(vm, &filename)])
        .or_else(|_| call(vm, "renpy.lexer", "elide_filename", vec![py::s(vm, &filename)]))?;
    let elided = py::string(vm, &elided)?;
    let mut started = false;
    for (line, s) in scan_file_strings(vm, &filename)? {
        let t = translator(vm)?;
        let strings = py::get(vm, &t, "strings")?;
        let stl = ops::getitem(vm, &strings, lang.clone())?;
        let tr = py::get(vm, &stl, "translations")?;
        if ops::contains(vm, &tr, &s)? {
            continue;
        }
        ops::setitem(vm, &tr, s.clone(), s.clone())?;
        if !started {
            started = true;
            tf_open(vm, &args[..1], &[])?;
            let f = py::get(vm, this, "f")?;
            let l = opt_str(vm, &lang)?.unwrap_or_else(|| "None".into());
            write(vm, &f, &format!("translate {l} strings:\n"))?;
            write(vm, &f, "\n")?;
        }
        let f = py::get(vm, this, "f")?;
        let fs = vm.call(&filter, vec![s.clone()], Vec::new())?;
        let (s, fs) = (py::string(vm, &s)?, py::string(vm, &fs)?);
        write(vm, &f, &format!("    # {elided}:{line}\n"))?;
        write(vm, &f, &format!("    old \"{}\"\n", quote_unicode(&s)))?;
        write(vm, &f, &format!("    new \"{}\"\n", quote_unicode(&fs)))?;
        write(vm, &f, "\n")?;
    }
    Ok(Value::None)
}

fn translate_command_618(vm: &mut Vm) -> PyResult<Value> {
    let spec = vec![
        Arg::Pos("language", "The language to generate translations for."),
        Arg::Flag("--rot13", "rot13", "Apply rot13 while generating translations."),
        Arg::Flag("--empty", "empty", "Produce empty strings while generating translations."),
    ];
    let args = parse_args(vm, "translate", "Generates or updates translations.", spec)?;
    let filter = if arg_bool(vm, &args, "rot13")? {
        py::import_attr(vm, TL, "rot13_filter")?
    } else if arg_bool(vm, &args, "empty")? {
        py::import_attr(vm, TL, "empty_filter")?
    } else {
        py::import_attr(vm, TL, "null_filter")?
    };
    let language = py::get(vm, &args, "language")?;
    let tfc = py::import_attr(vm, TL, "TranslateFile")?;
    let lst = call(vm, "renpy.loader", "listdirfiles", Vec::new())?;
    for e in py::items(vm, &lst)? {
        let p = py::items(vm, &e)?;
        let Some(dir) = opt_str(vm, &p[0])? else { continue };
        let name = py::string(vm, &p[1])?;
        let f = join(vm, &[&dir, &name])?;
        if !(f.ends_with(".rpy") || f.ends_with(".rpym")) {
            continue;
        }
        let f = normpath(&f);
        vm.call(&tfc, vec![py::s(vm, &f), language.clone(), filter.clone()], Vec::new())?;
    }
    Ok(Value::Bool(false))
}

/// Adds 6.18's translation tools to `renpy.translation`.
pub fn install_618(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, m, n, v);
    for (n, f) in [
        ("scan_strings", f_scan_strings_pairs as Native),
        ("open_tl_file", f_open_tl_file),
        ("null_filter", f_null_filter),
        ("empty_filter", f_empty_filter),
        ("rot13_filter", f_rot13_filter),
        ("translate_command", f_translate_command),
        ("notags_filter", f_notags_filter),
        ("dialogue_command", f_dialogue_command),
    ] {
        let b = py::builtin(vm, n, f);
        set(vm, n, b);
    }
    set(vm, "STRING_RE", py::s(vm, STRING_RE_699));
    let rot = vm.types.new_dict();
    for (a, b) in rot13_table(vm) {
        ops::setitem(vm, &rot, py::s(vm, &a.to_string()), py::s(vm, &b.to_string()))?;
    }
    set(vm, "ROT13", rot);
    let tf = py::class(
        vm,
        TL,
        "TranslateFile",
        &[],
        &[
            ("__init__", tf_init),
            ("open", tf_open),
            ("close", tf_close),
            ("write_translates", tf_write_translates),
            ("write_strings", tf_write_strings),
        ],
        &[],
    )?;
    set(vm, "TranslateFile", Value::Obj(tf));
    let df = dialogue_file_class(vm, TL)?;
    set(vm, "DialogueFile", df);
    let tc = py::builtin(vm, "translate_command", f_translate_command);
    register_command(vm, "translate", tc)?;
    let dc = py::builtin(vm, "dialogue_command", f_dialogue_command);
    register_command(vm, "dialogue", dc)?;
    Ok(())
}

fn dialogue_file_class(vm: &mut Vm, module: &str) -> PyResult<Value> {
    let mut methods: Vec<(&'static str, Native)> = vec![("__init__", df_init)];
    if at_least(vm, 6, 99) {
        methods.push(("write_dialogue", df_write_dialogue));
        methods.push(("get_strings", df_get_strings));
    } else {
        methods.push(("write_translates", df_write_dialogue));
    }
    Ok(Value::Obj(py::class(vm, module, "DialogueFile", &[], &methods, &[])?))
}

// Modules ---------------------------------------------------------------------------------------

fn generation(vm: &mut Vm) -> PyResult<ObjRef> {
    let p = rt(vm).profile.clone();
    let mut funcs: Vec<(&'static str, Native)> = vec![
        ("scan_comments", f_scan_comments_pairs),
        ("open_tl_file", f_open_tl_file),
        ("close_tl_files", f_close_tl_files),
        ("shorten_filename", f_shorten_filename),
        ("write_translates", f_write_translates),
        ("translation_filename", f_translation_filename),
        ("write_strings", f_write_strings),
        ("null_filter", f_null_filter),
        ("empty_filter", f_empty_filter),
        ("generic_filter", f_generic_filter),
        ("rot13_transform", f_rot13_transform),
        ("rot13_filter", f_rot13_filter),
        ("piglatin_transform", f_piglatin_transform),
        ("piglatin_filter", f_piglatin_filter),
        ("translate_list_files", f_translate_list_files),
        ("count_missing", f_count_missing),
        ("translate_command", f_translate_command),
        ("quote_unicode", f_quote_unicode),
    ];
    if p.since_pair(6, 1) {
        funcs.push(("is_empty_extend", f_is_empty_extend));
    }
    if p.at_least(7, 3) {
        funcs.push(("elide_filename", f_elide_filename));
    }
    if !p.at_least(7, 4) {
        funcs.push(("scan_strings", f_scan_strings_pairs));
    }
    let m = crysirror_python::modules::module_with(vm, GEN, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "tl_file_cache", py::dict(vm));
    set(vm, "todo", Value::Bool(true));
    if !p.at_least(7, 4) {
        set(vm, "STRING_RE", py::s(vm, STRING_RE_699));
    }
    let tc = py::get_raw(&m, "translate_command");
    register_command(vm, "translate", tc)?;
    // Upstream's import_all loads the other translation tools with it.
    for sub in [SCAN, DIA, EXTRACT, MERGE] {
        py::import(vm, sub)?;
    }
    Ok(m)
}

fn f_generic_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    let r = super::support::generic_filter(vm, &s, &args[1])?;
    Ok(vm.types.new_str(&r))
}

fn scanstrings(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut funcs: Vec<(&'static str, Native)> = vec![
        ("scan_strings", f_scan_strings),
        ("scan_comments", f_scan_comments_strings),
        ("scan", f_scan),
    ];
    if at_least(vm, 8, 4) {
        funcs.push(("scan_additional_strings", f_scan_additional_strings));
    }
    let m = crysirror_python::modules::module_with(vm, SCAN, &funcs);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let re = string_re(vm);
    set(vm, "STRING_RE", py::s(vm, re));
    let r = priorities(vm, REGULAR_PRIORITIES);
    set(vm, "REGULAR_PRIORITIES", r);
    let c = priorities(vm, COMMON_PRIORITIES);
    set(vm, "COMMON_PRIORITIES", c);
    let s = py::class(vm, SCAN, "String", &[], &[("__init__", string_init), ("__repr__", string_repr)], &[])?;
    set(vm, "String", Value::Obj(s));
    Ok(m)
}

fn dialogue(vm: &mut Vm) -> PyResult<ObjRef> {
    let p = rt(vm).profile.clone();
    let mut funcs: Vec<(&'static str, Native)> = vec![
        ("notags_filter", f_notags_filter),
        ("dialogue_command", f_dialogue_command),
        ("quote_unicode", f_quote_unicode),
    ];
    if p.at_least(7, 0) {
        funcs.push(("create_dialogue_map", f_create_dialogue_map));
    }
    if p.at_least(7, 4) {
        funcs.push(("what_filter", f_what_filter));
    } else {
        funcs.push(("scan_strings", f_scan_strings_pairs));
    }
    if p.since_pair(6, 1) {
        funcs.push(("combine_filter", f_combine_filter));
    }
    let m = crysirror_python::modules::module_with(vm, DIA, &funcs);
    let df = dialogue_file_class(vm, DIA)?;
    crysirror_python::modules::set(vm, &m, "DialogueFile", df);
    let dc = py::get_raw(&m, "dialogue_command");
    register_command(vm, "dialogue", dc)?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    if !at_least(vm, 6, 99) {
        // 6.18 keeps these tools in renpy.translation (install_618).
        return;
    }
    vm.register_native_module(GEN, generation);
    vm.register_native_module(SCAN, scanstrings);
    vm.register_native_module(DIA, dialogue);
    vm.register_native_module(EXTRACT, |vm| {
        let m = crysirror_python::modules::module_with(
            vm,
            EXTRACT,
            &[("extract_strings_core", f_extract_strings_core), ("extract_strings", f_extract_strings)],
        );
        let f = py::get_raw(&m, "extract_strings");
        register_command(vm, "extract_strings", f)?;
        Ok(m)
    });
    vm.register_native_module(MERGE, |vm| {
        let m = crysirror_python::modules::module_with(vm, MERGE, &[("merge_strings", f_merge_strings)]);
        let f = py::get_raw(&m, "merge_strings");
        register_command(vm, "merge_strings", f)?;
        Ok(m)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_follow_upstream() {
        assert_eq!(notags("{b}Hi{/b} [name]", true), "Hi [name]");
        assert_eq!(notags("{{x}", true), "{{x}");
        assert_eq!(notags("{{x}", false), "x");
        assert_eq!(piglatin("Hello string 42 apple"), "Ellohay ringstay 42 appleay");
        assert_eq!(normpath("/game/./a/../b.rpy"), "/game/b.rpy");
        assert_eq!(node_path("game/script.rpy"), "/game/script.rpy");
    }
}
