// SPDX-License-Identifier: MPL-2.0
//! Ren'Py statement parser (`renpy/parser.py`). Produces nodes in the
//! shared arena. Creator-defined statements, ATL warpers and screen
//! language parsing consult the load-time `ScriptContext`.

use crate::ast::*;
use crate::atl::{self, ParseContext};
use crate::lexer::{GroupedLine, Lexer, PResult, ParseError, PyExpr, TripleString};
use std::rc::Rc;

/// Load-time hooks: warpers and creator-defined statements registered by
/// `python early` code executed while the script loads.
pub trait ScriptContext: ParseContext {
    /// Longest registered creator-defined statement name matching at the
    /// lexer position (keyword sequence), without consuming input.
    fn user_statement_names(&self) -> Vec<Vec<String>>;
    /// Parses a creator-defined statement. Called with the lexer positioned
    /// after the statement keywords.
    fn parse_user_statement(&self, arena: &mut Arena, name: &[String], l: &mut Lexer, loc: &(Rc<str>, u32)) -> PResult<Vec<NodeId>>;
    /// Screen-language statements (built-in and creator-defined).
    fn sl2_registry(&self) -> &crate::sl2::Registry;
    /// Evaluates a parse-time `IF` condition.
    fn eval_parse_condition(&self, _expr: &PyExpr) -> PResult<bool> {
        Ok(false)
    }
}

pub struct Parser<'a> {
    pub arena: &'a mut Arena,
    pub cx: &'a dyn ScriptContext,
    pub errors: Vec<ParseError>,
}

type Loc = (Rc<str>, u32);

fn imspec_name_parts(name: &[String]) -> Vec<String> {
    name.to_vec()
}

impl<'a> Parser<'a> {
    pub fn new(arena: &'a mut Arena, cx: &'a dyn ScriptContext) -> Self {
        Parser { arena, cx, errors: Vec::new() }
    }

    fn add(&mut self, loc: &Loc, kind: NodeKind) -> NodeId {
        self.arena.add(loc, kind)
    }

    pub fn parse_image_name(&mut self, l: &mut Lexer, string: bool, nodash: bool) -> PResult<Vec<String>> {
        let mut rv = vec![match l.image_name_component() {
            Some(c) => c,
            None => return l.error("expected 'image_name_component' not found."),
        }];
        while let Some(n) = l.image_name_component() {
            rv.push(n.trim().to_string());
        }
        if string {
            let cp = l.checkpoint();
            match l.image_simple_expression()? {
                Some(s) => rv.push(s.text.to_string()),
                None => l.revert(&cp),
            }
        }
        if nodash && rv.iter().any(|c| c.starts_with('-')) {
            return l.error("image name components may not begin with a '-'.");
        }
        Ok(rv)
    }

    fn simple_list(&mut self, l: &mut Lexer, image: bool) -> PResult<Vec<PyExpr>> {
        let first = if image { l.image_simple_expression()? } else { l.simple_expression()? };
        let Some(first) = first else { return l.error("expected 'simple_expression' not found.") };
        let mut rv = vec![first];
        while l.match_re(",").is_some() {
            let e = if image { l.image_simple_expression()? } else { l.simple_expression()? };
            match e {
                Some(e) => rv.push(e),
                None => break,
            }
        }
        Ok(rv)
    }

    pub fn parse_image_specifier(&mut self, l: &mut Lexer) -> PResult<ImSpec> {
        let mut spec = ImSpec::default();
        if l.keyword("expression") || l.keyword("image") {
            let e = match l.image_simple_expression()? {
                Some(e) => e,
                None => return l.error("expected 'simple_expression' not found."),
            };
            spec.name = vec![e.text.trim().to_string()];
            spec.expression = Some(e);
        } else {
            spec.name = imspec_name_parts(&self.parse_image_name(l, true, false)?);
        }
        loop {
            if l.keyword("onlayer") {
                if spec.layer.is_some() {
                    return l.error("multiple onlayer clauses are prohibited.");
                }
                spec.layer = Some(l.image_name_component().map_or_else(|| l.error("expected 'image_name_component' not found."), Ok)?);
                continue;
            }
            if l.keyword("at") {
                if !spec.at_list.is_empty() {
                    return l.error("multiple at clauses are prohibited.");
                }
                spec.at_list = self.simple_list(l, true)?;
                continue;
            }
            if l.keyword("as") {
                if spec.tag.is_some() {
                    return l.error("multiple as clauses are prohibited.");
                }
                spec.tag = Some(l.require_name()?);
                continue;
            }
            if l.keyword("zorder") {
                if spec.zorder.is_some() {
                    return l.error("multiple zorder clauses are prohibited.");
                }
                spec.zorder = Some(match l.image_simple_expression()? {
                    Some(e) => e,
                    None => return l.error("expected 'simple_expression' not found."),
                });
                continue;
            }
            if l.keyword("behind") {
                if !spec.behind.is_empty() {
                    return l.error("multiple behind clauses are prohibited.");
                }
                loop {
                    spec.behind.push(l.require_name()?);
                    if l.match_re(",").is_none() {
                        break;
                    }
                }
                continue;
            }
            break;
        }
        Ok(spec)
    }

    /// `parse_with`: `stmt with t` becomes With(None, paired=t), stmt, With(t).
    fn parse_with(&mut self, l: &mut Lexer, node: NodeId) -> PResult<Vec<NodeId>> {
        let loc = l.location();
        if !l.keyword("with") {
            return Ok(vec![node]);
        }
        let expr = l.require_simple()?;
        let a = self.add(&loc, NodeKind::With { expr: None, paired: Some(expr.clone()) });
        let b = self.add(&loc, NodeKind::With { expr: Some(expr), paired: None });
        Ok(vec![a, node, b])
    }

    pub fn parse_parameters(&mut self, l: &mut Lexer) -> PResult<Option<Signature>> {
        if l.match_re(r"\(").is_none() {
            return Ok(None);
        }
        let mut params: Vec<Parameter> = Vec::new();
        let mut kind = 1u8;
        let mut got_slash = false;
        let mut kwonly = false;
        let mut now_default = false;
        loop {
            if l.match_re(r"\)").is_some() {
                break;
            }
            if l.match_re(r"\*\*").is_some() {
                let n = l.require_name()?;
                params.push(Parameter { name: n, kind: 4, default: None });
                l.match_re(",");
                l.require_re(r"\)", ")")?;
                break;
            } else if l.match_re(r"\*").is_some() {
                if kwonly {
                    return l.error("* may appear only once");
                }
                kwonly = true;
                kind = 3;
                now_default = false;
                if let Some(n) = l.name() {
                    params.push(Parameter { name: n, kind: 2, default: None });
                }
            } else if l.match_re("/").is_some() {
                if kwonly || got_slash || params.is_empty() {
                    return l.error("/ must follow at least one parameter, before *, and appear once");
                }
                for p in params.iter_mut() {
                    p.kind = 0;
                }
                got_slash = true;
            } else {
                let name = l.require_name()?;
                let mut default = None;
                if l.match_re("=").is_some() {
                    l.skip_whitespace();
                    let d = l.delimited_python("),")?.trim().to_string();
                    if d.is_empty() {
                        return l.error(format!("empty default value for parameter {name:?}"));
                    }
                    default = Some(d);
                    now_default = true;
                } else if now_default && !kwonly {
                    return l.error(format!("non-default parameter {name:?} follows a default parameter"));
                }
                if params.iter().any(|p| p.name == name) {
                    return l.error(format!("duplicate parameter name {name:?}"));
                }
                params.push(Parameter { name, kind, default });
            }
            if l.match_re(r"\)").is_some() {
                break;
            }
            l.require_re(",", ",")?;
        }
        Ok(Some(Signature { parameters: params }))
    }

    pub fn parse_arguments(&mut self, l: &mut Lexer) -> PResult<Option<ArgumentInfo>> {
        if l.match_re(r"\(").is_none() {
            return Ok(None);
        }
        let mut info = ArgumentInfo::default();
        if l.match_re(r"\)").is_some() {
            return Ok(Some(info));
        }
        let mut index = 0;
        let mut keyword_parsed = false;
        loop {
            if l.match_re(r"\)").is_some() {
                break;
            }
            let mut star = false;
            if l.match_re(r"\*\*").is_some() {
                info.doublestarred.push(index);
                star = true;
            } else if l.match_re(r"\*").is_some() {
                info.starred.push(index);
                star = true;
            }
            let mut name = None;
            if !star {
                let cp = l.checkpoint();
                let w = l.word();
                if w.is_some() && l.match_re("=").is_some() && l.match_re("=").is_none() {
                    let w = w.unwrap();
                    if info.arguments.iter().any(|(n, _)| n.as_deref() == Some(&w)) {
                        return l.error(format!("keyword argument repeated: '{w}'"));
                    }
                    name = Some(w);
                    keyword_parsed = true;
                } else if keyword_parsed {
                    return l.error("positional argument follows keyword argument");
                } else {
                    l.revert(&cp);
                }
            }
            l.skip_whitespace();
            let e = l.delimited_python("),")?;
            info.arguments.push((name, e.trim().to_string()));
            if l.match_re(r"\)").is_some() {
                break;
            }
            l.require_re(",", ",")?;
            index += 1;
        }
        Ok(Some(info))
    }

    fn say_attributes(&mut self, l: &mut Lexer) -> Option<Vec<String>> {
        let mut attrs = Vec::new();
        loop {
            let prefix = if l.match_re("-").is_some() { "-" } else { "" };
            match l.image_name_component() {
                Some(c) => attrs.push(format!("{prefix}{c}")),
                None => break,
            }
        }
        (!attrs.is_empty()).then_some(attrs)
    }

    fn what(&mut self, l: &mut Lexer) -> Option<What> {
        match l.triple_string() {
            Some(TripleString::Parts(p)) => Some(What::Parts(p)),
            Some(TripleString::Raw(s)) => Some(What::One(s)),
            None => l.string().map(What::One),
        }
    }

    fn finish_say(&mut self, l: &mut Lexer, loc: &Loc, who: Option<PyExpr>, what: What, attributes: Option<Vec<String>>, temporary: Option<Vec<String>>, mut interact: bool) -> PResult<Vec<NodeId>> {
        let mut with_ = None;
        let mut arguments = None;
        let mut identifier = None;
        loop {
            if l.keyword("nointeract") {
                interact = false;
            } else if l.keyword("with") {
                if with_.is_some() {
                    return l.error("say can only take a single with clause");
                }
                with_ = Some(l.require_simple()?);
            } else if l.keyword("id") {
                identifier = Some(l.require_name()?);
            } else {
                match self.parse_arguments(l)? {
                    Some(a) => {
                        if arguments.is_some() {
                            return l.error("say can only take a single set of arguments");
                        }
                        arguments = Some(a);
                    }
                    None => break,
                }
            }
        }
        let mk = |p: &mut Self, loc: &Loc, text: String| {
            p.add(loc, NodeKind::Say {
                who: who.clone(),
                what: text,
                with_: with_.clone(),
                interact,
                attributes: attributes.clone(),
                temporary_attributes: temporary.clone(),
                arguments: arguments.clone(),
                explicit_identifier: identifier.is_some(),
                identifier: identifier.clone(),
            })
        };
        match what {
            What::One(s) => Ok(vec![mk(self, loc, s)]),
            What::Parts(parts) => {
                if parts.len() > 1 && identifier.is_some() {
                    return l.error("Monologue mode say statements cannot have an id clause.");
                }
                let mut rv = Vec::new();
                for (off, text) in parts {
                    let ploc = (loc.0.clone(), loc.1 + off);
                    if text == "{clear}" {
                        rv.push(self.add(&ploc, NodeKind::UserStatement { line: "nvl clear".into(), block: Rc::new(Vec::new()), parsed: None, subparses: Vec::new(), code_block: None, init_priority: 0, translatable: false, atl: None }));
                    } else {
                        rv.push(mk(self, &ploc, text));
                    }
                }
                Ok(rv)
            }
        }
    }

    fn parse_menu(&mut self, stmtl: &mut Lexer, loc: &Loc, arguments: Option<ArgumentInfo>) -> PResult<Vec<NodeId>> {
        let mut l = stmtl.subblock_lexer(false);
        let mut has_choice = false;
        let mut say_ast: Option<NodeId> = None;
        let mut has_caption = false;
        let mut with_ = None;
        let mut set = None;
        let mut items = Vec::new();
        while l.advance() {
            if l.keyword("with") {
                with_ = Some(l.require_simple()?);
                l.expect_eol()?;
                l.expect_noblock("with clause")?;
                continue;
            }
            if l.keyword("set") {
                set = Some(l.require_simple()?);
                l.expect_eol()?;
                l.expect_noblock("set menuitem")?;
                continue;
            }
            let state = l.checkpoint();
            let who = l.simple_expression()?;
            let attributes = self.say_attributes(&mut l);
            let temporary = if l.match_re(r"\@").is_some() { self.say_attributes(&mut l) } else { None };
            let what = self.what(&mut l);
            if let (Some(who), Some(what)) = (who, what) {
                if has_caption {
                    return l.error("Say menuitems and captions may not exist in the same menu.");
                }
                if say_ast.is_some() {
                    return l.error("Only one say menuitem may exist per menu.");
                }
                let sloc = l.location();
                let v = self.finish_say(&mut l, &sloc, Some(who), what, attributes, temporary, false)?;
                if v.len() != 1 {
                    return l.error("Monologue mode cannot be used in a menu.");
                }
                say_ast = Some(v[0]);
                l.expect_eol()?;
                l.expect_noblock("say menuitem")?;
                continue;
            }
            l.revert(&state);
            let Some(label) = l.string() else { return l.error("expected menuitem") };
            if l.eol() {
                if l.has_block() {
                    return l.error("Line is followed by a block, despite not being a menu choice. Did you forget a colon at the end of the line?");
                }
                if !label.is_empty() && say_ast.is_some() {
                    return l.error("Captions and say menuitems may not exist in the same menu.");
                }
                if !label.is_empty() {
                    has_caption = true;
                }
                items.push(MenuItem { label, condition: l.expr("True".into()), block: None, arguments: None });
                continue;
            }
            has_choice = true;
            let args = self.parse_arguments(&mut l)?;
            let condition = if l.keyword("if") { l.python_expression()? } else { l.expr("True".into()) };
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("choice menuitem")?;
            let mut sub = l.subblock_lexer(false);
            let block = self.parse_block(&mut sub)?;
            items.push(MenuItem { label, condition, block: Some(block), arguments: args });
        }
        if !has_choice {
            return stmtl.error("Menu does not contain any choices.");
        }
        let mut rv = Vec::new();
        if let Some(s) = say_ast {
            rv.push(s);
        }
        let m = self.add(loc, NodeKind::Menu { items, set, with_, has_caption: say_ast.is_some() || has_caption, arguments });
        rv.push(m);
        for (i, id) in rv.iter().enumerate() {
            self.arena.get_mut(*id).rollback = if i == 0 { Rollback::Force } else { Rollback::Normal };
        }
        Ok(rv)
    }

    fn init_wrap(&mut self, l: &Lexer, loc: &Loc, node: NodeId, priority: i32, create: bool) -> NodeId {
        if create {
            self.add(loc, NodeKind::Init { block: vec![node], priority: priority + l.init_offset })
        } else {
            node
        }
    }

    fn priority(&mut self, l: &mut Lexer) -> (i32, bool) {
        match l.integer() {
            Some(p) => (p.parse().unwrap_or(0), true),
            None => (0, false),
        }
    }

    fn store_name(&mut self, l: &mut Lexer, word: bool) -> PResult<(String, String)> {
        let mut store = String::from("store");
        let mut name = if word { l.word() } else { l.name() }.map_or_else(|| l.error("expected 'name' not found."), Ok)?;
        while l.match_re(r"\.").is_some() {
            store = format!("{store}.{name}");
            name = l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?;
        }
        Ok((store, name))
    }

    fn pycode(&self, src: PyExpr, mode: &'static str) -> PyCode {
        PyCode { source: src, mode, py3: false }
    }

    /// One statement at the lexer's current line (may produce several nodes).
    pub fn parse_statement(&mut self, l: &mut Lexer) -> PResult<Vec<NodeId>> {
        let loc = l.location();
        // Creator-defined statements take precedence where registered names
        // match (Ren'Py's ParseTrie also holds them).
        let start = l.checkpoint();
        if let Some(name) = self.match_user(l) {
            return self.cx.parse_user_statement(self.arena, &name, l, &loc);
        }
        l.revert(&start);
        let old = l.pos;
        let w = l.word().or_else(|| l.match_re(r"\$"));
        let Some(w) = w else {
            l.pos = old;
            return self.say_statement(l, &loc);
        };
        let r = match w.as_str() {
            "if" => self.if_statement(l, &loc),
            "IF" => self.parse_time_if(l),
            "while" => self.while_statement(l, &loc),
            "pass" => {
                l.expect_noblock("pass statement")?;
                l.expect_eol()?;
                l.advance();
                Ok(vec![self.add(&loc, NodeKind::Pass)])
            }
            "menu" => self.menu_statement(l, &loc),
            "return" => {
                l.expect_noblock("return statement")?;
                let rest = l.rest();
                let e = (!rest.text.is_empty()).then_some(rest);
                l.expect_eol()?;
                l.advance();
                Ok(vec![self.add(&loc, NodeKind::Return { expression: e })])
            }
            "jump" => {
                l.expect_noblock("jump statement")?;
                let (target, expression) = if l.keyword("expression") {
                    (l.require_simple()?.text.to_string(), true)
                } else {
                    (l.label_name().map_or_else(|| l.error("expected 'label_name' not found."), Ok)?, false)
                };
                l.expect_eol()?;
                l.advance();
                let gl = if expression { l.global_label.as_deref().unwrap_or("").to_string() } else { String::new() };
                Ok(vec![self.add(&loc, NodeKind::Jump { target, expression, global_label: gl })])
            }
            "call" => self.call_statement(l, &loc),
            "scene" => self.scene_statement(l, &loc),
            "show" => {
                if l.keyword("layer") {
                    return self.show_layer(l, &loc, false);
                }
                self.show_statement(l, &loc)
            }
            "camera" => self.show_layer(l, &loc, true),
            "hide" => {
                let spec = self.parse_image_specifier(l)?;
                let n = self.add(&loc, NodeKind::Hide { imspec: spec });
                let rv = self.parse_with(l, n)?;
                l.expect_eol()?;
                l.expect_noblock("hide statement")?;
                l.advance();
                Ok(rv)
            }
            "with" => {
                let e = l.require_simple()?;
                l.expect_eol()?;
                l.expect_noblock("with statement")?;
                l.advance();
                let expr = if &*e.text == "None" { None } else { Some(e) };
                Ok(vec![self.add(&loc, NodeKind::With { expr, paired: None })])
            }
            "image" => self.image_statement(l, &loc),
            "define" => self.define_statement(l, &loc),
            "default" => self.default_statement(l, &loc),
            "transform" => self.transform_statement(l, &loc),
            "$" => {
                let code = l.rest();
                if code.text.is_empty() {
                    return l.error("expected python code");
                }
                l.expect_noblock("one-line python statement")?;
                l.advance();
                Ok(vec![self.add(&loc, NodeKind::Python { code: self.pycode(code, "exec"), hide: false, store: "store".into() })])
            }
            "python" => self.python_statement(l, &loc),
            "label" => self.label_statement(l, &loc, false),
            "init" => {
                if l.keyword("offset") {
                    l.require_re("=", "=")?;
                    let off = l.integer().map_or_else(|| l.error("expected 'integer' not found."), Ok)?;
                    l.expect_eol()?;
                    l.expect_noblock("init offset statement")?;
                    l.advance();
                    l.init_offset = off.parse().unwrap_or(0);
                    return Ok(Vec::new());
                }
                if l.keyword("label") {
                    return self.label_statement(l, &loc, true);
                }
                self.init_statement(l, &loc)
            }
            "rpy" => {
                if l.keyword("monologue") {
                    l.monologue_delimiter = if l.keyword("double") {
                        "\n\n".into()
                    } else if l.keyword("single") {
                        "\n".into()
                    } else if l.keyword("none") {
                        String::new()
                    } else {
                        return l.error("rpy monologue expects either none, single or double.");
                    };
                    l.expect_eol()?;
                    l.expect_noblock("rpy monologue")?;
                    l.advance();
                    return Ok(Vec::new());
                }
                if l.keyword("python") {
                    let mut rv = Vec::new();
                    loop {
                        let r = l.match_re("3").map_or_else(|| l.word().map_or_else(|| l.error("expected '__future__ name' not found."), Ok), Ok)?;
                        rv.push(self.add(&loc, NodeKind::Rpy { rest: ("python".into(), r) }));
                        if l.match_re(",").is_none() {
                            break;
                        }
                    }
                    l.expect_eol()?;
                    l.expect_noblock("rpy statement")?;
                    l.advance();
                    return Ok(rv);
                }
                l.pos = old;
                self.say_statement(l, &loc)
            }
            "screen" => {
                if let Some(v) = l.integer() {
                    let n: i32 = v.parse().unwrap_or(-1);
                    if !(0..=2).contains(&n) {
                        return l.error("Bad screen language version.");
                    }
                }
                let cx = self.cx;
                let mut pa = |ll: &mut Lexer| self.parse_parameters(ll);
                let screen = cx.sl2_registry().parse_screen(l, &loc, cx, &mut pa)?;
                l.advance();
                let n = self.add(&loc, NodeKind::Screen { screen: Rc::new(screen) });
                let create = !l.init;
                Ok(vec![self.init_wrap(l, &loc, n, -500, create)])
            }
            "testcase" | "testsuite" => {
                let name = l.require_name()?;
                // Test cases only run under the Ren'Py test runner.
                l.advance();
                Ok(vec![self.add(&loc, NodeKind::Testcase { name })])
            }
            "translate" => self.translate_statement(l, &loc),
            "style" => self.style_statement(l, &loc),
            _ => {
                l.pos = old;
                self.say_statement(l, &loc)
            }
        };
        r
    }

    fn match_user(&self, l: &mut Lexer) -> Option<Vec<String>> {
        let names = self.cx.user_statement_names();
        if names.is_empty() {
            return None;
        }
        let start = l.checkpoint();
        let mut best: Option<(usize, Vec<String>, crate::lexer::Checkpoint)> = None;
        for n in names {
            l.revert(&start);
            let mut ok = true;
            for w in &n {
                let old = l.pos;
                let got = l.word().or_else(|| l.match_re(r"\$"));
                if got.as_deref() != Some(w.as_str()) {
                    l.pos = old;
                    ok = false;
                    break;
                }
            }
            if ok && best.as_ref().map(|(len, _, _)| n.len() > *len).unwrap_or(true) {
                best = Some((n.len(), n.clone(), l.checkpoint()));
            }
        }
        match best {
            Some((_, n, cp)) => {
                l.revert(&cp);
                Some(n)
            }
            None => {
                l.revert(&start);
                None
            }
        }
    }

    fn if_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let mut entries = Vec::new();
        let cond = l.python_expression()?;
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("if statement")?;
        let mut sub = l.subblock_lexer(false);
        entries.push((cond, self.parse_block(&mut sub)?));
        l.advance();
        while l.keyword("elif") {
            let cond = l.python_expression()?;
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("elif clause")?;
            let mut sub = l.subblock_lexer(false);
            entries.push((cond, self.parse_block(&mut sub)?));
            l.advance();
        }
        if l.keyword("else") {
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("else clause")?;
            let mut sub = l.subblock_lexer(false);
            entries.push((l.expr("True".into()), self.parse_block(&mut sub)?));
            l.advance();
        }
        Ok(vec![self.add(loc, NodeKind::If { entries })])
    }

    fn parse_time_if(&mut self, l: &mut Lexer) -> PResult<Vec<NodeId>> {
        let mut rv: Option<Vec<NodeId>> = None;
        let cond = l.python_expression()?;
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("IF statement")?;
        if self.cx.eval_parse_condition(&cond)? {
            let mut sub = l.subblock_lexer(false);
            rv = Some(self.parse_block(&mut sub)?);
        }
        l.advance();
        while l.keyword("ELIF") {
            let cond = l.python_expression()?;
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("ELIF clause")?;
            if rv.is_none() && self.cx.eval_parse_condition(&cond)? {
                let mut sub = l.subblock_lexer(false);
                rv = Some(self.parse_block(&mut sub)?);
            }
            l.advance();
        }
        if l.keyword("ELSE") {
            l.require_re(":", ":")?;
            l.expect_eol()?;
            l.expect_block("ELSE clause")?;
            if rv.is_none() {
                let mut sub = l.subblock_lexer(false);
                rv = Some(self.parse_block(&mut sub)?);
            }
            l.advance();
        }
        Ok(rv.unwrap_or_default())
    }

    fn while_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let cond = l.python_expression()?;
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("while statement")?;
        let mut sub = l.subblock_lexer(false);
        let block = self.parse_block(&mut sub)?;
        l.advance();
        Ok(vec![self.add(loc, NodeKind::While { condition: cond, block })])
    }

    fn menu_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        l.expect_block("menu statement")?;
        let label = l.label_name();
        if let Some(lb) = &label {
            l.set_global_label(lb);
        }
        let args = self.parse_arguments(l)?;
        l.require_re(":", ":")?;
        l.expect_eol()?;
        let menu = self.parse_menu(l, loc, args)?;
        l.advance();
        let mut rv = Vec::new();
        if let Some(lb) = label {
            rv.push(self.add(loc, NodeKind::Label { name: lb.as_str().into(), block: Vec::new(), parameters: None, hide: false }));
        }
        rv.extend(menu);
        let first = rv[0];
        for id in &rv {
            if matches!(self.arena.get(*id).kind, NodeKind::Menu { .. }) {
                self.arena.get_mut(*id).statement_start = Some(first);
            }
        }
        Ok(rv)
    }

    fn call_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        l.expect_noblock("call statement")?;
        let (target, expression) = if l.keyword("expression") {
            (l.require_simple()?.text.to_string(), true)
        } else {
            (l.label_name().map_or_else(|| l.error("expected 'label_name' not found."), Ok)?, false)
        };
        l.keyword("pass");
        let arguments = self.parse_arguments(l)?;
        let gl = if expression { l.global_label.as_deref().unwrap_or("").to_string() } else { String::new() };
        let mut rv = vec![self.add(loc, NodeKind::Call { label: target, expression, arguments, global_label: gl })];
        if l.keyword("from") {
            let name = l.label_name().map_or_else(|| l.error("expected 'label_name' not found."), Ok)?;
            rv.push(self.add(loc, NodeKind::Label { name: name.as_str().into(), block: Vec::new(), parameters: None, hide: false }));
        }
        rv.push(self.add(loc, NodeKind::Pass));
        l.expect_eol()?;
        l.advance();
        Ok(rv)
    }

    fn scene_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let mut layer = None;
        if l.keyword("onlayer") {
            layer = Some(l.image_name_component().map_or_else(|| l.error("expected 'image_name_component' not found."), Ok)?);
            l.expect_eol()?;
        }
        if layer.is_some() || l.eol() {
            l.advance();
            return Ok(vec![self.add(loc, NodeKind::Scene { imspec: None, layer, atl: None })]);
        }
        let spec = self.parse_image_specifier(l)?;
        let layer = spec.layer.clone();
        let n = self.add(loc, NodeKind::Scene { imspec: Some(spec), layer, atl: None });
        let rv = self.parse_with(l, n)?;
        if l.match_re(":").is_some() {
            l.expect_block("scene statement")?;
            let mut sub = l.subblock_lexer(false);
            let a = atl::parse_atl(&mut sub, self.cx)?;
            if let NodeKind::Scene { atl, .. } = &mut self.arena.get_mut(n).kind {
                *atl = Some(Rc::new(a));
            }
        } else {
            l.expect_noblock("scene statement")?;
        }
        l.expect_eol()?;
        l.advance();
        Ok(rv)
    }

    fn show_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let spec = self.parse_image_specifier(l)?;
        let n = self.add(loc, NodeKind::Show { imspec: spec, atl: None });
        let rv = self.parse_with(l, n)?;
        if l.match_re(":").is_some() {
            l.expect_block("show statement")?;
            let mut sub = l.subblock_lexer(false);
            let a = atl::parse_atl(&mut sub, self.cx)?;
            if let NodeKind::Show { atl, .. } = &mut self.arena.get_mut(n).kind {
                *atl = Some(Rc::new(a));
            }
        } else {
            l.expect_noblock("show statement")?;
        }
        l.expect_eol()?;
        l.advance();
        Ok(rv)
    }

    fn show_layer(&mut self, l: &mut Lexer, loc: &Loc, camera: bool) -> PResult<Vec<NodeId>> {
        let layer = if camera {
            l.image_name_component().unwrap_or_else(|| "master".into())
        } else {
            l.image_name_component().map_or_else(|| l.error("expected 'image_name_component' not found."), Ok)?
        };
        let at_list = if l.keyword("at") { self.simple_list(l, false)? } else { Vec::new() };
        let atl = if l.match_re(":").is_some() {
            l.expect_block(if camera { "camera statement" } else { "show layer statement" })?;
            let mut sub = l.subblock_lexer(false);
            Some(Rc::new(atl::parse_atl(&mut sub, self.cx)?))
        } else {
            l.expect_noblock(if camera { "camera statement" } else { "show layer statement" })?;
            None
        };
        l.expect_eol()?;
        l.advance();
        let kind = if camera { NodeKind::Camera { layer, at_list, atl } } else { NodeKind::ShowLayer { layer, at_list, atl } };
        Ok(vec![self.add(loc, kind)])
    }

    fn image_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let name = self.parse_image_name(l, false, true)?;
        let (code, atl) = if l.match_re(":").is_some() {
            l.expect_eol()?;
            l.expect_block("image statement")?;
            let mut sub = l.subblock_lexer(false);
            (None, Some(Rc::new(atl::parse_atl(&mut sub, self.cx)?)))
        } else {
            l.require_re("=", "=")?;
            let e = l.rest();
            if e.text.is_empty() {
                return l.error("expected expression");
            }
            l.expect_noblock("image statement")?;
            (Some(self.pycode(e, "eval")), None)
        };
        let n = self.add(loc, NodeKind::Image { name, code, atl });
        let create = !l.init;
        let rv = self.init_wrap(l, loc, n, 500, create);
        l.advance();
        Ok(vec![rv])
    }

    fn define_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let (priority, explicit) = self.priority(l);
        let create = !l.init || explicit;
        let (store, name) = self.store_name(l, true)?;
        let index = if l.match_re(r"\[").is_some() {
            let e = l.delimited_python("]")?;
            l.require_re(r"\]", "]")?;
            Some(l.expr(e.trim().to_string()))
        } else {
            None
        };
        let operator = if l.match_re(r"\+=").is_some() {
            "+="
        } else if l.match_re(r"\|=").is_some() {
            "|="
        } else {
            l.require_re("=", "=")?;
            "="
        };
        let e = l.rest();
        if e.text.is_empty() {
            return l.error("expected expression");
        }
        l.expect_noblock("define statement")?;
        let n = self.add(loc, NodeKind::Define { store, name, index, operator: operator.into(), code: self.pycode(e, "eval") });
        let rv = self.init_wrap(l, loc, n, priority, create);
        l.advance();
        Ok(vec![rv])
    }

    fn default_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let (priority, explicit) = self.priority(l);
        let create = !l.init || explicit;
        let (store, name) = self.store_name(l, true)?;
        l.require_re("=", "=")?;
        let e = l.rest();
        if e.text.is_empty() {
            return l.error("expected expression");
        }
        l.expect_noblock("default statement")?;
        let n = self.add(loc, NodeKind::Default { store, name, code: self.pycode(e, "eval") });
        let rv = self.init_wrap(l, loc, n, priority, create);
        l.advance();
        Ok(vec![rv])
    }

    fn transform_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let (priority, explicit) = self.priority(l);
        let create = !l.init || explicit;
        let (store, name) = self.store_name(l, false)?;
        let parameters = self.parse_parameters(l)?;
        if let Some(sig) = &parameters {
            for p in &sig.parameters {
                match p.kind {
                    2 => return l.error(format!("the transform statement does not take *args (*{} is not allowed)", p.name)),
                    4 => return l.error(format!("the transform statement does not take **kwargs (**{} is not allowed)", p.name)),
                    3 if p.default.is_none() => return l.error(format!("the transform statement does not take required keyword-only parameters ({} is not allowed)", p.name)),
                    _ => {}
                }
            }
        }
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("transform statement")?;
        let mut sub = l.subblock_lexer(false);
        let a = atl::parse_atl(&mut sub, self.cx)?;
        let n = self.add(loc, NodeKind::Transform { store, name, atl: Rc::new(a), parameters });
        let rv = self.init_wrap(l, loc, n, priority, create);
        l.advance();
        Ok(vec![rv])
    }

    fn python_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let early = l.keyword("early");
        let hide = l.keyword("hide");
        let mut store = String::from("store");
        if l.keyword("in") {
            let d = l.dotted_name()?.map_or_else(|| l.error("expected 'dotted_name' not found."), Ok)?;
            store = format!("store.{d}");
        }
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("python block")?;
        let src = l.python_block();
        let code = PyCode { source: PyExpr::new(src, l.filename.clone(), l.number + 1), mode: if hide { "hide" } else { "exec" }, py3: false };
        l.advance();
        let kind = if early { NodeKind::EarlyPython { code, hide, store } } else { NodeKind::Python { code, hide, store } };
        Ok(vec![self.add(loc, kind)])
    }

    fn label_statement(&mut self, l: &mut Lexer, loc: &Loc, init: bool) -> PResult<Vec<NodeId>> {
        let name = l.label_name().map_or_else(|| l.error("expected 'label_name' not found."), Ok)?;
        l.set_global_label(&name);
        let parameters = self.parse_parameters(l)?;
        let hide = l.keyword("hide");
        l.require_re(":", ":")?;
        l.expect_eol()?;
        let mut sub = l.subblock_lexer(init);
        let block = self.parse_block(&mut sub)?;
        l.advance();
        Ok(vec![self.add(loc, NodeKind::Label { name: name.as_str().into(), block, parameters, hide })])
    }

    fn init_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let priority: i32 = l.integer().and_then(|p| p.parse().ok()).unwrap_or(0);
        let block = if l.match_re(":").is_some() {
            l.expect_eol()?;
            l.expect_block("init statement")?;
            let mut sub = l.subblock_lexer(true);
            let b = self.parse_block(&mut sub)?;
            l.advance();
            b
        } else {
            let old_init = l.init;
            l.init = true;
            let cp = l.checkpoint();
            let r = self.parse_statement(l);
            l.init = old_init;
            let r = r?;
            if r.is_empty() {
                l.revert(&cp);
                return l.error("init expects a block or statement");
            }
            r
        };
        Ok(vec![self.add(loc, NodeKind::Init { block, priority: priority + l.init_offset })])
    }

    fn translate_strings(&mut self, init_loc: &Loc, language: Option<String>, l: &mut Lexer) -> PResult<Vec<NodeId>> {
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("translate strings statement")?;
        let mut ll = l.subblock_lexer(false);
        let mut block = Vec::new();
        let mut old: Option<(String, Loc)> = None;
        while ll.advance() {
            if ll.keyword("old") {
                if old.is_some() {
                    return ll.error("previous string is missing a translation");
                }
                let loc = ll.location();
                let s = literal_string(&ll.raw_rest()).map_or_else(|| ll.error("could not parse string"), Ok)?;
                old = Some((s, loc));
            } else if ll.keyword("new") {
                let Some((o, loc)) = old.take() else { return ll.error("no string to translate") };
                let s = literal_string(&ll.raw_rest()).map_or_else(|| ll.error("could not parse string"), Ok)?;
                block.push(self.add(&loc, NodeKind::TranslateString { language: language.clone(), old: o, new: s }));
            } else {
                return ll.error("unknown statement");
            }
        }
        if old.is_some() {
            return ll.error("final string is missing a translation");
        }
        l.advance();
        if l.init {
            return Ok(block);
        }
        Ok(vec![self.add(init_loc, NodeKind::Init { block, priority: l.init_offset })])
    }

    fn translate_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let language = l.require_name()?;
        let language = if language == "None" { None } else { Some(language) };
        let ident = l.hash().map_or_else(|| l.error("expected 'hash' not found."), Ok)?;
        match ident.as_str() {
            "strings" => return self.translate_strings(loc, language, l),
            "python" => {
                let old = l.init;
                l.init = true;
                let r = self.python_statement(l, loc);
                l.init = old;
                let block = r?;
                return Ok(vec![self.add(loc, NodeKind::TranslateEarlyBlock { language, block })]);
            }
            "style" => {
                let old = l.init;
                l.init = true;
                let r = self.style_statement(l, loc);
                l.init = old;
                let block = r?;
                return Ok(vec![self.add(loc, NodeKind::TranslateBlock { language, block })]);
            }
            _ => {}
        }
        l.require_re(":", ":")?;
        l.expect_eol()?;
        l.expect_block("translate statement")?;
        let mut sub = l.subblock_lexer(false);
        let block = self.parse_block(&mut sub)?;
        l.advance();
        let t = self.add(loc, NodeKind::Translate { identifier: ident, language, block, alternate: None });
        let e = self.add(loc, NodeKind::EndTranslate);
        Ok(vec![t, e])
    }

    fn style_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let name = l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?;
        let mut st = Style { name, parent: None, clear: false, take: None, delattr: Vec::new(), variant: None, properties: Vec::new() };
        fn clause(l: &mut Lexer, st: &mut Style) -> PResult<bool> {
            if l.keyword("is") {
                if st.parent.is_some() {
                    return l.error("parent clause appears twice.");
                }
                st.parent = Some(l.word().map_or_else(|| l.error("expected 'word' not found."), Ok)?);
                return Ok(true);
            }
            if l.keyword("clear") {
                st.clear = true;
                return Ok(true);
            }
            if l.keyword("take") {
                if st.take.is_some() {
                    return l.error("take clause appears twice.");
                }
                st.take = Some(l.require_name()?);
                return Ok(true);
            }
            if l.keyword("del") {
                st.delattr.push(l.require_name()?);
                return Ok(true);
            }
            if l.keyword("variant") {
                if st.variant.is_some() {
                    return l.error("variant clause appears twice.");
                }
                st.variant = Some(l.require_simple()?);
                return Ok(true);
            }
            if let Some(p) = l.name() {
                if st.properties.iter().any(|(n, _)| *n == p) {
                    return l.error(format!("style property {p} appears twice."));
                }
                let e = l.require_simple()?;
                st.properties.push((p, e));
                return Ok(true);
            }
            Ok(false)
        }
        while clause(l, &mut st)? {}
        if l.match_re(":").is_none() {
            l.expect_noblock("style statement")?;
            l.expect_eol()?;
        } else {
            l.expect_block("style statement")?;
            l.expect_eol()?;
            let mut ll = l.subblock_lexer(false);
            while ll.advance() {
                while clause(&mut ll, &mut st)? {}
                ll.expect_eol()?;
            }
        }
        let n = self.add(loc, NodeKind::Style(Box::new(st)));
        let create = !l.init;
        let rv = self.init_wrap(l, loc, n, 0, create);
        l.advance();
        Ok(vec![rv])
    }

    fn say_statement(&mut self, l: &mut Lexer, loc: &Loc) -> PResult<Vec<NodeId>> {
        let state = l.checkpoint();
        if let Some(what) = self.what(l) {
            let rv = self.finish_say(l, loc, None, what, None, None, true)?;
            if l.eol() {
                l.expect_noblock("say statement")?;
                l.advance();
                return Ok(rv);
            }
            // Discard nodes created on this failed path.
            let keep = self.arena.nodes.len() - rv.len();
            self.arena.nodes.truncate(keep);
        }
        l.revert(&state);
        let who = l.say_expression()?;
        let attributes = self.say_attributes(l);
        let temporary = if l.match_re(r"\@").is_some() { self.say_attributes(l) } else { None };
        let what = self.what(l);
        if let (Some(who), Some(what)) = (who, what) {
            let rv = self.finish_say(l, loc, Some(who), what, attributes, temporary, true)?;
            l.expect_eol()?;
            l.expect_noblock("say statement")?;
            l.advance();
            return Ok(rv);
        }
        l.error("expected statement.")
    }

    pub fn parse_block(&mut self, l: &mut Lexer) -> PResult<Vec<NodeId>> {
        l.advance();
        let mut rv = Vec::new();
        while !l.eob {
            match self.parse_statement(l) {
                Ok(v) => rv.extend(v),
                Err(e) => {
                    self.errors.push(e);
                    l.advance();
                }
            }
        }
        Ok(rv)
    }

    /// Parses a whole file; appends the implicit trailing `return`.
    pub fn parse_file(&mut self, block: Rc<Vec<GroupedLine>>, filename: &str, last_line: u32) -> Vec<NodeId> {
        let mut l = Lexer::new(block, false);
        l.filename = filename.into();
        let mut rv = self.parse_block(&mut l).unwrap_or_default();
        if !rv.is_empty() {
            let f = self.arena.get(*rv.last().unwrap()).filename.clone();
            rv.push(self.arena.add(&(f, last_line + 1), NodeKind::Return { expression: None }));
        }
        rv
    }
}

enum What {
    One(String),
    Parts(Vec<(u32, String)>),
}

/// `ast.literal_eval` for a single Python string literal (with prefixes and
/// implicit concatenation).
pub fn literal_string(s: &str) -> Option<String> {
    let s = s.trim();
    let expr = crysirror_python::parser::parse_expression(s, crysirror_python::parser::ParseOptions::py3()).ok()?;
    match expr.kind {
        crysirror_python::ast::ExprKind::Constant(crysirror_python::ast::Constant::Str(v)) => Some(v),
        _ => None,
    }
}
