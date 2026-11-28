// SPDX-License-Identifier: MPL-2.0
//! Script loading (`renpy/script.py`): file discovery and order, `.rpyc`
//! preference, chaining, name map, init code collection and early
//! execution hooks.

use crate::ast::*;
use crate::files::GameFiles;
use crate::lexer;
use crate::parser::{Parser, ScriptContext};
use crate::rpyc;
use crysirror_python::Vm;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
pub struct Script {
    pub arena: Arena,
    pub namemap: HashMap<NodeName, NodeId>,
    pub labels: HashMap<String, NodeId>,
    /// (priority, Init node or init-producing node), stable-sorted.
    pub initcode: Vec<(i32, NodeId)>,
    pub files: Vec<String>,
    pub duplicate_labels: Vec<String>,
    /// Translate identifier -> language -> block (`translator.take_translates`).
    pub translates: HashMap<String, NodeId>,
    pub language_translates: HashMap<(String, String), NodeId>,
    pub serial: i64,
    pub errors: Vec<String>,
}

pub enum Source {
    Rpyc(Vec<u8>),
    Rpy(String),
}

/// One script file in load order.
pub struct ScriptFile {
    pub name: String,
    pub common: bool,
    pub source: Source,
}

/// `scan_script_files` + `load_appropriate_file` choice: `.rpyc` from
/// archives and loose files. Prefer matching compiled bytes over loose source.
pub fn discover(files: &dyn GameFiles) -> Result<Vec<ScriptFile>, String> {
    discover_interruptible(files, || Ok(()))
}

/// `discover` for a release: `_ren.py` scripts load from 7.6 and 8.1.
pub fn discover_release(files: &dyn GameFiles, ren_py: bool) -> Result<Vec<ScriptFile>, String> {
    discover_with(files, ren_py, || Ok(()))
}

pub(crate) fn discover_interruptible(
    files: &dyn GameFiles,
    check: impl Fn() -> Result<(), String>,
) -> Result<Vec<ScriptFile>, String> {
    discover_with(files, false, check)
}

fn discover_with(
    files: &dyn GameFiles,
    ren_py: bool,
    check: impl Fn() -> Result<(), String>,
) -> Result<Vec<ScriptFile>, String> {
    let mut entries = Vec::new();
    let mut conflict = None;
    let mut add = |list: Vec<String>, common: bool, entries: &mut Vec<_>| {
        let mut by_stem: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for n in list {
            if let Some(stem) = n.strip_suffix(".rpyc") {
                by_stem.entry(stem.to_string()).or_default().1 = Some(n.clone());
            } else if let Some(stem) = n
                .strip_suffix(".rpy")
                .or_else(|| n.strip_suffix("_ren.py").filter(|_| ren_py))
            {
                let e = by_stem.entry(stem.to_string()).or_default();
                if let Some(other) = &e.0 {
                    conflict.get_or_insert_with(|| {
                        format!("{other} and {n} conflict, and can't exist in the same game.")
                    });
                }
                e.0 = Some(n.clone());
            }
        }
        for (stem, (rpy, rpyc)) in by_stem {
            entries.push((stem, common, rpy, rpyc));
        }
    };
    add(files.list(), false, &mut entries);
    add(files.common_list(), true, &mut entries);
    if let Some(c) = conflict {
        return Err(c);
    }
    // Ren'Py sorts (fn, dir); archive members have dir None (sorts first in
    // Python 2), common files carry the common dir, game files the game dir.
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    entries.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for (_stem, common, source, compiled) in entries {
        check()?;
        let file = select_script(files, common, source.as_deref(), compiled.as_deref())?;
        let size = match &file.source {
            Source::Rpyc(raw) => raw.len(),
            Source::Rpy(text) => text.len(),
        };
        bytes = bytes
            .checked_add(size)
            .filter(|bytes| *bytes <= 128 * 1024 * 1024)
            .ok_or("Ren'Py script input exceeds memory limit")?;
        out.push(file);
    }
    Ok(out)
}

fn select_script(
    files: &dyn GameFiles,
    common: bool,
    source: Option<&str>,
    compiled: Option<&str>,
) -> Result<ScriptFile, String> {
    let prefix = if common { "renpy/common" } else { "game" };
    let read = |path: &str| {
        let data = if common {
            files.common_read(path)
        } else {
            files.read(path)
        };
        data.ok_or_else(|| format!("unable to read Ren'Py script {prefix}/{path}"))
    };
    let source = source
        .map(|path| {
            let bytes = read(path)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("{prefix}/{path}: {e}"))?;
            // Python 2's rU reads both CRLF and CR as LF before hashing.
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            Ok::<_, String>((path, text))
        })
        .transpose()?;
    // The digest covers the file; an `_ren.py` file parses as its `.rpy` form.
    let source = source
        .map(|(path, text)| {
            if path.ends_with("_ren.py") {
                let rpy = crate::lexer::ren_py_to_rpy(&text, Some(&format!("{prefix}/{path}")))?;
                Ok::<_, String>((path, text, Some(rpy)))
            } else {
                Ok((path, text, None))
            }
        })
        .transpose()?;
    if let Some(path) = compiled {
        let data = read(path)?;
        let current = source.as_ref().is_none_or(|(_, text, _)| {
            data.last_chunk::<16>()
                == Some(&crysirror_resource::checksum::md5_legacy(text.as_bytes()))
        });
        if current {
            return Ok(ScriptFile {
                name: format!("{prefix}/{path}"),
                common,
                source: Source::Rpyc(data),
            });
        }
    }
    let (path, text, rpy) = source.ok_or("Ren'Py script has neither source nor compiled bytes")?;
    Ok(ScriptFile {
        name: format!("{prefix}/{path}"),
        common,
        source: Source::Rpy(rpy.unwrap_or(text)),
    })
}

/// `name.rpym` / `name.rpymc`, from the game first, then the common files.
pub fn find_module(files: &dyn GameFiles, name: &str) -> Result<Option<ScriptFile>, String> {
    let compiled = format!("{name}.rpymc");
    let source = format!("{name}.rpym");
    for (common, names) in [(false, files.list()), (true, files.common_list())] {
        let has_source = names.contains(&source).then_some(source.as_str());
        let has_compiled = names.contains(&compiled).then_some(compiled.as_str());
        if has_source.is_some() || has_compiled.is_some() {
            return select_script(files, common, has_source, has_compiled).map(Some);
        }
    }
    Ok(None)
}

/// The native bootstrap loads these modules before any ordinary scripts.
pub(crate) fn bootstrap_modules(files: &dyn GameFiles) -> Result<Vec<ScriptFile>, String> {
    let game_names = files
        .list()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let common_names = files
        .common_list()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut modules = Vec::new();
    for name in ["_errorhandling", "tl/None/common"] {
        let compiled = format!("{name}.rpymc");
        let source = format!("{name}.rpym");
        for (common, names) in [(false, &game_names), (true, &common_names)] {
            let has_source = names.contains(&source).then_some(source.as_str());
            let has_compiled = names.contains(&compiled).then_some(compiled.as_str());
            if has_source.is_some() || has_compiled.is_some() {
                modules.push(select_script(files, common, has_source, has_compiled)?);
                break;
            }
        }
    }
    Ok(modules)
}

impl Script {
    /// Loads one file's top-level block, returning it chained.
    pub fn load_file(
        &mut self,
        vm: &mut Vm,
        shells: &mut rpyc::Shells,
        cx: &dyn ScriptContext,
        registry: &crate::sl2::Registry,
        f: &ScriptFile,
    ) -> Vec<NodeId> {
        let stmts = match &f.source {
            Source::Rpyc(bytes) => match rpyc::unpickle_script(vm, shells, bytes) {
                Ok((_data, objs)) => {
                    let mut c = rpyc::Converter::new(vm, &mut self.arena, registry);
                    let v = c.convert_block(&objs);
                    self.errors.extend(c.errors);
                    v
                }
                Err(e) => {
                    self.errors.push(format!("{}: {e}", f.name));
                    return Vec::new();
                }
            },
            Source::Rpy(text) => match lexer::lex_file(&f.name, text) {
                Ok(block) => {
                    let mut p = Parser::new(&mut self.arena, cx);
                    let v = p.parse_file(block, &f.name, text.lines().count() as u32);
                    for e in p.errors {
                        self.errors.push(e.to_string());
                    }
                    let start = self.serial;
                    self.assign_names(&v, &f.name, start);
                    crate::translation::restructure(&mut self.arena, &v, &self.translates)
                }
                Err(e) => {
                    self.errors.push(e.to_string());
                    return Vec::new();
                }
            },
        };
        self.arena.chain_block(&stmts, None);
        self.files.push(f.name.clone());
        stmts
    }

    /// Converts a compiled file's top-level block (unchained).
    pub fn load_rpyc(
        &mut self,
        vm: &mut Vm,
        shells: &mut rpyc::Shells,
        registry: &crate::sl2::Registry,
        f: &ScriptFile,
        bytes: &[u8],
    ) -> Vec<NodeId> {
        match rpyc::unpickle_script(vm, shells, bytes) {
            Ok((_data, objs)) => {
                let mut c = rpyc::Converter::new(vm, &mut self.arena, registry);
                let v = c.convert_block(&objs);
                self.errors.extend(c.errors);
                v
            }
            Err(e) => {
                self.errors.push(format!("{}: {e}", f.name));
                Vec::new()
            }
        }
    }

    /// Parses a source file (names not yet assigned); `None` after a lexer
    /// error.
    pub fn parse_rpy(
        &mut self,
        cx: &dyn ScriptContext,
        name: &str,
        text: &str,
    ) -> Option<Vec<NodeId>> {
        match lexer::lex_file(name, text) {
            Ok(block) => {
                let mut p = Parser::new(&mut self.arena, cx);
                let v = p.parse_file(block, name, text.lines().count() as u32);
                for e in p.errors {
                    self.errors.push(e.to_string());
                }
                Some(v)
            }
            Err(e) => {
                self.errors.push(e.to_string());
                None
            }
        }
    }

    /// Names a parsed source file's nodes and restructures its
    /// translations (`assign_names`, `Restructurer`).
    pub fn finish_rpy(&mut self, stmts: &[NodeId], name: &str) -> Vec<NodeId> {
        let start = self.serial;
        self.assign_names(stmts, name, start);
        crate::translation::restructure(&mut self.arena, stmts, &self.translates)
    }

    /// Chains a loaded file and records it.
    pub fn chain_file(&mut self, stmts: &[NodeId], name: &str) {
        self.arena.chain_block(stmts, None);
        self.files.push(name.to_owned());
    }

    fn assign_names(&mut self, stmts: &[NodeId], filename: &str, _start: i64) {
        let mut all = Vec::new();
        for s in stmts {
            self.collect(*s, &mut all);
        }
        let stamp = 0;
        for id in all {
            if self.arena.get(id).name.is_none() {
                let n = match &self.arena.get(id).kind {
                    NodeKind::Label { name, .. } => NodeName::Label(name.clone()),
                    _ => {
                        let n = NodeName::Generated {
                            filename: filename.into(),
                            stamp,
                            serial: self.serial,
                        };
                        self.serial += 1;
                        n
                    }
                };
                self.arena.get_mut(id).name = Some(n);
            }
        }
    }

    /// Every node reachable through child blocks (`get_children`).
    pub fn collect(&self, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        for b in self.arena.child_blocks(id) {
            for c in b {
                self.collect(c, out);
            }
        }
        if let NodeKind::TranslateSay { .. } = &self.arena.get(id).kind {}
    }

    /// `finish_load` bookkeeping for one file; returns nodes needing early
    /// execution in order.
    pub fn register(&mut self, stmts: &[NodeId]) -> Vec<NodeId> {
        let mut all = Vec::new();
        for s in stmts {
            self.collect(*s, &mut all);
        }
        let mut early = Vec::new();
        for id in all {
            let n = self.arena.get(id);
            if let Some(name) = n.name.clone() {
                if let NodeName::Label(l) = &name {
                    if let Some(old) = self.labels.get(&**l) {
                        let o = self.arena.get(*old);
                        self.duplicate_labels.push(format!(
                            "The label {} is defined twice, at {}:{} and {}:{}.",
                            l, o.filename, o.linenumber, n.filename, n.linenumber
                        ));
                    }
                    self.labels.insert(l.to_string(), id);
                }
                self.namemap.insert(name, id);
            }
            match &self.arena.get(id).kind {
                NodeKind::Init { priority, .. } => self.initcode.push((*priority, id)),
                NodeKind::UserStatement { init_priority, .. } => {
                    self.initcode.push((*init_priority, id))
                }
                NodeKind::Translate {
                    identifier,
                    language,
                    ..
                }
                | NodeKind::TranslateSay {
                    identifier,
                    language,
                    ..
                } => match language {
                    None => {
                        self.translates.insert(identifier.clone(), id);
                    }
                    Some(l) => {
                        self.language_translates
                            .insert((identifier.clone(), l.clone()), id);
                    }
                },
                _ => {}
            }
            if matches!(
                self.arena.get(id).kind,
                NodeKind::EarlyPython { .. }
                    | NodeKind::Python { .. }
                    | NodeKind::Transform { .. }
                    | NodeKind::Define { .. }
                    | NodeKind::Default { .. }
            ) {
                early.push(id);
            }
        }
        early
    }

    /// `ScriptTranslator.chain_translates`: each language translation
    /// continues where its default dialogue continues. Returns the
    /// (identifier, language) pairs without a default.
    pub fn chain_translates(&mut self) -> Vec<(String, String)> {
        let mut unchained = Vec::new();
        let mut pairs: Vec<_> = self
            .language_translates
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        pairs.sort();
        for ((identifier, language), id) in pairs {
            let Some(&default) = self.translates.get(&identifier) else {
                unchained.push((identifier, language));
                continue;
            };
            let after = self.arena.get(default).next_after();
            match self.arena.get(id).kind.clone() {
                NodeKind::TranslateSay { .. } => self.arena.get_mut(id).next = after,
                NodeKind::Translate { block, .. } => self.arena.chain_block(&block, after),
                _ => {}
            }
        }
        unchained
    }

    /// Sorts init code by priority, stable in load order.
    pub fn finish(&mut self) {
        self.initcode.sort_by_key(|(p, _)| *p);
    }

    /// ATL blocks are reached through the nodes that own them.
    pub fn find_atl(&self, file: &str, line: u32) -> Option<Rc<crate::atl::RawBlock>> {
        for i in 0..self.arena.len() {
            let n = self.arena.get(i);
            for atl in n.kind.atl_blocks() {
                if &*atl.loc.0 == file && atl.loc.1 == line {
                    return Some(atl.clone());
                }
            }
        }
        None
    }

    pub fn lookup_label(&self, name: &str) -> Option<NodeId> {
        self.labels.get(name).copied()
    }
}

pub fn filename_rc(s: &str) -> Rc<str> {
    s.into()
}
