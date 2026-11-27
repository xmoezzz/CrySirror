// SPDX-License-Identifier: MPL-2.0
//! Screen language 2 (`renpy/sl2/slast.py` node classes and
//! `slparser.py`). Displayable statements come from `sldisplayables.py`;
//! creator-defined screen statements are added through `register`.

use crate::atl::{self, ParseContext, RawBlock};
use crate::ast::{ArgumentInfo, Signature};
use crate::lexer::{Lexer, PResult, PyExpr};
use std::rc::Rc;

pub type Loc = (Rc<str>, u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NChildren {
    Zero,
    One,
    Many,
}

/// One screen-language displayable statement (`DisplayableParser`).
#[derive(Clone, Debug)]
pub struct StatementSpec {
    pub name: String,
    /// Python callable that creates the displayable (dotted name).
    pub displayable: String,
    pub style: Option<String>,
    pub nchildren: NChildren,
    pub positional: Vec<String>,
    pub scope: bool,
    pub pass_context: bool,
    pub imagemap: bool,
    pub hotspot: bool,
    pub replaces: bool,
    pub unique: bool,
    pub default_keywords: Vec<(String, String)>,
    /// Creator-defined statement using a screen (`CustomParser`).
    pub custom_screen: Option<String>,
}

fn spec(name: &str, disp: &str, style: Option<&str>, n: NChildren, pos: &[&str]) -> StatementSpec {
    StatementSpec {
        name: name.into(),
        displayable: disp.into(),
        style: style.map(|s| s.into()),
        nchildren: n,
        positional: pos.iter().map(|s| s.to_string()).collect(),
        scope: false,
        pass_context: false,
        imagemap: false,
        hotspot: false,
        replaces: false,
        unique: false,
        default_keywords: Vec::new(),
        custom_screen: None,
    }
}

/// Built-in displayable statements of `sldisplayables.py`.
pub fn builtin_statements() -> Vec<StatementSpec> {
    use NChildren::*;
    let mut v = Vec::new();
    v.push(spec("null", "renpy.display.layout.Null", Some("default"), Zero, &[]));
    let mut t = spec("text", "renpy.text.text.Text", Some("text"), Zero, &["text"]);
    t.scope = true;
    t.replaces = true;
    v.push(t);
    for (n, layout) in [("hbox", "horizontal"), ("vbox", "vertical"), ("fixed", "fixed")] {
        let mut s = spec(n, "renpy.display.layout.MultiBox", Some(n), Many, &[]);
        s.default_keywords = vec![("layout".into(), format!("'{layout}'"))];
        v.push(s);
    }
    v.push(spec("grid", "renpy.display.layout.Grid", Some("grid"), Many, &["cols", "rows"]));
    v.push(spec("side", "renpy.display.layout.Side", Some("side"), Many, &["positions"]));
    v.push(spec("_ime_candidates_placer", "renpy.display.ime.IMECandidatesPlacer", Some("default"), One, &[]));
    v.push(spec("window", "renpy.display.layout.Window", Some("window"), One, &[]));
    v.push(spec("frame", "renpy.display.layout.Window", Some("frame"), One, &[]));
    v.push(spec("key", "renpy.ui._key", None, Zero, &["key"]));
    let mut s = spec("timer", "renpy.display.behavior.Timer", Some("default"), Zero, &["delay"]);
    s.replaces = true;
    v.push(s);
    let mut s = spec("input", "renpy.display.behavior.Input", Some("input"), Zero, &[]);
    s.replaces = true;
    v.push(s);
    v.push(spec("button", "renpy.display.behavior.Button", Some("button"), One, &[]));
    v.push(spec("imagebutton", "renpy.ui._imagebutton", Some("image_button"), Zero, &[]));
    let mut s = spec("textbutton", "renpy.ui._textbutton", Some("button"), Zero, &["label"]);
    s.scope = true;
    v.push(s);
    let mut s = spec("label", "renpy.ui._label", Some("label"), Zero, &["label"]);
    s.scope = true;
    v.push(s);
    for (n, d) in [("bar", "renpy.sl2.sldisplayables.sl2bar"), ("vbar", "renpy.sl2.sldisplayables.sl2vbar")] {
        let mut s = spec(n, d, None, Zero, &[]);
        s.replaces = true;
        s.pass_context = true;
        v.push(s);
    }
    let mut s = spec("viewport", "renpy.sl2.sldisplayables.sl2viewport", Some("viewport"), One, &[]);
    s.replaces = true;
    s.pass_context = true;
    v.push(s);
    let mut s = spec("vpgrid", "renpy.sl2.sldisplayables.sl2vpgrid", Some("vpgrid"), Many, &[]);
    s.replaces = true;
    s.pass_context = true;
    v.push(s);
    let mut s = spec("imagemap", "renpy.ui._imagemap", Some("imagemap"), Many, &[]);
    s.imagemap = true;
    v.push(s);
    let mut s = spec("hotspot", "renpy.ui._hotspot", Some("hotspot"), One, &["spot"]);
    s.hotspot = true;
    v.push(s);
    let mut s = spec("hotbar", "renpy.ui._hotbar", Some("hotbar"), Zero, &["spot"]);
    s.hotspot = true;
    s.replaces = true;
    v.push(s);
    v.push(spec("transform", "renpy.display.motion.Transform", Some("transform"), One, &[]));
    for n in ["add", "image"] {
        let mut s = spec(n, "renpy.sl2.sldisplayables.sl2add", None, Zero, &["im"]);
        s.replaces = true;
        s.scope = true;
        v.push(s);
    }
    let mut s = spec("drag", "renpy.display.dragdrop.Drag", Some("drag"), One, &[]);
    s.replaces = true;
    v.push(s);
    let mut s = spec("draggroup", "renpy.display.dragdrop.DragGroup", None, Many, &[]);
    s.replaces = true;
    v.push(s);
    let mut s = spec("mousearea", "renpy.display.behavior.MouseArea", None, Zero, &[]);
    s.replaces = true;
    v.push(s);
    v.push(spec("on", "renpy.display.behavior.OnEvent", None, Zero, &["event"]));
    let mut s = spec("nearrect", "renpy.display.layout.NearRect", Some("default"), One, &[]);
    s.replaces = true;
    v.push(s);
    v.push(spec("dismiss", "renpy.display.behavior.DismissBehavior", Some("default"), Zero, &[]));
    v.push(spec("areapicker", "renpy.display.behavior.AreaPicker", Some("default"), One, &[]));
    v
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    pub keyword: Vec<(String, PyExpr)>,
    pub children: Vec<SlNode>,
    pub atl_transform: Option<Rc<RawBlock>>,
    pub variable: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Displayable {
    pub loc: Loc,
    pub statement: String,
    pub positional: Vec<PyExpr>,
    pub block: Block,
}

#[derive(Clone, Debug)]
pub enum SlNode {
    Displayable(Box<Displayable>),
    If { loc: Loc, entries: Vec<(Option<PyExpr>, Block)> },
    ShowIf { loc: Loc, entries: Vec<(Option<PyExpr>, Block)> },
    For { loc: Loc, variable: String, expression: PyExpr, index_expression: Option<PyExpr>, block: Block },
    Python { loc: Loc, code: PyExpr },
    Pass { loc: Loc },
    Default { loc: Loc, variable: String, expression: PyExpr },
    Use { loc: Loc, target: String, expression: bool, args: Option<ArgumentInfo>, id: Option<PyExpr>, block: Option<Block>, variable: Option<String> },
    Transclude { loc: Loc },
    CustomUse { loc: Loc, target: String, positional: Vec<PyExpr>, block: Block, variable: Option<String> },
    Break { loc: Loc },
    Continue { loc: Loc },
}

impl SlNode {
    fn has_python(&self) -> bool {
        match self {
            SlNode::Python { .. } => true,
            SlNode::Displayable(d) => d.block.children.iter().any(|c| c.has_python()),
            SlNode::If { entries, .. } | SlNode::ShowIf { entries, .. } => entries.iter().any(|(_, b)| b.children.iter().any(|c| c.has_python())),
            SlNode::For { block, .. } => block.children.iter().any(|c| c.has_python()),
            _ => false,
        }
    }
    fn is_condition(&self) -> bool {
        matches!(self, SlNode::If { .. } | SlNode::ShowIf { .. })
    }
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub loc: Loc,
    pub name: String,
    pub parameters: Option<Signature>,
    pub tag: Option<String>,
    pub docstring: Option<String>,
    pub block: Block,
    pub modal: String,
    pub zorder: String,
    pub variant: String,
    pub predict: String,
    pub layer: String,
    pub sensitive: String,
    pub roll_forward: String,
}

/// The statement registry (built-in plus creator-defined).
#[derive(Clone)]
pub struct Registry {
    pub statements: Vec<StatementSpec>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { statements: builtin_statements() }
    }
}

const CONTROL: &[&str] = &["if", "showif", "for", "$", "python", "pass", "default", "use", "transclude"];

impl Registry {
    pub fn get(&self, name: &str) -> Option<&StatementSpec> {
        self.statements.iter().rev().find(|s| s.name == name)
    }

    fn is_statement(&self, name: &str) -> bool {
        CONTROL.contains(&name) || self.get(name).is_some()
    }

    pub fn parse_screen(&self, l: &mut Lexer, loc: &Loc, cx: &dyn ParseContext, parse_args: &mut dyn FnMut(&mut Lexer) -> PResult<Option<Signature>>) -> PResult<Screen> {
        l.expect_block("screen statement")?;
        let name = l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?;
        let parameters = parse_args(l)?;
        let mut block = Block::default();
        let mut tag = None;
        let mut doc = None;
        self.parse_contents(l, &mut block, cx, Contents { can_tag: true, docstring: true, ..Contents::default() }, Some(&mut tag), Some(&mut doc), "screen", true)?;
        let kw = |k: &str, d: &str| block.keyword.iter().find(|(n, _)| n == k).map(|(_, e)| e.text.to_string()).unwrap_or_else(|| d.to_string());
        Ok(Screen {
            loc: loc.clone(),
            name,
            parameters,
            modal: kw("modal", "False"),
            zorder: kw("zorder", "0"),
            variant: kw("variant", "None"),
            predict: kw("predict", "None"),
            layer: kw("layer", "'screens'"),
            sensitive: kw("sensitive", "True"),
            roll_forward: kw("roll_forward", "None"),
            tag,
            docstring: doc,
            block,
        })
    }

    fn parse_child(&self, l: &mut Lexer, cx: &dyn ParseContext, layout: bool, keyword: bool, in_for: bool) -> PResult<Option<SlNode>> {
        let loc = l.location();
        let old = l.pos;
        let Some(word) = l.word().or_else(|| l.match_re(r"\$")) else {
            l.pos = old;
            return Ok(None);
        };
        let node = match word.as_str() {
            "if" | "showif" => {
                let mut entries = Vec::new();
                let cond = l.python_expression()?;
                l.require_re(":", ":")?;
                let mut b = Block::default();
                self.parse_contents(l, &mut b, cx, Contents { block_only: true, in_for, keyword, ..Default::default() }, None, None, &word, keyword)?;
                entries.push((Some(cond), b));
                let mut state = l.checkpoint();
                while l.advance() {
                    if l.keyword("elif") {
                        let cond = l.python_expression()?;
                        l.require_re(":", ":")?;
                        let mut b = Block::default();
                        self.parse_contents(l, &mut b, cx, Contents { block_only: true, in_for, keyword, ..Default::default() }, None, None, &word, keyword)?;
                        entries.push((Some(cond), b));
                        state = l.checkpoint();
                    } else if l.keyword("else") {
                        l.require_re(":", ":")?;
                        let mut b = Block::default();
                        self.parse_contents(l, &mut b, cx, Contents { block_only: true, in_for, keyword, ..Default::default() }, None, None, &word, keyword)?;
                        entries.push((None, b));
                        state = l.checkpoint();
                        break;
                    } else {
                        l.revert(&state);
                        break;
                    }
                }
                if !l.eob && l.line >= 0 && entries.len() > 1 {
                    // `revert` above left us on the last consumed line.
                }
                if word == "if" { SlNode::If { loc, entries } } else { SlNode::ShowIf { loc, entries } }
            }
            "for" => {
                l.skip_whitespace();
                let tuple_start = l.pos;
                let name = name_or_tuple(l)?;
                let mut pre = Vec::new();
                let variable = match name {
                    Some(n) => n,
                    None => {
                        let pattern: String = l.text[tuple_start..l.pos].iter().collect();
                        pre.push(SlNode::Python { loc: loc.clone(), code: l.expr(format!("{pattern} = _sl2_i")) });
                        "_sl2_i".into()
                    }
                };
                let index_expression = if l.match_re("index").is_some() { Some(l.say_expression()?.map_or_else(|| l.error("expected 'say_expression' not found."), Ok)?) } else { None };
                l.require_re("in", "in")?;
                let expression = l.python_expression()?;
                l.require_re(":", ":")?;
                l.expect_eol()?;
                let mut b = Block { children: pre, ..Default::default() };
                self.parse_contents(l, &mut b, cx, Contents { block_only: true, in_for: true, keyword: true, ..Default::default() }, None, None, "for", true)?;
                SlNode::For { loc, variable, expression, index_expression, block: b }
            }
            "break" | "continue" if in_for => {
                l.expect_eol()?;
                l.expect_noblock(&format!("{word} statement"))?;
                if word == "break" { SlNode::Break { loc } } else { SlNode::Continue { loc } }
            }
            "$" => {
                let loc = l.location();
                let src = l.rest();
                if src.text.is_empty() {
                    return l.error("expected 'rest' not found.");
                }
                l.expect_eol()?;
                l.expect_noblock("one-line python")?;
                SlNode::Python { loc, code: src }
            }
            "python" => {
                let loc = l.location();
                l.require_re(":", ":")?;
                l.expect_eol()?;
                l.expect_block("python block")?;
                let src = l.python_block();
                SlNode::Python { loc: loc.clone(), code: PyExpr::new(src, loc.0.clone(), loc.1 + 1) }
            }
            "pass" => {
                l.expect_eol()?;
                l.expect_noblock("pass statement")?;
                SlNode::Pass { loc }
            }
            "default" => {
                let n = l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?;
                l.require_re("=", "=")?;
                let rest = l.rest();
                l.expect_eol()?;
                l.expect_noblock("default statement")?;
                SlNode::Default { loc, variable: n, expression: rest }
            }
            "use" => {
                let (target, expression) = if l.keyword("expression") {
                    let t = l.require_simple()?;
                    l.keyword("pass");
                    (t.text.to_string(), true)
                } else {
                    (l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?, false)
                };
                let args = parse_arguments(l)?;
                let mut id = None;
                let mut variable = None;
                loop {
                    if l.keyword("id") {
                        id = l.simple_expression()?;
                    } else if l.keyword("as") {
                        variable = Some(l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?);
                    } else {
                        break;
                    }
                }
                let block = if l.match_re(":").is_some() {
                    l.expect_eol()?;
                    l.expect_block("use statement")?;
                    let mut b = Block::default();
                    self.parse_contents(l, &mut b, cx, Contents { can_has: true, block_only: true, keyword: true, ..Default::default() }, None, None, "use", true)?;
                    Some(b)
                } else {
                    l.expect_eol()?;
                    l.expect_noblock("use statement")?;
                    None
                };
                SlNode::Use { loc, target, expression, args, id, block, variable }
            }
            "transclude" => {
                l.expect_eol()?;
                SlNode::Transclude { loc }
            }
            w => {
                let Some(s) = self.get(w) else {
                    l.pos = old;
                    return Ok(None);
                };
                let s = s.clone();
                let mut positional = Vec::new();
                for _ in &s.positional {
                    match l.simple_expression()? {
                        Some(e) => positional.push(e),
                        None => break,
                    }
                }
                let mut b = Block::default();
                let contents = Contents { layout, can_has: s.nchildren == NChildren::One, keyword, ..Default::default() };
                self.parse_contents(l, &mut b, cx, contents, None, None, &s.name, keyword)?;
                if positional.len() != s.positional.len() && !b.keyword.iter().any(|(k, _)| k == "arguments") {
                    return l.error(format!("{} statement expects {} positional arguments, got {}.", s.name, s.positional.len(), positional.len()));
                }
                match &s.custom_screen {
                    Some(scr) => {
                        let variable = b.variable.take();
                        SlNode::CustomUse { loc, target: scr.clone(), positional, block: b, variable }
                    }
                    None => SlNode::Displayable(Box::new(Displayable { loc, statement: s.name.clone(), positional, block: b })),
                }
            }
        };
        Ok(Some(node))
    }

    fn parse_contents(&self, l: &mut Lexer, target: &mut Block, cx: &dyn ParseContext, c: Contents, mut tag: Option<&mut Option<String>>, mut doc: Option<&mut Option<String>>, stmt: &str, keyword: bool) -> PResult<()> {
        let mut seen: Vec<String> = Vec::new();
        let mut keyword_ok = keyword;
        let variable_ok = self.get(stmt).is_some();
        let mut parse_kw = |l: &mut Lexer, target: &mut Block, first_line: bool, tag: &mut Option<&mut Option<String>>| -> PResult<()> {
            let Some(name) = l.word() else { return l.error("expected a keyword argument, colon, or end of line.") };
            if c.can_tag && name == "tag" {
                if let Some(t) = tag.as_deref_mut() {
                    *t = Some(l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?);
                }
                l.expect_noblock(&name)?;
                return Ok(());
            }
            if variable_ok && name == "as" {
                if target.variable.is_some() {
                    return l.error(format!("an as clause may only appear once in a {stmt} statement."));
                }
                target.variable = Some(l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?);
                return Ok(());
            }
            if name == "continue" || name == "break" {
                return l.error(format!("The {name} statement may only appear inside a for statement, or an if statement inside a for statement."));
            }
            if self.is_statement(&name) && name != "at" && name != "default" && name != "text" {
                return l.error(format!("The {name} statement is not a valid child of the {stmt} statement."));
            }
            if name == "at" {
                let cp = l.checkpoint();
                if l.match_re("transform").is_some() && l.match_re(":").is_some() {
                    l.expect_eol()?;
                    l.expect_block("ATL block")?;
                    let mut sub = l.subblock_lexer(false);
                    target.atl_transform = Some(Rc::new(atl::parse_atl(&mut sub, cx)?));
                    return Ok(());
                }
                l.revert(&cp);
            }
            if seen.contains(&name) {
                return l.error(format!("keyword argument {name:?} appears more than once in a {stmt} statement."));
            }
            seen.push(name.clone());
            let Some(e) = l.comma_expression()? else {
                return l.error(format!("the {name} keyword argument was not given a value."));
            };
            target.keyword.push((name.clone(), e));
            if !first_line {
                l.expect_noblock(&name)?;
            }
            Ok(())
        };
        let block;
        if c.block_only {
            l.expect_eol()?;
            l.expect_block(stmt)?;
            block = true;
        } else {
            loop {
                if l.match_re(":").is_some() {
                    l.expect_eol()?;
                    l.expect_block(stmt)?;
                    block = true;
                    break;
                }
                if l.eol() {
                    if target.atl_transform.is_none() {
                        l.expect_noblock(stmt)?;
                    }
                    block = false;
                    break;
                }
                parse_kw(l, target, true, &mut tag)?;
            }
        }
        let mut sub = if block { Some(l.subblock_lexer(false)) } else { None };
        let mut run = |ll: &mut Lexer, target: &mut Block| -> PResult<()> {
            while ll.advance() {
                let state = ll.checkpoint();
                if c.docstring {
                    if let Some(d) = doc.as_deref_mut() {
                        let st = ll.pos;
                        if ll.python_string()? {
                            let s: String = ll.text[st..ll.pos].iter().collect();
                            ll.expect_eol()?;
                            ll.expect_noblock("docstring")?;
                            *d = Some(s);
                            continue;
                        }
                    }
                }
                if ll.keyword("has") {
                    if !c.can_has {
                        return ll.error("The has statement is not allowed here.");
                    }
                    if target.children.iter().any(|ch| !ch.is_condition()) {
                        return ll.error("The has statement may not be given after a child has been supplied.");
                    }
                    let Some(ch) = self.parse_child(ll, cx, true, keyword_ok, c.in_for)? else {
                        return ll.error("Has expects a child statement.");
                    };
                    if ch.has_python() {
                        keyword_ok = false;
                    }
                    target.children.push(ch);
                    continue;
                }
                if let Some(ch) = self.parse_child(ll, cx, false, keyword_ok, c.in_for)? {
                    if matches!(ch, SlNode::Pass { .. }) {
                        continue;
                    }
                    if ch.has_python() {
                        keyword_ok = false;
                    }
                    target.children.push(ch);
                    continue;
                }
                ll.revert(&state);
                if !ll.eol() {
                    parse_kw(ll, target, false, &mut tag)?;
                }
                while !ll.eol() {
                    parse_kw(ll, target, false, &mut tag)?;
                }
            }
            Ok(())
        };
        if let Some(s) = sub.as_mut() {
            run(s, target)?;
        }
        if c.layout {
            run(l, target)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Contents {
    layout: bool,
    can_has: bool,
    can_tag: bool,
    block_only: bool,
    docstring: bool,
    in_for: bool,
    keyword: bool,
}

fn name_or_tuple(l: &mut Lexer) -> PResult<Option<String>> {
    let mut name = None;
    let mut pattern = false;
    loop {
        if l.match_re(r"\(").is_some() {
            name_or_tuple(l)?;
            l.require_re(r"\)", ")")?;
            pattern = true;
        } else {
            name = l.name();
            if name.is_none() {
                break;
            }
        }
        if l.match_re(",").is_some() {
            pattern = true;
        } else {
            break;
        }
    }
    if pattern {
        return Ok(None);
    }
    if name.is_some() {
        return Ok(name);
    }
    l.error("expected variable or tuple pattern.")
}

/// Argument lists inside screens use the script parser's rules.
fn parse_arguments(l: &mut Lexer) -> PResult<Option<ArgumentInfo>> {
    struct NoCx;
    impl ParseContext for NoCx {
        fn is_warper(&self, _: &str) -> bool {
            false
        }
    }
    impl crate::parser::ScriptContext for NoCx {
        fn user_statement_names(&self) -> Vec<Vec<String>> {
            Vec::new()
        }
        fn parse_user_statement(&self, _: &mut crate::ast::Arena, _: &[String], l: &mut Lexer, _: &(Rc<str>, u32)) -> PResult<Vec<usize>> {
            l.error("unexpected statement")
        }
        fn sl2_registry(&self) -> &Registry {
            unreachable!("screens cannot nest")
        }
    }
    let mut arena = crate::ast::Arena::default();
    let cx = NoCx;
    let mut p = crate::parser::Parser::new(&mut arena, &cx);
    p.parse_arguments(l)
}
