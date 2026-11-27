// SPDX-License-Identifier: MPL-2.0
//! Translation structure (`renpy.translation.Restructurer`, MIT). Compiled
//! scripts carry the restructured form (slot 2: `Translate`/`EndTranslate`
//! around each say, single says combined into `TranslateSay`); freshly
//! parsed `.rpy` blocks are restructured here the same way, so dialogue
//! identifiers equal upstream's: the md5 of each group's `get_code()`
//! lines (CRLF-terminated), prefixed with the enclosing label.

use crate::ast::{Arena, NodeId, NodeKind, NodeName};
use std::collections::{HashMap, HashSet};

struct Restructurer<'a> {
    arena: &'a mut Arena,
    existing: &'a HashMap<String, NodeId>,
    label: Option<String>,
    alternate: Option<String>,
    preexisting: HashSet<String>,
    identifiers: HashSet<String>,
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// `Node.get_code()` of the translatable nodes.
pub fn get_code(kind: &NodeKind) -> Option<String> {
    match kind {
        NodeKind::Say { who, what, attributes, temporary_attributes, with_, interact, identifier, explicit_identifier, arguments } => {
            let mut rv = Vec::new();
            if let Some(w) = who {
                rv.push(w.text.to_string());
            }
            if let Some(a) = attributes {
                rv.extend(a.iter().cloned());
            }
            if let Some(t) = temporary_attributes.as_ref().filter(|t| !t.is_empty()) {
                rv.push("@".into());
                rv.extend(t.iter().cloned());
            }
            rv.push(crate::lexer::encode_say_string(what));
            if !interact {
                rv.push("nointeract".into());
            }
            if let (Some(id), true) = (identifier, *explicit_identifier) {
                rv.push("id".into());
                rv.push(id.clone());
            }
            if let Some(a) = arguments {
                rv.push(a.get_code());
            }
            if let Some(w) = with_ {
                rv.push("with".into());
                rv.push(w.text.to_string());
            }
            Some(rv.join(" "))
        }
        NodeKind::UserStatement { line, .. } => Some(line.clone()),
        _ => None,
    }
}

fn translatable(kind: &NodeKind) -> bool {
    match kind {
        NodeKind::Say { .. } => true,
        NodeKind::UserStatement { translatable, .. } => *translatable,
        _ => false,
    }
}

impl Restructurer<'_> {
    fn id_exists(&self, identifier: &str) -> bool {
        self.identifiers.contains(identifier) || self.existing.contains_key(identifier)
    }

    fn unique_identifier(&self, label: Option<&str>, digest: &str) -> String {
        let base = match label {
            None => digest.to_owned(),
            Some(l) => format!("{}_{digest}", l.replace('.', "_")),
        };
        let mut i = 0;
        loop {
            let id = if i == 0 { base.clone() } else { format!("{base}_{i}") };
            if !self.id_exists(&id) && !self.preexisting.contains(&id) {
                return id;
            }
            i += 1;
        }
    }

    fn create_translate(&mut self, block: Vec<NodeId>) -> Vec<NodeId> {
        let mut data = Vec::new();
        let mut id_identifier = None;
        for &i in &block {
            let k = &self.arena.get(i).kind;
            if let Some(code) = get_code(k) {
                data.extend_from_slice(code.as_bytes());
            }
            data.extend_from_slice(b"\r\n");
            if let NodeKind::Say { identifier, .. } = k {
                if identifier.is_some() {
                    id_identifier = identifier.clone();
                }
            }
        }
        let digest = hex(&crysirror_resource::checksum::md5_legacy(&data))[..8].to_owned();
        let mut identifier = self.unique_identifier(self.label.as_deref(), &digest);
        let alternate = if let Some(alt) = self.alternate.clone() {
            let a = self.unique_identifier(Some(&alt), &digest);
            if let Some(i) = &id_identifier {
                identifier = i.clone();
            }
            Some(a)
        } else if let Some(i) = id_identifier {
            let a = identifier.clone();
            identifier = i;
            Some(a)
        } else {
            None
        };
        self.identifiers.insert(identifier.clone());
        if let Some(a) = &alternate {
            self.identifiers.insert(a.clone());
        }
        let first = self.arena.get(block[0]).clone();
        let loc = (first.filename.clone(), first.linenumber);
        let base_name = first.name.clone().unwrap_or(NodeName::Other("".into()));
        let tl = self.arena.add(&loc, NodeKind::Translate { identifier, language: None, block, alternate });
        self.arena.get_mut(tl).name = Some(base_name.suffixed("translate"));
        self.arena.get_mut(tl).rollback = crate::ast::Rollback::Never;
        let ed = self.arena.add(&loc, NodeKind::EndTranslate);
        self.arena.get_mut(ed).name = Some(base_name.suffixed("end_translate"));
        self.arena.get_mut(ed).rollback = crate::ast::Rollback::Never;
        vec![tl, ed]
    }

    /// `combine_translate`: a translate around one say becomes TranslateSay.
    fn combine(&mut self, id: NodeId) -> NodeId {
        let NodeKind::Translate { identifier, language, block, .. } = self.arena.get(id).kind.clone() else { return id };
        if block.len() != 1 {
            return id;
        }
        let say = self.arena.get(block[0]).clone();
        if !matches!(say.kind, NodeKind::Say { .. }) {
            return id;
        }
        let loc = (say.filename.clone(), say.linenumber);
        let ts = self.arena.add(&loc, NodeKind::TranslateSay { identifier, language, say: Box::new(say.kind.clone()) });
        self.arena.get_mut(ts).name = say.name.clone();
        ts
    }

    fn callback(&mut self, children: &[NodeId]) -> Vec<NodeId> {
        let mut new_children = Vec::new();
        let mut group = Vec::new();
        for &i in children {
            if let NodeKind::Label { name, hide, .. } = &self.arena.get(i).kind {
                if !hide {
                    if name.starts_with('_') {
                        self.alternate = Some(name.to_string());
                    } else {
                        self.label = Some(name.to_string());
                        self.alternate = None;
                    }
                }
            }
            if !matches!(self.arena.get(i).kind, NodeKind::Translate { .. }) {
                self.restructure_children(i);
            }
            let kind = &self.arena.get(i).kind;
            if matches!(kind, NodeKind::Say { .. }) {
                group.push(i);
                let g = std::mem::take(&mut group);
                let tl = self.create_translate(g);
                new_children.extend(tl);
            } else if translatable(kind) {
                group.push(i);
            } else {
                if !group.is_empty() {
                    let g = std::mem::take(&mut group);
                    let tl = self.create_translate(g);
                    new_children.extend(tl);
                }
                new_children.push(i);
            }
        }
        if !group.is_empty() {
            let tl = self.create_translate(group);
            new_children.extend(tl);
        }
        let combined: Vec<NodeId> = new_children.into_iter().map(|n| self.combine(n)).collect();
        let mut out = Vec::new();
        let mut prev_say = false;
        for n in combined {
            let is_end = matches!(self.arena.get(n).kind, NodeKind::EndTranslate);
            if prev_say && is_end {
                prev_say = false;
                continue;
            }
            prev_say = matches!(self.arena.get(n).kind, NodeKind::TranslateSay { .. });
            out.push(n);
        }
        out
    }

    /// `Node.restructure(callback)` for the nodes with child blocks.
    fn restructure_children(&mut self, id: NodeId) {
        let kind = self.arena.get(id).kind.clone();
        match kind {
            NodeKind::Init { block, priority } => {
                let b = self.callback(&block);
                self.arena.get_mut(id).kind = NodeKind::Init { block: b, priority };
            }
            NodeKind::Label { name, block, parameters, hide } => {
                let b = self.callback(&block);
                self.arena.get_mut(id).kind = NodeKind::Label { name, block: b, parameters, hide };
            }
            NodeKind::While { condition, block } => {
                let b = self.callback(&block);
                self.arena.get_mut(id).kind = NodeKind::While { condition, block: b };
            }
            NodeKind::If { entries } => {
                let e = entries.into_iter().map(|(c, b)| (c, self.callback(&b))).collect();
                self.arena.get_mut(id).kind = NodeKind::If { entries: e };
            }
            NodeKind::Menu { mut items, set, with_, has_caption, arguments } => {
                for it in items.iter_mut() {
                    if let Some(b) = &it.block {
                        it.block = Some(self.callback(b));
                    }
                }
                self.arena.get_mut(id).kind = NodeKind::Menu { items, set, with_, has_caption, arguments };
            }
            NodeKind::UserStatement { line, block, parsed, subparses, code_block, init_priority, translatable, atl } => {
                let code_block = code_block.map(|b| self.callback(&b));
                let subparses = subparses.iter().map(|b| self.callback(b)).collect();
                self.arena.get_mut(id).kind = NodeKind::UserStatement { line, block, parsed, subparses, code_block, init_priority, translatable, atl };
            }
            _ => {}
        }
    }
}

fn collect(arena: &Arena, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for b in arena.child_blocks(id) {
        for c in b {
            collect(arena, c, out);
        }
    }
}

/// `renpy.translation.restructure(children)` for a file's top-level
/// block; `existing` holds identifiers of previously loaded files.
pub fn restructure(arena: &mut Arena, block: &[NodeId], existing: &HashMap<String, NodeId>) -> Vec<NodeId> {
    let mut preexisting = HashSet::new();
    let mut all = Vec::new();
    for &b in block {
        collect(arena, b, &mut all);
    }
    for id in all {
        if let NodeKind::Say { identifier: Some(i), .. } = &arena.get(id).kind {
            preexisting.insert(i.clone());
        }
    }
    let mut r = Restructurer { arena, existing, label: None, alternate: None, preexisting, identifiers: HashSet::new() };
    r.callback(block)
}
