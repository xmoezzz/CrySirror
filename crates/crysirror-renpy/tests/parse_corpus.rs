// SPDX-License-Identifier: MPL-2.0
//! Parses the reference games' scripts (local corpus, skipped when absent).

use crysirror_renpy::ast::{Arena, NodeKind};
use crysirror_renpy::lexer::{Lexer, PResult, lex_file};
use crysirror_renpy::parser::{Parser, ScriptContext};
use std::rc::Rc;

struct Cx(crysirror_renpy::sl2::Registry);
impl crysirror_renpy::atl::ParseContext for Cx {
    fn is_warper(&self, n: &str) -> bool {
        crysirror_renpy::atl::BUILTIN_WARPERS.contains(&n)
    }
}
impl ScriptContext for Cx {
    fn user_statement_names(&self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn parse_user_statement(
        &self,
        _a: &mut Arena,
        _n: &[String],
        _l: &mut Lexer,
        _loc: &(Rc<str>, u32),
    ) -> PResult<Vec<usize>> {
        unreachable!()
    }
    fn sl2_registry(&self) -> &crysirror_renpy::sl2::Registry {
        &self.0
    }
}

fn reference() -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/reference/renpy");
    p.exists().then_some(p)
}

#[test]
fn the_question_script() {
    let Some(root) = reference() else { return };
    let src = std::fs::read_to_string(root.join("the_question/game/script.rpy")).unwrap();
    let block = lex_file("game/script.rpy", &src).unwrap();
    let mut arena = Arena::default();
    let cx = Cx(Default::default());
    let mut p = Parser::new(&mut arena, &cx);
    let nodes = p.parse_file(block, "game/script.rpy", src.lines().count() as u32);
    for e in &p.errors {
        eprintln!("{e}");
    }
    assert!(p.errors.is_empty());
    let says = arena
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Say { .. }))
        .count();
    let menus = arena
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Menu { .. }))
        .count();
    eprintln!(
        "top-level {} nodes, {} total, {says} say, {menus} menu",
        nodes.len(),
        arena.nodes.len()
    );
    assert!(says > 20 && menus >= 1);
}

#[test]
fn whole_corpus_statement_errors() {
    let Some(root) = reference() else { return };
    let mut files = Vec::new();
    for dir in [
        "renpy/common",
        "the_question/game",
        "tutorial/game",
        "launcher/game",
        "gui/game",
    ] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("rpy") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    let mut total = 0;
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let name = f.to_string_lossy().to_string();
        let block = match lex_file(&name, &src) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("LEX {e}");
                total += 1;
                continue;
            }
        };
        let mut arena = Arena::default();
        let cx = Cx(Default::default());
        let mut p = Parser::new(&mut arena, &cx);
        p.parse_file(block, &name, src.lines().count() as u32);
        for e in &p.errors {
            let t = e.text.clone().unwrap_or_default();
            let first = t.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            *kinds.entry(format!("{} | {}", e.msg, first)).or_default() += 1;
            total += 1;
        }
    }
    for (k, v) in &kinds {
        eprintln!("{v:5} {k}");
    }
    eprintln!("{} files, {total} errors", files.len());
}

/// Dialogue identifiers must equal those Ren'Py generated into the
/// reference game's translation files.
#[test]
fn the_question_translation_identifiers_match_upstream() {
    let Some(root) = reference() else { return };
    let src = std::fs::read_to_string(root.join("the_question/game/script.rpy")).unwrap();
    let block = lex_file("game/script.rpy", &src).unwrap();
    let mut arena = Arena::default();
    let cx = Cx(Default::default());
    let mut p = Parser::new(&mut arena, &cx);
    let nodes = p.parse_file(block, "game/script.rpy", src.lines().count() as u32);
    assert!(p.errors.is_empty());
    drop(p);
    crysirror_renpy::translation::restructure(&mut arena, &nodes, &Default::default());
    let mut ours = std::collections::HashSet::new();
    for n in &arena.nodes {
        match &n.kind {
            NodeKind::Translate { identifier, .. } | NodeKind::TranslateSay { identifier, .. } => {
                ours.insert(identifier.clone());
            }
            _ => {}
        }
    }
    let tl = std::fs::read_to_string(root.join("the_question/game/tl/czech/script.rpy")).unwrap();
    let mut expected = Vec::new();
    for l in tl.lines() {
        if let Some(rest) = l.strip_prefix("translate czech ") {
            let id = rest.trim_end_matches(':').to_string();
            if id != "strings" {
                expected.push(id);
            }
        }
    }
    assert!(expected.len() > 20);
    let missing: Vec<_> = expected.iter().filter(|i| !ours.contains(*i)).collect();
    assert!(
        missing.is_empty(),
        "{} of {} upstream identifiers missing, e.g. {:?}",
        missing.len(),
        expected.len(),
        &missing[..missing.len().min(5)]
    );
}
