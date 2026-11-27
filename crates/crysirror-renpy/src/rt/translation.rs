// SPDX-License-Identifier: MPL-2.0
//! `renpy.translation`, after Ren'Py 8.3 `translation/__init__.py` (MIT):
//! the script translator (dialogue lookups over the restructured script,
//! translate blocks per language, string tables), `.rpt` string files and
//! language changes. Restructuring itself happens at load
//! (`crate::translation`); `ScriptTranslator` here is the Python face of
//! the loaded script's translation tables.
use super::{
    nodes,
    py::{self, *},
    state::rt,
};
use crate::ast::NodeKind;
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.translation";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn translator(vm: &mut Vm) -> PyResult<Value> {
    let script = py::import_attr(vm, "renpy.game", "script")?;
    py::get(vm, &script, "translator")
}

fn preferences_language(vm: &mut Vm) -> PyResult<Value> {
    let p = py::import_attr(vm, "renpy.game", "preferences")?;
    if py::is_none(&p) {
        return Ok(Value::None);
    }
    py::get_or_none(vm, &p, "language")
}

fn defaultdict(vm: &mut Vm, factory: Value) -> PyResult<Value> {
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    vm.call(&dd, vec![factory], Vec::new())
}

// TranslateInfo ----------------------------------------------------------------------------------------

fn ti_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let node = &args[1];
    let id = nodes::id_of(node).ok_or_else(|| vm.type_error("translate node expected"))?;
    let r = rt(vm);
    let (lang, ident, filename, line, codes) = {
        let s = r.script.borrow();
        let n = s.arena.get(id);
        let (lang, ident, block) = match &n.kind {
            NodeKind::Translate { language, identifier, block, .. } => (language.clone(), identifier.clone(), block.clone()),
            NodeKind::TranslateSay { language, identifier, .. } => (language.clone(), identifier.clone(), vec![id]),
            _ => (None, String::new(), Vec::new()),
        };
        let codes: Vec<String> = block
            .iter()
            .filter_map(|b| {
                let k = &s.arena.get(*b).kind;
                match k {
                    NodeKind::TranslateSay { say, .. } => crate::translation::get_code(say),
                    k => crate::translation::get_code(k),
                }
            })
            .collect();
        (lang, ident, n.filename.to_string(), n.linenumber, codes)
    };
    py::set(vm, this, "language", lang.map(|l| py::s(vm, &l)).unwrap_or(Value::None))?;
    py::set(vm, this, "identifier", py::s(vm, &ident))?;
    py::set(vm, this, "filename", py::s(vm, &filename))?;
    py::set(vm, this, "linenumber", Value::Int(line as i64))?;
    let src: Vec<Value> = codes.iter().map(|c| vm.types.new_str(c)).collect();
    py::set(vm, this, "source", vm.types.new_list(src))?;
    Ok(Value::None)
}

// ScriptTranslator -------------------------------------------------------------------------------------

fn st_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "languages", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "default_translates", vm.types.new_dict())?;
    py::set(vm, this, "language_translates", vm.types.new_dict())?;
    py::set(vm, this, "chain_worklist", vm.types.new_list(Vec::new()))?;
    let list = Value::Obj(vm.types.list.clone());
    for n in ["file_translates", "block", "early_block", "python", "additional_strings"] {
        let d = defaultdict(vm, list.clone())?;
        py::set(vm, this, n, d)?;
    }
    let stc = mget(vm, "StringTranslator")?;
    let strings = defaultdict(vm, stc)?;
    py::set(vm, this, "strings", strings)?;
    // Languages with a `tl/<language>/` directory.
    let r = rt(vm);
    let langs = py::get(vm, this, "languages")?;
    for f in r.files.list() {
        let parts: Vec<&str> = f.split('/').collect();
        if parts.len() >= 3 && parts[0] == "tl" && parts[1] != "None" {
            ops::set_add(vm, &langs, py::s(vm, parts[1]))?;
        }
    }
    Ok(Value::None)
}

fn append_to(vm: &mut Vm, this: &Value, attr: &str, key: Value, v: Value) -> PyResult<()> {
    let d = py::get(vm, this, attr)?;
    let l = ops::getitem(vm, &d, key)?;
    ops::list_append(vm, &l, v)
}

/// Fills the translator from the loaded script (`take_translates` of every
/// file, then `chain_translates`).
fn st_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let r = rt(vm);
    let unchained = r.script.borrow_mut().chain_translates();
    let ids: Vec<usize> = (0..r.script.borrow().arena.len()).collect();
    let langs = py::get(vm, this, "languages")?;
    let dt = py::get(vm, this, "default_translates")?;
    let lt = py::get(vm, this, "language_translates")?;
    let old_subs = py::config_bool(vm, "old_substitutions").unwrap_or(false);
    for id in ids {
        let (kind, filename, line) = {
            let s = r.script.borrow();
            let n = s.arena.get(id);
            (n.kind.clone(), n.filename.to_string(), n.linenumber)
        };
        let lang_v = |vm: &Vm, l: &Option<String>| l.as_ref().map(|l| vm.types.new_str(l)).unwrap_or(Value::None);
        match &kind {
            NodeKind::TranslatePython { language, .. } | NodeKind::TranslateEarlyBlock { language, .. } | NodeKind::TranslateBlock { language, .. } => {
                if let Some(l) = language {
                    ops::set_add(vm, &langs, py::s(vm, l))?;
                }
                let attr = match kind {
                    NodeKind::TranslatePython { .. } => "python",
                    NodeKind::TranslateEarlyBlock { .. } => "early_block",
                    _ => "block",
                };
                let h = nodes::handle(vm, id)?;
                let lv = lang_v(vm, language);
                append_to(vm, this, attr, lv, h)?;
            }
            NodeKind::Menu { items, .. } => {
                for it in items {
                    let s = if old_subs { it.label.replace("%%", "%") } else { it.label.clone() };
                    let t = vm.types.new_tuple(vec![Value::Int(line as i64), py::s(vm, &s)]);
                    let f = py::s(vm, &filename);
                    append_to(vm, this, "additional_strings", f, t)?;
                }
            }
            NodeKind::Translate { identifier, language, .. } | NodeKind::TranslateSay { identifier, language, .. } => {
                let h = nodes::handle(vm, id)?;
                match language {
                    None => {
                        ops::setitem(vm, &dt, py::s(vm, identifier), h.clone())?;
                        let t = vm.types.new_tuple(vec![Value::None, h]);
                        let f = py::s(vm, &filename);
                        append_to(vm, this, "file_translates", f, t)?;
                    }
                    Some(l) => {
                        ops::set_add(vm, &langs, py::s(vm, l))?;
                        let k = vm.types.new_tuple(vec![py::s(vm, identifier), py::s(vm, l)]);
                        ops::setitem(vm, &lt, k, h)?;
                    }
                }
            }
            _ => {}
        }
    }
    let cw = py::get(vm, this, "chain_worklist")?;
    for (i, l) in unchained {
        let t = vm.types.new_tuple(vec![py::s(vm, &i), py::s(vm, &l)]);
        ops::list_append(vm, &cw, t)?;
    }
    Ok(Value::None)
}

fn st_count_translates(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(rt(vm).script.borrow().translates.len() as i64))
}

fn st_take_translates(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    // Translation tables are filled from the loaded arena (`load`).
    Ok(Value::None)
}

fn st_chain_translates(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    rt(vm).script.borrow_mut().chain_translates();
    Ok(Value::None)
}

/// `lookup_translate(identifier, alternate)`.
pub fn lookup_translate(vm: &mut Vm, identifier: &str, alternate: Option<&str>) -> PyResult<Value> {
    let identifier = identifier.replace('.', "_");
    let language = preferences_language(vm)?;
    let r = rt(vm);
    let found = {
        let s = r.script.borrow();
        let mut tl = None;
        if let Some(lang) = language.as_str() {
            tl = s.language_translates.get(&(identifier.clone(), lang.to_owned())).copied();
            if tl.is_none() {
                if let Some(a) = alternate {
                    tl = s.language_translates.get(&(a.to_owned(), lang.to_owned())).copied();
                }
            }
        }
        tl.or_else(|| s.translates.get(&identifier).copied())
    };
    let Some(tl) = found else {
        return Err(vm.exc_msg("KeyError", &identifier));
    };
    let first = {
        let s = r.script.borrow();
        match &s.arena.get(tl).kind {
            NodeKind::TranslateSay { .. } => Some(tl),
            NodeKind::Translate { block, .. } => block.first().copied(),
            _ => Some(tl),
        }
    };
    nodes::opt_handle(vm, first)
}

fn st_lookup_translate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let id = py::string(vm, &args[1])?;
    let alt = arg_or(args, kw, 2, "alternate", Value::None);
    let alt = alt.as_str().map(|s| s.to_owned());
    lookup_translate(vm, &id, alt.as_deref())
}

fn st_get_translate_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let id = py::string(vm, &args[1])?.replace('.', "_");
    let language = args.get(2).cloned().unwrap_or(Value::None);
    let r = rt(vm);
    let tl = {
        let s = r.script.borrow();
        match language.as_str() {
            Some(l) => s.language_translates.get(&(id, l.to_owned())).copied(),
            None => s.translates.get(&id).copied(),
        }
    };
    let Some(tl) = tl else { return Ok(Value::None) };
    let h = nodes::handle(vm, tl)?;
    let c = mget(vm, "TranslateInfo")?;
    vm.call(&c, vec![h], Vec::new())
}

// Strings ----------------------------------------------------------------------------------------------

fn f_encode_say_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(py::s(vm, &crate::lexer::encode_say_string(&s)))
}

pub fn quote_unicode(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\u{7}', "\\a").replace('\u{8}', "\\b").replace('\u{c}', "\\f").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t").replace('\u{b}', "\\v")
}

fn f_quote_unicode(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args[0])?;
    Ok(py::s(vm, &quote_unicode(&s)))
}

fn sts_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "translations", vm.types.new_dict())?;
    py::set(vm, &args[0], "translation_loc", vm.types.new_dict())?;
    py::set(vm, &args[0], "unknown", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn sts_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, old, new, newloc) = (&args[0], &args[1], &args[2], args.get(3).cloned().unwrap_or(Value::None));
    let tr = py::get(vm, this, "translations")?;
    let loc = py::get(vm, this, "translation_loc")?;
    if ops::contains(vm, &tr, old)? {
        let o = py::string(vm, old)?;
        if ops::contains(vm, &loc, old)? {
            let l = ops::getitem(vm, &loc, old.clone())?;
            let p = py::items(vm, &l)?;
            let f = py::string(vm, &p[0])?;
            let n = py::int(vm, &p[1])?;
            return Err(py::error(vm, format!("A translation for \"{}\" already exists at {f}:{n}.", quote_unicode(&o))));
        }
        return Err(py::error(vm, format!("A translation for \"{}\" already exists.", quote_unicode(&o))));
    }
    ops::setitem(vm, &tr, old.clone(), new.clone())?;
    if !py::is_none(&newloc) {
        ops::setitem(vm, &loc, old.clone(), newloc)?;
    }
    Ok(Value::None)
}

fn sts_translate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, old) = (&args[0], &args[1]);
    let tr = py::get(vm, this, "translations")?;
    let new = vm.call_method(&tr, "get", vec![old.clone(), Value::None])?;
    if !py::is_none(&new) {
        return Ok(new);
    }
    // Strings that differ only in {#...} disambiguation tags share one.
    let s = py::string(vm, old)?;
    let mut notags = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("{#") {
        notags.push_str(&rest[..i]);
        match rest[i..].find('}') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                notags.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    notags.push_str(rest);
    let new = vm.call_method(&tr, "get", vec![py::s(vm, &notags), Value::None])?;
    if !py::is_none(&new) {
        return Ok(new);
    }
    Ok(old.clone())
}

fn sts_write_updated_strings(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    // `RENPY_UPDATE_STRINGS` is a developer workflow; no strings are
    // collected, so there is nothing to write.
    Ok(Value::None)
}

fn add_string_translation(vm: &mut Vm, language: Value, old: Value, new: Value, newloc: Value) -> PyResult<()> {
    let tl = translator(vm)?;
    let strings = py::get(vm, &tl, "strings")?;
    let stl = ops::getitem(vm, &strings, language.clone())?;
    let langs = py::get(vm, &tl, "languages")?;
    ops::set_add(vm, &langs, language)?;
    vm.call_method(&stl, "add", vec![old, new, newloc])?;
    Ok(())
}

fn f_add_string_translation(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    add_string_translation(vm, args[0].clone(), args[1].clone(), args[2].clone(), args.get(3).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

/// `translate_string(s, language=Default)`.
pub fn translate_string(vm: &mut Vm, s: &Value, language: Option<Value>) -> PyResult<Value> {
    let language = match language {
        Some(l) => l,
        None => preferences_language(vm)?,
    };
    let tl = translator(vm)?;
    let strings = py::get(vm, &tl, "strings")?;
    let stl = ops::getitem(vm, &strings, language)?;
    vm.call_method(&stl, "translate", vec![s.clone()])
}

fn f_translate_string(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let default = mget(vm, "Default")?;
    let lang = arg_or(args, kw, 1, "language", default.clone());
    let lang = if py::same(&lang, &default) { None } else { Some(lang) };
    translate_string(vm, &args[0], lang)
}

fn f_write_updated_strings(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// `load_rpt(fn)`: `<` old / `>` new string pairs.
fn load_rpt(vm: &mut Vm, name: &str) -> PyResult<()> {
    let r = rt(vm);
    let Some(bytes) = r.files.read(name) else {
        return Err(vm.exc_msg("IOError", &format!("Couldn't find file '{name}'.")));
    };
    let text = String::from_utf8_lossy(&bytes);
    let language = name.rsplit('/').next().unwrap_or(name).trim_end_matches(".rpt").to_owned();
    let unquote = |s: &str| s.replace("\\n", "\n").replace("\\\\", "\\");
    let mut old: Option<String> = None;
    for l in text.lines() {
        let l = l.trim_end();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let s = unquote(l.get(2..).unwrap_or(""));
        if l.starts_with('<') {
            if let Some(o) = &old {
                return Err(py::error(vm, format!("{language} string {o:?} does not have a translation.")));
            }
            old = Some(s);
        } else if l.starts_with('>') {
            let Some(o) = old.take() else {
                return Err(py::error(vm, format!("{language} translation {s:?} doesn't belong to a string.")));
            };
            let lv = py::s(vm, &language);
            let ov = py::s(vm, &o);
            let sv = py::s(vm, &s);
            add_string_translation(vm, lv, ov, sv, Value::None)?;
        }
    }
    if let Some(o) = old {
        return Err(py::error(vm, format!("{language} string {o:?} does not have a translation.")));
    }
    Ok(())
}

fn f_load_rpt(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = py::string(vm, &args[0])?;
    load_rpt(vm, &n)?;
    Ok(Value::None)
}

fn f_load_all_rpts(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let r = rt(vm);
    let mut names: Vec<String> = r.files.list().into_iter().filter(|n| n.ends_with(".rpt")).collect();
    names.sort();
    for n in names {
        load_rpt(vm, &n)?;
    }
    Ok(Value::None)
}

fn f_init_translation(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let backup = py::import_attr(vm, "renpy.style", "backup")?;
    let b = vm.call(&backup, Vec::new(), Vec::new())?;
    let m = py::import(vm, MODULE)?;
    py::set(vm, &m, "style_backup", b)?;
    f_load_all_rpts(vm, &[], &[])
}

// Language changes -------------------------------------------------------------------------------------

fn run_blocks(vm: &mut Vm, tl: &Value, attr: &str, language: &Value) -> PyResult<()> {
    let d = py::get(vm, tl, attr)?;
    let blocks = ops::getitem(vm, &d, language.clone())?;
    let blocks = py::items(vm, &blocks)?;
    if blocks.is_empty() {
        return Ok(());
    }
    let f = py::closure(vm, "run_blocks", run_blocks_in_context, vm.types.new_list(blocks));
    let inv = py::import_attr(vm, "renpy.game", "invoke_in_new_context")?;
    vm.call(&inv, vec![f], Vec::new())?;
    Ok(())
}

fn run_blocks_in_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cf = py::import_attr(vm, "renpy.game", "context")?;
    for b in py::items(vm, &args[0])? {
        let block = py::get(vm, &b, "block")?;
        let first = ops::getitem(vm, &block, Value::Int(0))?;
        let ctx = vm.call(&cf, Vec::new(), Vec::new())?;
        vm.call_method(&ctx, "run", vec![first])?;
    }
    Ok(Value::None)
}

fn run_python(vm: &mut Vm, tl: &Value, language: &Value) -> PyResult<()> {
    let d = py::get(vm, tl, "python")?;
    let nodes_v = ops::getitem(vm, &d, language.clone())?;
    for n in py::items(vm, &nodes_v)? {
        let Some(id) = nodes::id_of(&n) else { continue };
        let r = rt(vm);
        let code = match &r.script.borrow().arena.get(id).kind {
            NodeKind::TranslatePython { code, .. } => Some(code.clone()),
            _ => None,
        };
        if let Some(code) = code {
            let c = super::python::compile(vm, &code.source.text, "exec", &code.source.filename, code.source.line.max(1), code.py3.then_some(3))?;
            super::python::exec_bytecode(vm, c, false, &Value::None, &Value::None, "store")?;
        }
    }
    Ok(())
}

fn language_callbacks(vm: &mut Vm, language: &Value) -> PyResult<()> {
    let lc = py::config(vm, "language_callbacks")?;
    let cbs = ops::getitem(vm, &lc, language.clone())?;
    for cb in py::items(vm, &cbs)? {
        vm.call(&cb, Vec::new(), Vec::new())?;
    }
    Ok(())
}

fn deferred_styles(vm: &mut Vm) -> PyResult<()> {
    let ds = mget(vm, "deferred_styles")?;
    for s in py::items(vm, &ds)? {
        let Some(id) = nodes::id_of(&s) else { continue };
        let r = rt(vm);
        let style = match &r.script.borrow().arena.get(id).kind {
            NodeKind::Style(st) => Some(st.clone()),
            _ => None,
        };
        if let Some(st) = style {
            super::astexec::apply_style_node(vm, &st)?;
        }
    }
    Ok(())
}

fn old_change_language(vm: &mut Vm, tl: &Value, language: &Value) -> PyResult<()> {
    deferred_styles(vm)?;
    run_blocks(vm, tl, "early_block", language)?;
    run_blocks(vm, tl, "block", language)?;
    run_python(vm, tl, language)?;
    language_callbacks(vm, language)
}

fn new_change_language(vm: &mut Vm, tl: &Value, language: &Value) -> PyResult<()> {
    run_python(vm, tl, language)?;
    run_blocks(vm, tl, "early_block", language)?;
    language_callbacks(vm, language)?;
    deferred_styles(vm)?;
    run_blocks(vm, tl, "block", language)?;
    let iss = py::config(vm, "init_system_styles")?;
    if !py::is_none(&iss) {
        vm.call(&iss, Vec::new(), Vec::new())?;
    }
    Ok(())
}

fn f_clean_data(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let store = py::import(vm, "store")?;
    let list = py::get(vm, &store, "list")?;
    for n in ["_history_list", "nvl_list"] {
        let l = vm.call(&list, Vec::new(), Vec::new())?;
        py::set(vm, &store, n, l)?;
    }
    let log = py::import_attr(vm, "renpy.game", "log")?;
    if !py::is_none(&log) {
        py::set(vm, &log, "forward", vm.types.new_list(Vec::new()))?;
    }
    Ok(Value::None)
}

fn exports(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.exports", name)?;
    vm.call(&f, args, Vec::new())
}

fn f_change_language(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let language = args[0].clone();
    let force = py::truthy(vm, &arg_or(args, kw, 1, "force", Value::Bool(false)))?;
    exports(vm, "load_language", vec![language.clone()])?;
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    py::set(vm, &prefs, "language", language.clone())?;
    let old = mget(vm, "old_language")?;
    if ops::eq(vm, &old, &language)? && !force {
        return Ok(Value::None);
    }
    let tl = translator(vm)?;
    let sb = mget(vm, "style_backup")?;
    let restore = py::import_attr(vm, "renpy.style", "restore")?;
    vm.call(&restore, vec![sb], Vec::new())?;
    let rebuild = py::import_attr(vm, "renpy.style", "rebuild")?;
    vm.call(&rebuild, Vec::new(), Vec::new())?;
    let tcs = py::config(vm, "translate_clean_stores")?;
    let clean = py::import_attr(vm, "renpy.python", "clean_store")?;
    for s in py::items(vm, &tcs)? {
        vm.call(&clean, vec![s], Vec::new())?;
    }
    if py::config_bool(vm, "new_translate_order").unwrap_or(false) {
        new_change_language(vm, &tl, &language)?;
    } else {
        old_change_language(vm, &tl, &language)?;
    }
    let clc = py::config(vm, "change_language_callbacks")?;
    for cb in py::items(vm, &clc).unwrap_or_default() {
        vm.call(&cb, Vec::new(), Vec::new())?;
    }
    exports(vm, "free_memory", Vec::new())?;
    vm.call(&rebuild, Vec::new(), Vec::new())?;
    if let Ok(ti) = py::import_attr(vm, "renpy.display.tts", "init") {
        vm.call(&ti, Vec::new(), Vec::new())?;
    }
    let reset = py::import_attr(vm, "renpy.python", "reset_store_changes")?;
    for s in py::items(vm, &tcs)? {
        vm.call(&reset, vec![s], Vec::new())?;
    }
    exports(vm, "restart_interaction", Vec::new())?;
    if !ops::eq(vm, &language, &old)? {
        exports(vm, "block_rollback", Vec::new())?;
        let m = py::import(vm, MODULE)?;
        py::set(vm, &m, "old_language", language)?;
    }
    Ok(Value::None)
}

fn f_check_language(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    let ctx = ops::getitem(vm, &contexts, Value::Int(-1))?;
    let lang = preferences_language(vm)?;
    let tlang = py::get_or_none(vm, &ctx, "translate_language")?;
    if ops::eq(vm, &tlang, &lang)? {
        return Ok(Value::None);
    }
    f_clean_data(vm, &[], &[])?;
    py::set(vm, &ctx, "translate_language", lang)?;
    let mut tid = py::get_or_none(vm, &ctx, "translate_identifier")?;
    if !py::truthy(vm, &tid)? {
        tid = py::get_or_none(vm, &ctx, "deferred_translate_identifier")?;
    }
    if py::is_none(&tid) {
        return Ok(Value::None);
    }
    let t = py::string(vm, &tid)?;
    let node = lookup_translate(vm, &t, None)?;
    if !py::is_none(&node) {
        py::set(vm, &ctx, "translate_identifier", tid)?;
        f_clean_data(vm, &[], &[])?;
        let name = py::get(vm, &node, "name")?;
        let je = py::import_attr(vm, "renpy.game", "JumpException")?;
        let e = vm.call(&je, vec![name], Vec::new())?;
        return Err(PyErr::Exc(e.obj().cloned().unwrap()));
    }
    Ok(Value::None)
}

fn f_get_translation_identifier(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let contexts = py::import_attr(vm, "renpy.game", "contexts")?;
    let ctx = ops::getitem(vm, &contexts, Value::Int(-1))?;
    let t = py::get_or_none(vm, &ctx, "translate_identifier")?;
    if py::truthy(vm, &t)? {
        return Ok(t);
    }
    py::get_or_none(vm, &ctx, "deferred_translate_identifier")
}

fn f_get_translation_info(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut identifier = arg_or(args, kw, 0, "identifier", Value::None);
    let language = arg_or(args, kw, 1, "language", Value::None);
    if py::is_none(&identifier) {
        identifier = f_get_translation_identifier(vm, &[], &[])?;
    }
    if py::is_none(&identifier) {
        return Ok(Value::None);
    }
    let tl = translator(vm)?;
    vm.call_method(&tl, "get_translate_info", vec![identifier, language])
}

fn f_known_languages(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let tl = translator(vm)?;
    let langs = py::get(vm, &tl, "languages")?;
    let rv = vm.types.new_set(Default::default());
    for l in ops::collect(vm, &langs)? {
        if !py::is_none(&l) {
            ops::set_add(vm, &rv, l)?;
        }
    }
    Ok(rv)
}

/// The host locale is not exposed to the game.
fn f_detect_user_locale(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_tuple(vec![Value::None, Value::None]))
}

const LOCALES: &str = "ab:abkhazian aa:afar af:afrikaans ak:akan sq:albanian am:amharic ar:arabic an:aragonese hy:armenian as:assamese av:avaric ae:avestan ay:aymara az:azerbaijani bm:bambara ba:bashkir eu:basque be:belarusian bn:bengali bh:bihari bi:bislama bs:bosnian br:breton bg:bulgarian my:burmese ca:catalan ch:chamorro ce:chechen ny:chewa cv:chuvash kw:cornish co:corsican cr:cree hr:croatian cs:czech da:danish dv:maldivian nl:dutch dz:dzongkha en:english et:estonian ee:ewe fo:faroese fj:fijian fi:finnish fr:french ff:fulah gl:galician ka:georgian de:german el:greek gn:guaran gu:gujarati ht:haitian ha:hausa he:hebrew hz:herero hi:hindi ho:hiri_motu hu:hungarian id:indonesian ga:irish ig:igbo ik:inupiaq is:icelandic it:italian iu:inuktitut ja:japanese jv:javanese kl:greenlandic kn:kannada kr:kanuri ks:kashmiri kk:kazakh km:khmer ki:kikuyu rw:kinyarwanda ky:kirghiz kv:komi kg:kongo ko:korean ku:kurdish kj:kuanyama la:latin lb:luxembourgish lg:ganda li:limburgan ln:lingala lo:lao lt:lithuanian lv:latvian gv:manx mk:macedonian mg:malagasy ms:malay ml:malayalam mt:maltese mi:maori mr:marathi mh:marshallese mn:mongolian na:nauru nv:navaho ne:nepali ng:ndonga no:norwegian ii:nuosu nr:ndebele oc:occitan oj:ojibwa om:oromo or:oriya os:ossetian pa:panjabi pi:pali fa:persian pl:polish ps:pashto pt:portuguese qu:quechua rm:romansh rn:rundi ro:romanian ru:russian sa:sanskrit sc:sardinian sd:sindhi se:sami sm:samoan sg:sango sr:serbian gd:gaelic sn:shona si:sinhala sk:slovak sl:slovene so:somali st:sotho es:spanish su:sundanese sw:swahili ss:swati sv:swedish ta:tamil te:telugu tg:tajik th:thai ti:tigrinya bo:tibetan tk:turkmen tl:tagalog tn:tswana to:tongan tr:turkish ts:tsonga tt:tatar tw:twi ty:tahitian ug:uighur uk:ukrainian ur:urdu uz:uzbek ve:venda vi:vietnamese wa:walloon cy:welsh wo:wolof fy:frisian xh:xhosa yi:yiddish yo:yoruba za:zhuang zu:zulu chs:simplified_chinese cht:traditional_chinese zh:traditional_chinese zh_tw:traditional_chinese zh_cn:simplified_chinese zh_hk:traditional_chinese zh_sg:simplified_chinese zh_mo:traditional_chinese";

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("encode_say_string", f_encode_say_string),
        ("quote_unicode", f_quote_unicode),
        ("add_string_translation", f_add_string_translation),
        ("translate_string", f_translate_string),
        ("write_updated_strings", f_write_updated_strings),
        ("load_rpt", f_load_rpt),
        ("load_all_rpts", f_load_all_rpts),
        ("init_translation", f_init_translation),
        ("clean_data", f_clean_data),
        ("change_language", f_change_language),
        ("check_language", f_check_language),
        ("get_translation_identifier", f_get_translation_identifier),
        ("get_translation_info", f_get_translation_info),
        ("known_languages", f_known_languages),
        ("detect_user_locale", f_detect_user_locale),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let obj = Value::Obj(vm.types.object.clone());
    let ti = py::class(vm, MODULE, "TranslateInfo", &[obj.clone()], &[("__init__", ti_init)], &[])?;
    set(vm, "TranslateInfo", Value::Obj(ti));
    let sts = py::class(vm, MODULE, "StringTranslator", &[obj.clone()], &[("__init__", sts_init), ("add", sts_add), ("translate", sts_translate), ("write_updated_strings", sts_write_updated_strings)], &[])?;
    set(vm, "StringTranslator", Value::Obj(sts));
    let st = py::class(vm, MODULE, "ScriptTranslator", &[obj], &[
        ("__init__", st_init),
        ("load", st_load),
        ("count_translates", st_count_translates),
        ("take_translates", st_take_translates),
        ("chain_translates", st_chain_translates),
        ("lookup_translate", st_lookup_translate),
        ("get_translate_info", st_get_translate_info),
    ], &[])?;
    set(vm, "ScriptTranslator", Value::Obj(st));
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let d = vm.call(&sentinel, vec![py::s(vm, "default")], Vec::new())?;
    set(vm, "Default", d);
    set(vm, "update_translations", Value::Bool(false));
    set(vm, "style_backup", Value::None);
    set(vm, "old_language", py::s(vm, "language never set"));
    set(vm, "deferred_styles", vm.types.new_list(Vec::new()));
    let locales = vm.types.new_dict();
    for pair in LOCALES.split(' ') {
        if let Some((k, v)) = pair.split_once(':') {
            py::dict_set(vm, &locales, k, py::s(vm, v))?;
        }
    }
    set(vm, "locales", locales);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
