// SPDX-License-Identifier: MPL-2.0
//! Generated API reference (docs/api).
//!
//! The reference is derived from the same sources the engine runs on: the
//! class registry (which also generates script bindings and `.d.ts`), the
//! command-to-API table, serde defaults of the data specs and the `///`
//! comments of their Rust declarations. `tests/api_docs.rs` fails when the
//! checked-in files differ, so API changes show up as reviewed diffs.
//!
//! The *fingerprint* hashes signatures only (names, types, writability, data
//! fields and their defaults), not prose, so doc fixes do not count as API
//! changes; every fingerprint must be recorded in `docs/api/CHANGES.md`.
use crate::object::{ClassRegistry, TypeHint};
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::{collections::BTreeMap, fmt::Write};

/// One documented struct or enum, parsed from Rust source.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RustItem {
    pub name: String,
    pub doc: String,
    pub is_enum: bool,
    /// Serialized names: struct fields, or enum variants after `rename_all`.
    pub members: Vec<RustMember>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct RustMember {
    pub name: String,
    /// Rust type for fields; empty for variants.
    pub ty: String,
    pub doc: String,
}

/// Extracts top-level `pub struct`/`pub enum` items with their `///` docs.
/// Deliberately small: one declaration per line, as rustfmt writes them.
pub fn parse_rust_items(src: &str) -> Vec<RustItem> {
    let mut items = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    let mut attrs = String::new();
    let mut current: Option<(RustItem, String)> = None;
    let mut depth = 0i32;
    for line in src.lines() {
        let t = line.trim();
        if let Some(d) = t.strip_prefix("///") {
            doc.push(d.trim().to_owned());
            continue;
        }
        if t.starts_with("//") {
            continue;
        }
        if t.starts_with("#[") {
            attrs.push_str(t);
            continue;
        }
        if current.is_none() {
            let decl = t
                .strip_prefix("pub struct ")
                .map(|r| (r, false))
                .or_else(|| t.strip_prefix("pub enum ").map(|r| (r, true)));
            if let (Some((rest, is_enum)), true) = (decl, line.starts_with("pub ")) {
                if t.ends_with('{') {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    let rename_all = attr_value(&attrs, "rename_all").unwrap_or_default();
                    let mut d = doc.join(" ");
                    if let Some(tag) = attr_value(&attrs, "tag").filter(|_| is_enum) {
                        d = format!("{d} Variants are tagged by `\"{tag}\"`.").trim().to_owned();
                    }
                    current = Some((
                        RustItem {
                            name,
                            doc: d,
                            is_enum,
                            members: Vec::new(),
                        },
                        rename_all,
                    ));
                    depth = 1;
                }
            }
            doc.clear();
            attrs.clear();
            continue;
        }
        let (item, rename_all) = current.as_mut().unwrap();
        if depth == 1 {
            let rename = attr_value(&attrs, "rename");
            if !item.is_enum {
                if let Some(rest) = t.strip_prefix("pub ") {
                    if let Some((name, ty)) = rest.split_once(':') {
                        if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            item.members.push(RustMember {
                                name: rename.unwrap_or_else(|| name.to_owned()),
                                ty: ty.trim().trim_end_matches(',').trim().to_owned(),
                                doc: doc.join(" "),
                            });
                        }
                    }
                }
            } else if t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let name: String = t
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let name = rename.unwrap_or_else(|| match rename_all.as_str() {
                    "snake_case" => snake(&name),
                    "lowercase" => name.to_lowercase(),
                    _ => name,
                });
                // One-line struct variant: `Name { a: T, #[serde(..)] b: U },`.
                let inline = t
                    .split_once('{')
                    .and_then(|(_, r)| r.rsplit_once('}'))
                    .map(|(f, _)| {
                        f.split(',')
                            .filter_map(|f| {
                                let f = f.trim();
                                let f = match f.strip_prefix("#[").and_then(|r| r.split_once(']')) {
                                    Some((_, rest)) => rest,
                                    None => f,
                                };
                                let (n, t) = f.split_once(':')?;
                                Some(format!("{}: {}", n.trim(), t.trim()))
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                item.members.push(RustMember {
                    name,
                    ty: inline,
                    doc: doc.join(" "),
                });
            }
        }
        if depth == 2 && item.is_enum {
            if let (Some(last), Some((name, ty))) = (item.members.last_mut(), t.split_once(':')) {
                let name = name.trim_start_matches("pub ").trim();
                if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
                    let rename = attr_value(&attrs, "rename").unwrap_or_else(|| name.to_owned());
                    let ty = ty.trim().trim_end_matches(',').trim();
                    let sep = if last.ty.is_empty() { "" } else { ", " };
                    last.ty = format!("{}{sep}{rename}: {ty}", last.ty);
                }
            }
        }
        doc.clear();
        attrs.clear();
        depth += t.matches('{').count() as i32 - t.matches('}').count() as i32;
        if depth <= 0 {
            items.push(current.take().unwrap().0);
        }
    }
    items
}
fn attr_value(attrs: &str, key: &str) -> Option<String> {
    let pat = format!("{key} = \"");
    let i = attrs.find(&pat)? + pat.len();
    // `rename` must not match inside `rename_all`.
    if key == "rename" && attrs[..i].ends_with("rename_all = \"") {
        return None;
    }
    Some(attrs[i..].split('"').next()?.to_owned())
}
fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A data spec in the reference: its serde sample supplies field defaults.
pub struct DataType {
    /// Rust type name as declared (used to find its source docs).
    pub name: &'static str,
    /// Where it appears in projects and API calls.
    pub used_by: &'static str,
    /// `serde_json` serialization of the spec parsed from a minimal value,
    /// or `None` for items documented from source only.
    pub sample: Option<Json>,
}

pub struct ApiInput<'a> {
    pub classes: &'a ClassRegistry,
    pub globals: &'a [(&'a str, &'a str)],
    pub commands: &'a [(&'a str, &'a str, &'a str)],
    /// `system` → command variant names.
    pub command_variants: Vec<(&'static str, Vec<String>)>,
    /// `system` → its command enum in `data`; variants without docs are
    /// described by the API member that issues them.
    pub command_enums: Vec<(&'static str, &'static str)>,
    pub data: Vec<DataType>,
    /// Adapter name → its parsed `config` struct.
    pub adapters: Vec<(&'static str, Option<RustItem>)>,
    /// Parsed items from every source file that declares documented types.
    pub items: Vec<RustItem>,
}

pub struct ApiDocs {
    pub markdown: String,
    pub json: String,
    pub typescript: String,
    pub fingerprint: String,
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}
fn ty(t: &TypeHint) -> String {
    t.typescript()
}
fn cell(s: &str) -> String {
    let s = s.replace('|', "\\|").replace('\n', " ");
    if s.is_empty() {
        "—".into()
    } else {
        s
    }
}
fn anchor(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == ' ')
        .collect::<String>()
        .replace(' ', "-")
}

pub fn generate(input: &ApiInput) -> ApiDocs {
    let item = |name: &str| input.items.iter().find(|i| i.name == name);
    let infos = input.classes.infos();

    // Machine-readable reference; the signature subset feeds the fingerprint.
    let classes: Vec<Json> = infos
        .iter()
        .map(|c| {
            json!({
                "name": c.name, "doc": c.doc,
                "global": input.globals.iter().find(|(_, k)| *k == c.name).map(|(g, _)| *g),
                "properties": c.properties.iter().map(|p| json!({
                    "name": p.name, "type": ty(&p.ty), "writable": p.writable, "doc": p.doc})).collect::<Vec<_>>(),
                "methods": c.methods.iter().map(|m| json!({
                    "name": m.name,
                    "params": m.params.iter().map(|(n, t)| json!({"name": n, "type": ty(t)})).collect::<Vec<_>>(),
                    "returns": ty(&m.returns), "doc": m.doc})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let commands: Vec<Json> = input
        .commands
        .iter()
        .map(|(s, o, a)| json!({"system": s, "op": o, "api": a}))
        .collect();
    let data: Vec<Json> = input
        .data
        .iter()
        .map(|d| {
            let it = item(d.name);
            json!({
                "name": d.name, "used_by": d.used_by,
                "doc": it.map(|i| i.doc.clone()).unwrap_or_default(),
                "fields": it.map(|i| i.members.iter().map(|m| json!({
                    "name": m.name, "rust_type": m.ty, "doc": m.doc,
                    "default": d.sample.as_ref().and_then(|s| s.get(&m.name)).cloned(),
                })).collect::<Vec<_>>()).unwrap_or_default(),
            })
        })
        .collect();
    let adapters: Vec<Json> = input
        .adapters
        .iter()
        .map(|(a, cfg)| {
            json!({"adapter": a, "config": cfg.as_ref().map(|i| i.members.iter().map(|m| json!({
                "name": m.name, "rust_type": m.ty, "doc": m.doc})).collect::<Vec<_>>())})
        })
        .collect();
    let mut signature = json!({
        "classes": classes, "commands": commands, "data": data, "adapters": adapters,
        "command_variants": input.command_variants,
    });
    strip_docs(&mut signature);
    let fingerprint = format!("{:016x}", fnv(signature.to_string().as_bytes()));
    let full = json!({
        "generated_by": "crysirror-engine api_doc (tests/api_docs.rs)",
        "fingerprint": fingerprint,
        "globals": input.globals.iter().map(|(g, c)| json!({"global": g, "class": c})).collect::<Vec<_>>(),
        "classes": classes, "commands": commands, "data": data, "adapters": adapters,
    });

    let mut md = String::new();
    let _ = writeln!(
        md,
        "# CrySirror API reference\n\n\
         <!-- Generated by crysirror-engine::api_doc. Do not edit by hand. -->\n\n\
         Generated from the class registry, the command table, serde defaults \
         and Rust doc comments. Regenerate with\n\
         `CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs`.\n\
         Policy and change log: [README](README.md), [CHANGES](CHANGES.md).\n\n\
         API fingerprint: `{fingerprint}`\n\n\
         The same classes are exposed to Rust (`EngineContext` calls), to \
         JavaScript/TypeScript (bindings generated from the registry, \
         [crysirror.d.ts](crysirror.d.ts)) and to inspection tools.\n"
    );
    md.push_str("## Contents\n\n- [Globals](#globals)\n- Classes: ");
    md.push_str(
        &infos
            .iter()
            .map(|c| format!("[{}](#{})", c.name, anchor(&format!("class {}", c.name))))
            .collect::<Vec<_>>()
            .join(", "),
    );
    md.push_str("\n- [Commands](#commands)\n- Data: ");
    md.push_str(
        &input
            .data
            .iter()
            .map(|d| format!("[{}](#{})", d.name, anchor(d.name)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    md.push_str("\n- [Adapter configs](#adapter-configs)\n\n## Globals\n\n| Global | Class |\n|---|---|\n");
    for (g, c) in input.globals {
        let _ = writeln!(md, "| `{g}` | [{c}](#{}) |", anchor(&format!("class {c}")));
    }
    for c in &infos {
        let _ = writeln!(md, "\n## Class {}\n\n{}\n", c.name, c.doc);
        if let Some((g, _)) = input.globals.iter().find(|(_, k)| *k == c.name) {
            let _ = writeln!(md, "Script global: `{g}`.\n");
        }
        if !c.properties.is_empty() {
            md.push_str("| Property | Type | Access | Description |\n|---|---|---|---|\n");
            for p in &c.properties {
                let _ = writeln!(
                    md,
                    "| `{}` | `{}` | {} | {} |",
                    p.name,
                    cell(&ty(&p.ty)),
                    if p.writable { "read/write" } else { "read" },
                    cell(p.doc)
                );
            }
            md.push('\n');
        }
        if !c.methods.is_empty() {
            md.push_str("| Method | Returns | Description |\n|---|---|---|\n");
            for m in &c.methods {
                let params = m
                    .params
                    .iter()
                    .map(|(n, t)| format!("{n}: {}", ty(t)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(
                    md,
                    "| `{}({})` | `{}` | {} |",
                    m.name,
                    cell(&params),
                    cell(&ty(&m.returns)),
                    cell(m.doc)
                );
            }
            md.push('\n');
        }
        let issued: Vec<String> = input
            .commands
            .iter()
            .filter(|(_, _, a)| a.split_once('.').is_some_and(|(k, _)| k == c.name))
            .map(|(s, o, a)| format!("`{}` → `{s}.{o}`", a.split_once('.').unwrap().1))
            .collect();
        if !issued.is_empty() {
            let _ = writeln!(md, "Commands issued: {}.", issued.join(", "));
        }
    }
    md.push_str(
        "\n## Commands\n\nEvery typed engine command and the API member that issues it. \
         `tests/api_coverage.rs` fails when a command variant has no entry.\n\n\
         | System | Command | API |\n|---|---|---|\n",
    );
    for (s, o, a) in input.commands {
        let _ = writeln!(md, "| {s} | `{o}` | `{a}` |");
    }
    for (system, variants) in &input.command_variants {
        let unmapped: Vec<&String> = variants
            .iter()
            .filter(|v| !input.commands.iter().any(|(s, o, _)| s == system && o == *v))
            .collect();
        if !unmapped.is_empty() {
            let _ = writeln!(md, "\nUnmapped `{system}` commands: {unmapped:?}");
        }
    }
    md.push_str("\n## Data\n\nField defaults are the serde defaults (`—`: required, or absent when unset).\n");
    for d in &input.data {
        let _ = writeln!(md, "\n### {}\n\nUsed by: {}.\n", d.name, d.used_by);
        let Some(it) = item(d.name) else {
            md.push_str("(no source declaration found)\n");
            continue;
        };
        if !it.doc.is_empty() {
            let _ = writeln!(md, "{}\n", it.doc);
        }
        if it.is_enum {
            md.push_str("| Variant | Fields | Description |\n|---|---|---|\n");
            let system = input.command_enums.iter().find(|(_, n)| *n == d.name).map(|(s, _)| *s);
            for m in &it.members {
                let issued = system.and_then(|sys| input.commands.iter().find(|(s, o, _)| *s == sys && *o == m.name));
                let doc = match issued {
                    Some((_, _, api)) if m.doc.is_empty() => format!("Issued by `{api}`."),
                    _ => m.doc.clone(),
                };
                let m = RustMember { doc, ..m.clone() };
                let fields = if m.ty.is_empty() { "—".to_owned() } else { format!("`{}`", cell(&m.ty)) };
                let _ = writeln!(md, "| `{}` | {fields} | {} |", m.name, cell(&m.doc));
            }
        } else {
            md.push_str("| Field | Rust type | Default | Description |\n|---|---|---|---|\n");
            for m in &it.members {
                let default = d
                    .sample
                    .as_ref()
                    .and_then(|s| s.get(&m.name))
                    .map(|v| format!("`{}`", cell(&v.to_string())))
                    .unwrap_or_else(|| "—".into());
                let _ = writeln!(
                    md,
                    "| `{}` | `{}` | {} | {} |",
                    m.name,
                    cell(&m.ty),
                    default,
                    cell(&m.doc)
                );
            }
        }
    }
    md.push_str("\n## Adapter configs\n\n`clients[].config` of each original-VM adapter.\n");
    for (a, cfg) in &input.adapters {
        let _ = writeln!(md, "\n### `{a}`\n");
        match cfg {
            None => md.push_str("(no source declaration found)\n"),
            Some(it) => {
                md.push_str("| Field | Rust type | Description |\n|---|---|---|\n");
                for m in &it.members {
                    let _ = writeln!(md, "| `{}` | `{}` | {} |", m.name, cell(&m.ty), cell(&m.doc));
                }
            }
        }
    }
    let typescript = input.classes.typescript_declarations(input.globals);
    ApiDocs {
        markdown: md,
        json: serde_json::to_string_pretty(&full).unwrap() + "\n",
        typescript,
        fingerprint,
    }
}

fn strip_docs(v: &mut Json) {
    match v {
        Json::Object(m) => {
            m.remove("doc");
            m.values_mut().for_each(strip_docs);
        }
        Json::Array(a) => a.iter_mut().for_each(strip_docs),
        _ => {}
    }
}

/// Serde sample of `T` parsed from a minimal value (fills every default).
pub fn sample<T: serde::de::DeserializeOwned + Serialize>(minimal: Json) -> Option<Json> {
    let v: T = serde_json::from_value(minimal).ok()?;
    let mut v = serde_json::to_value(v).ok()?;
    shortest_f32(&mut v);
    Some(v)
}
/// Specs store `f32`; print `0.04`, not its widened `0.03999999910593033`.
fn shortest_f32(v: &mut Json) {
    match v {
        Json::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap() as f32;
            if let Some(m) = f.to_string().parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
                *n = m;
            }
        }
        Json::Object(m) => m.values_mut().for_each(shortest_f32),
        Json::Array(a) => a.iter_mut().for_each(shortest_f32),
        _ => {}
    }
}

/// Members that lack documentation, for the completeness test.
pub fn undocumented(classes: &ClassRegistry) -> Vec<String> {
    let mut out = BTreeMap::new();
    for c in classes.infos() {
        if c.doc.trim().is_empty() {
            out.insert(c.name.to_string(), ());
        }
        for p in &c.properties {
            if p.doc.trim().is_empty() {
                out.insert(format!("{}.{}", c.name, p.name), ());
            }
        }
        for m in &c.methods {
            if m.doc.trim().is_empty() {
                out.insert(format!("{}.{}()", c.name, m.name), ());
            }
        }
    }
    out.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_fields_variants_docs_and_renames() {
        let src = "/// A thing.\n#[derive(Serialize)]\npub struct Thing {\n    /// Size {in m}.\n    #[serde(default)]\n    pub size: [f32; 3],\n    hidden: u8,\n    #[serde(rename = \"kind\")]\n    pub ty: Option<String>,\n}\n#[serde(rename_all = \"snake_case\")]\npub enum Mode {\n    /// Fast.\n    FastPath,\n    Slow {\n        x: u8,\n    },\n}\n";
        let items = parse_rust_items(src);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].doc, "A thing.");
        let f: Vec<_> = items[0].members.iter().map(|m| (m.name.as_str(), m.doc.as_str())).collect();
        assert_eq!(f, [("size", "Size {in m}."), ("kind", "")]);
        let v: Vec<_> = items[1].members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(v, ["fast_path", "slow"]);
        assert_eq!(items[1].members[0].doc, "Fast.");
        assert_eq!(items[1].members[1].ty, "x: u8");
    }
}
